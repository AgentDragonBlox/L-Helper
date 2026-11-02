// Lighting presets, adapted from LOQ-24-zone-RGB-controller (rust-backend/src/presets)
// Copyright (c) 2026 Dhruv Goel, MIT License. See THIRD_PARTY_NOTICES.md

use super::samplers::{audio_intensity, AudioUser, KeyHookUser, RgbF, ScreenSampler, SystemMonitor, AMBIENT_HEIGHT};
use super::zone::{Color, ZoneKeyboard, NUM_ZONES};
use std::f32::consts::PI;

const N: usize = NUM_ZONES;

pub trait Effect: Send {
    fn start(&mut self) {}
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, delta: f32);
    fn stop(&mut self, c: &mut ZoneKeyboard) {
        let _ = c.clear();
    }
}

fn distance_from_center(zone: usize) -> f32 {
    let center = N as f32 / 2.0;
    (zone as f32 - center).abs() / center
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Pseudo random 0..1 per zone and time, shared by the sparkle presets
fn hash(i: usize, time: f32) -> f32 {
    ((i as f32 * 12.9898 + time * 78.233).sin() * 43758.5453).fract()
}

pub struct Off;
impl Effect for Off {
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, _: f32) {
        c.set_all_instant(Color::BLACK);
    }
}

pub struct Static(pub Color);
impl Effect for Static {
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, _: f32) {
        c.set_all_instant(self.0);
    }
}

pub struct ColorBreath {
    pub color: Color,
    pub speed: f32,
}
impl Effect for ColorBreath {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        let b = (time * self.speed * PI * 2.0).sin() + 1.0;
        c.set_all_instant(self.color.scale(b));
    }
}

pub struct PulseCenter {
    pub color: Color,
    pub speed: f32,
}
impl Effect for PulseCenter {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        for i in 0..N {
            let wave = ((distance_from_center(i) * 3.0 - time * self.speed).cos() * 0.5 + 0.5).powf(2.0);
            c.set_zone(i, self.color.scale(wave));
        }
        c.flush_buffered();
    }
}

pub struct RainbowCycle(pub f32);
impl Effect for RainbowCycle {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        c.set_all_instant(Color::from_hsv((time * self.0 * 60.0) % 360.0, 1.0, 1.0));
    }
}

pub struct RainbowWave(pub f32);
impl Effect for RainbowWave {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        for i in 0..N {
            c.set_zone(i, Color::from_hsv(((i as f32 / N as f32) * 360.0 + time * self.0 * 60.0) % 360.0, 1.0, 1.0));
        }
        c.flush_buffered();
    }
}

pub struct RainbowBreath(pub f32);
impl Effect for RainbowBreath {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        let hue = (time * self.0 * 60.0) % 360.0;
        let b = ((time * self.0 * PI).sin() + 1.0) / 2.0;
        c.set_all_instant(Color::from_hsv(hue, 1.0, b));
    }
}

pub struct ColorWheel(pub f32);
impl Effect for ColorWheel {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        let base = (time * self.0 * 60.0) % 360.0;
        for i in 0..N {
            c.set_zone(i, Color::from_hsv((base + (i as f32 / N as f32) * 60.0) % 360.0, 1.0, 1.0));
        }
        c.flush_buffered();
    }
}

pub struct RgbSweep(u32);
impl RgbSweep {
    pub fn new() -> Self {
        RgbSweep(0)
    }
}
impl Effect for RgbSweep {
    fn start(&mut self) {
        self.0 = 0;
    }
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, _: f32) {
        c.set_all_instant(Color::new((self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8));
        self.0 = (self.0 + 1) & 0x00FF_FFFF;
    }
}

pub struct Aurora(pub f32);
impl Effect for Aurora {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        for i in 0..N {
            let flow = (i as f32 / N as f32 + time * self.0).fract();
            let b = smoothstep(0.0, 0.5, (flow - 0.5).abs());
            c.set_zone(i, Color::from_hsv(160.0 + flow * 80.0, 0.8, b));
        }
        c.flush_buffered();
    }
}

