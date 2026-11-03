//! egui front end (Linux version of ../src/ui/mod.rs): main window plus Power limits, Keyboard Lighting
//! (shared with Windows) and Settings windows. The window can close while L-Helper keeps running in the tray;
//! `run` opens it again on request (tray click, second instance).

#[path = "../../../src/ui/lighting_ui.rs"]
mod lighting_ui;
mod power;
mod settings;
#[allow(dead_code)]
#[path = "../../../src/ui/widgets.rs"]
pub mod widgets;

use egui::{RichText, Vec2, ViewportBuilder, ViewportCommand};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use crate::app::{self, mode_color};
use crate::config;
use crate::hw::battery::BatteryMode;
use crate::hw::display;
use crate::hw::profile::Mode;
use crate::lighting::{self, presets};
use widgets::{icon_button, mode_button, rgb, row, section_header, Theme, ICON_BATTERY, ICON_BOLT, ICON_BULB, ICON_GAUGE, ICON_KEYBOARD, ICON_LEAF, ICON_MOON, ICON_POWER, ICON_SCREEN, ICON_SETTINGS, ICON_SLIDERS, ICON_SPEED, ICON_SYNC, ICON_THERMO};

pub const WIDTH: f32 = 500.0;
const APP_ID: &str = "lhelper";

static QUIT: AtomicBool = AtomicBool::new(false);
static WINDOW_OPEN: AtomicBool = AtomicBool::new(false);
static OPEN: Lazy<(Mutex<Sender<()>>, Mutex<Receiver<()>>)> = Lazy::new(|| {
    let (tx, rx) = channel();
    (Mutex::new(tx), Mutex::new(rx))
});

/// Shows the window: brings it forward when open, else opens it again
pub fn request_open() {
    if WINDOW_OPEN.load(Ordering::SeqCst) {
        if let Some(c) = app::ctx() {
            c.send_viewport_cmd(ViewportCommand::Minimized(false));
            c.send_viewport_cmd(ViewportCommand::Focus);
            c.request_repaint();
        }
    } else {
        let _ = OPEN.0.lock().unwrap().send(());
    }
}

/// Main thread from here on: the window while it's open, waiting for an open request while only the tray runs
pub fn run(start_hidden: bool) -> ! {
    let mut show = !start_hidden;
    loop {
        if show {
            WINDOW_OPEN.store(true, Ordering::SeqCst);
            if let Err(e) = run_window() {
                crate::log!("Window failed: {e}");
            }
            WINDOW_OPEN.store(false, Ordering::SeqCst);
            app::set_ctx(None);
            if QUIT.load(Ordering::SeqCst) || !crate::tray::available() || !config::is_not_false("minimize_to_tray") {
                app::quit();
            }
        }
        let rx = OPEN.1.lock().unwrap();
        let _ = rx.recv();
        while rx.try_recv().is_ok() {}
        show = true;
    }
}

fn run_window() -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("L-Helper")
            .with_app_id(APP_ID)
            .with_inner_size([WIDTH, 640.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_icon(egui::IconData { rgba: crate::logo::rgba(crate::logo::Logo::Blue, 64), width: 64, height: 64 }),
        ..Default::default()
    };
    eframe::run_native("L-Helper", options, Box::new(|cc| Ok(Box::new(LHelperApp::new(cc))))).map_err(|e| e.to_string())
}

// ---------------- light / dark ----------------

static DARK: AtomicBool = AtomicBool::new(true);

