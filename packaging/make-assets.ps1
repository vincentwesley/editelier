# Regenerates packaging/Assets/*.png (MSIX visual assets) from Editelier's own mark (brand/README.md, D22).
# Source: brand/png/mark-dark-1024.png (square, transparent, built-in margin; `node brand/render.mjs` makes it),
# scaled only, centred on a #11150e tile. Taskbar sizes (targetsize-*) fill the tile; the rest keep 10% clear space.
# Usage: powershell -File packaging/make-assets.ps1 [-Brand <path to brand/>]
param([string]$Brand = (Join-Path $PSScriptRoot '..\brand'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$srcPath = Join-Path $Brand 'png\mark-dark-1024.png'
if (-not (Test-Path $srcPath)) { throw "Mark not found: $srcPath (run node brand/render.mjs or pass -Brand)" }
$src = [System.Drawing.Image]::FromFile((Resolve-Path $srcPath))
$crop = New-Object System.Drawing.RectangleF 0, 0, $src.Width, $src.Height
$out = Join-Path $PSScriptRoot 'Assets'
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out -Filter *.png | Remove-Item

function Tile([string]$name, [int]$w, [int]$h, [double]$fill = 0.8) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.Clear([System.Drawing.ColorTranslator]::FromHtml('#11150e'))
  $g.InterpolationMode = 'HighQualityBicubic'
  $g.PixelOffsetMode = 'HighQuality'
  $g.SmoothingMode = 'HighQuality'
  $m = [Math]::Min($w, $h) * $fill
  $dst = New-Object System.Drawing.RectangleF (($w - $m) / 2), (($h - $m) / 2), $m, $m
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
  Tile "Square44x44Logo.targetsize-$t" $t $t 1.0
  Tile "Square44x44Logo.targetsize-${t}_altform-unplated" $t $t 1.0
}
$src.Dispose()
Write-Host "Assets -> $out"
