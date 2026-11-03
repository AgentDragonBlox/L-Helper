//! lhelper-helper: the only part of L-Helper that runs as root, started by the app through pkexec (polkit policy
//! io.github.agentdragonblox.lhelper.control). Each command writes one /sys file, and only a value the kernel
//! driver itself advertises: a listed platform profile, an integer firmware attribute inside its min..max, an
//! ideapad switch, a listed charge type or a backlight level up to its maximum. Names are checked so no
//! argument can point outside the expected folder.
//!
//!   lhelper-helper profile <profile>
//!   lhelper-helper attribute <lenovo-wmi-other class> <attribute> <value>
//!   lhelper-helper ideapad <conservation_mode | fn_lock | usb_charging> <0 | 1>
//!   lhelper-helper charge <BATn> <charge type>
//!   lhelper-helper kbd-backlight <led> <level>

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map(|s| s.trim().to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, value: &str) -> Result<(), String> {
    std::fs::write(path, value).map_err(|e| format!("{}: {e}", path.display()))
}

/// A single path component made of [A-Za-z0-9_.:-], never "." or ".."
fn name(s: &str) -> Result<&str, String> {
    let ok = !s.is_empty() && s != "." && s != ".." && s.len() < 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c));
    if ok { Ok(s) } else { Err(format!("Invalid name: {s:?}")) }
}

fn int(s: &str) -> Result<i64, String> {
    s.parse().map_err(|_| format!("Not a number: {s:?}"))
}

fn profile(value: &str) -> Result<(), String> {
    let choices = read(Path::new("/sys/firmware/acpi/platform_profile_choices"))?;
    if !choices.split_whitespace().any(|c| c == value) {
        return Err(format!("Profile {value:?} isn't one of: {choices}"));
    }
    write(Path::new("/sys/firmware/acpi/platform_profile"), value)
}

fn attribute(class: &str, attr: &str, value: &str) -> Result<(), String> {
    let class = name(class)?;
    if !class.starts_with("lenovo-wmi-other") {
        return Err(format!("Not a Lenovo gaming firmware class: {class}"));
    }
    let dir = Path::new("/sys/class/firmware-attributes").join(class).join("attributes").join(name(attr)?);
    if let Ok(kind) = read(&dir.join("type")) {
        if kind != "integer" {
            return Err(format!("{attr} isn't an integer setting"));
        }
    }
    let (v, min, max) = (int(value)?, int(&read(&dir.join("min_value"))?)?, int(&read(&dir.join("max_value"))?)?);
    if v < min || v > max {
        return Err(format!("{attr}: {v} is outside {min}..{max}"));
    }
    write(&dir.join("current_value"), &v.to_string())
}

fn ideapad_device() -> Result<PathBuf, String> {
    let dir = std::fs::read_dir("/sys/bus/platform/drivers/ideapad_acpi").map_err(|_| "ideapad_laptop isn't loaded".to_string())?;
    dir.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().contains(':'))).ok_or_else(|| "No ideapad_laptop device".into())
}

fn ideapad(setting: &str, value: &str) -> Result<(), String> {
    if !["conservation_mode", "fn_lock", "usb_charging"].contains(&setting) {
        return Err(format!("Unknown ideapad setting: {setting}"));
    }
    if value != "0" && value != "1" {
        return Err(format!("{setting} takes 0 or 1"));
    }
    write(&ideapad_device()?.join(setting), value)
}

fn charge(supply: &str, kind: &str) -> Result<(), String> {
    let supply = name(supply)?;
    if !supply.starts_with("BAT") {
        return Err(format!("Not a battery: {supply}"));
    }
    let file = Path::new("/sys/class/power_supply").join(supply).join("charge_types");
    let options = read(&file)?;
    if !options.split_whitespace().any(|o| o.trim_start_matches('[').trim_end_matches(']') == kind) {
        return Err(format!("Charge type {kind:?} isn't one of: {options}"));
    }
    write(&file, kind)
}

fn kbd_backlight(led: &str, level: &str) -> Result<(), String> {
    let led = name(led)?;
    if !led.ends_with("kbd_backlight") {
        return Err(format!("Not a keyboard backlight: {led}"));
    }
    let dir = Path::new("/sys/class/leds").join(led);
    let (v, max) = (int(level)?, int(&read(&dir.join("max_brightness"))?)?);
    if v < 0 || v > max {
        return Err(format!("Level {v} is outside 0..{max}"));
    }
    write(&dir.join("brightness"), &v.to_string())
}

fn run(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["profile", value] => profile(value),
        ["attribute", class, attr, value] => attribute(class, attr, value),
        ["ideapad", setting, value] => ideapad(setting, value),
        ["charge", supply, kind] => charge(supply, kind),
        ["kbd-backlight", led, level] => kbd_backlight(led, level),
        _ => Err("Usage: lhelper-helper profile | attribute | ideapad | charge | kbd-backlight ...".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lhelper-helper: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::name;

    #[test]
    fn names() {
        assert!(name("lenovo-wmi-other-0").is_ok());
        assert!(name("ppt_pl1_spl").is_ok());
        assert!(name("platform::kbd_backlight").is_ok());
        assert!(name("..").is_err());
        assert!(name("a/b").is_err());
        assert!(name("").is_err());
    }
}