/// The desktop's light / dark preference (XDG portal "color-scheme": 1 dark, 2 light), polled in the background
fn watch_theme() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("theme".into())
        .spawn(|| {
            let Ok(conn) = zbus::blocking::Connection::session() else { return };
            // ReadOne returns the value; the older Read wraps it in one more variant
            fn number(v: &zbus::zvariant::Value) -> Option<u32> {
                match v {
                    zbus::zvariant::Value::U32(n) => Some(*n),
                    zbus::zvariant::Value::Value(inner) => number(inner),
                    _ => None,
                }
            }
            loop {
                let read = |method: &str| {
                    let reply = conn
                        .call_method(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.Settings"), method, &("org.freedesktop.appearance", "color-scheme"))
                        .ok()?;
                    let value = reply.body().deserialize::<zbus::zvariant::OwnedValue>().ok()?;
                    number(&value)
                };
                if let Some(scheme) = read("ReadOne").or_else(|| read("Read")) {
                    let dark = scheme != 2;
                    if DARK.swap(dark, Ordering::SeqCst) != dark {
                        app::repaint();
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        })
        .ok();
}

// ---------------- windows ----------------

pub struct LHelperApp {
    theme: Theme,
    power: power::PowerWindow,
    lighting: lighting_ui::LightingWindow,
    settings: settings::SettingsWindow,
    last_height: f32,
}

impl LHelperApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        watch_theme();
        let theme = Theme::new(DARK.load(Ordering::SeqCst));
        Theme::load_fonts(&cc.egui_ctx);
        theme.apply(&cc.egui_ctx);
        app::set_ctx(Some(cc.egui_ctx.clone()));
        app::job(app::refresh_state);
        LHelperApp { theme, power: power::PowerWindow::new(), lighting: lighting_ui::LightingWindow::new(false), settings: settings::SettingsWindow::new(), last_height: 0.0 }
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let s = app::state().clone();
        let r = &s.sensors;
        let tall = 58.0;
        let full = |ui: &egui::Ui| Vec2::new(ui.available_width(), 32.0);

        // ---------- Mode ----------
        let mut info = Vec::new();
        if let Some(c) = r.cpu_temp {
            info.push(format!("CPU: {c}°C"));
        }
        if crate::hw::sensors::nvidia_present() {
            match (r.gpu_asleep, r.gpu_temp) {
                (true, _) => info.push("GPU: asleep".into()),
                (_, Some(c)) => info.push(format!("GPU: {c}°C")),
                _ => {}
            }
        }
        if let Some(f) = r.cpu_fan {
            info.push(format!("Fan: {f} RPM"));
        }
        let accent = s.mode.map(|m| rgb(mode_color(m))).unwrap_or(t.text2);
        section_header(ui, &t, ICON_SPEED, &format!("Mode: {}", s.mode.map(|m| m.name()).unwrap_or("-")), &info.join("   "), accent);
        if s.modes.is_empty() {
            note(ui, &t, "No performance modes: the kernel has no platform_profile for this laptop. LOQ and Legion need Linux 6.16 or newer (lenovo-wmi-gamezone); IdeaPads use ideapad_laptop.");
        } else {
            row(ui, s.modes.len(), tall, |ui, i, size| {
                let m = s.modes[i];
                let icon = match m {
                    Mode::Quiet => ICON_LEAF,
                    Mode::Balanced => ICON_POWER,
                    Mode::Performance => ICON_BOLT,
                    Mode::Extreme => ICON_GAUGE,
                    Mode::Custom => ICON_SLIDERS,
                };
                if mode_button(ui, &t, icon, m.name(), s.mode == Some(m), rgb(mode_color(m)), size).on_hover_text(m.hint()).clicked() {
                    app::set_mode(m);
                }
            });
        }
        if s.power_limits && icon_button(ui, &t, ICON_THERMO, "Power limits", full(ui)).clicked() {
            self.power.open();
        }

        // ---------- Screen ----------
        if let Some(screen) = s.screen.as_ref().filter(|sc| !sc.rates.is_empty()) {
            let auto = config::is("screen_auto");
            let mut shown = vec![];
            if let Some(low) = display::low_rate(&screen.rates) {
                shown.push(low);
            }
            if let Some(high) = screen.rates.last() {
                if !shown.contains(high) {
                    shown.push(*high);
                }
            }
            let accent_for = |hz: u32| if Some(&hz) == screen.rates.last() { app::COLOR_PERFORMANCE } else { app::COLOR_BALANCED };
            let current = if auto { app::COLOR_QUIET } else { screen.rate.map(accent_for).unwrap_or(app::COLOR_BALANCED) };
            section_header(ui, &t, ICON_SCREEN, "Laptop Screen", &screen.rate.map(|r| format!("{r} Hz")).unwrap_or_default(), rgb(current));
            row(ui, shown.len() + 1, tall, |ui, i, size| {
                if i == 0 {
                    if mode_button(ui, &t, ICON_SYNC, "Auto", auto, rgb(app::COLOR_QUIET), size).on_hover_text("Lowest refresh rate on battery, highest when plugged in").clicked() {
                        app::set_screen_auto();
                    }
                } else {
                    let hz = shown[i - 1];
                    let icon = if Some(&hz) == screen.rates.last() { ICON_BOLT } else { ICON_MOON };
                    if mode_button(ui, &t, icon, &format!("{hz} Hz"), !auto && screen.rate == Some(hz), rgb(accent_for(hz)), size).clicked() {
                        app::set_refresh_rate(hz);
                    }
                }
            });
            ui.horizontal(|ui| {
                let half = (ui.available_width() - 8.0) / 2.0;
                let rate_text = screen.rate.map(|r| format!("{r} Hz")).unwrap_or_else(|| "Refresh rate".into());
                egui::ComboBox::from_id_salt("refresh").width(half - 8.0).selected_text(rate_text).show_ui(ui, |ui| {
                    for &hz in screen.rates.iter().rev() {
                        if ui.selectable_label(!auto && screen.rate == Some(hz), format!("{hz} Hz")).clicked() {
                            app::set_refresh_rate(hz);
                        }
                    }
                });
                let res_text = screen.resolution.map(|(w, h)| format!("{w} × {h}")).unwrap_or_else(|| "Resolution".into());
                egui::ComboBox::from_id_salt("resolution").width(half - 8.0).height(320.0).selected_text(res_text).show_ui(ui, |ui| {
                    for (i, &(w, h)) in screen.resolutions.iter().enumerate() {
                        let label = if i == 0 { format!("{w} × {h}  (native)") } else { format!("{w} × {h}") };
                        if ui.selectable_label(screen.resolution == Some((w, h)), label).clicked() && screen.resolution != Some((w, h)) {
                            app::set_resolution(w, h);
                        }
                    }
                });
            });
        }

        // ---------- Keyboard ----------
        self.keyboard_ui(ui, &s);

        // ---------- Battery ----------
        if !s.battery_modes.is_empty() {
            let mut info = Vec::new();
            if let Some(p) = r.battery_percent {
                info.push(format!("Charge: {p}%"));
            }
            match r.battery_rate.filter(|x| x.abs() > 0.1) {
                Some(rate) if rate < 0.0 => info.push(format!("{rate:.1} W")),
                Some(rate) => info.push(format!("+{rate:.1} W")),
                None => {}
            }
            let all = [
                (BatteryMode::Conservation, ICON_LEAF, "Conservation", app::COLOR_QUIET, "Keeps the battery around 80% to extend its lifespan"),
                (BatteryMode::Normal, ICON_BATTERY, "Normal", app::COLOR_BALANCED, "Charges to 100%"),
                (BatteryMode::RapidCharge, ICON_BOLT, "Rapid Charge", app::COLOR_PERFORMANCE, "Charges faster, more battery wear"),
            ];
            let modes: Vec<_> = all.iter().filter(|b| s.battery_modes.contains(&b.0)).collect();
            let accent = modes.iter().find(|b| s.battery == Some(b.0)).map(|b| rgb(b.3)).unwrap_or(t.text2);
            section_header(ui, &t, ICON_BATTERY, "Battery", &info.join("   "), accent);
            row(ui, modes.len(), tall, |ui, i, size| {
                let (b, icon, name, color, hint) = *modes[i];
                if mode_button(ui, &t, icon, name, s.battery == Some(b), rgb(color), size).on_hover_text(hint).clicked() {
                    app::set_battery(b);
                }
            });
        }

        if !s.status.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(&s.status).size(12.5).color(rgb(app::COLOR_PERFORMANCE)));
        }

        // ---------- Footer ----------
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut startup = s.autostart;
            if widgets::toggle(ui, &t, &mut startup).on_hover_text("Starts L-Helper in the tray when you sign in").changed() {
                app::job(move || {
                    app::report(crate::autostart::set(startup));
                    app::refresh_state();
                });
            }
            ui.label(RichText::new("Run at login").size(13.0).color(t.text2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, &t, "✖", "Quit", Vec2::new(86.0, 32.0)).clicked() {
                    QUIT.store(true, Ordering::SeqCst);
                }
                if icon_button(ui, &t, ICON_SETTINGS, "Settings", Vec2::new(108.0, 32.0)).clicked() {
                    self.settings.open();
                }
            });
        });
    }

    fn keyboard_ui(&mut self, ui: &mut egui::Ui, s: &app::State) {
        let t = self.theme;
        if lighting::is_supported() && !crate::lighting::hidraw::permission_denied() {
            let current = lighting::current();
            let running = lighting::is_running();
            let info = if running { format!("{}  ·  {}%", current.name, lighting::brightness()) } else { "Firmware lighting".to_string() };
            let mid = lighting::frame()[12];
            let tint = if running && (mid.r as u16 + mid.g as u16 + mid.b as u16) > 40 { egui::Color32::from_rgb(mid.r, mid.g, mid.b) } else { rgb(app::COLOR_EXTREME) };
            section_header(ui, &t, ICON_KEYBOARD, "Keyboard", &info, tint);
            if running {
                if widgets::zone_strip(ui, &t, &lighting::frame(), 10.0).on_hover_text("Open keyboard lighting").clicked() {
                    self.lighting.open();
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
            }
            ui.horizontal(|ui| {
                let all = presets::all();
                let mut selected = all.iter().position(|p| p.id == current.id).unwrap_or(0);
                let buttons = 40.0 + 112.0 + 16.0;
                egui::ComboBox::from_id_salt("preset").width(ui.available_width() - buttons).height(380.0).selected_text(all[selected].name).show_ui(ui, |ui| {
                    for (j, p) in all.iter().enumerate() {
                        if ui.selectable_value(&mut selected, j, p.name).clicked() {
                            let p = p.clone();
                            app::job(move || {
                                if !lighting::is_running() {
                                    lighting::start();
                                }
                                lighting::apply(&p, None, true);
                            });
                        }
                    }
                });
                if icon_button(ui, &t, ICON_SYNC, "", Vec2::new(40.0, 28.0)).on_hover_text("Next preset in your cycle list").clicked() {
                    app::cycle_lighting();
                }
                if icon_button(ui, &t, ICON_BULB, "Lighting", Vec2::new(112.0, 28.0)).clicked() {
                    self.lighting.open();
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Brightness").size(13.0).color(t.text2));
                let mut b = lighting::brightness();
                ui.spacing_mut().slider_width = ui.available_width() - 56.0;
                if ui.add(egui::Slider::new(&mut b, 0..=100).suffix("%")).changed() {
                    lighting::set_brightness(b);
                }
            });
        } else if crate::lighting::hidraw::permission_denied() {
            section_header(ui, &t, ICON_KEYBOARD, "Keyboard", "", rgb(app::COLOR_EXTREME));
            note(ui, &t, "The 24-zone keyboard is there, but L-Helper may not open it. Install the package (its udev rule gives you access), then unplug or restart.");
        } else if let Some((level, max)) = s.keyboard_light {
            let name = |l: i32| match (l, max) {
                (0, _) => "Off".to_string(),
                (1, 2) => "Low".to_string(),
                (2, 2) => "High".to_string(),
                _ => l.to_string(),
            };
            section_header(ui, &t, ICON_KEYBOARD, "Keyboard backlight", "", rgb(app::COLOR_BALANCED));
            row(ui, (max + 1).min(6) as usize, 40.0, |ui, i, size| {
                let l = if max <= 5 { i as i32 } else { (i as i32 * max) / 5 };
                if mode_button(ui, &t, "", &name(l), level == l, rgb(app::COLOR_BALANCED), size).clicked() {
                    app::set_keyboard_light(l);
                }
            });
        }
    }
}

/// Grey explanatory text in a card
fn note(ui: &mut egui::Ui, t: &Theme, text: &str) {
    widgets::card(ui, t, |ui| {
        ui.label(RichText::new(text).size(12.5).color(t.text2));
    });
}

impl eframe::App for LHelperApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if QUIT.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dark = DARK.load(Ordering::SeqCst);
        if dark != self.theme.dark {
            self.theme = Theme::new(dark);
            self.theme.apply(&ctx);
        }

        // Never taller than the screen: scroll beyond that
        let max_height = ctx.input(|i| i.viewport().monitor_size).map(|m| m.y - 96.0).filter(|h| *h > 200.0).unwrap_or(f32::MAX);
        let mut content = 0.0;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.global_style()).inner_margin(egui::Margin::symmetric(16, 10)).fill(self.theme.bg))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 32.0);
                let output = egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(max_height - 12.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.main_ui(ui);
                });
                content = output.content_size.y.ceil() + 18.0;
            });

        // Fit the window to its content, up to the screen height
        let height = content.min(max_height);
        if (height - self.last_height).abs() > 1.0 {
            self.last_height = height;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(WIDTH, height)));
        }

        self.power.show(&ctx, &self.theme);
        self.lighting.show(&ctx, &self.theme);
        self.settings.show(&ctx, &self.theme);

        // Sensors and state are updated from other threads; repaint slowly to pick them up
        ctx.request_repaint_after(std::time::Duration::from_millis(1000));
    }
}

/// Brings an already open child window back in front
pub fn bring_to_front(id: egui::ViewportId) {
    if let Some(ctx) = app::ctx() {
        ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

/// Windows sets dark title bars here; Linux desktops draw their own
pub fn child_opened() {}

pub fn child_builder(title: &str, size: [f32; 2]) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(title)
        .with_app_id(APP_ID)
        .with_inner_size(size)
        .with_resizable(false)
        .with_maximize_button(false)
        .with_minimize_button(false)
        .with_icon(egui::IconData { rgba: crate::logo::rgba(crate::logo::Logo::Blue, 64), width: 64, height: 64 })
}
