# outils\commun.ps1 - fonctions partagees par les scripts de la distribution.
#
# Ne s'execute pas : les autres scripts le chargent par dot-sourcing.
#     . (Join-Path $PSScriptRoot "commun.ps1")
#
# Il cache aux autres scripts deux realites qui ont change :
#   - l'emplacement de l'executable : a cote des .bat dans l'archive publiee,
#     sous src-tauri\target\release en developpement ;
#   - le format du fichier de cles : coffre DPAPI depuis le LOT 4, ou texte en
#     clair pour une installation anterieure.

# Racine de la distribution : le dossier qui contient les .bat et l'executable,
# c'est-a-dire le parent de outils\.
if (-not $RacineDistribution) {
    $RacineDistribution = Split-Path -Parent $PSScriptRoot
}

function Trouver-Executable {
    <#
    .SYNOPSIS
        Chemin de apivault-proxy.exe.
    .DESCRIPTION
        Archive publiee : l'executable est a cote des .bat. Developpement :
        il est sous src-tauri\target\release. Si aucun des deux n'existe, le
        chemin attendu dans l'archive est rendu, pour un message d'erreur juste.
    #>
    $portable = Join-Path $RacineDistribution "apivault-proxy.exe"
    if (Test-Path -LiteralPath $portable -PathType Leaf) {
        return $portable
    }
    $developpement = Join-Path $RacineDistribution "src-tauri\target\release\apivault-proxy.exe"
    if (Test-Path -LiteralPath $developpement -PathType Leaf) {
        return $developpement
    }
    return $portable
}

