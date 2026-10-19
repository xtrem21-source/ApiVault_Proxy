//! Notifications Windows (bulles de la zone de notification).
//!
//! Le coeur (`serveur`) en a besoin pour prevenir d'une bascule de port, et
//! l'interface pour signaler qu'aucun modele ne repond. Le code vit donc dans
//! la bibliotheque, pas dans le binaire.

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Affiche une bulle Windows discrete.
///
/// Le script PowerShell est ecrit une fois dans le dossier de donnees ; son
/// execution est silencieuse (aucune fenetre) et un echec est sans
/// consequence : une notification qui rate ne doit jamais empecher de servir.
pub fn bulle(titre: &str, message: &str) {
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
    let chemin = crate::config::dossier_donnees().join("notifier.ps1");
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
        titre,
        "-Message",
        message,
    ]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let _ = cmd.spawn();
}
