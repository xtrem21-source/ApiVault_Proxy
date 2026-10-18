// LOT 6 — migration au demarrage puis plafond mensuel.
//
// Les tests n'ecrivent jamais dans le vrai %APPDATA% : chaque cas fabrique son
// propre dossier de donnees et le passe explicitement aux fonctions testees.

use apivault_proxy::coffre;
use apivault_proxy::config;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static COMPTEUR: AtomicUsize = AtomicUsize::new(0);

// Les cas qui touchent aux variables d'environnement ne peuvent pas tourner en
// parallele entre eux : un seul verrou, partage par la migration et le plafond.
static VERROU_ENV: Mutex<()> = Mutex::new(());

fn temp_dir() -> PathBuf {
    let n = COMPTEUR.fetch_add(1, Ordering::SeqCst);
    let dossier = std::env::temp_dir().join(format!(
        "apivault-lot6-{}-{}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&dossier);
    std::fs::create_dir_all(&dossier).unwrap();
    dossier
}

/// Fichier de cles en clair, avec des secrets manifestement fictifs.
fn ecrire_cles_en_clair(dossier: &Path) -> PathBuf {
    std::fs::create_dir_all(dossier).unwrap();
    let chemin = dossier.join("cles.env");
    std::fs::write(
        &chemin,
        "PROXY_MASTER_KEY=sk-FICTIF-MAITRE-42\nDEEPSEEK_KEY=sk-FICTIF-DEEPSEEK-7\n",
    )
    .unwrap();
    chemin
}

// ---------------------------------------------------------------------------
// Partie A — migration d'un fichier de cles en clair au demarrage
// ---------------------------------------------------------------------------

#[test]
fn fichier_interne_en_clair_migre_et_chiffre() {
    let base = temp_dir();
    let donnees = base.join("ApiVault_Proxy");
    let cles = ecrire_cles_en_clair(&donnees);

    assert_eq!(
        config::migrer_au_demarrage(&cles, &donnees),
        config::Migration::Chiffre
    );

    let brut = std::fs::read_to_string(&cles).unwrap();
    assert!(brut.starts_with(coffre::EN_TETE));
    assert!(!brut.contains("sk-FICTIF-MAITRE-42"));
    assert!(!brut.contains("sk-FICTIF-DEEPSEEK-7"));
    assert!(!brut.contains("PROXY_MASTER_KEY"));

    // Le contenu reste lisible pour l'application : rien n'a ete perdu.
    let clair = coffre::lire_texte(&cles).unwrap();
    assert!(clair.contains("PROXY_MASTER_KEY=sk-FICTIF-MAITRE-42"));
    assert!(clair.contains("DEEPSEEK_KEY=sk-FICTIF-DEEPSEEK-7"));

    // Deuxieme demarrage : plus rien a faire, le fichier n'est pas reecrit.
    let avant = std::fs::read(&cles).unwrap();
    assert_eq!(
        config::migrer_au_demarrage(&cles, &donnees),
        config::Migration::DejaChiffre
    );
    assert_eq!(std::fs::read(&cles).unwrap(), avant);
}

#[test]
fn fichier_externe_jamais_reecrit() {
    let base = temp_dir();
    let donnees = base.join("ApiVault_Proxy");
    std::fs::create_dir_all(&donnees).unwrap();
    let cles = ecrire_cles_en_clair(&base.join("ailleurs"));
    let avant = std::fs::read(&cles).unwrap();

    assert_eq!(
        config::migrer_au_demarrage(&cles, &donnees),
        config::Migration::Externe
    );

    assert_eq!(std::fs::read(&cles).unwrap(), avant);
    let texte = String::from_utf8(std::fs::read(&cles).unwrap()).unwrap();
    assert!(texte.contains("sk-FICTIF-MAITRE-42"));
    assert!(!texte.starts_with(coffre::EN_TETE));
}

#[test]
fn echec_decriture_laisse_le_fichier_dorigine_intact() {
    let base = temp_dir();
    let donnees = base.join("ApiVault_Proxy");
    let cles = ecrire_cles_en_clair(&donnees);
    let avant = std::fs::read(&cles).unwrap();

    // Un dossier porte le nom du fichier temporaire : l'ecriture echoue avant
    // de toucher a l'original, comme un disque plein ou un fichier verrouille.
    let temporaire = PathBuf::from(format!("{}.tmp", cles.display()));
    std::fs::create_dir(&temporaire).unwrap();

    match config::migrer_au_demarrage(&cles, &donnees) {
        config::Migration::Echec(_) => {}
        autre => panic!("migration inattendue : {:?}", autre),
    }

    assert!(cles.exists());
    assert_eq!(std::fs::read(&cles).unwrap(), avant);
    let texte = String::from_utf8(std::fs::read(&cles).unwrap()).unwrap();
    assert!(texte.contains("sk-FICTIF-MAITRE-42"));
    assert!(!texte.starts_with(coffre::EN_TETE));
}

#[test]
fn migration_apres_chargement_de_la_configuration() {
    let _garde = VERROU_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let base = temp_dir();
    let donnees = base.join("ApiVault_Proxy");
    let cles = ecrire_cles_en_clair(&donnees);

    let ancien_appdata = std::env::var_os("APPDATA");
    let ancien_cles = std::env::var_os("APIVAULT_CLE_ENV");
    std::env::set_var("APPDATA", &base);
    std::env::set_var("APIVAULT_CLE_ENV", &cles);

    let (configuration, migration) = config::charger_et_migrer();

    match ancien_appdata {
        Some(v) => std::env::set_var("APPDATA", v),
        None => std::env::remove_var("APPDATA"),
    }
    match ancien_cles {
        Some(v) => std::env::set_var("APIVAULT_CLE_ENV", v),
        None => std::env::remove_var("APIVAULT_CLE_ENV"),
    }

    assert_eq!(migration, config::Migration::Chiffre);
    assert_eq!(configuration.chemin_cles, cles);
    // Les cles sont deja en memoire, le disque est mis au propre apres coup.
    assert_eq!(configuration.jeton, "sk-FICTIF-MAITRE-42");
    let brut = std::fs::read_to_string(&cles).unwrap();
    assert!(brut.starts_with(coffre::EN_TETE));
    assert!(!brut.contains("sk-FICTIF"));
}

// ---------------------------------------------------------------------------
// Partie B — plafond mensuel
// ---------------------------------------------------------------------------

mod plafond {
    use apivault_proxy::coffre;
    use apivault_proxy::config;
    use apivault_proxy::config::Config;
    use apivault_proxy::metriques;
    use apivault_proxy::serveur::{AppState, Service};
    use serde_json::Value;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const JETON: &str = "sk-FICTIF-MAITRE-42";

    fn port_libre() -> u16 {
        let ecoute = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = ecoute.local_addr().unwrap().port();
        drop(ecoute);
        port
    }

    /// Amont factice qui compte ses appels et repond indefiniment.
    ///
    /// Les fils se terminent avec le processus de test ; le compteur partage dit
    /// exactement combien de requetes ont atteint le fournisseur.
    fn amont_compte(
        corps: &'static str,
        type_contenu: &'static str,
        statut: u16,
    ) -> (u16, Arc<AtomicUsize>) {
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

    const REPONSE_USAGE: &str = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"bonjour"}}],"usage":{"prompt_tokens":128,"completion_tokens":64,"prompt_cache_hit_tokens":32}}"#;
    const REPONSE_FLUX: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"bon\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":128,\"completion_tokens\":64}}\n\n",
        "data: [DONE]\n\n"
    );

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

    /// Prepare un `cles.env` temporaire (plafond inclus) sans demarrer le service.
    fn preparer(nom: &str, port: u16, url_amont: u16, plafond: u64) -> Banc {
        let dossier = std::env::temp_dir().join(format!(
            "apivault-lot6-{}-{}",
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
                "PROXY_PORT={}\nPROXY_MASTER_KEY={}\nPROXY_PLAFOND_MENSUEL={}\nDEEPSEEK_KEY=test\nDEEPSEEK_URLS={}\nOPENROUTER_KEY=test\nOPENROUTER_URLS={}\n",
                port, JETON, plafond, amont, amont
            ),
        )
        .unwrap();
        std::env::set_var("APIVAULT_CLE_ENV", &chemin);
        let appdata_original = std::env::var("APPDATA").ok();
        std::env::set_var("APPDATA", &dossier);
        let cfg = Config::charger();
        assert_eq!(cfg.port, port);
        assert_eq!(cfg.plafond_mensuel, plafond, "le plafond doit etre lu");
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

    async fn demander(client: &reqwest::Client, banc: &Banc, flux: bool) -> reqwest::Response {
        client
            .post(banc.url("/v1/chat/completions"))
            .header("authorization", format!("Bearer {}", JETON))
            .json(&serde_json::json!({
                "model": "defaut",
                "stream": flux,
                "messages": [{ "role": "user", "content": "bonjour" }]
            }))
            .send()
            .await
            .expect("requete relayee")
    }

    fn journal_texte(dossier: &Path) -> String {
        fs::read_to_string(dossier.join("journal.log")).unwrap_or_default()
    }

    fn ligne_metriques(horodatage: &str, entrees: u64, sorties: u64) -> String {
        format!(
            r#"{{"id":"0000000a","horodatage":"{}","jeton":"maitre","alias":"defaut","fournisseur":"deepseek","modele":"deepseek-chat","statut":200,"latence_ms":1,"tokens_in":{},"tokens_out":{},"tokens_cache":0,"cout_eur":null,"estime":false,"tentative":1,"flux":false}}"#,
            horodatage, entrees, sorties
        )
    }

    fn mois_precedent(periode: &str) -> String {
        let annee: i32 = periode[..4].parse().unwrap();
        let mois: i32 = periode[5..].parse().unwrap();
        if mois == 1 {
            format!("{:04}-12", annee - 1)
        } else {
            format!("{:04}-{:02}", annee, mois - 1)
        }
    }

    #[tokio::test]
    async fn plafond_desactive_laisse_passer() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        let banc = preparer("desactive", port, port_amont, 0);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        assert_eq!(compte.load(Ordering::SeqCst), 2);

        let totaux: Value = client
            .get(banc.url("/admin/metriques"))
            .header("authorization", format!("Bearer {}", JETON))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(totaux["plafond"], 0);
        assert_eq!(totaux["utilise"], 384);
        assert_eq!(totaux["niveau"], "desactive");
    }

    #[tokio::test]
    async fn plafond_tout_juste_en_dessous_laisse_passer() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        // 192 consommes apres la premiere requete : 193 est juste au-dessus.
        let banc = preparer("dessous", port, port_amont, 193);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        assert_eq!(
            demander(&client, &banc, false).await.status(),
            200,
            "192 < 193 : la deuxieme passe encore"
        );
        // 384 >= 193 : la troisieme est refusee, sans appeler l'amont.
        let refus = demander(&client, &banc, false).await;
        assert_eq!(refus.status(), 429);
        assert_eq!(compte.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn plafond_atteint_refuse_sans_appel_amont() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        let banc = preparer("atteint", port, port_amont, 100);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        let refus = demander(&client, &banc, false).await;
        assert_eq!(refus.status(), 429);
        let corps: Value = refus.json().await.unwrap();
        assert_eq!(corps["error"]["code"], "plafond_atteint");
        let message = corps["error"]["message"].as_str().unwrap();
        assert!(message.contains("192"), "message : {}", message);
        assert!(message.contains("100"), "message : {}", message);
        assert_eq!(
            compte.load(Ordering::SeqCst),
            1,
            "le fournisseur ne doit pas etre appele apres le plafond"
        );
        // Le refus n'ecrit pas de ligne de mesures : rien n'a ete consomme.
        let lignes = metriques::totaliser_fichier(
            &banc
                .donnees()
                .join(format!("metriques-{}.jsonl", metriques::periode_courante())),
            &metriques::periode_courante(),
        )
        .unwrap();
        assert_eq!(lignes.requetes, 1);
        assert!(journal_texte(&banc.donnees()).contains("plafond atteint"));
    }

    #[tokio::test]
    async fn plafond_flux_refuse_avant_de_commencer() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        let banc = preparer("flux-refuse", port, port_amont, 100);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        let refus = demander(&client, &banc, true).await;
        assert_eq!(refus.status(), 429);
        let corps: Value = refus.json().await.unwrap();
        assert_eq!(corps["error"]["code"], "plafond_atteint");
        assert_eq!(compte.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn plafond_compte_le_flux() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_FLUX, "text/event-stream", 200);
        let port = port_libre();
        let banc = preparer("flux-compte", port, port_amont, 100);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        let flux = demander(&client, &banc, true).await;
        assert_eq!(flux.status(), 200);
        let _ = flux.text().await.unwrap();
        // L'usage du flux est ecrit a la fermeture : on laisse le temps au relais.
        let mut utilise = 0;
        for _ in 0..100 {
            let totaux = metriques::totaliser_fichier(
                &banc
                    .donnees()
                    .join(format!("metriques-{}.jsonl", metriques::periode_courante())),
                &metriques::periode_courante(),
            )
            .unwrap();
            utilise = totaux.entrees + totaux.sorties;
            if utilise >= 192 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(utilise, 192, "l'usage final du flux doit etre compte");

        let refus = demander(&client, &banc, false).await;
        assert_eq!(refus.status(), 429);
        assert_eq!(compte.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn alerte_a_quatre_vingts_pour_cent() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, _compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        // 192 sur 200 : 96 %, donc alerte mais pas atteint.
        let banc = preparer("alerte", port, port_amont, 200);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        let totaux: Value = client
            .get(banc.url("/admin/metriques"))
            .header("authorization", format!("Bearer {}", JETON))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(totaux["utilise"], 192);
        assert_eq!(totaux["plafond"], 200);
        assert_eq!(totaux["pourcentage"], 96);
        assert_eq!(totaux["niveau"], "alerte");
    }

    #[tokio::test]
    async fn journal_illisible_ne_bloque_pas() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        let banc = preparer("illisible", port, port_amont, 1);
        // Un dossier a la place du fichier du mois : l'agregation echoue.
        let donnees = banc.donnees();
        fs::create_dir_all(&donnees).unwrap();
        fs::create_dir(donnees.join(format!(
            "metriques-{}.jsonl",
            metriques::periode_courante()
        )))
        .unwrap();
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        // Plafond de 1 token deja depasse si le journal etait lisible :
        // fail-open, la requete passe et le journal s'explique.
        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        assert_eq!(compte.load(Ordering::SeqCst), 1);
        assert!(journal_texte(&banc.donnees()).contains("plafond non verifie"));
    }

    #[tokio::test]
    async fn plafond_survit_au_rechargement() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!(
            "apivault-lot6-persistance-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let chemin = base.join("cles.env");
        fs::write(&chemin, "PROXY_MASTER_KEY=sk-FICTIF-MAITRE-42\n").unwrap();

        assert_eq!(config::plafond_depuis_fichier(&chemin), 0, "desactive par defaut");
        config::sauver_plafond(&chemin, 300).unwrap();
        assert_eq!(config::plafond_depuis_fichier(&chemin), 300);

        let brut = fs::read_to_string(&chemin).unwrap();
        assert!(brut.starts_with(coffre::EN_TETE), "le reglage est chiffre");
        assert!(!brut.contains("PROXY_PLAFOND_MENSUEL"));

        // Relecture complete, comme un redemarrage.
        let valeurs = coffre::lire_texte(&chemin).unwrap();
        assert!(valeurs.contains("PROXY_PLAFOND_MENSUEL=300"));
        assert!(valeurs.contains("PROXY_MASTER_KEY=sk-FICTIF-MAITRE-42"));
        let _ = fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn mois_precedent_nest_pas_compte() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        let banc = preparer("mois-precedent", port, port_amont, 100);
        let donnees = banc.donnees();
        fs::create_dir_all(&donnees).unwrap();
        let precedent = mois_precedent(&metriques::periode_courante());
        fs::write(
            donnees.join(format!("metriques-{}.jsonl", precedent)),
            format!("{}\n", ligne_metriques("2026-09-01T10:00:00+02:00", 99999, 99999)),
        )
        .unwrap();
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        // Le mois precedent est enorme : s'il comptait, la requete serait refusee.
        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        assert_eq!(compte.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn health_et_admin_ne_sont_jamais_bloques() {
        let _garde = super::VERROU_ENV
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (port_amont, _compte) = amont_compte(REPONSE_USAGE, "application/json", 200);
        let port = port_libre();
        let banc = preparer("health", port, port_amont, 1);
        banc.service.demarrer();
        let client = client();
        attendre_sante(&client, port).await;

        assert_eq!(demander(&client, &banc, false).await.status(), 200);
        // Plafond de 1 largement depasse : seul /v1 est refuse.
        assert_eq!(demander(&client, &banc, false).await.status(), 429);
        assert_eq!(
            client
                .get(banc.url("/health"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        let pool = client
            .get(banc.url("/admin/pool"))
            .header("authorization", format!("Bearer {}", JETON))
            .send()
            .await
            .unwrap();
        assert_eq!(pool.status(), 200);
    }
}
