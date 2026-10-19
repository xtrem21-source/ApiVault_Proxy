param(
    # Nom du fournisseur : openrouter, deepseek, nvidia, ou tout nom ajoute
    # depuis l'interface (<NOM>_KEY dans cles.env).
    [string]$Fournisseur = "",
    # Mode non interactif : cle lue depuis un fichier (jamais en argument).
    [string]$CleFichier = "",
    # Mode non interactif : cle lue depuis une variable d'environnement
    # temporaire, supprimee du processus apres lecture.
    [string]$CleEnv = ""
)

. (Join-Path $PSScriptRoot "commun.ps1")

if ($CleFichier -and $CleEnv) {
    "Choisissez -CleFichier ou -CleEnv, pas les deux. Rien n'a ete modifie."
    exit 1
}

$fichier = Trouver-FichierCles
if (-not (Test-Path -LiteralPath $fichier -PathType Leaf)) {
    "cles.env introuvable : $fichier"
    "Lancez l'application une premiere fois : elle cree le fichier chiffre."
    exit 1
}

if (-not $Fournisseur) {
    ""
    "Nom du fournisseur (openrouter, deepseek, nvidia, ou nom ajoute dans Gestion)."
    $Fournisseur = Read-Host "Fournisseur"
}
$Fournisseur = $Fournisseur.Trim().ToLowerInvariant()
if ($Fournisseur -notmatch '^[a-z0-9_]{2,32}$') {
    "Nom de fournisseur invalide : lettres minuscules, chiffres et _ (2 a 32)."
    exit 1
}
$variable = $Fournisseur.ToUpperInvariant() + "_KEY"

if ($CleFichier) {
    if (-not (Test-Path -LiteralPath $CleFichier -PathType Leaf)) {
        "Fichier de cle introuvable : $CleFichier"
        exit 1
    }
    $clair = [IO.File]::ReadAllText($CleFichier).Trim()
} elseif ($CleEnv) {
    $clair = [Environment]::GetEnvironmentVariable($CleEnv)
    Remove-Item "Env:$CleEnv" -ErrorAction SilentlyContinue
    if (-not $clair) {
        "Variable d'environnement vide ou absente : $CleEnv"
        exit 1
    }
    $clair = $clair.Trim()
} else {
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
}

if ($clair.Length -lt 10) {
    "Cle trop courte : rien n'a ete modifie."
    exit 1
}
if ($clair -match "\s") {
    "La cle contient un espace : verifiez le copier-coller. Rien n'a ete modifie."
    exit 1
}

# --- Lecture du coffre. Si elle echoue, on n'ecrit RIEN. -------------------
try {
    $texte = Lire-Clair -Chemin $fichier
} catch {
    "Refus : le fichier de cles n'a pas pu etre dechiffre, rien n'a ete ecrit."
    "  $($_.Exception.Message)"
    exit 1
}

if ($texte -notmatch "(?m)^$variable=") {
    "Fournisseur inconnu dans cles.env : variable $variable absente."
    "Ajoutez-le d'abord depuis Gestion, puis relancez ce script."
    exit 1
}

$lignes = New-Object System.Collections.Generic.List[string]
foreach ($ligne in ($texte -split "`r?`n")) {
    $lignes.Add($ligne.TrimStart([char]0xFEFF))
}
while ($lignes.Count -gt 0 -and $lignes[$lignes.Count - 1] -eq "") {
    $lignes.RemoveAt($lignes.Count - 1)
}

for ($i = 0; $i -lt $lignes.Count; $i++) {
    if ($lignes[$i] -match "^$variable=") {
        $lignes[$i] = "$variable=$clair"
    }
}
$nouveau = ($lignes -join "`r`n") + "`r`n"

try {
    Ecrire-Coffre -Chemin $fichier -Texte $nouveau
} catch {
    "Echec de l'ecriture du coffre : le fichier d'origine est intact."
    "  $($_.Exception.Message)"
    exit 1
}

# Le SID est plus fiable que le nom (domaine, comptes locaux, renommage).
Restreindre-Fichier -Chemin $fichier
# Une cle compromise ne doit pas survivre dans la sauvegarde de rotation.
Remove-Item -LiteralPath "$fichier.bak" -Force -ErrorAction SilentlyContinue
$clair = $null

"Cle $Fournisseur mise a jour (coffre chiffre)."

# Arret propre : la route admin laisse les flux en cours se terminer.
$arret = Join-Path $PSScriptRoot "Arreter.ps1"
if (Test-Path -LiteralPath $arret -PathType Leaf) {
    & $arret | Out-Host
} else {
    Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue | Stop-Process -Force
}
Start-Sleep -Milliseconds 800

$tache = Get-ScheduledTask -TaskName "ApiVault_Proxy_Externe" -ErrorAction SilentlyContinue
$exe = Trouver-Executable
if ($tache) {
    Start-ScheduledTask -TaskName "ApiVault_Proxy_Externe"
    "Service relance (tache planifiee) : la nouvelle cle est active."
} elseif (Test-Path -LiteralPath $exe -PathType Leaf) {
    Start-Process -FilePath $exe -ArgumentList "--discret"
    "Service relance : la nouvelle cle est active."
} else {
    "Cle enregistree. Relancez la passerelle manuellement."
}

"La verification des fournisseurs demarre automatiquement dans 2 secondes."
