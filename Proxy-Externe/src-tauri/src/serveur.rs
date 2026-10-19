//! Passerelle locale : serveur HTTP compatible OpenAI.
//!
//! v0.1 : un alias resolu vers une ou plusieurs cibles, diffusion du flux
//! (SSE) telle quelle, aucune intelligence de pool (elle arrive en v0.2).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, Query, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::watch;

#[cfg(test)]
use crate::pool::Pool;
use crate::coffre;
use crate::metriques::Enregistrement;
use crate::config::{
    construire_alias, decouper_adresses, dossier_donnees, ecrire_variable,
    ecrire_variable_sans_sauvegarde, modele_gratuit, sauver_modele_prefere, Alias, Cible, Config,
    Fournisseur, Perso,
};
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Inactivite maximale toleree entre deux octets du flux amont.
const FLUX_INACTIVITE: Duration = Duration::from_secs(120);
/// Duree totale maximale d'un flux, en-tetes comprises.
const FLUX_DUREE_MAX: Duration = Duration::from_secs(900);
/// Fenetre totale accordee aux tentatives et aux bascules (en-tetes comprises).
/// LOT 7, M-3 : c'est un budget global, plus une coupure seche a 45 s.
const BUDGET_BASCULES: Duration = Duration::from_secs(900);
/// Duree maximale d'une requete complete, flux compris.
const BUDGET_REQUETE: Duration = Duration::from_secs(900);
/// Plafond absolu d'une tentative isolee, meme si le budget de bascule le permet.
/// LOT 7, M-3 : 120 s (premier octet compris), pour laisser sa chance a chaque
/// cible avant de basculer sur la suivante.
const DELAI_CIBLE: Duration = Duration::from_secs(120);
/// Au plus 3 cibles essayees : 1 tentative + 2 bascules.
const CIBLES_MAX: usize = 3;

/// Duree lisible dans le journal : en secondes des qu'on y est, en
/// millisecondes en dessous (les tests raccourcissent les delais).
fn duree_texte(d: Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{} s", d.as_secs())
    } else {
        format!("{} ms", d.as_millis())
    }
}

/// Delai surchargeable, en millisecondes. Les tests ne peuvent pas attendre
/// 120 s : ils posent `APIVAULT_DELAI_CIBLE_MS` / `APIVAULT_BUDGET_BASCULE_MS`.
/// Sans variable (cas normal), la valeur de production s'applique.
fn duree_reglable(nom: &str, defaut: Duration) -> Duration {
    let Ok(valeur) = std::env::var(nom) else {
        return defaut;
    };
    valeur
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(defaut)
}

/// Horloge du processus, posee au premier appel (debut de `main`).
static DEBUT_PROCESSUS: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Millisecondes ecoulees depuis le debut du processus, pour le journal de
/// demarrage (config, pool, serveur, interface, total).
pub fn ms_depuis_demarrage() -> u128 {
    DEBUT_PROCESSUS.get_or_init(Instant::now).elapsed().as_millis()
}

pub struct AppState {
    pub cfg: Config,
    pub pool: crate::pool::Pool,
    /// Copie modifiable des cles, pour que l'interface puisse les changer
    /// sans redemarrer la passerelle.
    cles: Mutex<HashMap<String, String>>,
    /// Adresses actives par fournisseur (la premiere est celle par defaut).
    adresses: Mutex<HashMap<String, Vec<String>>>,
    http: reqwest::Client,
    http_flux: reqwest::Client,
    pub debut: Instant,
    /// Une verification des fournisseurs est en cours.
    pub sondage: AtomicBool,
    /// Date de la derniere verification (manuelle ou declenchee par un echec).
    /// Sert au garde-fou qui evite de sonder en boucle.
    derniere_sonde: Mutex<Option<Instant>>,
    /// Modele choisi dans l'interface. Il influence l'ordre des alias sans
    /// redemarrer la passerelle.
    preference_modele: Mutex<Option<String>>,
    journal: Mutex<VecDeque<String>>,
    chemin_journal: Option<PathBuf>,
    chemin_debug: Option<PathBuf>,
    /// LOT 5 : journal des tokens consommes, un fichier JSONL par mois.
    pub metriques: Arc<crate::metriques::Metriques>,
    /// Plafond de requetes amont simultanees (8).
    pub semaphore: Arc<tokio::sync::Semaphore>,
    /// Fournisseurs ajoutes depuis l'interface (persistes dans cles.env).
    perso: Mutex<Vec<Perso>>,
    /// Au moins une verification a eu lieu depuis le demarrage : l'etat affiche
    /// est alors une mesure, pas un etat repris du disque.
    verifie_session: AtomicBool,
    /// LOT 6 : plafond mensuel en tokens (0 = desactive). Il peut etre change
    /// depuis l'interface sans redemarrer la passerelle.
    plafond: AtomicU64,
    /// Total du mois courant pour le plafond. Initialise depuis le JSONL au
    /// premier controle, puis tenu a jour sans relire le mois a chaque requete.
    plafond_utilise: Mutex<Option<(String, u64)>>,
    /// Canal d'arret propre, partage avec `Service` pour `/admin/arreter`.
    arret: Mutex<Option<watch::Sender<bool>>>,
}

