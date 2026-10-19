. (Join-Path $PSScriptRoot "commun.ps1")

$port = Lire-Port
$fichier = Trouver-FichierCles

try {
    $present = Tester-EcouteAttendue -Port $port
} catch {
    "Refus : $($_.Exception.Message)"
    exit 1
}
if (-not $present) {
    "Deja arrete."
    exit 0
}

$jeton = $null
if (Test-Path -LiteralPath $fichier -PathType Leaf) {
    try {
        $jeton = Lire-Jeton -Chemin $fichier
    } catch {
        "Coffre illisible : $($_.Exception.Message)"
    }
}

$propre = $false
if ($jeton) {
    try {
        Invoke-RestMethod "http://127.0.0.1:$port/admin/arreter" `
            -Method Post -Headers @{ Authorization = "Bearer $jeton" } -TimeoutSec 5 | Out-Null
        $propre = $true
        "Arret propre demande (les flux en cours peuvent se terminer)."
    } catch {
        "Arret propre indisponible : $($_.Exception.Message)"
    }
}

if ($propre) {
    $fin = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $fin) {
        if (-not (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)) {
            "Arrete proprement."
            exit 0
        }
        Start-Sleep -Milliseconds 250
    }
    "Le service ecoute encore apres 15 s."
}

# Dernier recours seulement : ne jamais couper silencieusement un flux actif.
Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 600
if (Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue) {
    "Arret incomplet : un processus repond encore."
    exit 1
}
"Arrete (dernier recours)."
