# Third-party notices

LHelper is licensed under GPL-3.0 (see `LICENSE`). It includes or adapts code and knowledge from the projects below.

## LOQ-24-zone-RGB-controller (MIT)

https://github.com/DChitale/LOQ-24-zone-RGB-controller

The 24-zone keyboard driver (`app/Lighting/ZoneKeyboard.cs`), the lighting presets (`app/Lighting/Effects.cs`, `app/Lighting/Presets.cs`), the effect loop (`app/Lighting/LightingEngine.cs`) and the reactive inputs (`app/Lighting/Samplers.cs`) are C# ports of that project's Rust backend.

```
MIT License

Copyright (c) 2026 Dhruv Goel

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Lenovo Legion Toolkit (GPL-3.0)

https://github.com/LenovoLegionToolkit-Team/LenovoLegionToolkit

Lenovo WMI / EnergyDrv / HID protocol details, fan table layouts, BIOS workarounds, the Lenovo software disabler and Fn key handling,
the BIOS / driver update rules and the NVIDIA GPU overclock / undervolt logic (Rust version).

## NvAPIWrapper (LGPL-3.0) and LLT.NvAPIWrapper.Net

https://github.com/falahati/NvAPIWrapper, https://www.nuget.org/packages/LLT.NvAPIWrapper.Net

NVAPI function IDs and structure layouts used by the Rust version's GPU tuning (no code is included).

## G-Helper (GPL-3.0)

https://github.com/seerge/g-helper

UI concept, theming approach, config and startup patterns.

## NAudio (MIT)

https://github.com/naudio/NAudio. Used for the microphone mute key and audio-reactive lighting.
