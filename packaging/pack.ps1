# Builds and signs the sparse package (packaging\build\LHelper.msix) and exports its public certificate
# (packaging\build\LHelper.cer). Needs the Windows SDK. Called by install.ps1 and release.ps1.
# The package version follows Cargo.toml (1.0.0 -> 1.0.0.0): bump it for each release so testers can update.
#
# Signing uses the CN=LHelper certificate in your CurrentUser\My store (created on first run). Keep it: a new
# certificate means every tester has to register again. Back it up with
#   Export-PfxCertificate -Cert (Get-ChildItem Cert:\CurrentUser\My | ? Subject -eq "CN=LHelper") -FilePath LHelper.pfx -Password (Read-Host -AsSecureString)
# and never commit or upload the .pfx.
$ErrorActionPreference = "Stop"

$kit = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $kit) { throw "Windows SDK (makeappx.exe / signtool.exe) not found" }
$makeappx = $kit.FullName
$signtool = Join-Path $kit.DirectoryName "signtool.exe"
$makepri = Join-Path $kit.DirectoryName "makepri.exe"

$cargo = Get-Content (Join-Path $PSScriptRoot "..\Cargo.toml") -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') { throw "No version in Cargo.toml" }
$version = "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"

$build = Join-Path $PSScriptRoot "build"
$msix = Join-Path $build "LHelper.msix"
$cer = Join-Path $build "LHelper.cer"
New-Item -ItemType Directory -Force $build | Out-Null

# Package content: manifest (with the version), logos, the extension's public folder
$content = Join-Path $build "content"
if (Test-Path $content) { Remove-Item $content -Recurse -Force }
New-Item -ItemType Directory -Force (Join-Path $content "Assets"), (Join-Path $content "Public") | Out-Null
$manifest = Get-Content (Join-Path $PSScriptRoot "AppxManifest.xml") -Raw
$manifest = $manifest -replace '(<Identity[^>]*\sVersion=")[^"]*(")', "`${1}$version`${2}"
Set-Content (Join-Path $content "AppxManifest.xml") $manifest -Encoding UTF8
Copy-Item (Join-Path $PSScriptRoot "Assets\*.png") (Join-Path $content "Assets")
Set-Content (Join-Path $content "Public\lighting.txt") "L-Helper drives 24-zone keyboard lighting as a Dynamic Lighting background controller."

# Resource index, so Windows finds the right logo size for the taskbar, tray and Settings (else a blank square)
$priconfig = Join-Path $build "priconfig.xml"
& $makepri createconfig /cf $priconfig /dq en-US /pv 10.0.0 /o | Out-Host
& $makepri new /pr $content /cf $priconfig /mn (Join-Path $content "AppxManifest.xml") /of (Join-Path $content "resources.pri") /o | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makepri failed ($LASTEXITCODE)" }

& $makeappx pack /o /nv /d $content /p $msix | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)" }

# Self-signed certificate, reused if it exists
$cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object { $_.Subject -eq "CN=LHelper" -and $_.NotAfter -gt (Get-Date) } | Select-Object -First 1
if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type Custom -Subject "CN=LHelper" -FriendlyName "LHelper package signing" -KeyUsage DigitalSignature `
        -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(10) `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
}
Export-Certificate -Cert $cert -FilePath $cer | Out-Null

& $signtool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $msix | Out-Host
if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }
"Packed $msix ($version), certificate $cer ($($cert.Thumbprint))"