/// Client des appels non streames (300 s de garde-fou).
/// `Policy::none()` : aucune redirection amont n'est suivie, une reponse 3xx
/// remonte donc telle quelle et devient une erreur explicite.
fn client_http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .connect_timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("ApiVault_Proxy/{}", VERSION))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Client des flux SSE. Pas de timeout global : un flux peut durer plusieurs
/// minutes. Les bornes sont appliquees cote streaming (inactivite 120 s,
/// duree totale 900 s). Aucune redirection suivie non plus.
fn client_flux() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("ApiVault_Proxy/{}", VERSION))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl AppState {
    pub fn nouveau(cfg: Config) -> AppState {
        let http = client_http();
        let http_flux = client_flux();

        let dossier = dossier_donnees();
        let _ = std::fs::create_dir_all(&dossier);

        let cles = cfg.cles.clone();
        let preference_modele = cfg.modele_prefere.clone();
        let cfg_perso = cfg.perso.clone();
        let plafond = cfg.plafond_mensuel;
        let adresses: HashMap<String, Vec<String>> = cfg
            .fournisseurs
            .iter()
            .map(|(nom, f)| (nom.clone(), f.urls.clone()))
            .collect();

        AppState {
            cfg,
            pool: crate::pool::Pool::charger(),
            cles: Mutex::new(cles),
            adresses: Mutex::new(adresses),
            http,
            http_flux,
            debut: Instant::now(),
            sondage: AtomicBool::new(false),
            derniere_sonde: Mutex::new(None),
            preference_modele: Mutex::new(preference_modele),
            journal: Mutex::new(VecDeque::new()),
            chemin_journal: Some(dossier.join("journal.log")),
            chemin_debug: Some(dossier.join("debug-corps.json")),
            metriques: Arc::new(crate::metriques::Metriques::nouveau(dossier.clone())),
            semaphore: Arc::new(tokio::sync::Semaphore::new(8)),
            perso: Mutex::new(cfg_perso.clone()),
            verifie_session: AtomicBool::new(false),
            plafond: AtomicU64::new(plafond),
            plafond_utilise: Mutex::new(None),
            arret: Mutex::new(None),
        }
    }

    /// Plafond mensuel en tokens (0 = desactive). Lu a chaque requete : il peut
    /// changer depuis l'interface sans redemarrer.
    pub fn plafond(&self) -> u64 {
        self.plafond.load(Ordering::Relaxed)
    }

    /// Applique un nouveau plafond en memoire (la persistance est faite par
    /// `config::sauver_plafond`).
    pub fn fixer_plafond(&self, valeur: u64) {
        self.plafond.store(valeur, Ordering::Relaxed);
    }

    /// Total consomme pour le mois courant, pour le controle du plafond.
    /// Le premier appel lit le JSONL ; les suivants utilisent le cache tenu a
    /// jour par `ajouter_utilisation`, sans relire tout le mois.
    pub fn utilisation_plafond(&self) -> std::io::Result<(String, u64)> {
        let periode = crate::metriques::periode_courante();
        let mut cache = self
            .plafond_utilise
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((p, total)) = cache.as_ref() {
            if *p == periode {
                return Ok((periode, *total));
            }
        }
        let totaux = self.metriques.totaliser(&periode)?;
        let utilise = totaux.entrees.saturating_add(totaux.sorties);
        *cache = Some((periode.clone(), utilise));
        Ok((periode, utilise))
    }

    /// Ajoute la consommation d'une requete au cache du plafond, apres
    /// l'ecriture de sa ligne de metriques.
    pub fn ajouter_utilisation(&self, tokens: u64) {
        let periode = crate::metriques::periode_courante();
        let mut cache = self
            .plafond_utilise
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((p, total)) = cache.as_mut() {
            if *p == periode {
                *total = total.saturating_add(tokens);
                return;
            }
        }
        if let Ok(totaux) = self.metriques.totaliser(&periode) {
            *cache = Some((
                periode,
                totaux
                    .entrees
                    .saturating_add(totaux.sorties)
                    .saturating_add(tokens),
            ));
        }
    }

    /// Demande un arret propre du serveur HTTP (les flux en cours se terminent).
    pub fn demander_arret(&self) {
        if let Ok(mut garde) = self.arret.lock() {
            if let Some(tx) = garde.take() {
                let _ = tx.send(true);
            }
        }
    }

    /// Cle actuellement utilisee pour un fournisseur (vide si absente).
    pub fn cle_fournisseur(&self, nom: &str) -> String {
        let Some(fournisseur) = self.fournisseur_infos(nom) else {
            return String::new();
        };
        self.cles
            .lock()
            .ok()
            .and_then(|c| c.get(&fournisseur.cle_env).cloned())
            .unwrap_or_default()
    }

    /// Description d'un fournisseur, qu'il vienne de `cles.env` ou de la
    /// liste ajoutee depuis l'interface pendant la session.
    pub fn fournisseur_infos(&self, nom: &str) -> Option<Fournisseur> {
        if let Some(f) = self.cfg.fournisseurs.get(nom) {
            return Some(f.clone());
        }
        self.perso
            .lock()
            .ok()?
            .iter()
            .find(|p| p.nom == nom)
            .map(|p| Fournisseur {
                urls: p.urls.clone(),
                cle_env: p.cle_env(),
                entetes: Vec::new(),
            })
    }

    /// Fournisseurs a afficher : ceux du demarrage puis ceux ajoutes depuis
    /// l'interface, sans doublon (les seconds sont deja dans `cfg` au boot).
    fn fournisseurs_effectifs(&self) -> Vec<(String, Fournisseur)> {
        let mut liste: Vec<(String, Fournisseur)> = self
            .cfg
            .fournisseurs
            .iter()
            .map(|(nom, f)| (nom.clone(), f.clone()))
            .collect();
        if let Ok(perso) = self.perso.lock() {
            for p in perso.iter() {
                if !liste.iter().any(|(nom, _)| *nom == p.nom) {
                    liste.push((
                        p.nom.clone(),
                        Fournisseur {
                            urls: p.urls.clone(),
                            cle_env: p.cle_env(),
                            entetes: Vec::new(),
                        },
                    ));
                }
            }
        }
        liste.sort_by(|a, b| a.0.cmp(&b.0));
        liste
    }

    /// Adresses actives d'un fournisseur (au moins une si connue).
    pub fn adresses_fournisseur(&self, nom: &str) -> Vec<String> {
        if let Ok(a) = self.adresses.lock() {
            if let Some(urls) = a.get(nom) {
                if !urls.is_empty() {
                    return urls.clone();
                }
            }
        }
        self.fournisseur_infos(nom)
            .map(|f| f.urls.clone())
            .unwrap_or_default()
    }

    /// Alias avec le modele prefere place en tete, puis les modeles des
    /// fournisseurs ajoutes depuis l'interface (rattaches a `defaut`).
    pub fn alias_effectifs(&self) -> Vec<Alias> {
        let prefere = self
            .preference_modele
            .lock()
            .ok()
            .and_then(|p| p.clone());
        let mut alias = construire_alias(prefere.as_deref());
        if let Ok(perso) = self.perso.lock() {
            if !perso.is_empty() {
                if let Some(defaut) = alias.iter_mut().find(|a| a.nom == "defaut") {
                    for p in perso.iter() {
                        for modele in &p.modeles {
                            let cible = Cible {
                                fournisseur: p.nom.clone(),
                                modele: modele.clone(),
                            };
                            if !defaut.cibles.contains(&cible) {
                                defaut.cibles.push(cible);
                            }
                        }
                    }
                }
            }
        }
        alias
    }

    /// Modele prefere actuellement applique.
    pub fn modele_prefere(&self) -> Option<String> {
        self.preference_modele
            .lock()
            .ok()
            .and_then(|p| p.clone())
    }

    /// Change le modele prefere sans redemarrer : l'ordre des alias est
    /// recalcule a chaque requete. La preference est aussi ecrite dans
    /// `cles.env` pour le prochain demarrage.
    pub fn definir_modele_prefere(&self, modele: Option<&str>) -> Result<(), String> {
        let prefere = modele
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());

        if let Some(ref modele) = prefere {
            let connu = self
                .alias_effectifs()
                .iter()
                .flat_map(|a| a.cibles.iter())
                .any(|c| c.modele == *modele);
            if !connu {
                return Err(format!("Modele inconnu : {}", modele));
            }
        }

        sauver_modele_prefere(&self.cfg.chemin_cles, prefere.as_deref())
            .map_err(|e| format!("ecriture cles.env : {}", e))?;
        if let Ok(mut p) = self.preference_modele.lock() {
            *p = prefere.clone();
        }
        match prefere {
            Some(m) => self.journaliser(&format!("Modele prefere : {}", m)),
            None => self.journaliser("Modele prefere : aucun"),
        }
        Ok(())
    }

    /// Etat des fournisseurs pour le panneau de gestion (cle masquee).
    pub fn gestion_json(&self) -> Value {
        let alias = self.alias_effectifs();
        let mut liste: Vec<Value> = self
            .fournisseurs_effectifs()
            .iter()
            .map(|(nom, f)| {
                let nom = nom.as_str();
                let cle = self.cle_fournisseur(nom);
                // A5 : un modele peut etre declare par plusieurs alias ; une
                // seule ligne, avec la provenance (liste des alias) conservee.
                let par_modele = modeles_par_fournisseur(&alias, nom);
                json!({
                    "id": nom,
                    "cle_env": f.cle_env,
                    "variable_urls": f.variable_urls(),
                    "cle_presente": !cle.is_empty(),
                    "cle_masquee": if cle.is_empty() { String::new() } else { masquer(&cle) },
                    "urls": self.adresses_fournisseur(nom),
                    "cibles": par_modele.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>(),
                    // Detail par modele : le panneau affiche un badge
                    // gratuit / payant et peut tester le fournisseur d'un coup.
                    "modeles": par_modele
                        .iter()
                        .map(|(m, noms)| {
                            json!({
                                "modele": m,
                                "gratuit": modele_gratuit(m),
                                "alias": noms,
                            })
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        liste.sort_by(|a, b| {
            a["id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["id"].as_str().unwrap_or(""))
        });
        json!({
            "fournisseurs": liste,
            "chemin_cles": self.cfg.chemin_cles.to_string_lossy(),
            "modele_prefere": self.modele_prefere(),
        })
    }

    /// Requalifie toutes les cibles d'un fournisseur puis les reteste.
    pub fn retester_fournisseur(self: &Arc<Self>, nom: &str) {
        for alias in self.alias_effectifs() {
            for cible in &alias.cibles {
                if cible.fournisseur == nom {
                    self.pool.requalifier(&cible.fournisseur, &cible.modele);
                }
            }
        }
        self.lancer_sondage(false);
    }

    /// Enregistre une cle dans le fichier et l'applique immediatement.
    pub fn enregistrer_cle(self: &Arc<Self>, nom: &str, cle: &str) -> Result<(), String> {
        let Some(fournisseur) = self.fournisseur_infos(nom) else {
            return Err(format!("Fournisseur inconnu : {}", nom));
        };
        let cle = cle.trim().to_string();
        ecrire_variable(&self.cfg.chemin_cles, &fournisseur.cle_env, &cle)
            .map_err(|e| format!("Ecriture impossible : {}", e))?;
        if let Ok(mut c) = self.cles.lock() {
            c.insert(fournisseur.cle_env.clone(), cle.clone());
        }
        self.journaliser(&format!(
            "cle {} enregistree depuis l'interface ({})",
            fournisseur.cle_env,
            if cle.is_empty() { "effacee" } else { "mise a jour" }
        ));
        self.retester_fournisseur(nom);
        Ok(())
    }

    /// Enregistre la liste d'adresses d'un fournisseur (une par ligne).
    pub fn enregistrer_adresses(self: &Arc<Self>, nom: &str, texte: &str) -> Result<(), String> {
        let Some(fournisseur) = self.fournisseur_infos(nom) else {
            return Err(format!("Fournisseur inconnu : {}", nom));
        };
        let urls = decouper_adresses(texte);
        if urls.is_empty() {
            return Err("Au moins une adresse est necessaire.".to_string());
        }
        for url in &urls {
            if !adresse_upstream_valide(url) {
                return Err(format!(
                    "Adresse invalide : '{}' (https:// requis ; http:// accepte uniquement vers 127.0.0.1/localhost)",
                    url
                ));
            }
        }
        let valeur = urls.join(",");
        ecrire_variable(&self.cfg.chemin_cles, &fournisseur.variable_urls(), &valeur)
            .map_err(|e| format!("Ecriture impossible : {}", e))?;
        if let Ok(mut a) = self.adresses.lock() {
            a.insert(nom.to_string(), urls.clone());
        }
        self.journaliser(&format!(
            "adresses {} enregistrees : {}",
            nom,
            urls.join(", ")
        ));
        self.retester_fournisseur(nom);
        Ok(())
    }

    /// Ajoute un fournisseur dans cles.env puis l'active sans redemarrer.
    /// Ecriture atomique, sauvegarde `.bak`, commentaires conserves, et la cle
    /// n'apparait ni dans le journal ni dans un message d'erreur.
    pub fn ajouter_fournisseur(
        self: &Arc<Self>,
        nom: &str,
        url: &str,
        cle: &str,
        modeles: &str,
    ) -> Result<(), String> {
        let nom = nom.trim().to_lowercase().replace([' ', '-', '.'], "_");
        if nom.len() < 2 || nom.len() > 32 {
            return Err("Nom invalide : 2 a 32 caracteres.".to_string());
        }
        if !nom.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return Err("Nom invalide : il doit commencer par une lettre.".to_string());
        }
        if !nom
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err("Nom invalide : lettres, chiffres et _ uniquement.".to_string());
        }
        if self.fournisseur_infos(&nom).is_some() {
            return Err(format!("Ce fournisseur existe deja : {}", nom));
        }

        let urls = decouper_adresses(url);
        if urls.is_empty() {
            return Err("Une adresse est necessaire.".to_string());
        }
        for u in &urls {
            if !adresse_upstream_valide(u) {
                return Err(format!(
                    "Adresse invalide : '{}' (https:// requis ; http:// accepte uniquement vers 127.0.0.1/localhost)",
                    u
                ));
            }
        }
        let modeles: Vec<String> = modeles
            .split([',', ';', '\n', '\r', ' ', '\t'])
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if modeles.is_empty() {
            return Err("Au moins un modele est necessaire.".to_string());
        }

        // Sauvegarde avant modification. `ecrire_variable` ne remplace qu'une
        // ligne : les commentaires et les autres variables restent intacts.
        // Sauvegarde avant modification, chiffree par le coffre : la copie de
        // secours ne doit pas contenir l'ancienne cle en clair.
        let _ = coffre::copier_chiffre(
            &self.cfg.chemin_cles,
            &self.cfg.chemin_cles.with_extension("env.bak"),
        );

        let mut liste: Vec<String> = self.cfg.perso.iter().map(|p| p.nom.clone()).collect();
        if let Ok(perso) = self.perso.lock() {
            for p in perso.iter() {
                if !liste.contains(&p.nom) {
                    liste.push(p.nom.clone());
                }
            }
        }
        liste.push(nom.clone());

        let perso = Perso {
            nom: nom.clone(),
            urls: urls.clone(),
            modeles: modeles.clone(),
        };
        let ecrire = |variable: &str, valeur: &str| {
            ecrire_variable_sans_sauvegarde(&self.cfg.chemin_cles, variable, valeur)
                .map_err(|e| format!("Ecriture impossible : {}", e))
        };
        ecrire("PROXY_FOURNISSEURS", &liste.join(";"))?;
        ecrire(&perso.variable_urls(), &urls.join(","))?;
        ecrire(&perso.variable_modeles(), &modeles.join(","))?;
        ecrire(&perso.cle_env(), cle.trim())?;

        if let Ok(mut c) = self.cles.lock() {
            c.insert(perso.cle_env(), cle.trim().to_string());
        }
        if let Ok(mut a) = self.adresses.lock() {
            a.insert(nom.clone(), urls.clone());
        }
        if let Ok(mut p) = self.perso.lock() {
            p.push(perso.clone());
        }
        self.journaliser(&format!(
            "fournisseur {} ajoute depuis l'interface ({} modele(s))",
            nom,
            modeles.len()
        ));
        self.retester_fournisseur(&nom);
        Ok(())
    }

    /// Teste une cle sur `{url}/models`, sans rien enregistrer.
    pub async fn tester_cle(&self, url: &str, cle: &str) -> Value {
        let base = url.trim().trim_end_matches('/');
        let base = base.strip_suffix("/chat/completions").unwrap_or(base);
        let cible = if base.ends_with("/models") {
            base.to_string()
        } else {
            format!("{}/models", base)
        };
        let debut = Instant::now();
        let reponse = self
            .http
            .get(&cible)
            .timeout(std::time::Duration::from_secs(8))
            .header("authorization", format!("Bearer {}", cle.trim()))
            .send()
            .await;
        let duree_ms = debut.elapsed().as_millis() as u64;
        match reponse {
            Ok(r) => {
                let statut = r.status().as_u16();
                if !r.status().is_success() {
                    return json!({
                        "ok": false,
                        "statut": statut,
                        "erreur": format!("HTTP {}", statut),
                        "duree_ms": duree_ms,
                    });
                }
                let texte = r.text().await.unwrap_or_default();
                let mut modeles: Vec<String> = Vec::new();
                if let Ok(valeur) = serde_json::from_str::<Value>(&texte) {
                    if let Some(liste) = valeur.get("data").and_then(|d| d.as_array()) {
                        for modele in liste.iter().take(50) {
                            if let Some(id) = modele.get("id").and_then(|i| i.as_str()) {
                                modeles.push(id.to_string());
                            }
                        }
                    }
                }
                json!({
                    "ok": true,
                    "statut": statut,
                    "modeles": modeles,
                    "duree_ms": duree_ms,
                })
            }
            Err(e) => json!({
                "ok": false,
                "statut": 0,
                "erreur": format!("{}", e),
                "duree_ms": duree_ms,
            }),
        }
    }

    pub fn autorise(&self, entetes: &HeaderMap) -> bool {
        // Fail-closed : sans jeton configure, aucune requete n'est acceptee.
        // L'ancien comportement (fail-open) laissait la passerelle ouverte a
        // quiconque lorsque PROXY_MASTER_KEY etait vide.
        jeton_valide(&self.cfg.jeton, entetes)
    }

    /// Ligne de journal rattachee a une requete : porte toujours son id.
    pub fn journaliser_req(&self, id: &str, message: &str) {
        self.journaliser(&format!("req={} {}", id, message));
    }

    pub fn journaliser(&self, message: &str) {
        // Ne jamais laisser une cle connue atterrir dans journal.log, meme si
        // un fournisseur la renvoie dans son corps d'erreur.
        let mut message = masquer_porteur(message);
        message = masquer_parametres(&message);
        if let Ok(cles) = self.cles.try_lock() {
            for (nom, valeur) in cles.iter() {
                // Seules les vraies cles sont masquees : PROXY_PORT et les URL
                // ne doivent pas etre remplaces dans les messages.
                if (nom == "PROXY_MASTER_KEY" || nom.ends_with("_KEY"))
                    && !valeur.trim().is_empty()
                {
                    message = message.replace(valeur.as_str(), "***");
                }
            }
        }
        if !self.cfg.jeton.trim().is_empty() {
            message = message.replace(self.cfg.jeton.as_str(), "***");
        }
        let ligne = format!("[{:>7.1}s] {}", self.debut.elapsed().as_secs_f64(), message);
        if let Ok(mut j) = self.journal.lock() {
            j.push_back(ligne.clone());
            while j.len() > 400 {
                j.pop_front();
            }
        }
        if let Some(chemin) = &self.chemin_journal {
            let trop_gros = std::fs::metadata(chemin)
                .map(|m| m.len() > 2 * 1024 * 1024)
                .unwrap_or(false);
            if trop_gros {
                // Rotation : on conserve journal.log.1 au lieu de vider le
                // fichier, pour ne pas perdre la trace de l'incident.
                let archive = chemin.with_extension("log.1");
                let _ = std::fs::remove_file(&archive);
                let _ = std::fs::rename(chemin, &archive);
            }
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(chemin)
            {
                let _ = writeln!(f, "{}", ligne);
            }
        }
        println!("{}", ligne);
    }

    pub fn lignes_journal(&self) -> Vec<String> {
        self.journal
            .lock()
            .map(|j| j.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Ecrit la derniere requete recue dans debug-corps.json lorsque
    /// PROXY_DEBUG_CORPS=1 (diagnostic uniquement, desactive par defaut).
    pub fn journaliser_corps(&self, valeur: &Value) {
        let actif = std::env::var("PROXY_DEBUG_CORPS")
            .map(|v| v == "1")
            .unwrap_or(false);
        if !actif {
            return;
        }
        if let Some(chemin) = &self.chemin_debug {
            if let Ok(texte) = serde_json::to_string_pretty(valeur) {
                let _ = std::fs::write(chemin, texte);
            }
        }
    }

    /// Requalifie une cible puis l'interroge reellement.
    pub fn sonder_une(self: &Arc<Self>, fournisseur: &str, modele: &str) {
        let Some(cible) = self
            .cfg
            .alias
            .iter()
            .flat_map(|a| a.cibles.iter())
            .find(|c| c.fournisseur == fournisseur && c.modele == modele)
            .cloned()
        else {
            return;
        };

        self.pool.requalifier(fournisseur, modele);
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            sonder_cible(&app, &cible).await;
        });
    }

    /// Interroge reellement chaque fournisseur, en arriere-plan.
    /// `reinitialiser` remet d'abord tous les etats a « inconnu ».
    pub fn lancer_sondage(self: &Arc<Self>, reinitialiser: bool) {
        if self.sondage.swap(true, Ordering::SeqCst) {
            return; // deja en cours
        }
        if let Ok(mut d) = self.derniere_sonde.lock() {
            *d = Some(Instant::now());
        }
        if reinitialiser {
            self.pool.requalifier_tout();
        }
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            app.journaliser(if reinitialiser {
                "rafraichissement : interrogation des fournisseurs"
            } else {
                "verification des fournisseurs"
            });
            sonder(app.clone()).await;
            app.sondage.store(false, Ordering::SeqCst);
            app.journaliser(&format!(
                "verification terminee : {} cible(s) saine(s)",
                app.pool.compter_sains()
            ));
        });
    }

    /// Verification de fond, une seule fois par demarrage : uniquement les
    /// cibles jamais testees, 4 en parallele au plus, 5 s par cible.
    pub fn sonder_les_inconnus(self: &Arc<Self>, parallelisme: usize, delai: std::time::Duration) {
        if self.sondage.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let cibles = app.pool.inconnues(&app.alias_effectifs());
            if cibles.is_empty() {
                app.sondage.store(false, Ordering::SeqCst);
                return;
            }
            app.journaliser(&format!(
                "verification silencieuse : {} cible(s) jamais testee(s)",
                cibles.len()
            ));
            let limite = Arc::new(tokio::sync::Semaphore::new(parallelisme.max(1)));
            let mut taches = Vec::new();
            for cible in cibles {
                let app_tache = Arc::clone(&app);
                let permis = Arc::clone(&limite);
                taches.push(tauri::async_runtime::spawn(async move {
                    let _garde = permis.acquire_owned().await;
                    sonder_cible_borne(&app_tache, &cible, delai).await;
                }));
            }
            for tache in taches {
                let _ = tache.await;
            }
            app.sondage.store(false, Ordering::SeqCst);
            app.journaliser("verification silencieuse terminee");
        });
    }

    /// Verification de secours declenchee par un appel qui echoue.
    /// Elle ne part pas si une verification vient d'avoir lieu (60 s), afin
    /// qu'une rafale d'erreurs ne transforme pas la passerelle en sondeur fou.
    pub fn rafraichir_si_besoin(self: &Arc<Self>) {
        if self.sondage.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(d) = self.derniere_sonde.lock() {
            if let Some(t) = *d {
                if t.elapsed() < std::time::Duration::from_secs(60) {
                    return;
                }
            }
        }
        self.journaliser("un appel a echoue : verification automatique des fournisseurs");
        self.lancer_sondage(false);
    }

    /// Diagnostic complet et copiable, sans aucun secret.
    pub fn diagnostic_json(
        &self,
        port: u16,
        jeton: &str,
        actif: bool,
        depuis_s: Option<u64>,
    ) -> Value {
        let mut secrets: Vec<String> = self
            .cfg
            .cles
            .values()
            .filter(|v| !v.trim().is_empty())
            .cloned()
            .collect();
        if let Ok(cles) = self.cles.lock() {
            for valeur in cles.values() {
                if !valeur.trim().is_empty() && !secrets.contains(valeur) {
                    secrets.push(valeur.clone());
                }
            }
        }
        if !jeton.trim().is_empty() {
            secrets.push(jeton.to_string());
        }
        let masquer_secrets = |texte: &str| -> String {
            let mut sortie = texte.to_string();
            for secret in &secrets {
                sortie = sortie.replace(secret.as_str(), "***");
            }
            sortie
        };
        // Une URL peut porter une cle en query string : on la coupe.
        let url_propre = |url: &str| -> String {
            let sans_requete = url.split(['?', '#']).next().unwrap_or(url);
            masquer_secrets(sans_requete)
        };

        let alias = self.alias_effectifs();
        let fournisseurs: Vec<Value> = self
            .fournisseurs_effectifs()
            .iter()
            .map(|(nom, f)| {
                let mut modeles: Vec<String> = Vec::new();
                for a in alias.iter() {
                    for c in a.cibles.iter().filter(|c| c.fournisseur == *nom) {
                        if !modeles.contains(&c.modele) {
                            modeles.push(c.modele.clone());
                        }
                    }
                }
                json!({
                    "nom": nom,
                    "urls": f.urls.iter().map(|u| url_propre(u)).collect::<Vec<_>>(),
                    "modeles": modeles,
                    "cle_presente": !self.cle_fournisseur(nom).is_empty(),
                })
            })
            .collect();

        let mut journal: Vec<String> = self
            .lignes_journal()
            .iter()
            .rev()
            .take(80)
            .map(|l| masquer_secrets(l))
            .collect();
        journal.reverse();

        let genere_le = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        json!({
            "genere_le_unix": genere_le,
            "version": VERSION,
            "port": port,
            "adresse": format!("http://127.0.0.1:{}/v1", port),
            "actif": actif,
            "depuis_s": depuis_s,
            "demarrage_ms": ms_depuis_demarrage(),
            "fichier_cles": self.cfg.chemin_cles.display().to_string(),
            "chemin_journal": self.chemin_journal.as_ref().map(|c| c.display().to_string()),
            "dossier_donnees": dossier_donnees().display().to_string(),
            "fournisseurs": fournisseurs,
            "alias": alias.iter().map(|a| json!({
                "nom": a.nom,
                "cibles": self.vue_redigee(a),
            })).collect::<Vec<_>>(),
            "journal": journal,
        })
    }

    /// Vue d'un alias telle qu'elle part vers l'interface : les erreurs amont
    /// sont redigees avant affichage (B3).
    pub fn vue_redigee(&self, alias: &Alias) -> Vec<Value> {
        let mut cibles = self.pool.vue(alias);
        for cible in cibles.iter_mut() {
            for cle in ["erreur", "detail"] {
                if let Some(Value::String(texte)) = cible.get_mut(cle) {
                    *texte = self.rediger_erreur(texte);
                }
            }
        }
        cibles
    }

    /// Masque les secrets connus, les jetons `Bearer` et les parametres d'URL
    /// sensibles avant qu'un texte d'erreur ne soit affiche.
    pub fn rediger_erreur(&self, texte: &str) -> String {
        let mut sortie = texte.to_string();
        for secret in self.secrets_connus() {
            if secret.chars().count() >= 6 {
                sortie = sortie.replace(&secret, "***");
            }
        }
        masquer_parametres(&masquer_porteur(&sortie))
    }

    fn secrets_connus(&self) -> Vec<String> {
        let mut secrets: Vec<String> = self
            .cfg
            .cles
            .values()
            .filter(|v| !v.trim().is_empty())
            .cloned()
            .collect();
        if let Ok(cles) = self.cles.lock() {
            for valeur in cles.values() {
                if !valeur.trim().is_empty() && !secrets.contains(valeur) {
                    secrets.push(valeur.clone());
                }
            }
        }
        secrets
    }

    /// Constructeur minimal pour les tests : aucun fichier n'est touche.
    #[cfg(test)]
    pub fn pour_test(cfg: Config) -> AppState {
        let cfg_perso = cfg.perso.clone();
        AppState {
            cles: Mutex::new(cfg.cles.clone()),
            adresses: Mutex::new(HashMap::new()),
            http: client_http(),
            http_flux: client_flux(),
            cfg,
            pool: Pool::vide(),
            debut: Instant::now(),
            sondage: AtomicBool::new(false),
            derniere_sonde: Mutex::new(None),
            preference_modele: Mutex::new(None),
            journal: Mutex::new(VecDeque::new()),
            chemin_journal: None,
            chemin_debug: None,
            metriques: Arc::new(crate::metriques::Metriques::nouveau(
                std::env::temp_dir().join("apivault-metriques-test"),
            )),
            semaphore: Arc::new(tokio::sync::Semaphore::new(8)),
            perso: Mutex::new(cfg_perso),
            verifie_session: AtomicBool::new(false),
            plafond: AtomicU64::new(0),
            plafond_utilise: Mutex::new(None),
            arret: Mutex::new(None),
        }
    }
}

