//! LOT 4 - le coffre : verification que les cles ne restent jamais en clair.
//!
//! Ces tests pilotent le module `coffre` directement. Le cablage dans
//! `config::ecrire_variable` est couvert dans un second temps.

use std::fs;
use std::path::PathBuf;

use apivault_proxy::coffre;
use apivault_proxy::config;

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

#[test]
fn ecrire_variable_ne_laisse_aucune_cle_en_clair() {
    let banc = Banc::nouveau("config-clair");
    let fichier = banc.chemin("cles.env");
    let secret = "sk-lot4-secret-42";

    config::ecrire_variable(&fichier, "DEEPSEEK_KEY", secret).expect("ecriture de la cle");

    let sur_disque = fs::read_to_string(&fichier).expect("lecture brute");
    assert!(
        sur_disque.starts_with(coffre::EN_TETE),
        "le fichier de configuration doit etre chiffre"
    );
    assert!(
        !sur_disque.contains(secret),
        "la cle ne doit pas apparaitre en clair dans cles.env"
    );
    assert_eq!(
        coffre::lire_texte(&fichier).expect("relecture"),
        format!("DEEPSEEK_KEY={}\r\n", secret)
    );
}

#[test]
fn la_sauvegarde_bak_est_chiffree() {
    let banc = Banc::nouveau("bak");
    let fichier = banc.chemin("cles.env");
    let sauvegarde = banc.chemin("cles.env.bak");

    config::ecrire_variable(&fichier, "PROXY_PORT", "3100").expect("premier ecrit");
    config::ecrire_variable(&fichier, "PROXY_PORT", "3200").expect("second ecrit");

    assert!(sauvegarde.exists(), "la sauvegarde doit exister apres ecriture");
    let brut = fs::read_to_string(&sauvegarde).expect("lecture brute de la sauvegarde");
    assert!(
        brut.starts_with(coffre::EN_TETE),
        "la sauvegarde doit etre chiffree, obtenu : {:?}",
        brut.lines().next()
    );
    assert!(
        !brut.contains("PROXY_PORT=3100"),
        "la sauvegarde ne doit pas contenir l'ancienne valeur en clair"
    );
    assert_eq!(
        coffre::lire_texte(&sauvegarde).expect("relecture de la sauvegarde"),
        "PROXY_PORT=3100\r\n",
        "la sauvegarde doit rester lisible (une generation)"
    );
}

#[test]
fn la_sauvegarde_est_supprimee_apres_effacement_d_une_cle() {
    let banc = Banc::nouveau("effacement");
    let fichier = banc.chemin("cles.env");
    let sauvegarde = banc.chemin("cles.env.bak");

    config::ecrire_variable(&fichier, "DEEPSEEK_KEY", "sk-lot4-secret-42").expect("ajout de la cle");
    config::ecrire_variable(&fichier, "PROXY_PORT", "3100").expect("reglage du port");
    assert!(sauvegarde.exists(), "la sauvegarde doit exister avant l'effacement");

    config::ecrire_variable(&fichier, "DEEPSEEK_KEY", "").expect("effacement de la cle");

    assert!(
        !sauvegarde.exists(),
        "la sauvegarde doit disparaitre quand une cle est effacee"
    );
    let clair = coffre::lire_texte(&fichier).expect("relecture");
    assert!(
        !clair.contains("sk-lot4-secret-42"),
        "la cle effacee ne doit plus trainer dans le fichier"
    );
}

#[test]
fn la_rotation_d_une_cle_supprime_la_sauvegarde() {
    let banc = Banc::nouveau("rotation");
    let fichier = banc.chemin("cles.env");
    let sauvegarde = banc.chemin("cles.env.bak");

    config::ecrire_variable(&fichier, "DEEPSEEK_KEY", "sk-lot4-secret-42").expect("ajout");
    config::ecrire_variable(&fichier, "PROXY_PORT", "3100").expect("reglage du port");
    assert!(sauvegarde.exists(), "sauvegarde attendue avant la rotation");

    config::ecrire_variable(&fichier, "DEEPSEEK_KEY", "sk-lot4-secret-43").expect("rotation");

    assert!(
        !sauvegarde.exists(),
        "une sauvegarde qui contient l'ancienne cle ne doit pas survivre a la rotation"
    );
    let clair = coffre::lire_texte(&fichier).expect("relecture");
    assert!(clair.contains("sk-lot4-secret-43"), "la nouvelle cle doit etre stockee");
    assert!(!clair.contains("sk-lot4-secret-42"), "l'ancienne cle doit avoir disparu");
}

