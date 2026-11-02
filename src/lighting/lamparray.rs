// Windows Dynamic Lighting cooperation, started from LOQ-24-zone-RGB-controller (LampArrayManager.cs)
// Copyright (c) 2026 Dhruv Goel, MIT License. See THIRD_PARTY_NOTICES.md
//
// The keyboard is also a Windows LampArray, and Windows decides who controls it: the foreground app, else the
// background controller ranked first in Settings > Dynamic Lighting > Background light control, else Windows' own
// effect. With Dynamic Lighting off it goes to autonomous mode and the firmware ignores our zone reports.
//
// LHelper registers as a background controller through its sparse package (packaging/, gives the exe a package
// identity with the com.microsoft.windows.lighting extension). It holds the LampArray open, keeps the device
// enabled like the original controller, and pauses its own writes while Windows has handed the keyboard to
// another app (IsAvailable false after having been true), so the two never draw over each other.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Lights::LampArray;
use winreg::enums::*;
use winreg::RegKey;

static RUNNING: AtomicBool = AtomicBool::new(false);
static LAMPS: Mutex<Vec<LampArray>> = Mutex::new(Vec::new());
/// Lamp index for each of the 24 zones, left to right (from the lamps' reported positions)
static ZONE_LAMPS: Mutex<Vec<i32>> = Mutex::new(Vec::new());
/// Windows currently gives LHelper control of the keyboard
static AVAILABLE: AtomicBool = AtomicBool::new(false);
/// LHelper had control at some point this session (so "unavailable" means another app took over)
static HAD_CONTROL: AtomicBool = AtomicBool::new(false);

/// "Use Dynamic Lighting on my devices" (Settings > Personalization > Dynamic Lighting)
pub fn dynamic_lighting_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Lighting")
        .and_then(|k| k.get_value::<u32, _>("AmbientLightingEnabled"))
        .map(|v| v == 1)
        .unwrap_or(true)
}

/// Package identity from the sparse package, e.g. "LHelper_1.0.0.0_x64__abcd"; None when it isn't registered
pub fn package_name() -> Option<String> {
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut len = 0u32;
    unsafe {
        let _ = GetCurrentPackageFullName(&mut len, None);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if GetCurrentPackageFullName(&mut len, Some(windows::core::PWSTR(buf.as_mut_ptr()))).is_err() {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize]))
    }
}

/// Windows has given the keyboard to another app (e.g. a game with its own lighting): don't draw over it
pub fn yielded() -> bool {
    HAD_CONTROL.load(Ordering::SeqCst) && !AVAILABLE.load(Ordering::SeqCst)
}

/// Draws the 24 zones through Windows' LampArray API when LHelper has control. That keeps the keyboard out of
/// autonomous mode, where its firmware effect (e.g. type lighting) reacts to key presses underneath our frames.
/// Returns false when LHelper doesn't have control, so the caller falls back to the vendor HID reports.
pub fn draw(colors: &[super::zone::Color]) -> bool {
    if !AVAILABLE.load(Ordering::SeqCst) {
        return false;
    }
    let lamps = LAMPS.lock().unwrap();
    let Some(lamp) = lamps.first() else { return false };
    let order = ZONE_LAMPS.lock().unwrap();
    if order.len() != colors.len() {
        return false;
    }
    let desired: Vec<windows::UI::Color> = colors.iter().map(|c| windows::UI::Color { A: 255, R: c.r, G: c.g, B: c.b }).collect();
    match lamp.SetColorsForIndices(&desired, &order) {
        Ok(_) => true,
        Err(e) => {
            crate::log!("LampArray draw failed: {e}");
            false
        }
    }
}

/// Lamp indices sorted left to right (then top to bottom) = zone order. Assumes one lamp per zone (24 lamps).
fn zone_order(lamp: &LampArray) -> Vec<i32> {
    let count = lamp.LampCount().unwrap_or(0);
    let mut lamps: Vec<(i32, f32, f32)> = (0..count)
        .map(|i| {
            let p = lamp.GetLampInfo(i).and_then(|info| info.Position()).unwrap_or_default();
            (i, p.X, p.Y)
        })
        .collect();
    lamps.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
    crate::log!("LampArray zone order: {:?}", lamps.iter().map(|l| l.0).collect::<Vec<_>>());
    lamps.into_iter().map(|l| l.0).collect()
}

