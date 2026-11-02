//! Lenovo Fn keys. Gaming models report them through LENOVO_UTILITY_EVENT, IdeaPads through the energy driver.
//! Like LLT, keys are only acted on while the Lenovo Hotkeys app isn't running, so they never fire twice.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

use super::software;

/// LENOVO_UTILITY_EVENT.PressTypeDataVal values (LLT SpecialKey)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKey {
    FnLockOn,
    FnLockOff,
    FnPrtSc,
    CameraOn,
    CameraOff,
    FnR,
    SpectrumBacklight(u8), // 0 off .. 3
    SpectrumPreset(u8),    // 1..6
    FnF4,
    FnF8,
    WhiteBacklight(u8), // 0..2
    Other(i32),
}

impl SpecialKey {
    pub fn from_raw(v: i32) -> SpecialKey {
        match v {
            2 => Self::FnLockOn,
            3 => Self::FnLockOff,
            4 | 45 => Self::FnPrtSc,
            12 => Self::CameraOn,
            13 => Self::CameraOff,
            16 | 0x0041002A => Self::FnR,
            24 => Self::SpectrumBacklight(0),
            25 => Self::SpectrumBacklight(1),
            26 => Self::SpectrumBacklight(2),
            38 => Self::SpectrumBacklight(3),
            32..=37 => Self::SpectrumPreset((v - 31) as u8),
            62 => Self::FnF4,
            63 => Self::FnF8,
            64 => Self::WhiteBacklight(0),
            65 => Self::WhiteBacklight(1),
            66 => Self::WhiteBacklight(2),
            other => Self::Other(other),
        }
    }
}

/// Hotkey bits from EnergyDrv (LLT DriverKey)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKey {
    FnQ = 1,
    FnF10 = 32,
    FnF4 = 256,
    FnSpace = 4096,
    FnF8 = 8192,
}

static HOTKEYS_STATUS: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

/// Lenovo Hotkeys running? (cached 30 s; the process scan isn't free)
pub fn lenovo_hotkeys_running() -> bool {
    let mut cache = HOTKEYS_STATUS.lock().unwrap();
    if let Some((t, v)) = *cache {
        if t.elapsed() < Duration::from_secs(30) {
            return v;
        }
    }
    let running = software::FN_KEYS.status() == software::Status::Enabled;
    *cache = Some((Instant::now(), running));
    running
}

pub fn invalidate_status() {
    *HOTKEYS_STATUS.lock().unwrap() = None;
}

pub fn listen_special(handler: impl Fn(SpecialKey) + Send + 'static) -> crate::wmi::Listener {
    crate::wmi::listen("LENOVO_UTILITY_EVENT", move |e| {
        let raw = e.i32("PressTypeDataVal").unwrap_or(-1);
        let key = SpecialKey::from_raw(raw);
        crate::log!("Fn key event: {raw} ({key:?})");
        if !lenovo_hotkeys_running() {
            handler(key);
        }
    })
}

static DRIVER_KEYS_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn listen_driver(handler: impl Fn(DriverKey) + Send + 'static) {
    if DRIVER_KEYS_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("driver-keys".into())
        .spawn(move || unsafe {
            let Ok(event) = CreateEventW(None, true, false, None) else { return };
            if let Err(e) = super::energy::bind_key_event(event) {
                crate::log!("Driver hotkeys: bind failed {e}");
                let _ = CloseHandle(event);
                return;
            }
            let _ = super::energy::read_key_value();
            crate::log!("Driver hotkeys: listening");
            while DRIVER_KEYS_RUNNING.load(Ordering::SeqCst) {
                if WaitForSingleObject(event, 500) != WAIT_OBJECT_0 {
                    continue;
                }
                let value = super::energy::read_key_value().unwrap_or(0);
                let _ = ResetEvent(event);
                crate::log!("Driver hotkey: 0x{value:X}");
                if lenovo_hotkeys_running() {
                    continue;
                }
                for key in [DriverKey::FnQ, DriverKey::FnF10, DriverKey::FnF4, DriverKey::FnSpace, DriverKey::FnF8] {
                    if value & key as u32 != 0 {
                        handler(key);
                    }
                }
            }
            let _ = CloseHandle(event);
        })
        .ok();
}

pub fn stop_driver() {
    DRIVER_KEYS_RUNNING.store(false, Ordering::SeqCst);
}
