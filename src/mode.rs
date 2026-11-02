//! Performance modes: GameZone firmware modes on gaming models, ITS modes on IdeaPads, plus Custom limits / fan curve

use crate::config;
use crate::lenovo::gamezone::{self, Cap};
use crate::lenovo::its::{self, ItsMode};
use crate::lenovo::{fans, machine, sensors};
use crate::win::power;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Quiet = 0,
    Balanced = 1,
    Performance = 2,
    Extreme = 3,
    Custom = 4,
}

impl Mode {
    pub fn from_index(i: i64) -> Option<Mode> {
        [Mode::Quiet, Mode::Balanced, Mode::Performance, Mode::Extreme, Mode::Custom].get(i as usize).copied()
    }

    pub fn name(self) -> &'static str {
        if machine::get().is_ideapad() {
            match self {
                Mode::Quiet => "Battery Saving",
                Mode::Balanced => "Intelligent",
                Mode::Performance => "Extreme",
                Mode::Extreme if its::geek_is_creator() => "Creator",
                Mode::Extreme => "Geek",
                Mode::Custom => "Custom",
            }
        } else {
            match self {
                Mode::Quiet => "Quiet",
                Mode::Balanced => "Balanced",
                Mode::Performance => "Performance",
                Mode::Extreme => "Extreme",
                Mode::Custom => "Custom",
            }
        }
    }

    pub fn hint(self) -> &'static str {
        if machine::get().is_ideapad() {
            match self {
                Mode::Quiet => "Intelligent Battery Saving: quiet, longest battery life",
                Mode::Balanced => "Intelligent Cooling: Lenovo adjusts power to the workload",
                Mode::Performance => "Extreme Performance: highest firmware power limits",
                Mode::Extreme => "Geek mode: unlocked limits on supported AMD models (AC only)",
                Mode::Custom => "Extreme Performance with your own CPU limits (Power window)",
            }
        } else {
            match self {
                Mode::Quiet => "Low noise, lowest power limits (Fn+Q blue)",
                Mode::Balanced => "Firmware balanced mode (Fn+Q white)",
                Mode::Performance => "Firmware performance mode (Fn+Q red)",
                Mode::Extreme => "Firmware extreme mode",
                Mode::Custom => "Your own power limits and fan curve (Fn+Q purple)",
            }
        }
    }

    fn to_firmware(self) -> i32 {
        match self {
            Mode::Quiet => 1,
            Mode::Balanced => 2,
            Mode::Performance => 3,
            Mode::Extreme => 224,
            Mode::Custom => 255,
        }
    }

    pub fn from_firmware(v: i32) -> Mode {
        match v {
            1 => Mode::Quiet,
            3 => Mode::Performance,
            224 => Mode::Extreme,
            255 => Mode::Custom,
            _ => Mode::Balanced,
        }
    }

    fn to_its(self) -> ItsMode {
        match self {
            Mode::Quiet => ItsMode::Cool,
            Mode::Balanced => ItsMode::Auto,
            Mode::Extreme => ItsMode::Geek,
            _ => ItsMode::Performance,
        }
    }

    fn from_its(s: ItsMode) -> Option<Mode> {
        match s {
            ItsMode::Cool => Some(Mode::Quiet),
            ItsMode::Auto => Some(Mode::Balanced),
            // ITS has no custom state: Custom = Extreme Performance with our limits on top
            ItsMode::Performance if config::is("its_custom") => Some(Mode::Custom),
            ItsMode::Performance => Some(Mode::Performance),
            ItsMode::Geek => Some(Mode::Extreme),
            ItsMode::None => None,
        }
    }
}

pub fn available() -> Vec<Mode> {
    let m = machine::get();
    let mut v = Vec::new();
    if m.supports_quiet {
        v.push(Mode::Quiet);
    }
    if m.supports_balanced {
        v.push(Mode::Balanced);
    }
    if m.supports_performance {
        v.push(Mode::Performance);
    }
    if m.supports_extreme {
        v.push(Mode::Extreme);
    }
    if m.supports_custom {
        v.push(Mode::Custom);
    }
    v
}

