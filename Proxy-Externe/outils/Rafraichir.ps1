$racine = Split-Path -Parent $PSScriptRoot
$fichier = Join-Path $racine "cles.env"

if (-not (Test-Path $fichier)) {
    "cles.env introuvable."
    exit 1
}

$ligne = Select-String -Path $fichier -Pattern '^PROXY_MASTER_KEY='
if (-not $ligne) {
    "Jeton introuvable dans cles.env."
    exit 1
}

$jeton = ($ligne.Line -split '=', 2)[1].Trim()

try {
    $null = Invoke-RestMethod "http://127.0.0.1:3100/admin/rafraichir" `
        -Method Post `
        -Headers @{ Authorization = "Bearer $jeton" } `
        -TimeoutSec 10
    "Rafraichissement lance : les fournisseurs sont interroges."
} catch {
    "Echec : $($_.Exception.Message)"
    exit 1
}
