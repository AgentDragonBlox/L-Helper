// Effect runner, adapted from LOQ-24-zone-RGB-controller (lib.rs rgb_init / effect loop)
// Copyright (c) 2026 Dhruv Goel, MIT License. See THIRD_PARTY_NOTICES.md
//
// Runs the selected preset at 25 FPS on a background thread and owns the 24-zone keyboard.
// Settings (same keys as the C# version): rgb_preset, rgb_brightness, rgb_params_<id>, rgb_cycle, rgb_software

pub mod effects;
pub mod lamparray;
pub mod presets;
pub mod samplers;
pub mod zone;

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config;
use crate::lenovo::gamezone;
use effects::Effect;
use presets::{Preset, Values};
use zone::{Color, ZoneKeyboard, NUM_ZONES};

/// Pause after each frame, as in the original controller (~22 FPS with the HID writes). Driving the keyboard
/// faster (30 FPS) made the controller flicker on a LOQ 15AHP10.
const FRAME: Duration = Duration::from_millis(40);

struct Engine {
    keyboard: ZoneKeyboard,
    effect: Option<Box<dyn Effect>>,
    current: Preset,
}

static ENGINE: Lazy<Mutex<Engine>> = Lazy::new(|| Mutex::new(Engine { keyboard: ZoneKeyboard::new(), effect: None, current: presets::all().remove(0) }));
static RUNNING: AtomicBool = AtomicBool::new(false);
static SUPPORTED: AtomicBool = AtomicBool::new(false);

pub fn detect() {
    let present = zone::is_present();
    SUPPORTED.store(present, Ordering::SeqCst);
    crate::log!("24-zone lighting: {present}");
}

