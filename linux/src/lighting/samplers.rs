//! Inputs for the shared effects (../src/lighting/effects.rs) on Linux. The reactive presets (screen, audio,
//! typing) are hidden here (presets::all), so their inputs stay idle; CPU / memory / GPU usage is real.

use super::zone::{Color, NUM_ZONES};

pub const AMBIENT_HEIGHT: usize = 12;

#[derive(Debug, Clone, Copy, Default)]
pub struct RgbF {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl RgbF {
    pub const BLACK: RgbF = RgbF { r: 0.0, g: 0.0, b: 0.0 };
    pub fn luminance(&self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
    pub fn add(self, o: RgbF) -> RgbF {
        RgbF { r: self.r + o.r, g: self.g + o.g, b: self.b + o.b }
    }
    pub fn scale(self, s: f32) -> RgbF {
        RgbF { r: self.r * s, g: self.g * s, b: self.b * s }
    }
    pub fn lerp(self, t: RgbF, a: f32) -> RgbF {
        RgbF { r: self.r + (t.r - self.r) * a, g: self.g + (t.g - self.g) * a, b: self.b + (t.b - self.b) * a }
    }
    pub fn saturate(self, amount: f32) -> RgbF {
        let l = self.luminance();
        let f = |c: f32| (l + (c - l) * amount).clamp(0.0, 1.0);
        RgbF { r: f(self.r), g: f(self.g), b: f(self.b) }
    }
    pub fn powf(self, p: f32) -> RgbF {
        RgbF { r: self.r.powf(p), g: self.g.powf(p), b: self.b.powf(p) }
    }
    pub fn to_color(self) -> Color {
        let f = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
        Color::new(f(self.r), f(self.g), f(self.b))
    }
}

pub fn audio_intensity() -> f32 {
    0.0
}

pub struct AudioUser;

impl AudioUser {
    pub fn new() -> Self {
        AudioUser
    }
}

pub struct ScreenSampler;

impl ScreenSampler {
    pub fn new(_left: f32, _width: f32) -> Self {
        ScreenSampler
    }

    pub fn sample(&mut self) -> [[RgbF; AMBIENT_HEIGHT]; NUM_ZONES] {
        [[RgbF::BLACK; AMBIENT_HEIGHT]; NUM_ZONES]
    }
}

pub struct KeyHookUser;

impl KeyHookUser {
    pub fn new() -> Self {
        KeyHookUser
    }

    pub fn drain(&self) -> Vec<usize> {
        Vec::new()
    }
}

// ---------------- CPU / memory / NVIDIA GPU utilisation ----------------

pub struct SystemMonitor {
    last_idle: u64,
    last_total: u64,
}

impl SystemMonitor {
    pub fn new() -> Self {
        SystemMonitor { last_idle: 0, last_total: 0 }
    }

    pub fn read(&mut self) -> (f32, f32, f32) {
        let mut cpu = 0.0;
        // /proc/stat "cpu  user nice system idle iowait irq softirq steal ..."
        if let Some(line) = std::fs::read_to_string("/proc/stat").ok().and_then(|s| s.lines().next().map(str::to_string)) {
            let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 5 {
                let idle = v[3] + v[4];
                let total: u64 = v.iter().take(8).sum();
                let (d_total, d_idle) = (total.saturating_sub(self.last_total), idle.saturating_sub(self.last_idle));
                if self.last_total != 0 && d_total > 0 {
                    cpu = (1.0 - d_idle as f32 / d_total as f32).clamp(0.0, 1.0);
                }
                self.last_total = total;
                self.last_idle = idle;
            }
        }
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let field = |name: &str| meminfo.lines().find_map(|l| l.strip_prefix(name)).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<f32>().ok());
        let memory = match (field("MemTotal:"), field("MemAvailable:")) {
            (Some(total), Some(available)) if total > 0.0 => (1.0 - available / total).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let gpu = crate::hw::sensors::nvidia().and_then(|g| g.usage).unwrap_or(0.0);
        (cpu, memory, gpu)
    }
}
