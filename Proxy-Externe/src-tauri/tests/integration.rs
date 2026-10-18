//! Tests d'integration de la passerelle reelle.
//!
//! Chaque test demarre un faux fournisseur local (axum), ecrit un `cles.env`
//! temporaire, pointe `APIVAULT_CLE_ENV` dessus, puis lance le vrai `Service`
//! sur un port libre. Les tests qui touchent l'environnement sont serialises
//! par `VERROU` (variables d'environnement globales au processus).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use tokio::sync::Mutex;

use apivault_proxy::config::Config;
use apivault_proxy::serveur::{AppState, Service};

static VERROU: Mutex<()> = Mutex::const_new(());

const JETON: &str = "sk-apivault-integration";
const CORPS_OK: &str = r#"{"id":"chatcmpl-test","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;

#[derive(Clone, Copy)]
enum Comportement {
    /// 200 + JSON complet.
    Succes,
    /// 401 immediat.
    Interdit,
    /// 429 immediat.
    Trop,
    /// Ne repond qu'apres 60 s.
    Lent,
    /// Repond 200 en SSE puis coupe le corps au milieu.
    FluxCasse,
}

struct Factice {
    base: String,
    appels: Arc<AtomicUsize>,
}

async fn demarrer_factice(comportement: Comportement) -> Factice {
    #[derive(Clone)]
    struct Etat {
        comportement: Comportement,
        appels: Arc<AtomicUsize>,
    }

    async fn chat(State(etat): State<Etat>) -> Response {
        etat.appels.fetch_add(1, Ordering::SeqCst);
        match etat.comportement {
            Comportement::Succes => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(CORPS_OK))
                .unwrap(),
            Comportement::Interdit => Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"error":{"message":"invalid api key"}}"#))
                .unwrap(),
            Comportement::Trop => Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"error":{"message":"rate limited"}}"#))
                .unwrap(),
            Comportement::Lent => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(CORPS_OK))
                    .unwrap()
            }
            Comportement::FluxCasse => {
                use futures_util::StreamExt;
                // Les delais garantissent que les en-tetes et les deux premieres
                // trames partent avant la coupure (sinon hyper abandonne avant
                // d'avoir ecrit la reponse).
                let flux = futures_util::stream::iter(vec![
                    Ok::<Bytes, std::io::Error>(Bytes::from_static(
                        b"data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a\"},\"finish_reason\":null}]}\n\n",
                    )),
                    Ok(Bytes::from_static(
                        b"data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"b\"},\"finish_reason\":null}]}\n\n",
                    )),
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "coupure volontaire du flux",
                    )),
                ])
                .then(|element| async move {
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    element
                });
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(flux))
                    .unwrap()
            }
        }
    }

    let appels = Arc::new(AtomicUsize::new(0));
    let etat = Etat {
        comportement,
        appels: Arc::clone(&appels),
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/health", get(|| async { "ok" }))
        .with_state(etat);

    let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adresse = ecoute.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(ecoute, app).await;
    });

    Factice {
        base: format!("http://{adresse}/v1"),
        appels,
    }
}

fn port_libre() -> u16 {
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    drop(ecoute);
    port
}

