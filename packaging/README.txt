L-Helper - control tool for Lenovo LOQ, Legion and IdeaPad laptops
====================================================================

1. Unzip the whole folder somewhere (keep the four files together).
2. Run lhelper.exe and accept the administrator prompt.
   Windows SmartScreen may say "Windows protected your PC" because the exe isn't code-signed:
   click "More info" > "Run anyway".
3. Keyboard lighting with Windows Dynamic Lighting (recommended):
   Settings (gear icon) > Dynamic Lighting > Register, then Restart.
   This copies L-Helper to C:\Program Files\LHelper, adds a Start menu shortcut, and trusts the
   L-Helper signing certificate (LHelper.cer) for app packages on this PC.
   Then in Windows Settings > Personalization > Dynamic Lighting > Background light control,
   move L-Helper to the top.
4. Optional: Settings > Start with Windows.

Remove: L-Helper Settings > Dynamic Lighting > Remove, turn off Start with Windows, quit,
then delete C:\Program Files\LHelper and C:\ProgramData\LHelper.

Logs for bug reports: C:\ProgramData\LHelper\log.txt
