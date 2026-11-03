//! Laptop screen refresh rate and resolution, through the desktop: GNOME (Mutter's DisplayConfig D-Bus API),
//! KDE Plasma (kscreen-doctor) or plain X11 (xrandr). Wayland has no common protocol for it.

#[derive(Debug, Clone, Default)]
pub struct Screen {
    /// Refresh rates at the current resolution, ascending
    pub rates: Vec<u32>,
    pub rate: Option<u32>,
    /// Largest first
    pub resolutions: Vec<(u32, u32)>,
    pub resolution: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
struct ModeInfo {
    id: String,
    width: u32,
    height: u32,
    hz: f64,
    current: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Backend {
    Gnome,
    Kde,
    X11,
}

fn has(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join(program).is_file()))
}

fn backend() -> Option<Backend> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
    if desktop.contains("GNOME") || desktop.contains("UNITY") {
        Some(Backend::Gnome)
    } else if desktop.contains("KDE") && has("kscreen-doctor") {
        Some(Backend::Kde)
    } else if !wayland && std::env::var_os("DISPLAY").is_some() && has("xrandr") {
        Some(Backend::X11)
    } else {
        None
    }
}

fn is_builtin(connector: &str) -> bool {
    connector.starts_with("eDP") || connector.starts_with("LVDS") || connector.starts_with("DSI")
}

fn modes(b: Backend) -> Result<(String, Vec<ModeInfo>), String> {
    match b {
        Backend::Gnome => gnome::modes(),
        Backend::Kde => kde::modes(),
        Backend::X11 => x11::modes(),
    }
}

pub fn laptop_screen() -> Option<Screen> {
    let b = backend()?;
    let (_, modes) = modes(b).map_err(|e| crate::log!("Screen ({b:?}): {e}")).ok()?;
    let current = modes.iter().find(|m| m.current)?.clone();
    let mut rates: Vec<u32> = modes.iter().filter(|m| m.width == current.width && m.height == current.height).map(|m| m.hz.round() as u32).collect();
    rates.sort();
    rates.dedup();
    let mut resolutions: Vec<(u32, u32)> = modes.iter().map(|m| (m.width, m.height)).collect();
    resolutions.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
    resolutions.dedup();
    Some(Screen { rates, rate: Some(current.hz.round() as u32), resolutions, resolution: Some((current.width, current.height)) })
}

fn apply(b: Backend, output: &str, mode: &ModeInfo) -> Result<(), String> {
    crate::log!("Screen: {output} -> {}x{} @ {:.2} Hz", mode.width, mode.height, mode.hz);
    match b {
        Backend::Gnome => gnome::apply(output, &mode.id),
        Backend::Kde => kde::apply(output, &mode.id),
        Backend::X11 => x11::apply(output, mode),
    }
}

pub fn set_refresh_rate(hz: u32) -> Result<(), String> {
    let b = backend().ok_or("Changing the refresh rate isn't supported on this desktop")?;
    let (name, modes) = modes(b)?;
    let current = modes.iter().find(|m| m.current).ok_or("No current screen mode")?;
    let target = modes
        .iter()
        .filter(|m| m.width == current.width && m.height == current.height && m.hz.round() as u32 == hz)
        .max_by(|a, b| a.hz.total_cmp(&b.hz))
        .ok_or_else(|| format!("{hz} Hz isn't available"))?;
    if target.current {
        return Ok(());
    }
    apply(b, &name, target)
}

pub fn set_resolution(width: u32, height: u32) -> Result<(), String> {
    let b = backend().ok_or("Changing the resolution isn't supported on this desktop")?;
    let (name, modes) = modes(b)?;
    let hz = modes.iter().find(|m| m.current).map(|m| m.hz).unwrap_or(60.0);
    // Same refresh rate if the new resolution has it, else the closest
    let target = modes
        .iter()
        .filter(|m| m.width == width && m.height == height)
        .min_by(|a, b| (a.hz - hz).abs().total_cmp(&(b.hz - hz).abs()))
        .ok_or_else(|| format!("{width} × {height} isn't available"))?;
    apply(b, &name, target)
}

pub fn low_rate(rates: &[u32]) -> Option<u32> {
    if rates.contains(&60) { Some(60) } else { rates.first().copied() }
}

// ---------------- GNOME: org.gnome.Mutter.DisplayConfig ----------------

