#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use apivault_proxy::coffre;
use apivault_proxy::config;
use apivault_proxy::serveur::{self, AppState, Service};


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

/// Journal de demarrage : l'interface signale qu'elle est prete (P5).
#[tauri::command]
fn interface_prete(service: tauri::State<'_, Arc<Service>>) -> Value {
    let ms = serveur::ms_depuis_demarrage();
    service
        .app
        .journaliser(&format!("demarrage interface : {} ms", ms));
    service
        .app
        .journaliser(&format!("demarrage total : {} ms", ms));
    serde_json::json!({ "ms": ms })
}

/// Diagnostic copiable, sans aucune cle ni query string (P6).
#[tauri::command]
fn diagnostic(service: tauri::State<'_, Arc<Service>>) -> Value {
    service.diagnostic_json()
}

/// LOT 5 : totaux de consommation d'une periode `AAAA-MM`.
///
/// Une periode vide ou inconnue rend le mois courant ; une periode mal formee
/// est refusee avec un message explicite.
#[tauri::command]
fn metriques(service: tauri::State<'_, Arc<Service>>, periode: Option<String>) -> Result<Value, String> {
    let periode = match periode {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => apivault_proxy::metriques::periode_courante(),
    };
    if !apivault_proxy::metriques::periode_valide(&periode) {
        return Err(format!(
            "Periode invalide : attendu AAAA-MM, recu « {} ».",
            periode
        ));
    }
    let totaux = service
        .inner()
        .app
        .metriques
        .totaliser(&periode)
        .map_err(|e| format!("Journal de metriques illisible : {}", e))?;
    Ok(apivault_proxy::metriques::totaux_avec_plafond(
        &totaux,
        service.inner().app.plafond(),
    ))
}

/// Enregistre le plafond mensuel (LOT 6) et l'applique sans redemarrer.
#[tauri::command]
fn sauver_plafond(service: tauri::State<'_, Arc<Service>>, valeur: u64) -> Result<Value, String> {
    let app = &service.inner().app;
    config::sauver_plafond(&app.cfg.chemin_cles, valeur)
        .map_err(|e| format!("Plafond non enregistre : {}", e))?;
    app.fixer_plafond(valeur);
    app.journaliser(&format!("plafond mensuel regle a {} tokens", valeur));
    let totaux = app
        .metriques
        .totaliser(&apivault_proxy::metriques::periode_courante())
        .map_err(|e| format!("Journal de metriques illisible : {}", e))?;
    Ok(apivault_proxy::metriques::totaux_avec_plafond(&totaux, valeur))
}

/// Ajout d'un fournisseur depuis l'interface (P8).
#[tauri::command]
fn ajouter_fournisseur(
    service: tauri::State<'_, Arc<Service>>,
    nom: String,
    url: String,
    cle: String,
    modeles: String,
) -> Result<Value, String> {
    service
        .inner()
        .clone()
        .ajouter_fournisseur(&nom, &url, &cle, &modeles)?;
    Ok(service.gestion_json())
}

/// Test d'une cle avant enregistrement (P8) : rien n'est ecrit.
#[tauri::command]
async fn tester_cle(
    service: tauri::State<'_, Arc<Service>>,
    url: String,
    cle: String,
) -> Result<Value, String> {
    Ok(service.app.tester_cle(&url, &cle).await)
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
    // Horloge du processus posee au plus tot, pour le journal de demarrage.
    let debut = std::time::Instant::now();
    let _ = serveur::ms_depuis_demarrage();

    let (configuration, migration) = config::charger_et_migrer();
    let ms_config = debut.elapsed().as_millis();
    let port = configuration.port;
    let jeton = configuration.jeton.clone();
    // LOT 4 : les cles ne doivent appartenir qu'a ce compte Windows. En
    // revanche on ne touche pas aux droits d'un dossier qui n'est pas celui de
    // l'application : un fichier designe par APIVAULT_CLE_ENV peut vivre sur
    // le Bureau, dans Documents ou sur un partage, qui ne nous appartiennent pas.
    let chemin_cles = configuration.chemin_cles.clone();
    let etat_partage = Arc::new(AppState::nouveau(configuration));
    let ms_pool = debut.elapsed().as_millis();
    etat_partage.journaliser(&format!("demarrage config : {} ms", ms_config));
    etat_partage.journaliser(&format!("demarrage pool : {} ms", ms_pool));

    // LOT 6 : une installation anterieure peut avoir un fichier de cles en
    // clair. Il est mis au coffre ici, apres le chargement, pour que la
    // promesse « plus de cle en clair sur le disque » soit vraie des le premier
    // demarrage et pas seulement apres une modification. Best-effort : en cas
    // d'echec le fichier d'origine reste en place et le journal le dit.
    match migration {
        config::Migration::Chiffre => etat_partage.journaliser(
            "fichier de cles en clair : migration chiffree au demarrage",
        ),
        config::Migration::Externe => etat_partage.journaliser(&format!(
            "fichier de cles externe : migration non appliquee ({})",
            chemin_cles.display()
        )),
        config::Migration::Echec(ref e) => etat_partage.journaliser(&format!(
            "migration du fichier de cles non appliquee, fichier d'origine conserve : {}",
            e
        )),
        config::Migration::Absent | config::Migration::DejaChiffre => {}
    }

    // Restriction des droits : retrait de l'heritage, seul le compte courant
    // garde le controle. Un echec n'empeche pas de demarrer, mais il est
    // journalise pour ne pas passer inapercu.
    match config::dossier_a_restreindre(&chemin_cles, &config::dossier_donnees()) {
        Some(dossier_cles) => match coffre::restreindre_droits(&dossier_cles) {
            Ok(compte) => etat_partage.journaliser(&format!(
                "droits du dossier de configuration : heritage retire, {} seul autorise",
                compte
            )),
            Err(e) => etat_partage.journaliser(&format!(
                "droits du dossier de configuration non restreints : {}",
                e
            )),
        },
        None => etat_partage.journaliser(&format!(
            "droits non appliques : fichier de cles externe ({})",
            chemin_cles.display()
        )),
    }
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
            interface_prete,
            diagnostic,
            metriques,
            sauver_plafond,
            ajouter_fournisseur,
            tester_cle,
            quitter
        ])
        .setup(|app| {
            let srv = app.state::<Arc<Service>>().inner().clone();
            construire_icone(app)?;
            srv.demarrer();

            // Aucune interrogation au demarrage : l'etat de la derniere session
            // est repris tel quel depuis etat-pool.json. Une seule passe de
            // fond, 60 s apres le lancement, et uniquement sur les cibles
            // jamais testees (4 en parallele, 5 s chacune). Un echec d'appel
            // reel peut aussi declencher une verification avant cela.
            {
                let verif = srv.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    verif.sonder_les_inconnus(4, std::time::Duration::from_secs(5));
                });
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
