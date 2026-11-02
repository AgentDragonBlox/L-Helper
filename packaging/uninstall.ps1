# Removes LHelper's sparse package, signing certificate, Program Files copy and Start menu shortcut
# (run as administrator). Settings in %ProgramData%\LHelper are kept.
Get-AppxPackage -Name LHelper | Remove-AppxPackage
Get-ChildItem Cert:\LocalMachine\TrustedPeople, Cert:\CurrentUser\My | Where-Object { $_.Subject -eq "CN=LHelper" } | Remove-Item
Get-Process lhelper -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$env:ProgramW6432\LHelper\*" } | Stop-Process -Force
Remove-Item (Join-Path $env:ProgramW6432 "LHelper") -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs\L-Helper.lnk"), (Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs\LHelper.lnk") -ErrorAction SilentlyContinue
"LHelper package, certificate, program folder and shortcut removed"
