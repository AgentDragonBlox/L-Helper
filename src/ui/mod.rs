//! egui front end: main window plus Fans/Power, Lighting and Extra windows (immediate viewports)

mod extra;
mod fans;
mod gpu_tune;
mod lighting_ui;
pub mod widgets;

use egui::{RichText, Vec2, ViewportBuilder, ViewportCommand};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::{self, mode_color};
use crate::config;
use crate::gpu;
use crate::lenovo::energy::BatteryMode;
use crate::lenovo::{gamezone, keyboard, machine};
use crate::lighting::{self, presets};
use crate::mode;
use crate::win;
use widgets::{icon_button, mode_button, rgb, row, section_header, toggle_row, Theme, ICON_BATTERY, ICON_BOLT, ICON_BULB, ICON_CHIP, ICON_GAUGE, ICON_KEYBOARD, ICON_LAPTOP, ICON_LEAF, ICON_MOON, ICON_POWER, ICON_SCREEN, ICON_SETTINGS, ICON_SLIDERS, ICON_SPARKLE, ICON_SPEED, ICON_SYNC, ICON_THERMO};

const POWER_HINT: &str = "CPU and GPU power from their own meters. The laptop has no sensor for the total from the charger: \
                          the screen, memory, SSD and keyboard add roughly 5-15 W, plus whatever goes into the battery.";


pub const WIDTH: f32 = 500.0;

