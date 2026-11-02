//! Shared app state, the hardware worker, background loops and event handling.
//! The UI only reads `state()` and queues jobs; all hardware calls happen on worker threads.

use once_cell::sync::{Lazy, OnceCell};
use std::sync::atomic::{AtomicI64, AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::config;
use crate::gpu::{self, GpuMode};
use crate::lenovo::energy::{self, BatteryMode};
use crate::lenovo::keys::{DriverKey, SpecialKey};
use crate::lenovo::{gamezone, keyboard, keys, machine, sensors, software};
use crate::lighting;
use crate::mode::{self, Mode};
use crate::win::{display, toast, toggles};

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
    pub gpu: Option<GpuMode>,
    pub battery: Option<BatteryMode>,
    pub sensors: sensors::Reading,
    pub screen: Option<String>,
    pub rates: Vec<u32>,
    pub rate: Option<u32>,
    /// Resolutions the panel supports (largest first) and the current one
    pub resolutions: Vec<(u32, u32)>,
    pub resolution: Option<(u32, u32)>,
    /// OLED panel (burn-in protection offered)
    pub oled: bool,
    pub overdrive: Option<bool>,
    pub white_level: Option<i32>,
    pub white_via_driver: bool,
    pub white_max: i32,
    pub spectrum_brightness: Option<i32>,
    pub logo: Option<bool>,
    pub startup: bool,
    pub status: String,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));
static CTX: OnceCell<egui::Context> = OnceCell::new();
static JOBS: OnceCell<Mutex<Sender<Box<dyn FnOnce() + Send>>>> = OnceCell::new();
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static OWN_MODE_CHANGE: AtomicI64 = AtomicI64::new(0);
static START: Lazy<Instant> = Lazy::new(Instant::now);

pub fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap()
}

pub fn set_ctx(ctx: egui::Context) {
    let _ = CTX.set(ctx);
}

pub fn repaint() {
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

pub fn ctx() -> Option<&'static egui::Context> {
    CTX.get()
}

pub fn set_hwnd(hwnd: isize) {
    HWND_RAW.store(hwnd, Ordering::SeqCst);
}

pub fn hwnd() -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(HWND_RAW.load(Ordering::SeqCst) as _)
}

/// Remembers that we changed the mode ourselves, so the firmware echo event is ignored
pub fn mark_own_mode_change() {
    OWN_MODE_CHANGE.store(START.elapsed().as_millis() as i64, Ordering::SeqCst);
}

fn own_change_recent() -> bool {
    START.elapsed().as_millis() as i64 - OWN_MODE_CHANGE.load(Ordering::SeqCst) < 2000
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
    if let Err(e) = result {
        crate::log!("Action failed: {e}");
        state().status = e.clone();
        toast::show(&e, COLOR_PERFORMANCE);
    }
}

// ---------------- state refresh ----------------

pub fn refresh_state() {
    let m = machine::get();
    let mode = mode::get();
    let gpu = if gpu::supported() { gpu::get() } else { None };
    let battery = if m.has_energy_driver { energy::battery_mode() } else { None };
    let overdrive = if m.supports_overdrive { gamezone::overdrive().ok().map(|v| v == 1) } else { None };
    let (white_via_driver, white_max) = (m.keyboard_levels > 0, if m.keyboard_levels > 0 { m.keyboard_levels } else { 2 });
    let white_level = if lighting::is_supported() || keyboard::has_spectrum() {
        None
    } else if white_via_driver {
        energy::keyboard_level()
    } else {
        keyboard::white_level()
    };
    let (spectrum_brightness, logo) = if !lighting::is_supported() && keyboard::has_spectrum() { (keyboard::spectrum_brightness(), keyboard::logo()) } else { (None, None) };
    let screen = display::laptop_screen();
    let rates = screen.as_deref().map(display::refresh_rates).unwrap_or_default();
    let rate = screen.as_deref().and_then(display::refresh_rate);
    let resolutions = screen.as_deref().map(display::resolutions).unwrap_or_default();
    let resolution = screen.as_deref().and_then(display::resolution);
    let oled = screen.is_some() && display::laptop_panel_is_oled();
    let startup = crate::win::startup::is_scheduled();

    let mut s = state();
    s.mode = mode;
    s.gpu = gpu;
    s.battery = battery;
    s.overdrive = overdrive;
    s.white_level = white_level;
    s.white_via_driver = white_via_driver;
    s.white_max = white_max;
    s.spectrum_brightness = spectrum_brightness;
    s.logo = logo;
    s.screen = screen;
    s.rates = rates;
    s.rate = rate;
    s.resolutions = resolutions;
    s.resolution = resolution;
    s.oled = oled;
    s.startup = startup;
    drop(s);
    crate::tray::refresh_icon();
    repaint();
}

