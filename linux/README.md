# L-Helper for Linux

The Linux build of L-Helper, first aimed at **Ubuntu 26.04**. It's a separate crate in `linux/`. The lighting effects, presets, widgets, Keyboard Lighting window and settings code are shared with the Windows app in `../src`.

This build is a first release. It has been compile-checked but not yet run on a laptop, so test it before publishing it.

## What it does

| Feature | Comes from | Needs |
|---|---|---|
| Quiet / Balanced / Performance / Extreme / Custom | `platform_profile` | LOQ / Legion: Linux 6.16+ (`lenovo-wmi-gamezone`). IdeaPad: `ideapad_laptop` |
| Fn+Q popup | Watching `platform_profile` | Same |
| Power limits (Custom) | `lenovo-wmi-other` firmware attributes | Linux 6.16+ |
| 24-zone keyboard lighting, all non-reactive presets | `/dev/hidraw`, same reports as Windows | The package's udev rule |
| White keyboard backlight | `/sys/class/leds/*kbd_backlight` | — |
| Battery: Conservation / Normal / Rapid Charge | Battery `charge_types`, else `ideapad_laptop` conservation mode | Rapid Charge only where the kernel lists `Fast` |
| Fn lock, always-on USB | `ideapad_laptop` | — |
| Refresh rate, resolution, Auto (low on battery) | GNOME (Mutter D-Bus), KDE (`kscreen-doctor`), X11 (`xrandr`) | — |
| CPU / GPU temperature, fan speed, battery draw | hwmon, NVML (NVIDIA driver), power_supply | NVIDIA readings only while the GPU is awake |
| Tray icon and menu | StatusNotifierItem | GNOME: the AppIndicator extension (Ubuntu has it on) |
| Run at login | `~/.config/autostart` | — |

Not on Linux yet:
- Fan curves (needs the `legion-laptop` driver)
- GPU modes and NVIDIA overclocking
- The reactive lighting presets (screen, audio and typing)
- The game overlay
- A built-in cycle hotkey. Instead, bind `lhelper --next-lighting` to a shortcut in your desktop's keyboard settings.

## How it runs

The app runs as you. Changing hardware settings needs root, so each change goes through a small helper, `/usr/libexec/lhelper/lhelper-helper`, started with `pkexec`:
- **No password for you:** the polkit policy lets the person signed in at the laptop run the helper without one.
- **Admin password for anyone else:** SSH sessions and other users need one.
- **Fixed values only:** the helper only writes values the kernel driver lists itself, such as a listed profile or a number inside the firmware's own min..max.

Keyboard lighting needs no root. The udev rule gives the signed-in user access to the lighting controller.

Settings are in `~/.config/lhelper/config.json` and the log is in `~/.local/state/lhelper/log.txt`.

## Build the .deb (Ubuntu 26.04)

```bash
sudo apt install build-essential curl git
```

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Open a new terminal, then:

```bash
git clone https://github.com/AgentDragonBlox/L-Helper.git
```

```bash
bash L-Helper/linux/packaging/build-deb.sh
```

The script installs `cargo-deb` if needed, runs the tests and prints the package path and its SHA256, for example `L-Helper/linux/target/debian/lhelper_1.0.0-1_amd64.deb`. No other system libraries are needed to build: the display libraries are loaded at run time.

## Install and remove

```bash
sudo apt install ./L-Helper/linux/target/debian/lhelper_1.0.0-1_amd64.deb
```

Open **L-Helper** from the app grid. To remove it:

```bash
sudo apt remove lhelper
```

## Test checklist

Please report back on each item. If something fails, attach `~/.local/state/lhelper/log.txt` and a screenshot of **Settings → This laptop**.

1. The window opens. **Settings → This laptop** lists the model, kernel, profiles and drivers.
2. Each mode button switches modes. Check with `cat /sys/firmware/acpi/platform_profile`.
3. Fn+Q changes the mode, a popup appears, and the window follows.
4. GNOME's power mode menu (top right) and L-Helper stay in sync.
5. **Power limits:** move a slider and click Apply. Check that the mode switches to Custom and the value sticks, then switch modes and back to Custom.
6. **Keyboard:** presets change the lighting, and brightness works. Unticking "L-Helper controls the keyboard lighting" stops the effects.
7. **Battery:** Conservation and Normal switch modes. Check `cat /sys/class/power_supply/BAT*/charge_types`, or `conservation_mode` on older kernels.
8. **Settings:** Fn lock and Always-on USB toggle and stay.
9. **Screen:** the refresh rate buttons and the resolution list change the screen. **Auto** drops to 60 Hz on battery and goes back up when plugged in.
10. **Tray:**
    - Left click opens the window.
    - The menu switches modes.
    - **Quit** exits.
    - Closing the window keeps the tray icon, and clicking the icon opens the window again.
11. **Run at login:** tick it, sign out and back in. L-Helper starts in the tray.
12. **Suspend and resume:** lighting and Custom limits come back.

If modes, limits or the keyboard don't show up at all, run the hardware probe and send its archive:

```bash
bash L-Helper/linux/probe.sh
```

## Troubleshooting

- **No performance modes:**
  - LOQ and Legion need Linux 6.16 or newer; check with `uname -r`.
  - The Lenovo drivers must be loaded. **Settings → This laptop → Drivers** shows which ones are.
- **"The 24-zone keyboard is there, but L-Helper may not open it":** the udev rule didn't apply. Reinstall the package, then reboot.
- **No tray icon on GNOME:** turn on the AppIndicator extension (Extensions app), or run `sudo apt install gnome-shell-extension-appindicator`. Without a tray, closing the window quits L-Helper.
- **"Not authorized":** the polkit policy isn't installed, or you're not in the active local session. Install the package rather than running a bare `cargo build`.
- **Other tools fighting over the mode:** TLP, auto-cpufreq and tuned also change power settings, so turn those off. power-profiles-daemon (GNOME's power menu) is fine: both change the same setting, and L-Helper follows it.

## Development

```bash
cd linux && cargo build --release
```

Unpackaged builds run, but every hardware change then asks for an admin password, because the polkit policy only covers the installed helper path. The keyboard also needs the udev rule installed. The Windows build is unaffected by this crate: build it from the repository root as before.
