//! Internal panel refresh rate (QueryDisplayConfig finds the built-in panel, then ChangeDisplaySettingsEx)

use windows::core::PCWSTR;
use windows::Win32::Devices::Display::*;
use windows::Win32::Graphics::Gdi::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// GDI device name of the laptop panel, e.g. \\.\DISPLAY1
pub fn laptop_screen() -> Option<String> {
    internal_panel().map(|p| p.0)
}

/// The panel's own name from its EDID (e.g. "ATNA60YV02-0" for a Samsung OLED), when Windows reports it
pub fn laptop_panel_name() -> Option<String> {
    internal_panel().and_then(|p| p.1).filter(|n| !n.is_empty())
}

/// OLED laptop panels by EDID name: "OLED" in the name, or Samsung Display's ATNA / ATNB OLED series
pub fn laptop_panel_is_oled() -> bool {
    laptop_panel_name().is_some_and(|n| {
        let n = n.to_ascii_uppercase();
        n.contains("OLED") || n.starts_with("ATNA") || n.starts_with("ATNB")
    })
}

/// (GDI device name, EDID monitor name) of the built-in panel
fn internal_panel() -> Option<(String, Option<String>)> {
    unsafe {
        let (mut paths_n, mut modes_n) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths_n, &mut modes_n).0 != 0 {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); paths_n as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); modes_n as usize];
        if QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut paths_n, paths.as_mut_ptr(), &mut modes_n, modes.as_mut_ptr(), None).0 != 0 {
            return None;
        }
        for path in paths.iter().take(paths_n as usize) {
            let tech = path.targetInfo.outputTechnology;
            let internal = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
                || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL;
            if !internal {
                continue;
            }
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut source.header) == 0 {
                let len = source.viewGdiDeviceName.iter().position(|c| *c == 0).unwrap_or(32);
                let gdi = String::from_utf16_lossy(&source.viewGdiDeviceName[..len]);
                let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
                target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
                target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
                target.header.adapterId = path.targetInfo.adapterId;
                target.header.id = path.targetInfo.id;
                let name = (DisplayConfigGetDeviceInfo(&mut target.header) == 0).then(|| {
                    let n = target.monitorFriendlyDeviceName.iter().position(|c| *c == 0).unwrap_or(64);
                    String::from_utf16_lossy(&target.monitorFriendlyDeviceName[..n])
                });
                return Some((gdi, name));
            }
        }
    }
    None
}

fn devmode() -> DEVMODEW {
    DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() }
}

pub fn refresh_rate(screen: &str) -> Option<u32> {
    let name = wide(screen);
    let mut dm = devmode();
    unsafe { EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)) }
        .as_bool()
        .then_some(dm.dmDisplayFrequency)
}

/// Refresh rates available at the current resolution, ascending
pub fn refresh_rates(screen: &str) -> Vec<u32> {
    let name = wide(screen);
    let mut current = devmode();
    let mut rates = std::collections::BTreeSet::new();
    unsafe {
        if !EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut current, ENUM_DISPLAY_SETTINGS_FLAGS(0)).as_bool() {
            return Vec::new();
        }
        let mut i = 0;
        loop {
            let mut dm = devmode();
            if !EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)).as_bool() {
                break;
            }
            if dm.dmPelsWidth == current.dmPelsWidth && dm.dmPelsHeight == current.dmPelsHeight && dm.dmDisplayFrequency > 1 {
                rates.insert(dm.dmDisplayFrequency);
            }
            i += 1;
        }
    }
    rates.into_iter().collect()
}

pub fn set_refresh_rate(screen: &str, hz: u32) {
    let name = wide(screen);
    let mut dm = devmode();
    unsafe {
        if !EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)).as_bool() || dm.dmDisplayFrequency == hz {
            return;
        }
        dm.dmDisplayFrequency = hz;
        dm.dmFields = DM_DISPLAYFREQUENCY;
        let r = ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_UPDATEREGISTRY, None);
        crate::log!("Screen {screen} = {hz}Hz : {:?}", r);
    }
}

pub fn resolution(screen: &str) -> Option<(u32, u32)> {
    let name = wide(screen);
    let mut dm = devmode();
    unsafe { EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)) }
        .as_bool()
        .then_some((dm.dmPelsWidth, dm.dmPelsHeight))
}

/// Resolutions the panel supports (any refresh rate, 32-bit color), largest first
pub fn resolutions(screen: &str) -> Vec<(u32, u32)> {
    let name = wide(screen);
    let mut set = std::collections::BTreeSet::new();
    let mut i = 0;
    loop {
        let mut dm = devmode();
        if !unsafe { EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)) }.as_bool() {
            break;
        }
        if dm.dmBitsPerPel >= 32 && dm.dmPelsWidth >= 800 {
            set.insert((dm.dmPelsWidth, dm.dmPelsHeight));
        }
        i += 1;
    }
    set.into_iter().rev().collect()
}

/// Changes the resolution, keeping the current refresh rate when the new resolution offers it (else its highest)
pub fn set_resolution(screen: &str, width: u32, height: u32) {
    let name = wide(screen);
    let current_hz = refresh_rate(screen).unwrap_or(60);
    let mut best: Option<DEVMODEW> = None;
    let mut i = 0;
    loop {
        let mut dm = devmode();
        if !unsafe { EnumDisplaySettingsExW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)) }.as_bool() {
            break;
        }
        i += 1;
        if dm.dmPelsWidth != width || dm.dmPelsHeight != height || dm.dmBitsPerPel < 32 {
            continue;
        }
        let better = match &best {
            None => true,
            Some(b) => dm.dmDisplayFrequency == current_hz || (b.dmDisplayFrequency != current_hz && dm.dmDisplayFrequency > b.dmDisplayFrequency),
        };
        if better {
            best = Some(dm);
        }
    }
    let Some(mut dm) = best else { return };
    dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    let r = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_UPDATEREGISTRY, None) };
    crate::log!("Screen {screen} = {width}x{height} @ {}Hz : {:?}", dm.dmDisplayFrequency, r);
}

/// 60Hz when the panel offers it (VRR panels also list 48Hz), else the lowest
pub fn low_rate(rates: &[u32]) -> Option<u32> {
    if rates.contains(&60) {
        Some(60)
    } else {
        rates.first().copied()
    }
}
