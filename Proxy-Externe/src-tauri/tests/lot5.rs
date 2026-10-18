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
