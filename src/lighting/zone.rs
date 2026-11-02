// 24-zone keyboard driver, adapted from LOQ-24-zone-RGB-controller (rust-backend/src/led_driver.rs)
// Copyright (c) 2026 Dhruv Goel, MIT License. See THIRD_PARTY_NOTICES.md

use hidapi::{HidApi, HidDevice};

pub const NUM_ZONES: usize = 24;
const VID: u16 = 0x048D;
/// hidapi pads / truncates to the device's feature report size (51 bytes on LOQ 15AHP10)
const PACKET_SIZE: usize = 65;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }
    pub const BLACK: Color = Color::new(0, 0, 0);
    pub const WHITE: Color = Color::new(255, 255, 255);

    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h / 60.0;
        let i = (h.floor() as i32).rem_euclid(6);
        let f = h - h.floor();
        let p = v * (1.0 - s);
        let q = v * (1.0 - f * s);
        let t = v * (1.0 - (1.0 - f) * s);
        let (r, g, b) = match i {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        Color::new((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }

    /// Linear scale, brightness clamped to 0..1
    pub fn scale(&self, brightness: f32) -> Self {
        let b = brightness.clamp(0.0, 1.0);
        Color::new((self.r as f32 * b) as u8, (self.g as f32 * b) as u8, (self.b as f32 * b) as u8)
    }

    /// Gamma 2.2 brightness, applied as the final pass before writing
    pub fn perceptual_scale(&self, brightness: f32) -> Self {
        let b = brightness.clamp(0.0, 1.0);
        let m = if b <= 0.0 { 0.0 } else { b.powf(1.0 / 2.2) };
        let f = |c: u8| (c as f32 * m).round().clamp(0.0, 255.0) as u8;
        Color::new(f(self.r), f(self.g), f(self.b))
    }

    pub fn lerp(&self, other: &Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        Color::new(f(self.r, other.r), f(self.g, other.g), f(self.b, other.b))
    }

    /// Additive blend, clamped per channel (overlapping ripples)
    pub fn saturating_add(&self, o: &Color) -> Self {
        Color::new(self.r.saturating_add(o.r), self.g.saturating_add(o.g), self.b.saturating_add(o.b))
    }

    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    pub fn parse_hex(s: &str) -> Option<Color> {
        let v = u32::from_str_radix(s.trim_start_matches('#'), 16).ok()?;
        Some(Color::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

pub struct ZoneKeyboard {
    device: Option<HidDevice>,
    frame: [Color; NUM_ZONES],
    /// Last frame actually sent (after brightness), for the UI preview
    pub shown: [Color; NUM_ZONES],
    pub brightness: f32,
}
// Every frame is written even when unchanged, as in the original controller. When LHelper is the keyboard's
// Dynamic Lighting controller, frames go through Windows' LampArray API (lamparray.rs) instead of the vendor reports.

/// Interface 1 of the ITE controller (VID 048D). Any PID: the LOQ is C693, the Legion Pro 5 gen 10 uses another.
fn find(api: &HidApi) -> Option<HidDevice> {
    for info in api.device_list() {
        if info.vendor_id() == VID && info.interface_number() == 1 {
            if let Ok(d) = api.open_path(info.path()) {
                crate::log!("24-zone keyboard: {:04X}:{:04X}", VID, info.product_id());
                return Some(d);
            }
        }
    }
    None
}

pub fn is_present() -> bool {
    HidApi::new()
        .map(|api| api.device_list().any(|i| i.vendor_id() == VID && i.interface_number() == 1 && crate::lenovo::hid::feature_length(&i.path().to_string_lossy()).unwrap_or(0) >= 51))
        .unwrap_or(false)
}

impl ZoneKeyboard {
    pub fn new() -> Self {
        ZoneKeyboard { device: None, frame: [Color::BLACK; NUM_ZONES], shown: [Color::BLACK; NUM_ZONES], brightness: 1.0 }
    }

    pub fn connect(&mut self) -> bool {
        self.device = HidApi::new().ok().and_then(|api| find(&api));
        self.device.is_some()
    }

    pub fn reconnect(&mut self) -> bool {
        self.device = None;
        self.connect()
    }

    pub fn disconnect(&mut self) {
        self.device = None;
    }

    pub fn is_connected(&self) -> bool {
        self.device.is_some()
    }

    pub fn set_zone(&mut self, zone: usize, color: Color) {
        if zone < NUM_ZONES {
            self.frame[zone] = color;
        }
    }

    pub fn fill(&mut self, color: Color) {
        self.frame = [color; NUM_ZONES];
    }

    fn send(&mut self, buf: &[u8]) -> bool {
        let Some(d) = &self.device else { return false };
        if d.send_feature_report(buf).is_ok() {
            return true;
        }
        crate::log!("24-zone write failed");
        self.device = None;
        false
    }

    /// 0x05: one color for a zone range, applied immediately
    pub fn set_range(&mut self, start: u8, end: u8, color: Color) -> bool {
        if start > 23 || end > 23 || start > end {
            return false;
        }
        let scaled = color.perceptual_scale(self.brightness);
        for i in start..=end {
            self.frame[i as usize] = color;
            self.shown[i as usize] = scaled;
        }
        // Through Windows Dynamic Lighting when LHelper is the keyboard's controller
        if self.device.is_some() && super::lamparray::draw(&self.shown) {
            return true;
        }
        let mut buf = vec![0x05, 0x01, start, 0x00, end, 0x00, scaled.r, scaled.g, scaled.b, 0x01];
        buf.resize(PACKET_SIZE, 0);
        self.send(&buf)
    }

    pub fn set_all_instant(&mut self, color: Color) -> bool {
        self.set_range(0, 23, color)
    }

    pub fn clear(&mut self) -> bool {
        self.fill(Color::BLACK);
        self.set_all_instant(Color::BLACK)
    }

    /// 0x04: 8 zones per packet, commit on the last
    fn send_zone_packet(&mut self, start: u8, colors: &[Color], commit: bool) -> bool {
        let mut buf = vec![0x04, 0x08, commit as u8];
        for i in 0..8 {
            buf.push(start + i);
            buf.push(0);
        }
        for c in colors.iter().take(8) {
            buf.extend_from_slice(&[c.r, c.g, c.b, 0x01]);
        }
        buf.resize(PACKET_SIZE, 0);
        self.send(&buf)
    }

    pub fn flush_buffered(&mut self) -> bool {
        let mut scaled = [Color::BLACK; NUM_ZONES];
        for (i, c) in self.frame.iter().enumerate() {
            scaled[i] = c.perceptual_scale(self.brightness);
        }
        self.shown = scaled;
        // Through Windows Dynamic Lighting when LHelper is the keyboard's controller, else the vendor reports
        if self.device.is_some() && super::lamparray::draw(&scaled) {
            return true;
        }
        self.send_zone_packet(0, &scaled[0..8], false) && self.send_zone_packet(8, &scaled[8..16], false) && self.send_zone_packet(16, &scaled[16..24], true)
    }
}