function Trouver-FichierCles {
    <#
    .SYNOPSIS
        Le cles.env que l'application utilise reellement.
    .DESCRIPTION
        Meme ordre que le coeur Rust : APIVAULT_CLE_ENV (si c'est un fichier),
        puis le dossier de la distribution, puis %APPDATA%\ApiVault_Proxy.
        Le chemin est rendu meme s'il n'existe pas encore : l'appelant decide
        s'il doit le creer (application) ou refuser (script).
    #>
    if ($env:APIVAULT_CLE_ENV -and (Test-Path -LiteralPath $env:APIVAULT_CLE_ENV -PathType Leaf)) {
        return $env:APIVAULT_CLE_ENV
    }
    $portable = Join-Path $RacineDistribution "cles.env"
    if (Test-Path -LiteralPath $portable -PathType Leaf) {
        return $portable
    }
    return (Join-Path $env:APPDATA "ApiVault_Proxy\cles.env")
}

function Activer-Dpapi {
    <#
    .SYNOPSIS
        Rend ProtectedData utilisable dans Windows PowerShell 5.1.
    .DESCRIPTION
        Sous PowerShell 7 le type est deja resolu. Sous 5.1 il faut charger
        System.Security. L'echec n'est pas fatal ici : c'est l'appel a
        ProtectedData qui rendra l'erreur, avec son contexte.
    #>
    try {
        Add-Type -AssemblyName System.Security -ErrorAction Stop
    } catch {
        # PowerShell 7 : rien a charger, ProtectedData est disponible.
    }
}

function Lire-Clair {
    <#
    .SYNOPSIS
        Contenu en clair d'un fichier de cles, chiffre ou non.
    .DESCRIPTION
        Format du coffre (LOT 4) : premiere ligne APIVAULT-COFFRE-1, puis le
        blob DPAPI en base64 (portee utilisateur, sans entropie). Un fichier
        sans en-tete est rendu tel quel : c'est celui d'avant le coffre.
        Toute erreur est levee : l'appelant ne doit jamais ecrire a l'aveugle.
    #>
    param([Parameter(Mandatory = $true)][string]$Chemin)

    if (-not (Test-Path -LiteralPath $Chemin -PathType Leaf)) {
        throw "fichier de cles introuvable : $Chemin"
    }

    $lignes = @(Get-Content -LiteralPath $Chemin -Encoding UTF8)
    if ($lignes.Count -ge 2 -and $lignes[0].Trim() -eq 'APIVAULT-COFFRE-1') {
        $base64 = ($lignes[1..($lignes.Count - 1)] -join '').Trim()
        if (-not $base64) {
            throw "coffre vide : $Chemin"
        }
        Activer-Dpapi
        try {
            $blob = [Convert]::FromBase64String($base64)
            $octets = [System.Security.Cryptography.ProtectedData]::Unprotect(
                $blob,
                $null,
                [System.Security.Cryptography.DataProtectionScope]::CurrentUser)
        } catch {
            throw "coffre illisible (compte Windows different, fichier abime ou droits refuses) : $($_.Exception.Message)"
        }
        return [Text.Encoding]::UTF8.GetString($octets)
    }

    return ($lignes -join "`r`n")
}

function Lire-Jeton {
    <#
    .SYNOPSIS
        Jeton maitre du fichier de cles, ou $null.
    .DESCRIPTION
        Lit PROXY_MASTER_KEY dans le contenu dechiffre. Ne suppose jamais que
        le fichier est en clair : c'est Lire-Clair qui decide.
    #>
    param([string]$Chemin = "")

    if (-not $Chemin) {
        $Chemin = Trouver-FichierCles
    }
    $texte = Lire-Clair -Chemin $Chemin
    $ligne = ($texte -split "`r?`n") |
        Where-Object { $_ -match '^PROXY_MASTER_KEY=' } |
        Select-Object -First 1
    if (-not $ligne) {
        return $null
    }
    return (($ligne -split '=', 2)[1]).Trim()
}

function Lire-Port {
    <#
    .SYNOPSIS
        Port d'ecoute du service (PROXY_PORT, 3100 par defaut).
    .DESCRIPTION
        Un port modifie dans cles.env doit etre suivi par Etat et Rafraichir,
        sinon ils interrogent un service qui n'ecoute pas la. Le coffre est lu
        comme pour le jeton ; en cas d'echec on retombe sur le port par defaut
        plutot que d'empecher l'affichage.
    #>
    param([int]$Defaut = 3100)

    $fichier = Trouver-FichierCles
    if (-not (Test-Path -LiteralPath $fichier -PathType Leaf)) {
        return $Defaut
    }
    try {
        $texte = Lire-Clair -Chemin $fichier
    } catch {
        return $Defaut
    }
    $ligne = ($texte -split "`r?`n") |
        Where-Object { $_ -match '^PROXY_PORT=' } |
        Select-Object -First 1
    if (-not $ligne) {
        return $Defaut
    }
    $valeur = 0
    if ([int]::TryParse((($ligne -split '=', 2)[1]).Trim(), [ref]$valeur) -and $valeur -gt 0) {
        return $valeur
    }
    return $Defaut
}

function Ecrire-Coffre {
    <#
    .SYNOPSIS
        Ecrit un fichier de cles chiffre, atomiquement.
    .DESCRIPTION
        Meme format que le coeur Rust : en-tete, puis base64 du blob DPAPI
        (portee utilisateur, sans entropie). Ecriture dans un .tmp du meme
        dossier, puis remplacement : un coffre n'est jamais laisse tronque.
        Aucun chemin d'ecriture en clair n'existe ici, volontairement.
    #>
    param(
        [Parameter(Mandatory = $true)][string]$Chemin,
        [Parameter(Mandatory = $true)][string]$Texte
    )

    Activer-Dpapi
    $octets = [Text.Encoding]::UTF8.GetBytes($Texte)
    $blob = [System.Security.Cryptography.ProtectedData]::Protect(
        $octets,
        $null,
        [System.Security.Cryptography.DataProtectionScope]::CurrentUser)
    $base64 = [Convert]::ToBase64String($blob)
    $tmp = "$Chemin.tmp"
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($tmp, "APIVAULT-COFFRE-1`r`n$base64`r`n", $utf8)
    Move-Item -LiteralPath $tmp -Destination $Chemin -Force
}
