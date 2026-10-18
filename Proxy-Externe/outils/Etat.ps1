$racine = Split-Path -Parent $PSScriptRoot
$port = 3100

$ecoute = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
if (-not $ecoute) {
    "Service : ARRETE (rien n'ecoute sur $port)"
    exit 0
}

"Service : EN MARCHE sur 127.0.0.1:$port (pid $($ecoute[0].OwningProcess))"

try {
    $sante = Invoke-RestMethod "http://127.0.0.1:$port/health" -TimeoutSec 3
    "Version : $($sante.version)   uptime : $([math]::Round($sante.uptime_s)) s"
    "Alias   : $($sante.alias -join ', ')"
} catch {
    "Health  : KO ($($_.Exception.Message))"
    exit 0
}

$fichier = Join-Path $racine "cles.env"
if (Test-Path $fichier) {
    $ligne = Select-String -Path $fichier -Pattern '^PROXY_MASTER_KEY='
    if ($ligne) {
        $jeton = ($ligne.Line -split '=', 2)[1].Trim()
        try {
            $modeles = Invoke-RestMethod "http://127.0.0.1:$port/v1/models" -Headers @{ Authorization = "Bearer $jeton" } -TimeoutSec 5
            "Modeles : $(($modeles.data | ForEach-Object { $_.id }) -join ', ')"
        } catch {
            "Modeles : KO ($($_.Exception.Message))"
        }
    }
}