pub struct Service {
    pub port: u16,
    pub jeton: String,
    pub app: Arc<AppState>,
    pub actif: AtomicBool,
    interface_vue: AtomicBool,
    stop: Mutex<Option<watch::Sender<bool>>>,
    erreur: Mutex<Option<String>>,
    /// Code stable de la derniere erreur de demarrage (`port_occupe`, ...),
    /// pour que l'interface affiche un message actionnable en FR et en EN.
    erreur_code: Mutex<Option<String>>,
    depuis: Mutex<Option<Instant>>,
}

impl Service {
    pub fn nouveau(port: u16, jeton: String, app: Arc<AppState>) -> Arc<Service> {
        let service = Arc::new(Service {
            port,
            jeton,
            app,
            actif: AtomicBool::new(false),
            interface_vue: AtomicBool::new(false),
            stop: Mutex::new(None),
            erreur: Mutex::new(None),
            erreur_code: Mutex::new(None),
            depuis: Mutex::new(None),
        });
        if service.jeton.trim().is_empty() {
            service.app.journaliser(
                "jeton absent : toutes les requetes seront refusees (401). Renseignez PROXY_MASTER_KEY.",
            );
        }
        service
    }

    pub fn demarrer(self: &Arc<Self>) {
        {
            let garde = match self.stop.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if garde.is_some() {
                return; // deja demarre ou en cours de demarrage
            }
        }

        let (tx, mut rx) = watch::channel(false);
        if let Ok(mut a) = self.app.arret.lock() {
            *a = Some(tx.clone());
        }
        if let Ok(mut garde) = self.stop.lock() {
            *garde = Some(tx);
        }
        if let Ok(mut e) = self.erreur.lock() {
            *e = None;
        }
        if let Ok(mut c) = self.erreur_code.lock() {
            *c = None;
        }

        let srv = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let adresse = SocketAddr::from(([127, 0, 0, 1], srv.port));
            match tokio::net::TcpListener::bind(adresse).await {
                Ok(ecoute) => {
                    srv.actif.store(true, Ordering::SeqCst);
                    if let Ok(mut d) = srv.depuis.lock() {
                        *d = Some(Instant::now());
                    }
                    srv.app.journaliser(&format!(
                        "demarrage serveur : {} ms (en ecoute sur http://127.0.0.1:{} - {} alias)",
                        ms_depuis_demarrage(),
                        srv.port,
                        srv.app.cfg.alias.len()
                    ));

                    let app = Arc::clone(&srv.app);
                    let _ = axum::serve(ecoute, routeur(app))
                        .with_graceful_shutdown(async move {
                            let _ = rx.changed().await;
                        })
                        .await;

                    srv.actif.store(false, Ordering::SeqCst);
                    if let Ok(mut d) = srv.depuis.lock() {
                        *d = None;
                    }
                    srv.app.journaliser("service arrete");
                }
                Err(e) => {
                    srv.actif.store(false, Ordering::SeqCst);
                    // Message affiche : explicite et actionnable, jamais la
                    // seule trace technique.
                    let (code, message) = if e.kind() == std::io::ErrorKind::AddrInUse {
                        (
                            "port_occupe",
                            format!(
                                "Le port {} est deja utilise par une autre application. Ferme-la, puis relance ApiVault Proxy.",
                                srv.port
                            ),
                        )
                    } else {
                        (
                            "port_indisponible",
                            format!(
                                "Le service n'a pas pu demarrer sur le port {}. La cause technique est dans le journal.",
                                srv.port
                            ),
                        )
                    };
                    // LOT 7 bis, point 6 : le journal porte la cause technique
                    // et le message actionnable. Un utilisateur qui lit le
                    // journal doit savoir quoi faire, pas seulement ce qui a
                    // echoue.
                    srv.app.journaliser(&format!(
                        "Impossible d'ecouter sur 127.0.0.1:{} : {}. {}",
                        srv.port, e, message
                    ));
                    if let Ok(mut err) = srv.erreur.lock() {
                        *err = Some(message);
                    }
                    if let Ok(mut c) = srv.erreur_code.lock() {
                        *c = Some(code.to_string());
                    }
                }
            }

            if let Ok(mut garde) = srv.stop.lock() {
                *garde = None;
            }
        });
    }

    pub fn arreter(&self) {
        let tx = match self.stop.lock() {
            Ok(mut g) => g.take(),
            Err(_) => None,
        };
        if let Some(tx) = tx {
            let _ = tx.send(true);
            self.app.journaliser("arret demande");
        }
    }

    pub fn basculer(self: &Arc<Self>) {
        if self.actif.load(Ordering::SeqCst) {
            self.arreter();
        } else {
            self.demarrer();
        }
    }

    pub fn etat_json(&self) -> Value {
        if !self.interface_vue.swap(true, Ordering::SeqCst) {
            self.app.journaliser("interface connectee");
        }
        let alias = self.app.alias_effectifs();
        json!({
            "modele_prefere": self.app.modele_prefere(),
            "actif": self.actif.load(Ordering::SeqCst),
            "port": self.port,
            "jeton": self.jeton,
            "jeton_masque": masquer(&self.jeton),
            "erreur": self.erreur.lock().ok().and_then(|e| e.clone()),
            "erreur_code": self.erreur_code.lock().ok().and_then(|c| c.clone()),
            "depuis_s": self.depuis.lock().ok().and_then(|d| d.map(|i| i.elapsed().as_secs())),
            "version": VERSION,
            // « connecte » ne dit pas que le service tourne, mais qu'au moins
            // un fournisseur a repondu *pendant cette session* : un etat
            // simplement repris du disque ne suffit pas.
            "connecte": self.actif.load(Ordering::SeqCst) && self.app.pool.a_un_sain_verifie(),
            "sains": self.app.pool.compter_sains(),
            "sains_verifies": self.app.pool.compter_sains_verifies(),
            "sondage": self.app.sondage.load(Ordering::SeqCst),
            "derniere_reussite_s": self.app.pool.derniere_reussite_s(),
            // Etat repris de la session precedente : age de la derniere
            // verification enregistree sur le disque (aucune sonde au demarrage).
            "derniere_verification_s": self.app.pool.derniere_verification_s(),
            "demarrage_s": self.app.debut.elapsed().as_secs(),
            "chemin_cles": self.app.cfg.chemin_cles.display().to_string(),
            "alias": alias.iter().map(|a| json!({
                "nom": a.nom,
                "cibles": self.app.vue_redigee(a),
            })).collect::<Vec<_>>(),
        })
    }

    /// Ajoute un fournisseur (delegue a l'etat, qui ecrit dans cles.env).
    pub fn ajouter_fournisseur(
        &self,
        nom: &str,
        url: &str,
        cle: &str,
        modeles: &str,
    ) -> Result<(), String> {
        self.app.ajouter_fournisseur(nom, url, cle, modeles)
    }

    /// Liste des fournisseurs telle que l'affiche l'interface.
    pub fn gestion_json(&self) -> Value {
        self.app.gestion_json()
    }

    /// Passe de fond sur les cibles jamais testees (delegue a l'etat).
    pub fn sonder_les_inconnus(&self, parallelisme: usize, delai: std::time::Duration) {
        self.app.sonder_les_inconnus(parallelisme, delai);
    }

    /// Diagnostic complet et copiable. Il ne contient aucune valeur de cle,
    /// aucun entete d'authentification et aucune query string : tout ce qui
    /// ressemble a un secret connu est remplace par `***`.
    pub fn diagnostic_json(&self) -> Value {
        self.app.diagnostic_json(
            self.port,
            &self.jeton,
            self.actif.load(Ordering::SeqCst),
            Some(self.app.debut.elapsed().as_secs()),
        )
    }
}

