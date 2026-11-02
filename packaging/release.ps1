# Builds the GitHub release files in dist\:
#   L-Helper-<version>.msi   installer (the main download)
#   L-Helper-<version>.zip   portable: lhelper.exe, LHelper.msix, LHelper.cer, README.txt (register from Settings)
# Needs the Windows SDK and the .NET SDK (WiX 5 comes from .config\dotnet-tools.json). No admin needed.
#   powershell -ExecutionPolicy Bypass -File packaging\release.ps1
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$wixVersion = "5.0.2"

Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    & (Join-Path $PSScriptRoot "pack.ps1")

    $cargo = Get-Content (Join-Path $root "Cargo.toml") -Raw
    $null = $cargo -match '(?m)^version\s*=\s*"([^"]+)"'
    $version = $Matches[1]

    $exe = Join-Path $root "target\release\lhelper.exe"
    $msix = Join-Path $PSScriptRoot "build\LHelper.msix"
    $cer = Join-Path $PSScriptRoot "build\LHelper.cer"
    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force $dist | Out-Null

    # Installer
    dotnet tool restore | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "dotnet tool restore failed" }
    dotnet wix extension add -g "WixToolset.UI.wixext/$wixVersion" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "wix extension add failed" }
    # Build values as an include file: WiX 5 drops the value of a repeated -d
    $esc = { param($s) [System.Security.SecurityElement]::Escape($s) }
    @"
<?xml version="1.0" encoding="utf-8"?>
<Include>
  <?define Version = "$version" ?>
  <?define Exe = "$(& $esc $exe)" ?>
  <?define Msix = "$(& $esc $msix)" ?>
  <?define Cer = "$(& $esc $cer)" ?>
  <?define Root = "$(& $esc $root)" ?>
</Include>
"@ | Set-Content (Join-Path $PSScriptRoot "build\installer.wxi") -Encoding UTF8
    $msi = Join-Path $dist "L-Helper-$version.msi"
    dotnet wix build (Join-Path $PSScriptRoot "installer\LHelper.wxs") -arch x64 -ext "WixToolset.UI.wixext/$wixVersion" -o $msi | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
    # Windows Installer's consistency checks (ICE); ICE69 about the shortcut's target is expected
    dotnet wix msi validate $msi | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "MSI validation failed" }
    Remove-Item (Join-Path $dist "L-Helper-$version.wixpdb") -ErrorAction SilentlyContinue

    # Portable zip
    $stage = Join-Path $dist "L-Helper-$version"
    $zip = "$stage.zip"
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item $exe, $msix, $cer, (Join-Path $PSScriptRoot "README.txt") $stage
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
    Remove-Item $stage -Recurse -Force

    "Release files:"
    Get-Item $msi, $zip | ForEach-Object { "  {0,-24} {1,12:N0} bytes  SHA256 {2}" -f $_.Name, $_.Length, (Get-FileHash $_.FullName).Hash }
} finally { Pop-Location }
