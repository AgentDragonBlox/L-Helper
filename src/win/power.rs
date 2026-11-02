//! Windows power mode overlay (Best power efficiency / Balanced / Best performance), as in G-Helper.
//! The overlay functions are exported by powrprof.dll but missing from the SDK import library, so they're loaded at runtime.

use once_cell::sync::Lazy;
use windows::core::{s, w, GUID};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

pub const POWER_SILENT: &str = "961cc777-2547-4f9d-8174-7d86181b8a7a";
pub const POWER_BALANCED: &str = "00000000-0000-0000-0000-000000000000";
pub const POWER_TURBO: &str = "ded574b5-45a0-4f42-8737-46345c09c238";

type GetFn = unsafe extern "system" fn(*mut GUID) -> u32;
type SetFn = unsafe extern "system" fn(GUID) -> u32;

struct Api {
    get: Option<GetFn>,
    set: Option<SetFn>,
}

static API: Lazy<Api> = Lazy::new(|| unsafe {
    let Ok(lib) = LoadLibraryW(w!("powrprof.dll")) else { return Api { get: None, set: None } };
    Api {
        get: GetProcAddress(lib, s!("PowerGetEffectiveOverlayScheme")).map(|f| std::mem::transmute::<_, GetFn>(f)),
        set: GetProcAddress(lib, s!("PowerSetActiveOverlayScheme")).map(|f| std::mem::transmute::<_, SetFn>(f)),
    }
});

fn parse(s: &str) -> Option<GUID> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    u128::from_str_radix(&hex, 16).ok().map(GUID::from_u128)
}

pub fn power_mode() -> Option<GUID> {
    let get = API.get?;
    let mut g = GUID::zeroed();
    (unsafe { get(&mut g) } == 0).then_some(g)
}

pub fn set_power_mode(scheme: &str) {
    let (Some(guid), Some(set)) = (parse(scheme), API.set) else { return };
    if power_mode() == Some(guid) {
        return;
    }
    let status = unsafe { set(guid) };
    crate::log!("Windows power mode: {scheme} {}", if status == 0 { "OK".to_string() } else { status.to_string() });
}
