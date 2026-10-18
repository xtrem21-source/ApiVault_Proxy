//! LOT 5 - compteur de tokens : format du journal et agregation.
//!
//! Ces tests ne touchent ni au reseau ni a l'interface : ils verifient le
//! contrat qui sera lu par le LOT 6 (plafond de consommation).

use std::fs;
use std::path::PathBuf;

use apivault_proxy::metriques::{self, Enregistrement, Metriques};

/// Dossier de travail propre a chaque test.
struct Banc {
    dossier: PathBuf,
}

impl Banc {
    fn nouveau(nom: &str) -> Banc {
        let dossier = std::env::temp_dir().join(format!(
            "apivault-lot5-{}-{}",
            nom,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dossier).expect("creation du dossier de test");
        Banc { dossier }
    }
}

impl Drop for Banc {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dossier);
    }
}

fn enregistrement(id: &str, alias: &str, entree: u64, sortie: u64, cache: u64) -> Enregistrement {
    let mut e = Enregistrement::nouveau(id, alias);
    e.horodatage = "2026-10-18T18:12:03.412+02:00".to_string();
    e.fournisseur = "deepseek".to_string();
    e.modele = "deepseek-chat".to_string();
    e.statut = 200;
    e.latence_ms = 842;
    e.tokens_in = entree;
    e.tokens_out = sortie;
    e.tokens_cache = cache;
    e.estime = false;
    e.tentative = 1;
    e
}

#[test]
fn une_ligne_json_par_requete_et_aucune_cle() {
    let banc = Banc::nouveau("format");
    let mesure = Metriques::nouveau(&banc.dossier);

    mesure.enregistrer(&enregistrement("req-1", "defaut", 128, 64, 32)).unwrap();
    mesure.enregistrer(&enregistrement("req-2", "defaut", 10, 5, 0)).unwrap();

    let fichier = banc.dossier.join("metriques-2026-10.jsonl");
    assert!(fichier.exists(), "le fichier du mois doit exister : {:?}", fichier);
    let contenu = fs::read_to_string(&fichier).unwrap();
    let lignes: Vec<&str> = contenu.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lignes.len(), 2, "une ligne par requete");

    // Le format est public : voici la liste des champs, et rien d'autre.
    let valeur: serde_json::Value = serde_json::from_str(lignes[0]).expect("JSON valide");
    let objet = valeur.as_object().expect("objet JSON");
    let attendus = [
        "id",
        "horodatage",
        "jeton",
        "alias",
        "fournisseur",
        "modele",
        "statut",
        "latence_ms",
        "tokens_in",
        "tokens_out",
        "tokens_cache",
        "cout_eur",
        "estime",
        "tentative",
        "flux",
    ];
    for champ in attendus {
        assert!(objet.contains_key(champ), "champ manquant : {}", champ);
    }
    assert_eq!(objet.len(), attendus.len(), "aucun champ inattendu");
    assert_eq!(valeur["tokens_in"], 128);
    assert_eq!(valeur["tokens_cache"], 32);
    assert_eq!(valeur["cout_eur"], serde_json::Value::Null);

    // Rien de sensible : ni cle, ni contenu de message.
    assert!(!contenu.contains("sk-"), "aucune cle ne doit figurer dans le journal");
    assert!(!contenu.contains("Authorization"));
}

