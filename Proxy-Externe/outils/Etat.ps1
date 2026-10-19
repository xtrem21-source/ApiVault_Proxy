. (Join-Path $PSScriptRoot "commun.ps1")

$port = Lire-Port

$ecoute = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
if (-not $ecoute) {
    "Service : ARRETE (rien n'ecoute sur $port)"
    exit 0
}

try {
    [void](Tester-EcouteAttendue -Port $port)
} catch {
    "Securite : $($_.Exception.Message)"
    exit 1
}

"Service : EN MARCHE sur 127.0.0.1:$port (pid $($ecoute[0].OwningProcess))"

try {
    $sante = Invoke-RestMethod "http://127.0.0.1:$port/health" -TimeoutSec 3
    "Version : $($sante.version)"
} catch {
    "Health  : KO ($($_.Exception.Message))"
    exit 0
}

$fichier = Trouver-FichierCles
if (-not (Test-Path -LiteralPath $fichier -PathType Leaf)) {
    "Jeton   : aucun fichier de cles ($fichier)"
    exit 0
}

try {
    $jeton = Lire-Jeton -Chemin $fichier
} catch {
    "Jeton   : KO (coffre illisible : $($_.Exception.Message))"
    exit 0
}

if (-not $jeton) {
    "Jeton   : introuvable dans $fichier"
    exit 0
}

try {
    $modeles = Invoke-RestMethod "http://127.0.0.1:$port/v1/models" `
        -Headers @{ Authorization = "Bearer $jeton" } -TimeoutSec 5
    $alias = @($modeles.data)
    "Modeles : $($alias.Count) alias"
    foreach ($a in $alias) {
        $cibles = @($a.apivault_cibles)
        "  - $($a.id) : $($cibles.Count) cible(s)"
    }
} catch {
    "Modeles : KO ($($_.Exception.Message))"
}
