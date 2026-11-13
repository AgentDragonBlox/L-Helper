# Builds the GitHub release files in dist\:
#   L-Helper-<version>.exe   single exe: carries its Dynamic Lighting package, offers to install itself on first start
#   L-Helper-<version>.msi   installer
#   L-Helper-<version>.zip   portable: lhelper.exe, LHelper.msix, LHelper.cer, README.txt (register from Settings)
# Needs the Windows SDK and the .NET SDK (WiX 5 comes from .config\dotnet-tools.json). No admin needed.
#   powershell -ExecutionPolicy Bypass -File packaging\release.ps1
#
# -Test builds "L-Helper Test" instead (src/identity.rs: own folders, package, certificate, exe name, startup task
# and MSI UpgradeCode), as L-Helper-Test-<version>.exe / .msi, for trying a build on a PC that has another
# L-Helper installed. Only one of them should run at a time: both drive the same firmware and keyboard.
param([switch]$Test)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$wixVersion = "5.0.2"

if ($Test) {
    $product = "L-Helper Test"; $folder = "LHelperTest"; $exeName = "lhelper-test.exe"; $aumid = "LHelper.Test"
    $package = "LHelperTest"; $prefix = "L-Helper-Test"; $buildDir = "build-test"; $targetDir = Join-Path $root "target\test"
    # Never change: it's how a newer test MSI finds and replaces the installed one
    $upgradeCode = "3DF40A12-8AFD-4DC1-9A81-19C0591E5BA7"
    $cargoArgs = @("build", "--release", "--features", "test-identity", "--target-dir", $targetDir)
} else {
    $product = "L-Helper"; $folder = "LHelper"; $exeName = "lhelper.exe"; $aumid = "LHelper.LHelper"
    $package = "LHelper"; $prefix = "L-Helper"; $buildDir = "build"; $targetDir = Join-Path $root "target"
    $upgradeCode = "733AF42B-05B5-4025-9531-1864EEF0EEE2"
    $cargoArgs = @("build", "--release")
}

Push-Location $root
try {
    # The package first: cargo embeds it in the exe (build.rs)
    if ($Test) { & (Join-Path $PSScriptRoot "pack.ps1") -Test } else { & (Join-Path $PSScriptRoot "pack.ps1") }

    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $cargo = Get-Content (Join-Path $root "Cargo.toml") -Raw
    $null = $cargo -match '(?m)^version\s*=\s*"([^"]+)"'
    $version = $Matches[1]

    $exe = Join-Path $targetDir "release\lhelper.exe"
    $msix = Join-Path $PSScriptRoot "$buildDir\LHelper.msix"
    $cer = Join-Path $PSScriptRoot "$buildDir\LHelper.cer"
    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force $dist | Out-Null

    # Single exe
    $single = Join-Path $dist "$prefix-$version.exe"
    Copy-Item $exe $single -Force

    # Installer
    dotnet tool restore | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "dotnet tool restore failed" }
    dotnet wix extension add -g "WixToolset.UI.wixext/$wixVersion" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "wix extension add failed" }
    # Build values as an include file (LHelper.wxs includes ..\build\installer.wxi): WiX 5 drops the value of a repeated -d
    $wxi = Join-Path $PSScriptRoot "build\installer.wxi"
    New-Item -ItemType Directory -Force (Split-Path $wxi) | Out-Null
    $esc = { param($s) [System.Security.SecurityElement]::Escape($s) }
    @"
<?xml version="1.0" encoding="utf-8"?>
<Include>
  <?define Version = "$version" ?>
  <?define Exe = "$(& $esc $exe)" ?>
  <?define Msix = "$(& $esc $msix)" ?>
  <?define Cer = "$(& $esc $cer)" ?>
  <?define Root = "$(& $esc $root)" ?>
  <?define ProductName = "$product" ?>
  <?define Folder = "$folder" ?>
  <?define ExeName = "$exeName" ?>
  <?define Aumid = "$aumid" ?>
  <?define Package = "$package" ?>
  <?define UpgradeCode = "$upgradeCode" ?>
</Include>
"@ | Set-Content $wxi -Encoding UTF8
    $msi = Join-Path $dist "$prefix-$version.msi"
    dotnet wix build (Join-Path $PSScriptRoot "installer\LHelper.wxs") -arch x64 -ext "WixToolset.UI.wixext/$wixVersion" -o $msi | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
    # Windows Installer's consistency checks (ICE); ICE69 about the shortcut's target is expected
    dotnet wix msi validate $msi | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "MSI validation failed" }
    Remove-Item (Join-Path $dist "$prefix-$version.wixpdb") -ErrorAction SilentlyContinue
    $outputs = @($single, $msi)

    # Portable zip (regular release only)
    if (-not $Test) {
        $stage = Join-Path $dist "$prefix-$version"
        $zip = "$stage.zip"
        if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
        New-Item -ItemType Directory -Force $stage | Out-Null
        Copy-Item $exe, $msix, $cer, (Join-Path $PSScriptRoot "README.txt") $stage
        if (Test-Path $zip) { Remove-Item $zip -Force }
        Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
        Remove-Item $stage -Recurse -Force
        $outputs += $zip
    }

    "Release files:"
    Get-Item $outputs | ForEach-Object { "  {0,-28} {1,12:N0} bytes  SHA256 {2}" -f $_.Name, $_.Length, (Get-FileHash $_.FullName).Hash }
} finally { Pop-Location }
