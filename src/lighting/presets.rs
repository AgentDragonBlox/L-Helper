// Preset metadata and factory, adapted from LOQ-24-zone-RGB-controller (presets/mod.rs, lib.rs apply_preset)
// Copyright (c) 2026 Dhruv Goel, MIT License. See THIRD_PARTY_NOTICES.md

use std::collections::BTreeMap;

use super::effects::*;
use super::zone::Color;

#[derive(Debug, Clone)]
pub struct Param {
    pub name: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
    pub color: Option<Color>,
}

#[derive(Debug, Clone)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    #[allow(dead_code)]
    pub group: &'static str,
    pub params: Vec<Param>,
}

const fn float(name: &'static str, label: &'static str, min: f32, max: f32, default: f32, step: f32) -> Param {
    Param { name, label, min, max, default, step, color: None }
}
const fn color(name: &'static str, label: &'static str, r: u8, g: u8, b: u8) -> Param {
    Param { name, label, min: 0.0, max: 255.0, default: 0.0, step: 1.0, color: Some(Color::new(r, g, b)) }
}
const fn speed(min: f32, max: f32, default: f32) -> Param {
    float("speed", "Speed", min, max, default, 0.1)
}
const SENSITIVITY: Param = float("sensitivity", "Sensitivity", 0.1, 10.0, 1.0, 0.1);
const BASE_DENSITY: Param = float("base_density", "Base density", 0.0, 0.5, 0.0, 0.01);

fn p(id: &'static str, name: &'static str, description: &'static str, group: &'static str, params: Vec<Param>) -> Preset {
    Preset { id, name, description, group, params }
}

pub fn all() -> Vec<Preset> {
    vec![
        p("staticColor", "Static Color", "One solid color", "Static", vec![color("color", "Color", 255, 255, 200)]),
        p("breathing", "Color Breath", "Fade in → fade out", "Static", vec![speed(0.1, 5.0, 1.0), color("color", "Color", 255, 0, 0)]),
        p("pulse", "Pulse Center", "Pulsing effect from the center", "Static", vec![speed(0.1, 5.0, 1.0), color("color", "Color", 255, 0, 0)]),
        p("aurora", "Aurora", "Flowing aurora", "Static", vec![speed(0.1, 3.0, 0.5)]),
        p("nebula", "Nebula", "Soft, atmospheric, no harsh transitions", "Static", vec![speed(0.1, 10.0, 1.0)]),
        p("chromaticBreath", "Chromatic Breath", "Clean breathing through 20 colors, good for idle", "Static", vec![speed(0.1, 10.0, 1.0)]),
        p("sparkle", "Sparkle", "Random white sparkles", "Static", vec![float("density", "Density", 0.01, 0.5, 0.1, 0.01)]),
        p("rainbowBreath", "Rainbow Breath", "Whole keyboard breathes through rainbow hues", "Dynamic", vec![speed(0.1, 10.0, 1.0)]),
        p("rainbowCycle", "Rainbow Cycle", "Whole keyboard cycles through hues", "Dynamic", vec![speed(0.1, 10.0, 1.0)]),
        p("rainbowWave", "Rainbow Wave", "Left → right rainbow", "Dynamic", vec![speed(0.1, 10.0, 1.0)]),
        p("wheel", "Color Wheel", "Each zone has a fixed hue offset, the keyboard spins like a wheel", "Dynamic", vec![speed(0.1, 3.0, 0.5)]),
        p("sweep", "Color Sweep", "All 16.7 million colors, one per frame", "Dynamic", vec![]),
        p("scan", "Color Scan", "Compress, lock, implode, explode", "Dynamic", vec![speed(0.1, 3.0, 1.0)]),
        p("horse", "Horse Color", "A sharp chaser racing across a solid base color", "Dynamic", vec![
            speed(0.1, 10.0, 5.0),
            float("length", "Length", 0.1, 10.0, 5.0, 0.1),
            color("base_color", "Base color", 81, 169, 158),
            color("horse_color", "Horse color", 255, 0, 0),
        ]),
        p("horseCycle", "Horse Cycle", "A racing chaser over a color-cycling base", "Dynamic", vec![speed(0.1, 10.0, 5.0), float("length", "Length", 0.1, 10.0, 5.0, 0.1)]),
        p("rpm", "Ferrari RPM", "Fast, aggressive, red-dominant motion", "Dynamic", vec![]),
        p("ambient", "Screen Ambiance", "Mimics ambient light from screen content", "Reactive", vec![
            float("smoothing", "Smoothing", 0.0, 10.0, 5.0, 0.1),
            float("sample_left", "Screen sample left", 0.0, 1.0, 0.0, 0.01),
            float("sample_width", "Screen sample width", 0.01, 1.0, 1.0, 0.01),
        ]),
        p("audio_sparkle", "Audio Sparkle", "Sparkles in sync with system audio", "Reactive", vec![SENSITIVITY, BASE_DENSITY]),
        p("audio_sparkle_rainbow", "Audio Sparkle Rainbow", "Rainbow sparkles that react to audio", "Reactive", vec![SENSITIVITY, BASE_DENSITY, float("rainbow_speed", "Rainbow speed", 0.1, 5.0, 1.0, 0.1)]),
        p("audio_sparkle_media", "Audio Sparkle Media", "Sparkles matching screen colors, reacting to audio", "Reactive", vec![SENSITIVITY, BASE_DENSITY]),
        p("audio_ripple", "Audio Ripple", "Ripples flash from the center on beats", "Reactive", vec![
            SENSITIVITY,
            float("speed", "Speed", 10.0, 100.0, 40.0, 1.0),
            float("width", "Width", 1.0, 10.0, 3.0, 0.1),
            float("lifetime", "Lifetime", 0.1, 2.0, 0.8, 0.1),
        ]),
        p("rainbow_ripple", "Typing Rainbow Ripple", "Rainbow waves from the keys you press", "Reactive", vec![
            float("speed", "Speed", 10.0, 100.0, 40.0, 1.0),
            float("width", "Width", 1.0, 10.0, 3.0, 0.1),
            float("lifetime", "Lifetime", 0.1, 2.0, 0.8, 0.1),
        ]),
        p("thermalStatus", "CPU / Memory / GPU usage", "Left = CPU, middle = memory, right = GPU", "System", vec![]),
        p("off", "Off", "All lighting off", "System", vec![]),
    ]
}