// ---------------------------------------------------------------------------
// Routage
// ---------------------------------------------------------------------------

pub fn routeur(app: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(racine))
        .route("/health", get(sante))
        .route("/health/liveliness", get(sante))
        .route("/health/readiness", get(sante))
        .route("/v1/models", get(modeles))
        .route("/v1/models/{id}", get(modele_unique))
        .route("/models", get(modeles))
        .route("/v1/chat/completions", post(chat))
        .route("/chat/completions", post(chat))
        .route("/admin/pool", get(pool_vue))
        .route("/admin/metriques", get(admin_metriques))
        .route("/admin/requalifier", post(pool_requalifier))
        .route("/admin/rafraichir", post(pool_rafraichir))
        .route("/admin/arreter", post(admin_arreter))
        .fallback(route_inconnue)
        // 16 MiB : un contexte de 200k tokens pese environ 0,8 Mo, la marge
        // couvre les longs historiques et les pieces jointes textuelles.
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
        // B2 : un `Host` etranger est refuse avant tout traitement.
        .layer(axum::middleware::from_fn(verifier_hote))
        .with_state(app)
}

/// Modeles d'un fournisseur, dedoublonnes, avec la liste des alias qui les
/// declarent (A5).
fn modeles_par_fournisseur(alias: &[Alias], nom: &str) -> Vec<(String, Vec<String>)> {
    let mut par_modele: Vec<(String, Vec<String>)> = Vec::new();
    for a in alias.iter() {
        for c in a.cibles.iter().filter(|c| c.fournisseur == nom) {
            match par_modele.iter_mut().find(|(m, _)| *m == c.modele) {
                Some((_, noms)) => {
                    if !noms.contains(&a.nom) {
                        noms.push(a.nom.clone());
                    }
                }
                None => par_modele.push((c.modele.clone(), vec![a.nom.clone()])),
            }
        }
    }
    par_modele
}

fn adresse_upstream_valide(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    match parsed.scheme() {
        "https" => true,
        // HTTP n'est tolere que vers la boucle locale (tests, faux fournisseur).
        "http" => matches!(
            parsed.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1") | Some("[::1]")
        ),
        _ => false,
    }
}

/// Masque le jeton qui suit `Bearer` (insensible a la casse).
fn masquer_porteur(texte: &str) -> String {
    let octets: Vec<char> = texte.chars().collect();
    let minuscule: Vec<char> = texte.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut resultat = String::new();
    let mut copie = 0usize;
    let mut i = 0usize;
    while i + 6 <= octets.len() {
        let mot: String = minuscule[i..i + 6].iter().collect();
        if mot == "bearer" && i + 6 < octets.len() && octets[i + 6].is_whitespace() {
            let mut debut = i + 6;
            while debut < octets.len() && octets[debut].is_whitespace() {
                debut += 1;
            }
            let mut fin = debut;
            while fin < octets.len()
                && !octets[fin].is_whitespace()
                && !matches!(octets[fin], '"' | '\'' | ',' | ')' | ']' | ';')
            {
                fin += 1;
            }
            if fin > debut {
                resultat.extend(&octets[copie..debut]);
                resultat.push_str("***");
                copie = fin;
            }
            i = fin;
        } else {
            i += 1;
        }
    }
    resultat.extend(&octets[copie..]);
    resultat
}

/// Masque la valeur des parametres d'URL dont le nom evoque un secret
/// (`api_key`, `token`, `secret`, `auth`, `password`, `sig`).
fn masquer_parametres(texte: &str) -> String {
    let octets: Vec<char> = texte.chars().collect();
    let mut plages: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < octets.len() {
        if octets[i] == '?' || octets[i] == '&' {
            let debut_cle = i + 1;
            let mut j = debut_cle;
            while j < octets.len() && octets[j] != '=' && octets[j] != '&' && !octets[j].is_whitespace() {
                j += 1;
            }
            if j < octets.len() && octets[j] == '=' {
                let cle: String = octets[debut_cle..j]
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                let debut_valeur = j + 1;
                let mut k = debut_valeur;
                while k < octets.len()
                    && octets[k] != '&'
                    && !octets[k].is_whitespace()
                    && !matches!(octets[k], '"' | '\'' | ')' | ']' | ';')
                {
                    k += 1;
                }
                let sensible = ["key", "token", "secret", "auth", "pass", "sig", "credential"]
                    .iter()
                    .any(|motif| cle.contains(motif));
                if sensible && k > debut_valeur {
                    plages.push((debut_valeur, k));
                }
                i = k;
                continue;
            }
        }
        i += 1;
    }

    if plages.is_empty() {
        return texte.to_string();
    }
    let mut resultat = String::new();
    let mut dernier = 0usize;
    for (debut, fin) in plages {
        resultat.extend(&octets[dernier..debut]);
        resultat.push_str("***");
        dernier = fin;
    }
    resultat.extend(&octets[dernier..]);
    resultat
}

/// Refuse (403) tout en-tete `Host` qui n'est ni `127.0.0.1[:port]` ni
/// `localhost[:port]` : protege contre le DNS rebinding.
async fn verifier_hote(requete: axum::extract::Request, suite: Next) -> Response {
    let hote = requete
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_ascii_lowercase());
    let autorise = match hote.as_deref() {
        Some(h) => {
            let nom = match h.rsplit_once(':') {
                Some((nom, port)) if !nom.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                    nom
                }
                _ => h,
            };
            nom == "127.0.0.1" || nom == "localhost"
        }
        None => false,
    };
    if !autorise {
        return erreur_json(
            StatusCode::FORBIDDEN,
            "Hote non autorise : adresse locale uniquement.",
            "hote_refuse",
        );
    }
    suite.run(requete).await
}

async fn racine() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(
            "ApiVault_Proxy - passerelle locale compatible OpenAI. Voir /health et /v1/models.",
        ))
        .unwrap()
}

/// Sonde publique : deux champs seulement, sans jeton. Tout ce qui est utile
/// a l'interface passe par la commande Tauri `etat` ou par `/admin/...`.
async fn sante() -> Response {
    json_reponse(
        StatusCode::OK,
        json!({
            "status": "ok",
            "version": VERSION,
        }),
    )
}

async fn modeles(AxumState(app): AxumState<Arc<AppState>>, entetes: HeaderMap) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }
    let alias = app.alias_effectifs();
    let data: Vec<Value> = alias
        .iter()
        .map(|a| {
            json!({
                "id": a.nom,
                "object": "model",
                "created": 1_700_000_000i64,
                "owned_by": "ApiVault_Proxy",
                "apivault_cibles": a.cibles.iter()
                    .map(|c| format!("{}/{}", c.fournisseur, c.modele))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    json_reponse(StatusCode::OK, json!({ "object": "list", "data": data }))
}

async fn modele_unique(
    AxumState(app): AxumState<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    entetes: HeaderMap,
) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }
    if !app.alias_effectifs().iter().any(|a| a.nom == id) {
        return erreur_json(
            StatusCode::NOT_FOUND,
            &format!("Alias inconnu : '{}'", id),
            "model_not_found",
        );
    }
    json_reponse(
        StatusCode::OK,
        json!({
            "id": id,
            "object": "model",
            "created": 1_700_000_000i64,
            "owned_by": "ApiVault_Proxy",
        }),
    )
}

async fn route_inconnue(uri: axum::http::Uri) -> Response {
    erreur_json(
        StatusCode::NOT_FOUND,
        &format!("Route inconnue : {}", uri.path()),
        "not_found",
    )
}

// ---------------------------------------------------------------------------
// Verification reelle des fournisseurs
// ---------------------------------------------------------------------------

/// Interroge chaque cible une fois (requete minimale) et met a jour son etat.
pub async fn sonder(app: Arc<AppState>) {
    let mut vus: std::collections::HashSet<String> = std::collections::HashSet::new();
    for alias in &app.cfg.alias {
        for cible in &alias.cibles {
            let cle = format!("{}/{}", cible.fournisseur, cible.modele);
            if !vus.insert(cle) {
                continue;
            }
            sonder_cible(&app, cible).await;
        }
    }
}

/// Envoie une requete de completion au fournisseur en essayant ses adresses
/// l'une apres l'autre : si la premiere ne repond pas (reseau), la suivante
/// est tentee automatiquement.
async fn poster(
    app: &AppState,
    client: &reqwest::Client,
    nom_fournisseur: &str,
    cle: &str,
    corps: &Value,
    timeout: Option<std::time::Duration>,
) -> Result<reqwest::Response, String> {
    let adresses = app.adresses_fournisseur(nom_fournisseur);
    if adresses.is_empty() {
        return Err(format!("Aucune adresse configuree pour {}", nom_fournisseur));
    }
    let entetes = app
        .fournisseur_infos(nom_fournisseur)
        .map(|f| f.entetes)
        .unwrap_or_default();

    let mut derniere = String::from("adresse injoignable");
    for (rang, base) in adresses.iter().enumerate() {
        let url = format!("{}/chat/completions", base.trim_end_matches('/'));
        let mut requete = client
            .post(&url)
            .header(header::AUTHORIZATION, format!("Bearer {}", cle))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(delai) = timeout {
            requete = requete.timeout(delai);
        }
        for (nom, valeur) in &entetes {
            requete = requete.header(nom.as_str(), valeur.as_str());
        }
        match requete.json(corps).send().await {
            Ok(reponse) => {
                if rang > 0 {
                    app.journaliser(&format!(
                        "{} : adresse de repli {} utilisee",
                        nom_fournisseur, base
                    ));
                }
                return Ok(reponse);
            }
            Err(e) => {
                derniere = e.to_string();
                if rang + 1 < adresses.len() {
                    app.journaliser(&format!(
                        "{} : adresse {} injoignable ({}), essai de la suivante",
                        nom_fournisseur, base, e
                    ));
                }
            }
        }
    }
    Err(derniere)
}

async fn sonder_cible(app: &AppState, cible: &Cible) {
    sonder_cible_borne(app, cible, std::time::Duration::from_secs(25)).await
}

async fn sonder_cible_borne(app: &AppState, cible: &Cible, delai: std::time::Duration) {
    let Some(fournisseur) = app.fournisseur_infos(&cible.fournisseur) else {
        return;
    };

    let cle = app.cle_fournisseur(&cible.fournisseur);
    if cle.is_empty() {
        app.pool.enregistrer_echec(
            cible,
            Some(401),
            &format!("cle {} absente du fichier cles.env", fournisseur.cle_env),
        );
        app.journaliser(&format!(
            "sondage {}/{} : cle {} absente",
            cible.fournisseur, cible.modele, fournisseur.cle_env
        ));
        return;
    }

    let corps = json!({
        "model": cible.modele,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "stream": false,
    });

    let debut = Instant::now();
    // Une sonde est une observation reelle : l'etat cesse d'etre « repris ».
    app.verifie_session.store(true, Ordering::SeqCst);
    match poster(
        app,
        &app.http,
        &cible.fournisseur,
        &cle,
        &corps,
        Some(delai),
    )
    .await
    {
        Ok(reponse) if reponse.status().is_success() => {
            let latence = debut.elapsed().as_millis() as u64;
            app.pool.enregistrer_succes(cible, latence);
            app.journaliser(&format!(
                "sondage {}/{} : ok ({} ms)",
                cible.fournisseur, cible.modele, latence
            ));
        }
        Ok(reponse) => {
            let statut = reponse.status();
            let texte = reponse.text().await.unwrap_or_default();
            app.pool
                .enregistrer_echec(cible, Some(statut.as_u16()), &texte);
            app.journaliser(&format!(
                "sondage {}/{} : HTTP {} - {}",
                cible.fournisseur,
                cible.modele,
                statut.as_u16(),
                tronquer(&texte.replace('\n', " "), 160)
            ));
        }
        Err(e) => {
            app.pool.enregistrer_echec(cible, None, &e.to_string());
            app.journaliser(&format!(
                "sondage {}/{} : connexion impossible - {}",
                cible.fournisseur, cible.modele, e
            ));
        }
    }
}

/// Interroge de nouveau toutes les cibles (rafraichissement manuel via l'API).
async fn pool_rafraichir(AxumState(app): AxumState<Arc<AppState>>, entetes: HeaderMap) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }
    app.lancer_sondage(true);
    json_reponse(StatusCode::OK, json!({ "ok": true, "sondage": true }))
}