/// Saved GPU tuning for the button label, e.g. "+100 core · +300 mem · 900 mV" (None when stock / not applied at startup)
fn nvapi_summary() -> Option<String> {
    let (core, memory, cap) = crate::nvapi::saved();
    let mut parts = Vec::new();
    if core != 0 {
        parts.push(format!("{core:+} core"));
    }
    if memory != 0 {
        parts.push(format!("{memory:+} mem"));
    }
    if cap != 0 {
        parts.push(format!("{cap} mV"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub static QUIT: AtomicBool = AtomicBool::new(false);

pub struct LHelperApp {
    theme: Theme,
    fans: fans::FansWindow,
    gpu_tune: gpu_tune::GpuTuneWindow,
    lighting: lighting_ui::LightingWindow,
    extra: extra::ExtraWindow,
    last_height: f32,
    screenshot_dir: Option<std::path::PathBuf>,
    frames: u32,
    spectrum_effect: usize,
    spectrum_speed: u8,
    dark_titles_done: bool,
}

const SPECTRUM_EFFECTS: [(u8, &str); 10] = [
    (11, "Static"),
    (4, "Breathe"),
    (2, "Rainbow wave"),
    (1, "Rainbow spiral"),
    (3, "Color cycle"),
    (6, "Smooth"),
    (5, "Color wave"),
    (7, "Rain"),
    (8, "Ripple"),
    (12, "Type lighting"),
];

impl LHelperApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot_dir: Option<std::path::PathBuf>) -> Self {
        let theme = Theme::new(win::dark_mode());
        Theme::load_fonts(&cc.egui_ctx);
        theme.apply(&cc.egui_ctx);
        app::set_ctx(cc.egui_ctx.clone());

        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = cc.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                app::set_hwnd(h.hwnd.get());
                win::set_window_app_id(app::hwnd());
                crate::logo::apply_window_icons(app::hwnd());
                win::set_dark_title_bar(app::hwnd(), theme.dark);
                win::set_menu_theme(theme.dark);
            }
        }

        let screenshots = screenshot_dir.is_some();
        LHelperApp {
            theme,
            fans: fans::FansWindow::new(screenshots),
            gpu_tune: gpu_tune::GpuTuneWindow::new(false),
            lighting: lighting_ui::LightingWindow::new(screenshots && lighting::is_supported()),
            extra: extra::ExtraWindow::new(screenshots),
            last_height: 0.0,
            screenshot_dir,
            frames: 0,
            spectrum_effect: config::get("kb_effect", 0).clamp(0, 9) as usize,
            spectrum_speed: config::get("kb_speed", 2).clamp(1, 3) as u8,
            dark_titles_done: false,
        }
    }

    fn main_ui(&mut self, ui: &mut egui::Ui) {
        let t = self.theme;
        let m = machine::get();
        let s = app::state().clone();
        let r = &s.sensors;
        let tall = 58.0;
        let full = |ui: &egui::Ui| Vec2::new(ui.available_width(), 32.0);

        // ---------- Mode ----------
        let mut cpu = Vec::new();
        if let Some(c) = r.cpu_temp {
            cpu.push(format!("CPU: {c}°C"));
        }
        if let Some(f) = r.cpu_fan {
            cpu.push(format!("Fan: {f} RPM"));
        }
        let mode_accent = s.mode.map(|md| rgb(mode_color(md))).unwrap_or(t.text2);
        section_header(ui, &t, ICON_SPEED, &format!("Mode: {}", s.mode.map(|md| md.name()).unwrap_or("-")), &cpu.join("   "), mode_accent);
        let modes = mode::available();
        row(ui, modes.len(), tall, |ui, i, size| {
            let md = modes[i];
            let icon = match md {
                mode::Mode::Quiet => ICON_LEAF,
                mode::Mode::Balanced => ICON_POWER,
                mode::Mode::Performance => ICON_BOLT,
                mode::Mode::Extreme => ICON_GAUGE,
                mode::Mode::Custom => ICON_SLIDERS,
            };
            if mode_button(ui, &t, icon, md.name(), s.mode == Some(md), rgb(mode_color(md)), size).on_hover_text(md.hint()).clicked() {
                app::set_mode(md);
            }
        });
        if m.supports_custom {
            let label = if m.is_ideapad() { "Power limits" } else { "Fans + Power" };
            if icon_button(ui, &t, ICON_THERMO, label, full(ui)).clicked() {
                self.fans.open();
            }
        }

        // ---------- GPU ----------
        let tuning = gpu::dgpu_present();
        if gpu::supported() || tuning {
            let gpu_accent = |g: gpu::GpuMode| {
                rgb(match g {
                    gpu::GpuMode::Eco | gpu::GpuMode::Optimized => app::COLOR_QUIET,
                    gpu::GpuMode::Standard => app::COLOR_BALANCED,
                    gpu::GpuMode::Ultimate => app::COLOR_PERFORMANCE,
                })
            };
            let mut info = Vec::new();
            match (r.gpu_asleep, r.gpu_temp, r.gpu_power) {
                (true, _, _) => info.push("GPU: asleep".to_string()),
                (_, Some(c), _) => info.push(format!("GPU: {c}°C")),
                _ => {}
            }
            if let Some(f) = r.gpu_fan {
                info.push(format!("Fan: {f} RPM"));
            }
            let title = match s.gpu.filter(|_| gpu::supported()) {
                Some(g) => format!("GPU: {}", g.name()),
                None => "GPU".to_string(),
            };
            section_header(ui, &t, ICON_CHIP, &title, &info.join("   "), s.gpu.map(gpu_accent).unwrap_or(rgb(app::COLOR_QUIET)));
            if gpu::supported() {
                let modes = gpu::available();
                row(ui, modes.len(), tall, |ui, i, size| {
                    let g = modes[i];
                    let icon = match g {
                        gpu::GpuMode::Eco => ICON_LEAF,
                        gpu::GpuMode::Standard => ICON_LAPTOP,
                        gpu::GpuMode::Optimized => ICON_SPARKLE,
                        gpu::GpuMode::Ultimate => ICON_CHIP,
                    };
                    if mode_button(ui, &t, icon, g.name(), s.gpu == Some(g), gpu_accent(g), size).on_hover_text(g.hint()).clicked() {
                        app::set_gpu(g);
                    }
                });
            }
            // NVIDIA overclock / undervolt (the dGPU must be connected: not in Eco mode)
            if tuning {
                let label = match nvapi_summary() {
                    Some(sum) => format!("GPU Tuning  ·  {sum}"),
                    None => "GPU Tuning".to_string(),
                };
                if icon_button(ui, &t, ICON_SLIDERS, &label, full(ui)).on_hover_text("Overclock and undervolt").clicked() {
                    self.gpu_tune.open();
                }
            }
        }

        // ---------- Screen ----------
        if s.screen.is_some() && !s.rates.is_empty() {
            let auto = config::is("screen_auto");
            let mut shown = vec![];
            if let Some(low) = win::display::low_rate(&s.rates) {
                shown.push(low);
            }
            if let Some(high) = s.rates.last() {
                if !shown.contains(high) {
                    shown.push(*high);
                }
            }
            let accent_for = |hz: u32| if Some(&hz) == s.rates.last() { app::COLOR_PERFORMANCE } else { app::COLOR_BALANCED };
            let current = if auto { app::COLOR_QUIET } else { s.rate.map(accent_for).unwrap_or(app::COLOR_BALANCED) };
            section_header(ui, &t, ICON_SCREEN, "Laptop Screen", &s.rate.map(|r| format!("{r} Hz")).unwrap_or_default(), rgb(current));
            row(ui, shown.len() + 1, tall, |ui, i, size| {
                if i == 0 {
                    if mode_button(ui, &t, ICON_SYNC, "Auto", auto, rgb(app::COLOR_QUIET), size).on_hover_text("Lowest refresh rate on battery, highest when plugged in").clicked() {
                        app::set_screen_auto();
                    }
                } else {
                    let hz = shown[i - 1];
                    let icon = if Some(&hz) == s.rates.last() { ICON_BOLT } else { ICON_MOON };
                    if mode_button(ui, &t, icon, &format!("{hz} Hz"), !auto && s.rate == Some(hz), rgb(accent_for(hz)), size).clicked() {
                        app::set_refresh_rate(hz);
                    }
                }
            });
            // Every refresh rate and resolution the panel reports (nothing it can't do)
            ui.horizontal(|ui| {
                let half = (ui.available_width() - 8.0) / 2.0;
                let rate_text = s.rate.map(|r| format!("{r} Hz")).unwrap_or_else(|| "Refresh rate".into());
                egui::ComboBox::from_id_salt("refresh").width(half - 8.0).selected_text(rate_text).show_ui(ui, |ui| {
                    for &hz in s.rates.iter().rev() {
                        if ui.selectable_label(!auto && s.rate == Some(hz), format!("{hz} Hz")).clicked() {
                            app::set_refresh_rate(hz);
                        }
                    }
                });
                let res_text = s.resolution.map(|(w, h)| format!("{w} × {h}")).unwrap_or_else(|| "Resolution".into());
                egui::ComboBox::from_id_salt("resolution").width(half - 8.0).height(320.0).selected_text(res_text).show_ui(ui, |ui| {
                    for (i, &(w, h)) in s.resolutions.iter().enumerate() {
                        let label = if i == 0 { format!("{w} × {h}  (native)") } else { format!("{w} × {h}") };
                        if ui.selectable_label(s.resolution == Some((w, h)), label).clicked() && s.resolution != Some((w, h)) {
                            app::set_resolution(w, h);
                        }
                    }
                });
            });
            // Only on panels that support it (LLT's capability or GameZone check)
            if let Some(mut od) = s.overdrive {
                if toggle_row(ui, &t, "Overdrive", "Faster pixel response, less ghosting", &mut od).changed() {
                    app::job(move || {
                        let _ = gamezone::set_overdrive(od as i32);
                        app::refresh_state();
                    });
                }
            }
            if s.oled {
                let mut care = crate::win::oled::enabled();
                if toggle_row(ui, &t, "OLED burn-in protection", "Auto-hide the taskbar, turn the screen off sooner", &mut care).changed() {
                    app::set_oled_care(care);
                }
            }
        }

        // ---------- Keyboard ----------
        self.keyboard_ui(ui, &s);

        // ---------- Battery ----------
        if m.has_energy_driver {
            let mut info = Vec::new();
            if let Some(p) = r.battery_percent {
                info.push(format!("Charge: {p}%"));
            }
            match r.battery_rate.filter(|x| x.abs() > 0.1) {
                Some(rate) if rate < 0.0 => info.push(format!("{:.1} W", rate)),
                Some(rate) => info.push(format!("+{rate:.1} W")),
                None => {}
            }
            if let Some(w) = r.total_power() {
                info.push(format!("CPU+GPU: {w:.1} W"));
            }
            let modes = [
                (BatteryMode::Conservation, ICON_LEAF, "Conservation", app::COLOR_QUIET, "Keeps the battery around 80% to extend its lifespan"),
                (BatteryMode::Normal, ICON_BATTERY, "Normal", app::COLOR_BALANCED, "Charges to 100%"),
                (BatteryMode::RapidCharge, ICON_BOLT, "Rapid Charge", app::COLOR_PERFORMANCE, "Charges faster, more battery wear"),
            ];
            let accent = modes.iter().find(|b| s.battery == Some(b.0)).map(|b| rgb(b.3)).unwrap_or(t.text2);
            ui.scope(|ui| {
                section_header(ui, &t, ICON_BATTERY, "Battery", &info.join("   "), accent);
            })
            .response
            .on_hover_text(POWER_HINT);
            row(ui, 3, tall, |ui, i, size| {
                let (b, icon, name, color, hint) = modes[i];
                if mode_button(ui, &t, icon, name, s.battery == Some(b), rgb(color), size).on_hover_text(hint).clicked() {
                    app::set_battery(b);
                }
            });
        }

        // ---------- Footer ----------
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let mut startup = s.startup;
            if widgets::toggle(ui, &t, &mut startup).on_hover_text("Starts L-Helper when you sign in, with admin rights and no UAC prompt").changed() {
                app::job(move || {
                    if startup {
                        win::startup::schedule();
                    } else {
                        win::startup::unschedule();
                    }
                    app::refresh_state();
                });
            }
            ui.label(RichText::new("Run on Startup").size(13.0).color(t.text2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, &t, ICON_POWER, "Quit", Vec2::new(86.0, 32.0)).clicked() {
                    QUIT.store(true, Ordering::SeqCst);
                }
                if icon_button(ui, &t, ICON_SETTINGS, "Settings", Vec2::new(108.0, 32.0)).clicked() {
                    self.extra.open();
                }
            });
        });
    }

    fn keyboard_ui(&mut self, ui: &mut egui::Ui, s: &app::State) {
        let t = self.theme;
        if lighting::is_supported() {
            let current = lighting::current();
            let running = lighting::is_running();
            let info = if running { format!("{}  ·  {}%", current.name, lighting::brightness()) } else { "Firmware lighting".to_string() };
            // the icon takes the keyboard's own middle color
            let mid = lighting::frame()[12];
            let tint = if running && (mid.r as u16 + mid.g as u16 + mid.b as u16) > 40 { egui::Color32::from_rgb(mid.r, mid.g, mid.b) } else { rgb(app::COLOR_EXTREME) };
            section_header(ui, &t, ICON_KEYBOARD, "Keyboard", &info, tint);
            if running {
                if widgets::zone_strip(ui, &t, &lighting::frame(), 10.0).on_hover_text("Open keyboard lighting").clicked() {
                    self.lighting.open();
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(40)); // live colors while visible
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
        } else if keyboard::has_spectrum() {
            section_header(ui, &t, ICON_KEYBOARD, "Keyboard", &s.spectrum_brightness.map(|b| format!("Brightness {b}")).unwrap_or_default(), rgb(app::COLOR_EXTREME));
            let mut changed = false;
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("effect").selected_text(SPECTRUM_EFFECTS[self.spectrum_effect].1).show_ui(ui, |ui| {
                    for (i, (_, name)) in SPECTRUM_EFFECTS.iter().enumerate() {
                        changed |= ui.selectable_value(&mut self.spectrum_effect, i, *name).changed();
                    }
                });
                egui::ComboBox::from_id_salt("speed").selected_text(["Slow", "Normal", "Fast"][self.spectrum_speed as usize - 1]).show_ui(ui, |ui| {
                    for (i, name) in ["Slow", "Normal", "Fast"].iter().enumerate() {
                        changed |= ui.selectable_value(&mut self.spectrum_speed, i as u8 + 1, *name).changed();
                    }
                });
                let argb = config::get("kb_color", 0xFFFFFF);
                let mut color = [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8];
                if ui.color_edit_button_srgb(&mut color).changed() {
                    config::set("kb_color", ((color[0] as i64) << 16) | ((color[1] as i64) << 8) | color[2] as i64);
                    changed = true;
                }
            });
            if changed {
                config::set("kb_effect", self.spectrum_effect as i64);
                config::set("kb_speed", self.spectrum_speed as i64);
                let (effect, speed) = (SPECTRUM_EFFECTS[self.spectrum_effect].0, self.spectrum_speed);
                let argb = config::get("kb_color", 0xFFFFFF);
                app::job(move || {
                    let profile = config::get("kb_profile", keyboard::spectrum_profile().unwrap_or(1) as i64).clamp(1, 6) as i32;
                    config::set("kb_profile", profile as i64);
                    keyboard::set_spectrum_effect(profile, effect, Some([(argb >> 16) as u8, (argb >> 8) as u8, argb as u8]), speed);
                });
            }
            if let Some(mut b) = s.spectrum_brightness {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Brightness").size(13.0).color(t.text2));
                    ui.spacing_mut().slider_width = ui.available_width() - 40.0;
                    if ui.add(egui::Slider::new(&mut b, 0..=9)).drag_stopped() {
                        app::job(move || {
                            keyboard::set_spectrum_brightness(b);
                            app::refresh_state();
                        });
                    }
                });
            }
            if let Some(mut logo) = s.logo {
                if toggle_row(ui, &t, "Logo light", "", &mut logo).changed() {
                    app::job(move || {
                        keyboard::set_logo(logo);
                        app::refresh_state();
                    });
                }
            }
        } else if s.white_level.is_some() || machine::get().keyboard_levels > 0 {
            let max = s.white_max;
            let name = |l: i32| match (l, max) {
                (0, _) => "Off".to_string(),
                (1, 2) => "Low".to_string(),
                (2, 2) => "High".to_string(),
                _ => l.to_string(),
            };
            section_header(ui, &t, ICON_KEYBOARD, "Keyboard backlight", "", rgb(app::COLOR_BALANCED));
            row(ui, (max + 1) as usize, 40.0, |ui, i, size| {
                let level = i as i32;
                if mode_button(ui, &t, "", &name(level), s.white_level == Some(level), rgb(app::COLOR_BALANCED), size).clicked() {
                    app::set_white_level(level);
                }
            });
        }
    }

    fn screenshots(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.screenshot_dir.clone() else { return };
        self.frames += 1;
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        // ~6 s for state, sensors and windows to settle, then capture every window with PrintWindow
        if self.frames == 45 {
            win::overlay::toggle();
        }
        if self.frames == 60 {
            let _ = std::fs::create_dir_all(&dir);
            let mut targets = vec![("LHelper", "main"), (if machine::get().is_ideapad() { "Power" } else { "Fans + Power" }, "fans"), ("Settings", "extra"), ("LHelper Overlay", "overlay")];
            if lighting::is_supported() {
                targets.push(("Keyboard Lighting", "lighting"));
            }
            for (title, name) in targets {
                let shot = if name == "main" { win::capture_hwnd(app::hwnd()) } else { win::capture_window(title) };
                if let Some((w, h, rgba)) = shot {
                    if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) {
                        let _ = img.save(dir.join(format!("{name}.png")));
                    }
                }
            }
            let _ = std::fs::copy(crate::log::log_file(), dir.join("log.txt"));
            std::process::exit(0);
        }
    }
}

