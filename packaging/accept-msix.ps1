# A-MSIX-02..04: install the signed MSIX, launch it with package identity (execution alias) on a
# short `--measure` run that exits by itself, then uninstall and check for leftovers.
# Exit 0 = pass, 1 = fail, 2 = blocked (dev cert not trusted). Run via `cargo xtask accept A-MSIX-02`.
param([Parameter(Mandatory)][string]$Msix, [string]$Cer)
$ErrorActionPreference = 'Stop'

if (-not ($Cer -and (Test-Path $Cer))) { Write-Host "BLOCKED: dev cert $Cer missing (cargo xtask package --dev-cert)"; exit 2 }
$thumb = (New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $Cer).Thumbprint
if (-not ((Test-Path "Cert:\LocalMachine\TrustedPeople\$thumb") -or (Test-Path "Cert:\LocalMachine\Root\$thumb"))) {
  Write-Host "BLOCKED: dev cert not trusted. Once, in an admin PowerShell:"
  Write-Host "  Import-Certificate -FilePath `"$Cer`" -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
  exit 2
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $Msix))
try {
  $reader = New-Object IO.StreamReader ($zip.GetEntry('AppxManifest.xml').Open())
  [xml]$man = $reader.ReadToEnd(); $reader.Dispose()
} finally { $zip.Dispose() }
$name = $man.Package.Identity.Name

$fail = @()
Add-AppxPackage -Path $Msix                                   # A-MSIX-02
$pkg = Get-AppxPackage -Name $name
if (-not $pkg) { Write-Host "A-MSIX-02 fail: $name not installed"; exit 1 }
Write-Host "A-MSIX-02 pass: $($pkg.PackageFullName)"
try {
  $alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\editelier.exe'
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $alias -ArgumentList '--measure' -PassThru
  if (-not $p.WaitForExit(180000)) {
    Stop-Process -Id $p.Id -Force
    $fail += 'A-MSIX-03: app did not exit within 180 s (killed)'
  } elseif ($p.ExitCode -ne 0) {
    $fail += "A-MSIX-03: --measure exit code $($p.ExitCode)"
  } else { Write-Host "A-MSIX-03 pass: packaged --measure run exited 0 after $([int]$sw.Elapsed.TotalSeconds) s" }
} finally {
  Remove-AppxPackage -Package $pkg.PackageFullName              # A-MSIX-04
  $left = Join-Path $env:LOCALAPPDATA "Packages\$($pkg.PackageFamilyName)"
  if (Get-AppxPackage -Name $name) { $fail += 'A-MSIX-04: package still installed' }
  elseif (Test-Path $left) { $fail += "A-MSIX-04: leftover $left" }
  else { Write-Host 'A-MSIX-04 pass: removed, no leftovers' }
}
if ($fail) { $fail | ForEach-Object { Write-Host "FAIL $_" }; exit 1 }
