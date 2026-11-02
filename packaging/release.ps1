# Builds the GitHub release zip: dist\L-Helper-<version>.zip with lhelper.exe, LHelper.msix, LHelper.cer and
# README.txt. Testers unzip it, run lhelper.exe, and register for Dynamic Lighting from Settings; no script.
#   powershell -ExecutionPolicy Bypass -File packaging\release.ps1
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")

Push-Location $root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }

& (Join-Path $PSScriptRoot "pack.ps1")

$cargo = Get-Content (Join-Path $root "Cargo.toml") -Raw
$null = $cargo -match '(?m)^version\s*=\s*"([^"]+)"'
$version = $Matches[1]

$dist = Join-Path $root "dist"
$stage = Join-Path $dist "L-Helper-$version"
$zip = "$stage.zip"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $root "target\release\lhelper.exe") $stage
Copy-Item (Join-Path $PSScriptRoot "build\LHelper.msix"), (Join-Path $PSScriptRoot "build\LHelper.cer") $stage
Copy-Item (Join-Path $PSScriptRoot "README.txt") $stage
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip

"Release zip: $zip"
Get-ChildItem $stage | ForEach-Object { "  {0,-14} {1,10:N0} bytes  SHA256 {2}" -f $_.Name, $_.Length, (Get-FileHash $_.FullName).Hash }