// ---------------------------------------------------------------------------
// LOT 5 : consommation de tokens
// ---------------------------------------------------------------------------

/// `GET /admin/metriques?periode=AAAA-MM` (mois courant par defaut).
///
/// Rend les totaux d'entree, de sortie et de cache de la periode. Aucun nom
/// de cle, aucun contenu de message ne sort d'ici : le journal ne les contient
/// pas non plus.
async fn admin_metriques(
    AxumState(app): AxumState<Arc<AppState>>,
    entetes: HeaderMap,
    Query(parametres): Query<HashMap<String, String>>,
) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }

    let periode = parametres
        .get("periode")
        .cloned()
        .unwrap_or_else(crate::metriques::periode_courante);
    if !crate::metriques::periode_valide(&periode) {
        return erreur_json(
            StatusCode::BAD_REQUEST,
            "Periode invalide : attendu AAAA-MM, par exemple 2026-10.",
            "periode_invalide",
        );
    }

    match app.metriques.totaliser(&periode) {
        Ok(totaux) => json_reponse(
            StatusCode::OK,
            crate::metriques::totaux_avec_plafond(&totaux, app.plafond()),
        ),
        Err(e) => erreur_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Journal de metriques illisible : {}", e),
            "metriques_illisibles",
        ),
    }
}

// ---------------------------------------------------------------------------
// Administration du pool (rafraichissement manuel)
// ---------------------------------------------------------------------------

async fn pool_vue(AxumState(app): AxumState<Arc<AppState>>, entetes: HeaderMap) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }
    let alias: Vec<Value> = app
        .alias_effectifs()
        .iter()
        .map(|a| {
            json!({
                "nom": a.nom,
                "cibles": app.vue_redigee(a),
            })
        })
        .collect();

    json_reponse(StatusCode::OK, json!({ "alias": alias }))
}

/// Corps vide ou `{}` : requalifie toutes les cibles.
/// `{"fournisseur":"openrouter","modele":"..."}` : une seule cible.
async fn pool_requalifier(
    AxumState(app): AxumState<Arc<AppState>>,
    entetes: HeaderMap,
    corps: Bytes,
) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }

    let valeur: Value = if corps.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(sans_bom(&corps)).unwrap_or_else(|_| json!({}))
    };

    let fournisseur = valeur.get("fournisseur").and_then(|v| v.as_str());
    let modele = valeur.get("modele").and_then(|v| v.as_str());

    match (fournisseur, modele) {
        (Some(fournisseur), Some(modele)) => {
            let connue = app.pool.requalifier(fournisseur, modele);
            app.journaliser(&format!(
                "requalification manuelle (api) : {}/{}",
                fournisseur, modele
            ));
            json_reponse(
                StatusCode::OK,
                json!({
                    "ok": true,
                    "cible": format!("{}/{}", fournisseur, modele),
                    "connue": connue,
                }),
            )
        }
        _ => {
            app.pool.requalifier_tout();
            app.journaliser("requalification manuelle (api) : toutes les cibles");
            json_reponse(StatusCode::OK, json!({ "ok": true, "toutes": true }))
        }
    }
}

async fn admin_arreter(AxumState(app): AxumState<Arc<AppState>>, entetes: HeaderMap) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }
    app.journaliser("arret demande via /admin/arreter");
    app.demander_arret();
    json_reponse(StatusCode::OK, json!({ "ok": true }))
}

// ---------------------------------------------------------------------------
// Chat completions
// ---------------------------------------------------------------------------

/// Identifiant de requete : 16 caracteres hexadecimaux (64 bits).
fn identifiant_requete() -> String {
    let brut = uuid::Uuid::new_v4().simple().to_string();
    brut[..16].to_string()
}

/// Ce qu'une requete a consomme, rempli au fil du relais.
///
/// Le flux SSE remplit la meme structure, mais ecrit lui-meme sa ligne quand
/// il se termine (`rapporte`) : a cet instant `chat` a deja rendu la reponse.
#[derive(Debug, Clone, Default)]
pub struct Mesure {
    /// Horodatage du depart de la requete (LOT 7, m-1). C'est lui qui decide du
    /// mois d'imputation, pas l'instant d'ecriture de la ligne.
    pub horodatage: String,
    pub alias: String,
    pub fournisseur: String,
    pub modele: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache: u64,
    /// Vrai dès qu'un `usage` exploitable a ete lu.
    pub usage_vu: bool,
    pub tentative: u32,
    pub flux: bool,
    pub rapporte: bool,
}

/// Enveloppe `chat_interne` : attribue un X-Request-Id, le renvoie au client
/// et ecrit la ligne de metriques de la requete.
async fn chat(
    AxumState(app): AxumState<Arc<AppState>>,
    entetes: HeaderMap,
    corps: Bytes,
) -> Response {
    let id = identifiant_requete();
    let debut = Instant::now();
    let mesure = Arc::new(std::sync::Mutex::new(Mesure::default()));
    {
        // LOT 7, m-1 : l'horodatage est pris au depart, pas a l'ecriture. Une
        // requete a cheval sur minuit reste ainsi imputee au mois ou elle a ete
        // autorisee, celui du controle de plafond.
        let mut m = mesure.lock().unwrap_or_else(|e| e.into_inner());
        m.horodatage = crate::metriques::horodatage_maintenant();
    }
    let mut reponse = chat_interne(app.clone(), entetes, corps, &id, mesure.clone()).await;
    let statut = reponse.status().as_u16();

    let m = mesure.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !m.rapporte {
        let mut e = Enregistrement::nouveau(&id, &m.alias);
        if !m.horodatage.is_empty() {
            e.horodatage = m.horodatage.clone();
        }
        e.fournisseur = m.fournisseur.clone();
        e.modele = m.modele.clone();
        e.statut = statut;
        e.latence_ms = debut.elapsed().as_millis() as u64;
        e.tokens_in = m.tokens_in;
        e.tokens_out = m.tokens_out;
        e.tokens_cache = m.tokens_cache;
        e.estime = !m.usage_vu;
        e.tentative = m.tentative;
        e.flux = false;
        match app.metriques.enregistrer(&e) {
            Ok(_) => app.ajouter_utilisation(e.tokens_in.saturating_add(e.tokens_out)),
            Err(err) => {
                // Une mesure perdue ne doit jamais faire echouer la requete.
                app.journaliser_req(&id, &format!("metriques : ecriture impossible ({})", err));
            }
        }
    }

    if let Ok(valeur) = HeaderValue::from_str(&id) {
        reponse.headers_mut().insert("x-request-id", valeur);
    }
    reponse
}

/// Retire une eventuelle marque d'ordre des octets UTF-8 (EF BB BF).
///
/// Le BOM n'appartient pas au document JSON ; pourtant plusieurs editeurs et
/// clients Windows en prependent un. Sans ce retrait, `serde_json` echoue des
/// le premier octet et le proxy rend 400 alors que le corps est valide.
fn sans_bom(octets: &[u8]) -> &[u8] {
    octets.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(octets)
}

