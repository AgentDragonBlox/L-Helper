//! Performance modes through the kernel's platform_profile: lenovo-wmi-gamezone on LOQ / Legion (kernel 6.16+),
//! ideapad_laptop (DYTC) on IdeaPads. Fn+Q changes it in firmware; the background loop notices (app.rs).

use once_cell::sync::Lazy;

const PROFILE: &str = "/sys/firmware/acpi/platform_profile";
const CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Quiet,
    Balanced,
    Performance,
    Extreme,
    Custom,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Quiet => "Quiet",
            Mode::Balanced => "Balanced",
            Mode::Performance => "Performance",
            Mode::Extreme => "Extreme",
            Mode::Custom => "Custom",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Mode::Quiet => "Low noise and power",
            Mode::Balanced => "Balanced noise and performance",
            Mode::Performance => "High performance, louder fans",
            Mode::Extreme => "Highest performance the firmware allows",
            Mode::Custom => "Your own power limits (Power limits)",
        }
    }

    pub fn from_index(i: i64) -> Option<Mode> {
        [Mode::Quiet, Mode::Balanced, Mode::Performance, Mode::Extreme, Mode::Custom].get(i as usize).copied()
    }
}

fn rank(profile: &str) -> Option<u8> {
    match profile {
        "low-power" | "quiet" | "cool" => Some(0),
        "balanced" => Some(1),
        "balanced-performance" => Some(2),
        "performance" => Some(3),
        "max-power" => Some(4),
        _ => None,
    }
}

/// Each mode L-Helper offers and the platform_profile value behind it. Drivers name the firmware's modes
/// differently (gamezone has used both performance + max-power and balanced-performance + performance for
/// Performance + Extreme), so modes go by order: the two highest profiles are Performance and Extreme.
fn mapping_from(choices: &str) -> Vec<(Mode, String)> {
    let mut ranked: Vec<(u8, &str)> = choices.split_whitespace().filter_map(|c| rank(c).map(|r| (r, c))).collect();
    ranked.sort();
    let mut out = Vec::new();
    let mut high = Vec::new();
    for (r, c) in ranked {
        match r {
            0 if !out.iter().any(|(m, _)| *m == Mode::Quiet) => out.push((Mode::Quiet, c.to_string())),
            0 => {}
            1 => out.push((Mode::Balanced, c.to_string())),
            _ => high.push(c),
        }
    }
    match high.len() {
        0 => {}
        1 => out.push((Mode::Performance, high[0].to_string())),
        n => {
            out.push((Mode::Performance, high[n - 2].to_string()));
            out.push((Mode::Extreme, high[n - 1].to_string()));
        }
    }
    if choices.split_whitespace().any(|c| c == "custom") {
        out.push((Mode::Custom, "custom".into()));
    }
    out
}

static MAPPING: Lazy<Vec<(Mode, String)>> = Lazy::new(|| {
    let choices = super::read(CHOICES).unwrap_or_default();
    let mapping = mapping_from(&choices);
    crate::log!("platform_profile choices: [{choices}] -> {:?}", mapping.iter().map(|(m, p)| format!("{}={p}", m.name())).collect::<Vec<_>>());
    mapping
});

pub fn available() -> Vec<Mode> {
    MAPPING.iter().map(|(m, _)| *m).collect()
}

/// Current platform_profile value as the kernel names it
pub fn raw() -> Option<String> {
    super::read(PROFILE)
}

pub fn from_raw(raw: &str) -> Option<Mode> {
    MAPPING.iter().find(|(_, p)| p == raw).map(|(m, _)| *m)
}

pub fn get() -> Option<Mode> {
    from_raw(&raw()?)
}

pub fn set(mode: Mode) -> Result<(), String> {
    let (_, profile) = MAPPING.iter().find(|(m, _)| *m == mode).ok_or_else(|| format!("{} isn't available on this laptop", mode.name()))?;
    crate::log!("Mode: {:?} -> {mode:?} ({profile})", get());
    super::privileged(&["profile", profile])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(choices: &str) -> Vec<(Mode, &'static str)> {
        mapping_from(choices).into_iter().map(|(m, p)| (m, &*String::leak(p))).collect()
    }

    #[test]
    fn gamezone_with_max_power() {
        assert_eq!(
            modes("low-power balanced performance max-power custom"),
            vec![(Mode::Quiet, "low-power"), (Mode::Balanced, "balanced"), (Mode::Performance, "performance"), (Mode::Extreme, "max-power"), (Mode::Custom, "custom")]
        );
    }

    #[test]
    fn gamezone_with_balanced_performance() {
        assert_eq!(
            modes("low-power balanced balanced-performance performance custom"),
            vec![(Mode::Quiet, "low-power"), (Mode::Balanced, "balanced"), (Mode::Performance, "balanced-performance"), (Mode::Extreme, "performance"), (Mode::Custom, "custom")]
        );
    }

    #[test]
    fn ideapad() {
        assert_eq!(modes("low-power balanced performance"), vec![(Mode::Quiet, "low-power"), (Mode::Balanced, "balanced"), (Mode::Performance, "performance")]);
    }

    #[test]
    fn none() {
        assert!(modes("").is_empty());
    }
}
