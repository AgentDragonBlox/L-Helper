//! Shared app state, the hardware worker, the background loop and actions (Linux version of ../src/app.rs).
//! The UI only reads `state()` and queues jobs; hardware calls happen on worker threads.

use once_cell::sync::{Lazy, OnceCell};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::config;
use crate::hw::battery::{self, BatteryMode, Switch};
use crate::hw::display::{self, Screen};
use crate::hw::profile::{self, Mode};
use crate::hw::{firmware, sensors};
use crate::lighting;
use crate::notify;

pub const COLOR_QUIET: (u8, u8, u8) = (6, 180, 138);
pub const COLOR_BALANCED: (u8, u8, u8) = (58, 174, 239);
pub const COLOR_PERFORMANCE: (u8, u8, u8) = (255, 32, 32);
pub const COLOR_EXTREME: (u8, u8, u8) = (190, 70, 255);
pub const COLOR_CUSTOM: (u8, u8, u8) = (255, 128, 0);

pub fn mode_color(m: Mode) -> (u8, u8, u8) {
    match m {
        Mode::Quiet => COLOR_QUIET,
        Mode::Balanced => COLOR_BALANCED,
        Mode::Performance => COLOR_PERFORMANCE,
        Mode::Extreme => COLOR_EXTREME,
        Mode::Custom => COLOR_CUSTOM,
    }
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub mode: Option<Mode>,
    pub modes: Vec<Mode>,
    pub battery: Option<BatteryMode>,
    pub battery_modes: Vec<BatteryMode>,
    pub sensors: sensors::Reading,
    pub screen: Option<Screen>,
    pub fn_lock: Option<bool>,
    pub usb_charging: Option<bool>,
    /// White backlight (level, max)
    pub keyboard_light: Option<(i32, i32)>,
    pub power_limits: bool,
    pub autostart: bool,
    pub status: String,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));
static CTX: Mutex<Option<egui::Context>> = Mutex::new(None);
static JOBS: OnceCell<Mutex<Sender<Box<dyn FnOnce() + Send>>>> = OnceCell::new();
static OWN_MODE_CHANGE: AtomicI64 = AtomicI64::new(-10_000);
static START: Lazy<Instant> = Lazy::new(Instant::now);

pub fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap()
}

/// The open window's context (None while only the tray runs)
pub fn set_ctx(ctx: Option<egui::Context>) {
    *CTX.lock().unwrap() = ctx;
}

pub fn ctx() -> Option<egui::Context> {
    CTX.lock().unwrap().clone()
}

pub fn repaint() {
    if let Some(c) = ctx() {
        c.request_repaint();
    }
}

fn mark_own_mode_change() {
    OWN_MODE_CHANGE.store(START.elapsed().as_millis() as i64, Ordering::SeqCst);
}

fn own_change_recent() -> bool {
    START.elapsed().as_millis() as i64 - OWN_MODE_CHANGE.load(Ordering::SeqCst) < 3000
}

/// Runs hardware work on the (single, ordered) worker thread
pub fn job(f: impl FnOnce() + Send + 'static) {
    let sender = JOBS.get_or_init(|| {
        let (tx, rx) = channel::<Box<dyn FnOnce() + Send>>();
        std::thread::Builder::new()
            .name("worker".into())
            .spawn(move || {
                for f in rx {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
                        crate::log!("Worker job panicked");
                    }
                    repaint();
                }
            })
            .ok();
        Mutex::new(tx)
    });
    let _ = sender.lock().unwrap().send(Box::new(f));
}

pub fn report(result: Result<(), String>) {
    match result {
        Ok(()) => state().status.clear(),
        Err(e) => {
            crate::log!("Action failed: {e}");
            state().status = e.clone();
            notify::show("L-Helper", &e);
        }
    }
}

// ---------------- state refresh ----------------

pub fn refresh_state() {
    let s = State {
        mode: profile::get(),
        modes: profile::available(),
        battery: battery::get(),
        battery_modes: battery::modes(),
        sensors: state().sensors.clone(),
        screen: display::laptop_screen(),
        fn_lock: battery::switch(Switch::FnLock),
        usb_charging: battery::switch(Switch::UsbCharging),
        keyboard_light: if lighting::is_supported() { None } else { battery::keyboard_light() },
        power_limits: firmware::supported(),
        autostart: crate::autostart::enabled(),
        status: state().status.clone(),
    };
    *state() = s;
    crate::tray::refresh();
    repaint();
}

pub fn refresh_sensors() {
    let r = sensors::read();
    state().sensors = r;
    crate::tray::refresh();
    repaint();
}

// ---------------- actions (UI / tray / second instance) ----------------

fn remember_mode(m: Mode) {
    config::set("performance_mode", m as i64);
    config::set(if sensors::plugged_in() { "performance_mode_ac" } else { "performance_mode_dc" }, m as i64);
}

fn switch_mode(m: Mode) -> Result<(), String> {
    mark_own_mode_change();
    profile::set(m)?;
    mark_own_mode_change();
    if m == Mode::Custom {
        firmware::apply_saved()?;
    }
    Ok(())
}

pub fn set_mode(m: Mode) {
    job(move || {
        let result = switch_mode(m);
        if result.is_ok() {
            remember_mode(m);
        }
        report(result);
        refresh_state();
    });
}

