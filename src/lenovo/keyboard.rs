//! Keyboard backlight for models without the 24-zone controller:
//!  - Spectrum per-key RGB (960 byte feature reports on VID 048D), brightness / logo / one whole-keyboard effect
//!  - White backlight through LENOVO_LIGHTING_METHOD
//! Protocol from Lenovo Legion Toolkit.

use std::sync::Mutex;

use super::hid::{self, HidDevice, LENOVO_VID};
use crate::wmi::{self, Arg};

const SPECTRUM_LEN: usize = 960;

struct Spectrum {
    device: Option<HidDevice>,
    searched: bool,
    keys: Option<Vec<u16>>,
}

static SPECTRUM: Mutex<Spectrum> = Mutex::new(Spectrum { device: None, searched: false, keys: None });

pub fn has_spectrum() -> bool {
    let mut s = SPECTRUM.lock().unwrap();
    ensure(&mut s);
    s.device.is_some()
}

fn ensure(s: &mut Spectrum) {
    if s.searched {
        return;
    }
    s.searched = true;
    if let Some(info) = hid::enumerate(LENOVO_VID).into_iter().find(|i| i.feature_len as usize == SPECTRUM_LEN) {
        crate::log!("Spectrum keyboard {:04X}:{:04X}", LENOVO_VID, info.pid);
        s.device = HidDevice::open(&info.path);
    }
}

fn request(op: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; SPECTRUM_LEN];
    b[0] = 7;
    b[1] = op;
    b[2] = 0xC0;
    b[3] = 3;
    b[4..4 + payload.len()].copy_from_slice(payload);
    b
}

fn send(data: &[u8]) -> bool {
    let mut s = SPECTRUM.lock().unwrap();
    ensure(&mut s);
    s.device.as_ref().map(|d| d.set_feature(data)).unwrap_or(false)
}

fn send_get(data: &[u8]) -> Option<Vec<u8>> {
    let mut s = SPECTRUM.lock().unwrap();
    ensure(&mut s);
    let d = s.device.as_ref()?;
    if !d.set_feature(data) {
        return None;
    }
    let mut out = vec![0u8; SPECTRUM_LEN];
    out[0] = 7;
    d.get_feature(&mut out).then_some(out)
}

/// 0-9
pub fn spectrum_brightness() -> Option<i32> {
    send_get(&request(0xCD, &[])).map(|r| r[4] as i32)
}
pub fn set_spectrum_brightness(level: i32) {
    send(&request(0xCE, &[level.clamp(0, 9) as u8]));
}
pub fn spectrum_profile() -> Option<i32> {
    send_get(&request(0xCA, &[])).map(|r| r[4] as i32)
}
pub fn set_spectrum_profile(profile: i32) {
    send(&request(0xC8, &[profile.clamp(1, 6) as u8]));
    std::thread::sleep(std::time::Duration::from_millis(100));
}
pub fn logo() -> Option<bool> {
    send_get(&request(0xA5, &[])).map(|r| r[4] == 1)
}
pub fn set_logo(on: bool) {
    send(&request(0xA6, &[on as u8]));
}

fn one_zone() -> bool {
    wmi::query("SELECT Lighting_Type FROM LENOVO_LIGHTING_DATA")
        .ok()
        .and_then(|rows| rows.iter().map(|r| (r.i32("Lighting_Type").unwrap_or(0) >> 1) & 7).find(|t| *t != 0))
        == Some(4)
}

fn key_codes() -> Vec<u16> {
    if let Some(k) = SPECTRUM.lock().unwrap().keys.clone() {
        return k;
    }
    let Some(count) = send_get(&request(0xC4, &[7])) else { return Vec::new() };
    let (height, width) = (count[5] as usize, count[6] as usize);
    let mut keys = std::collections::BTreeSet::new();
    let mut page = |param: u8, index: u8| {
        if let Some(p) = send_get(&request(0xC5, &[param, index])) {
            for x in 0..width {
                let o = 6 + x * 3 + 1;
                if o + 1 < p.len() {
                    let code = u16::from_le_bytes([p[o], p[o + 1]]);
                    if code > 0 {
                        keys.insert(code);
                    }
                }
            }
        }
    };
    for y in 0..height {
        page(7, y as u8);
    }
    page(8, 0);
    let keys: Vec<u16> = keys.into_iter().collect();
    crate::log!("Spectrum key map {width}x{height}: {} keys", keys.len());
    SPECTRUM.lock().unwrap().keys = Some(keys.clone());
    keys
}

/// Effect types: 11 static, 2 rainbow wave, 1 rainbow screw, 3 color change, 4 pulse, 5 color wave, 6 smooth, 7 rain, 8 ripple, 12 type
pub fn set_spectrum_effect(profile: i32, effect: u8, color: Option<[u8; 3]>, speed: u8) {
    let rainbow = matches!(effect, 1 | 2);
    let uses_color = color.is_some() && !rainbow;
    let mut b: Vec<u8> = vec![7, 0xCB, 0, 0, profile as u8, 1, 1];
    b.push(1); // effect number
    b.extend_from_slice(&[0x6, 0x1, effect, 0x2, if effect == 11 { 0 } else { speed.clamp(1, 3) }]);
    b.extend_from_slice(&[0x3, if effect == 1 { 1 } else { 0 }, 0x4, if matches!(effect, 2 | 5) { 4 } else { 0 }]);
    b.extend_from_slice(&[0x5, if uses_color { 2 } else if rainbow { 0 } else { 1 }, 0x6, 0x0]);
    match color {
        Some(c) if uses_color => {
            b.push(1);
            b.extend_from_slice(&c);
        }
        _ => b.push(0),
    }
    let keys = if one_zone() { vec![1u16] } else { key_codes() };
    if keys.is_empty() {
        crate::log!("Spectrum: no key codes");
        return;
    }
    b.push(keys.len() as u8);
    for k in &keys {
        b.extend_from_slice(&k.to_le_bytes());
    }
    let len = (b.len() - 4) as u16;
    b[2..4].copy_from_slice(&len.to_le_bytes());
    b.resize(SPECTRUM_LEN, 0);
    send(&b);
    set_spectrum_profile(profile);
}

// ---------- white backlight via Lenovo Lighting WMI (Legion / LOQ) ----------

#[allow(dead_code)]
pub fn has_white_wmi() -> bool {
    wmi::query("SELECT * FROM LENOVO_LIGHTING_DATA WHERE Lighting_ID = 0 AND Control_Interface = 0 AND Lighting_Type = 1")
        .map(|r| !r.is_empty())
        .unwrap_or(false)
}

/// 0 off, 1 low, 2 high
pub fn white_level() -> Option<i32> {
    wmi::call("LENOVO_LIGHTING_METHOD", "Get_Lighting_Current_Status", &[("Lighting_ID", Arg::I32(0))])
        .ok()??
        .i32("Current_Brightness_Level")
        .map(|v| v - 1)
}

pub fn set_white_level(level: i32) -> Result<(), String> {
    wmi::call(
        "LENOVO_LIGHTING_METHOD",
        "Set_Lighting_Current_Status",
        &[("Lighting_ID", Arg::I32(0)), ("Current_State_Type", Arg::I32(0)), ("Current_Brightness_Level", Arg::I32(level.clamp(0, 2) + 1))],
    )
    .map(|_| ())
}
