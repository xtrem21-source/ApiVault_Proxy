//! LOT 5 : compteur de tokens.
//!
//! Un enregistrement par requete HTTP servie au client, une ligne JSON par
//! enregistrement, dans `%APPDATA%\ApiVault_Proxy\metriques-AAAA-MM.jsonl`.
//!
//! FORMAT PUBLIC ET STABLE : on n'y retire ni ne renomme jamais un champ ;
//! on ne fait qu'en ajouter. Le LOT 6 (plafond de consommation) lit ces memes
//! fichiers sans migration.
//!
//! Aucune cle, aucun contenu de message n'est ecrit ici : seulement des
//! compteurs, un statut, une latence et des noms.
//!
//! L'agregation (`totaliser_fichier`) ne connait ni le reseau ni l'interface :
//! elle prend un fichier et rend des totaux. C'est volontaire, pour que le
//! LOT 6 puisse la reutiliser telle quelle.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local};
use serde::{Deserialize, Serialize};

/// Prefixe des fichiers mensuels.
pub const PREFIXE_FICHIER: &str = "metriques";

/// Une requete servie. Les noms de champs reprennent `SPECIFICATION.md` 4.3
/// (journal), avec `tokens_cache` et `flux` en plus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Enregistrement {
    /// Identifiant de la requete, le meme que l'en-tete `x-request-id`.
    pub id: String,
    /// Horodatage local RFC 3339, a la milliseconde.
    pub horodatage: String,
    /// Nom du jeton d'acces utilise (« maitre » tant que les jetons nommes
    /// n'existent pas).
    pub jeton: String,
    /// Alias demande par le client.
    pub alias: String,
    /// Fournisseur retenu (vide si aucune cible n'a repondu).
    pub fournisseur: String,
    /// Modele reellement appele (vide si aucune cible n'a repondu).
    pub modele: String,
    /// Statut HTTP rendu au client.
    pub statut: u16,
    /// Duree de la requete vue par le client.
    pub latence_ms: u64,
    /// Tokens d'entree annonces par le fournisseur.
    pub tokens_in: u64,
    /// Tokens de sortie annonces par le fournisseur.
    pub tokens_out: u64,
    /// Tokens d'entree servis depuis le cache du fournisseur.
    pub tokens_cache: u64,
    /// Cout en euros : `null` tant que le catalogue de prix n'alimente pas le
    /// calcul (le champ est present des maintenant pour ne pas le renommer).
    pub cout_eur: Option<f64>,
    /// Vrai quand le fournisseur n'a pas annonce d'usage : les compteurs de
    /// tokens ne sont alors pas fiables (0 par defaut).
    pub estime: bool,
    /// Numero de la cible qui a repondu (1 = premiere tentative).
    pub tentative: u32,
    /// Vrai si la reponse etait un flux SSE.
    pub flux: bool,
}

impl Enregistrement {
    /// Enregistrement par defaut : aucune cible n'a encore repondu.
    pub fn nouveau(id: &str, alias: &str) -> Self {
        Enregistrement {
            id: id.to_string(),
            horodatage: horodatage_maintenant(),
            jeton: "maitre".to_string(),
            alias: alias.to_string(),
            fournisseur: String::new(),
            modele: String::new(),
            statut: 0,
            latence_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache: 0,
            cout_eur: None,
            estime: true,
            tentative: 0,
            flux: false,
        }
    }
}