pub fn refresh_sensors() {
    let r = sensors::read();
    state().sensors = r.clone();
    crate::tray::update_tooltip(&r, state().mode);
    repaint();
}

// ---------------- actions (called from UI / tray / keys) ----------------

pub fn set_mode(m: Mode) {
    job(move || {
        report(mode::set(m));
        refresh_state();
    });
}

pub fn set_gpu(g: GpuMode) {
    let current = state().gpu;
    if current == Some(g) {
        return;
    }
    let needs_reboot = g == GpuMode::Ultimate || current == Some(GpuMode::Ultimate);
    if needs_reboot && !crate::win::message_box("Switching to or from Ultimate (dGPU only) changes the display MUX and needs a restart.\n\nSwitch now? You can restart later.", true) {
        return;
    }
    job(move || {
        match gpu::set(g) {
            Ok(true) => {
                if crate::win::message_box("Restart now to apply the GPU mode?", true) {
                    let _ = std::process::Command::new(crate::win::system_exe("shutdown.exe")).args(["/r", "/t", "1"]).spawn();
                }
            }
            Ok(false) => {}
            Err(e) => report(Err(e)),
        }
        refresh_state();
    });
}

pub fn set_battery(b: BatteryMode) {
    job(move || {
        report(energy::set_battery_mode(b));
        refresh_state();
    });
}

pub fn set_refresh_rate(hz: u32) {
    config::set("screen_auto", 0);
    job(move || {
        if let Some(s) = display::laptop_screen() {
            display::set_refresh_rate(&s, hz);
        }
        refresh_state();
    });
}

pub fn set_resolution(width: u32, height: u32) {
    job(move || {
        if let Some(s) = display::laptop_screen() {
            display::set_resolution(&s, width, height);
        }
        refresh_state();
    });
}

pub fn set_oled_care(on: bool) {
    job(move || {
        crate::win::oled::set(on);
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

fn auto_screen() {
    if !config::is("screen_auto") {
        return;
    }
    let Some(screen) = display::laptop_screen() else { return };
    let rates = display::refresh_rates(&screen);
    let target = if sensors::plugged_in() { rates.last().copied() } else { display::low_rate(&rates) };
    if let Some(hz) = target {
        display::set_refresh_rate(&screen, hz);
    }
}

pub fn set_white_level(level: i32) {
    let via_driver = state().white_via_driver;
    job(move || {
        report(if via_driver { energy::set_keyboard_level(level) } else { keyboard::set_white_level(level) });
        refresh_state();
    });
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
            toast::show(&format!("Lighting: {}", p.name), COLOR_CUSTOM);
        }
        repaint();
    });
}

fn cycle_refresh_rate() {
    let Some(screen) = display::laptop_screen() else { return };
    let rates = display::refresh_rates(&screen);
    let (Some(low), Some(high)) = (display::low_rate(&rates), rates.last().copied()) else { return };
    let next = if display::refresh_rate(&screen) == Some(high) { low } else { high };
    config::set("screen_auto", 0);
    display::set_refresh_rate(&screen, next);
    toast::show(&format!("Refresh rate {next}Hz"), COLOR_BALANCED);
    refresh_state();
}

// ---------------- Fn keys ----------------

