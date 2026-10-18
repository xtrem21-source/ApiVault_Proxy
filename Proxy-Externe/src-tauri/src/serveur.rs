//! Passerelle locale : serveur HTTP compatible OpenAI.
//!
//! v0.1 : un alias resolu vers une ou plusieurs cibles, diffusion du flux
//! (SSE) telle quelle, aucune intelligence de pool (elle arrive en v0.2).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path as AxumPath, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::config::{
    construire_alias, decouper_adresses, dossier_donnees, ecrire_variable, modele_gratuit,
    sauver_modele_prefere, Alias, Cible, Config,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Inactivite maximale toleree entre deux octets du flux amont.
const FLUX_INACTIVITE: Duration = Duration::from_secs(120);
/// Duree totale maximale d'un flux, en-tetes comprises.
const FLUX_DUREE_MAX: Duration = Duration::from_secs(900);
/// Fenetre totale accordee aux tentatives et aux bascules (en-tetes comprises).
const BUDGET_BASCULES: Duration = Duration::from_secs(45);
/// Duree maximale d'une requete complete, flux compris.
const BUDGET_REQUETE: Duration = Duration::from_secs(900);
/// Plafond absolu d'une tentative isolee, meme si le budget de bascule le permet.
const DELAI_CIBLE: Duration = Duration::from_secs(300);
/// Au plus 3 cibles essayees : 1 tentative + 2 bascules.
const CIBLES_MAX: usize = 3;

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
        }
    }

    /// Cle actuellement utilisee pour un fournisseur (vide si absente).
    pub fn cle_fournisseur(&self, nom: &str) -> String {
        let Some(fournisseur) = self.cfg.fournisseurs.get(nom) else {
            return String::new();
        };
        self.cles
            .lock()
            .ok()
            .and_then(|c| c.get(&fournisseur.cle_env).cloned())
            .unwrap_or_default()
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
        self.cfg
            .fournisseurs
            .get(nom)
            .map(|f| f.urls.clone())
            .unwrap_or_default()
    }

    /// Alias avec le modele prefere place en tete.
    pub fn alias_effectifs(&self) -> Vec<Alias> {
        let prefere = self
            .preference_modele
            .lock()
            .ok()
            .and_then(|p| p.clone());
        construire_alias(prefere.as_deref())
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
            .cfg
            .fournisseurs
            .iter()
            .map(|(nom, f)| {
                let cle = self.cle_fournisseur(nom);
                json!({
                    "id": nom,
                    "cle_env": f.cle_env,
                    "variable_urls": f.variable_urls(),
                    "cle_presente": !cle.is_empty(),
                    "cle_masquee": if cle.is_empty() { String::new() } else { masquer(&cle) },
                    "urls": self.adresses_fournisseur(nom),
                    "cibles": alias
                        .iter()
                        .flat_map(|a| a.cibles.iter())
                        .filter(|c| c.fournisseur == *nom)
                        .map(|c| c.modele.clone())
                        .collect::<Vec<_>>(),
                    // Detail par modele : le panneau affiche un badge
                    // gratuit / payant et peut tester le fournisseur d'un coup.
                    "modeles": alias
                        .iter()
                        .flat_map(|a| a.cibles.iter())
                        .filter(|c| c.fournisseur == *nom)
                        .map(|c| {
                            json!({
                                "modele": c.modele,
                                "gratuit": modele_gratuit(&c.modele),
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
        let Some(fournisseur) = self.cfg.fournisseurs.get(nom) else {
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
        let Some(fournisseur) = self.cfg.fournisseurs.get(nom) else {
            return Err(format!("Fournisseur inconnu : {}", nom));
        };
        let urls = decouper_adresses(texte);
        if urls.is_empty() {
            return Err("Au moins une adresse est necessaire.".to_string());
        }
        for url in &urls {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!(
                    "Adresse invalide : '{}' (elle doit commencer par http:// ou https://)",
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

    pub fn autorise(&self, entetes: &HeaderMap) -> bool {
        // Fail-closed : sans jeton configure, aucune requete n'est acceptee.
        // L'ancien comportement (fail-open) laissait la passerelle ouverte a
        // quiconque lorsque PROXY_MASTER_KEY etait vide.
        jeton_valide(&self.cfg.jeton, entetes)
    }

    pub fn journaliser(&self, message: &str) {
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
                let _ = std::fs::write(chemin, b"");
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
}

pub struct Service {
    pub port: u16,
    pub jeton: String,
    pub app: Arc<AppState>,
    pub actif: AtomicBool,
    interface_vue: AtomicBool,
    stop: Mutex<Option<watch::Sender<bool>>>,
    erreur: Mutex<Option<String>>,
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
        if let Ok(mut garde) = self.stop.lock() {
            *garde = Some(tx);
        }
        if let Ok(mut e) = self.erreur.lock() {
            *e = None;
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
                        "en ecoute sur http://127.0.0.1:{} - {} alias",
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
                    let message = format!(
                        "Impossible d'ecouter sur 127.0.0.1:{} : {} (port deja utilise ?)",
                        srv.port, e
                    );
                    if let Ok(mut err) = srv.erreur.lock() {
                        *err = Some(message.clone());
                    }
                    srv.app.journaliser(&message);
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
            "depuis_s": self.depuis.lock().ok().and_then(|d| d.map(|i| i.elapsed().as_secs())),
            "version": VERSION,
            // « connecte » ne dit pas que le service tourne, mais qu'au moins
            // un fournisseur a repondu.
            "connecte": self.actif.load(Ordering::SeqCst) && self.app.pool.a_un_sain(),
            "sains": self.app.pool.compter_sains(),
            "sondage": self.app.sondage.load(Ordering::SeqCst),
            "derniere_reussite_s": self.app.pool.derniere_reussite_s(),
            // Etat repris de la session precedente : age de la derniere
            // verification enregistree sur le disque (aucune sonde au demarrage).
            "derniere_verification_s": self.app.pool.derniere_verification_s(),
            "demarrage_s": self.app.debut.elapsed().as_secs(),
            "chemin_cles": self.app.cfg.chemin_cles.display().to_string(),
            "alias": alias.iter().map(|a| json!({
                "nom": a.nom,
                "cibles": self.app.pool.vue(a),
            })).collect::<Vec<_>>(),
        })
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
        .route("/admin/requalifier", post(pool_requalifier))
        .route("/admin/rafraichir", post(pool_rafraichir))
        .fallback(route_inconnue)
        .with_state(app)
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

async fn sante(AxumState(app): AxumState<Arc<AppState>>) -> Response {
    json_reponse(
        StatusCode::OK,
        json!({
            "status": "ok",
            "service": "apivault-proxy",
            "version": VERSION,
            "uptime_s": app.debut.elapsed().as_secs(),
            "alias": app.cfg.noms_alias(),
            "cles_chargees": app.cfg.cles.keys().filter(|k| k.ends_with("_KEY")).count(),
        }),
    )
}

async fn modeles(AxumState(app): AxumState<Arc<AppState>>, entetes: HeaderMap) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }
    let data: Vec<Value> = app
        .cfg
        .alias
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
    if !app.cfg.alias.iter().any(|a| a.nom == id) {
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
        .cfg
        .fournisseurs
        .get(nom_fournisseur)
        .map(|f| f.entetes.clone())
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
    let Some(fournisseur) = app.cfg.fournisseurs.get(&cible.fournisseur) else {
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
    match poster(
        app,
        &app.http,
        &cible.fournisseur,
        &cle,
        &corps,
        Some(std::time::Duration::from_secs(25)),
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
                "cibles": app.pool.vue(a),
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
        serde_json::from_slice(&corps).unwrap_or_else(|_| json!({}))
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

// ---------------------------------------------------------------------------
// Chat completions
// ---------------------------------------------------------------------------

async fn chat(
    AxumState(app): AxumState<Arc<AppState>>,
    entetes: HeaderMap,
    corps: Bytes,
) -> Response {
    if !app.autorise(&entetes) {
        return erreur_jeton();
    }

    let mut payload: Value = match serde_json::from_slice(&corps) {
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

    if payload.get("messages").is_none() {
        return erreur_json(
            StatusCode::BAD_REQUEST,
            "Champ 'messages' obligatoire.",
            "invalid_request",
        );
    }

    let flux = payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut derniere: Option<(StatusCode, String)> = None;
    let debut_requete = Instant::now();
    let ordre = app.pool.ordre(&alias);

    if ordre.is_empty() {
        let resume = app.pool.resume(&alias);
        app.journaliser(&format!(
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

    // Budget : au plus 3 cibles (1 tentative + 2 bascules) et 45 s de bascule.
    for (_rang, cible) in ordre.iter().take(CIBLES_MAX).enumerate() {
        let restant_bascule = BUDGET_BASCULES.saturating_sub(debut_requete.elapsed());
        if restant_bascule.is_zero() {
            app.journaliser(&format!(
                "alias={} : budget de {} s epuise, arret des bascules",
                alias.nom,
                BUDGET_BASCULES.as_secs()
            ));
            derniere = Some((
                StatusCode::GATEWAY_TIMEOUT,
                "Budget de bascule epuise.".to_string(),
            ));
            break;
        }
        let debut_cible = Instant::now();
        let Some(fournisseur) = app.cfg.fournisseurs.get(&cible.fournisseur) else {
            app.journaliser(&format!(
                "fournisseur inconnu : {} (cible ignoree)",
                cible.fournisseur
            ));
            continue;
        };

        let cle = app.cle_fournisseur(&cible.fournisseur);
        if cle.is_empty() {
            let message = format!("Cle {} absente du fichier cles.env", fournisseur.cle_env);
            app.journaliser(&message);
            derniere = Some((StatusCode::INTERNAL_SERVER_ERROR, message));
            continue;
        }

        let mut corps_up = payload.clone();
        corps_up["model"] = json!(cible.modele);
        let mut nettoye = false;

        loop {
            let client = if flux { &app.http_flux } else { &app.http };
            // Chaque tentative est plafonnee : min(budget de bascule restant, 300 s).
            let restant_bascule = BUDGET_BASCULES.saturating_sub(debut_requete.elapsed());
            if restant_bascule.is_zero() {
                app.journaliser(&format!(
                    "alias={} : budget de {} s epuise avant {}/{}",
                    alias.nom,
                    BUDGET_BASCULES.as_secs(),
                    cible.fournisseur,
                    cible.modele
                ));
                derniere = Some((
                    StatusCode::GATEWAY_TIMEOUT,
                    "Budget de bascule epuise.".to_string(),
                ));
                break;
            }
            let delai_tentative = restant_bascule.min(DELAI_CIBLE);
            match tokio::time::timeout(
                delai_tentative,
                poster(&app, client, &cible.fournisseur, &cle, &corps_up, None),
            )
            .await
            {
                Err(_) => {
                    app.journaliser(&format!(
                        "alias={} cible={}/{} : delai de {} s depasse",
                        alias.nom,
                        cible.fournisseur,
                        cible.modele,
                        delai_tentative.as_secs()
                    ));
                    app.pool.enregistrer_echec(cible, Some(504), "delai depasse");
                    derniere = Some((
                        StatusCode::GATEWAY_TIMEOUT,
                        format!(
                            "Delai depasse vers {}/{}.",
                            cible.fournisseur, cible.modele
                        ),
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
                            app.journaliser(&format!(
                                "alias={} -> {} flux (200)",
                                alias.nom, etiquette
                            ));
                            let journal_app = Arc::clone(&app);
                            let journal: Option<Arc<dyn Fn(String) + Send + Sync>> =
                                Some(Arc::new(move |m: String| journal_app.journaliser(&m)));
                            return reponse_flux(reponse, &alias, &etiquette, journal);
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
                                app.journaliser(&format!(
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
                        app.journaliser(&format!(
                            "alias={} -> {} complet ({} octets)",
                            alias.nom,
                            etiquette,
                            octets.len()
                        ));
                        return reponse_complete(octets, &alias, &etiquette);
                    }

                    let texte = reponse.text().await.unwrap_or_default();
                    let court = tronquer(&texte, 500);
                    app.journaliser(&format!(
                        "alias={} cible={}/{} HTTP {}{} : {}",
                        alias.nom,
                        cible.fournisseur,
                        cible.modele,
                        statut.as_u16(),
                        if statut == StatusCode::UNAUTHORIZED || statut == StatusCode::FORBIDDEN {
                            " (cle invalide : fournisseur condamne)"
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
                        app.journaliser(
                            "400 sur un parametre optionnel : nouvel essai sans tools/response_format",
                        );
                        continue;
                    }

                    if reessayable(statut) {
                        derniere = Some((statut, court));
                        break;
                    }

                    return reponse_amont(statut, texte);
                }
                Ok(Err(e)) => {
                    app.journaliser(&format!(
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
fn flux_borne(
    reponse: reqwest::Response,
    etiquette: String,
    bornes: BornesFlux,
    journal: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    let debut = Instant::now();
    let amont = reponse.bytes_stream();
    let etat = (amont, debut, false, etiquette, bornes, journal);

    futures_util::stream::unfold(
        etat,
        move |(mut amont, debut, termine, etiquette, bornes, journal)| async move {
            if termine {
                return None;
            }
            let ecoule = debut.elapsed();
            if ecoule >= bornes.duree_max {
                if let Some(j) = &journal {
                    j(format!(
                        "flux {} : duree totale maximale depassee ({} s), fermeture",
                        etiquette,
                        bornes.duree_max.as_secs()
                    ));
                }
                let evenement = sse_erreur(
                    "upstream_timeout",
                    "La duree totale maximale du flux est depassee.",
                );
                return Some((
                    Ok(evenement),
                    (amont, debut, true, etiquette, bornes, journal),
                ));
            }

            let reste = bornes.duree_max - ecoule;
            let (delai, cause) = if reste < bornes.inactivite {
                (reste, "duree")
            } else {
                (bornes.inactivite, "inactivite")
            };
            match tokio::time::timeout(delai, amont.next()).await {
                Ok(Some(Ok(octets))) => {
                    Some((Ok(octets), (amont, debut, false, etiquette, bornes, journal)))
                }
                Ok(Some(Err(e))) => {
                    if let Some(j) = &journal {
                        j(format!("flux {} : erreur amont ({})", etiquette, e));
                    }
                    let evenement = sse_erreur(
                        "upstream_broken",
                        "Le flux amont s'est interrompu avant la fin.",
                    );
                    Some((
                        Ok(evenement),
                        (amont, debut, true, etiquette, bornes, journal),
                    ))
                }
                Ok(None) => None,
                Err(_) => {
                    if let Some(j) = &journal {
                        if cause == "duree" {
                            j(format!(
                                "flux {} : duree totale maximale depassee ({} s), fermeture",
                                etiquette,
                                bornes.duree_max.as_secs()
                            ));
                        } else {
                            j(format!(
                                "flux {} : inactivite de {} s depassee, fermeture",
                                etiquette,
                                bornes.inactivite.as_secs()
                            ));
                        }
                    }
                    let evenement = sse_erreur(
                        "upstream_timeout",
                        "Aucun octet recu du fournisseur dans le delai imparti.",
                    );
                    Some((
                        Ok(evenement),
                        (amont, debut, true, etiquette, bornes, journal),
                    ))
                }
            }
        },
    )
}

fn reponse_flux(
    reponse: reqwest::Response,
    alias: &Alias,
    etiquette: &str,
    journal: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Response {
    let flux = flux_borne(
        reponse,
        etiquette.to_string(),
        BornesFlux::default(),
        journal,
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
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for i in 0..a.len() {
        difference |= a[i] ^ b[i];
    }
    difference == 0
}

fn masquer(jeton: &str) -> String {
    if jeton.len() <= 12 {
        return "…".to_string();
    }
    format!("{}…{}", &jeton[..10], &jeton[jeton.len() - 4..])
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
        let mut flux = Box::pin(flux_borne(reponse, "test".into(), bornes, None));

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
        let mut flux = Box::pin(flux_borne(reponse, "test".into(), bornes, None));
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
        ));
        let mut total = Vec::new();
        while let Some(morceau) = flux.next().await {
            total.extend_from_slice(&morceau.unwrap());
        }
        assert_eq!(String::from_utf8_lossy(&total), "data: ok\n\n");
    }
}