pub struct Nebula(pub f32);
impl Effect for Nebula {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        for i in 0..N {
            let drift = (time * self.0 + (i as f32 / N as f32) * 3.0).sin();
            c.set_zone(i, Color::from_hsv(260.0 + drift * 40.0, 0.7, (drift * 0.4 + 0.6).clamp(0.0, 1.0)));
        }
        c.flush_buffered();
    }
}

pub struct ChromaticBreath(pub f32);
impl Effect for ChromaticBreath {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        const COLORS: [f32; 20] = [0.0, 305.0, 24.0, 265.0, 48.0, 225.0, 75.0, 185.0, 120.0, 165.0, 145.0, 205.0, 95.0, 245.0, 60.0, 285.0, 36.0, 325.0, 12.0, 345.0];
        let cycle = 8.0 / self.0;
        let t = time % (COLORS.len() as f32 * cycle);
        let index = (t / cycle) as usize;
        let local = (t % cycle) / cycle;
        c.set_all_instant(Color::from_hsv(COLORS[index % COLORS.len()], 1.0, (local * PI).sin()));
    }
}

#[derive(PartialEq)]
enum Phase {
    Compress,
    Lock,
    Implode,
    Explode,
}

pub struct ColorScan {
    phase: Phase,
    progress: f32,
    speed: f32,
}
impl ColorScan {
    pub fn new(speed: f32) -> Self {
        ColorScan { phase: Phase::Compress, progress: 0.0, speed }
    }
}
impl Effect for ColorScan {
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, delta: f32) {
        let center = N as f32 / 2.0;
        self.progress += delta * self.speed;
        let eased = match self.phase {
            Phase::Compress => self.progress.powi(3),
            Phase::Implode => 1.0 - (1.0 - self.progress).powi(3),
            Phase::Explode => 1.0 - (1.0 - self.progress).powi(4),
            Phase::Lock => self.progress,
        };
        match self.phase {
            Phase::Compress if self.progress >= 1.0 => (self.phase, self.progress) = (Phase::Lock, 0.0),
            Phase::Lock if self.progress >= 0.2 => (self.phase, self.progress) = (Phase::Implode, 0.0),
            Phase::Implode if self.progress >= 1.0 => (self.phase, self.progress) = (Phase::Explode, 0.0),
            Phase::Explode if self.progress >= 1.0 => (self.phase, self.progress) = (Phase::Compress, 0.0),
            _ => {}
        }
        for z in 0..N {
            let dist = (z as f32 - center).abs();
            let color = match self.phase {
                Phase::Compress => {
                    let threshold = center * (1.0 - eased);
                    let i = ((dist - threshold).max(0.0) / 2.0).min(1.0);
                    Color::new((i * 100.0) as u8, (i * 180.0) as u8, (i * 255.0) as u8)
                }
                Phase::Lock => {
                    let pulse = (self.progress * 10.0).sin().abs();
                    if dist <= 1.5 { Color::WHITE } else { Color::new((pulse * 50.0) as u8, 0, 0) }
                }
                Phase::Implode => {
                    let fade = 1.0 - eased;
                    if dist <= (1.0 - eased) * center { Color::new((fade * 255.0) as u8, (fade * 255.0) as u8, (fade * 255.0) as u8) } else { Color::BLACK }
                }
                Phase::Explode => {
                    let edge = 1.0 - ((dist - eased * center).abs() / 3.0).min(1.0);
                    let p = 1.0 - eased;
                    Color::new((edge * p * 255.0) as u8, (edge * p * 100.0) as u8, (edge * p * 50.0) as u8)
                }
            };
            c.set_zone(z, color);
        }
        c.flush_buffered();
    }
}

pub struct Sparkle(pub f32);
impl Effect for Sparkle {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        c.fill(Color::BLACK);
        for i in 0..N {
            if hash(i, time) > 1.0 - self.0 {
                c.set_zone(i, Color::WHITE);
            }
        }
        c.flush_buffered();
    }
}

