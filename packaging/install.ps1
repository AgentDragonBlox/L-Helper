# Installs L-Helper on this (developer) PC: copies the exe to %ProgramFiles%\LHelper (admin-only folder: the
# no-prompt startup task can't be pointed at a swapped exe), adds a Start menu shortcut, and builds, signs and
# registers the sparse package that makes it a Dynamic Lighting background controller. Run as administrator.
# Re-run after building a new version. Testers don't need this: the release zip registers from Settings.
#   powershell -ExecutionPolicy Bypass -File packaging\install.ps1 [-Source <built lhelper.exe>]
param([string]$Source = (Join-Path $PSScriptRoot "..\target\release\lhelper.exe"))

$ErrorActionPreference = "Stop"
$log = Join-Path $PSScriptRoot "install.log"
Start-Transcript -Path $log -Force | Out-Null
try {
    $Source = (Resolve-Path $Source).Path
    $ExeDir = Join-Path $env:ProgramW6432 "LHelper"
    New-Item -ItemType Directory -Force $ExeDir | Out-Null
    Get-Process lhelper -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $ExeDir "lhelper.exe") } | Stop-Process -Force
    Start-Sleep -Milliseconds 500
    Copy-Item $Source (Join-Path $ExeDir "lhelper.exe") -Force
    "Installed $Source -> $ExeDir"

    $programs = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs"
    Remove-Item (Join-Path $programs "LHelper.lnk") -ErrorAction SilentlyContinue
    $shell = New-Object -ComObject WScript.Shell
    $lnk = $shell.CreateShortcut((Join-Path $programs "L-Helper.lnk"))
    $lnk.TargetPath = Join-Path $ExeDir "lhelper.exe"
    $lnk.WorkingDirectory = $ExeDir
    $lnk.IconLocation = (Join-Path $ExeDir "lhelper.exe") + ",0"
    $lnk.Description = "L-Helper"
    $lnk.Save()

    & (Join-Path $PSScriptRoot "pack.ps1")
    $build = Join-Path $PSScriptRoot "build"
    $msix = Join-Path $build "LHelper.msix"
    $cer = Join-Path $build "LHelper.cer"

    # Trusted for packages on this PC only
    $thumb = (New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $cer).Thumbprint
    if (-not (Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object { $_.Thumbprint -eq $thumb })) {
        Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    }

    # Replace an older registration, then register with the exe's folder as external location
    Get-AppxPackage -Name LHelper | Remove-AppxPackage
    Add-AppxPackage -Path $msix -ExternalLocation $ExeDir
    $pkg = Get-AppxPackage -Name LHelper
    if (-not $pkg) { throw "Package registration failed" }
    "Registered $($pkg.PackageFullName) -> $ExeDir"
    "Next: Settings > Personalization > Dynamic Lighting > Background light control: move L-Helper to the top."
} catch {
    "ERROR: $($_.Exception.Message)"
    exit 1
} finally {
    Stop-Transcript | Out-Null
}