pub fn is_supported() -> bool {
    SUPPORTED.load(Ordering::SeqCst)
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// LHelper's effects need Windows Dynamic Lighting on (see lamparray.rs)
pub fn dynamic_lighting_enabled() -> bool {
    lamparray::dynamic_lighting_enabled()
}

pub fn init() {
    detect();
    if is_supported() && config::get("rgb_software", 1) != 0 {
        start();
    }
}

/// Take lighting control from firmware and run the saved preset
pub fn start() {
    if !is_supported() || is_running() {
        return;
    }
    if let Err(e) = gamezone::set_light_control_owner(true) {
        crate::log!("SetLightControlOwner failed: {e}");
    }
    {
        let mut e = ENGINE.lock().unwrap();
        if !e.keyboard.connect() {
            crate::log!("24-zone keyboard connect failed");
            return;
        }
        e.keyboard.brightness = config::get("rgb_brightness", 100) as f32 / 100.0;
    }
    if !dynamic_lighting_enabled() {
        crate::log!("Windows Dynamic Lighting is off: the keyboard ignores software effects until it's turned on");
    }
    lamparray::start();
    let preset = config::get_string("rgb_preset").and_then(|id| presets::find(&id)).unwrap_or_else(|| presets::all().remove(0));
    apply(&preset, None, false);
    RUNNING.store(true, Ordering::SeqCst);
    config::set("rgb_software", 1);
    std::thread::Builder::new().name("lighting".into()).spawn(run).ok();
}

/// Stop effects and hand lighting back to the keyboard firmware (Fn+Space, onboard effects)
pub fn stop(remember: bool) {
    if !RUNNING.swap(false, Ordering::SeqCst) {
        return;
    }
    std::thread::sleep(FRAME * 2);
    {
        let mut e = ENGINE.lock().unwrap();
        let Engine { keyboard, effect, .. } = &mut *e;
        if let Some(mut fx) = effect.take() {
            fx.stop(keyboard);
        }
        keyboard.disconnect();
    }
    lamparray::stop();
    let _ = gamezone::set_light_control_owner(false);
    crate::log!("Lighting handed back to firmware");
    if remember {
        config::set("rgb_software", 0);
    }
}

pub fn values(preset: &Preset) -> Values {
    config::get_string(&format!("rgb_params_{}", preset.id)).map(|s| Values::parse(&s)).unwrap_or_default()
}

pub fn save_values(preset: &Preset, values: &Values) {
    config::set_string(&format!("rgb_params_{}", preset.id), &values.serialize());
}

pub fn apply(preset: &Preset, values: Option<&Values>, save: bool) {
    let values = values.cloned().unwrap_or_else(|| self::values(preset));
    let mut next = presets::create(preset, &values);
    next.start();
    {
        let mut e = ENGINE.lock().unwrap();
        let Engine { keyboard, effect, current } = &mut *e;
        if let Some(mut old) = effect.take() {
            old.stop(keyboard);
        }
        *effect = Some(next);
        *current = preset.clone();
    }
    if save {
        config::set_string("rgb_preset", preset.id);
    }
    crate::log!("Lighting preset: {} [{}]", preset.id, values.serialize());
}

pub fn current() -> Preset {
    ENGINE.lock().unwrap().current.clone()
}

pub fn frame() -> [Color; NUM_ZONES] {
    ENGINE.lock().unwrap().keyboard.shown
}

pub fn brightness() -> i32 {
    (ENGINE.lock().unwrap().keyboard.brightness * 100.0).round() as i32
}

pub fn set_brightness(percent: i32) {
    let p = percent.clamp(0, 100);
    ENGINE.lock().unwrap().keyboard.brightness = p as f32 / 100.0;
    config::set("rgb_brightness", p as i64);
}

pub fn cycle_list() -> Vec<String> {
    config::get_string("rgb_cycle")
        .unwrap_or_else(|| "staticColor,rainbowWave,aurora,breathing,off".into())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn set_cycle_list(list: &[String]) {
    config::set_string("rgb_cycle", &list.join(","));
}

/// Next preset in the user's cycle list (hotkey / Fn preset keys)
pub fn cycle() -> Option<Preset> {
    let list: Vec<Preset> = cycle_list().iter().filter_map(|id| presets::find(id)).collect();
    if list.is_empty() {
        return None;
    }
    let current = current();
    let index = list.iter().position(|p| p.id == current.id).map(|i| (i + 1) % list.len()).unwrap_or(0);
    apply(&list[index], None, true);
    Some(list[index].clone())
}

/// Resume after sleep: the HID handle is stale and firmware may have taken lighting back
pub fn on_resume() {
    if !is_running() {
        return;
    }
    let _ = gamezone::set_light_control_owner(true);
    lamparray::reopen();
    ENGINE.lock().unwrap().keyboard.reconnect();
}

fn run() {
    let clock = Instant::now();
    let mut last = 0.0f32;
    let mut failures = 0;
    let mut work = Duration::ZERO;
    let mut frames = 0u32;
    while RUNNING.load(Ordering::SeqCst) {
        let started = Instant::now();
        let now = clock.elapsed().as_secs_f32();
        let mut delta = now - last;
        last = now;
        {
            let mut e = ENGINE.lock().unwrap();
            let Engine { keyboard, effect, .. } = &mut *e;
            // A huge gap means we slept: reconnect and use one normal frame of delta
            if delta > 2.0 {
                keyboard.reconnect();
                delta = FRAME.as_secs_f32();
            }
            // While Windows has given the keyboard to another app, don't draw over it
            if let Some(fx) = effect.as_mut().filter(|_| !lamparray::yielded()) {
                fx.update(keyboard, now, delta);
            }
            failures = if keyboard.is_connected() { 0 } else { failures + 1 };
            if failures >= 5 {
                keyboard.reconnect();
                failures = 0;
            }
        }
        let spent = started.elapsed();
        work += spent;
        frames += 1;
        if frames == 300 {
            crate::log!("Lighting: {:.1} ms per frame", work.as_secs_f32() * 1000.0 / frames as f32);
        }
        std::thread::sleep(FRAME);
    }
}