#[test]
fn agregation_somme_entree_sortie_cache() {
    let banc = Banc::nouveau("agregation");
    let mesure = Metriques::nouveau(&banc.dossier);

    mesure.enregistrer(&enregistrement("r1", "defaut", 128, 64, 32)).unwrap();
    mesure.enregistrer(&enregistrement("r2", "defaut", 10, 5, 0)).unwrap();
    mesure.enregistrer(&enregistrement("r3", "travail", 200, 100, 50)).unwrap();

    let totaux = mesure.totaliser("2026-10").expect("agregation");
    assert_eq!(totaux.requetes, 3);
    assert_eq!(totaux.entrees, 338);
    assert_eq!(totaux.sorties, 169);
    assert_eq!(totaux.cache, 82);
    assert_eq!(totaux.estimees, 0);
    assert_eq!(totaux.lignes_illisibles, 0);

    assert_eq!(totaux.par_alias.len(), 2);
    let defaut = totaux
        .par_alias
        .iter()
        .find(|d| d.nom == "defaut")
        .expect("alias defaut");
    assert_eq!(defaut.requetes, 2);
    assert_eq!(defaut.entrees, 138);
    let travail = totaux
        .par_alias
        .iter()
        .find(|d| d.nom == "travail")
        .expect("alias travail");
    assert_eq!(travail.sorties, 100);
    assert_eq!(totaux.par_modele.len(), 1);
    assert_eq!(totaux.par_modele[0].requetes, 3);
}

#[test]
fn les_periodes_ne_se_melangent_pas() {
    let banc = Banc::nouveau("periodes");
    let mesure = Metriques::nouveau(&banc.dossier);

    mesure.enregistrer(&enregistrement("r1", "defaut", 100, 50, 0)).unwrap();
    let mut septembre = enregistrement("r2", "defaut", 7, 3, 0);
    septembre.horodatage = "2026-09-30T23:59:59.000+02:00".to_string();
    mesure.enregistrer(&septembre).unwrap();

    assert!(banc.dossier.join("metriques-2026-10.jsonl").exists());
    assert!(banc.dossier.join("metriques-2026-09.jsonl").exists());

    let octobre = mesure.totaliser("2026-10").unwrap();
    assert_eq!(octobre.requetes, 1);
    assert_eq!(octobre.entrees, 100);

    let septembre = mesure.totaliser("2026-09").unwrap();
    assert_eq!(septembre.requetes, 1);
    assert_eq!(septembre.entrees, 7);

    // Mois sans aucune donnee : des zeros, pas une erreur.
    let aout = mesure.totaliser("2026-08").unwrap();
    assert_eq!(aout.requetes, 0);
    assert_eq!(aout.entrees, 0);
    assert_eq!(aout.periode, "2026-08");
}

#[test]
fn ligne_illisible_ignoree_et_comptee() {
    let banc = Banc::nouveau("illisible");
    let mesure = Metriques::nouveau(&banc.dossier);
    mesure.enregistrer(&enregistrement("r1", "defaut", 10, 5, 0)).unwrap();

    let fichier = banc.dossier.join("metriques-2026-10.jsonl");
    let mut contenu = fs::read_to_string(&fichier).unwrap();
    contenu.push_str("{ceci n'est pas du json}\n");
    fs::write(&fichier, contenu).unwrap();

    let totaux = mesure.totaliser("2026-10").unwrap();
    assert_eq!(totaux.requetes, 1, "la ligne cassee ne compte pas comme requete");
    assert_eq!(totaux.lignes_illisibles, 1);
    assert_eq!(totaux.entrees, 10);
}

#[test]
fn usage_des_trois_formes_de_fournisseurs() {
    // OpenAI / DeepSeek
    let openai: serde_json::Value = serde_json::json!({
        "usage": { "prompt_tokens": 128, "completion_tokens": 64, "total_tokens": 192 }
    });
    assert_eq!(metriques::usage_depuis_json(&openai), Some((128, 64, 0)));

    // DeepSeek avec cache explicite
    let deepseek: serde_json::Value = serde_json::json!({
        "usage": {
            "prompt_tokens": 128,
            "completion_tokens": 64,
            "prompt_cache_hit_tokens": 32,
            "prompt_cache_miss_tokens": 96
        }
    });
    assert_eq!(metriques::usage_depuis_json(&deepseek), Some((128, 64, 32)));

    // OpenAI recent : cache dans prompt_tokens_details
    let details: serde_json::Value = serde_json::json!({
        "usage": {
            "prompt_tokens": 50,
            "completion_tokens": 20,
            "prompt_tokens_details": { "cached_tokens": 40 }
        }
    });
    assert_eq!(metriques::usage_depuis_json(&details), Some((50, 20, 40)));

    // Anthropic
    let anthropic: serde_json::Value = serde_json::json!({
        "usage": { "input_tokens": 11, "output_tokens": 22, "cache_read_input_tokens": 33 }
    });
    assert_eq!(metriques::usage_depuis_json(&anthropic), Some((11, 22, 33)));

    // Pas d'usage du tout : le fournisseur n'annonce rien.
    let sans: serde_json::Value = serde_json::json!({ "choices": [] });
    assert_eq!(metriques::usage_depuis_json(&sans), None);
}

