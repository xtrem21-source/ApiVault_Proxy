#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod pool;
mod serveur;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::serveur::{AppState, Service};

const TACHE_DEMARRAGE: &str = "ApiVault_Proxy_Externe";

// Icone de la zone de notification : la couleur resume l'etat reel.
const ICONE_ARRET: &[u8] = include_bytes!("../icons/icon-rouge.png");
const ICONE_SANS_MODELE: &[u8] = include_bytes!("../icons/icon-orange.png");
const ICONE_JOIGNABLE: &[u8] = include_bytes!("../icons/icon-vert.png");
const ICONE_PRINCIPALE: &[u8] = include_bytes!("../icons/icon.png");

#[tauri::command]
fn etat(service: tauri::State<'_, Arc<Service>>) -> Value {
    service.etat_json()
}

#[tauri::command]
fn demarrer(service: tauri::State<'_, Arc<Service>>) -> Result<(), String> {
    service.inner().clone().demarrer();
    Ok(())
}

#[tauri::command]
fn arreter(service: tauri::State<'_, Arc<Service>>) -> Result<(), String> {
    service.arreter();
    Ok(())
}

#[tauri::command]
fn basculer(service: tauri::State<'_, Arc<Service>>) -> Result<(), String> {
    service.inner().clone().basculer();
    Ok(())
}

#[tauri::command]
fn journal(service: tauri::State<'_, Arc<Service>>) -> Vec<String> {
    service.app.lignes_journal()
}

#[tauri::command]
fn requalifier_tout(service: tauri::State<'_, Arc<Service>>) -> Result<(), String> {
    service.app.pool.requalifier_tout();
    service
        .app
        .journaliser("requalification manuelle : toutes les cibles remises a l'etat inconnu");
    Ok(())
}

#[tauri::command]
fn requalifier(
    service: tauri::State<'_, Arc<Service>>,
    fournisseur: String,
    modele: String,
) -> Result<(), String> {
    let trouve = service.app.pool.requalifier(&fournisseur, &modele);
    service.app.journaliser(&format!(
        "requalification manuelle : {}/{} ({})",
        fournisseur,
        modele,
        if trouve { "ok" } else { "cible inconnue" }
    ));
    Ok(())
}

#[tauri::command]
fn rafraichir(service: tauri::State<'_, Arc<Service>>) -> Result<(), String> {
    service.app.lancer_sondage(true);
    Ok(())
}

#[tauri::command]
fn tester_cible(
    service: tauri::State<'_, Arc<Service>>,
    fournisseur: String,
    modele: String,
) -> Result<(), String> {
    service.app.sonder_une(&fournisseur, &modele);
    Ok(())
}

#[tauri::command]
fn tester_fournisseur(
    service: tauri::State<'_, Arc<Service>>,
    fournisseur: String,
) -> Result<(), String> {
    service.app.retester_fournisseur(&fournisseur);
    Ok(())
}

/// Etat du demarrage automatique, lu directement dans le planificateur de
/// taches Windows : ainsi l'affichage ne peut pas mentir sur la realite.
#[tauri::command]
fn demarrage_windows_etat() -> Value {
    let mut cmd = std::process::Command::new("schtasks");
    cmd.args(["/Query", "/TN", TACHE_DEMARRAGE, "/XML"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    match cmd.output() {
        Ok(sortie) => {
            let xml = String::from_utf8_lossy(&sortie.stdout).to_string();
            json!({
                "actif": sortie.status.success(),
                "tache": TACHE_DEMARRAGE,
                "discret": xml.contains("--discret"),
                "delai_s": 20,
            })
        }
        Err(e) => json!({
            "actif": false,
            "tache": TACHE_DEMARRAGE,
            "delai_s": 20,
            "erreur": e.to_string(),
        }),
    }
}

/// Cree (ou supprime) la tache de demarrage automatique. La tache demarre
/// 20 secondes apres l'ouverture de session, en mode discret : la fenetre
/// reste dans la zone de notification.
///
/// La creation passe par le module ScheduledTasks de PowerShell : sur
/// certaines machines, `schtasks /Create /SC ONLOGON` repond « Acces refuse »
/// meme sans elevation, alors que Register-ScheduledTask fonctionne.
#[tauri::command]
fn configurer_demarrage_windows(actif: bool) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| format!("chemin inconnu : {e}"))?;

    if !actif {
        let mut cmd = std::process::Command::new("schtasks");
        cmd.args(["/Delete", "/TN", TACHE_DEMARRAGE, "/F"]);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let sortie = cmd
            .output()
            .map_err(|e| format!("impossible de lancer schtasks : {e}"))?;
        if !sortie.status.success() {
            let err = String::from_utf8_lossy(&sortie.stderr).trim().to_string();
            let out = String::from_utf8_lossy(&sortie.stdout).trim().to_string();
            return Err(if err.is_empty() { out } else { err });
        }
        return Ok(demarrage_windows_etat());
    }

    let script = r#"param([string]$Exe)
$ErrorActionPreference = "Stop"
try {
    $nom = "ApiVault_Proxy_Externe"
    $action = New-ScheduledTaskAction -Execute $Exe -Argument "--discret" -WorkingDirectory (Split-Path -Parent $Exe)
    $declencheur = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
    $declencheur.Delay = "PT20S"
    $reglages = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable
    Register-ScheduledTask -TaskName $nom -Action $action -Trigger $declencheur -Settings $reglages -Description "Passerelle locale ApiVault_Proxy" -Force | Out-Null
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
exit 0
"#;
    let chemin = config::dossier_donnees().join("demarrage-windows.ps1");
    std::fs::write(&chemin, script).map_err(|e| format!("ecriture impossible : {e}"))?;

    let mut cmd = std::process::Command::new("powershell");
    let chemin_s = chemin.to_string_lossy().to_string();
    let exe_s = exe.to_string_lossy().to_string();
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-WindowStyle",
        "Hidden",
        "-File",
        &chemin_s,
        "-Exe",
        &exe_s,
    ]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let sortie = cmd
        .output()
        .map_err(|e| format!("impossible de lancer PowerShell : {e}"))?;
    if !sortie.status.success() {
        let err = String::from_utf8_lossy(&sortie.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "Windows a refuse la creation de la tache.".to_string()
        } else {
            err
        });
    }
    Ok(demarrage_windows_etat())
}