async fn chat_interne(
    app: Arc<AppState>,
    entetes: HeaderMap,
    corps: Bytes,
    id: &str,
    mesure: Arc<std::sync::Mutex<Mesure>>,
) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }

    // Au plus 8 requetes amont simultanees. Le permis est conserve jusqu'a la
    // fin du flux (il est deplace dans le corps de reponse).
    let permis = match app.semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return erreur_json(
                StatusCode::SERVICE_UNAVAILABLE,
                "Trop de requetes amont simultanees (8 maximum).",
                "gateway_busy",
            )
        }
    };

    let mut payload: Value = match serde_json::from_slice(sans_bom(&corps)) {
        Ok(v) => v,
        Err(e) => {
            return erreur_json(
                StatusCode::BAD_REQUEST,
                &format!("Corps JSON invalide : {}", e),
                "invalid_json",
            )
        }
    };

    // Certains clients renvoient des blocs de raisonnement
    // Anthropic dans l'historique. La plupart des fournisseurs OpenAI les
    // refusent : on les retire avant l'envoi.
    normaliser_requete(&mut payload);
    app.journaliser_corps(&payload);

    let nom_alias = payload
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    let alias = app
        .alias_effectifs()
        .into_iter()
        .find(|a| a.nom == nom_alias);
    let Some(alias) = alias else {
        return erreur_json(
            StatusCode::NOT_FOUND,
            &format!(
                "Alias inconnu : '{}'. Disponibles : {}",
                nom_alias,
                app.cfg.noms_alias().join(", ")
            ),
            "model_not_found",
        );
    };

    {
        let mut m = mesure.lock().unwrap_or_else(|e| e.into_inner());
        m.alias = alias.nom.clone();
    }

    if payload.get("messages").is_none() {
        return erreur_json(
            StatusCode::BAD_REQUEST,
            "Champ 'messages' obligatoire.",
            "invalid_request",
        );
    }

    // Plafond mensuel (LOT 6) : controle sur les totaux du mois courant, avant
    // tout appel amont, flux compris. Un journal illisible ne bloque pas : la
    // disponibilite passe avant la stricte comptabilite, et c'est assume.
    let plafond = app.plafond();
    if plafond > 0 {
        match app.utilisation_plafond() {
            Ok((periode, utilise)) => {
                if utilise >= plafond {
                    {
                        let mut m = mesure.lock().unwrap_or_else(|e| e.into_inner());
                        // Rien n'a ete consomme chez le fournisseur : pas de
                        // ligne de mesures, seulement une trace au journal.
                        m.rapporte = true;
                    }
                    app.journaliser_req(
                        id,
                        &format!(
                            "plafond atteint : {} / {} tokens ({}), requete refusee sans appel amont",
                            utilise, plafond, periode
                        ),
                    );
                    return erreur_json(
                        StatusCode::TOO_MANY_REQUESTS,
                        &format!(
                            "Plafond mensuel atteint : {} tokens utilises sur {} pour la periode {}. La requete n'a pas ete envoyee au fournisseur.",
                            utilise, plafond, periode
                        ),
                        "plafond_atteint",
                    );
                }
            }
            // Fail-open assume : on sert la requete et le journal dit pourquoi.
            Err(e) => app.journaliser_req(
                id,
                &format!("plafond non verifie : journal illisible ({})", e),
            ),
        }
    }

    let flux = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut derniere: Option<(StatusCode, String)> = None;
    let debut_requete = Instant::now();
    let ordre = app.pool.ordre(&alias);

    if ordre.is_empty() {
        let resume = app.pool.resume(&alias);
        app.journaliser_req(id, &format!(
            "alias={} : aucune cible disponible ({})",
            alias.nom, resume
        ));
        // L'etat vient peut-etre d'une session precedente : on verifie
        // maintenant plutot que de laisser le client se heurter a un mur.
        app.rafraichir_si_besoin();
        return erreur_json(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!(
                "Alias '{}' : aucune cible disponible. {}. Utilisez « Tout requalifier » \
                 dans la fenetre de controle pour les remettre en jeu.",
                alias.nom, resume
            ),
            "no_candidate",
        );
    }

    // Budget : au plus 3 cibles (1 tentative + 2 bascules) dans une enveloppe
    // globale de 900 s, chaque tentative etant plafonnee a 120 s.
    let budget_bascules = duree_reglable("APIVAULT_BUDGET_BASCULE_MS", BUDGET_BASCULES);
    let delai_cible = duree_reglable("APIVAULT_DELAI_CIBLE_MS", DELAI_CIBLE);
    'cibles: for (rang, cible) in ordre.iter().take(CIBLES_MAX).enumerate() {
        let restant_bascule = budget_bascules.saturating_sub(debut_requete.elapsed());
        if restant_bascule.is_zero() {
            app.journaliser_req(id, &format!(
                "alias={} : budget de {} epuise, arret des bascules",
                alias.nom,
                duree_texte(budget_bascules)
            ));
            derniere = Some((
                StatusCode::GATEWAY_TIMEOUT,
                "Budget de bascule epuise.".to_string(),
            ));
            break;
        }
        let debut_cible = Instant::now();
        {
            let mut m = mesure.lock().unwrap_or_else(|e| e.into_inner());
            m.fournisseur = cible.fournisseur.clone();
            m.modele = cible.modele.clone();
            m.tentative = rang as u32 + 1;
        }
        let Some(fournisseur) = app.cfg.fournisseurs.get(&cible.fournisseur) else {
            app.journaliser_req(id, &format!(
                "fournisseur inconnu : {} (cible ignoree)",
                cible.fournisseur
            ));
            continue;
        };

        let cle = app.cle_fournisseur(&cible.fournisseur);
        if cle.is_empty() {
            let message = format!("Cle {} absente du fichier cles.env", fournisseur.cle_env);
            app.journaliser_req(id, &message);
            derniere = Some((StatusCode::INTERNAL_SERVER_ERROR, message));
            continue;
        }

        let mut corps_up = payload.clone();
        corps_up["model"] = json!(cible.modele);
        // LOT 7, M-1 : taille du corps reellement envoye au fournisseur. Elle sert
        // a estimer l'entree quand aucun usage n'est annonce, en flux comme hors flux.
        let octets_entree = serde_json::to_vec(&corps_up)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        let mut nettoye = false;

        loop {
            let client = if flux { &app.http_flux } else { &app.http };
            // Chaque tentative est plafonnee : min(budget global restant, 120 s).
            let restant_bascule = budget_bascules.saturating_sub(debut_requete.elapsed());
            if restant_bascule.is_zero() {
                app.journaliser_req(id, &format!(
                    "alias={} : budget de {} epuise avant {}/{}",
                    alias.nom,
                    duree_texte(budget_bascules),
                    cible.fournisseur,
                    cible.modele
                ));
                derniere = Some((
                    StatusCode::GATEWAY_TIMEOUT,
                    "Budget de bascule epuise.".to_string(),
                ));
                break 'cibles;
            }
            let delai_tentative = restant_bascule.min(delai_cible);
            match tokio::time::timeout(
                delai_tentative,
                poster(&app, client, &cible.fournisseur, &cle, &corps_up, None),
            )
            .await
            {
                Err(_) => {
                    app.journaliser_req(id, &format!(
                        "alias={} cible={}/{} : delai de {} depasse",
                        alias.nom,
                        cible.fournisseur,
                        cible.modele,
                        duree_texte(delai_tentative)
                    ));
                    app.pool.enregistrer_echec(cible, Some(504), "delai depasse");
                    derniere = Some((
                        StatusCode::GATEWAY_TIMEOUT,
                        format!(
                            "Delai depasse vers {}/{}.",
                            cible.fournisseur, cible.modele
                        ),
                    ));
                    // LOT 7, M-3 : un depassement n'est plus une coupure seche.
                    // S'il reste du budget global, on bascule sur la cible
                    // suivante ; sinon seulement, la requete rend 504.
                    let restant = budget_bascules.saturating_sub(debut_requete.elapsed());
                    if restant.is_zero() {
                        app.journaliser_req(id, &format!(
                            "alias={} : budget de {} epuise, plus de bascule apres {}/{}",
                            alias.nom,
                            duree_texte(budget_bascules),
                            cible.fournisseur,
                            cible.modele
                        ));
                        break 'cibles;
                    }
                    app.journaliser_req(id, &format!(
                        "alias={} : bascule vers la cible suivante ({} de budget restant)",
                        alias.nom,
                        duree_texte(restant)
                    ));
                    break;
                }
                Ok(Ok(reponse)) => {
                    let statut = reponse.status();

                    if statut.is_success() {
                        let etiquette = format!("{}/{}", cible.fournisseur, cible.modele);
                        let latence = debut_cible.elapsed().as_millis() as u64;
                        app.pool.enregistrer_succes(cible, latence);
                        if flux {
                            app.journaliser_req(id, &format!(
                                "alias={} -> {} flux (200)",
                                alias.nom, etiquette
                            ));
                            let journal_app = Arc::clone(&app);
                            let id_flux = id.to_string();
                            let journal: Option<Arc<dyn Fn(String) + Send + Sync>> =
                                Some(Arc::new(move |m: String| {
                                    journal_app.journaliser_req(&id_flux, &m)
                                }));
                            // LOT 5 : c'est le flux qui ecrira la ligne, a sa
                            // fermeture, quand l'usage final sera connu.
                            {
                                let mut m =
                                    mesure.lock().unwrap_or_else(|e| e.into_inner());
                                m.flux = true;
                                m.rapporte = true;
                            }
                            return reponse_flux_avec(
                                reponse,
                                &alias,
                                &etiquette,
                                journal,
                                permis,
                                Some(Arc::new(RapportFlux {
                                    metriques: Arc::clone(&app.metriques),
                                    app: Arc::clone(&app),
                                    mesure: Arc::clone(&mesure),
                                    id: id.to_string(),
                                    alias: alias.nom.clone(),
                                    debut: debut_requete,
                                    octets_entree,
                                })),
                            );
                        }

                        let restant_requete =
                            BUDGET_REQUETE.saturating_sub(debut_requete.elapsed());
                        let lecture =
                            tokio::time::timeout(restant_requete, reponse.bytes()).await;
                        let octets = match lecture {
                            Ok(Ok(b)) => b,
                            Ok(Err(e)) => {
                                app.pool.enregistrer_echec(
                                    cible,
                                    Some(502),
                                    "lecture de la reponse impossible",
                                );
                                derniere = Some((
                                    StatusCode::BAD_GATEWAY,
                                    format!("Lecture de la reponse impossible : {}", e),
                                ));
                                break;
                            }
                            Err(_) => {
                                app.journaliser_req(id, &format!(
                                    "alias={} cible={}/{} : duree totale de {} s depassee",
                                    alias.nom,
                                    cible.fournisseur,
                                    cible.modele,
                                    BUDGET_REQUETE.as_secs()
                                ));
                                app.pool.enregistrer_echec(
                                    cible,
                                    Some(504),
                                    "duree totale de la requete depassee",
                                );
                                derniere = Some((
                                    StatusCode::GATEWAY_TIMEOUT,
                                    "Duree totale de la requete depassee.".to_string(),
                                ));
                                break;
                            }
                        };
                        app.journaliser_req(id, &format!(
                            "alias={} -> {} complet ({} octets)",
                            alias.nom,
                            etiquette,
                            octets.len()
                        ));
                        // LOT 5 : l'usage annonce par le fournisseur alimente
                        // la mesure. Absent -> la ligne sera marquee estimee.
                        if let Ok(valeur) =
                            serde_json::from_slice::<serde_json::Value>(&octets)
                        {
                            if let Some((entree, sortie, cache)) =
                                crate::metriques::usage_depuis_json(&valeur)
                            {
                                let mut m = mesure.lock().unwrap_or_else(|e| e.into_inner());
                                m.tokens_in = entree;
                                m.tokens_out = sortie;
                                m.tokens_cache = cache;
                                m.usage_vu = true;
                            }
                        }
                        // LOT 7, M-1 : sans usage, on estime a partir des octets
                        // echanges. Jamais zero quand du texte a circule.
                        {
                            let mut m = mesure.lock().unwrap_or_else(|e| e.into_inner());
                            if !m.usage_vu {
                                m.tokens_in =
                                    crate::metriques::estimer_tokens(octets_entree);
                                m.tokens_out =
                                    crate::metriques::estimer_tokens(octets.len() as u64);
                            }
                        }
                        return reponse_complete(octets, &alias, &etiquette);
                    }

                    let texte = reponse.text().await.unwrap_or_default();
                    let court = tronquer(&texte, 500);
                    app.journaliser_req(id, &format!(
                        "alias={} cible={}/{} HTTP {}{} : {}",
                        alias.nom,
                        cible.fournisseur,
                        cible.modele,
                        statut.as_u16(),
                        if statut == StatusCode::UNAUTHORIZED {
                            " (cle invalide : fournisseur condamne)"
                        } else if statut == StatusCode::FORBIDDEN {
                            " (acces refuse : pause temporaire)"
                        } else {
                            ""
                        },
                        court.replace('\n', " ")
                    ));
                    app.pool
                        .enregistrer_echec(cible, Some(statut.as_u16()), &court);

                    // 400 du a un parametre non supporte par le modele :
                    // on reessaie une fois sans outils / format de reponse.
                    if statut == StatusCode::BAD_REQUEST
                        && !nettoye
                        && parametres_incompatibles(&texte)
                    {
                        enlever_parametres_optionnels(&mut corps_up);
                        nettoye = true;
                        app.journaliser_req(id, 
                            "400 sur un parametre optionnel : nouvel essai sans tools/response_format",
                        );
                        continue;
                    }

                    // 401 : le fournisseur est condamne (toutes ses cibles).
                    // 403 : simple pause ciblee, reprise par la reconciliation.
                    if reessayable(statut)
                        || statut == StatusCode::UNAUTHORIZED
                        || statut == StatusCode::FORBIDDEN
                    {
                        derniere = Some((statut, court));
                        break;
                    }

                    return reponse_amont(statut, texte);
                }
                Ok(Err(e)) => {
                    app.journaliser_req(id, &format!(
                        "alias={} cible={}/{} connexion impossible : {}",
                        alias.nom, cible.fournisseur, cible.modele, e
                    ));
                    app.pool.enregistrer_echec(cible, None, &e.to_string());
                    derniere = Some((
                        StatusCode::BAD_GATEWAY,
                        format!("Connexion a {} impossible : {}", cible.fournisseur, e),
                    ));
                    break;
                }
            }
        }
    }

    let (statut, message) = derniere.unwrap_or((
        StatusCode::BAD_GATEWAY,
        "Aucune cible configuree pour cet alias.".to_string(),
    ));

    // Toutes les cibles ont echoue : on relance une verification en tache de
    // fond (au plus une par minute) pour rafraichir l'etat repris.
    app.rafraichir_si_besoin();

    let statut_final = match statut {
        StatusCode::TOO_MANY_REQUESTS => StatusCode::TOO_MANY_REQUESTS,
        StatusCode::PAYMENT_REQUIRED => StatusCode::PAYMENT_REQUIRED,
        StatusCode::GATEWAY_TIMEOUT => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    };

    erreur_json(
        statut_final,
        &format!(
            "Alias '{}' : toutes les cibles ont echoue. Derniere erreur : {}",
            alias.nom, message
        ),
        "upstream_failed",
    )
}

/// Bornes de securite appliquees a un flux SSE.
#[derive(Clone, Copy)]
struct BornesFlux {
    inactivite: Duration,
    duree_max: Duration,
}

impl Default for BornesFlux {
    fn default() -> Self {
        Self {
            inactivite: FLUX_INACTIVITE,
            duree_max: FLUX_DUREE_MAX,
        }
    }
}

/// Evenement SSE d'erreur au format OpenAI.
fn sse_erreur(type_code: &str, message: &str) -> Bytes {
    let charge = json!({
        "error": {
            "message": message,
            "type": type_code,
            "code": type_code
        }
    });
    Bytes::from(format!("data: {}\n\n", charge))
}

/// Enveloppe le flux amont : coupe apres `inactivite` sans octet, et apres
/// `duree_max` au total. A l'expiration, un evenement SSE d'erreur au format
/// OpenAI est emis, puis le flux se ferme proprement (pas de tache orpheline).
/// Taille de la fenetre glissante gardee pour retrouver l'evenement `usage`.
const TAMPON_FLUX_MAX: usize = 64 * 1024;

/// Rapport de fin de flux : ecrit la ligne de metriques du flux.
struct RapportFlux {
    metriques: Arc<crate::metriques::Metriques>,
    app: Arc<AppState>,
    mesure: Arc<std::sync::Mutex<Mesure>>,
    id: String,
    alias: String,
    debut: Instant,
    /// Taille du corps envoye au fournisseur, pour estimer l'entree (LOT 7).
    octets_entree: u64,
}

impl RapportFlux {
    /// Appelee une seule fois, quand le flux se termine. L'usage est lu dans
    /// la fenetre glissante ; absent, l'entree et la sortie sont estimees a
    /// partir des octets (LOT 7, M-1) : jamais zero quand du texte a circule.
    fn conclure(&self, tampon: &[u8], relayes: u64) {
        let m = self
            .mesure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut e = Enregistrement::nouveau(&self.id, &self.alias);
        if !m.horodatage.is_empty() {
            // LOT 7, m-1 : meme regle en flux, l'horodatage vient du depart.
            e.horodatage = m.horodatage.clone();
        }
        e.fournisseur = m.fournisseur;
        e.modele = m.modele;
        e.statut = 200;
        e.latence_ms = self.debut.elapsed().as_millis() as u64;
        e.tentative = m.tentative;
        e.flux = true;
        match crate::metriques::usage_dans_flux(&String::from_utf8_lossy(tampon)) {
            Some((entree, sortie, cache)) => {
                e.tokens_in = entree;
                e.tokens_out = sortie;
                e.tokens_cache = cache;
                e.estime = false;
            }
            None => {
                // Le flux relaye est du SSE : le total cache est deja dans
                // `relayes`, et l'entree vient du corps de la requete.
                e.tokens_in = crate::metriques::estimer_tokens(self.octets_entree);
                e.tokens_out = crate::metriques::estimer_tokens(relayes);
                e.estime = true;
            }
        }
        if self.metriques.enregistrer(&e).is_ok() {
            self.app
                .ajouter_utilisation(e.tokens_in.saturating_add(e.tokens_out));
        }
    }
}

/// Etat interne du flux borne.
struct EtatFlux {
    amont:
        std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    debut: Instant,
    termine: bool,
    etiquette: String,
    bornes: BornesFlux,
    journal: Option<Arc<dyn Fn(String) + Send + Sync>>,
    /// Garde le creneau du semaphore jusqu'a la fin du flux : jamais lu,
    /// seulement conserve en vie.
    _gardien: Option<tokio::sync::OwnedSemaphorePermit>,
    tampon: Vec<u8>,
    /// Octets relayes depuis le debut, hors fenetre glissante : sert a estimer
    /// quand le flux n'annonce pas d'usage (LOT 7, M-1).
    relayes: u64,
    rapport: Option<Arc<RapportFlux>>,
}

impl EtatFlux {
    /// Ecrit la ligne de metriques du flux, une seule fois.
    fn conclure(&mut self) {
        if let Some(rapport) = self.rapport.take() {
            rapport.conclure(&self.tampon, self.relayes);
        }
    }
}

impl Drop for EtatFlux {
    /// Filet de securite (LOT 7, m-2) : une coupure client fait tomber le flux
    /// sans passer par la fin propre. La ligne est ecrite ici ; `conclure`
    /// vide `rapport`, donc jamais deux ecritures.
    fn drop(&mut self) {
        self.conclure();
    }
}

