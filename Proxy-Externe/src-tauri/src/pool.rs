//! Etat du pool de cibles.
//!
//! Chaque couple fournisseur/modele porte un etat :
//!   sain      - a repondu recemment
//!   inconnu   - jamais teste ou requalifie manuellement
//!   en_pause  - rate limite ou erreur passagere, retour automatique
//!   en_panne  - cle refusee, configuration a corriger
//!   epuise    - credits termines
//!
//! Aucune cible n'est jamais supprimee : on change son etat.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{dossier_donnees, Alias, Cible};

pub const PAUSE_MAX_S: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statut {
    Sain,
    Inconnu,
    EnPause,
    EnPanne,
    Epuise,
}

impl Statut {
    pub fn nom(&self) -> &'static str {
        match self {
            Statut::Sain => "sain",
            Statut::Inconnu => "inconnu",
            Statut::EnPause => "en_pause",
            Statut::EnPanne => "en_panne",
            Statut::Epuise => "epuise",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Etat {
    pub statut: Statut,
    /// Fin de pause, en millisecondes depuis l'epoque.
    pub jusqu_a: Option<u64>,
    pub echecs: u32,
    pub ok: u64,
    pub ko: u64,
    /// Latence lisse (moyenne mobile), en millisecondes.
    pub latence_ms: Option<u64>,
    pub erreur: Option<String>,
    pub derniere_utilisation: Option<u64>,
}

impl Default for Etat {
    fn default() -> Self {
        Etat {
            statut: Statut::Inconnu,
            jusqu_a: None,
            echecs: 0,
            ok: 0,
            ko: 0,
            latence_ms: None,
            erreur: None,
            derniere_utilisation: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Fichier {
    version: u32,
    candidats: HashMap<String, Etat>,
}

pub struct Pool {
    etats: Mutex<HashMap<String, Etat>>,
    chemin: Option<PathBuf>,
}

impl Pool {
    pub fn charger() -> Pool {
        let chemin = Some(dossier_donnees().join("etat-pool.json"));
        let etats = chemin
            .as_ref()
            .and_then(|c| std::fs::read_to_string(c).ok())
            .and_then(|texte| serde_json::from_str::<Fichier>(&texte).ok())
            .map(|f| f.candidats)
            .unwrap_or_default();

        Pool {
            etats: Mutex::new(etats),
            chemin,
        }
    }

    fn cle(cible: &Cible) -> String {
        format!("{}/{}", cible.fournisseur, cible.modele)
    }

    fn maintenant() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Etat courant, en faisant expirer les pauses arrivees a terme.
    fn etat_courant(&self, cible: &Cible) -> Etat {
        let Ok(mut map) = self.etats.lock() else {
            return Etat::default();
        };
        let etat = map.entry(Self::cle(cible)).or_default();
        if etat.statut == Statut::EnPause {
            if let Some(fin) = etat.jusqu_a {
                if Self::maintenant() >= fin {
                    etat.statut = Statut::Inconnu;
                    etat.jusqu_a = None;
                }
            }
        }
        etat.clone()
    }

    /// Ordre d'essai pour un alias : les cibles saines, puis les inconnues,
    /// en respectant l'ordre de l'alias (donc la priorite choisie dans
    /// l'interface). Une pause expiree redevient inconnue. Les cibles en
    /// panne ou epuisees sont ecartees.
    pub fn ordre(&self, alias: &Alias) -> Vec<Cible> {
        let mut sains: Vec<Cible> = Vec::new();
        let mut inconnus: Vec<Cible> = Vec::new();

        for cible in &alias.cibles {
            let etat = self.etat_courant(cible);
            match etat.statut {
                Statut::Sain => sains.push(cible.clone()),
                Statut::Inconnu => inconnus.push(cible.clone()),
                _ => {}
            }
        }

        let mut resultat = sains;
        resultat.extend(inconnus);
        resultat
    }

    pub fn enregistrer_succes(&self, cible: &Cible, latence_ms: u64) {
        let Ok(mut map) = self.etats.lock() else { return };
        let etat = map.entry(Self::cle(cible)).or_default();
        etat.statut = Statut::Sain;
        etat.jusqu_a = None;
        etat.echecs = 0;
        etat.ok += 1;
        etat.erreur = None;
        etat.derniere_utilisation = Some(Self::maintenant());
        etat.latence_ms = Some(match etat.latence_ms {
            Some(ancienne) => (ancienne * 3 + latence_ms) / 4,
            None => latence_ms,
        });
        drop(map);
        self.sauver();
    }

    /// `statut_http` vaut None pour une erreur reseau.
    pub fn enregistrer_echec(&self, cible: &Cible, statut_http: Option<u16>, message: &str) {
        let Ok(mut map) = self.etats.lock() else { return };
        let etat = map.entry(Self::cle(cible)).or_default();
        let maintenant = Self::maintenant();

        etat.ko += 1;
        etat.echecs = etat.echecs.saturating_add(1);
        etat.erreur = Some(tronquer(message, 300));
        etat.derniere_utilisation = Some(maintenant);

        match statut_http {
            Some(401) | Some(403) => {
                etat.statut = Statut::EnPanne;
                etat.jusqu_a = None;
            }
            Some(402) => {
                etat.statut = Statut::Epuise;
                etat.jusqu_a = None;
            }
            Some(404) => {
                // Identifiant de modele errone : correction manuelle requise.
                etat.statut = Statut::EnPanne;
                etat.jusqu_a = None;
                etat.erreur = Some("modele introuvable (404)".to_string());
            }
            Some(429) => {
                etat.statut = Statut::EnPause;
                etat.jusqu_a = Some(maintenant + Self::pause_ms(etat.echecs));
            }
            Some(code) if code >= 500 => {
                etat.statut = Statut::EnPause;
                etat.jusqu_a = Some(maintenant + 60_000);
            }
            None => {
                etat.statut = Statut::EnPause;
                etat.jusqu_a = Some(maintenant + 60_000);
            }
            // 400/422 : la requete est en cause, pas la cible.
            _ => {}
        }

        drop(map);
        self.sauver();
    }

    fn pause_ms(echecs: u32) -> u64 {
        let facteur = 1u64 << echecs.saturating_sub(1).min(4);
        (30_000 * facteur).min(PAUSE_MAX_S * 1000)
    }

    pub fn requalifier_tout(&self) {
        if let Ok(mut map) = self.etats.lock() {
            for etat in map.values_mut() {
                etat.statut = Statut::Inconnu;
                etat.jusqu_a = None;
                etat.echecs = 0;
            }
        }
        self.sauver();
    }

    pub fn requalifier(&self, fournisseur: &str, modele: &str) -> bool {
        let cle = format!("{}/{}", fournisseur, modele);
        let trouve = if let Ok(mut map) = self.etats.lock() {
            match map.get_mut(&cle) {
                Some(etat) => {
                    etat.statut = Statut::Inconnu;
                    etat.jusqu_a = None;
                    etat.echecs = 0;
                    true
                }
                None => false,
            }
        } else {
            false
        };
        self.sauver();
        trouve
    }

    /// Fait revenir en jeu les pauses terminees. Retourne le nombre de cibles
    /// concernees.
    pub fn reconcilier(&self) -> usize {
        let maintenant = Self::maintenant();
        let mut revenus = 0;
        if let Ok(mut map) = self.etats.lock() {
            for etat in map.values_mut() {
                if etat.statut == Statut::EnPause {
                    if let Some(fin) = etat.jusqu_a {
                        if maintenant >= fin {
                            etat.statut = Statut::Inconnu;
                            etat.jusqu_a = None;
                            revenus += 1;
                        }
                    }
                }
            }
        }
        if revenus > 0 {
            self.sauver();
        }
        revenus
    }

    pub fn compter_sains(&self) -> usize {
        let Ok(map) = self.etats.lock() else { return 0 };
        map.values().filter(|e| e.statut == Statut::Sain).count()
    }

    pub fn a_un_sain(&self) -> bool {
        self.compter_sains() > 0
    }

    /// Age de la reussite la plus recente, en secondes.
    pub fn derniere_reussite_s(&self) -> Option<u64> {
        let Ok(map) = self.etats.lock() else { return None };
        let maintenant = Self::maintenant();
        map.values()
            .filter(|e| e.statut == Statut::Sain)
            .filter_map(|e| e.derniere_utilisation)
            .map(|t| maintenant.saturating_sub(t) / 1000)
            .min()
    }

    /// Age de la verification la plus recente (succes ou echec), en secondes.
    /// Sert a afficher « etat repris, verifie il y a X » au demarrage.
    pub fn derniere_verification_s(&self) -> Option<u64> {
        let Ok(map) = self.etats.lock() else { return None };
        let maintenant = Self::maintenant();
        map.values()
            .filter_map(|e| e.derniere_utilisation)
            .map(|t| maintenant.saturating_sub(t) / 1000)
            .min()
    }

    /// Vue destinee a l'interface et au diagnostic.
    pub fn vue(&self, alias: &Alias) -> Vec<Value> {
        let maintenant = Self::maintenant();
        alias
            .cibles
            .iter()
            .map(|cible| {
                let etat = self.etat_courant(cible);
                json!({
                    "cible": Self::cle(cible),
                    "fournisseur": cible.fournisseur,
                    "modele": cible.modele,
                    "statut": etat.statut.nom(),
                    "jusqu_a_s": etat.jusqu_a.map(|fin| fin.saturating_sub(maintenant) / 1000),
                    "echecs": etat.echecs,
                    "ok": etat.ok,
                    "ko": etat.ko,
                    "latence_ms": etat.latence_ms,
                    "erreur": etat.erreur,
                    "derniere_utilisation_s": etat.derniere_utilisation.map(|t| maintenant.saturating_sub(t) / 1000),
                })
            })
            .collect()
    }

    /// Resume lisible de l'etat des cibles, pour les messages d'erreur.
    pub fn resume(&self, alias: &Alias) -> String {
        alias
            .cibles
            .iter()
            .map(|cible| {
                let etat = self.etat_courant(cible);
                let complement = match etat.statut {
                    Statut::EnPause => etat
                        .jusqu_a
                        .map(|fin| {
                            let reste = fin.saturating_sub(Self::maintenant()) / 1000;
                            format!(" (encore {} s)", reste)
                        })
                        .unwrap_or_default(),
                    _ => etat
                        .erreur
                        .as_ref()
                        .map(|e| format!(" ({})", tronquer(e, 80)))
                        .unwrap_or_default(),
                };
                format!("{}/{}={}{}", cible.fournisseur, cible.modele, etat.statut.nom(), complement)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn sauver(&self) {
        let Some(chemin) = &self.chemin else { return };
        let Ok(map) = self.etats.lock() else { return };
        let fichier = Fichier {
            version: 1,
            candidats: map.clone(),
        };
        drop(map);

        if let Ok(texte) = serde_json::to_string_pretty(&fichier) {
            let temporaire = chemin.with_extension("json.tmp");
            if std::fs::write(&temporaire, texte).is_ok() {
                let _ = std::fs::rename(&temporaire, chemin);
            }
        }
    }
}

fn tronquer(texte: &str, max: usize) -> String {
    let propre: String = texte.chars().take(max).collect();
    if texte.chars().count() > max {
        format!("{}…", propre)
    } else {
        propre
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cible(modele: &str) -> Cible {
        Cible {
            fournisseur: "openrouter".to_string(),
            modele: modele.to_string(),
        }
    }

    fn etat(statut: Statut, latence_ms: u64) -> Etat {
        Etat {
            statut,
            latence_ms: Some(latence_ms),
            ..Etat::default()
        }
    }

    #[test]
    fn ordre_respecte_la_priorite_de_l_alias() {
        let premier = cible("modele-lent-mais-prioritaire");
        let second = cible("modele-rapide");
        let mut etats = HashMap::new();
        etats.insert(Pool::cle(&premier), etat(Statut::Sain, 900));
        etats.insert(Pool::cle(&second), etat(Statut::Sain, 50));
        let pool = Pool {
            etats: Mutex::new(etats),
            chemin: None,
        };
        let alias = Alias {
            nom: "defaut".to_string(),
            cibles: vec![premier.clone(), second.clone()],
        };

        let ordre = pool.ordre(&alias);
        assert_eq!(ordre[0].modele, premier.modele);
        assert_eq!(ordre[1].modele, second.modele);
    }

    #[test]
    fn ordre_ecarte_les_cibles_en_pause() {
        let bon = cible("bon");
        let pause = cible("pause");
        let mut etats = HashMap::new();
        etats.insert(Pool::cle(&bon), etat(Statut::Inconnu, 0));
        let mut en_pause = etat(Statut::EnPause, 0);
        en_pause.jusqu_a = Some(Pool::maintenant() + 60_000);
        etats.insert(Pool::cle(&pause), en_pause);
        let pool = Pool {
            etats: Mutex::new(etats),
            chemin: None,
        };
        let alias = Alias {
            nom: "gratuit".to_string(),
            cibles: vec![pause, bon.clone()],
        };

        let ordre = pool.ordre(&alias);
        assert_eq!(ordre.len(), 1);
        assert_eq!(ordre[0].modele, bon.modele);
    }
}