pub fn find(id: &str) -> Option<Preset> {
    all().into_iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// Parameter values for one preset. Serialized like the C# version: "speed=1.5;color=#FF0000"
#[derive(Debug, Clone, Default)]
pub struct Values {
    pub floats: BTreeMap<String, f32>,
    pub colors: BTreeMap<String, Color>,
}

impl Values {
    pub fn f(&self, p: &Preset, name: &str) -> f32 {
        self.floats.get(name).copied().or_else(|| p.params.iter().find(|x| x.name == name).map(|x| x.default)).unwrap_or(1.0)
    }
    pub fn c(&self, p: &Preset, name: &str) -> Color {
        self.colors.get(name).copied().or_else(|| p.params.iter().find(|x| x.name == name).and_then(|x| x.color)).unwrap_or(Color::WHITE)
    }
    pub fn serialize(&self) -> String {
        self.floats.iter().map(|(k, v)| format!("{k}={v}")).chain(self.colors.iter().map(|(k, c)| format!("{k}={}", c.hex()))).collect::<Vec<_>>().join(";")
    }
    pub fn parse(text: &str) -> Values {
        let mut v = Values::default();
        for part in text.split(';') {
            let Some((k, val)) = part.split_once('=') else { continue };
            if val.starts_with('#') {
                if let Some(c) = Color::parse_hex(val) {
                    v.colors.insert(k.to_string(), c);
                }
            } else if let Ok(f) = val.parse::<f32>() {
                v.floats.insert(k.to_string(), f);
            }
        }
        v
    }
}

pub fn create(p: &Preset, v: &Values) -> Box<dyn Effect> {
    match p.id {
        "staticColor" => Box::new(Static(v.c(p, "color"))),
        "breathing" => Box::new(ColorBreath { color: v.c(p, "color"), speed: v.f(p, "speed") }),
        "pulse" => Box::new(PulseCenter { color: v.c(p, "color"), speed: v.f(p, "speed") }),
        "aurora" => Box::new(Aurora(v.f(p, "speed"))),
        "nebula" => Box::new(Nebula(v.f(p, "speed"))),
        "chromaticBreath" => Box::new(ChromaticBreath(v.f(p, "speed"))),
        "sparkle" => Box::new(Sparkle(v.f(p, "density"))),
        "rainbowBreath" => Box::new(RainbowBreath(v.f(p, "speed"))),
        "rainbowCycle" => Box::new(RainbowCycle(v.f(p, "speed"))),
        "rainbowWave" => Box::new(RainbowWave(v.f(p, "speed"))),
        "wheel" => Box::new(ColorWheel(v.f(p, "speed"))),
        "sweep" => Box::new(RgbSweep::new()),
        "scan" => Box::new(ColorScan::new(v.f(p, "speed"))),
        "horse" => Box::new(Horse { position: 0.0, speed: v.f(p, "speed"), length: v.f(p, "length"), base: v.c(p, "base_color"), horse: v.c(p, "horse_color") }),
        "horseCycle" => Box::new(HorseCycle::new(v.f(p, "speed"), v.f(p, "length"))),
        "rpm" => Box::new(FerrariRpm::new()),
        "ambient" => Box::new(Ambient::new(v.f(p, "smoothing"), v.f(p, "sample_left"), v.f(p, "sample_width"))),
        "audio_sparkle" => Box::new(AudioSparkle::new(v.f(p, "sensitivity"), v.f(p, "base_density"), None)),
        "audio_sparkle_rainbow" => Box::new(AudioSparkle::new(v.f(p, "sensitivity"), v.f(p, "base_density"), Some(v.f(p, "rainbow_speed")))),
        "audio_sparkle_media" => Box::new(AudioSparkleMedia::new(v.f(p, "sensitivity"), v.f(p, "base_density"))),
        "audio_ripple" => Box::new(RippleEffect::audio(v.f(p, "sensitivity"), v.f(p, "speed"), v.f(p, "width"), v.f(p, "lifetime"))),
        "rainbow_ripple" => Box::new(RippleEffect::typing(v.f(p, "speed"), v.f(p, "width"), v.f(p, "lifetime"))),
        "thermalStatus" => Box::new(ThermalStatus::new()),
        _ => Box::new(Off),
    }
}
