//! Coffre local des cles : chiffrement DPAPI, portee UTILISATEUR.
//!
//! Format sur disque, une cle par fichier :
//!
//! ```text
//! APIVAULT-COFFRE-1
//! <blob DPAPI encode en base64>
//! ```
//!
//! - en-tete present -> on dechiffre ;
//! - en-tete absent  -> on lit comme du texte simple : c'est le chemin de
//!   migration des installations anterieures (cles.env en clair). Elles sont
//!   relues telles quelles une derniere fois, puis reecrites chiffrees au
//!   premier enregistrement.
//!
//! Portee UTILISATEUR (pas MACHINE) : le blob n'est dechiffrable que par le
//! compte Windows qui l'a produit, sur cette machine. C'est ce qui protege la
//! cle meme si quelqu'un copie le fichier.
//!
//! LIMITE HONNETE, a ne pas oublier : ecrire « atomiquement » (fichier
//! temporaire puis renommage) remplace le fichier, mais n'efface pas
//! physiquement les blocs sur un SSD. L'ancien contenu en clair peut rester
//! recuperable par un outil d'analyse disque jusqu'a ce que le controleur
//! reutilise ces pages. Le coffre protege le fichier courant, pas l'historique
//! du disque.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;

/// Premiere ligne d'un fichier chiffre par le coffre.
pub const EN_TETE: &str = "APIVAULT-COFFRE-1";

/// Erreur du coffre. Aucun de ces cas ne panique : ils sont rendus a
/// l'appelant pour qu'il decide (journal, message a l'ecran, abandon).
#[derive(Debug)]
pub enum ErreurCoffre {
    /// DPAPI (portee utilisateur) a refuse l'operation.
    Dpapi {
        operation: &'static str,
        code: i32,
    },
    /// En-tete present, mais le reste n'est pas du base64 exploitable.
    Base64(String),
    /// Le contenu dechiffre n'est pas du texte UTF-8.
    Texte(String),
}

impl std::fmt::Display for ErreurCoffre {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurCoffre::Dpapi { operation, code } => write!(
                f,
                "{} refusee par Windows (code {}). Le coffre appartient-il a un autre compte Windows ?",
                operation, code
            ),
            ErreurCoffre::Base64(detail) => write!(
                f,
                "coffre corrompu : le bloc chiffre n'est pas du base64 valide ({})",
                detail
            ),
            ErreurCoffre::Texte(detail) => {
                write!(f, "coffre corrompu : contenu dechiffre illisible ({})", detail)
            }
        }
    }
}

impl std::error::Error for ErreurCoffre {}

/// Vrai si le texte commence par l'en-tete du coffre.
pub fn est_chiffre(texte: &str) -> bool {
    texte
        .lines()
        .next()
        .map(|l| l.trim() == EN_TETE)
        .unwrap_or(false)
}

/// Chiffre un texte en clair au format du coffre (en-tete + base64).
pub fn chiffrer(texte: &str) -> Result<String, ErreurCoffre> {
    let blob = dpapi(true, texte.as_bytes())?;
    Ok(format!("{}\n{}\n", EN_TETE, BASE64.encode(blob)))
}

/// Dechiffre un contenu de coffre. Un contenu sans en-tete est rendu tel quel :
/// c'est le fichier en clair d'une installation anterieure (migration).
pub fn dechiffrer(texte: &str) -> Result<String, ErreurCoffre> {
    if !est_chiffre(texte) {
        return Ok(texte.to_string());
    }

    let mut lignes = texte.lines();
    // `est_chiffre` a deja valide la premiere ligne.
    let _ = lignes.next();
    let charge: String = lignes.collect::<Vec<_>>().join("");
    let charge = charge.trim();
    if charge.is_empty() {
        return Err(ErreurCoffre::Base64("bloc vide".to_string()));
    }

    let blob = BASE64
        .decode(charge.as_bytes())
        .map_err(|e| ErreurCoffre::Base64(e.to_string()))?;
    let clair = dpapi(false, &blob)?;
    String::from_utf8(clair).map_err(|e| ErreurCoffre::Texte(e.to_string()))
}