// Signature historique conservee pour les tests du corps de flux ; le relais
// de production passe par `flux_borne_avec`, qui emporte le rapport de mesures.
#[cfg(test)]
fn flux_borne(
    reponse: reqwest::Response,
    etiquette: String,
    bornes: BornesFlux,
    journal: Option<Arc<dyn Fn(String) + Send + Sync>>,
    gardien: Option<tokio::sync::OwnedSemaphorePermit>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    flux_borne_avec(reponse, etiquette, bornes, journal, gardien, None)
}

fn flux_borne_avec(
    reponse: reqwest::Response,
    etiquette: String,
    bornes: BornesFlux,
    journal: Option<Arc<dyn Fn(String) + Send + Sync>>,
    gardien: Option<tokio::sync::OwnedSemaphorePermit>,
    rapport: Option<Arc<RapportFlux>>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    let debut = Instant::now();
    let etat = EtatFlux {
        amont: Box::pin(reponse.bytes_stream()),
        debut,
        termine: false,
        etiquette,
        bornes,
        journal,
        _gardien: gardien,
        tampon: Vec::new(),
        relayes: 0,
        rapport,
    };

    futures_util::stream::unfold(etat, move |mut etat| async move {
            if etat.termine {
                etat.conclure();
                return None;
            }
            let ecoule = etat.debut.elapsed();
            if ecoule >= etat.bornes.duree_max {
                if let Some(j) = &etat.journal {
                    j(format!(
                        "flux {} : duree totale maximale depassee ({} s), fermeture",
                        etat.etiquette,
                        etat.bornes.duree_max.as_secs()
                    ));
                }
                let evenement = sse_erreur(
                    "upstream_timeout",
                    "La duree totale maximale du flux est depassee.",
                );
                etat.termine = true;
                return Some((Ok(evenement), etat));
            }

            let reste = etat.bornes.duree_max - ecoule;
            let (delai, cause) = if reste < etat.bornes.inactivite {
                (reste, "duree")
            } else {
                (etat.bornes.inactivite, "inactivite")
            };
            match tokio::time::timeout(delai, etat.amont.next()).await {
                Ok(Some(Ok(octets))) => {
                    // LOT 7, M-1 : compteur cumule, independant de la fenetre.
                    etat.relayes += octets.len() as u64;
                    // LOT 5 : fenetre glissante pour retrouver l'evenement
                    // final `usage` du flux, sans garder tout le contenu.
                    if etat.rapport.is_some() {
                        etat.tampon.extend_from_slice(&octets);
                        if etat.tampon.len() > TAMPON_FLUX_MAX {
                            let surplus = etat.tampon.len() - TAMPON_FLUX_MAX / 2;
                            etat.tampon.drain(..surplus);
                        }
                    }
                    Some((Ok(octets), etat))
                }
                Ok(Some(Err(e))) => {
                    if let Some(j) = &etat.journal {
                        j(format!("flux {} : erreur amont ({})", etat.etiquette, e));
                    }
                    let evenement = sse_erreur(
                        "upstream_broken",
                        "Le flux amont s'est interrompu avant la fin.",
                    );
                    etat.termine = true;
                    Some((Ok(evenement), etat))
                }
                Ok(None) => {
                    etat.conclure();
                    None
                }
                Err(_) => {
                    if let Some(j) = &etat.journal {
                        if cause == "duree" {
                            j(format!(
                                "flux {} : duree totale maximale depassee ({} s), fermeture",
                                etat.etiquette,
                                etat.bornes.duree_max.as_secs()
                            ));
                        } else {
                            j(format!(
                                "flux {} : inactivite de {} s depassee, fermeture",
                                etat.etiquette,
                                etat.bornes.inactivite.as_secs()
                            ));
                        }
                    }
                    let evenement = sse_erreur(
                        "upstream_timeout",
                        "Aucun octet recu du fournisseur dans le delai imparti.",
                    );
                    etat.termine = true;
                    Some((Ok(evenement), etat))
                }
            }
    })
}

fn reponse_flux_avec(
    reponse: reqwest::Response,
    alias: &Alias,
    etiquette: &str,
    journal: Option<Arc<dyn Fn(String) + Send + Sync>>,
    gardien: tokio::sync::OwnedSemaphorePermit,
    rapport: Option<Arc<RapportFlux>>,
) -> Response {
    let flux = flux_borne_avec(
        reponse,
        etiquette.to_string(),
        BornesFlux::default(),
        journal,
        Some(gardien),
        rapport,
    );
    let mut sortie = Response::new(Body::from_stream(flux));
    let entetes = sortie.headers_mut();
    entetes.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    entetes.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    entetes.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    entetes.insert("x-apivault-alias", entete(&alias.nom));
    entetes.insert("x-apivault-cible", entete(etiquette));
    sortie
}

fn reponse_complete(octets: Bytes, alias: &Alias, etiquette: &str) -> Response {
    let mut valeur: Value = serde_json::from_slice(&octets).unwrap_or(Value::Null);
    if let Some(objet) = valeur.as_object_mut() {
        // Le client voit toujours le nom de l'alias, pas le modele reel.
        objet.insert("model".to_string(), json!(alias.nom));
    }
    let corps = serde_json::to_vec(&valeur).unwrap_or_else(|_| octets.to_vec());

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header("x-apivault-alias", entete(&alias.nom))
        .header("x-apivault-cible", entete(etiquette))
        .body(Body::from(corps))
        .unwrap()
}

fn reponse_amont(statut: StatusCode, texte: String) -> Response {
    if serde_json::from_str::<Value>(&texte).is_ok() {
        Response::builder()
            .status(statut)
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .body(Body::from(texte))
            .unwrap()
    } else {
        erreur_json(statut, &tronquer(&texte, 800), "upstream_error")
    }
}

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

fn json_reponse(statut: StatusCode, valeur: Value) -> Response {
    Response::builder()
        .status(statut)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(valeur.to_string()))
        .unwrap()
}

fn erreur_json(statut: StatusCode, message: &str, code: &str) -> Response {
    json_reponse(
        statut,
        json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "code": code,
            }
        }),
    )
}

fn erreur_jeton() -> Response {
    let mut reponse = erreur_json(
        StatusCode::UNAUTHORIZED,
        "Jeton absent ou invalide. En-tete attendu : Authorization: Bearer <jeton>",
        "invalid_api_key",
    );
    reponse.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"ApiVault_Proxy\""),
    );
    reponse
}

fn entete(valeur: &str) -> HeaderValue {
    HeaderValue::from_str(valeur).unwrap_or_else(|_| HeaderValue::from_static("-"))
}

/// Valide l'en-tete d'authentification contre le jeton attendu.
/// Fail-closed : un jeton attendu vide ne valide jamais rien.
fn jeton_valide(attendu: &str, entetes: &HeaderMap) -> bool {
    if attendu.trim().is_empty() {
        return false;
    }
    let brut = entetes
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .or_else(|| entetes.get("x-api-key").and_then(|v| v.to_str().ok()));
    let Some(brut) = brut else { return false };
    let brut = brut.trim();
    let jeton = brut
        .strip_prefix("Bearer ")
        .or_else(|| brut.strip_prefix("bearer "))
        .unwrap_or(brut)
        .trim();
    egalite_constante(jeton, attendu)
}

fn egalite_constante(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // Pas de retour anticipe : la longueur entre dans la difference et la
    // boucle parcourt toujours la plus grande des deux valeurs.
    let mut difference = if a.len() == b.len() { 0u8 } else { 1u8 };
    let max = a.len().max(b.len());
    for i in 0..max {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        difference |= x ^ y;
    }
    difference == 0
}

fn masquer(jeton: &str) -> String {
    let caracteres: Vec<char> = jeton.chars().collect();
    if caracteres.len() <= 12 {
        return "…".to_string();
    }
    let debut: String = caracteres.iter().take(10).collect();
    let fin: String = caracteres.iter().skip(caracteres.len() - 4).collect();
    format!("{}…{}", debut, fin)
}

fn tronquer(texte: &str, max: usize) -> String {
    let propre: String = texte.chars().take(max).collect();
    if texte.chars().count() > max {
        format!("{}…", propre)
    } else {
        propre
    }
}

/// Retire de l'historique les blocs de raisonnement non standards
/// (`thinking`, `redacted_thinking`) et les champs associes. Un message
/// assistant qui ne contient plus rien recoit un contenu texte vide.
fn normaliser_requete(corps: &mut Value) {
    if let Some(messages) = corps.get_mut("messages").and_then(|m| m.as_array_mut()) {
        for message in messages.iter_mut() {
            let Some(objet) = message.as_object_mut() else { continue };

            objet.remove("reasoning");
            objet.remove("reasoning_content");
            objet.remove("thinking_blocks");

            if let Some(contenu) = objet.get_mut("content") {
                if let Some(items) = contenu.as_array_mut() {
                    items.retain(|item| {
                        let type_item = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        !matches!(type_item, "thinking" | "redacted_thinking" | "reasoning")
                    });
                    if items.is_empty() {
                        *contenu = json!("");
                    }
                }
            }

            if objet.get("content").map(|c| c.is_null()).unwrap_or(false) {
                objet.insert("content".to_string(), json!(""));
            }
        }

        // Un message sans contenu ni appel d'outil fait echouer plusieurs
        // fournisseurs (Cohere notamment). On le retire de l'historique.
        let mut gardes: Vec<Value> = Vec::with_capacity(messages.len());
        for message in messages.iter() {
            if !message_vide(message) {
                gardes.push(message.clone());
            }
        }
        if !gardes.is_empty() && gardes.len() != messages.len() {
            *messages = gardes;
        }
    }

    if let Some(objet) = corps.as_object_mut() {
        // Parametre Anthropic : inutile pour les modeles OpenAI.
        objet.remove("thinking");
    }
}

fn message_vide(message: &Value) -> bool {
    let Some(objet) = message.as_object() else {
        return false;
    };

    let a_des_outils = objet
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if a_des_outils || objet.get("tool_call_id").is_some() {
        return false;
    }

    match objet.get("content") {
        None => true,
        Some(Value::Null) => true,
        Some(Value::String(texte)) => texte.trim().is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        _ => false,
    }
}

fn reessayable(statut: StatusCode) -> bool {
    matches!(
        statut.as_u16(),
        402 | 404 | 408 | 409 | 425 | 429 | 500..=599
    )
}

/// Un modele est « gratuit » quand OpenRouter le publie avec le suffixe
fn parametres_incompatibles(texte: &str) -> bool {
    let t = texte.to_lowercase();
    [
        "tool",
        "function call",
        "response_format",
        "json mode",
        "image",
        "vision",
        "unsupported",
        "not support",
    ]
    .iter()
    .any(|motif| t.contains(motif))
}

