//! \\.\EnergyDrv IOCTLs (battery charge mode, Fn lock, always-on USB, keyboard backlight, hotkeys, ITS geek mode).
//! Codes and bit layouts from Lenovo Legion Toolkit.

use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;

const IOCTL_ENERGY_BATTERY_CHARGE_MODE: u32 = 0x831020F8;
const IOCTL_ENERGY_SETTINGS: u32 = 0x831020E8;
const IOCTL_ENERGY_KEYBOARD: u32 = 0x83102144;
const IOCTL_ENERGY_SMART_POWER: u32 = 0x8310213C;
const IOCTL_KEY_WAIT_HANDLE: u32 = 0x831020D8;
const IOCTL_KEY_VALUE: u32 = 0x831020CC;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryMode {
    Conservation,
    Normal,
    RapidCharge,
}

struct Handle(HANDLE);
unsafe impl Send for Handle {}

static DRIVER: Mutex<Option<Handle>> = Mutex::new(None);

/// IdeaPad / Yoga report the Fn lock bit inverted compared to Legion / LOQ
pub static FN_LOCK_INVERTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn open() -> Result<HANDLE, String> {
    let name: Vec<u16> = "\\\\.\\EnergyDrv\0".encode_utf16().collect();
    unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .map_err(|e| format!("EnergyDrv: {e}"))
    }
}

fn ioctl(code: u32, input: &[u8], output: &mut [u8]) -> Result<(), String> {
    let mut guard = DRIVER.lock().unwrap();
    if guard.is_none() {
        *guard = Some(Handle(open()?));
    }
    let handle = guard.as_ref().unwrap().0;
    let mut returned = 0u32;
    let result = unsafe {
        DeviceIoControl(
            handle,
            code,
            Some(input.as_ptr() as _),
            input.len() as u32,
            if output.is_empty() { None } else { Some(output.as_mut_ptr() as _) },
            output.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    if let Err(e) = result {
        unsafe {
            let _ = CloseHandle(handle);
        }
        *guard = None;
        return Err(format!("DeviceIoControl 0x{code:X}: {e}"));
    }
    Ok(())
}

fn send(code: u32, value: u32) -> Result<u32, String> {
    let mut out = [0u8; 4];
    ioctl(code, &value.to_le_bytes(), &mut out)?;
    Ok(u32::from_le_bytes(out))
}

pub fn is_available() -> bool {
    let mut guard = DRIVER.lock().unwrap();
    if guard.is_some() {
        return true;
    }
    match open() {
        Ok(h) => {
            *guard = Some(Handle(h));
            true
        }
        Err(_) => false,
    }
}

// ---------- battery ----------

pub fn battery_mode() -> Option<BatteryMode> {
    match send(IOCTL_ENERGY_BATTERY_CHARGE_MODE, 0xFF) {
        Ok(state) if state & 0x20 != 0 => Some(BatteryMode::Conservation),
        Ok(state) if state & 0x04 != 0 => Some(BatteryMode::RapidCharge),
        Ok(_) => Some(BatteryMode::Normal),
        Err(e) => {
            crate::log!("Battery mode read failed: {e}");
            None
        }
    }
}

pub fn set_battery_mode(mode: BatteryMode) -> Result<(), String> {
    let codes: [u32; 2] = match mode {
        BatteryMode::Conservation => [0x08, 0x03],
        BatteryMode::Normal => [0x05, 0x08],
        BatteryMode::RapidCharge => [0x05, 0x07],
    };
    for code in codes {
        send(IOCTL_ENERGY_BATTERY_CHARGE_MODE, code)?;
    }
    crate::log!("Battery mode: {mode:?}");
    Ok(())
}

// ---------- settings: always-on USB, Fn lock ----------

pub fn always_on_usb() -> Option<bool> {
    send(IOCTL_ENERGY_SETTINGS, 0x2).ok().map(|s| s.swap_bytes() & (1 << 31) != 0)
}

pub fn set_always_on_usb(enabled: bool) -> Result<(), String> {
    send(IOCTL_ENERGY_SETTINGS, if enabled { 0xA } else { 0xB })?;
    send(IOCTL_ENERGY_SETTINGS, 0x12)?;
    crate::log!("Always-on USB: {enabled}");
    Ok(())
}

fn fn_inverted() -> bool {
    FN_LOCK_INVERTED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn fn_lock() -> Option<bool> {
    send(IOCTL_ENERGY_SETTINGS, 0x2).ok().map(|s| (s & (1 << 10) != 0) ^ fn_inverted())
}

pub fn set_fn_lock(enabled: bool) -> Result<(), String> {
    send(IOCTL_ENERGY_SETTINGS, if enabled ^ fn_inverted() { 0xE } else { 0xF })?;
    crate::log!("Fn lock: {enabled}");
    Ok(())
}

// ---------- white keyboard backlight (IdeaPad) ----------

fn keyboard_capability() -> Result<u32, String> {
    let cap = send(IOCTL_ENERGY_KEYBOARD, 0x1)?;
    if cap & 1 == 0 {
        return Err("keyboard capability invalid".into());
    }
    Ok(cap)
}

fn keyboard_guard(cap: u32) -> u32 {
    (cap & !1) << 3
}

pub fn keyboard_max_level() -> i32 {
    keyboard_capability().map(|c| (c >> 1) as i32).unwrap_or(0)
}

pub fn keyboard_level() -> Option<i32> {
    let cap = keyboard_capability().ok()?;
    let state = send(IOCTL_ENERGY_KEYBOARD, keyboard_guard(cap) | 0x2).ok()?;
    if state & 1 == 0 {
        return None;
    }
    Some(((state & 0xF) >> 1) as i32)
}

pub fn set_keyboard_level(level: i32) -> Result<(), String> {
    let cap = keyboard_capability()?;
    let level = level.clamp(0, (cap >> 1) as i32) as u32;
    send(IOCTL_ENERGY_KEYBOARD, keyboard_guard(cap) | (level << 16) | 0x3)?;
    crate::log!("Keyboard backlight level: {level}");
    Ok(())
}

// ---------- ITS geek mode ----------

pub fn geek_mode_supported() -> bool {
    send(IOCTL_ENERGY_SMART_POWER, 10).map(|c| c & 0x00020001 == 0x00020001).unwrap_or(false)
}

pub fn set_legacy_geek_mode(enabled: bool) -> Result<(), String> {
    send(IOCTL_ENERGY_SMART_POWER, if enabled { 0x001F100B } else { 0x000F100B }).map(|_| ())
}

// ---------- hotkeys (IdeaPad) ----------

/// Registers an event the driver signals on every hotkey press
pub fn bind_key_event(event: HANDLE) -> Result<(), String> {
    let mut buffer = [0u8; 16];
    buffer[..4].copy_from_slice(&(event.0 as usize as u32).to_le_bytes());
    ioctl(IOCTL_KEY_WAIT_HANDLE, &buffer, &mut [])
}

/// Reads (and clears) the pending hotkey bit mask
pub fn read_key_value() -> Result<u32, String> {
    send(IOCTL_KEY_VALUE, 0)
}
