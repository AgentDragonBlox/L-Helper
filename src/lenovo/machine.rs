//! One-time hardware detection (model, series, generation, platform, supported features)

use once_cell::sync::OnceCell;
use std::sync::atomic::Ordering;

use super::{energy, gamezone, gamezone::Cap, its};
use crate::wmi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Series {
    Legion5,
    LegionPro5,
    LegionSlim5,
    Legion7,
    LegionPro7,
    Legion9,
    LegionLegacy,
    IdeaPad,
    IdeaPadGaming,
    LenovoSlim,
    Loq,
    Unknown,
}

/// Gaming = GameZone WMI (LOQ / Legion / IdeaPad Gaming); IdeaPad = non-gaming models with ITS modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Unknown,
    Gaming,
    IdeaPad,
}

#[derive(Debug, Clone)]
pub struct Machine {
    pub model: String,
    pub machine_type: String,
    pub bios: String,
    pub bios_prefix: String,
    pub cpu: String,
    pub series: Series,
    pub generation: i32,
    pub platform: Platform,

    pub has_gamezone: bool,
    pub smart_fan_version: i32,
    pub legion_zone_version: i32,

    pub supports_quiet: bool,
    pub supports_balanced: bool,
    pub supports_performance: bool,
    pub supports_extreme: bool,
    pub supports_custom: bool,

    pub supports_gsync: bool,
    pub supports_igpu_mode: bool,
    pub supports_overdrive: bool,
    pub supports_winkey_lock: bool,
    pub supports_touchpad_lock: bool,
    pub has_energy_driver: bool,

    pub supports_its: bool,
    pub keyboard_levels: i32,
    pub simulate_ideapad: bool,
}

static MACHINE: OnceCell<Machine> = OnceCell::new();

pub fn get() -> &'static Machine {
    MACHINE.get().expect("machine not detected")
}

impl Machine {
    pub fn is_ideapad(&self) -> bool {
        self.platform == Platform::IdeaPad
    }

    // BIOS quirks from LLT Compatibility
    pub fn has_quiet_to_performance_bug(&self) -> bool {
        self.bios_prefix.eq_ignore_ascii_case("J2CN")
    }
    pub fn has_custom_to_other_bug(&self) -> bool {
        self.bios_prefix.eq_ignore_ascii_case("K1CN")
    }

    /// Gen 10+ Legion 5 / 7 / LOQ use the "v3" fan layout where (1,4) is the PCH fan
    pub fn is_v3_fan_layout(&self) -> bool {
        self.generation >= 10
            && (matches!(self.series, Series::Legion5 | Series::Legion7 | Series::Loq)
                || ["Legion 5", "Legion 7", "Legion Pro 5 16IAX10H", "LOQ", "Y7000", "R7000"].iter().any(|m| self.model.contains(m)))
    }
}

const MACHINE_TYPES: &[(&str, Series)] = &[
    ("83LT", Series::LegionPro5), ("83F3", Series::LegionPro5), ("83DF", Series::LegionPro5), ("83F2", Series::LegionPro5),
    ("83LU", Series::LegionPro5), ("82WM", Series::LegionPro5), ("83NN", Series::LegionPro5), ("82WK", Series::LegionPro5),
    ("82JQ", Series::LegionPro5),
    ("83KY", Series::Legion7), ("83FD", Series::Legion7), ("82UH", Series::Legion7), ("82TD", Series::Legion7),
    ("82N6", Series::Legion7), ("83V9", Series::Legion7),
    ("83RU", Series::LegionPro7), ("83F5", Series::LegionPro7), ("83DE", Series::LegionPro7), ("82WR", Series::LegionPro7),
    ("82WQ", Series::LegionPro7), ("82WS", Series::LegionPro7),
    ("83G0", Series::Legion9), ("83EY", Series::Legion9),
];

fn detect_series(model: &str, machine_type: &str) -> Series {
    for (t, s) in MACHINE_TYPES {
        if machine_type.to_uppercase().starts_with(t) {
            return *s;
        }
    }
    let m = model.to_lowercase();
    let checks: &[(&str, Series)] = &[
        ("loq", Series::Loq),
        ("legion pro 5", Series::LegionPro5),
        ("legion pro 7", Series::LegionPro7),
        ("legion slim 5", Series::LegionSlim5),
        ("legion 9", Series::Legion9),
        ("legion 7", Series::Legion7),
        ("legion 5", Series::Legion5),
        ("ideapad gaming", Series::IdeaPadGaming),
        ("ideapad", Series::IdeaPad),
        ("lenovo slim", Series::LenovoSlim),
        ("legion", Series::LegionLegacy),
    ];
    checks.iter().find(|(k, _)| m.contains(k)).map(|(_, s)| *s).unwrap_or(Series::Unknown)
}

/// Same parsing as LLT: digits after a 3 letter platform code ("AHP10", "IRX10", "AKP10")
fn detect_generation(model: &str) -> i32 {
    let bytes = model.as_bytes();
    for i in 3..bytes.len() {
        if bytes[i - 3..i].iter().all(|b| b.is_ascii_alphabetic()) && bytes[i].is_ascii_digit() {
            let digits: String = model[i..].chars().take_while(|c| c.is_ascii_digit()).take(2).collect();
            if let Ok(g) = digits.parse() {
                return g;
            }
        }
    }
    0
}

