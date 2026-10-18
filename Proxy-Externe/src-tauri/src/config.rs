//! Chargement de la configuration locale (fichier `cles.env`).
//!
//! v0.1 : la configuration reste un simple fichier texte, comme la version A,
//! pour que les deux projets restent interchangeables.
//!
//! LOT 4 : le fichier n'est plus jamais ecrit en clair. Toute ecriture passe
//! par `coffre` (DPAPI portee utilisateur, en-tete `APIVAULT-COFFRE-1`), y
//! compris la sauvegarde `cles.env.bak`. Un fichier encore en clair reste
//! lisible (migration), et il est remplace sur place au premier enregistrement.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::coffre;

#[derive(Clone, Debug, PartialEq)]
pub struct Cible {
    pub fournisseur: String,
    pub modele: String,
}

#[derive(Clone, Debug)]
pub struct Alias {
    pub nom: String,
    pub cibles: Vec<Cible>,
}

#[derive(Clone, Debug)]
pub struct Fournisseur {
    /// Adresses de base du fournisseur (une ligne = une adresse).
    /// La premiere est utilisee par defaut ; les suivantes servent de repli.
    pub urls: Vec<String>,
    pub cle_env: String,
    pub entetes: Vec<(String, String)>,
}

impl Fournisseur {
    /// Variable `cles.env` qui contient les adresses de ce fournisseur,
    /// par exemple `OPENROUTER_URLS`.
    pub fn variable_urls(&self) -> String {
        let base = self
            .cle_env
            .strip_suffix("_KEY")
            .unwrap_or(self.cle_env.as_str());
        format!("{}_URLS", base)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Perso {
    pub nom: String,
    pub urls: Vec<String>,
    pub modeles: Vec<String>,
}

impl Perso {
    /// Variable `cles.env` qui contient la cle de ce fournisseur.
    pub fn cle_env(&self) -> String {
        format!("{}_KEY", self.nom.to_uppercase())
    }
    pub fn variable_urls(&self) -> String {
        format!("{}_URLS", self.nom.to_uppercase())
    }
    pub fn variable_modeles(&self) -> String {
        format!("{}_MODELES", self.nom.to_uppercase())
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub jeton: String,
    pub cles: HashMap<String, String>,
    pub alias: Vec<Alias>,
    pub fournisseurs: HashMap<String, Fournisseur>,
    pub chemin_cles: PathBuf,
    /// Modele choisi dans l'interface. Il est place en tete des alias
    /// `defaut` et `rapide`, et en tete de `gratuit` uniquement s'il est gratuit.
    pub modele_prefere: Option<String>,
    /// Fournisseurs ajoutes depuis l'interface (`PROXY_FOURNISSEURS`).
    pub perso: Vec<Perso>,
}

impl Config {
    /// Charge la configuration. Cree un fichier modele si aucun n'existe.
    pub fn charger() -> Config {
        let chemin = trouver_fichier_cles();
        let mut brut = lire_fichier(&chemin);

        // Le modele n'est ecrit que si le fichier n'existe pas. Un fichier
        // present mais illisible (coffre corrompu, autre compte Windows) est
        // laisse intact : mieux vaut demarrer avec un jeton en memoire que
        // detruire les cles de l'utilisateur.
        if brut.is_none() && !chemin.exists() {
            let jeton = nouveau_jeton();
            let modele = format!(
                "# ApiVault_Proxy - cles locales (ne pas partager)\n\
                 PROXY_PORT=3100\n\
                 PROXY_MASTER_KEY={}\n\
                 # Cle OpenRouter (modeles gratuits + payants)\n\
                 OPENROUTER_KEY=\n\
                 # Cle DeepSeek (alias rapide)\n\
                 DEEPSEEK_KEY=\n\
                 # Cle NVIDIA (credits gratuits)\n\
                 NVIDIA_KEY=\n",
                jeton
            );
            let _ = coffre::ecrire_atomique(&chemin, &modele);
            brut = lire_fichier(&chemin);
        }

        let valeurs = brut.unwrap_or_default();

        let port = valeurs
            .get("PROXY_PORT")
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(3100);

        let mut jeton = valeurs.get("PROXY_MASTER_KEY").cloned().unwrap_or_default();
        if jeton.is_empty() {
            // Securite : jamais de passerelle sans jeton.
            jeton = nouveau_jeton();
            let _ = ajouter_ligne(&chemin, &format!("PROXY_MASTER_KEY={}", jeton));
        }

        let modele_prefere = valeurs
            .get("PROXY_MODELE_PREFERE")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let perso = construire_perso(&valeurs);

        Config {
            port,
            jeton,
            cles: valeurs.clone(),
            alias: construire_alias(modele_prefere.as_deref()),
            fournisseurs: construire_fournisseurs(&valeurs),
            chemin_cles: chemin,
            modele_prefere,
            perso,
        }
    }

    pub fn noms_alias(&self) -> Vec<String> {
        self.alias.iter().map(|a| a.nom.clone()).collect()
    }
}

/// Decoupe une liste d'adresses saisie par l'utilisateur (virgules, points
/// virgules ou retours a la ligne). Les entrees vides sont ignorees.
pub fn decouper_adresses(texte: &str) -> Vec<String> {
    texte
        .split(|c| c == ',' || c == ';' || c == '\n' || c == '\r')
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Lit `PROXY_FOURNISSEURS` (noms separes par des points-virgules) puis, pour
/// chacun, `<NOM>_URLS` et `<NOM>_MODELES`. La cle reste dans `<NOM>_KEY`.
fn construire_perso(valeurs: &HashMap<String, String>) -> Vec<Perso> {
    let Some(liste) = valeurs.get("PROXY_FOURNISSEURS") else {
        return Vec::new();
    };
    let mut resultat = Vec::new();
    for nom in liste.split([';', ',']) {
        let nom = nom.trim().to_lowercase();
        if nom.is_empty() {
            continue;
        }
        let urls = valeurs
            .get(&format!("{}_URLS", nom.to_uppercase()))
            .map(|v| decouper_adresses(v))
            .unwrap_or_default();
        let modeles = valeurs
            .get(&format!("{}_MODELES", nom.to_uppercase()))
            .map(|v| {
                v.split(',')
                    .map(|m| m.trim().to_string())
                    .filter(|m| !m.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if urls.is_empty() || modeles.is_empty() {
            continue;
        }
        resultat.push(Perso {
            nom,
            urls,
            modeles,
        });
    }
    resultat
}

fn construire_fournisseurs(valeurs: &HashMap<String, String>) -> HashMap<String, Fournisseur> {
    let mut m = HashMap::new();

    let mut ajouter = |nom: &str, defaut: &[&str], cle_env: &str, entetes: Vec<(String, String)>| {
        let variable = format!(
            "{}_URLS",
            cle_env.strip_suffix("_KEY").unwrap_or(cle_env)
        );
        let mut urls = valeurs
            .get(&variable)
            .map(|v| decouper_adresses(v))
            .unwrap_or_default();
        if urls.is_empty() {
            urls = defaut
                .iter()
                .map(|u| u.trim_end_matches('/').to_string())
                .collect();
        }
        m.insert(
            nom.to_string(),
            Fournisseur {
                urls,
                cle_env: cle_env.to_string(),
                entetes,
            },
        );
    };

    ajouter(
        "openrouter",
        &["https://openrouter.ai/api/v1"],
        "OPENROUTER_KEY",
        vec![
            ("HTTP-Referer".to_string(), "https://apivault.local".to_string()),
            ("X-Title".to_string(), "ApiVault_Proxy".to_string()),
        ],
    );

    // DeepSeek accepte les deux formes d'adresse ; la seconde sert de repli.
    ajouter(
        "deepseek",
        &["https://api.deepseek.com/v1", "https://api.deepseek.com"],
        "DEEPSEEK_KEY",
        vec![],
    );

    ajouter(
        "nvidia",
        &["https://integrate.api.nvidia.com/v1"],
        "NVIDIA_KEY",
        vec![],
    );

    // Fournisseurs ajoutes depuis l'interface : ils sont declares dans
    // PROXY_FOURNISSEURS et pointent vers <NOM>_URLS / <NOM>_KEY.
    for p in construire_perso(valeurs) {
        m.insert(
            p.nom.clone(),
            Fournisseur {
                urls: p.urls.clone(),
                cle_env: p.cle_env(),
                entetes: Vec::new(),
            },
        );
    }

    m
}

fn c(fournisseur: &str, modele: &str) -> Cible {
    Cible {
        fournisseur: fournisseur.to_string(),
        modele: modele.to_string(),
    }
}

/// Un modele `:free` est gratuit. Les autres sont consideres payants.
pub fn modele_gratuit(modele: &str) -> bool {
    modele.to_ascii_lowercase().ends_with(":free")
}

/// Place le modele prefere en tete d'un alias s'il y figure.
/// `seulement_gratuits` permet de ne pas polluer les alias gratuits avec un
/// modele payant.
fn prioriser(alias: &mut Alias, modele_prefere: Option<&str>, seulement_gratuits: bool) {
    let Some(modele) = modele_prefere else {
        return;
    };
    if modele.trim().is_empty() || (seulement_gratuits && !modele_gratuit(modele)) {
        return;
    }
    if let Some(position) = alias.cibles.iter().position(|c| c.modele == modele) {
        let cible = alias.cibles.remove(position);
        alias.cibles.insert(0, cible);
    }
}

/// Alias v0.1 : meme contrat que la version A, pour que la bascule
/// d'un client IA ne soit qu'un changement de base_url.
///
/// Gemma est volontairement en dernier : il renvoie souvent 429 cote
/// OpenRouter/Google. On le garde comme secours, pas comme premier choix.
pub fn construire_alias(modele_prefere: Option<&str>) -> Vec<Alias> {
    let mut alias = vec![
        Alias {
            nom: "gratuit".to_string(),
            cibles: vec![
                c("openrouter", "cohere/north-mini-code:free"),
                c("openrouter", "google/gemma-4-31b-it:free"),
            ],
        },
        Alias {
            nom: "gratuit-secours".to_string(),
            cibles: vec![
                c("openrouter", "cohere/north-mini-code:free"),
                c("openrouter", "google/gemma-4-31b-it:free"),
            ],
        },
        Alias {
            nom: "defaut".to_string(),
            cibles: vec![
                c("deepseek", "deepseek-chat"),
                c("openrouter", "google/gemma-4-31b-it:free"),
            ],
        },
        Alias {
            nom: "rapide".to_string(),
            cibles: vec![
                c("deepseek", "deepseek-chat"),
                c("openrouter", "google/gemma-4-31b-it:free"),
            ],
        },
    ];

    for a in &mut alias {
        match a.nom.as_str() {
            "gratuit" | "gratuit-secours" => prioriser(a, modele_prefere, true),
            "defaut" | "rapide" => prioriser(a, modele_prefere, false),
            _ => {}
        }
    }
    alias
}

fn nouveau_jeton() -> String {
    format!("sk-apivault-{}", uuid::Uuid::new_v4().simple())
}

fn dossier_executable() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn dossier_donnees() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("ApiVault_Proxy")
    } else {
        std::env::temp_dir().join("ApiVault_Proxy")
    }
}

/// Chemin normalise sans toucher au disque : `.` disparait, `..` remonte.
fn normaliser(chemin: &Path) -> PathBuf {
    let mut sortie = PathBuf::new();
    for composant in chemin.components() {
        match composant {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                sortie.pop();
            }
            autre => sortie.push(autre.as_os_str()),
        }
    }
    sortie
}

/// Dossier dont on accepte de restreindre les droits.
///
/// Rend `Some(dossier)` seulement si le fichier de cles vit dans le dossier de
/// donnees de l'application. Un fichier designe par `APIVAULT_CLE_ENV` peut
/// pointer ailleurs (Bureau, Documents, partage reseau) : ce dossier ne nous
/// appartient pas, on n'y touche pas.
pub fn dossier_a_restreindre(chemin_cles: &Path, dossier_donnees: &Path) -> Option<PathBuf> {
    let dossier = normaliser(chemin_cles.parent()?);
    let racine = normaliser(dossier_donnees);
    let racine_txt = racine.to_string_lossy().to_lowercase();
    if racine_txt.is_empty() {
        return None;
    }
    let dossier_txt = dossier.to_string_lossy().to_lowercase();
    let dedans = dossier_txt == racine_txt
        || dossier_txt.starts_with(&format!("{}{}", racine_txt, std::path::MAIN_SEPARATOR));
    if dedans {
        Some(dossier)
    } else {
        None
    }
}

/// Ordre de recherche :
/// 1. variable d'environnement APIVAULT_CLE_ENV
/// 2. dossier de l'executable puis 4 parents (mode developpement)
/// 3. %APPDATA%\ApiVault_Proxy\cles.env
fn trouver_fichier_cles() -> PathBuf {
    if let Ok(chemin) = std::env::var("APIVAULT_CLE_ENV") {
        let p = PathBuf::from(chemin);
        if p.is_file() {
            return p;
        }
    }

    let mut dossier = dossier_executable();
    for _ in 0..5 {
        let candidat = dossier.join("cles.env");
        if candidat.is_file() {
            return candidat;
        }
        match dossier.parent() {
            Some(parent) => dossier = parent.to_path_buf(),
            None => break,
        }
    }

    let donnees = dossier_donnees();
    let _ = std::fs::create_dir_all(&donnees);
    donnees.join("cles.env")
}

fn lire_fichier(chemin: &Path) -> Option<HashMap<String, String>> {
    // `coffre::lire_texte` dechiffre le fichier s'il porte l'en-tete du coffre,
    // et rend le contenu tel quel sinon : une installation encore en clair
    // reste donc lisible une derniere fois (migration).
    let contenu = match coffre::lire_texte(chemin) {
        Ok(contenu) => contenu,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                // Jamais de panique, et surtout on n'ecrase pas un coffre
                // illisible : l'utilisateur peut encore le mettre de cote.
                eprintln!(
                    "cles.env illisible ({}). Le fichier n'a pas ete modifie : mets-le de cote ou supprime-le pour repartir proprement.",
                    e
                );
            }
            return None;
        }
    };
    let mut map = HashMap::new();

    for ligne in contenu.lines() {
        let ligne = ligne.trim_start_matches('\u{feff}').trim();
        if ligne.is_empty() || ligne.starts_with('#') {
            continue;
        }
        // "export" tolere pour les fichiers venant d'un shell
        let ligne = ligne.strip_prefix("export ").unwrap_or(ligne);
        if let Some((cle, valeur)) = ligne.split_once('=') {
            let cle = cle.trim().to_string();
            let mut valeur = valeur.trim().to_string();
            if valeur.len() >= 2 {
                let premier = valeur.chars().next().unwrap();
                let dernier = valeur.chars().last().unwrap();
                if (premier == '"' && dernier == '"') || (premier == '\'' && dernier == '\'') {
                    valeur = valeur[1..valeur.len() - 1].to_string();
                }
            }
            if !cle.is_empty() {
                map.insert(cle, valeur);
            }
        }
    }

    Some(map)
}

fn ajouter_ligne(chemin: &Path, ligne: &str) -> std::io::Result<()> {
    // Plus d'ajout en fin de fichier : le fichier est relu (dechiffre si
    // besoin), complete, puis reecrit chiffre et atomiquement.
    let mut contenu = match coffre::lire_texte(chemin) {
        Ok(contenu) => contenu,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if !contenu.is_empty() && !contenu.ends_with('\n') {
        contenu.push_str("\r\n");
    }
    contenu.push_str(ligne);
    contenu.push_str("\r\n");
    coffre::ecrire_atomique(chemin, &contenu)
}

/// Remplace la valeur d'une variable dans le fichier de cles, en conservant
/// les commentaires et les autres lignes. La variable est ajoutee si absente.
pub fn ecrire_variable(chemin: &Path, variable: &str, valeur: &str) -> std::io::Result<()> {
    ecrire_variable_options(chemin, variable, valeur, true)
}

/// Identique a `ecrire_variable`, mais permet de ne pas ecraser la sauvegarde
/// `.bak` : utile quand plusieurs variables sont ecrites d'un seul geste.
pub fn ecrire_variable_sans_sauvegarde(
    chemin: &Path,
    variable: &str,
    valeur: &str,
) -> std::io::Result<()> {
    ecrire_variable_options(chemin, variable, valeur, false)
}

fn ecrire_variable_options(
    chemin: &Path,
    variable: &str,
    valeur: &str,
    sauvegarde: bool,
) -> std::io::Result<()> {
    // Une cle qui change (rotation) ou qui disparait (effacement) ne doit pas
    // survivre dans la sauvegarde : la generation precedente est supprimee.
    let ancienne = lire_fichier(chemin)
        .and_then(|valeurs| valeurs.get(variable).cloned())
        .unwrap_or_default();
    let secrete = variable == "PROXY_MASTER_KEY" || variable.ends_with("_KEY");
    let nouvelle = valeur.trim();
    let effacement = secrete && !ancienne.is_empty() && nouvelle.is_empty();
    let rotation = secrete && !ancienne.is_empty() && !nouvelle.is_empty() && ancienne != valeur;
    let sauvegarde_proscrite = effacement || rotation;
    let sauvegarde_bak = chemin.with_extension("env.bak");

    // Sauvegarde de la version precedente (une seule generation) : une valeur
    // ecrasee par erreur reste recuperable dans cles.env.bak. La copie passe
    // par le coffre : meme la sauvegarde ne contient jamais de cle en clair.
    if sauvegarde && chemin.exists() && !sauvegarde_proscrite {
        let _ = coffre::copier_chiffre(chemin, &sauvegarde_bak);
    }
    if sauvegarde_proscrite {
        // Efface aussi une sauvegarde laissee par une generation anterieure.
        let _ = std::fs::remove_file(&sauvegarde_bak);
    }

    // Relecture dechiffree : un fichier encore en clair (installation
    // anterieure) est accepte et sera reecrit chiffre juste apres.
    let contenu = match coffre::lire_texte(chemin) {
        Ok(contenu) => contenu,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let prefixe = format!("{}=", variable);
    let mut lignes: Vec<String> = Vec::new();
    let mut trouve = false;

    for ligne in contenu.lines() {
        let debut = ligne.trim_start();
        if debut.starts_with(&prefixe) || debut.starts_with(&format!("export {}", prefixe)) {
            lignes.push(format!("{}={}", variable, valeur));
            trouve = true;
        } else {
            lignes.push(ligne.to_string());
        }
    }

    if !trouve {
        lignes.push(format!("{}={}", variable, valeur));
    }

    let mut texte = lignes.join("\r\n");
    texte.push_str("\r\n");
    // Ecriture atomique (temporaire dans le meme dossier puis renommage) et
    // toujours chiffree : plus aucune cle en clair sur le disque, ni dans le
    // fichier, ni dans sa sauvegarde.
    coffre::ecrire_atomique(chemin, &texte)
}

/// Persiste le modele prefere dans le fichier de cles. Une valeur vide
/// supprime la preference.
pub fn sauver_modele_prefere(chemin: &Path, modele: Option<&str>) -> std::io::Result<()> {
    let valeur = modele
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .unwrap_or("");
    ecrire_variable(chemin, "PROXY_MODELE_PREFERE", valeur)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alias<'a>(alias: &'a [Alias], nom: &str) -> &'a Alias {
        alias.iter().find(|a| a.nom == nom).expect("alias absent")
    }

    #[test]
    fn gemma_reste_present_mais_en_dernier() {
        let liste = construire_alias(None);
        for nom in ["gratuit", "gratuit-secours", "defaut", "rapide"] {
            let a = alias(&liste, nom);
            assert_eq!(
                a.cibles.last().map(|c| c.modele.as_str()),
                Some("google/gemma-4-31b-it:free"),
                "gemma doit rester en dernier dans {}",
                nom
            );
        }
    }

    #[test]
    fn modele_prefere_gratuit_passe_devant() {
        let liste = construire_alias(Some("google/gemma-4-31b-it:free"));
        for nom in ["gratuit", "gratuit-secours", "defaut", "rapide"] {
            assert_eq!(
                alias(&liste, nom).cibles[0].modele,
                "google/gemma-4-31b-it:free"
            );
        }
    }

    #[test]
    fn modele_prefere_payant_ne_pollue_pas_les_alias_gratuits() {
        let liste = construire_alias(Some("deepseek-chat"));
        assert_eq!(alias(&liste, "defaut").cibles[0].modele, "deepseek-chat");
        assert_eq!(alias(&liste, "rapide").cibles[0].modele, "deepseek-chat");
        assert!(
            alias(&liste, "gratuit")
                .cibles
                .iter()
                .all(modele_gratuit_cible),
            "un modele payant ne doit pas entrer dans gratuit"
        );
        assert!(
            alias(&liste, "gratuit-secours")
                .cibles
                .iter()
                .all(modele_gratuit_cible),
            "un modele payant ne doit pas entrer dans gratuit-secours"
        );
    }

    fn modele_gratuit_cible(c: &Cible) -> bool {
        modele_gratuit(&c.modele)
    }

    #[test]
    fn decoupage_des_adresses() {
        assert_eq!(
            decouper_adresses(" https://a.tld/v1/ , https://b.tld\nhttps://c.tld "),
            vec![
                "https://a.tld/v1".to_string(),
                "https://b.tld".to_string(),
                "https://c.tld".to_string()
            ]
        );
    }

    #[test]
    fn detection_gratuit() {
        assert!(modele_gratuit("cohere/north-mini-code:free"));
        assert!(!modele_gratuit("deepseek-chat"));
    }

    #[test]
    fn sauvegarde_du_modele_prefere() {
        let dossier = std::env::temp_dir().join(format!(
            "apivault-test-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dossier).unwrap();
        let chemin = dossier.join("cles.env");
        std::fs::write(&chemin, "PROXY_PORT=3100\r\nPROXY_MASTER_KEY=abc\r\n").unwrap();

        sauver_modele_prefere(&chemin, Some("deepseek-chat")).unwrap();
        // LOT 4 : le format sur disque est desormais chiffre. La lecture brute
        // reste en garde-fou (rien en clair), la lecture metier passe par le
        // coffre. Les assertions metier ci-dessous sont inchangees.
        let brut = std::fs::read_to_string(&chemin).unwrap();
        assert!(brut.starts_with(crate::coffre::EN_TETE));
        assert!(!brut.contains("abc"));
        let texte = crate::coffre::lire_texte(&chemin).unwrap();
        assert!(texte.contains("PROXY_MODELE_PREFERE=deepseek-chat"));
        assert!(texte.contains("PROXY_MASTER_KEY=abc"));

        sauver_modele_prefere(&chemin, None).unwrap();
        let brut = std::fs::read_to_string(&chemin).unwrap();
        assert!(brut.starts_with(crate::coffre::EN_TETE));
        assert!(!brut.contains("abc"));
        let texte = crate::coffre::lire_texte(&chemin).unwrap();
        assert!(texte.contains("PROXY_MODELE_PREFERE="));
        assert!(!texte.contains("PROXY_MODELE_PREFERE=deepseek-chat"));

        std::fs::remove_dir_all(&dossier).unwrap();
    }
}
