. (Join-Path $PSScriptRoot "commun.ps1")

$exe = Trouver-Executable

if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    "Executable introuvable : $exe"
    "L'archive doit contenir apivault-proxy.exe a cote des fichiers .bat."
    exit 1
}

# Le dossier de travail est celui de l'executable : dans l'archive c'est la
# racine de la distribution, en developpement le dossier de compilation.
$dossier = Split-Path -Parent $exe

$action = New-ScheduledTaskAction -Execute $exe -Argument "--discret" -WorkingDirectory $dossier
$declencheur = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
# Petit temps de latence apres l'ouverture de session (comme la case de l'interface).
$declencheur.Delay = "PT20S"
$reglages = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable

Register-ScheduledTask -TaskName "ApiVault_Proxy_Externe" `
    -Action $action `
    -Trigger $declencheur `
    -Settings $reglages `
    -Description "Passerelle locale ApiVault_Proxy" `
    -Force | Out-Null

"Demarrage automatique active (tache ApiVault_Proxy_Externe)."