/// Horodatage local, a la milliseconde, avec fuseau.
pub fn horodatage_maintenant() -> String {
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// Periode `AAAA-MM` d'un instant.
pub fn periode_de(instant: &DateTime<Local>) -> String {
    format!("{:04}-{:02}", instant.year(), instant.month())
}

/// Periode `AAAA-MM` courante (mois local).
pub fn periode_courante() -> String {
    periode_de(&Local::now())
}

/// Chemin du fichier d'une periode.
pub fn chemin_periode(dossier: &Path, periode: &str) -> PathBuf {
    dossier.join(format!("{}-{}.jsonl", PREFIXE_FICHIER, periode))
}

/// Vrai si la chaine ressemble a une periode `AAAA-MM`.
pub fn periode_valide(periode: &str) -> bool {
    let octets = periode.as_bytes();
    octets.len() == 7
        && octets[4] == b'-'
        && octets[..4].iter().all(|c| c.is_ascii_digit())
        && octets[5..].iter().all(|c| c.is_ascii_digit())
        && {
            let mois: u32 = periode[5..].parse().unwrap_or(0);
            (1..=12).contains(&mois)
        }
}

/// Totaux d'un ensemble d'enregistrements.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Totaux {
    pub periode: String,
    pub requetes: u64,
    pub entrees: u64,
    pub sorties: u64,
    pub cache: u64,
    /// Nombre de requetes dont les tokens sont estimes (pas d'usage amont).
    pub estimees: u64,
    /// Lignes illisibles ignorees (fichier tronque, edition manuelle).
    pub lignes_illisibles: u64,
    pub par_alias: Vec<Detail>,
    pub par_modele: Vec<Detail>,
}

/// Detail par alias ou par modele.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Detail {
    pub nom: String,
    pub requetes: u64,
    pub entrees: u64,
    pub sorties: u64,
    pub cache: u64,
}

impl Totaux {
    fn vide(periode: &str) -> Totaux {
        Totaux {
            periode: periode.to_string(),
            ..Totaux::default()
        }
    }
}

/// Agregation pure : un fichier en entree, des totaux en sortie.
///
/// Un fichier absent rend des totaux a zero (premier mois, rien a mesurer).
/// Une ligne illisible est ignoree et comptee, jamais fatale.
pub fn totaliser_fichier(chemin: &Path, periode: &str) -> io::Result<Totaux> {
    let contenu = match std::fs::read_to_string(chemin) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Totaux::vide(periode)),
        Err(e) => return Err(e),
    };

    let mut totaux = Totaux::vide(periode);
    let mut alias: BTreeMap<String, Detail> = BTreeMap::new();
    let mut modeles: BTreeMap<String, Detail> = BTreeMap::new();

    for ligne in contenu.lines() {
        let ligne = ligne.trim();
        if ligne.is_empty() {
            continue;
        }
        let enregistrement: Enregistrement = match serde_json::from_str(ligne) {
            Ok(v) => v,
            Err(_) => {
                totaux.lignes_illisibles += 1;
                continue;
            }
        };

        totaux.requetes += 1;
        totaux.entrees += enregistrement.tokens_in;
        totaux.sorties += enregistrement.tokens_out;
        totaux.cache += enregistrement.tokens_cache;
        if enregistrement.estime {
            totaux.estimees += 1;
        }

        let cle_alias = if enregistrement.alias.is_empty() {
            "(sans alias)".to_string()
        } else {
            enregistrement.alias.clone()
        };
        let a = alias.entry(cle_alias.clone()).or_insert_with(|| Detail {
            nom: cle_alias,
            ..Detail::default()
        });
        a.requetes += 1;
        a.entrees += enregistrement.tokens_in;
        a.sorties += enregistrement.tokens_out;
        a.cache += enregistrement.tokens_cache;

        if !enregistrement.modele.is_empty() {
            let m = modeles
                .entry(enregistrement.modele.clone())
                .or_insert_with(|| Detail {
                    nom: enregistrement.modele.clone(),
                    ..Detail::default()
                });
            m.requetes += 1;
            m.entrees += enregistrement.tokens_in;
            m.sorties += enregistrement.tokens_out;
            m.cache += enregistrement.tokens_cache;
        }
    }

    totaux.par_alias = alias.into_values().collect();
    totaux.par_modele = modeles.into_values().collect();
    Ok(totaux)
}

/// Ecrit et relit le journal de mesure.
#[derive(Debug)]
pub struct Metriques {
    dossier: PathBuf,
    verrou: std::sync::Mutex<()>,
}

impl Metriques {
    pub fn nouveau(dossier: impl Into<PathBuf>) -> Metriques {
        Metriques {
            dossier: dossier.into(),
            verrou: std::sync::Mutex::new(()),
        }
    }

