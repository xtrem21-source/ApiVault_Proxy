$racine = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $racine "src-tauri\target\release\apivault-proxy.exe"

if (-not (Test-Path $exe)) {
    "Executable introuvable : $exe"
    exit 1
}

$action = New-ScheduledTaskAction -Execute $exe -Argument "--discret" -WorkingDirectory (Split-Path -Parent $exe)
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
