// LOT 7 — correctifs d'audit n° 1.
//
// Chaque partie ajoute d'abord un test qui doit echouer sur le code d'avant,
// puis le correctif. Le banc est le meme que celui du LOT 6 : un amont factice
// local, un `cles.env` par cas, un dossier de donnees par cas.

use apivault_proxy::config::Config;
use apivault_proxy::metriques;
use apivault_proxy::serveur::{AppState, Service};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static COMPTEUR: AtomicUsize = AtomicUsize::new(0);
// Un seul verrou pour tout le fichier : les cas touchent a APPDATA et
// APIVAULT_CLE_ENV, qui sont communs au processus.
static VERROU_ENV: Mutex<()> = Mutex::new(());

const JETON: &str = "sk-FICTIF-MAITRE-42";

fn temp_dir() -> PathBuf {
    let n = COMPTEUR.fetch_add(1, Ordering::SeqCst);
    let dossier = std::env::temp_dir().join(format!("apivault-lot7-{}-{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dossier);
    fs::create_dir_all(&dossier).unwrap();
    dossier
}

fn port_libre() -> u16 {
    let ecoute = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    drop(ecoute);
    port
}

/// Amont factice : repond toujours le meme corps et compte ses appels.
fn amont_texte(corps: &'static str, type_contenu: &'static str, statut: u16) -> (u16, Arc<AtomicUsize>) {
    let ecoute = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    let compte = Arc::new(AtomicUsize::new(0));
    let compteur = compte.clone();
    std::thread::spawn(move || {
        while let Ok((mut flux, _)) = ecoute.accept() {
            compteur.fetch_add(1, Ordering::SeqCst);
            let mut tampon = [0u8; 8192];
            let _ = flux.read(&mut tampon);
            let reponse = format!(
                "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                statut,
                type_contenu,
                corps.len(),
                corps
            );
            let _ = flux.write_all(reponse.as_bytes());
            let _ = flux.flush();
            let _ = flux.shutdown(std::net::Shutdown::Both);
        }
    });
    (port, compte)
}

/// Amont SSE : envoie les trames fournies avec un petit delai, comme un vrai
/// fournisseur, puis ferme.
fn amont_flux(trames: Vec<String>) -> (u16, Arc<AtomicUsize>) {
    let ecoute = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    let compte = Arc::new(AtomicUsize::new(0));
    let compteur = compte.clone();
    std::thread::spawn(move || {
        while let Ok((mut flux, _)) = ecoute.accept() {
            compteur.fetch_add(1, Ordering::SeqCst);
            let mut tampon = [0u8; 8192];
            let _ = flux.read(&mut tampon);
            let entete = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            if flux.write_all(entete.as_bytes()).is_err() {
                continue;
            }
            for trame in &trames {
                if flux.write_all(trame.as_bytes()).is_err() {
                    break;
                }
                let _ = flux.flush();
                std::thread::sleep(Duration::from_millis(15));
            }
            let _ = flux.shutdown(std::net::Shutdown::Both);
        }
    });
    (port, compte)
}

struct Banc {
    service: Arc<Service>,
    port: u16,
    dossier: PathBuf,
    appdata_original: Option<String>,
}

impl Banc {
    fn url(&self, chemin: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, chemin)
    }

    fn donnees(&self) -> PathBuf {
        self.dossier.join("ApiVault_Proxy")
    }
}

impl Drop for Banc {
    fn drop(&mut self) {
        self.service.arreter();
        let _ = fs::remove_dir_all(&self.dossier);
        std::env::remove_var("APIVAULT_CLE_ENV");
        match &self.appdata_original {
            Some(valeur) => std::env::set_var("APPDATA", valeur),
            None => std::env::remove_var("APPDATA"),
        }
    }
}

fn preparer(nom: &str, port: u16, url_amont: u16) -> Banc {
    let dossier = temp_dir();
    let chemin = dossier.join("cles.env");
    let amont = format!("http://127.0.0.1:{}", url_amont);
    fs::write(
        &chemin,
        format!(
            "PROXY_PORT={}\nPROXY_MASTER_KEY={}\nPROXY_PLAFOND_MENSUEL=0\nDEEPSEEK_KEY=test\nDEEPSEEK_URLS={}\nOPENROUTER_KEY=test\nOPENROUTER_URLS={}\n",
            port, JETON, amont, amont
        ),
    )
    .unwrap();
    std::env::set_var("APIVAULT_CLE_ENV", &chemin);
    let appdata_original = std::env::var("APPDATA").ok();
    std::env::set_var("APPDATA", &dossier);
    let cfg = Config::charger();
    assert_eq!(cfg.port, port, "cas {}", nom);
    let app = Arc::new(AppState::nouveau(cfg));
    let service = Service::nouveau(port, JETON.to_string(), app);
    Banc {
        service,
        port,
        dossier,
        appdata_original,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap()
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
        if debut.elapsed() > Duration::from_secs(15) {
            panic!("la passerelle n'a pas demarre sur le port {port}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn demander(client: &reqwest::Client, banc: &Banc, flux: bool) -> reqwest::Response {
    client
        .post(banc.url("/v1/chat/completions"))
        .header("authorization", format!("Bearer {}", JETON))
        .json(&serde_json::json!({
            "model": "defaut",
            "stream": flux,
            "messages": [{ "role": "user", "content": "bonjour, raconte-moi une histoire courte" }]
        }))
        .send()
        .await
        .expect("requete relayee")
}

fn chemin_mesures(banc: &Banc) -> PathBuf {
    banc.donnees()
        .join(format!("metriques-{}.jsonl", metriques::periode_courante()))
}

fn lignes_mesures(banc: &Banc) -> Vec<Value> {
    let texte = fs::read_to_string(chemin_mesures(banc)).unwrap_or_default();
    texte
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("ligne JSON valide"))
        .collect()
}

/// Attend qu'une ligne de mesures apparaisse (le flux ecrit a sa fermeture).
async fn attendre_lignes(banc: &Banc, nombre: usize) -> Vec<Value> {
    for _ in 0..200 {
        let lignes = lignes_mesures(banc);
        if lignes.len() >= nombre {
            return lignes;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "aucune ligne de mesures apres 5 s (fichier : {})",
        chemin_mesures(banc).display()
    );
}

// ---------------------------------------------------------------------------
// Partie A — M-1 : estimation quand l'usage manque
// ---------------------------------------------------------------------------

const FLUX_SANS_USAGE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"Voici une histoire courte.\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\" Elle parle d'un chat gris.\"}}]}\n\n",
    "data: [DONE]\n\n"
);

const FLUX_AVEC_USAGE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"bonjour\"}}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"prompt_cache_hit_tokens\":60}}\n\n",
    "data: [DONE]\n\n"
);

const JSON_SANS_USAGE: &str = r#"{"id":"x","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"bonjour, ceci est une reponse sans usage declare par le fournisseur."},"finish_reason":"stop"}]}"#;

#[tokio::test]
async fn flux_sans_usage_donne_une_estimation_non_nulle() {
    let _garde = VERROU_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let trames: Vec<String> = FLUX_SANS_USAGE.split_inclusive("\n\n").map(|s| s.to_string()).collect();
    let (port_amont, _compte) = amont_flux(trames);
    let port = port_libre();
    let banc = preparer("flux-sans-usage", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = demander(&client, &banc, true).await;
    assert_eq!(reponse.status(), 200);
    let _ = reponse.text().await.unwrap();

    let lignes = attendre_lignes(&banc, 1).await;
    let ligne = lignes.last().unwrap();
    assert_eq!(ligne["flux"], true);
    assert_eq!(ligne["estime"], true, "sans usage, la ligne doit etre estimee");
    assert!(
        ligne["tokens_in"].as_u64().unwrap_or(0) > 0,
        "l'entree doit etre estimee, recu : {}",
        ligne["tokens_in"]
    );
    assert!(
        ligne["tokens_out"].as_u64().unwrap_or(0) > 0,
        "la sortie doit etre estimee, recu : {}",
        ligne["tokens_out"]
    );
}

#[tokio::test]
async fn flux_avec_usage_reste_exact() {
    let _garde = VERROU_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let trames: Vec<String> = FLUX_AVEC_USAGE.split_inclusive("\n\n").map(|s| s.to_string()).collect();
    let (port_amont, _compte) = amont_flux(trames);
    let port = port_libre();
    let banc = preparer("flux-avec-usage", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = demander(&client, &banc, true).await;
    assert_eq!(reponse.status(), 200);
    let _ = reponse.text().await.unwrap();

    let lignes = attendre_lignes(&banc, 1).await;
    let ligne = lignes.last().unwrap();
    assert_eq!(ligne["estime"], false);
    assert_eq!(ligne["tokens_in"], 100);
    assert_eq!(ligne["tokens_out"], 20);
    assert_eq!(ligne["tokens_cache"], 60);
}

#[tokio::test]
async fn non_flux_sans_usage_donne_une_estimation_non_nulle() {
    let _garde = VERROU_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let (port_amont, _compte) = amont_texte(JSON_SANS_USAGE, "application/json", 200);
    let port = port_libre();
    let banc = preparer("json-sans-usage", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = demander(&client, &banc, false).await;
    assert_eq!(reponse.status(), 200);
    let _ = reponse.text().await.unwrap();

    let lignes = attendre_lignes(&banc, 1).await;
    let ligne = lignes.last().unwrap();
    assert_eq!(ligne["flux"], false);
    assert_eq!(ligne["estime"], true, "sans usage, la ligne doit etre estimee");
    assert!(
        ligne["tokens_in"].as_u64().unwrap_or(0) > 0,
        "l'entree doit etre estimee, recu : {}",
        ligne["tokens_in"]
    );
    assert!(
        ligne["tokens_out"].as_u64().unwrap_or(0) > 0,
        "la sortie doit etre estimee, recu : {}",
        ligne["tokens_out"]
    );
}
