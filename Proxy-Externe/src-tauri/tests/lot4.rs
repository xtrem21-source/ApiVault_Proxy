//! LOT 4 - le coffre : verification que les cles ne restent jamais en clair.
//!
//! Ces tests pilotent le module `coffre` directement. Le cablage dans
//! `config::ecrire_variable` est couvert dans un second temps.

use std::fs;
use std::path::PathBuf;

use apivault_proxy::coffre;

/// Dossier de travail propre a chaque test.
struct Banc {
    dossier: PathBuf,
}

impl Banc {
    fn nouveau(nom: &str) -> Banc {
        let dossier = std::env::temp_dir().join(format!(
            "apivault-lot4-{}-{}",
            nom,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dossier).expect("creation du dossier de test");
        Banc { dossier }
    }

    fn chemin(&self, nom: &str) -> PathBuf {
        self.dossier.join(nom)
    }
}

impl Drop for Banc {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dossier);
    }
}

#[test]
fn aller_retour_chiffrement() {
    let clair = "PROXY_PORT=3100\r\nDEEPSEEK_KEY=sk-lot4-secret-42\r\n";

    let scelle = coffre::chiffrer(clair).expect("chiffrement");
    assert!(coffre::est_chiffre(&scelle), "l'en-tete doit etre present");
    assert!(
        scelle.starts_with(coffre::EN_TETE),
        "premiere ligne attendue {}, obtenu {:?}",
        coffre::EN_TETE,
        scelle.lines().next()
    );
    assert_eq!(
        coffre::dechiffrer(&scelle).expect("dechiffrement"),
        clair,
        "l'aller-retour doit rendre le texte exact"
    );

    // DPAPI ajoute un sel aleatoire : deux chiffrements du meme texte diffserent.
    let autre = coffre::chiffrer(clair).expect("chiffrement");
    assert_ne!(scelle, autre, "deux chiffrements identiques : sel absent ?");
}

#[test]
fn lecture_d_un_fichier_en_clair_existant() {
    let banc = Banc::nouveau("migration");
    let fichier = banc.chemin("cles.env");
    let historique = "# anciennes cles\r\nPROXY_PORT=3100\r\nDEEPSEEK_KEY=sk-ancienne-99\r\n";
    fs::write(&fichier, historique).expect("ecriture du fichier historique");

    assert!(
        !coffre::est_chiffre(historique),
        "le fichier historique ne doit pas porter l'en-tete"
    );
    assert_eq!(
        coffre::lire_texte(&fichier).expect("lecture du fichier en clair"),
        historique,
        "un fichier en clair doit rester lisible tel quel (migration)"
    );
}

#[test]
fn le_fichier_ecrit_ne_contient_pas_la_cle_en_clair() {
    let banc = Banc::nouveau("clair");
    let fichier = banc.chemin("cles.env");
    let secret = "sk-lot4-secret-42";

    coffre::ecrire_atomique(&fichier, &format!("DEEPSEEK_KEY={}\r\n", secret))
        .expect("ecriture chiffree");

    let sur_disque = fs::read_to_string(&fichier).expect("lecture brute");
    assert!(
        sur_disque.starts_with(coffre::EN_TETE),
        "le fichier ecrit doit commencer par l'en-tete du coffre"
    );
    assert!(
        !sur_disque.contains(secret),
        "la cle ne doit jamais apparaitre en clair sur le disque"
    );
    assert!(
        !sur_disque.contains("DEEPSEEK_KEY"),
        "le nom de la variable ne doit pas non plus etre en clair"
    );
    assert!(
        !banc.chemin("cles.env.tmp").exists(),
        "le fichier temporaire doit avoir ete renomme"
    );
    assert_eq!(
        coffre::lire_texte(&fichier).expect("relecture"),
        format!("DEEPSEEK_KEY={}\r\n", secret)
    );
}

#[test]
fn en_tete_corrompu_donne_une_erreur_claire() {
    let banc = Banc::nouveau("corrompu");
    let fichier = banc.chemin("cles.env");

    // En-tete present, charge qui n'est pas du base64.
    fs::write(&fichier, format!("{}\nceci-n-est-pas-du-base64!!\n", coffre::EN_TETE))
        .expect("ecriture du coffre corrompu");

    let erreur = coffre::lire_texte(&fichier).expect_err("un coffre corrompu doit echouer");
    assert_eq!(
        erreur.kind(),
        std::io::ErrorKind::InvalidData,
        "l'erreur doit etre identifiee comme donnee invalide"
    );
    let message = erreur.to_string();
    assert!(
        message.contains("corrompu"),
        "le message doit dire que le coffre est corrompu, obtenu : {}",
        message
    );
    // Le fichier n'est pas detruit : on doit pouvoir le mettre de cote a la main.
    assert!(fichier.exists(), "le fichier corrompu ne doit pas etre efface");

    // En-tete present, base64 valide mais pas un blob DPAPI.
    fs::write(&fichier, format!("{}\nQUJDRA==\n", coffre::EN_TETE))
        .expect("ecriture du faux blob");
    let erreur = coffre::lire_texte(&fichier).expect_err("un faux blob doit echouer");
    assert!(
        erreur.to_string().contains("refusee par Windows"),
        "DPAPI doit refuser un blob etranger, obtenu : {}",
        erreur
    );
}