pub fn get() -> Option<Mode> {
    if machine::get().is_ideapad() {
        return Mode::from_its(its::get());
    }
    match gamezone::smart_fan_mode() {
        Ok(v) => Some(Mode::from_firmware(v)),
        Err(e) => {
            crate::log!("Get mode failed: {e}");
            None
        }
    }
}

fn remember(mode: Mode) {
    config::set("performance_mode", mode as i64);
    config::set(if sensors::plugged_in() { "performance_mode_ac" } else { "performance_mode_dc" }, mode as i64);
}

pub fn set(mode: Mode) -> Result<(), String> {
    let m = machine::get();
    let current = get();
    crate::log!("Mode: {current:?} -> {mode:?}");
    crate::app::mark_own_mode_change();

    if m.is_ideapad() {
        its::set(mode.to_its())?;
        config::set("its_custom", (mode == Mode::Custom) as i64);
        if mode == Mode::Custom {
            apply_custom();
        }
        // Lenovo's dispatcher sets its own Windows power mode right after a switch; apply ours after it
        apply_windows_power_mode(mode);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            apply_windows_power_mode(mode);
        });
        remember(mode);
        return Ok(());
    }

    // Some Legion BIOSes misbehave going straight Quiet -> Performance; route through Balanced (LLT workaround)
    if (config::is("mode_workaround") || m.has_quiet_to_performance_bug()) && current == Some(Mode::Quiet) && mode == Mode::Performance {
        gamezone::set_smart_fan_mode(Mode::Balanced.to_firmware())?;
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    // K1CN BIOS: leaving Custom needs an intermediate hop
    if m.has_custom_to_other_bug() && current == Some(Mode::Custom) && mode != Mode::Custom {
        let hop = match mode {
            Mode::Quiet => Mode::Performance,
            Mode::Balanced => Mode::Quiet,
            Mode::Performance => Mode::Balanced,
            other => other,
        };
        gamezone::set_smart_fan_mode(hop.to_firmware())?;
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    gamezone::set_smart_fan_mode(mode.to_firmware())?;
    // Firmware can refuse modes (e.g. Custom on battery on some BIOSes): read back so we never pretend it worked
    std::thread::sleep(std::time::Duration::from_millis(150));
    if let Ok(actual) = gamezone::smart_fan_mode().map(Mode::from_firmware) {
        if actual != mode {
            crate::log!("Firmware kept {actual:?} instead of {mode:?}");
            return Err(format!(
                "The firmware didn't accept {} mode{} (it stayed in {}).",
                mode.name(),
                if sensors::plugged_in() { "" } else { " on battery" },
                actual.name()
            ));
        }
    }
    if mode == Mode::Custom {
        apply_custom();
    }
    apply_windows_power_mode(mode);
    remember(mode);
    Ok(())
}

pub fn apply_windows_power_mode(mode: Mode) {
    if config::get("windows_power_mode", 1) == 0 {
        return;
    }
    let scheme = config::get_string(&format!("powermode_{}", mode as i64)).unwrap_or_else(|| {
        match mode {
            Mode::Quiet => power::POWER_SILENT,
            Mode::Balanced => power::POWER_BALANCED,
            _ => power::POWER_TURBO,
        }
        .to_string()
    });
    power::set_power_mode(&scheme);
}

/// Mode remembered for the current power source (G-Helper behaviour)
pub fn auto_performance() {
    let key = if sensors::plugged_in() { "performance_mode_ac" } else { "performance_mode_dc" };
    let mut saved = config::get(key, -1);
    if saved < 0 {
        saved = config::get("performance_mode", -1);
    }
    let Some(mode) = Mode::from_index(saved) else { return };
    if !available().contains(&mode) || (get() == Some(mode) && mode != Mode::Custom) {
        return;
    }
    if let Err(e) = set(mode) {
        crate::log!("Auto mode failed: {e}");
    }
}

/// IdeaPad Fn+Q (Lenovo Hotkeys disabled): Battery Saving → Intelligent → Extreme (→ Geek on AC), like LLT
pub fn cycle_its() -> Mode {
    let mut order = vec![Mode::Quiet, Mode::Balanced, Mode::Performance];
    if machine::get().supports_extreme && sensors::plugged_in() {
        order.push(Mode::Extreme);
    }
    let current = get().unwrap_or(Mode::Balanced);
    let index = order.iter().position(|m| *m == current).map(|i| i + 1).unwrap_or(0);
    let next = order[index % order.len()];
    if let Err(e) = set(next) {
        crate::log!("ITS cycle failed: {e}");
    }
    next
}

// ---------------- Custom mode ----------------

pub const POWER_CAPS: [Cap; 12] = [
    Cap::CpuLongTerm,
    Cap::CpuShortTerm,
    Cap::CpuPeak,
    Cap::CpuCrossLoading,
    Cap::ApuSppt,
    Cap::CpuPl2Tau,
    Cap::CpuTempLimit,
    Cap::GpuTgp,
    Cap::GpuPowerBoost,
    Cap::GpuTppOffset,
    Cap::GpuToCpuBoost,
    Cap::GpuTempLimit,
];

/// Custom mode has two profiles: plugged in (keys as in the C# version) and on battery ("_dc" keys)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Ac,
    Battery,
}

