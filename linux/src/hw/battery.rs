//! Battery charge modes, the ideapad_laptop switches (Fn lock, always-on USB) and a white keyboard backlight.
//! Charge modes come from the battery's charge_types (newer kernels: Standard / Long_Life / Fast) or, on older
//! ones, ideapad_laptop's conservation_mode.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryMode {
    Conservation,
    Normal,
    RapidCharge,
}

impl BatteryMode {
    fn charge_type(self) -> &'static str {
        match self {
            BatteryMode::Conservation => "Long_Life",
            BatteryMode::Normal => "Standard",
            BatteryMode::RapidCharge => "Fast",
        }
    }

    fn from_charge_type(t: &str) -> Option<BatteryMode> {
        [BatteryMode::Conservation, BatteryMode::Normal, BatteryMode::RapidCharge].into_iter().find(|m| m.charge_type() == t)
    }
}

pub fn battery() -> Option<PathBuf> {
    super::dirs("/sys/class/power_supply").into_iter().find(|p| super::read(p.join("type")).as_deref() == Some("Battery") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("BAT")))
}

/// ideapad_laptop's platform device (VPC2004:00)
pub fn ideapad() -> Option<PathBuf> {
    super::dirs("/sys/bus/platform/drivers/ideapad_acpi").into_iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains(':')))
}

/// Options and the current one from charge_types ("[Standard] Long_Life Fast")
fn charge_types() -> Option<(Vec<String>, Option<String>)> {
    let text = super::read(battery()?.join("charge_types"))?;
    let mut current = None;
    let options = text
        .split_whitespace()
        .map(|t| {
            let bare = t.trim_start_matches('[').trim_end_matches(']').to_string();
            if t.starts_with('[') {
                current = Some(bare.clone());
            }
            bare
        })
        .collect();
    Some((options, current))
}

pub fn modes() -> Vec<BatteryMode> {
    if let Some((options, _)) = charge_types() {
        let mut modes: Vec<BatteryMode> = options.iter().filter_map(|o| BatteryMode::from_charge_type(o)).collect();
        modes.sort_by_key(|m| *m as u8);
        if modes.len() >= 2 {
            return modes;
        }
    }
    if ideapad().is_some_and(|d| d.join("conservation_mode").exists()) {
        return vec![BatteryMode::Conservation, BatteryMode::Normal];
    }
    Vec::new()
}

pub fn get() -> Option<BatteryMode> {
    if let Some((options, current)) = charge_types() {
        if options.iter().filter(|o| BatteryMode::from_charge_type(o).is_some()).count() >= 2 {
            return current.and_then(|c| BatteryMode::from_charge_type(&c));
        }
    }
    match super::read_i64(ideapad()?.join("conservation_mode"))? {
        1 => Some(BatteryMode::Conservation),
        _ => Some(BatteryMode::Normal),
    }
}

pub fn set(mode: BatteryMode) -> Result<(), String> {
    crate::log!("Battery: {mode:?}");
    if let (Some(bat), Some((options, _))) = (battery(), charge_types()) {
        if options.iter().any(|o| o == mode.charge_type()) {
            let name = bat.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            return super::privileged(&["charge", &name, mode.charge_type()]);
        }
    }
    match mode {
        BatteryMode::RapidCharge => Err("Rapid Charge isn't available on this laptop".into()),
        _ => super::privileged(&["ideapad", "conservation_mode", if mode == BatteryMode::Conservation { "1" } else { "0" }]),
    }
}

// ---------------- ideapad switches ----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    FnLock,
    UsbCharging,
}

impl Switch {
    pub fn file(self) -> &'static str {
        match self {
            Switch::FnLock => "fn_lock",
            Switch::UsbCharging => "usb_charging",
        }
    }
}

pub fn switch(s: Switch) -> Option<bool> {
    super::read_i64(ideapad()?.join(s.file())).map(|v| v != 0)
}

pub fn set_switch(s: Switch, on: bool) -> Result<(), String> {
    crate::log!("{}: {on}", s.file());
    super::privileged(&["ideapad", s.file(), if on { "1" } else { "0" }])
}

// ---------------- white keyboard backlight ----------------

fn backlight() -> Option<PathBuf> {
    super::dirs("/sys/class/leds").into_iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("kbd_backlight")))
}

/// (level, max)
pub fn keyboard_light() -> Option<(i32, i32)> {
    let dir = backlight()?;
    let max = super::read_i64(dir.join("max_brightness"))? as i32;
    (max > 0).then_some((super::read_i64(dir.join("brightness"))? as i32, max))
}

pub fn set_keyboard_light(level: i32) -> Result<(), String> {
    let dir = backlight().ok_or("No keyboard backlight")?;
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    super::privileged(&["kbd-backlight", &name, &level.to_string()])
}