fn on_special_key(key: SpecialKey) {
    match key {
        SpecialKey::FnR => job(cycle_refresh_rate),
        SpecialKey::FnF4 => job(mic_toggle),
        SpecialKey::FnF8 => job(airplane_toggle),
        SpecialKey::FnPrtSc => toggles::open_snipping_tool(),
        SpecialKey::CameraOn => toast::show("Camera on", COLOR_QUIET),
        SpecialKey::CameraOff => toast::show("Camera off", COLOR_PERFORMANCE),
        SpecialKey::FnLockOn => toast::show("Fn lock on", COLOR_BALANCED),
        SpecialKey::FnLockOff => toast::show("Fn lock off", COLOR_BALANCED),
        SpecialKey::SpectrumBacklight(level) => {
            let percent = [0, 33, 66, 100][level.min(3) as usize];
            if lighting::is_running() {
                lighting::set_brightness(percent);
            }
            toast::show(&if percent == 0 { "Keyboard light off".to_string() } else { format!("Keyboard light {percent}%") }, COLOR_CUSTOM);
            repaint();
        }
        SpecialKey::SpectrumPreset(n) => {
            if lighting::is_running() {
                cycle_lighting();
            } else {
                toast::show(&format!("Lighting profile {n}"), COLOR_CUSTOM);
            }
        }
        SpecialKey::WhiteBacklight(level) => toast::show(["Keyboard light off", "Keyboard light low", "Keyboard light high"][level.min(2) as usize], COLOR_CUSTOM),
        SpecialKey::Other(_) => {}
    }
}

fn on_driver_key(key: DriverKey) {
    match key {
        DriverKey::FnQ if machine::get().supports_its => job(|| {
            let m = mode::cycle_its();
            toast::show(&format!("Mode: {}", m.name()), mode_color(m));
            refresh_state();
        }),
        DriverKey::FnF4 => job(mic_toggle),
        DriverKey::FnF8 => job(airplane_toggle),
        DriverKey::FnF10 => toast::show("Touchpad toggled", COLOR_BALANCED),
        DriverKey::FnSpace if state().white_via_driver => job(|| {
            std::thread::sleep(Duration::from_millis(150));
            if let Some(level) = energy::keyboard_level() {
                toast::show(&if level == 0 { "Keyboard light off".to_string() } else { format!("Keyboard light {level}") }, COLOR_CUSTOM);
            }
            refresh_state();
        }),
        _ => {}
    }
}

fn mic_toggle() {
    match toggles::toggle_microphone() {
        Ok(true) => toast::show("Microphone off", COLOR_PERFORMANCE),
        Ok(false) => toast::show("Microphone on", COLOR_QUIET),
        Err(e) => crate::log!("Mic toggle failed: {e}"),
    }
}

fn airplane_toggle() {
    match toggles::toggle_airplane_mode() {
        Ok(on) => toast::show(if on { "Airplane mode on" } else { "Airplane mode off" }, COLOR_BALANCED),
        Err(e) => crate::log!("Airplane toggle failed: {e}"),
    }
}

// ---------------- startup / background ----------------

static LISTENERS: Mutex<Vec<crate::wmi::Listener>> = Mutex::new(Vec::new());