mod gnome {
    use super::ModeInfo;
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};

    type Props = HashMap<String, OwnedValue>;
    /// connector, vendor, product, serial
    type Spec = (String, String, String, String);
    /// id, width, height, refresh, preferred scale, supported scales, properties
    type Mode = (String, i32, i32, f64, f64, Vec<f64>, Props);
    type Monitor = (Spec, Vec<Mode>, Props);
    /// x, y, scale, transform, primary, monitors, properties
    type Logical = (i32, i32, f64, u32, bool, Vec<Spec>, Props);
    type State = (u32, Vec<Monitor>, Vec<Logical>, Props);
    type Assigned<'a> = (String, String, HashMap<String, Value<'a>>);
    type LogicalConfig<'a> = (i32, i32, f64, u32, bool, Vec<Assigned<'a>>);

    const NAME: &str = "org.gnome.Mutter.DisplayConfig";
    const PATH: &str = "/org/gnome/Mutter/DisplayConfig";

    /// One session bus connection, reused (the window refreshes the screen state every few seconds)
    fn connection() -> Result<zbus::blocking::Connection, String> {
        static CONNECTION: std::sync::Mutex<Option<zbus::blocking::Connection>> = std::sync::Mutex::new(None);
        let mut slot = CONNECTION.lock().unwrap();
        if let Some(c) = slot.as_ref() {
            return Ok(c.clone());
        }
        let c = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
        *slot = Some(c.clone());
        Ok(c)
    }

    fn flag(props: &Props, key: &str) -> bool {
        props.get(key).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
    }

    fn state(c: &zbus::blocking::Connection) -> Result<State, String> {
        let reply = c.call_method(Some(NAME), PATH, Some(NAME), "GetCurrentState", &()).map_err(|e| e.to_string())?;
        reply.body().deserialize::<State>().map_err(|e| e.to_string())
    }

    fn builtin(monitors: &[Monitor]) -> Option<&Monitor> {
        monitors.iter().find(|m| flag(&m.2, "is-builtin")).or_else(|| monitors.iter().find(|m| super::is_builtin(&m.0 .0)))
    }

    pub fn modes() -> Result<(String, Vec<ModeInfo>), String> {
        let c = connection()?;
        let (_, monitors, _, _) = state(&c)?;
        let m = builtin(&monitors).ok_or("No built-in screen")?;
        let modes = m.1.iter().map(|x| ModeInfo { id: x.0.clone(), width: x.1 as u32, height: x.2 as u32, hz: x.3, current: flag(&x.6, "is-current") }).collect();
        Ok((m.0 .0.clone(), modes))
    }

    /// Re-sends the whole current layout with the built-in screen's mode replaced (Mutter wants every monitor)
    pub fn apply(output: &str, mode_id: &str) -> Result<(), String> {
        let c = connection()?;
        let (serial, monitors, logical, _) = state(&c)?;
        let mut config: Vec<LogicalConfig> = Vec::new();
        for (x, y, mut scale, transform, primary, specs, _) in logical {
            let mut assigned = Vec::new();
            for spec in specs {
                let Some(monitor) = monitors.iter().find(|m| m.0 == spec) else { continue };
                let mode = if spec.0 == output { monitor.1.iter().find(|m| m.0 == mode_id) } else { monitor.1.iter().find(|m| flag(&m.6, "is-current")) };
                let Some(mode) = mode else { return Err(format!("No mode for {}", spec.0)) };
                // A new resolution may not support the current scale
                if spec.0 == output && !mode.5.iter().any(|s| (s - scale).abs() < 0.001) {
                    scale = mode.4;
                }
                assigned.push((spec.0.clone(), mode.0.clone(), HashMap::new()));
            }
            config.push((x, y, scale, transform, primary, assigned));
        }
        // Method 2: persistent (kept after sign-out, like Settings → Displays)
        c.call_method(Some(NAME), PATH, Some(NAME), "ApplyMonitorsConfig", &(serial, 2u32, config, HashMap::<String, Value>::new())).map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------- KDE Plasma: kscreen-doctor ----------------

mod kde {
    use super::ModeInfo;
    use std::process::Command;

    pub fn modes() -> Result<(String, Vec<ModeInfo>), String> {
        let out = Command::new("kscreen-doctor").arg("-j").output().map_err(|e| e.to_string())?;
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
        let outputs = json["outputs"].as_array().ok_or("No outputs")?;
        let o = outputs.iter().find(|o| o["name"].as_str().is_some_and(super::is_builtin) && o["enabled"].as_bool() != Some(false)).ok_or("No built-in screen")?;
        let current = o["currentModeId"].as_str().map(str::to_string).or_else(|| o["currentModeId"].as_i64().map(|i| i.to_string())).unwrap_or_default();
        let id = |m: &serde_json::Value| m["id"].as_str().map(str::to_string).or_else(|| m["id"].as_i64().map(|i| i.to_string())).unwrap_or_default();
        let modes = o["modes"]
            .as_array()
            .map(|modes| {
                modes
                    .iter()
                    .map(|m| ModeInfo {
                        id: id(m),
                        width: m["size"]["width"].as_u64().unwrap_or(0) as u32,
                        height: m["size"]["height"].as_u64().unwrap_or(0) as u32,
                        hz: m["refreshRate"].as_f64().unwrap_or(0.0),
                        current: id(m) == current,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok((o["name"].as_str().unwrap_or_default().to_string(), modes))
    }

    pub fn apply(output: &str, mode_id: &str) -> Result<(), String> {
        let status = Command::new("kscreen-doctor").arg(format!("output.{output}.mode.{mode_id}")).status().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err("kscreen-doctor failed".into()) }
    }
}

// ---------------- X11: xrandr ----------------

mod x11 {
    use super::ModeInfo;
    use std::process::Command;

    pub fn modes() -> Result<(String, Vec<ModeInfo>), String> {
        let out = Command::new("xrandr").arg("--current").output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut output = None;
        let mut modes = Vec::new();
        for line in text.lines() {
            if !line.starts_with(' ') {
                if output.is_some() {
                    break;
                }
                let name = line.split_whitespace().next().unwrap_or_default();
                if super::is_builtin(name) && line.contains(" connected") {
                    output = Some(name.to_string());
                }
                continue;
            }
            if output.is_none() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some((w, h)) = parts.next().and_then(|r| r.split_once('x')) else { continue };
            let (Ok(width), Ok(height)) = (w.parse::<u32>(), h.trim_end_matches('i').parse::<u32>()) else { continue };
            for rate in parts {
                let current = rate.contains('*');
                let number = rate.trim_end_matches(['*', '+']);
                if let Ok(hz) = number.parse::<f64>() {
                    modes.push(ModeInfo { id: number.to_string(), width, height, hz, current });
                }
            }
        }
        Ok((output.ok_or("No built-in screen")?, modes))
    }

    pub fn apply(output: &str, mode: &ModeInfo) -> Result<(), String> {
        let status = Command::new("xrandr")
            .args(["--output", output, "--mode", &format!("{}x{}", mode.width, mode.height), "--rate", &mode.id])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err("xrandr failed".into()) }
    }
}

