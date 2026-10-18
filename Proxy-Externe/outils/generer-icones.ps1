# Genere les icones de ApiVault_Proxy (cadenas blanc + fleches de passerelle).
# Style : carre arrondi, degrade bleu, cadenas central, double fleche en bas.
# OBSOLETE : le logo officiel est src-tauri/icons/icon.svg (trousseau, tuile bleue).
param([switch]$AncienLogo)
if (-not $AncienLogo) {
    Write-Host "OBSOLETE : ce script redessine l'ancien cadenas." -ForegroundColor Yellow
    Write-Host "Le logo officiel est src-tauri/icons/icon.svg. Pour l'ancien logo : -AncienLogo"
    exit 1
}

Add-Type -AssemblyName System.Drawing

$dossier = Join-Path $PSScriptRoot "..\src-tauri\icons"
$dossier = [System.IO.Path]::GetFullPath($dossier)
New-Item -ItemType Directory -Force -Path $dossier | Out-Null

function New-PathArrondi([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

function New-Image([string]$c1, [string]$c2, [string]$trou) {
    $bmp = New-Object System.Drawing.Bitmap 256, 256
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    # Fond : carre arrondi en degrade.
    $rect = New-Object System.Drawing.RectangleF 0, 0, 256, 256
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        $rect,
        [System.Drawing.ColorTranslator]::FromHtml($c1),
        [System.Drawing.ColorTranslator]::FromHtml($c2),
        50)
    $fond = New-PathArrondi 8 8 240 240 54
    $g.FillPath($brush, $fond)
    $bord = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(50, 255, 255, 255)), 2
    $g.DrawPath($bord, $fond)

    # Anse du cadenas.
    $blanc = [System.Drawing.Color]::White
    $penAnse = New-Object System.Drawing.Pen $blanc, 15
    $penAnse.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $penAnse.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $g.DrawArc($penAnse, 97, 68, 62, 70, 180, 180)

    # Corps du cadenas.
    $corps = New-PathArrondi 84 118 88 72 16
    $g.FillPath([System.Drawing.Brushes]::White, $corps)

    # Trou de serrure.
    $cTrou = [System.Drawing.ColorTranslator]::FromHtml($trou)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush $cTrou), 121, 140, 14, 14)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush $cTrou), 124, 150, 8, 21)

    # Double fleche de passerelle (echange dans les deux sens).
    $penFleche = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(220, 255, 255, 255)), 7
    $penFleche.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $penFleche.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $g.DrawLine($penFleche, 66, 214, 106, 214)
    $g.DrawLine($penFleche, 190, 214, 150, 214)
    $p1 = @(
        (New-Object System.Drawing.PointF 106, 200),
        (New-Object System.Drawing.PointF 124, 214),
        (New-Object System.Drawing.PointF 106, 228))
    $p2 = @(
        (New-Object System.Drawing.PointF 150, 200),
        (New-Object System.Drawing.PointF 132, 214),
        (New-Object System.Drawing.PointF 150, 228))
    $g.FillPolygon([System.Drawing.Brushes]::White, $p1)
    $g.FillPolygon([System.Drawing.Brushes]::White, $p2)

    $g.Dispose()
    return $bmp
}

function Redimensionner($source, [int]$taille) {
    $bmp = New-Object System.Drawing.Bitmap $taille, $taille
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.DrawImage($source, 0, 0, $taille, $taille)
    $g.Dispose()
    return $bmp
}

function Ecrire-Ico($source, [string]$chemin) {
    $tailles = @(16, 24, 32, 48, 64, 128, 256)
    $morceaux = @()
    foreach ($t in $tailles) {
        $b = Redimensionner $source $t
        $ms = New-Object System.IO.MemoryStream
        $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $morceaux += ,@($t, $ms.ToArray())
        $b.Dispose()
        $ms.Dispose()
    }

    $fs = [System.IO.File]::Create($chemin)
    $bw = New-Object System.IO.BinaryWriter $fs
    $bw.Write([UInt16]0)          # reserve
    $bw.Write([UInt16]1)          # type icone
    $bw.Write([UInt16]$morceaux.Count)
    $offset = 6 + 16 * $morceaux.Count
    foreach ($m in $morceaux) {
        $t = $m[0]
        $octets = $m[1]
        $dim = if ($t -ge 256) { 0 } else { $t }
        $bw.Write([Byte]$dim)
        $bw.Write([Byte]$dim)
        $bw.Write([Byte]0)        # palette
        $bw.Write([Byte]0)        # reserve
        $bw.Write([UInt16]1)      # plans
        $bw.Write([UInt16]32)     # bits
        $bw.Write([UInt32]$octets.Length)
        $bw.Write([UInt32]$offset)
        $offset += $octets.Length
    }
    foreach ($m in $morceaux) { $bw.Write($m[1]) }
    $bw.Flush()
    $bw.Close()
    $fs.Close()
}

$variantes = @(
    @("icon.png",       "#123A7A", "#2F7BFF", "#16406F"),
    @("icon-vert.png",  "#0E6B3F", "#24B36B", "#0B5130"),
    @("icon-orange.png","#A8560A", "#F0A020", "#7A3F06"),
    @("icon-rouge.png", "#4A5361", "#7C8798", "#333A45")
)

foreach ($v in $variantes) {
    $img = New-Image $v[1] $v[2] $v[3]
    $chemin = Join-Path $dossier $v[0]
    $img.Save($chemin, [System.Drawing.Imaging.ImageFormat]::Png)
    if ($v[0] -eq "icon.png") {
        Ecrire-Ico $img (Join-Path $dossier "icon.ico")
    }
    $img.Dispose()
    Write-Output ("cree : " + $chemin)
}

Write-Output "Termine."