pub struct Horse {
    pub position: f32,
    pub speed: f32,
    pub length: f32,
    pub base: Color,
    pub horse: Color,
}
impl Effect for Horse {
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, delta: f32) {
        let n = N as f32;
        self.position = (self.position + delta * self.speed) % n;
        for i in 0..N {
            let d = ((i as f32 - self.position + n) % n).min((self.position - i as f32 + n) % n);
            c.set_zone(i, self.base.lerp(&self.horse, (1.0 - d / self.length).clamp(0.0, 1.0)));
        }
        c.flush_buffered();
    }
}

pub struct HorseCycle {
    position: f32,
    speed: f32,
    length: f32,
    direction: f32,
    hue: f32,
}
impl HorseCycle {
    pub fn new(speed: f32, length: f32) -> Self {
        HorseCycle { position: 0.0, speed, length, direction: 1.0, hue: 30.0 }
    }
}
impl Effect for HorseCycle {
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, delta: f32) {
        self.position += self.direction * delta * self.speed;
        let max = N as f32 - 1.0;
        let mut bounced = false;
        if self.position <= 0.0 {
            (self.position, self.direction, bounced) = (0.0, 1.0, true);
        } else if self.position >= max {
            (self.position, self.direction, bounced) = (max, -1.0, true);
        }
        if bounced {
            self.hue = (self.hue + 60.0) % 360.0;
        }
        let base = Color::from_hsv((self.hue + 180.0) % 360.0, 0.25, 0.45);
        let horse = Color::from_hsv(self.hue, 1.0, 1.0);
        for i in 0..N {
            c.set_zone(i, base.lerp(&horse, (1.0 - (i as f32 - self.position).abs() / self.length).clamp(0.0, 1.0)));
        }
        c.flush_buffered();
    }
}

pub struct FerrariRpm {
    pos: f32,
    direction: f32,
    rpm: f32,
    shift_flash: i32,
    backfire: i32,
}
impl FerrariRpm {
    pub fn new() -> Self {
        FerrariRpm { pos: -6.0, direction: 1.0, rpm: 0.2, shift_flash: 0, backfire: 0 }
    }
}
impl Effect for FerrariRpm {
    fn start(&mut self) {
        *self = FerrariRpm::new();
    }
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, delta: f32) {
        let throttle = (((time * 2.0).sin() + 1.0) * 0.5).powf(1.2);
        self.rpm += (0.2 + throttle * 0.5 - self.rpm) * 0.05;
        self.rpm = self.rpm.clamp(0.2, 1.1);
        if self.rpm >= 1.0 {
            self.rpm -= 0.1;
            self.shift_flash = 2;
            self.backfire = rand::random_range(1..=3);
        }
        self.pos += self.rpm * 1.2 * self.direction * delta * 50.0;
        if self.pos > N as f32 + 6.0 {
            (self.direction, self.backfire) = (-1.0, 2);
        } else if self.pos < -6.0 {
            (self.direction, self.backfire) = (1.0, 2);
        }
        c.fill(Color::BLACK);
        for z in 0..N {
            let dist = (z as f32 - self.pos).abs();
            if dist < 6.0 {
                let heat = (1.0 - dist / 6.0).max(0.0).powf(1.5);
                c.set_zone(z, Color::from_hsv(5.0 + heat * 35.0, 1.0, (0.2 + heat * (0.7 + self.rpm * 0.4)).min(1.0)));
            }
        }
        if self.shift_flash > 0 {
            let center = self.pos.round() as i32;
            for dz in -1..=1 {
                let z = center + dz;
                if (0..N as i32).contains(&z) {
                    c.set_zone(z as usize, Color::WHITE);
                }
            }
            self.shift_flash -= 1;
        }
        if self.backfire > 0 {
            let tail = (self.pos - 2.0 * self.direction).round() as i32;
            if (0..N as i32).contains(&tail) {
                let color = match rand::random_range(0..3) {
                    0 => Color::new(255, 140, 50),
                    1 => Color::new(255, 180, 80),
                    _ => Color::WHITE,
                };
                c.set_zone(tail as usize, color);
            }
            self.backfire -= 1;
        }
        c.flush_buffered();
    }
}