impl eframe::App for LHelperApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if QUIT.load(Ordering::SeqCst) {
            app::quit();
        }
        let to_tray = config::is_not_false("minimize_to_tray");
        if ctx.input(|i| i.viewport().close_requested()) {
            if to_tray {
                // Close hides to the tray, like G-Helper
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                win::show_window(app::hwnd(), false);
            } else {
                app::quit();
            }
        }
        // Minimize goes to the tray too (no taskbar button) when the option is on
        if to_tray && ctx.input(|i| i.viewport().minimized) == Some(true) && win::is_visible(app::hwnd()) {
            win::show_window(app::hwnd(), false);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Follow the Windows theme
        let dark = win::dark_mode();
        if dark != self.theme.dark {
            self.theme = Theme::new(dark);
            self.theme.apply(&ctx);
            win::set_dark_title_bar(app::hwnd(), dark);
            win::set_menu_theme(dark);
            self.dark_titles_done = false;
        }

        // Never taller than the screen it's on (14" 1080p at 150% has ~650 pt of height): scroll beyond that
        let max_height = win::max_client_height(app::hwnd()).map(|px| (px as f32 / ctx.pixels_per_point()).floor()).filter(|h| *h > 200.0).unwrap_or(f32::MAX);
        let mut content = 0.0;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.global_style()).inner_margin(egui::Margin::symmetric(16, 10)).fill(self.theme.bg))
            .show(ui, |ui| {
                ui.set_width(WIDTH - 32.0);
                let output = egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(max_height - 12.0).show(ui, |ui| {
                    // the scrollbar, when shown, takes its share: lay out in what's left
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
            let hwnd = app::hwnd().0 as isize;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(60));
                win::keep_on_screen(windows::Win32::Foundation::HWND(hwnd as _));
            });
        }

        self.fans.show(&ctx, &self.theme);
        self.gpu_tune.show(&ctx, &self.theme);
        self.lighting.show(&ctx, &self.theme);
        self.extra.show(&ctx, &self.theme);

        if !self.dark_titles_done {
            self.dark_titles_done = true;
            let dark = self.theme.dark;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                for title in ["Fans + Power", "Power", "Keyboard Lighting", "Settings", "GPU Tuning"] {
                    win::dark_title_for(title, dark);
                }
            });
        }

        self.screenshots(&ctx);
        // Sensors and state are updated from other threads; repaint slowly to pick them up
        ctx.request_repaint_after(std::time::Duration::from_millis(1000));
    }
}

/// Brings an already open child window back in front (its button clicked again while it's behind other windows)
pub fn bring_to_front(id: egui::ViewportId) {
    if let Some(ctx) = app::ctx() {
        ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
        ctx.request_repaint();
    }
}

/// Opens a child window's dark title bar after creation
pub fn child_opened() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let dark = win::dark_mode();
        for title in ["Fans + Power", "Power", "Keyboard Lighting", "Settings", "GPU Tuning"] {
            win::dark_title_for(title, dark);
        }
    });
}

pub fn child_builder(title: &str, size: [f32; 2]) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(false)
        .with_maximize_button(false)
        .with_minimize_button(false)
        .with_taskbar(false)
        .with_icon(egui::IconData { rgba: crate::logo::rgba(crate::logo::Logo::Blue, 64), width: 64, height: 64 })
}
