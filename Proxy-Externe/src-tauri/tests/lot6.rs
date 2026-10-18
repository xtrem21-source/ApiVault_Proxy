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
// parallele entre eux : un seul verrou pour tout le fichier.
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