/// Starts event listeners and the background loop (normal runs only; screenshot mode stays read-only)
pub fn start_background() {
    let m = machine::get();
    let mut listeners = LISTENERS.lock().unwrap();

    // Fn+Q on gaming models
    if m.has_gamezone {
        listeners.push(crate::wmi::listen("LENOVO_GAMEZONE_SMART_FAN_MODE_EVENT", |e| {
            let mode = Mode::from_firmware(e.i32("mode").unwrap_or(2));
            if own_change_recent() {
                return;
            }
            crate::log!("Mode changed by firmware (Fn+Q): {mode:?}");
            job(move || {
                if mode == Mode::Custom {
                    mode::apply_custom();
                }
                mode::apply_windows_power_mode(mode);
                config::set("performance_mode", mode as i64);
                config::set(if sensors::plugged_in() { "performance_mode_ac" } else { "performance_mode_dc" }, mode as i64);
                toast::show(&format!("Mode: {}", mode.name()), mode_color(mode));
                refresh_state();
            });
        }));
    }
    listeners.push(keys::listen_special(on_special_key));
    if m.is_ideapad() {
        keys::listen_driver(on_driver_key);
    }
    drop(listeners);

    crate::win::hotkey::start(cycle_lighting);

    // Initial state: re-apply Custom (firmware forgets it on some models), per-power-source mode, auto screen
    job(|| {
        if mode::get() == Some(Mode::Custom) {
            mode::apply_custom();
        }
        if config::is_not_false("auto_mode") {
            mode::auto_performance();
        }
        auto_screen();
        refresh_state();
        refresh_sensors();
        // The NVIDIA driver forgets clock offsets on restart
        crate::nvapi::apply_saved();
    });

    // Lenovo software: keep disabled, and offer it once on first run
    std::thread::spawn(|| {
        software::enforce_configured();
        crate::win::startup::secure_existing_task();
        if config::exists("lenovo_software_asked") {
            return;
        }
        let running = software::running();
        if running.is_empty() {
            return;
        }
        config::set("lenovo_software_asked", 1);
        let names: Vec<&str> = running.iter().map(|d| d.name).collect();
        let text = format!(
            "These Lenovo apps are running and control the same settings as L-Helper:\n\n  • {}\n\nDisable them now? Their services, scheduled tasks and startup entries are turned off and can be re-enabled any time in Settings → Lenovo software. L-Helper takes over the Fn keys.",
            names.join("\n  • ")
        );
        if crate::win::message_box(&text, true) {
            for d in running {
                let _ = d.disable();
                config::set(&d.config_key(), 1);
            }
            keys::invalidate_status();
        }
    });

    // Sensors, power source changes and resume detection
    std::thread::Builder::new()
        .name("background".into())
        .spawn(|| {
            let mut plugged = sensors::plugged_in();
            let mut last = Instant::now();
            loop {
                let visible = crate::win::is_visible(hwnd());
                std::thread::sleep(Duration::from_secs(if visible || crate::win::overlay::visible() { 1 } else { 5 }));
                let gap = last.elapsed();
                last = Instant::now();

                if gap > Duration::from_secs(30) {
                    // Woke from sleep: Custom limits and lighting ownership can be lost
                    crate::log!("Resume detected");
                    job(|| {
                        std::thread::sleep(Duration::from_secs(3));
                        lighting::on_resume();
                        if mode::get() == Some(Mode::Custom) {
                            mode::apply_custom();
                        }
                        crate::nvapi::apply_saved();
                        refresh_state();
                    });
                }

                let now_plugged = sensors::plugged_in();
                if now_plugged != plugged {
                    plugged = now_plugged;
                    crate::log!("Power source: {}", if plugged { "AC" } else { "Battery" });
                    let now = plugged;
                    job(move || {
                        use crate::automation::{self, Source, KEEP};
                        // An automation rule for the mode / refresh rate replaces the built-in per-source behaviour
                        let source = if now { Source::Plugged } else { Source::Battery };
                        let rule = |what: &str| automation::enabled() && source.get(what) != KEEP;
                        if !rule("mode") {
                            if config::is_not_false("auto_mode") {
                                mode::auto_performance();
                            } else if mode::get() == Some(Mode::Custom) {
                                mode::apply_custom(); // switch to the plugged-in / battery Custom profile
                            }
                        }
                        if !rule("hz") {
                            auto_screen();
                        }
                        let done = automation::on_power_change(now);
                        if !done.is_empty() {
                            toast::show(&format!("{}: {}", if now { "Plugged in" } else { "On battery" }, done.join(" · ")), if now { COLOR_BALANCED } else { COLOR_QUIET });
                        }
                        refresh_state();
                    });
                }
                refresh_sensors();
            }
        })
        .ok();
}

pub fn shutdown() {
    LISTENERS.lock().unwrap().clear();
    keys::stop_driver();
    // Hand lighting back to the keyboard firmware, but keep "LHelper drives lighting" for next start
    lighting::stop(false);
    config::flush();
}

/// Cleans up and ends the process. Plain process exit could hang inside library unload code (seen after
/// "Lighting handed back to firmware" with the WinRT LampArray and audio capture loaded), leaving a zombie
/// LHelper that had to be killed from Task Manager; everything that matters is saved by then.
pub fn quit() -> ! {
    shutdown();
    crate::log!("Exit");
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        let _ = TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::exit(0)
}
