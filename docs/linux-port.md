# L-Helper for Linux: port plan

Status: **first Ubuntu 26.04 build written (`linux/`), not yet tested on hardware.** See [linux/README.md](../linux/README.md) for the build and the test checklist.

### What the first build does differently from this plan (2026-11-03)

- **Ubuntu 26.04 first.** Its kernel has the mainline Lenovo drivers, so there's no DKMS module yet.
- **No daemon yet.** The app runs as the user and sends each hardware write through `lhelper-helper` with pkexec. Same polkit model, far fewer moving parts. Lighting runs in the app via a udev `uaccess` rule. A root daemon comes back if settings need to apply before login, or for fan curves.
- **No workspace refactor yet.** `linux/` is its own crate, and it includes the platform-neutral files from `../src` with `#[path]`: lighting effects, presets, the keyboard driver, widgets, the Keyboard Lighting window, config and logo. Those files have small `cfg(windows)` branches, and the Windows build is unchanged.

## Decisions (2026-11-03)

| Question | Decision |
|---|---|
| Distros | Ubuntu 24.04 (HWE kernel) and newer, Linux Mint 22+, Debian 13+, Arch, Fedora. Debian 12 runs in a limited mode: modes, battery, keyboard lighting and sensors. |
| Kernel drivers | Use the mainline `lenovo-wmi-*` and `ideapad_laptop` drivers first. The [LenovoLegionLinux](https://github.com/johnfanv2/LenovoLegionLinux) `legion-laptop` DKMS driver is an optional add-on for fan curves and for kernels that are too old. |
| Repository | Same repo, as a Cargo workspace. Shares the effects engine, presets, fan curve maths and UI. |
| Desktops | GNOME and KDE Plasma on Wayland first, then X11 (xrandr). wlroots compositors (Sway, Hyprland) come later. |

## Architecture

Linux doesn't let normal programs call Lenovo's WMI firmware methods. Kernel drivers expose them as files under `/sys`, and only root can write most of those. That gives the same split that asusctl, supergfxctl and power-profiles-daemon use:

```
lhelper (egui app + tray, runs as the user)
   │  D-Bus system bus: com.lhelper.Daemon1, every write checked by polkit
   ▼
lhelperd (systemd service, root)
   ├─ /sys: platform_profile, firmware-attributes, ideapad_laptop, legion-laptop, power_supply, hwmon, powercap
   ├─ /dev/hidraw*: 24-zone keyboard (same ITE feature reports as Windows)
   └─ NVML: NVIDIA clocks and power
```

- The daemon owns the hardware and keeps applying settings: the lighting effects loop, automations, and re-applying after resume.
- The app can be closed. Settings survive because the daemon holds them in `/etc/lhelper/config.json`.
- polkit is Linux's version of the Windows security work: users in the active local session may change modes and lighting, while GPU clocks, power limits and fan curves need admin auth, cached per session.

### Workspace layout (phase 1)

```
crates/
  lhelper-core/      modes, config, lighting effects + presets, fan curve maths, automation rules (no OS code)
  lhelper-ui/        egui windows and widgets, talking to a Backend trait
  lhelper-windows/   today's app behind that trait (WMI, EnergyDrv, NVAPI, Win32)
  lhelperd/          Linux daemon (sysfs, hidraw, NVML, D-Bus server)
  lhelper-linux/     Linux app (D-Bus client, tray, notifications)
linux/               probe script, systemd unit, udev rules, polkit policy, D-Bus config, packaging
```

The Windows build must behave the same after the split. The release scripts keep producing the same MSI and zip.

### Hardware layer, in fallback order

1. Mainline `lenovo-wmi-gamezone`, `lenovo-wmi-other`, `lenovo-wmi-capdata01` and `lenovo-wmi-events` (around kernel 6.16 and newer): `platform_profile` including Extreme, and firmware-attributes for power limits.
2. The `legion-laptop` add-on driver, if installed: fan curves, power modes, overdrive and GPU switching on supported models.
3. `ideapad_laptop`: conservation mode, Fn lock, USB charging, and IdeaPad (DYTC) profiles.
4. Read-only: sensors and keyboard lighting only.

The daemon detects which driver is bound, and the app hides what the machine doesn't support, the same way it does per model on Windows. Mainline and `legion-laptop` must not both drive the same hardware: the daemon uses whichever one is bound and warns when both are loaded.

## Feature map

| Feature | Linux mechanism | Risk |
|---|---|---|
| Quiet / Balanced / Performance / Extreme | `platform_profile`, through power-profiles-daemon when it runs | Low |
| Custom power limits (PL1/PL2/TGP) | firmware-attributes (`lenovo-wmi-other`) or `legion-laptop` | Medium |
| Fan curve | `legion-laptop` hwmon (add-on driver) | High |
| Battery conservation / rapid charge | `ideapad_laptop` `conservation_mode`, rapid charge via `legion-laptop` | Low |
| Fn lock, Win key lock, always-on USB | `ideapad_laptop` / `lenovo-wmi` attributes | Low |
| Temperatures, power, battery draw | hwmon (k10temp / coretemp, amdgpu), powercap RAPL, `power_now`, NVML | Low |
| 24-zone keyboard lighting | hidraw plus a udev rule; same protocol, same effects engine | Low |
| GPU modes | NVIDIA runtime power management for Eco / Optimized; MUX switch through firmware attribute, needs a reboot | Medium |
| GPU overclock / undervolt | NVML clock offsets and locked clocks. Undervolt = offset plus clock cap, no per-point curve | Medium |
| Refresh rate / resolution | GNOME Mutter DisplayConfig D-Bus, KDE KScreen, xrandr on X11 | Medium |
| Panel overdrive | firmware attribute or `legion-laptop`, where exposed | High |
| Fn+Q popup | Watch the `platform_profile` change, then a desktop notification | Low |
| Tray | StatusNotifierItem (`ksni`). GNOME needs the AppIndicator extension (Ubuntu ships it) | Low |
| Game overlay | Wayland blocks drawing over fullscreen games: integrate with MangoHud instead | Medium |
| Automations | upower / udev power-supply events in the daemon | Low |
| BIOS / firmware updates | fwupd / LVFS | Low |
| Lenovo software disabler, Dynamic Lighting, UMA buffer | Not applicable | — |

Other daemons that touch the same knobs: power-profiles-daemon (use it when present), TLP, auto-cpufreq, tuned (warn on conflict) and OpenRGB (it can grab the keyboard).

The Segoe Fluent icon font is Windows-only and can't be redistributed. Linux gets an open icon font (Lucide or Material Symbols) through the same icon helper.

## Phases

- [ ] **0. Discovery.** Run `linux/probe.sh` on the LOQ 15AHP10 and the IdeaPad Slim 5 from live USBs. It sets the minimum kernel and which drivers to target.
- [ ] **1. Workspace refactor.** Split out `lhelper-core` and `lhelper-ui` behind a `Backend` trait. No behaviour change on Windows.
- [ ] **2. Daemon MVP.** Detection, modes, battery, Fn lock, sensors, keyboard lighting; D-Bus API, polkit policy, systemd unit, udev rule.
- [ ] **3. App MVP.** egui app on D-Bus, tray, mode-change notifications, autostart.
- [ ] **4. Advanced.** Power limits, fan curves (add-on driver), GPU modes, NVML clocks, display backends, overdrive, automations, fwupd, MangoHud.
- [ ] **5. Packaging and CI.** `.deb` (cargo-deb), AUR PKGBUILD, `.rpm` (cargo-generate-rpm), DKMS / Secure Boot notes, GitHub Actions.
- [ ] **6. Test matrix.** Ubuntu, Mint, Debian, Arch, Fedora × GNOME, KDE × Wayland, X11 × LOQ, IdeaPad.

Flatpak and Snap don't fit: the daemon needs root and raw `/sys` access.

## Phase 0: running the probe

The probe is read-only. It reads `/sys`, `/proc` and some tool output, then writes one archive to your home folder. It changes no settings, loads no drivers and writes nothing to `/sys`. It leaves out serial numbers, UUIDs, MAC addresses and the ACPI table that holds the Windows product key (MSDM).

Run it from two live USBs, so we see an LTS kernel and a recent one:
1. **Ubuntu 24.04** (latest point release, GNOME): the baseline for Ubuntu, Mint and Debian users.
2. **Fedora Workstation** (latest, GNOME) or **CachyOS** (KDE): close to the newest kernel.

The plain Arch ISO also works, but it has no desktop, so the desktop section stays empty.

On each live session, in a terminal:

```bash
bash probe.sh
```

It asks for sudo once (live sessions have no password). The archive appears as `~/lhelper-probe-<model>-<date>.tar.gz`. A live session forgets files on shutdown, so copy it to a USB stick before rebooting.

What we learn from it:
- which Lenovo WMI interfaces the firmware has, and which kernel drivers bind to them;
- whether `platform_profile` and firmware-attributes exist, and their ranges;
- the keyboard's HID interface and report sizes;
- the sensors, battery and power-limit nodes;
- the GPU driver state;
- whether the desktop has a tray host.

## Open questions (answered by phase 0)

- Does the LOQ 15AHP10's kernel driver expose Extreme mode and the PPT limits, and from which kernel?
- Does `legion-laptop` support the 83JG, and does it conflict with the mainline drivers?
- Does the IdeaPad Slim 5 (Ryzen AI 7 350) expose DYTC profiles and conservation mode through `ideapad_laptop`?
- Is the 24-zone keyboard's hidraw node the same interface 1 with 0x04 / 0x05 feature reports as on Windows?
