. (Join-Path $PSScriptRoot "commun.ps1")

$exe = Trouver-Executable
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    "Executable introuvable : $exe"
    "L'archive doit contenir apivault-proxy.exe a cote des fichiers .bat,"
    "et le dossier outils\ doit rester a cote de lui."
    exit 1
}

$port = Lire-Port
if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
    # L'instance active detecte le second lancement et reaffiche sa fenetre.
    Start-Process -FilePath $exe
    "Deja en marche sur 127.0.0.1:$port : fenetre demandee a l'instance active."
    exit 0
}

Start-Process -FilePath $exe
"Lance."
