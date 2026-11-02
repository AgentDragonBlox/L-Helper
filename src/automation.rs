//! Automations: what to change when the laptop is unplugged or plugged in.
//!
//! Rules run once, at the moment the power source changes, and are never re-applied afterwards, so anything the
//! user changes by hand sticks until the next plug / unplug. "Keep" leaves a setting alone. A master switch turns
//! all of them off.
//!
//! Config: auto_rules (on/off), rule_<dc|ac>_mode / _hz / _kb / _gpu (see the constants below).

use crate::config;
use crate::gpu::{self, GpuMode};
use crate::lighting;
use crate::mode::{self, Mode};
use crate::win::display;

/// Leave this setting as it is
pub const KEEP: i64 = -1;
/// Mode: the mode last used on this power source. Keyboard: the brightness before the laptop was unplugged.
pub const RESTORE: i64 = -2;
/// Refresh rate: the panel's lowest (60 Hz when offered) / highest
pub const HZ_LOWEST: i64 = 0;
pub const HZ_HIGHEST: i64 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Battery,
    Plugged,
}

impl Source {
    fn key(self, what: &str) -> String {
        format!("rule_{}_{what}", if self == Source::Battery { "dc" } else { "ac" })
    }
    pub fn get(self, what: &str) -> i64 {
        config::get(&self.key(what), KEEP)
    }
    pub fn set(self, what: &str, value: i64) {
        config::set(&self.key(what), value);
    }
}

pub fn enabled() -> bool {
    config::is("auto_rules")
}

/// The example rules: on battery Quiet, lowest refresh rate, keyboard off; plugged in back to the remembered
/// mode, highest refresh rate and the keyboard as it was. GPU is left to Optimized mode (firmware).
pub fn use_battery_saver_rules() {
    Source::Battery.set("mode", Mode::Quiet as i64);
    Source::Battery.set("hz", HZ_LOWEST);
    Source::Battery.set("kb", 0);
    Source::Battery.set("gpu", KEEP);
    Source::Plugged.set("mode", RESTORE);
    Source::Plugged.set("hz", HZ_HIGHEST);
    Source::Plugged.set("kb", RESTORE);
    Source::Plugged.set("gpu", KEEP);
    config::set("auto_rules", 1);
}

/// Runs the rules for the new power source (called on the worker thread). Returns what was changed, for the OSD.
pub fn on_power_change(plugged: bool) -> Vec<String> {
    let mut done = Vec::new();
    if !enabled() {
        return done;
    }
    let source = if plugged { Source::Plugged } else { Source::Battery };

    // Performance mode
    match source.get("mode") {
        KEEP => {}
        RESTORE => mode::auto_performance(),
        i => {
            if let Some(m) = Mode::from_index(i).filter(|m| mode::available().contains(m)) {
                if mode::get() != Some(m) || m == Mode::Custom {
                    match mode::set(m) {
                        Ok(()) => done.push(m.name().to_string()),
                        Err(e) => crate::log!("Automation mode: {e}"),
                    }
                }
            }
        }
    }

    // Refresh rate
    let hz = source.get("hz");
    if hz != KEEP {
        if let Some(screen) = display::laptop_screen() {
            let rates = display::refresh_rates(&screen);
            let target = match hz {
                HZ_LOWEST => display::low_rate(&rates),
                HZ_HIGHEST => rates.last().copied(),
                v => rates.iter().copied().find(|r| *r as i64 == v),
            };
            if let Some(t) = target {
                if display::refresh_rate(&screen) != Some(t) {
                    display::set_refresh_rate(&screen, t);
                    done.push(format!("{t} Hz"));
                }
            }
        }
    }

    // Keyboard lighting brightness (24-zone keyboards)
    let kb = source.get("kb");
    if kb != KEEP && lighting::is_supported() {
        if !plugged {
            // remembered for "restore" when plugged back in
            config::set("kb_before_battery", lighting::brightness() as i64);
        }
        let target = if kb == RESTORE { config::get("kb_before_battery", lighting::brightness() as i64) } else { kb };
        let target = target.clamp(0, 100) as i32;
        if target != lighting::brightness() {
            lighting::set_brightness(target);
            done.push(if target == 0 { "keyboard off".to_string() } else { format!("keyboard {target}%") });
        }
    }

    // GPU mode (Eco / Standard / Optimized; Ultimate needs a restart so it isn't offered)
    let g = source.get("gpu");
    if g >= 0 && gpu::supported() {
        let target = [GpuMode::Eco, GpuMode::Standard, GpuMode::Optimized].get(g as usize).copied();
        if let Some(target) = target.filter(|t| gpu::available().contains(t)) {
            if gpu::get() != Some(target) {
                match gpu::set(target) {
                    Ok(_) => done.push(format!("GPU {}", target.name())),
                    Err(e) => crate::log!("Automation GPU: {e}"),
                }
            }
        }
    }

    if !done.is_empty() {
        crate::log!("Automations ({}): {}", if plugged { "plugged in" } else { "on battery" }, done.join(", "));
    }
    done
}
