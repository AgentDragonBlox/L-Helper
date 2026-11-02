//! OLED burn-in protection (offered when the internal panel is OLED): auto-hides the taskbar, the most static
//! thing on screen, and turns the display off sooner (3 min plugged in, 2 min on battery). The previous taskbar
//! and display timeout settings are saved and restored exactly when it's turned off.

use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA};

use crate::config;

const AC_TIMEOUT_MIN: u32 = 3;
const DC_TIMEOUT_MIN: u32 = 2;

pub fn enabled() -> bool {
    config::is("oled_care")
}

fn taskbar_autohide() -> bool {
    let mut data = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) as u32 & ABS_AUTOHIDE != 0 }
}

fn set_taskbar_autohide(on: bool) {
    let mut data = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    data.lParam = windows::Win32::Foundation::LPARAM(if on { ABS_AUTOHIDE as isize } else { 0 });
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut data);
    }
}

const SUB_VIDEO: windows::core::GUID = windows::core::GUID::from_u128(0x7516b95f_f776_4464_8c53_06167f40cc99);
const VIDEO_IDLE: windows::core::GUID = windows::core::GUID::from_u128(0x3c0bc021_c8a8_4e07_a973_6b14cbcb2b7e);

fn active_scheme() -> Option<windows::core::GUID> {
    use windows::Win32::System::Power::PowerGetActiveScheme;
    unsafe {
        let mut ptr = std::ptr::null_mut();
        if PowerGetActiveScheme(None, &mut ptr).is_err() || ptr.is_null() {
            return None;
        }
        let guid = *ptr;
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(ptr as _)));
        Some(guid)
    }
}

/// Current "turn off display after" of the active power plan, in seconds (AC, DC)
fn display_timeouts() -> Option<(u32, u32)> {
    use windows::Win32::System::Power::{PowerReadACValueIndex, PowerReadDCValueIndex};
    let scheme = active_scheme()?;
    let (mut ac, mut dc) = (0u32, 0u32);
    unsafe {
        if PowerReadACValueIndex(None, Some(&scheme), Some(&SUB_VIDEO), Some(&VIDEO_IDLE), &mut ac).is_err()
            || PowerReadDCValueIndex(None, Some(&scheme), Some(&SUB_VIDEO), Some(&VIDEO_IDLE), &mut dc) != 0
        {
            return None;
        }
    }
    Some((ac, dc))
}

fn set_display_timeouts(ac_seconds: u32, dc_seconds: u32) {
    use windows::Win32::System::Power::{PowerSetActiveScheme, PowerWriteACValueIndex, PowerWriteDCValueIndex};
    let Some(scheme) = active_scheme() else { return };
    unsafe {
        let _ = PowerWriteACValueIndex(None, &scheme, Some(&SUB_VIDEO), Some(&VIDEO_IDLE), ac_seconds);
        let _ = PowerWriteDCValueIndex(None, &scheme, Some(&SUB_VIDEO), Some(&VIDEO_IDLE), dc_seconds);
        let _ = PowerSetActiveScheme(None, Some(&scheme));
    }
}

pub fn set(on: bool) {
    if on == enabled() {
        return;
    }
    if on {
        // Remember what to restore
        config::set("oled_prev_autohide", taskbar_autohide() as i64);
        if let Some((ac, dc)) = display_timeouts() {
            config::set("oled_prev_ac", ac as i64);
            config::set("oled_prev_dc", dc as i64);
        }
        set_taskbar_autohide(true);
        set_display_timeouts(AC_TIMEOUT_MIN * 60, DC_TIMEOUT_MIN * 60);
    } else {
        set_taskbar_autohide(config::is("oled_prev_autohide"));
        let (ac, dc) = (config::get("oled_prev_ac", -1), config::get("oled_prev_dc", -1));
        if ac >= 0 && dc >= 0 {
            set_display_timeouts(ac as u32, dc as u32);
        }
    }
    config::set("oled_care", on as i64);
    crate::log!("OLED burn-in protection: {on}");
}