#[tauri::command]
fn gestion(service: tauri::State<'_, Arc<Service>>) -> Value {
    service.app.gestion_json()
}

#[tauri::command]
fn enregistrer_cle(
    service: tauri::State<'_, Arc<Service>>,
    fournisseur: String,
    cle: String,
) -> Result<Value, String> {
    service.app.enregistrer_cle(&fournisseur, &cle)?;
    Ok(service.app.gestion_json())
}

#[tauri::command]
fn enregistrer_adresses(
    service: tauri::State<'_, Arc<Service>>,
    fournisseur: String,
    adresses: String,
) -> Result<Value, String> {
    service.app.enregistrer_adresses(&fournisseur, &adresses)?;
    Ok(service.app.gestion_json())
}

#[tauri::command]
fn definir_modele_prefere(
    service: tauri::State<'_, Arc<Service>>,
    modele: Option<String>,
) -> Result<(), String> {
    service
        .app
        .definir_modele_prefere(modele.as_deref())
}

#[tauri::command]
fn quitter(app: tauri::AppHandle) {
    app.exit(0);
}

fn afficher_fenetre(app: &tauri::AppHandle) {
    if let Some(fenetre) = app.get_webview_window("main") {
        let _ = fenetre.unminimize();
        let _ = fenetre.show();
        let _ = fenetre.set_focus();
        return;
    }

    // Fenetre creee a la demande : tant qu'elle est fermee, aucun moteur de
    // rendu ne tourne et la passerelle ne consomme que quelques dizaines de Mo.
    let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("ApiVault_Proxy")
        .inner_size(780.0, 620.0)
        .min_inner_size(640.0, 500.0)
        .center()
        .build();
}

/// Notification Windows discrete quand plus aucun modele ne repond.
/// Le script PowerShell est ecrit une fois dans le dossier de donnees ; son
/// execution est silencieuse (aucune fenetre) et un echec est sans consequence.
fn notifier_aucun_modele() {
    let script = r#"param([string]$Titre, [string]$Message)
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null
$modele = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)
$textes = $modele.GetElementsByTagName("text")
$textes.Item(0).AppendChild($modele.CreateTextNode($Titre)) > $null
$textes.Item(1).AppendChild($modele.CreateTextNode($Message)) > $null
$toast = [Windows.UI.Notifications.ToastNotification]::new($modele)
$id = "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe"
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($id).Show($toast)
"#;
    let chemin = config::dossier_donnees().join("notifier.ps1");
    if std::fs::write(&chemin, script).is_err() {
        return;
    }
    let chemin = chemin.to_string_lossy().to_string();
    let mut cmd = std::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-WindowStyle",
        "Hidden",
        "-File",
        &chemin,
        "-Titre",
        "ApiVault_Proxy",
        "-Message",
        "Aucun modele ne repond. Ouvrez la fenetre pour verifier vos cles et vos adresses.",
    ]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let _ = cmd.spawn();
}

/// Couleur de l'icone = etat reel : vert joignable, orange en service mais
/// aucun modele, rouge/gris arrete.
fn mettre_a_jour_icone(app: &tauri::AppHandle, actif: bool, sains: usize) {
    let (image, bulle) = if !actif {
        (ICONE_ARRET, "ApiVault_Proxy - arrete".to_string())
    } else if sains == 0 {
        (
            ICONE_SANS_MODELE,
            "ApiVault_Proxy - en service, aucun modele ne repond".to_string(),
        )
    } else {
        (
            ICONE_JOIGNABLE,
            format!(
                "ApiVault_Proxy - en service, {} modele(s) joignable(s)",
                sains
            ),
        )
    };
    if let Some(icone) = app.tray_by_id("principal") {
        if let Ok(img) = Image::from_bytes(image) {
            let _ = icone.set_icon(Some(img));
        }
        let _ = icone.set_tooltip(Some(bulle));
    }
}

