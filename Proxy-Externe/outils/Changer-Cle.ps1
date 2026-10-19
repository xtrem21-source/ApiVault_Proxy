param(
    # "" autorise : l'utilisateur choisit alors dans la liste.
    [ValidateSet("", "openrouter", "deepseek", "nvidia")]
    [string]$Fournisseur = "",
    # Renseigner -Cle evite toute question : indispensable aux verifications
    # automatisees, pratique aussi pour un envoi par un autre script.
    [string]$Cle = ""
)

. (Join-Path $PSScriptRoot "commun.ps1")

$fichier = Trouver-FichierCles

if (-not (Test-Path -LiteralPath $fichier -PathType Leaf)) {
    "cles.env introuvable : $fichier"
    "Lancez l'application une premiere fois : elle cree le fichier chiffre."
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

if ($Cle) {
    # Mode non interactif : la cle arrive en argument.
    $clair = $Cle.Trim()
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
# Le defaut precedent venait d'ici : Get-Content sur le fichier chiffre, puis
# reecriture en clair sous l'en-tete du coffre, donc coffre corrompu.
try {
    $texte = Lire-Clair -Chemin $fichier
} catch {
    "Refus : le fichier de cles n'a pas pu etre dechiffre, rien n'a ete ecrit."
    "  $($_.Exception.Message)"
    "  Verifiez que le fichier appartient bien a votre compte Windows."
    exit 1
}

$lignes = New-Object System.Collections.Generic.List[string]
foreach ($ligne in ($texte -split "`r?`n")) {
    # Un BOM eventuel ne doit pas coller au premier nom de variable.
    $lignes.Add($ligne.TrimStart([char]0xFEFF))
}
# Un fichier termine par un saut de ligne produit une derniere entree vide :
# on la retire pour ne pas accumuler des lignes vides a chaque changement.
while ($lignes.Count -gt 0 -and $lignes[$lignes.Count - 1] -eq "") {
    $lignes.RemoveAt($lignes.Count - 1)
}

$trouve = $false
for ($i = 0; $i -lt $lignes.Count; $i++) {
    if ($lignes[$i] -match "^$variable=") {
        $lignes[$i] = "$variable=$clair"
        $trouve = $true
    }
}
if (-not $trouve) {
    $lignes.Add("$variable=$clair")
}

$nouveau = ($lignes -join "`r`n") + "`r`n"

try {
    # Ecriture chiffree, par un .tmp remplace atomiquement. Jamais de clair.
    Ecrire-Coffre -Chemin $fichier -Texte $nouveau
} catch {
    "Echec de l'ecriture du coffre : le fichier d'origine est intact."
    "  $($_.Exception.Message)"
    exit 1
}

# Droits du fichier : il contient la seule copie des cles.
icacls $fichier /inheritance:r /grant:r "$($env:USERNAME):(F)" | Out-Null
$clair = $null

"Cle $Fournisseur mise a jour (coffre chiffre)."

# Redemarrage : la configuration est lue au lancement.
$tache = Get-ScheduledTask -TaskName "ApiVault_Proxy_Externe" -ErrorAction SilentlyContinue
Get-Process -Name apivault-proxy -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

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