/// Saves the power limits and applies them in Custom mode
pub fn apply_power_limits(values: Vec<(firmware::Attribute, i64)>) {
    job(move || {
        for (a, v) in &values {
            config::set(&a.config_key(), *v);
        }
        let result = (|| {
            if profile::get() != Some(Mode::Custom) && profile::available().contains(&Mode::Custom) {
                mark_own_mode_change();
                profile::set(Mode::Custom)?;
                mark_own_mode_change();
                remember_mode(Mode::Custom);
            }
            firmware::apply_saved()
        })();
        report(result);
        refresh_state();
    });
}

pub fn set_battery(b: BatteryMode) {
    job(move || {
        report(battery::set(b));
        refresh_state();
    });
}

pub fn set_switch(s: Switch, on: bool) {
    job(move || {
        report(battery::set_switch(s, on));
        refresh_state();
    });
}

pub fn set_keyboard_light(level: i32) {
    job(move || {
        report(battery::set_keyboard_light(level));
        refresh_state();
    });
}

pub fn set_refresh_rate(hz: u32) {
    config::set("screen_auto", 0);
    job(move || {
        report(display::set_refresh_rate(hz));
        refresh_state();
    });
}

pub fn set_resolution(width: u32, height: u32) {
    job(move || {
        report(display::set_resolution(width, height));
        refresh_state();
    });
}

pub fn set_screen_auto() {
    config::set("screen_auto", 1);
    job(|| {
        auto_screen();
        refresh_state();
    });
}

/// Lowest refresh rate on battery, highest when plugged in
fn auto_screen() {
    if !config::is("screen_auto") {
        return;
    }
    let Some(screen) = display::laptop_screen() else { return };
    let target = if sensors::plugged_in() { screen.rates.last().copied() } else { display::low_rate(&screen.rates) };
    if let Some(hz) = target.filter(|hz| screen.rate != Some(*hz)) {
        report(display::set_refresh_rate(hz));
    }
}

pub fn cycle_lighting() {
    if !lighting::is_supported() {
        return;
    }
    job(|| {
        if !lighting::is_running() {
            lighting::start();
        }
        if let Some(p) = lighting::cycle() {
            notify::osd(&format!("Lighting: {}", p.name));
        }
        repaint();
    });
}

/// Per power source mode (the last one chosen on battery / plugged in), when "auto_mode" is on
fn auto_mode() {
    if !config::is_not_false("auto_mode") {
        return;
    }
    let key = if sensors::plugged_in() { "performance_mode_ac" } else { "performance_mode_dc" };
    let Some(m) = config::exists(key).then(|| Mode::from_index(config::get(key, 1))).flatten() else { return };
    if profile::get() != Some(m) && profile::available().contains(&m) {
        report(switch_mode(m));
    }
}

// ---------------- background ----------------

/// Initial state, then sensors, Fn+Q (platform_profile changed by firmware), power source and resume
pub fn start_background() {
    job(|| {
        if profile::get() == Some(Mode::Custom) {
            report(firmware::apply_saved());
        }
        auto_mode();
        auto_screen();
        refresh_state();
        refresh_sensors();
    });

    std::thread::Builder::new()
        .name("background".into())
        .spawn(|| {
            let mut plugged = sensors::plugged_in();
            let mut last_profile = profile::raw();
            let mut last = Instant::now();
            let mut ticks = 0u32;
            loop {
                let visible = ctx().is_some();
                std::thread::sleep(Duration::from_secs(1));
                let gap = last.elapsed();
                last = Instant::now();
                ticks += 1;

                if gap > Duration::from_secs(30) {
                    crate::log!("Resume detected");
                    job(|| {
                        std::thread::sleep(Duration::from_secs(3));
                        lighting::on_resume();
                        if profile::get() == Some(Mode::Custom) {
                            report(firmware::apply_saved());
                        }
                        refresh_state();
                    });
                }

                // Fn+Q, GNOME's power mode menu or powerprofilesctl
                let now_profile = profile::raw();
                if now_profile != last_profile {
                    last_profile = now_profile.clone();
                    if let Some(m) = now_profile.as_deref().and_then(profile::from_raw) {
                        if !own_change_recent() {
                            crate::log!("Mode changed outside L-Helper: {m:?}");
                            job(move || {
                                if m == Mode::Custom {
                                    report(firmware::apply_saved());
                                }
                                remember_mode(m);
                                notify::osd(&format!("Mode: {}", m.name()));
                                refresh_state();
                            });
                        } else {
                            job(refresh_state);
                        }
                    }
                }

                let now_plugged = sensors::plugged_in();
                if now_plugged != plugged {
                    plugged = now_plugged;
                    crate::log!("Power source: {}", if plugged { "AC" } else { "Battery" });
                    job(move || {
                        auto_mode();
                        auto_screen();
                        notify::osd(if now_plugged { "Plugged in" } else { "On battery" });
                        refresh_state();
                    });
                }

                // Sensors every second while the window is open, every 5 s for the tray tooltip
                if visible || ticks % 5 == 0 {
                    refresh_sensors();
                }
                // Fn+Space and other firmware-side changes
                if visible && ticks % 3 == 0 {
                    job(refresh_state);
                }
            }
        })
        .ok();
}

pub fn quit() -> ! {
    lighting::stop(false);
    config::flush();
    crate::log!("Exit");
    std::process::exit(0)
}