/// Opens every LampArray, keeps them enabled and tracks who Windows gives control to, until `stop`
pub fn start() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    match package_name() {
        Some(name) => crate::log!("Package identity: {name} (Dynamic Lighting background controller)"),
        None => crate::log!("No package identity: Windows' own Dynamic Lighting effect can draw over LHelper (run packaging\\install.ps1)"),
    }
    std::thread::Builder::new()
        .name("lamparray".into())
        .spawn(|| {
            let mut ticks = 0u32;
            while RUNNING.load(Ordering::SeqCst) {
                // (Re)discover devices at start and every ~10 s while none are open (resume, Dynamic Lighting toggled)
                if LAMPS.lock().unwrap().is_empty() && ticks % 20 == 0 {
                    open_all();
                }
                force_enable_all();
                track_availability();
                ticks = ticks.wrapping_add(1);
                std::thread::sleep(Duration::from_millis(500));
            }
            LAMPS.lock().unwrap().clear();
        })
        .ok();
}

pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
    // Release the devices now rather than when the keep-alive thread next wakes
    LAMPS.lock().unwrap().clear();
    AVAILABLE.store(false, Ordering::SeqCst);
    HAD_CONTROL.store(false, Ordering::SeqCst);
}

/// After sleep the LampArray objects can be stale
pub fn reopen() {
    LAMPS.lock().unwrap().clear();
}

fn open_all() {
    let result = (|| -> windows::core::Result<Vec<LampArray>> {
        let selector = LampArray::GetDeviceSelector()?;
        let devices = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
        let mut lamps = Vec::new();
        for i in 0..devices.Size()? {
            let id = devices.GetAt(i)?.Id()?;
            // Only the 24-zone keyboard (ITE, VID 048D): other LampArrays (mice, headsets) aren't ours to hold
            if !id.to_string().to_ascii_uppercase().contains("VID_048D") {
                continue;
            }
            match LampArray::FromIdAsync(&id).and_then(|op| op.join()) {
                Ok(lamp) => {
                    let _ = lamp.SetIsEnabled(true);
                    // Brightness is already applied to the colors (perceptual scale), like the HID path
                    let _ = lamp.SetBrightnessLevel(1.0);
                    if lamps.is_empty() {
                        *ZONE_LAMPS.lock().unwrap() = zone_order(&lamp);
                    }
                    crate::log!(
                        "LampArray {} lamps={} available={} enabled={}",
                        id,
                        lamp.LampCount().unwrap_or(0),
                        lamp.IsAvailable().unwrap_or(false),
                        lamp.IsEnabled().unwrap_or(false)
                    );
                    lamps.push(lamp);
                }
                Err(e) => crate::log!("LampArray {id}: {e}"),
            }
        }
        Ok(lamps)
    })();
    match result {
        Ok(lamps) => *LAMPS.lock().unwrap() = lamps,
        Err(e) => crate::log!("LampArray enumeration failed: {e}"),
    }
}

fn force_enable_all() {
    let lamps = LAMPS.lock().unwrap();
    for lamp in lamps.iter() {
        if !lamp.IsEnabled().unwrap_or(true) {
            match lamp.SetIsEnabled(true) {
                Ok(_) => crate::log!("LampArray re-enabled"),
                Err(e) => crate::log!("LampArray re-enable failed: {e}"),
            }
        }
    }
}

fn track_availability() {
    let lamps = LAMPS.lock().unwrap();
    let Some(lamp) = lamps.first() else { return };
    let now = lamp.IsAvailable().unwrap_or(false);
    if AVAILABLE.swap(now, Ordering::SeqCst) != now {
        crate::log!("Dynamic Lighting: {}", if now { "LHelper has control of the keyboard" } else { "another app has control of the keyboard" });
    }
    if now {
        HAD_CONTROL.store(true, Ordering::SeqCst);
    }
}
