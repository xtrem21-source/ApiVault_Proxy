. (Join-Path $PSScriptRoot "commun.ps1")

$port = Lire-Port
$fichier = Trouver-FichierCles

if (-not (Test-Path -LiteralPath $fichier -PathType Leaf)) {
    "cles.env introuvable : $fichier"
    exit 1
}

try {
    $jeton = Lire-Jeton -Chemin $fichier
} catch {
    "Coffre illisible : $($_.Exception.Message)"
    "Rien n'a ete envoye. Utilisez Changer-Cle.bat pour reposer la cle."
    exit 1
}

if (-not $jeton) {
    "Jeton maitre introuvable dans $fichier"
    exit 1
}

try {
    Invoke-RestMethod "http://127.0.0.1:$port/admin/rafraichir" `
        -Method Post -Headers @{ Authorization = "Bearer $jeton" } -TimeoutSec 30 | Out-Null
    "Rafraichissement lance sur 127.0.0.1:$port (le sondage continue en arriere-plan)."
} catch {
    "Rafraichissement impossible : $($_.Exception.Message)"
    exit 1
}
