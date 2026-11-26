# Regenerates packaging/Assets/*.png (MSIX visual assets) from the house brand mark.
# Source: _house/brand/png/mark-b.png (already cropped to the artwork, 3150x1800), scaled only
# (never recoloured or redrawn), centred on
# a #11150e tile with >= 0.5x mark-height clear space on every side (brand README rules).
# Usage: powershell -File packaging/make-assets.ps1 [-Brand <path to _house/brand>]
param([string]$Brand = (Join-Path $PSScriptRoot '..\..\_house\brand'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$srcPath = Join-Path $Brand 'png\mark-b.png'
if (-not (Test-Path $srcPath)) { throw "Brand mark not found: $srcPath (pass -Brand)" }
$src = [System.Drawing.Image]::FromFile((Resolve-Path $srcPath))
$crop = New-Object System.Drawing.RectangleF 0, 0, $src.Width, $src.Height
$out = Join-Path $PSScriptRoot 'Assets'
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out -Filter *.png | Remove-Item

function Tile([string]$name, [int]$w, [int]$h) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.Clear([System.Drawing.ColorTranslator]::FromHtml('#11150e'))
  $g.InterpolationMode = 'HighQualityBicubic'
  $g.PixelOffsetMode = 'HighQuality'
  $g.SmoothingMode = 'HighQuality'
  # Mark height m: m + 2 * 0.5m <= h and markWidth + 2 * 0.5m <= w.
  $ar = $crop.Width / $crop.Height
  $m = [Math]::Min($h / 2.0, $w / ($ar + 1.0))
  $mw = $m * $ar
  $dst = New-Object System.Drawing.RectangleF (($w - $mw) / 2), (($h - $m) / 2), $mw, $m
  $g.DrawImage($src, $dst, $crop, [System.Drawing.GraphicsUnit]::Pixel)
  $g.Dispose()
  $bmp.Save((Join-Path $out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

$scales = 100, 125, 150, 200, 400
foreach ($s in $scales) {
  $k = $s / 100.0
  foreach ($a in @(@('Square44x44Logo', 44, 44), @('Square71x71Logo', 71, 71), @('Square150x150Logo', 150, 150),
      @('Square310x310Logo', 310, 310), @('Wide310x150Logo', 310, 150), @('StoreLogo', 50, 50))) {
    Tile "$($a[0]).scale-$s" ([Math]::Round($a[1] * $k)) ([Math]::Round($a[2] * $k))
  }
}
foreach ($t in 16, 24, 32, 48, 256) {
  Tile "Square44x44Logo.targetsize-$t" $t $t
  Tile "Square44x44Logo.targetsize-${t}_altform-unplated" $t $t
}
$src.Dispose()
Write-Host "Assets -> $out"