/// LLT GetSupportITSMode model list
fn is_its_model(model: &str) -> bool {
    let m = model.to_lowercase();
    if m.contains("ideapad gaming") {
        return false;
    }
    ["ideapad", "thinkbook", "lenovo slim", "motobook", "xiaoxin", "yoga"].iter().any(|k| m.contains(k))
}

pub fn detect(simulate_ideapad: bool) -> &'static Machine {
    let mut m = Machine {
        model: String::new(),
        machine_type: String::new(),
        bios: String::new(),
        bios_prefix: String::new(),
        cpu: String::new(),
        series: Series::Unknown,
        generation: 0,
        platform: Platform::Unknown,
        has_gamezone: false,
        smart_fan_version: -1,
        legion_zone_version: -1,
        supports_quiet: true,
        supports_balanced: true,
        supports_performance: true,
        supports_extreme: false,
        supports_custom: false,
        supports_gsync: false,
        supports_igpu_mode: false,
        supports_overdrive: false,
        supports_winkey_lock: false,
        supports_touchpad_lock: false,
        has_energy_driver: false,
        supports_its: false,
        keyboard_levels: 0,
        simulate_ideapad,
    };

    // Win32 classes live in root\cimv2, read them from the registry instead (same data, no second namespace)
    if let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).open_subkey("HARDWARE\\DESCRIPTION\\System\\BIOS") {
        m.model = key.get_value("SystemFamily").unwrap_or_default();
        m.machine_type = key.get_value("SystemProductName").unwrap_or_default();
        m.bios = key.get_value("BIOSVersion").unwrap_or_default();
    }
    if let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0") {
        let name: String = key.get_value("ProcessorNameString").unwrap_or_default();
        m.cpu = name.trim().to_string();
    }
    if m.bios.len() >= 4 {
        m.bios_prefix = m.bios[..4].to_string();
    }

    m.series = detect_series(&m.model, &m.machine_type);
    m.generation = detect_generation(&m.model);

    m.has_gamezone = gamezone::exists() && !simulate_ideapad;
    if m.has_gamezone {
        m.platform = Platform::Gaming;
        m.smart_fan_version = gamezone::smart_fan_version().unwrap_or(-1);
        m.legion_zone_version = gamezone::legion_zone_version().unwrap_or(-1);
        detect_power_modes(&mut m);
        m.supports_gsync = gamezone::supports_gsync();
        m.supports_igpu_mode = gamezone::supports_igpu_mode();
        m.supports_overdrive = gamezone::supports_overdrive(matches!(m.series, Series::Legion7 | Series::LegionPro7) && m.generation >= 10);
        m.supports_winkey_lock = gamezone::supports_winkey_lock();
        m.supports_touchpad_lock = gamezone::supports_touchpad_lock();
    }

    m.has_energy_driver = energy::is_available();

    if !m.has_gamezone && (is_its_model(&m.model) || simulate_ideapad) {
        m.platform = Platform::IdeaPad;
        m.supports_its = its::is_available() || simulate_ideapad;
        m.supports_quiet = m.supports_its;
        m.supports_balanced = m.supports_its;
        m.supports_performance = m.supports_its;
        m.supports_extreme = m.supports_its && energy::geek_mode_supported();
        m.supports_custom = simulate_ideapad || gamezone::try_feature(Cap::CpuLongTerm).is_some();
        m.keyboard_levels = if m.has_energy_driver { energy::keyboard_max_level() } else { 0 };
    }

    energy::FN_LOCK_INVERTED.store(m.platform == Platform::IdeaPad, Ordering::Relaxed);

    crate::log!(
        "Machine: {} ({}) {:?} gen {} BIOS {} CPU {} platform={:?} v3fans={}",
        m.model, m.machine_type, m.series, m.generation, m.bios, m.cpu, m.platform, m.is_v3_fan_layout()
    );
    crate::log!(
        "GameZone={} SmartFan={} LegionZone={} Modes Q={} B={} P={} E={} C={} GSync={} IGPU={} OD={} WinKey={} TP={} Energy={} ITS={} kbLevels={}",
        m.has_gamezone, m.smart_fan_version, m.legion_zone_version, m.supports_quiet, m.supports_balanced, m.supports_performance,
        m.supports_extreme, m.supports_custom, m.supports_gsync, m.supports_igpu_mode, m.supports_overdrive, m.supports_winkey_lock,
        m.supports_touchpad_lock, m.has_energy_driver, m.supports_its, m.keyboard_levels
    );

    MACHINE.get_or_init(|| m)
}

fn detect_power_modes(m: &mut Machine) {
    // Same order as LLT: capability bitmask first, then GetSupportThermalMode
    let mut extreme_bit = false;
    let mask = match gamezone::try_feature(Cap::SupportedPowerModes) {
        Some(mask) => {
            extreme_bit = mask & (1 << 4) != 0;
            Some(mask)
        }
        None => gamezone::support_thermal_mode(),
    };
    match mask {
        Some(mask) if mask != 0 => {
            m.supports_quiet = mask & 1 != 0;
            m.supports_balanced = mask & 2 != 0;
            m.supports_performance = mask & 4 != 0;
            m.supports_custom = mask & (1 << 16) != 0;
        }
        _ => m.supports_custom = m.smart_fan_version >= 6 || m.legion_zone_version >= 3,
    }
    m.supports_extreme = extreme_bit && (matches!(m.smart_fan_version, 6..=8) || matches!(m.legion_zone_version, 3..=5));
}

#[allow(dead_code)]
pub fn wmi_ok() -> bool {
    wmi::exists("LENOVO_GAMEZONE_DATA")
}