#[test]
fn usage_lu_dans_un_flux_sse() {
    let flux = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"bon\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"jour\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,",
        "\"prompt_cache_hit_tokens\":60}}\n\n",
        "data: [DONE]\n\n"
    );
    assert_eq!(metriques::usage_dans_flux(flux), Some((100, 20, 60)));
    assert_eq!(metriques::usage_dans_flux("data: [DONE]\n\n"), None);
    // Evenement coupe en deux : le morceau incomplet est ignore, pas fatal.
    let coupe = "data: {\"usage\":{\"prompt_tokens\":10";
    assert_eq!(metriques::usage_dans_flux(coupe), None);
}

#[test]
fn periode_valide_et_nom_de_fichier() {
    assert!(metriques::periode_valide("2026-10"));
    assert!(metriques::periode_valide("2026-01"));
    assert!(!metriques::periode_valide("2026-13"));
    assert!(!metriques::periode_valide("2026-1"));
    assert!(!metriques::periode_valide("26-10-18"));
    assert!(!metriques::periode_valide(""));

    let chemin = metriques::chemin_periode(std::path::Path::new("C:\\donnees"), "2026-10");
    assert_eq!(chemin, PathBuf::from("C:\\donnees").join("metriques-2026-10.jsonl"));
}

#[test]
fn la_periode_courante_a_la_forme_attendue() {
    let periode = metriques::periode_courante();
    assert!(
        metriques::periode_valide(&periode),
        "periode courante inattendue : {}",
        periode
    );
}

// ---------------------------------------------------------------------------
// Capture de l'usage sur une vraie requete relayee
// ---------------------------------------------------------------------------