/// Configuration d'un dossier temporaire propre a un test.
fn dossier_temporaire(nom: &str) -> std::path::PathBuf {
    let dossier = std::env::temp_dir().join(format!(
        "apivault-test-{}-{}",
        nom,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dossier);
    std::fs::create_dir_all(&dossier).unwrap();
    dossier
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

/// Demarre la passerelle : `deepseek_urls` et `openrouter_urls` pointent vers
/// les faux fournisseurs fournis.
async fn demarrer(
    nom: &str,
    jeton_configure: &str,
    deepseek_urls: &str,
    openrouter_urls: &str,
) -> Banc {
    let dossier = dossier_temporaire(nom);
    let port = port_libre();
    let chemin = dossier.join("cles.env");
    let contenu = format!(
        "PROXY_PORT={}\nPROXY_MASTER_KEY={}\nDEEPSEEK_KEY=test\nOPENROUTER_KEY=test\nNVIDIA_KEY=test\nDEEPSEEK_URLS={}\nOPENROUTER_URLS={}\n",
        port, jeton_configure, deepseek_urls, openrouter_urls
    );
    std::fs::write(&chemin, contenu).unwrap();
    std::env::set_var("APIVAULT_CLE_ENV", &chemin);

    // Les etats de pool et le journal vivent dans %APPDATA%\ApiVault_Proxy :
    // on les isole dans le dossier temporaire du test.
    let appdata_original = std::env::var("APPDATA").ok();
    std::env::set_var("APPDATA", &dossier);

    let cfg = Config::charger();
    assert_eq!(cfg.port, port, "le port du fichier de test doit etre lu");

    let app = Arc::new(AppState::nouveau(cfg));
    let service = Service::nouveau(port, jeton_configure.to_string(), app);

    let client = reqwest::Client::new();
    let debut = Instant::now();
    service.demarrer();
    loop {
        if let Ok(r) = client
            .get(format!("http://127.0.0.1:{}/health", port))
            .send()
            .await
        {
            if r.status().is_success() {
                break;
            }
        }
        if debut.elapsed() > Duration::from_secs(10) {
            panic!("la passerelle n'a pas demarre sur le port {port}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Banc {
        service,
        port,
        dossier,
        appdata_original,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap()
}

fn corps_chat(alias: &str, flux: bool) -> String {
    format!(
        r#"{{"model":"{}","messages":[{{"role":"user","content":"ping"}}],"stream":{}}}"#,
        alias, flux
    )
}

// ---------------------------------------------------------------------------
// a) sans jeton / mauvais jeton -> 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_sans_jeton_et_mauvais_jeton_donnent_401() {
    let _verrou = VERROU.lock().await;
    let factice = demarrer_factice(Comportement::Succes).await;
    let banc = demarrer("jeton", "sk-apivault-bon", &factice.base, &factice.base).await;

    let sans_entete = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(corps_chat("rapide", false))
        .send()
        .await
        .unwrap();
    assert_eq!(sans_entete.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        factice.appels.load(Ordering::SeqCst),
        0,
        "aucune requete amont ne doit partir sans jeton"
    );

    let mauvais = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer sk-apivault-faux")
        .body(corps_chat("rapide", false))
        .send()
        .await
        .unwrap();
    assert_eq!(mauvais.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(factice.appels.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// b) 401 amont -> bascule vers la 2e cible
// ---------------------------------------------------------------------------

#[tokio::test]
async fn b_401_amont_bascule_vers_la_cible_suivante() {
    let _verrou = VERROU.lock().await;
    let premier = demarrer_factice(Comportement::Interdit).await;
    let second = demarrer_factice(Comportement::Succes).await;
    let banc = demarrer("bascule401", JETON, &premier.base, &second.base).await;

    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("defaut", false))
        .send()
        .await
        .unwrap();

    let etat_b = reponse.status();
    let cible = reponse
        .headers()
        .get("x-apivault-cible")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let corps_b = reponse.text().await.unwrap();
    assert_eq!(etat_b, StatusCode::OK, "corps : {corps_b}");
    assert_eq!(premier.appels.load(Ordering::SeqCst), 1);
    assert_eq!(second.appels.load(Ordering::SeqCst), 1);
    assert!(
        cible.starts_with("openrouter/"),
        "la bascule doit viser le fournisseur sain, recu : {cible}"
    );
}

// ---------------------------------------------------------------------------
// c) 429 amont -> pause, bascule, pas de panic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn c_429_amont_pause_et_bascule_sans_panic() {
    let _verrou = VERROU.lock().await;
    let premier = demarrer_factice(Comportement::Trop).await;
    let second = demarrer_factice(Comportement::Succes).await;
    let banc = demarrer("bascule429", JETON, &premier.base, &second.base).await;

    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("defaut", false))
        .send()
        .await
        .unwrap();

    assert_eq!(reponse.status(), StatusCode::OK);
    assert_eq!(premier.appels.load(Ordering::SeqCst), 1);
    assert_eq!(second.appels.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// d) flux interrompu au milieu -> erreur SSE + fermeture, pas de blocage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn d_flux_interrompu_emet_une_erreur_sse_et_ferme() {
    let _verrou = VERROU.lock().await;
    let casse = demarrer_factice(Comportement::FluxCasse).await;
    let banc = demarrer("fluxcasse", JETON, &casse.base, &casse.base).await;

    let debut = Instant::now();
    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("rapide", true))
        .send()
        .await
        .unwrap();

    assert_eq!(reponse.status(), StatusCode::OK);
    let texte = reponse.text().await.unwrap();
    assert!(
        debut.elapsed() < Duration::from_secs(15),
        "le flux doit se fermer rapidement, mesure : {:?}",
        debut.elapsed()
    );
    assert!(
        texte.contains("upstream_broken"),
        "erreur SSE attendue, recu : {texte}"
    );
    assert!(texte.contains("\"error\""), "format OpenAI attendu : {texte}");
}

// ---------------------------------------------------------------------------
// e) succes -> UNE SEULE requete amont
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e_succes_ne_genere_qu_une_requete_amont() {
    let _verrou = VERROU.lock().await;
    let factice = demarrer_factice(Comportement::Succes).await;
    let banc = demarrer("succes", JETON, &factice.base, &factice.base).await;

    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("rapide", false))
        .send()
        .await
        .unwrap();

    assert_eq!(reponse.status(), StatusCode::OK);
    assert_eq!(
        factice.appels.load(Ordering::SeqCst),
        1,
        "aucune double generation : une seule requete amont"
    );
    let id = reponse
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(id.len(), 8, "X-Request-Id doit faire 8 caracteres : {id}");
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "recu : {id}");

    // Deuxieme requete : id different.
    let seconde = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("rapide", false))
        .send()
        .await
        .unwrap();
    assert_eq!(seconde.status(), StatusCode::OK);
    let id2 = seconde
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_ne!(id, id2, "deux requetes doivent porter deux ids differents");
    assert_eq!(factice.appels.load(Ordering::SeqCst), 2);
}

// ---------------------------------------------------------------------------
// g) corps au-dela de 16 MiB -> 413
// ---------------------------------------------------------------------------

#[tokio::test]
async fn g_corps_trop_gros_donne_413() {
    let _verrou = VERROU.lock().await;
    let factice = demarrer_factice(Comportement::Succes).await;
    let banc = demarrer("taille", JETON, &factice.base, &factice.base).await;

    // 17 MiB : au-dela de la limite explicite de 16 MiB.
    let gros = "x".repeat(17 * 1024 * 1024);
    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(gros)
        .send()
        .await
        .unwrap();

    assert_eq!(reponse.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        factice.appels.load(Ordering::SeqCst),
        0,
        "le corps doit etre rejete avant tout appel amont"
    );
}

// ---------------------------------------------------------------------------
// f) budget de bascule : un amont lent ne coupe plus, il bascule
// ---------------------------------------------------------------------------

/// Les tests ne peuvent pas attendre 120 s. Les deux delais de production sont
/// surchargeables par environnement, ce qui permet d'observer la politique de
/// bascule sans attendre.
fn regler_delais(delai_ms: u64, budget_ms: u64) {
    std::env::set_var("APIVAULT_DELAI_CIBLE_MS", delai_ms.to_string());
    std::env::set_var("APIVAULT_BUDGET_BASCULE_MS", budget_ms.to_string());
}

fn oublier_delais() {
    std::env::remove_var("APIVAULT_DELAI_CIBLE_MS");
    std::env::remove_var("APIVAULT_BUDGET_BASCULE_MS");
}

#[tokio::test]
async fn f_amont_lent_bascule_vers_la_cible_saine() {
    let _verrou = VERROU.lock().await;
    let lent = demarrer_factice(Comportement::Lent).await;
    let sain = demarrer_factice(Comportement::Succes).await;
    regler_delais(400, 8_000);
    let banc = demarrer("budget", JETON, &lent.base, &sain.base).await;

    let debut = Instant::now();
    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("defaut", false))
        .send()
        .await
        .unwrap();
    let duree = debut.elapsed();
    let cible = reponse
        .headers()
        .get("x-apivault-cible")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let etat = reponse.status();
    let corps = reponse.text().await.unwrap();
    oublier_delais();

    assert_eq!(
        etat,
        StatusCode::OK,
        "un delai depasse doit basculer, pas couper. corps : {corps}"
    );
    assert_eq!(
        lent.appels.load(Ordering::SeqCst),
        1,
        "la cible lente est bien tentee une fois"
    );
    assert_eq!(
        sain.appels.load(Ordering::SeqCst),
        1,
        "la cible saine doit servir la requete apres la bascule"
    );
    assert!(
        cible.starts_with("openrouter/"),
        "la bascule doit viser le fournisseur sain, recu : {cible}"
    );
    assert!(
        duree <= Duration::from_secs(8),
        "le budget de bascule doit rester une borne : {duree:?}"
    );
}