impl Profile {
    pub fn current() -> Profile {
        if sensors::plugged_in() { Profile::Ac } else { Profile::Battery }
    }
    fn suffix(self) -> &'static str {
        match self {
            Profile::Ac => "",
            Profile::Battery => "_dc",
        }
    }
    pub fn cap_key(self, id: Cap) -> String {
        format!("cap_{:08X}{}", id as u32, self.suffix())
    }
    pub fn table_key(self) -> String {
        format!("fan_table{}", self.suffix())
    }
    pub fn full_speed_key(self) -> String {
        format!("fan_full_speed{}", self.suffix())
    }
    /// A profile counts as set up once it has been saved from the Fans + Power window
    pub fn is_saved(self) -> bool {
        match self {
            Profile::Ac => config::exists(&self.table_key()) || POWER_CAPS.iter().any(|id| config::exists(&self.cap_key(*id))),
            Profile::Battery => config::is("custom_dc_saved"),
        }
    }

    fn other(self) -> Profile {
        match self {
            Profile::Ac => Profile::Battery,
            Profile::Battery => Profile::Ac,
        }
    }
}


/// Applies saved Custom limits and fan curve. Firmware must already be in Custom (255) / Extreme Performance.
pub fn apply_custom() {
    // Use the profile for the current power source; if only the other one was ever saved, use that rather than
    // leaving whatever curve and limits the firmware (or another app) had
    let current = Profile::current();
    let profile = if current.is_saved() || !current.other().is_saved() { current } else { current.other() };
    crate::log!("Applying custom mode settings ({profile:?} profile)");
    for id in POWER_CAPS {
        let v = config::get(&profile.cap_key(id), i64::MIN);
        if v == i64::MIN {
            continue;
        }
        match gamezone::set_feature(id, v as i32) {
            Ok(_) => crate::log!("  {id:?} = {v}"),
            Err(e) => crate::log!("  {id:?} = {v} failed: {e}"),
        }
    }

    let full = config::is(&profile.full_speed_key());
    if full || fans::full_speed() == Some(true) {
        if let Err(e) = fans::set_full_speed(full) {
            crate::log!("Fan full speed failed: {e}");
        }
    }
    if let Some(table) = config::get_array(&profile.table_key()).or_else(|| config::get_array("fan_table")) {
        if !full && !machine::get().is_ideapad() {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if let Err(e) = fans::set_table(&table) {
                crate::app::report(Err(e));
            }
        }
    }
}