use std::io::{Read as _, Write as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

use apivault_proxy::config::Config;
use apivault_proxy::serveur::{AppState, Service};
use serde_json::Value;
use tokio::sync::Mutex;

static VERROU: Mutex<()> = Mutex::const_new(());
const JETON: &str = "sk-apivault-lot5";

fn port_libre() -> u16 {
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    drop(ecoute);
    port
}

/// Amont factice : repond `attentes` fois avec le corps et le type donnes.
fn amont_factice(
    corps: &'static str,
    type_contenu: &'static str,
    statut: u16,
    attentes: usize,
) -> (u16, std::thread::JoinHandle<()>) {
    let ecoute = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = ecoute.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        for _ in 0..attentes {
            let Ok((mut flux, _)) = ecoute.accept() else {
                return;
            };
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
    (port, handle)
}

struct BancService {
    service: Arc<Service>,
    port: u16,
    dossier: PathBuf,
    appdata_original: Option<String>,
}

impl BancService {
    fn url(&self, chemin: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, chemin)
    }
}

impl Drop for BancService {
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

/// Prepare un `cles.env` temporaire dont les fournisseurs pointent sur l'amont
/// factice, et rend le service sans le demarrer.
fn preparer(nom: &str, port: u16, url_amont: u16) -> BancService {
    let dossier = std::env::temp_dir().join(format!(
        "apivault-lot5-{}-{}",
        nom,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dossier);
    fs::create_dir_all(&dossier).unwrap();
    let chemin = dossier.join("cles.env");
    let amont = format!("http://127.0.0.1:{}", url_amont);
    fs::write(
        &chemin,
        format!(
            "PROXY_PORT={}\nPROXY_MASTER_KEY={}\nDEEPSEEK_KEY=test\nDEEPSEEK_URLS={}\nOPENROUTER_KEY=test\nOPENROUTER_URLS={}\n",
            port, JETON, amont, amont
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
    BancService {
        service,
        port,
        dossier,
        appdata_original,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
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

/// Le journal ecrit par le service, ou qu'il soit dans le dossier de donnees.
fn trouver_journal(dossier: &std::path::Path) -> Option<PathBuf> {
    let mut pile = vec![dossier.to_path_buf()];
    let mut trouve = None;
    while let Some(d) = pile.pop() {
        if let Ok(entrees) = fs::read_dir(&d) {
            for e in entrees.flatten() {
                let p = e.path();
                if p.is_dir() {
                    pile.push(p);
                } else if p
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with("metriques-"))
                    .unwrap_or(false)
                {
                    trouve = Some(p);
                }
            }
        }
    }
    trouve
}

fn journal_ecrit(dossier: &std::path::Path) -> (PathBuf, Vec<Value>) {
    let chemin = trouver_journal(dossier).expect("un fichier metriques-AAAAMM.jsonl doit exister");
    let contenu = fs::read_to_string(&chemin).unwrap();
    let lignes = contenu
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("chaque ligne est du JSON"))
        .collect();
    (chemin, lignes)
}

#[tokio::test]
async fn capture_non_flux_ecrit_les_tokens() {
    let _garde = VERROU.lock().await;
    let reponse_amont = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"bonjour"}}],"usage":{"prompt_tokens":128,"completion_tokens":64,"prompt_cache_hit_tokens":32}}"#;
    let (port_amont, _serveur) = amont_factice(reponse_amont, "application/json", 200, 1);
    let port = port_libre();
    let banc = preparer("non-flux", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = client
        .post(banc.url("/v1/chat/completions"))
        .header("authorization", format!("Bearer {}", JETON))
        .json(&serde_json::json!({
            "model": "defaut",
            "messages": [{ "role": "user", "content": "bonjour" }]
        }))
        .send()
        .await
        .expect("requete relayee");
    assert_eq!(reponse.status(), 200);
    let corps: Value = reponse.json().await.unwrap();
    assert_eq!(corps["choices"][0]["message"]["content"], "bonjour");

    let (chemin, lignes) = journal_ecrit(&banc.dossier);
    assert_eq!(lignes.len(), 1, "une requete = une ligne : {:?}", chemin);
    let ligne = &lignes[0];
    assert_eq!(ligne["alias"], "defaut");
    assert_eq!(ligne["fournisseur"], "deepseek");
    assert_eq!(ligne["modele"], "deepseek-chat");
    assert_eq!(ligne["statut"], 200);
    assert_eq!(ligne["tokens_in"], 128);
    assert_eq!(ligne["tokens_out"], 64);
    assert_eq!(ligne["tokens_cache"], 32);
    assert_eq!(ligne["estime"], false);
    assert_eq!(ligne["flux"], false);
    assert_eq!(ligne["tentative"], 1);
    assert!(!ligne["id"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn capture_flux_ecrit_les_tokens_a_la_fin() {
    let _garde = VERROU.lock().await;
    let flux = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"bon\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"jour\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,",
        "\"prompt_cache_hit_tokens\":60}}\n\n",
        "data: [DONE]\n\n"
    );
    let (port_amont, _serveur) = amont_factice(flux, "text/event-stream", 200, 1);
    let port = port_libre();
    let banc = preparer("flux", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = client
        .post(banc.url("/v1/chat/completions"))
        .header("authorization", format!("Bearer {}", JETON))
        .json(&serde_json::json!({
            "model": "defaut",
            "stream": true,
            "messages": [{ "role": "user", "content": "bonjour" }]
        }))
        .send()
        .await
        .expect("requete relayee");
    assert_eq!(reponse.status(), 200);
    let texte = reponse.text().await.expect("flux lu jusqu'au bout");
    assert!(texte.contains("bon"), "le flux doit traverser : {}", texte);

    // La ligne n'est ecrite qu'a la fermeture du flux : on laisse au gardien
    // le temps de conclure.
    let mut lignes = Vec::new();
    for _ in 0..100 {
        if trouver_journal(&banc.dossier).is_some() {
            let (_, l) = journal_ecrit(&banc.dossier);
            if !l.is_empty() {
                lignes = l;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(lignes.len(), 1, "une seule ligne pour un flux");
    let ligne = &lignes[0];
    assert_eq!(ligne["flux"], true);
    assert_eq!(ligne["tokens_in"], 100);
    assert_eq!(ligne["tokens_out"], 20);
    assert_eq!(ligne["tokens_cache"], 60);
    assert_eq!(ligne["estime"], false);
    assert_eq!(ligne["statut"], 200);
}

#[tokio::test]
async fn admin_metriques_rend_les_totaux_et_refuse_sans_jeton() {
    let _garde = VERROU.lock().await;
    let reponse_amont = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"prompt_cache_hit_tokens":2}}"#;
    let (port_amont, _serveur) = amont_factice(reponse_amont, "application/json", 200, 1);
    let port = port_libre();
    let banc = preparer("admin", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let _ = client
        .post(banc.url("/v1/chat/completions"))
        .header("authorization", format!("Bearer {}", JETON))
        .json(&serde_json::json!({
            "model": "defaut",
            "messages": [{ "role": "user", "content": "salut" }]
        }))
        .send()
        .await
        .unwrap();

    // Sans jeton : refus.
    let sans = client.get(banc.url("/admin/metriques")).send().await.unwrap();
    assert_eq!(sans.status(), 401, "l'endpoint doit exiger le jeton");

    // Avec jeton : les totaux de la periode.
    let periode = metriques::periode_courante();
    let avec = client
        .get(banc.url(&format!("/admin/metriques?periode={}", periode)))
        .header("authorization", format!("Bearer {}", JETON))
        .send()
        .await
        .unwrap();
    assert_eq!(avec.status(), 200);
    let totaux: Value = avec.json().await.unwrap();
    assert_eq!(totaux["periode"], periode);
    assert_eq!(totaux["requetes"], 1);
    assert_eq!(totaux["entrees"], 10);
    assert_eq!(totaux["sorties"], 5);
    assert_eq!(totaux["cache"], 2);
    assert_eq!(totaux["estimees"], 0);
    assert_eq!(totaux["par_alias"][0]["nom"], "defaut");

    // Periode mal formee : 400 explicite, pas une panique.
    let invalide = client
        .get(banc.url("/admin/metriques?periode=2026-13"))
        .header("authorization", format!("Bearer {}", JETON))
        .send()
        .await
        .unwrap();
    assert_eq!(invalide.status(), 400);
    let corps: Value = invalide.json().await.unwrap();
    assert_eq!(corps["error"]["code"], "periode_invalide");
}

#[tokio::test]
async fn echec_amont_compte_et_marque_estime() {
    let _garde = VERROU.lock().await;
    let (port_amont, _serveur) = amont_factice(
        r#"{"error":{"message":"boum"}}"#,
        "application/json",
        500,
        2,
    );
    let port = port_libre();
    let banc = preparer("echec", port, port_amont);
    banc.service.demarrer();
    let client = client();
    attendre_sante(&client, port).await;

    let reponse = client
        .post(banc.url("/v1/chat/completions"))
        .header("authorization", format!("Bearer {}", JETON))
        .json(&serde_json::json!({
            "model": "defaut",
            "messages": [{ "role": "user", "content": "bonjour" }]
        }))
        .send()
        .await
        .expect("le proxy doit rendre une reponse d'erreur");
    assert!(!reponse.status().is_success(), "l'amont a echoue");

    let (_, lignes) = journal_ecrit(&banc.dossier);
    assert_eq!(lignes.len(), 1, "un echec compte aussi comme une requete");
    let ligne = &lignes[0];
    assert_eq!(ligne["estime"], true, "sans usage, la ligne est estimee");
    assert_eq!(ligne["tokens_in"], 0);
    assert_eq!(ligne["tokens_out"], 0);
    assert!(ligne["statut"].as_u64().unwrap() >= 400);
    assert_eq!(ligne["alias"], "defaut");
}
