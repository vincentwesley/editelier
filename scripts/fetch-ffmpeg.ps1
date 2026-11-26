# Downloads a pinned BtbN FFmpeg LGPL shared build into third_party/ffmpeg/ (gitignored). DEV ONLY.
#
# Shipped builds use our own LGPL build (BtbN scripts at a pinned tag) with the software
# h264/hevc/aac/aac_fixed codecs disabled (D3, docs/LEGAL-REVIEW.md). BtbN lgpl-shared still
# contains those decoders, so it must never be packaged. editelier-media only asks FFmpeg to
# decode royalty-free codecs (allow-list in crates/editelier-media/src/ffmpeg.rs).
# BtbN prunes old autobuild tags; when the download 404s, bump Tag/File/Sha256 to a newer n8.1 build.
$ErrorActionPreference = 'Stop'
$Tag    = 'autobuild-2026-11-25-12-41'
$File   = 'ffmpeg-n8.1.3-41-ga8029b6528-win64-lgpl-shared-8.1.zip'
$Sha256 = '03ebc42b7c3956d5397f1033f4caf56d453b99908d47b573ba6c87e4dd72dc43'

$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root 'third_party\ffmpeg'
$stamp = Join-Path $dest 'PINNED.txt'
if ((Test-Path $stamp) -and ((Get-Content $stamp -Raw).Trim() -eq "$Tag/$File")) {
  Write-Host "FFmpeg already at $Tag/$File"; exit 0
}
$tmp = Join-Path ([IO.Path]::GetTempPath()) "editelier-ffmpeg-$([guid]::NewGuid())"
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
  $zip = Join-Path $tmp $File
  $ProgressPreference = 'SilentlyContinue'
  Invoke-WebRequest -UseBasicParsing "https://github.com/BtbN/FFmpeg-Builds/releases/download/$Tag/$File" -OutFile $zip
  $got = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
  if ($got -ne $Sha256) { throw "SHA-256 mismatch for ${File}: got $got" }
  $out = Join-Path $tmp 'x'
  Expand-Archive $zip -DestinationPath $out
  $inner = Get-ChildItem $out -Directory | Select-Object -First 1
  if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
  New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
  Move-Item $inner.FullName $dest
  $conf = & (Join-Path $dest 'bin\ffmpeg.exe') -hide_banner -buildconf
  if ($conf -match '--enable-(gpl|nonfree)') { throw 'Build is not LGPL-only' }
  Set-Content -Encoding utf8 $stamp "$Tag/$File"
  Write-Host "FFmpeg $File -> $dest"
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