#[tokio::test]
async fn f_amont_lent_partout_donne_504_dans_le_budget() {
    let _verrou = VERROU.lock().await;
    let premier = demarrer_factice(Comportement::Lent).await;
    let second = demarrer_factice(Comportement::Lent).await;
    regler_delais(300, 1_000);
    let banc = demarrer("budget504", JETON, &premier.base, &second.base).await;

    let debut = Instant::now();
    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps_chat("defaut", false))
        .send()
        .await
        .unwrap();
    let duree = debut.elapsed();
    let etat = reponse.status();
    let corps = reponse.text().await.unwrap();
    oublier_delais();

    assert_eq!(etat, StatusCode::GATEWAY_TIMEOUT, "corps : {corps}");
    assert!(
        duree <= Duration::from_secs(5),
        "le budget global doit rester une borne : {duree:?}"
    );
    assert!(
        premier.appels.load(Ordering::SeqCst) >= 1,
        "au moins une cible doit avoir ete tentee"
    );
}

// ---------------------------------------------------------------------------
// h) BOM UTF-8 : un corps precede de EF BB BF doit etre accepte
// ---------------------------------------------------------------------------

#[tokio::test]
async fn h_corps_avec_bom_utf8_est_accepte() {
    let _verrou = VERROU.lock().await;
    let factice = demarrer_factice(Comportement::Succes).await;
    let banc = demarrer("bom", JETON, &factice.base, &factice.base).await;

    // Certains editeurs et clients Windows prependent un BOM UTF-8 au JSON.
    // Le BOM n'appartient pas au document : il doit etre retire avant lecture.
    let corps = format!("\u{FEFF}{}", corps_chat("rapide", false));

    let reponse = client()
        .post(banc.url("/v1/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps.clone())
        .send()
        .await
        .unwrap();
    let etat = reponse.status();
    let texte = reponse.text().await.unwrap();
    assert_eq!(
        etat,
        StatusCode::OK,
        "un BOM UTF-8 ne doit pas faire rejeter le corps : {texte}"
    );
    assert_eq!(
        factice.appels.load(Ordering::SeqCst),
        1,
        "le corps doit atteindre le fournisseur apres retrait du BOM"
    );

    // Route soeur : meme tolerance.
    let soeur = client()
        .post(banc.url("/chat/completions"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", JETON))
        .body(corps)
        .send()
        .await
        .unwrap();
    let etat_soeur = soeur.status();
    let texte_soeur = soeur.text().await.unwrap();
    assert_eq!(
        etat_soeur,
        StatusCode::OK,
        "le BOM doit aussi etre accepte sur /chat/completions : {texte_soeur}"
    );
    assert_eq!(factice.appels.load(Ordering::SeqCst), 2);
}
