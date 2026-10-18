$racine = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $racine "src-tauri\target\release\apivault-proxy.exe"

if (-not (Test-Path $exe)) {
    "Executable introuvable : $exe"
    "Compilez d'abord :  cd src-tauri ; cargo build --release"
    exit 1
}

if (Get-NetTCPConnection -LocalPort 3100 -State Listen -ErrorAction SilentlyContinue) {
    # L'instance active detecte le second lancement et reaffiche sa fenetre.
    Start-Process -FilePath $exe
    "Deja en marche sur 127.0.0.1:3100 : fenetre demandee a l'instance active."
    exit 0
}

Start-Process -FilePath $exe
"Lance."
