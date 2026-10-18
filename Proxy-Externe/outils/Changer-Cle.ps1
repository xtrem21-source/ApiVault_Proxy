param(
    [ValidateSet("openrouter", "deepseek", "nvidia")]
    [string]$Fournisseur = ""
)

$racine = Split-Path -Parent $PSScriptRoot
$fichier = Join-Path $racine "cles.env"

if (-not (Test-Path $fichier)) {
    "cles.env introuvable : $fichier"
    exit 1
}

if (-not $Fournisseur) {
    ""
    "Quelle cle voulez-vous changer ?"
    "  1) OpenRouter"
    "  2) DeepSeek"
    "  3) NVIDIA"
    ""
    $choix = Read-Host "Choix (1-3)"
    switch ($choix) {
        "1" { $Fournisseur = "openrouter" }
        "2" { $Fournisseur = "deepseek" }
        "3" { $Fournisseur = "nvidia" }
        default { "Choix invalide."; exit 1 }
    }
}

$variable = switch ($Fournisseur) {
    "openrouter" { "OPENROUTER_KEY" }
    "deepseek"   { "DEEPSEEK_KEY" }
    "nvidia"     { "NVIDIA_KEY" }
}

""
"Nouvelle cle $Fournisseur."
"Collez-la puis validez (la saisie reste invisible)."
$secret = Read-Host "Cle" -AsSecureString

$pointeur = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret)
try {
    $clair = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointeur)
} finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointeur)
}

$clair = $clair.Trim()
if ($clair.Length -lt 10) {
    "Cle trop courte : rien n'a ete modifie."
    exit 1
}
if ($clair -match "\s") {
    "La cle contient un espace : verifiez le copier-coller. Rien n'a ete modifie."
    exit 1
}

$lignes = @(Get-Content -LiteralPath $fichier)
$trouve = $false
for ($i = 0; $i -lt $lignes.Count; $i++) {
    if ($lignes[$i] -match "^$variable=") {
        $lignes[$i] = "$variable=$clair"
        $trouve = $true
    }
}
if (-not $trouve) {
    $lignes += "$variable=$clair"
}

Set-Content -LiteralPath $fichier -Value $lignes -Encoding ASCII
icacls $fichier /inheritance:r /grant:r "$($env:USERNAME):(F)" | Out-Null
$clair = $null

"Cle $Fournisseur mise a jour."

# Redemarrage : la configuration est lue au lancement.
$tache = Get-ScheduledTask -TaskName "ApiVault_Proxy_Externe" -ErrorAction SilentlyContinue
Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

$exe = Join-Path $racine "src-tauri\target\release\apivault-proxy.exe"
if ($tache) {
    Start-ScheduledTask -TaskName "ApiVault_Proxy_Externe"
    "Service relance (tache planifiee) : la nouvelle cle est active."
} elseif (Test-Path $exe) {
    Start-Process -FilePath $exe -ArgumentList "--discret"
    "Service relance : la nouvelle cle est active."
} else {
    "Cle enregistree. Relancez la passerelle manuellement."
}

"La verification des fournisseurs demarre automatiquement dans 2 secondes."