/// Left third CPU, middle memory, right GPU usage
pub struct ThermalStatus {
    monitor: SystemMonitor,
    values: (f32, f32, f32),
    next: f32,
}
impl ThermalStatus {
    pub fn new() -> Self {
        ThermalStatus { monitor: SystemMonitor::new(), values: (0.0, 0.0, 0.0), next: 0.0 }
    }
}
fn usage_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    let (blue, green, yellow, red) = (Color::from_hsv(220.0, 1.0, 1.0), Color::from_hsv(120.0, 1.0, 1.0), Color::from_hsv(60.0, 1.0, 1.0), Color::from_hsv(0.0, 1.0, 1.0));
    if v < 0.33 {
        blue.lerp(&green, v / 0.33)
    } else if v < 0.66 {
        green.lerp(&yellow, (v - 0.33) / 0.33)
    } else {
        yellow.lerp(&red, (v - 0.66) / 0.34)
    }
}
impl Effect for ThermalStatus {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        if time >= self.next {
            self.values = self.monitor.read();
            self.next = time + 0.5;
        }
        let (cpu, mem, gpu) = self.values;
        for (start, v) in [(0, cpu), (8, mem), (16, gpu)] {
            let color = usage_color(v);
            for i in 0..8 {
                c.set_zone(start + i, color);
            }
        }
        c.flush_buffered();
    }
}

pub struct AudioSparkle {
    pub sensitivity: f32,
    pub base_density: f32,
    pub rainbow_speed: Option<f32>,
    _audio: AudioUser,
}
impl AudioSparkle {
    pub fn new(sensitivity: f32, base_density: f32, rainbow_speed: Option<f32>) -> Self {
        AudioSparkle { sensitivity, base_density, rainbow_speed, _audio: AudioUser::new() }
    }
}
impl Effect for AudioSparkle {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        c.fill(Color::BLACK);
        let intensity = audio_intensity();
        let density = (self.base_density + intensity * self.sensitivity).clamp(0.0, 1.0);
        let brightness = (intensity * self.sensitivity * 5.0).clamp(0.0, 1.0);
        for i in 0..N {
            if hash(i, time) > 1.0 - density {
                let color = match self.rainbow_speed {
                    Some(s) => Color::from_hsv(((i as f32 / N as f32) * 360.0 + time * s * 180.0) % 360.0, 1.0, 1.0),
                    None => Color::WHITE,
                };
                c.set_zone(i, color.scale(brightness));
            }
        }
        c.flush_buffered();
    }
}

pub struct AudioSparkleMedia {
    sensitivity: f32,
    base_density: f32,
    screen: ScreenSampler,
    _audio: AudioUser,
}
impl AudioSparkleMedia {
    pub fn new(sensitivity: f32, base_density: f32) -> Self {
        AudioSparkleMedia { sensitivity, base_density, screen: ScreenSampler::new(0.0, 1.0), _audio: AudioUser::new() }
    }
}
impl Effect for AudioSparkleMedia {
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        c.fill(Color::BLACK);
        let intensity = audio_intensity();
        let density = (self.base_density + intensity * self.sensitivity).clamp(0.0, 1.0);
        let brightness = (intensity * self.sensitivity * 5.0).clamp(0.0, 1.0);
        let pixels = self.screen.sample();
        for i in 0..N {
            if hash(i, time) <= 1.0 - density {
                continue;
            }
            let mut best = RgbF::BLACK;
            let mut max_luma = -1.0;
            for y in 0..AMBIENT_HEIGHT {
                let p = pixels[i][y];
                if p.luminance() > max_luma {
                    max_luma = p.luminance();
                    best = p;
                }
            }
            c.set_zone(i, best.saturate(1.5).powf(0.8).to_color().scale(brightness));
        }
        c.flush_buffered();
    }
}

struct Ripple {
    center: f32,
    start: f32,
    hue: f32,
}