fn enlever_parametres_optionnels(corps: &mut Value) {
    if let Some(objet) = corps.as_object_mut() {
        for cle in [
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "functions",
            "function_call",
            "response_format",
        ] {
            objet.remove(cle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entetes_avec(valeur: &str) -> HeaderMap {
        let mut entetes = HeaderMap::new();
        entetes.insert(header::AUTHORIZATION, HeaderValue::from_str(valeur).unwrap());
        entetes
    }

    #[test]
    fn jeton_absent_dans_la_config_refuse_tout() {
        // Fail-closed : c'est le coeur du correctif de securite.
        assert!(!jeton_valide("", &HeaderMap::new()));
        assert!(!jeton_valide("   ", &entetes_avec("Bearer sk-apivault-test")));
    }

    #[test]
    fn requete_sans_entete_est_refusee() {
        assert!(!jeton_valide("sk-apivault-test", &HeaderMap::new()));
    }

    #[test]
    fn bon_jeton_est_accepte() {
        assert!(jeton_valide(
            "sk-apivault-test",
            &entetes_avec("Bearer sk-apivault-test")
        ));
        let mut x_api_key = HeaderMap::new();
        x_api_key.insert("x-api-key", HeaderValue::from_static("sk-apivault-test"));
        assert!(jeton_valide("sk-apivault-test", &x_api_key));
    }

    #[test]
    fn mauvais_jeton_est_refuse() {
        assert!(!jeton_valide(
            "sk-apivault-test",
            &entetes_avec("Bearer sk-apivault-autre")
        ));
        assert!(!jeton_valide("sk-apivault-test", &entetes_avec("")));
    }

    #[test]
    fn identifiant_de_requete_unique_et_hexadecimal() {
        let a = identifiant_requete();
        let b = identifiant_requete();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "recu : {a}");
        assert!(b.chars().all(|c| c.is_ascii_hexdigit()), "recu : {b}");
        assert_ne!(a, b, "deux requetes doivent porter deux ids differents");
    }

    #[tokio::test]
    async fn redirection_amont_non_suivie() {
        use axum::routing::get;

        let app = axum::Router::new()
            .route(
                "/redirige",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [(header::LOCATION, "http://127.0.0.1:1/ailleurs")],
                    )
                }),
            )
            .route("/ailleurs", get(|| async { "ne doit jamais etre atteint" }));

        let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = ecoute.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(ecoute, app).await;
        });

        let url = format!("http://{adresse}/redirige");
        let reponse = client_http().get(&url).send().await.unwrap();
        assert_eq!(reponse.status(), StatusCode::FOUND);
        let reponse = client_flux().get(&url).send().await.unwrap();
        assert_eq!(reponse.status(), StatusCode::FOUND);
    }

    async fn serveur_de_test(app: axum::Router) -> String {
        let ecoute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = ecoute.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(ecoute, app).await;
        });
        format!("http://{adresse}")
    }

    fn route_silencieuse(premier: bool) -> axum::Router {
        use axum::routing::get;
        axum::Router::new().route(
            "/flux",
            get(move || async move {
                let amorce = futures_util::stream::once(async move {
                    Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: {\"a\":1}\n\n"))
                });
                let corps = if premier {
                    let flux = amorce.chain(futures_util::stream::pending());
                    Body::from_stream(flux)
                } else {
                    Body::from_stream(futures_util::stream::pending::<
                        Result<Bytes, std::io::Error>,
                    >())
                };
                ([(header::CONTENT_TYPE, "text/event-stream")], corps)
            }),
        )
    }

    #[tokio::test]
    async fn flux_silencieux_coupe_et_signale() {
        let base = serveur_de_test(route_silencieuse(true)).await;
        let reponse = client_flux()
            .get(format!("{base}/flux"))
            .send()
            .await
            .unwrap();
        let bornes = BornesFlux {
            inactivite: Duration::from_millis(80),
            duree_max: Duration::from_secs(5),
        };
        let mut flux = Box::pin(flux_borne(reponse, "test".into(), bornes, None, None));

        // Le premier morceau passe tel quel.
        let premier = flux.next().await.unwrap().unwrap();
        assert_eq!(&premier[..], b"data: {\"a\":1}\n\n");

        // Puis l'inactivite coupe et un evenement d'erreur OpenAI est emis.
        let second = flux.next().await.unwrap().unwrap();
        let texte = String::from_utf8_lossy(&second);
        assert!(texte.starts_with("data: {"), "recu : {texte}");
        assert!(texte.contains("\"error\""), "recu : {texte}");
        assert!(texte.contains("upstream_timeout"), "recu : {texte}");
        assert!(flux.next().await.is_none(), "le flux doit se fermer");
    }

    #[tokio::test]
    async fn flux_total_borne_par_la_duree() {
        let base = serveur_de_test(route_silencieuse(false)).await;
        let reponse = client_flux()
            .get(format!("{base}/flux"))
            .send()
            .await
            .unwrap();
        let bornes = BornesFlux {
            inactivite: Duration::from_secs(3600),
            duree_max: Duration::from_millis(80),
        };
        let debut = Instant::now();
        let mut flux = Box::pin(flux_borne(reponse, "test".into(), bornes, None, None));
        let evenement = flux.next().await.unwrap().unwrap();
        let texte = String::from_utf8_lossy(&evenement).to_string();
        assert!(texte.contains("upstream_timeout"), "recu : {texte}");
        assert!(flux.next().await.is_none());
        assert!(
            debut.elapsed() < Duration::from_secs(3),
            "coupure trop tardive : {:?}",
            debut.elapsed()
        );
    }

    #[tokio::test]
    async fn flux_normal_transmet_les_octets() {
        use axum::routing::get;
        let app = axum::Router::new().route(
            "/ok",
            get(|| async { ([(header::CONTENT_TYPE, "text/event-stream")], "data: ok\n\n") }),
        );
        let base = serveur_de_test(app).await;
        let reponse = client_flux()
            .get(format!("{base}/ok"))
            .send()
            .await
            .unwrap();
        let mut flux = Box::pin(flux_borne(
            reponse,
            "test".into(),
            BornesFlux::default(),
            None,
            None,
        ));
        let mut total = Vec::new();
        while let Some(morceau) = flux.next().await {
            total.extend_from_slice(&morceau.unwrap());
        }
        assert_eq!(String::from_utf8_lossy(&total), "data: ok\n\n");
    }

    /// P6 : le diagnostic copiable ne doit jamais contenir une valeur de cle,
    /// ni une query string qui en porterait une.
    #[test]
    fn le_diagnostic_ne_contient_aucune_cle() {
        let mut cles = HashMap::new();
        cles.insert(
            "OPENROUTER_KEY".to_string(),
            "sk-or-v1-SECRET-ABC-123".to_string(),
        );
        let mut fournisseurs = HashMap::new();
        fournisseurs.insert(
            "openrouter".to_string(),
            Fournisseur {
                urls: vec!["https://openrouter.ai/api/v1?api_key=sk-or-v1-SECRET-ABC-123".to_string()],
                cle_env: "OPENROUTER_KEY".to_string(),
                entetes: vec![("x-custom".to_string(), "valeur".to_string())],
            },
        );
        let cfg = Config {
            port: 3100,
            jeton: "jeton-maitre-SECRET-999".to_string(),
            cles,
            alias: Vec::new(),
            fournisseurs,
            chemin_cles: std::path::PathBuf::from("C:/temp/cles.env"),
            modele_prefere: None,
            perso: Vec::new(),
            plafond_mensuel: 0,
        };
        let app = AppState::pour_test(cfg);
        // Une erreur amont peut recopier une cle : le diagnostic doit la masquer.
        if let Ok(mut journal) = app.journal.lock() {
            journal.push_back(
                "erreur amont : sk-or-v1-SECRET-ABC-123 / jeton-maitre-SECRET-999"
                    .to_string(),
            );
        }
        let diagnostic = app.diagnostic_json(3100, "jeton-maitre-SECRET-999", true, Some(3));
        let texte = diagnostic.to_string();

        assert!(
            !texte.contains("SECRET-ABC-123"),
            "une valeur de cle figure dans le diagnostic : {texte}"
        );
        assert!(
            !texte.contains("jeton-maitre-SECRET-999"),
            "le jeton maitre figure dans le diagnostic : {texte}"
        );
        assert!(
            !texte.contains("api_key="),
            "une query string figure dans le diagnostic : {texte}"
        );
        assert!(texte.contains("***"), "les secrets doivent etre masques");
        assert!(texte.contains("openrouter"));
        assert!(texte.contains("cle_presente"));
    }

    /// P8 : l'ajout d'un fournisseur ecrit dans cles.env sans perdre les
    /// commentaires, laisse un `.bak` et n'expose jamais la cle au journal.
    #[test]
    fn ajouter_fournisseur_preserve_le_fichier_et_le_secret() {
        let dossier = std::env::temp_dir().join(format!(
            "apivault-ajout-{}-{}",
            std::process::id(),
            ms_depuis_demarrage()
        ));
        std::fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join("cles.env");
        std::fs::write(
            &chemin,
            "\u{feff}# mon commentaire\r\nPROXY_MASTER_KEY=abc\r\n",
        )
        .unwrap();

        let cfg = Config {
            port: 3100,
            jeton: "abc".to_string(),
            cles: HashMap::new(),
            alias: Vec::new(),
            fournisseurs: HashMap::new(),
            chemin_cles: chemin.clone(),
            modele_prefere: None,
            perso: Vec::new(),
            plafond_mensuel: 0,
        };
        let app = std::sync::Arc::new(AppState::pour_test(cfg));
        app.ajouter_fournisseur(
            "Mistral Test",
            "https://api.mistral.ai/v1",
            "cle-SECRET-42",
            "mistral-large, mistral-small",
        )
        .unwrap();

        // LOT 4 : lecture brute en garde-fou (rien en clair, ni dans le
        // fichier ni dans sa sauvegarde), puis lecture metier via le coffre.
        // Les assertions ci-dessous sont inchangees.
        let brut = std::fs::read_to_string(&chemin).unwrap();
        assert!(brut.starts_with(coffre::EN_TETE));
        assert!(!brut.contains("cle-SECRET-42"));
        let chemin_bak = dossier.join("cles.env.bak");
        assert!(chemin_bak.exists(), "la sauvegarde .bak doit exister");
        let brut_bak = std::fs::read_to_string(&chemin_bak).unwrap();
        assert!(brut_bak.starts_with(coffre::EN_TETE));
        assert!(!brut_bak.contains("cle-SECRET-42"));

        let contenu = coffre::lire_texte(&chemin).unwrap();
        assert!(contenu.contains("# mon commentaire"));
        assert!(contenu.contains("PROXY_MASTER_KEY=abc"));
        assert!(contenu.contains("PROXY_FOURNISSEURS=mistral_test"));
        assert!(contenu.contains("MISTRAL_TEST_URLS=https://api.mistral.ai/v1"));
        assert!(contenu.contains("MISTRAL_TEST_MODELES=mistral-large,mistral-small"));
        assert!(contenu.contains("MISTRAL_TEST_KEY=cle-SECRET-42"));
        assert!(
            dossier.join("cles.env.bak").exists(),
            "la sauvegarde .bak doit exister"
        );
        assert_eq!(app.cle_fournisseur("mistral_test"), "cle-SECRET-42");

        let journal = app.lignes_journal().join("\n");
        assert!(
            !journal.contains("cle-SECRET-42"),
            "la cle ne doit jamais apparaitre dans le journal : {journal}"
        );
        let diagnostic = app.diagnostic_json(3100, "abc", true, None).to_string();
        assert!(!diagnostic.contains("cle-SECRET-42"));

        let _ = std::fs::remove_dir_all(&dossier);
    }

    /// A5 : un modele declare par deux alias n'apparait qu'une fois, mais la
    /// provenance (liste des alias) est conservee.
    #[test]
    fn a5_modele_en_double_une_seule_ligne_avec_provenance() {
        let alias = vec![
            Alias {
                nom: "defaut".to_string(),
                cibles: vec![Cible {
                    fournisseur: "openrouter".to_string(),
                    modele: "modele-x".to_string(),
                }],
            },
            Alias {
                nom: "rapide".to_string(),
                cibles: vec![
                    Cible {
                        fournisseur: "openrouter".to_string(),
                        modele: "modele-x".to_string(),
                    },
                    Cible {
                        fournisseur: "openrouter".to_string(),
                        modele: "modele-y".to_string(),
                    },
                ],
            },
        ];
        let modeles = modeles_par_fournisseur(&alias, "openrouter");
        assert_eq!(
            modeles.len(),
            2,
            "modele-x ne doit apparaitre qu'une fois : {modeles:?}"
        );
        let (_, provenance) = modeles
            .iter()
            .find(|(m, _)| m == "modele-x")
            .expect("modele-x present");
        assert!(provenance.contains(&"defaut".to_string()));
        assert!(provenance.contains(&"rapide".to_string()));
    }

    // --- LOT 3 : correctifs d'interface (A2, A3, A4) ---------------------
    // La page est embarquee a la compilation : ces tests echouent si les
    // libelles ou les conditions regressent.

    const PAGE: &str = include_str!("../../src/index.html");

    /// A2 : le nom du fournisseur est substitue dans la confirmation.
    #[test]
    fn a2_le_nom_est_substitue_dans_la_confirmation() {
        assert!(
            PAGE.contains(r#"t("fournisseur_ajoute", { nom })"#),
            "l'appel doit passer le nom en parametre"
        );
        assert!(
            !PAGE.contains(r#"t("fournisseur_ajoute")"#),
            "l'appel sans parametre laisse {{nom}} affiche"
        );
        assert!(
            PAGE.contains("fournisseur_ajoute: \"Fournisseur {nom} ajouté.\""),
            "le modele de phrase doit contenir {{nom}}"
        );
    }

    /// A3 : un separateur explicite entre l'etat et « non joignable ».
    #[test]
    fn a3_separateur_entre_etat_et_non_joignable() {
        assert!(
            PAGE.contains(r#"tdEtat.appendChild(document.createTextNode(" · "));"#)
                && PAGE.contains(r#"const injoignable = document.createElement("span");"#),
            "le badge « non joignable » doit etre precede d'un separateur"
        );
        assert!(
            !PAGE.contains("en attentenon joignable"),
            "les deux libelles ne doivent jamais etre colles"
        );
    }

    /// A4 : « Aucun modele disponible » seulement si la liste est vide.
    #[test]
    fn a4_aucun_modele_seulement_si_liste_vide() {
        assert!(
            PAGE.contains(r#"} else if (e.sains > 0) {"#)
                && PAGE.contains(r#"t("dernier_etat_connu", { n: e.sains })"#),
            "un etat repris doit afficher « dernier etat connu », pas « aucun modele »"
        );
    }

    /// B3 : une erreur amont contenant une cle est redigee dans la vue
    /// exposee a l'interface.
    #[test]
    fn b3_erreur_amont_redigee_avant_affichage() {
        let secret = "sk-or-v1-SECRET-B3-123456";
        let mut cles = HashMap::new();
        cles.insert("OPENROUTER_KEY".to_string(), secret.to_string());
        let cfg = Config {
            port: 3100,
            jeton: "jeton-test".to_string(),
            cles,
            alias: Vec::new(),
            fournisseurs: HashMap::new(),
            chemin_cles: std::path::PathBuf::from("C:/temp/cles.env"),
            modele_prefere: None,
            perso: Vec::new(),
            plafond_mensuel: 0,
        };
        let app = AppState::pour_test(cfg);
        let cible = Cible {
            fournisseur: "openrouter".to_string(),
            modele: "modele-x".to_string(),
        };
        app.pool.enregistrer_echec(
            &cible,
            Some(500),
            &format!(
                "amont 500 : https://openrouter.ai/v1?api_key={secret}&autre=visible Authorization: Bearer {secret}",
            ),
        );
        let alias = Alias {
            nom: "defaut".to_string(),
            cibles: vec![cible],
        };
        let vue = app.vue_redigee(&alias);
        let texte = serde_json::to_string(&vue).unwrap();

        assert!(!texte.contains(secret), "la cle fuit dans la vue : {texte}");
        assert!(!texte.contains("api_key=sk"), "le parametre d'URL fuit : {texte}");
        assert!(texte.contains("***"), "les secrets doivent etre masques : {texte}");
        assert!(
            texte.contains("visible"),
            "les parametres non sensibles restent lisibles : {texte}"
        );
        // Le texte brut, lui, n'est pas modifie (le pool garde la trace).
        assert!(app.pool.vue(&alias)[0]["erreur"]
            .as_str()
            .unwrap()
            .contains(secret));
    }
}
