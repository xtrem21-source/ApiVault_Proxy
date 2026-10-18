//! LOT 3 : port occupe (A1), /health minimal (B1), validation `Host` (B2).
//!
//! Ce fichier est un binaire de test separe : il n'altere pas les tests
//! d'integration existants. Les variables d'environnement sont globales au
//! processus, donc les tests sont serialises par `VERROU`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::header;
use serde_json::Value;
use tokio::sync::Mutex;

use apivault_proxy::config::Config;
use apivault_proxy::serveur::{AppState, Service};

static VERROU: Mutex<()> = Mutex::const_new(());

const JETON: &str = "sk-apivault-lot3";

fn port_libre() -> u16 {
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    drop(ecoute);
    port
}

struct Banc {
    service: Arc<Service>,
    port: u16,
    dossier: std::path::PathBuf,
    appdata_original: Option<String>,
}

impl Banc {
    fn url(&self, chemin: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, chemin)
    }
}

impl Drop for Banc {
    fn drop(&mut self) {
        self.service.arreter();
        let _ = std::fs::remove_dir_all(&self.dossier);
        std::env::remove_var("APIVAULT_CLE_ENV");
        match &self.appdata_original {
            Some(valeur) => std::env::set_var("APPDATA", valeur),
            None => std::env::remove_var("APPDATA"),
        }
    }
}

/// Prépare un `cles.env` temporaire et rend le service sans le demarrer.
fn preparer(nom: &str, port: u16) -> (Arc<Service>, std::path::PathBuf, Option<String>) {
    let dossier = std::env::temp_dir().join(format!(
        "apivault-lot3-{}-{}",
        nom,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dossier);
    std::fs::create_dir_all(&dossier).unwrap();
    let chemin = dossier.join("cles.env");
    std::fs::write(
        &chemin,
        format!(
            "PROXY_PORT={}\nPROXY_MASTER_KEY={}\nDEEPSEEK_KEY=test\n",
            port, JETON
        ),
    )
    .unwrap();
    std::env::set_var("APIVAULT_CLE_ENV", &chemin);
    let appdata_original = std::env::var("APPDATA").ok();
    std::env::set_var("APPDATA", &dossier);
    let cfg = Config::charger();
    assert_eq!(cfg.port, port);
    let app = Arc::new(AppState::nouveau(cfg));
    let service = Service::nouveau(port, JETON.to_string(), app);
    (service, dossier, appdata_original)
}

async fn attendre_sante(client: &reqwest::Client, port: u16) {
    let debut = Instant::now();
    loop {
        if let Ok(r) = client
            .get(format!("http://127.0.0.1:{}/health", port))
            .send()
            .await
        {
            if r.status().is_success() {
                return;
            }
        }
        if debut.elapsed() > Duration::from_secs(10) {
            panic!("la passerelle n'a pas demarre sur le port {port}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

#[tokio::test]
async fn b1_health_ne_rend_que_status_et_version() {
    let _garde = VERROU.lock().await;
    let port = port_libre();
    let (service, dossier, appdata_original) = preparer("health", port);
    let banc = Banc {
        service,
        port,
        dossier,
        appdata_original,
    };
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = client.get(banc.url("/health")).send().await.unwrap();
    assert_eq!(reponse.status(), 200);
    let corps: Value = reponse.json().await.unwrap();
    let objet = corps.as_object().expect("un objet JSON");
    assert_eq!(
        objet.len(),
        2,
        "sans jeton, /health ne doit exposer que status et version : {corps}"
    );
    assert_eq!(corps["status"], "ok");
    assert!(corps.get("version").is_some());
    for interdit in ["uptime_s", "alias", "cles_chargees", "service"] {
        assert!(
            corps.get(interdit).is_none(),
            "/health ne doit plus exposer {interdit} : {corps}"
        );
    }
}

#[tokio::test]
async fn b2_hote_etranger_refuse_et_hote_local_accepte() {
    let _garde = VERROU.lock().await;
    let port = port_libre();
    let (service, dossier, appdata_original) = preparer("hote", port);
    let banc = Banc {
        service,
        port,
        dossier,
        appdata_original,
    };
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let etranger = client
        .get(banc.url("/health"))
        .header(header::HOST, "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(etranger.status(), 403, "Host etranger doit etre refuse");
    let corps: Value = etranger.json().await.unwrap();
    assert_eq!(corps["error"]["code"], "hote_refuse");

    let local = client
        .get(banc.url("/health"))
        .header(header::HOST, format!("127.0.0.1:{}", port))
        .send()
        .await
        .unwrap();
    assert_eq!(local.status(), 200, "Host 127.0.0.1 doit etre accepte");

    let localhost = client
        .get(banc.url("/health"))
        .header(header::HOST, format!("localhost:{}", port))
        .send()
        .await
        .unwrap();
    assert_eq!(localhost.status(), 200, "Host localhost doit etre accepte");
}

#[tokio::test]
async fn a1_port_occupe_message_explicite_et_cause_dans_le_journal() {
    let _garde = VERROU.lock().await;
    // Un ecouteur factice garde le port : la passerelle ne peut pas s'y lier.
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    let (service, dossier, appdata_original) = preparer("port", port);
    let banc = Banc {
        service,
        port,
        dossier,
        appdata_original,
    };
    banc.service.demarrer();

    let debut = Instant::now();
    let mut etat = banc.service.etat_json();
    while etat["erreur"].is_null() && debut.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        etat = banc.service.etat_json();
    }
    assert!(!etat["actif"].as_bool().unwrap_or(true));
    assert_eq!(
        etat["erreur_code"], "port_occupe",
        "le code doit permettre a l'interface d'afficher un message actionnable : {etat}"
    );
    let message = etat["erreur"].as_str().unwrap_or_default();
    assert!(
        message.contains(&port.to_string()) && message.contains("deja utilise"),
        "message explicite attendu, obtenu : {message}"
    );

    let journal = banc.service.app.lignes_journal().join("\n");
    assert!(
        journal.contains("os error"),
        "la cause technique doit rester dans le journal : {journal}"
    );
    // LOT 7 bis, point 6 : le journal doit aussi porter le message actionnable,
    // pas seulement la cause technique (audit : « dans l'interface et dans le
    // journal »). Un utilisateur qui lit le journal doit savoir quoi faire.
    assert!(
        journal.contains("deja utilise"),
        "le journal doit dire que le port est deja utilise : {journal}"
    );
    assert!(
        journal.contains(&port.to_string()),
        "le journal doit nommer le port occupe : {journal}"
    );
    assert!(
        journal.contains("relance"),
        "le journal doit dire quoi faire (relancer) : {journal}"
    );
    drop(ecoute);
}
