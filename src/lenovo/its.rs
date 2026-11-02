//! IdeaPad / Yoga / ThinkBook "Intelligent Thermal Solution" modes (LLT ITSModeFeature):
//! newer machines use the Lenovo Process Management dispatcher (service control codes + registry state),
//! older ones the LITSSVC service.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::PCWSTR;
use windows::Win32::System::Services::*;
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItsMode {
    None,
    Auto,        // Intelligent Cooling
    Cool,        // Battery Saving
    Performance, // Extreme Performance
    Geek,        // Geek / Creator
}

const REG_DISPATCHER: &str = r"SYSTEM\CurrentControlSet\Services\LenovoProcessManagement\Performance\PowerSlider";
const REG_LEGACY: &str = r"SYSTEM\CurrentControlSet\Services\LITSSVC\LNBITS\IC\MMC";
const DISPATCHER_V3: u32 = 8192;

static LEGACY_GEEK: AtomicBool = AtomicBool::new(false);

fn read(path: &str, name: &str) -> Option<u32> {
    RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path).ok()?.get_value::<u32, _>(name).ok()
}

pub fn dispatcher_version() -> u32 {
    read(REG_DISPATCHER, "Version").unwrap_or(0)
}

pub fn is_available() -> bool {
    dispatcher_version() >= DISPATCHER_V3 || RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(REG_LEGACY).is_ok()
}

/// Some models label Geek mode "Creator"
pub fn geek_is_creator() -> bool {
    read(REG_DISPATCHER, "ITS_FN_Capability").map(|c| c & 0x20 != 0).unwrap_or(false)
}

pub fn get() -> ItsMode {
    if dispatcher_version() >= DISPATCHER_V3 {
        let versioned = read(REG_DISPATCHER, "ITS_FN_Capability").map(|c| c & 0x10 != 0).unwrap_or(false);
        let (first, second) = if versioned { ("ITS_CurrentSettingV", "ITS_CurrentSetting") } else { ("ITS_CurrentSetting", "ITS_CurrentSettingV") };
        let current = read(REG_DISPATCHER, first).or_else(|| read(REG_DISPATCHER, second));
        return match current {
            Some(0) => ItsMode::Auto,
            Some(1) => ItsMode::Cool,
            Some(3) => ItsMode::Performance,
            Some(4) => ItsMode::Geek,
            _ => ItsMode::None,
        };
    }
    match (read(REG_LEGACY, "AutomaticModeSetting"), read(REG_LEGACY, "CurrentSetting")) {
        (Some(2), Some(0)) => ItsMode::Auto,
        (Some(1), Some(1)) => ItsMode::Cool,
        (Some(1), Some(3)) if LEGACY_GEEK.load(Ordering::Relaxed) => ItsMode::Geek,
        (Some(1), Some(3)) => ItsMode::Performance,
        _ => ItsMode::None,
    }
}

fn control(service: &str, messages: &[u32]) -> Result<(), String> {
    let name: Vec<u16> = service.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let manager = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).map_err(|e| format!("OpenSCManager: {e}"))?;
        let handle = OpenServiceW(manager, PCWSTR(name.as_ptr()), SERVICE_USER_DEFINED_CONTROL);
        let result = match handle {
            Ok(h) => {
                let mut r = Ok(());
                for m in messages {
                    let mut status = SERVICE_STATUS::default();
                    if let Err(e) = ControlService(h, *m, &mut status) {
                        r = Err(format!("ControlService {service} {m}: {e}"));
                        break;
                    }
                }
                let _ = CloseServiceHandle(h);
                r
            }
            Err(e) => Err(format!("OpenService {service}: {e}")),
        };
        let _ = CloseServiceHandle(manager);
        result
    }
}

pub fn set(mode: ItsMode) -> Result<(), String> {
    if mode == ItsMode::None {
        return Ok(());
    }
    crate::log!("ITS mode: {:?} -> {mode:?}", get());
    if dispatcher_version() >= DISPATCHER_V3 {
        let message = match mode {
            ItsMode::Auto => 163,
            ItsMode::Cool => 164,
            ItsMode::Performance => 165,
            _ => 172,
        };
        control("LenovoProcessManagement", &[message])?;
    } else {
        let messages: &[u32] = match mode {
            ItsMode::Auto => &[135],
            ItsMode::Cool => &[134, 146],
            _ => &[134, 148],
        };
        if mode != ItsMode::Geek && LEGACY_GEEK.load(Ordering::Relaxed) {
            let _ = super::energy::set_legacy_geek_mode(false);
            LEGACY_GEEK.store(false, Ordering::Relaxed);
        }
        control("LITSSVC", messages)?;
        if mode == ItsMode::Geek {
            super::energy::set_legacy_geek_mode(true)?;
            LEGACY_GEEK.store(true, Ordering::Relaxed);
        }
    }
    // The service applies changes asynchronously; wait up to 3 s like LLT
    for _ in 0..30 {
        if get() == mode {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    crate::log!("ITS mode did not confirm {mode:?}");
    Ok(())
}