fn construire_icone(app: &tauri::App) -> tauri::Result<()> {
    let ouvrir = MenuItem::with_id(app, "ouvrir", "Ouvrir la fenetre", true, None::<&str>)?;
    let basculer = MenuItem::with_id(app, "basculer", "Demarrer / arreter", true, None::<&str>)?;
    let quitter = MenuItem::with_id(app, "quitter", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&ouvrir, &basculer, &quitter])?;

    let icone = Image::from_bytes(ICONE_PRINCIPALE)?;

    TrayIconBuilder::with_id("principal")
        .icon(icone)
        .tooltip("ApiVault_Proxy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|icone, evenement| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = evenement
            {
                afficher_fenetre(icone.app_handle());
            }
        })
        .on_menu_event(|app, evenement| match evenement.id.as_ref() {
            "ouvrir" => afficher_fenetre(app),
            "basculer" => {
                if let Some(service) = app.try_state::<Arc<Service>>() {
                    service.inner().clone().basculer();
                }
            }
            "quitter" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn main() {
    let configuration = config::Config::charger();
    let port = configuration.port;
    let jeton = configuration.jeton.clone();
    let etat_partage = Arc::new(AppState::nouveau(configuration));
    let service = Service::nouveau(port, jeton, etat_partage);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Deuxieme lancement : on affiche la fenetre de l'instance deja active.
            afficher_fenetre(app);
        }))
        .manage(service)
        .invoke_handler(tauri::generate_handler![
            etat,
            demarrer,
            arreter,
            basculer,
            journal,
            requalifier,
            requalifier_tout,
            rafraichir,
            tester_cible,
            tester_fournisseur,
            demarrage_windows_etat,
            configurer_demarrage_windows,
            gestion,
            enregistrer_cle,
            enregistrer_adresses,
            definir_modele_prefere,
            quitter
        ])
        .setup(|app| {
            let srv = app.state::<Arc<Service>>().inner().clone();
            construire_icone(app)?;
            srv.demarrer();

            // Aucune interrogation au demarrage : l'etat de la derniere session
            // est repris tel quel depuis etat-pool.json. On ne verifie qu'une
            // fois si ce fichier ne contient encore aucune verification (toute
            // premiere installation), sinon c'est un echec d'appel qui
            // declenchera la verification.
            if srv.app.pool.derniere_verification_s().is_none() {
                let verif = srv.app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    verif.lancer_sondage(false);
                });
            } else {
                srv.app
                    .journaliser("etat repris de la derniere session (aucune sonde au demarrage)");
            }

            // L'icone de notification suit l'etat reel (vert / orange / rouge)
            // et previent une fois lorsqu'aucun modele ne repond plus.
            let veille_icone = srv.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut dernier: Option<u8> = None;
                let mut derniere_alerte: Option<std::time::Instant> = None;
                loop {
                    let etat = veille_icone.etat_json();
                    let actif = etat["actif"].as_bool().unwrap_or(false);
                    let sains = etat["sains"].as_u64().unwrap_or(0) as usize;
                    let cle = if !actif {
                        0u8
                    } else if sains == 0 {
                        1
                    } else {
                        2
                    };
                    if dernier != Some(cle) {
                        mettre_a_jour_icone(&handle, actif, sains);
                        if cle == 1 && dernier.is_some() && !veille_icone.app.sondage.load(Ordering::SeqCst) {
                            let prevenu = derniere_alerte
                                .map(|t| t.elapsed() < std::time::Duration::from_secs(900))
                                .unwrap_or(false);
                            if !prevenu {
                                notifier_aucun_modele();
                                derniere_alerte = Some(std::time::Instant::now());
                            }
                        }
                        dernier = Some(cle);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            });

            // Boucle de reconciliation : une pause terminee revient en jeu.
            let veille = srv.app.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let revenus = veille.pool.reconcilier();
                    if revenus > 0 {
                        veille.journaliser(&format!(
                            "reconciliation : {} cible(s) de nouveau en jeu",
                            revenus
                        ));
                    }
                }
            });

            let discret = std::env::args().any(|a| a == "--discret")
                || std::env::var("PROXY_DEMARRAGE_DISCRET")
                    .map(|v| v == "1")
                    .unwrap_or(false);
            if !discret {
                afficher_fenetre(app.handle());
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("echec du lancement de ApiVault_Proxy")
        .run(|_app, evenement| {
            // Fermer la fenetre ne doit pas arreter la passerelle : elle
            // continue en arriere-plan, pilotable depuis l'icone de notification.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = evenement {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
