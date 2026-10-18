$tache = Get-ScheduledTask -TaskName "ApiVault_Proxy_Externe" -ErrorAction SilentlyContinue

if (-not $tache) {
    "La tache n'existe pas (deja desactivee)."
    exit 0
}

Unregister-ScheduledTask -TaskName "ApiVault_Proxy_Externe" -Confirm:$false
"Demarrage automatique desactive."