/// Ripples from a zone: rainbow_ripple.rs (key presses) and audio_ripple.rs (beats) from the original controller,
/// same math: cosine rings in a Gaussian envelope, quadratic decay, hue from the clock, additive blending
pub struct RippleEffect {
    ripples: Vec<Ripple>,
    speed: f32,
    width: f32,
    lifetime: f32,
    saturation: f32,
    audio_sensitivity: Option<f32>,
    last_intensity: f32,
    last_ripple: f32,
    _audio: Option<AudioUser>,
    _keys: Option<KeyHookUser>,
}
impl RippleEffect {
    pub fn audio(sensitivity: f32, speed: f32, width: f32, lifetime: f32) -> Self {
        RippleEffect { ripples: Vec::new(), speed, width, lifetime, saturation: 1.0, audio_sensitivity: Some(sensitivity), last_intensity: 0.0, last_ripple: -10.0, _audio: Some(AudioUser::new()), _keys: None }
    }
    pub fn typing(speed: f32, width: f32, lifetime: f32) -> Self {
        RippleEffect { ripples: Vec::new(), speed, width, lifetime, saturation: 0.9, audio_sensitivity: None, last_intensity: 0.0, last_ripple: -10.0, _audio: None, _keys: Some(KeyHookUser::new()) }
    }
}
impl Effect for RippleEffect {
    fn start(&mut self) {
        self.ripples.clear();
    }
    fn update(&mut self, c: &mut ZoneKeyboard, time: f32, _: f32) {
        if let Some(sens) = self.audio_sensitivity {
            let intensity = audio_intensity() * sens;
            if intensity > 0.1 && intensity - self.last_intensity > 0.05 && time - self.last_ripple > 0.3 {
                self.ripples.push(Ripple { center: N as f32 / 2.0, start: time, hue: (time * 60.0) % 360.0 });
                self.last_ripple = time;
            }
            self.last_intensity = intensity;
        }
        if let Some(keys) = &self._keys {
            for zone in keys.drain() {
                self.ripples.push(Ripple { center: zone as f32, start: time, hue: (time * 60.0) % 360.0 });
            }
        }
        let lifetime = self.lifetime;
        self.ripples.retain(|r| time - r.start < lifetime);
        c.fill(Color::BLACK);
        for i in 0..N {
            let mut final_color = Color::BLACK;
            let mut total = 0.0;
            for r in &self.ripples {
                let age = time - r.start;
                let dist = (i as f32 - r.center).abs();
                // Water drop: cosine rings around the wavefront, Gaussian envelope, decay over the lifetime
                let diff = dist - age * self.speed;
                let wave = (diff * 1.2).cos();
                let envelope = (-(diff * diff) / (self.width * self.width)).exp();
                let decay = (1.0 - age / self.lifetime).powi(2);
                let intensity = (wave * 0.5 + 0.5) * envelope * decay;
                if intensity > 0.01 {
                    let hue = (r.hue + dist * 10.0 + age * 20.0) % 360.0;
                    final_color = final_color.saturating_add(&Color::from_hsv(hue, self.saturation, 1.0).scale(intensity));
                    total += intensity;
                }
            }
            if total > 0.0 {
                c.set_zone(i, final_color);
            }
        }
        c.flush_buffered();
    }
    fn stop(&mut self, c: &mut ZoneKeyboard) {
        self.ripples.clear();
        let _ = c.clear();
    }
}

pub struct Ambient {
    smoothing: f32,
    screen: ScreenSampler,
    last: [RgbF; N],
}
impl Ambient {
    pub fn new(smoothing: f32, left: f32, width: f32) -> Self {
        Ambient { smoothing, screen: ScreenSampler::new(left, width), last: [RgbF::BLACK; N] }
    }
}
impl Effect for Ambient {
    fn update(&mut self, c: &mut ZoneKeyboard, _: f32, delta: f32) {
        let pixels = self.screen.sample();
        for x in 0..N {
            let mut sum = RgbF::BLACK;
            for y in 0..AMBIENT_HEIGHT {
                sum = sum.add(pixels[x][y]);
            }
            let target = sum.scale(1.0 / AMBIENT_HEIGHT as f32).saturate(2.8);
            self.last[x] = if self.smoothing > 0.01 { self.last[x].lerp(target, 1.0 - (-self.smoothing * delta * 4.0).exp()) } else { target };
            c.set_zone(x, self.last[x].to_color());
        }
        c.flush_buffered();
    }
}