/// Lit un fichier de cles en le dechiffrant si necessaire.
/// `ErrorKind::NotFound` remonte tel quel ; un coffre illisible donne
/// `InvalidData` avec un message explicite (jamais de panique).
pub fn lire_texte(chemin: &Path) -> io::Result<String> {
    let brut = fs::read_to_string(chemin)?;
    dechiffrer(&brut).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Nom du fichier temporaire utilise pour l'ecriture atomique.
fn temporaire_pour(chemin: &Path) -> PathBuf {
    let mut nom = chemin.as_os_str().to_os_string();
    nom.push(".tmp");
    PathBuf::from(nom)
}

/// Ecrit toujours chiffre : fichier temporaire dans le meme dossier, puis
/// renommage (MoveFileEx avec REPLACE_EXISTING sous Windows). Une coupure de
/// courant ne laisse jamais un fichier tronque.
pub fn ecrire_atomique(chemin: &Path, texte: &str) -> io::Result<()> {
    let scelle = chiffrer(texte).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let temporaire = temporaire_pour(chemin);
    fs::write(&temporaire, scelle.as_bytes())?;
    fs::rename(&temporaire, chemin)?;
    Ok(())
}

/// Recopie un fichier de cles vers une sauvegarde, en clair -> chiffre.
/// La sauvegarde ne contient jamais de cle en clair, meme si la source
/// etait encore un fichier historique en texte simple.
pub fn copier_chiffre(source: &Path, destination: &Path) -> io::Result<()> {
    let clair = lire_texte(source)?;
    ecrire_atomique(destination, &clair)
}

/// Retire l'heritage des droits sur le dossier de configuration et ne laisse
/// que le compte Windows courant (controle total, propagation aux enfants).
///
/// Echec non bloquant pour l'appelant : la fonction rend un message lisible
/// que le demarrage journalise.
pub fn restreindre_droits(dossier: &Path) -> Result<String, String> {
    if !dossier.exists() {
        return Err(format!("dossier absent : {}", dossier.display()));
    }

    let utilisateur = std::env::var("USERNAME")
        .map_err(|_| "compte Windows inconnu (USERNAME absent de l'environnement)".to_string())?;
    let domaine = std::env::var("USERDOMAIN").unwrap_or_default();
    let compte = if domaine.is_empty() {
        utilisateur
    } else {
        format!("{}\\{}", domaine, utilisateur)
    };
    let regle = format!("{}:(OI)(CI)F", compte);

    let mut commande = std::process::Command::new("icacls");
    commande
        .arg(dossier)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(&regle)
        .arg("/T")
        .arg("/C")
        .arg("/Q");
    // Pas de fenetre noire au demarrage.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        commande.creation_flags(0x0800_0000);
    }

    let sortie = commande
        .output()
        .map_err(|e| format!("icacls n'a pas pu etre lance : {}", e))?;
    if !sortie.status.success() {
        let detail = String::from_utf8_lossy(&sortie.stderr).trim().to_string();
        let detail = if detail.is_empty() {
            String::from_utf8_lossy(&sortie.stdout).trim().to_string()
        } else {
            detail
        };
        return Err(format!(
            "icacls a echoue (code {}) : {}",
            sortie.status.code().unwrap_or(-1),
            detail
        ));
    }

    Ok(compte)
}

#[cfg(windows)]
fn dpapi(chiffrer: bool, donnees: &[u8]) -> Result<Vec<u8>, ErreurCoffre> {
    use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let entree = CRYPT_INTEGER_BLOB {
        cbData: donnees.len() as u32,
        pbData: donnees.as_ptr() as *mut u8,
    };
    let mut sortie = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let ok = unsafe {
        if chiffrer {
            CryptProtectData(
                &entree,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut sortie,
            )
        } else {
            CryptUnprotectData(
                &entree,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut sortie,
            )
        }
    };

    if ok == 0 {
        return Err(ErreurCoffre::Dpapi {
            operation: if chiffrer { "chiffrement" } else { "dechiffrement" },
            code: io::Error::last_os_error().raw_os_error().unwrap_or(0),
        });
    }

    let resultat = unsafe { std::slice::from_raw_parts(sortie.pbData, sortie.cbData as usize) }.to_vec();
    unsafe {
        LocalFree(sortie.pbData as HLOCAL);
    }
    Ok(resultat)
}

#[cfg(not(windows))]
fn dpapi(_chiffrer: bool, _donnees: &[u8]) -> Result<Vec<u8>, ErreurCoffre> {
    Err(ErreurCoffre::Dpapi {
        operation: "DPAPI",
        code: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entete_detecte_et_texte_simple_tolere() {
        assert!(est_chiffre(&format!("{}\nabc\n", EN_TETE)));
        assert!(!est_chiffre("DEEPSEEK_KEY=abc\n"));
        // Un fichier historique en clair n'est pas une erreur : il est rendu tel quel.
        assert_eq!(dechiffrer("A=1\r\nB=2\r\n").unwrap(), "A=1\r\nB=2\r\n");
    }

    #[test]
    fn en_tete_seul_donne_une_erreur_claire() {
        let err = dechiffrer(&format!("{}\n", EN_TETE)).unwrap_err();
        assert!(err.to_string().contains("corrompu"), "message : {}", err);
    }
}