    pub fn dossier(&self) -> &Path {
        &self.dossier
    }

    /// Ajoute une ligne au fichier du mois de son horodatage. Le fichier est
    /// cree au besoin, jamais reecrit : un enregistrement deja ecrit reste
    /// lisible meme si l'application tombe juste apres.
    pub fn enregistrer(&self, enregistrement: &Enregistrement) -> io::Result<PathBuf> {
        let periode = match periode_de_texte(&enregistrement.horodatage) {
            Some(p) => p,
            None => periode_courante(),
        };
        let chemin = chemin_periode(&self.dossier, &periode);
        if let Some(parent) = chemin.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut ligne = serde_json::to_string(enregistrement)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        ligne.push('\n');

        // Un verrou par processus : plusieurs requetes peuvent se terminer en
        // meme temps, on ne veut pas entrelacer deux lignes.
        let _garde = self.verrou.lock().unwrap_or_else(|e| e.into_inner());
        let mut fichier = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&chemin)?;
        fichier.write_all(ligne.as_bytes())?;
        fichier.flush()?;
        Ok(chemin)
    }

    /// Totaux d'une periode `AAAA-MM`.
    pub fn totaliser(&self, periode: &str) -> io::Result<Totaux> {
        totaliser_fichier(&chemin_periode(&self.dossier, periode), periode)
    }
}

/// `AAAA-MM` a partir d'un horodatage RFC 3339 (avec fuseau).
fn periode_de_texte(horodatage: &str) -> Option<String> {
    let instant = DateTime::parse_from_rfc3339(horodatage).ok()?;
    Some(format!(
        "{:04}-{:02}",
        instant.year(),
        instant.month()
    ))
}

/// Tokens annonces par un fournisseur OpenAI/DeepSeek/Anthropic.
///
/// Rend `None` quand aucun `usage` exploitable n'est present.
pub fn usage_depuis_json(valeur: &serde_json::Value) -> Option<(u64, u64, u64)> {
    let usage = valeur.get("usage")?;
    if !usage.is_object() {
        return None;
    }

    let nombre = |objet: &serde_json::Value, cle: &str| -> u64 {
        objet.get(cle).and_then(|v| v.as_u64()).unwrap_or(0)
    };

    let entree = if usage.get("prompt_tokens").is_some() {
        nombre(usage, "prompt_tokens")
    } else {
        nombre(usage, "input_tokens")
    };
    let sortie = if usage.get("completion_tokens").is_some() {
        nombre(usage, "completion_tokens")
    } else {
        nombre(usage, "output_tokens")
    };
    let cache = if usage.get("prompt_cache_hit_tokens").is_some() {
        nombre(usage, "prompt_cache_hit_tokens")
    } else if usage.get("cache_read_input_tokens").is_some() {
        nombre(usage, "cache_read_input_tokens")
    } else {
        // OpenAI : usage.prompt_tokens_details.cached_tokens
        usage
            .get("prompt_tokens_details")
            .map(|d| nombre(d, "cached_tokens"))
            .unwrap_or(0)
    };

    Some((entree, sortie, cache))
}

/// Dernier `usage` trouve dans un flux SSE deja recu.
///
/// Le fournisseur place generalement l'usage dans le dernier evenement
/// (`stream_options.include_usage`). Une ligne partielle est ignoree : elle
/// sera vue au prochain morceau.
pub fn usage_dans_flux(texte: &str) -> Option<(u64, u64, u64)> {
    let mut trouve = None;
    for ligne in texte.lines() {
        let ligne = ligne.trim();
        let charge = match ligne.strip_prefix("data:") {
            Some(c) => c.trim(),
            None => continue,
        };
        if charge.is_empty() || charge == "[DONE]" {
            continue;
        }
        if let Ok(valeur) = serde_json::from_str::<serde_json::Value>(charge) {
            if let Some(u) = usage_depuis_json(&valeur) {
                trouve = Some(u);
            }
        }
    }
    trouve
}
