//! ApiVault_Proxy - bibliotheque du coeur (sans interface).
//!
//! Exposee pour que les tests d'integration (`tests/`) puissent demarrer la
//! passerelle reelle sans lancer la fenetre Tauri.

pub mod coffre;
pub mod config;
pub mod metriques;
pub mod notification;
pub mod pool;
pub mod serveur;