#[test]
fn migration_d_un_fichier_en_clair_au_premier_enregistrement() {
    let banc = Banc::nouveau("migration-ecriture");
    let fichier = banc.chemin("cles.env");
    let historique = "# anciennes cles\r\nPROXY_PORT=3100\r\nDEEPSEEK_KEY=sk-ancienne-99\r\n";
    fs::write(&fichier, historique).expect("ecriture du fichier historique");
    assert!(!coffre::est_chiffre(historique));

    config::ecrire_variable(&fichier, "MISTRAL_KEY", "sk-lot4-secret-42")
        .expect("premier enregistrement apres migration");

    let sur_disque = fs::read_to_string(&fichier).expect("lecture brute");
    assert!(
        sur_disque.starts_with(coffre::EN_TETE),
        "l'ancien fichier en clair doit avoir ete remplace sur place"
    );
    assert!(
        !sur_disque.contains("sk-ancienne-99"),
        "l'ancienne cle en clair ne doit plus etre sur le disque"
    );
    let clair = coffre::lire_texte(&fichier).expect("relecture");
    assert!(clair.contains("PROXY_PORT=3100"), "les autres lignes sont preservees");
    assert!(clair.contains("MISTRAL_KEY=sk-lot4-secret-42"), "la nouvelle cle est ajoutee");
}

#[test]
fn restriction_des_droits_au_compte_courant() {
    let banc = Banc::nouveau("droits");
    let fichier = banc.chemin("cles.env");
    fs::write(&fichier, "PROXY_PORT=3100\r\n").expect("fichier de depart");

    let compte = coffre::restreindre_droits(&banc.dossier).expect("icacls doit reussir");
    println!("compte autorise : {}", compte);

    let sortie = std::process::Command::new("icacls")
        .arg(&banc.dossier)
        .output()
        .expect("verification par icacls");
    let texte = String::from_utf8_lossy(&sortie.stdout).to_string();
    println!("ACL obtenue :\n{}", texte);

    let utilisateur = std::env::var("USERNAME").expect("USERNAME");
    assert!(
        texte.to_lowercase().contains(&utilisateur.to_lowercase()),
        "le compte courant doit apparaitre dans l'ACL : {}",
        texte
    );
    assert!(
        texte.contains("(OI)(CI)(F)"),
        "le compte courant doit avoir le controle total en propagation : {}",
        texte
    );
    assert!(
        !texte.contains("(I)(") && !texte.contains("(I))"),
        "l'heritage doit avoir ete retire : {}",
        texte
    );
}

/// Artefact volontaire : ecrit un vrai fichier hors du dossier ephemere pour
/// qu'on puisse le montrer tel qu'il est sur le disque dans le rapport.
#[test]
fn preuve_disque_sur_un_fichier_reel() {
    let dossier = std::env::temp_dir().join("apivault-preuves");
    fs::create_dir_all(&dossier).expect("dossier de preuve");
    let fichier = dossier.join("cles.env");
    config::ecrire_variable(&fichier, "DEEPSEEK_KEY", "sk-lot4-secret-42")
        .expect("ecriture de la cle");
    println!("fichier de preuve : {}", fichier.display());
}

/// L'ACL d'un dossier, telle que Windows la rend.
fn acl(chemin: &std::path::Path) -> String {
    let sortie = std::process::Command::new("icacls")
        .arg(chemin)
        .output()
        .expect("icacls");
    String::from_utf8_lossy(&sortie.stdout).to_string()
}

#[test]
fn restriction_jamais_appliquee_a_un_fichier_externe() {
    let banc = Banc::nouveau("externe");
    let donnees = banc.chemin("donnees");
    let bureau = banc.chemin("Bureau");
    fs::create_dir_all(&donnees).expect("dossier de donnees");
    fs::create_dir_all(&bureau).expect("dossier externe");

    let cles_donnees = donnees.join("cles.env");
    let cles_externes = bureau.join("cles.env");

    // Fichier dans le dossier de l'application : la restriction s'applique.
    assert_eq!(
        config::dossier_a_restreindre(&cles_donnees, &donnees),
        Some(donnees.clone()),
        "un fichier de cles interne doit pouvoir etre restreint"
    );

    // Fichier designe par APIVAULT_CLE_ENV, hors du dossier : on n'y touche pas.
    assert_eq!(
        config::dossier_a_restreindre(&cles_externes, &donnees),
        None,
        "un fichier de cles externe ne doit jamais faire restreindre son dossier"
    );

    // Meme chose en passant par une remontee relative.
    let piege = donnees.join("..").join("Bureau").join("cles.env");
    assert_eq!(
        config::dossier_a_restreindre(&piege, &donnees),
        None,
        "une remontee .. ne doit pas ramener un chemin externe dans le dossier"
    );

    // Et si on applique la decision telle quelle, l'ACL du dossier externe
    // reste exactement la meme : heritage compris.
    let avant = acl(&bureau);
    if let Some(dossier) = config::dossier_a_restreindre(&cles_externes, &donnees) {
        let _ = coffre::restreindre_droits(&dossier);
    }
    let apres = acl(&bureau);
    assert_eq!(
        avant, apres,
        "les droits d'un dossier externe ne doivent pas bouger"
    );
    assert!(
        apres.contains("(I)"),
        "l'heritage doit rester en place sur un dossier externe : {}",
        apres
    );
}
