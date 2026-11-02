//! Settings window (G-Helper's Extra): key locks, USB charging, Lenovo software, behaviour, theme

use egui::{RichText, Vec2, ViewportId};
use std::sync::{Arc, Mutex};

use super::widgets::{bold, card, divider, primary_button, secondary_button, toggle_row, Theme};
use crate::app;
use crate::config;
use crate::lenovo::software::{self, Status};
use crate::lenovo::{energy, gamezone, keys, machine};
use crate::updates::{self, Status as UpdateStatus};

#[derive(Clone, Default)]
struct Hardware {
    startup: Option<bool>,
    winkey: Option<bool>,
    touchpad: Option<bool>,
    fn_lock: Option<bool>,
    usb: Option<bool>,
    software: Vec<Status>,
    /// Dynamic Lighting package registered for this user (win/package.rs)
    package: Option<String>,
}

#[derive(Clone)]
enum Updates {
    Idle,
    Checking,
    Done(Result<updates::Report, String>),
}

pub struct ExtraWindow {
    open: bool,
    loaded: Arc<Mutex<Option<Hardware>>>,
    hw: Hardware,
    busy: Arc<Mutex<Vec<bool>>>,
    updates: Arc<Mutex<Updates>>,
    /// Dynamic Lighting registration in progress (None), or its last result
    package_job: Arc<Mutex<Option<Option<Result<(), String>>>>>,
}

fn load() -> Hardware {
    let m = machine::get();
    Hardware {
        winkey: m.supports_winkey_lock.then(|| gamezone::winkey_lock().ok().map(|v| v == 1)).flatten(),
        touchpad: m.supports_touchpad_lock.then(|| gamezone::touchpad_lock().ok().map(|v| v == 1)).flatten(),
        fn_lock: if m.has_energy_driver { energy::fn_lock() } else { None },
        usb: if m.has_energy_driver { energy::always_on_usb() } else { None },
        software: software::ALL.iter().map(|d| d.status()).collect(),
        startup: Some(crate::win::startup::is_scheduled()),
        package: crate::win::package::registered(),
    }
}

impl ExtraWindow {
    pub fn new(open: bool) -> Self {
        let mut w = ExtraWindow {
            open: false,
            loaded: Arc::new(Mutex::new(None)),
            hw: Hardware::default(),
            busy: Arc::new(Mutex::new(vec![false; 4])),
            updates: Arc::new(Mutex::new(Updates::Idle)),
            package_job: Arc::new(Mutex::new(None)),
        };
        if open {
            w.open();
        }
        w
    }

    pub fn id() -> ViewportId {
        ViewportId::from_hash_of("extra")
    }

    pub fn open(&mut self) {
        if self.open {
            super::bring_to_front(Self::id());
            return;
        }
        self.open = true;
        self.reload();
        super::child_opened();
    }

    fn reload(&self) {
        let slot = self.loaded.clone();
        std::thread::spawn(move || {
            *slot.lock().unwrap() = Some(load());
            app::repaint();
        });
    }

    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.open {
            return;
        }
        if let Some(hw) = self.loaded.lock().unwrap().take() {
            self.hw = hw;
        }
        let t = *theme;
        ctx.show_viewport_immediate(Self::id(), super::child_builder("Settings", [520.0, 760.0]), |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(16).fill(t.bg)).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.contents(ui, &t));
            });
        });
    }

    /// Hardware switch backed by a driver call; disabled when the current state couldn't be read
    fn hw_toggle(ui: &mut egui::Ui, t: &Theme, label: &str, description: &str, value: Option<bool>, set: impl FnOnce(bool) -> Result<(), String> + Send + 'static) -> Option<bool> {
        let mut v = value.unwrap_or(false);
        let changed = ui.add_enabled_ui(value.is_some(), |ui| toggle_row(ui, t, label, description, &mut v).changed()).inner;
        if changed {
            app::job(move || {
                if let Err(e) = set(v) {
                    crate::log!("{e}");
                }
            });
            return Some(v);
        }
        None
    }

    fn automations(&mut self, ui: &mut egui::Ui, t: &Theme) {
        use crate::automation::{self, Source, HZ_HIGHEST, HZ_LOWEST, KEEP, RESTORE};
        let mut on = automation::enabled();
        if toggle_row(ui, t, "When I plug in or unplug", "Applied once at that moment, never forced: change anything by hand and it stays", &mut on).changed() {
            config::set("auto_rules", on as i64);
        }
        if !on {
            ui.horizontal(|ui| {
                if secondary_button(ui, t, "Use battery saver rules", Vec2::new(190.0, 30.0))
                    .on_hover_text("On battery: Quiet, lowest refresh rate, keyboard off. Plugged in: last mode, highest refresh rate, keyboard as before.")
                    .clicked()
                {
                    automation::use_battery_saver_rules();
                }
            });
            return;
        }
        let s = app::state().clone();
        let modes: Vec<(i64, String)> = crate::mode::available().into_iter().map(|m| (m as i64, m.name().to_string())).collect();
        let rates: Vec<(i64, String)> = s.rates.iter().rev().map(|r| (*r as i64, format!("{r} Hz"))).collect();
        let gpus: Vec<(i64, String)> = [crate::gpu::GpuMode::Eco, crate::gpu::GpuMode::Standard, crate::gpu::GpuMode::Optimized]
            .into_iter()
            .enumerate()
            .filter(|(_, g)| crate::gpu::supported() && crate::gpu::available().contains(g))
            .map(|(i, g)| (i as i64, g.name().to_string()))
            .collect();

        let combo = |ui: &mut egui::Ui, id: String, label: &str, options: &[(i64, String)], source: Source, what: &str| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let current = source.get(what);
                    let text = options.iter().find(|o| o.0 == current).map(|o| o.1.clone()).unwrap_or_else(|| "Keep".into());
                    egui::ComboBox::from_id_salt(id).width(190.0).selected_text(text).show_ui(ui, |ui| {
                        for (value, name) in options {
                            if ui.selectable_label(current == *value, name.as_str()).clicked() {
                                source.set(what, *value);
                            }
                        }
                    });
                });
            });
        };

        for source in [Source::Battery, Source::Plugged] {
            divider(ui, t);
            let battery = source == Source::Battery;
            ui.label(RichText::new(if battery { "On battery" } else { "Plugged in" }).font(bold(13.5)));
            let tag = if battery { "dc" } else { "ac" };

            let mut mode_options = vec![(KEEP, "Keep".to_string()), (RESTORE, "Last used here".to_string())];
            mode_options.extend(modes.iter().cloned());
            combo(ui, format!("rule_mode_{tag}"), "Mode", &mode_options, source, "mode");

            if !s.rates.is_empty() {
                let mut hz_options = vec![(KEEP, "Keep".to_string()), (HZ_LOWEST, "Lowest".to_string()), (HZ_HIGHEST, "Highest".to_string())];
                hz_options.extend(rates.iter().cloned());
                combo(ui, format!("rule_hz_{tag}"), "Refresh rate", &hz_options, source, "hz");
            }

            if crate::lighting::is_supported() {
                let mut kb_options = vec![(KEEP, "Keep".to_string())];
                if !battery {
                    kb_options.push((RESTORE, "As before unplugging".to_string()));
                }
                kb_options.extend([(0, "Off".to_string()), (25, "25%".to_string()), (50, "50%".to_string()), (100, "100%".to_string())]);
                combo(ui, format!("rule_kb_{tag}"), "Keyboard lighting", &kb_options, source, "kb");
            }

            if !gpus.is_empty() {
                let mut gpu_options = vec![(KEEP, "Keep".to_string())];
                gpu_options.extend(gpus.iter().cloned());
                combo(ui, format!("rule_gpu_{tag}"), "GPU", &gpu_options, source, "gpu");
            }
        }
    }

    /// Background lighting control needs L-Helper's sparse package; register it from the release zip's files
    fn dynamic_lighting(&mut self, ui: &mut egui::Ui, t: &Theme) {
        use crate::win::package;
        let small = |ui: &mut egui::Ui, s: &str, color| {
            ui.label(RichText::new(s).size(12.0).color(color));
        };
        let active = crate::lighting::lamparray::package_name().is_some();
        let job = self.package_job.lock().unwrap().clone();
        let busy = matches!(job, Some(None));
        let start = |job: &Arc<Mutex<Option<Option<Result<(), String>>>>>, f: fn() -> Result<(), String>| {
            *job.lock().unwrap() = Some(None);
            let job = job.clone();
            std::thread::spawn(move || {
                let result = f();
                *job.lock().unwrap() = Some(Some(result));
                app::repaint();
            });
        };

        if active {
            small(ui, "L-Helper is a background lighting controller. Keep it at the top of Background light control so other apps don't take the keyboard over.", t.text2);
        } else if self.hw.package.is_some() || matches!(job, Some(Some(Ok(())))) {
            small(ui, "Registered. Restart L-Helper to control the keyboard alongside Dynamic Lighting.", t.text2);
        } else if package::bundle().is_some() {
            small(ui, "Lets Windows Dynamic Lighting hand the keyboard to L-Helper in the background. Installs L-Helper to Program Files and trusts its signing certificate for app packages on this PC.", t.text2);
        } else {
            small(ui, "Not registered. Unzip the whole release (LHelper.msix and LHelper.cer next to lhelper.exe) to register from here.", t.text2);
        }
        if let Some(Some(Err(e))) = &job {
            let (r, g, b) = app::COLOR_PERFORMANCE;
            small(ui, e, egui::Color32::from_rgb(r, g, b));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!busy, |ui| {
                let size = Vec2::new(110.0, 30.0);
                if active {
                    if secondary_button(ui, t, "Open settings", size).clicked() {
                        crate::win::open_url("ms-settings:personalization-lighting");
                    }
                    if secondary_button(ui, t, "Remove", size).clicked() {
                        start(&self.package_job, package::unregister);
                        self.reload();
                    }
                } else if self.hw.package.is_some() || matches!(job, Some(Some(Ok(())))) {
                    if primary_button(ui, t, "Restart", size).clicked() {
                        package::restart_installed();
                    }
                } else if package::bundle().is_some() && primary_button(ui, t, if busy { "Registering…" } else { "Register" }, size).clicked() {
                    start(&self.package_job, package::register);
                }
            });
        });
    }

    fn updates_section(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let state = self.updates.lock().unwrap().clone();
        let checking = matches!(state, Updates::Checking);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 160.0);
                ui.label(RichText::new("BIOS and driver updates").size(13.5));
                let note = match &state {
                    Updates::Idle => "From Lenovo's update catalog for your model".to_string(),
                    Updates::Checking => "Reading Lenovo's catalog and your installed drivers…".to_string(),
                    Updates::Done(Err(e)) => e.clone(),
                    Updates::Done(Ok(r)) if r.packages.iter().all(|p| p.status != UpdateStatus::Available) => format!("Up to date · {} packages checked for {}", r.checked, r.machine_type),
                    Updates::Done(Ok(r)) => format!("{} packages checked for {}", r.checked, r.machine_type),
                };
                ui.label(RichText::new(note).size(12.0).color(t.text2));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!checking, |ui| {
                    if secondary_button(ui, t, if checking { "Checking…" } else { "Check now" }, Vec2::new(120.0, 30.0)).clicked() {
                        *self.updates.lock().unwrap() = Updates::Checking;
                        let slot = self.updates.clone();
                        std::thread::spawn(move || {
                            let result = updates::check();
                            if let Err(e) = &result {
                                crate::log!("Update check failed: {e}");
                            }
                            *slot.lock().unwrap() = Updates::Done(result);
                            app::repaint();
                        });
                    }
                });
            });
        });

        let Updates::Done(Ok(report)) = state else { return };
        for p in &report.packages {
            divider(ui, t);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&p.title).font(bold(13.5)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(&p.version).size(12.0).color(t.text2));
                });
            });
            let mut details = vec![p.category.clone(), p.release_date.clone(), format!("{:.1} MB", p.size_mb)];
            if p.restart {
                details.push("restart needed".into());
            }
            ui.label(RichText::new(details.join("  ·  ")).size(12.0).color(t.text2));
            if p.status == UpdateStatus::CheckManually {
                ui.label(RichText::new("Lenovo checks this one with its own tool, so L-Helper can't tell if you already have it.").size(12.0).color(t.text2));
            }
            ui.horizontal(|ui| {
                if secondary_button(ui, t, "Download", Vec2::new(100.0, 28.0)).on_hover_text(&p.file_url).clicked() {
                    crate::win::open_url(&p.file_url);
                }
                if let Some(readme) = &p.readme_url {
                    if secondary_button(ui, t, "Release notes", Vec2::new(116.0, 28.0)).clicked() {
                        crate::win::open_url(readme);
                    }
                }
            });
        }
        if !report.packages.is_empty() {
            ui.label(RichText::new("Downloads open in your browser from download.lenovo.com. Plug in before a BIOS update and don't interrupt it.").size(12.0).color(t.text2));
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let m = machine::get();
        let heading = |ui: &mut egui::Ui, s: &str| {
            ui.add_space(6.0);
            ui.label(RichText::new(s).font(bold(14.0)));
        };

        // ---------- Hardware ----------
        if m.supports_winkey_lock || m.supports_touchpad_lock || m.has_energy_driver {
            heading(ui, "Keyboard and ports");
            card(ui, t, |ui| {
                let mut first = true;
                let mut sep = |ui: &mut egui::Ui| {
                    if !std::mem::replace(&mut first, false) {
                        divider(ui, t);
                    }
                };
                if m.supports_winkey_lock {
                    sep(ui);
                    if let Some(v) = Self::hw_toggle(ui, t, "Lock Windows key", "", self.hw.winkey, |v| gamezone::set_winkey_lock(v as i32)) {
                        self.hw.winkey = Some(v);
                    }
                }
                if m.supports_touchpad_lock {
                    sep(ui);
                    if let Some(v) = Self::hw_toggle(ui, t, "Lock touchpad", "", self.hw.touchpad, |v| gamezone::set_touchpad_lock(v as i32)) {
                        self.hw.touchpad = Some(v);
                    }
                }
                if m.has_energy_driver {
                    sep(ui);
                    if let Some(v) = Self::hw_toggle(ui, t, "Fn lock", "F1–F12 work without holding Fn", self.hw.fn_lock, energy::set_fn_lock) {
                        self.hw.fn_lock = Some(v);
                    }
                    sep(ui);
                    if let Some(v) = Self::hw_toggle(ui, t, "Always-on USB", "Charge USB devices while the laptop sleeps", self.hw.usb, energy::set_always_on_usb) {
                        self.hw.usb = Some(v);
                    }
                }
            });
        }

        // ---------- Startup & tray ----------
        heading(ui, "Startup and tray");
        card(ui, t, |ui| {
            let mut startup = self.hw.startup.unwrap_or(false);
            let changed = ui
                .add_enabled_ui(self.hw.startup.is_some(), |ui| toggle_row(ui, t, "Start with Windows", "At sign-in with admin rights, no UAC prompt", &mut startup).changed())
                .inner;
            if changed {
                self.hw.startup = Some(startup);
                app::job(move || {
                    if startup {
                        crate::win::startup::schedule();
                    } else {
                        crate::win::startup::unschedule();
                    }
                    app::refresh_state();
                });
            }
            divider(ui, t);
            let mut start_minimized = config::is_not_false("start_minimized");
            if toggle_row(ui, t, "Start minimized", "Stay in the tray when started with Windows", &mut start_minimized).changed() {
                config::set("start_minimized", start_minimized as i64);
            }
            divider(ui, t);
            let mut to_tray = config::is_not_false("minimize_to_tray");
            if toggle_row(ui, t, "Close to tray", "Off: closing the window exits L-Helper", &mut to_tray).changed() {
                config::set("minimize_to_tray", to_tray as i64);
            }
        });

        // ---------- Dynamic Lighting ----------
        heading(ui, "Dynamic Lighting");
        card(ui, t, |ui| self.dynamic_lighting(ui, t));

        // ---------- Game overlay ----------
        heading(ui, "Game overlay");
        card(ui, t, |ui| {
            use crate::win::overlay;
            let mut shown = overlay::visible();
            if toggle_row(ui, t, "Show overlay", "CPU, GPU, RAM and VRAM usage, temperatures and power · Ctrl+Shift+O", &mut shown).changed() {
                overlay::toggle();
            }
            divider(ui, t);
            let mut changed = false;
            let slider = |ui: &mut egui::Ui, label: &str, key: &str, default: i64, range: std::ops::RangeInclusive<i64>, zero: &str| -> bool {
                let mut v = config::get(key, default).clamp(*range.start(), *range.end());
                let mut moved = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).size(13.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(if v == 0 && !zero.is_empty() { zero.to_string() } else { format!("{v}%") }).font(bold(13.0)));
                        ui.spacing_mut().slider_width = 190.0;
                        if ui.add(egui::Slider::new(&mut v, range).show_value(false)).changed() {
                            config::set(key, v);
                            moved = true;
                        }
                    });
                });
                moved
            };
            changed |= slider(ui, "Background", "overlay_bg", 80, 0..=100, "Transparent");
            changed |= slider(ui, "Size", "overlay_scale", 100, 60..=200, "");
            let combo = |ui: &mut egui::Ui, label: &str, key: &str, names: &[&str]| -> bool {
                let mut current = config::get(key, 0).clamp(0, names.len() as i64 - 1) as usize;
                let before = current;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).size(13.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::ComboBox::from_id_salt(key).width(150.0).selected_text(names[current]).show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(&mut current, i, *n);
                            }
                        });
                    });
                });
                if current != before {
                    config::set(key, current as i64);
                }
                current != before
            };
            changed |= combo(ui, "Position", "overlay_corner", &["Top right", "Top left", "Bottom right", "Bottom left"]);
            changed |= combo(ui, "Layout", "overlay_layout", &["One row", "Column"]);
            if changed {
                overlay::refresh();
            }
        });

        // ---------- Behaviour ----------
        heading(ui, "Behaviour");
        card(ui, t, |ui| {
            let flags = [
                ("Remember mode per power source", "Separate modes for plugged in and battery", "auto_mode", true),
                ("On-screen notifications", "For Fn keys and mode changes", "osd", true),
                ("Match Windows power mode", "Set the Windows power mode with each performance mode", "windows_power_mode", true),
                ("Quiet → Performance workaround", "Go through Balanced; for BIOSes that misbehave otherwise", "mode_workaround", false),
            ];
            for (i, (label, description, key, default_on)) in flags.into_iter().enumerate() {
                if i > 0 {
                    divider(ui, t);
                }
                let mut v = if default_on { config::is_not_false(key) } else { config::is(key) };
                if toggle_row(ui, t, label, description, &mut v).changed() {
                    config::set(key, v as i64);
                }
                if key == "osd" {
                    divider(ui, t);
                    ui.add_enabled_ui(v, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Notification position").size(13.5));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                use crate::win::toast;
                                let current = (config::get("osd_position", 0).max(0) as usize).min(toast::POSITIONS.len() - 1);
                                let mut selected = current;
                                egui::ComboBox::from_id_salt("osd_position").width(170.0).selected_text(toast::POSITIONS[selected]).show_ui(ui, |ui| {
                                    for (i, n) in toast::POSITIONS.iter().enumerate() {
                                        ui.selectable_value(&mut selected, i, *n);
                                    }
                                });
                                if selected != current {
                                    config::set("osd_position", selected as i64);
                                    // Preview where it lands
                                    toast::show("Notifications appear here", app::COLOR_BALANCED);
                                }
                            });
                        });
                    });
                }
            }
            divider(ui, t);
            ui.horizontal(|ui| {
                ui.label(RichText::new("App theme").size(13.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let current = match config::get_string("ui_mode").as_deref() {
                        Some("dark") => 1,
                        Some("light") => 2,
                        _ => 0,
                    };
                    let names = ["Use Windows setting", "Dark", "Light"];
                    let mut selected = current;
                    egui::ComboBox::from_id_salt("theme").width(170.0).selected_text(names[selected]).show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut selected, i, *n);
                        }
                    });
                    if selected != current {
                        match selected {
                            1 => config::set_string("ui_mode", "dark"),
                            2 => config::set_string("ui_mode", "light"),
                            _ => config::remove("ui_mode"),
                        }
                    }
                });
            });
        });

        // ---------- Automations ----------
        heading(ui, "Automations");
        card(ui, t, |ui| self.automations(ui, t));

        // ---------- Lenovo software ----------
        heading(ui, "Lenovo software");
        card(ui, t, |ui| {
            ui.label(RichText::new("Disabled apps stay off: L-Helper disables them again if an update turns them back on.").size(12.0).color(t.text2));
            for (i, d) in software::ALL.iter().enumerate() {
                let status = self.hw.software.get(i).copied();
                let busy = self.busy.lock().unwrap()[i];
                divider(ui, t);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - 110.0);
                        ui.label(RichText::new(d.name).size(13.5)).on_hover_text(d.description);
                        let text = match (busy, status) {
                            (true, _) => "Working…",
                            (_, Some(Status::Enabled)) => "Running",
                            (_, Some(Status::Stopped)) => "Not running",
                            (_, Some(Status::Disabled)) => "Disabled",
                            (_, Some(Status::NotFound)) => "Not installed",
                            (_, None) => "…",
                        };
                        ui.label(RichText::new(text).size(12.0).color(if status == Some(Status::Enabled) { t.accent } else { t.text2 }));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let disable = matches!(status, Some(Status::Enabled | Status::Stopped));
                        let enabled = !busy && !matches!(status, None | Some(Status::NotFound));
                        ui.add_enabled_ui(enabled, |ui| {
                            if secondary_button(ui, t, if disable { "Disable" } else { "Enable" }, Vec2::new(96.0, 30.0)).clicked() {
                                self.busy.lock().unwrap()[i] = true;
                                let busy = self.busy.clone();
                                let slot = self.loaded.clone();
                                std::thread::spawn(move || {
                                    let d = software::ALL[i];
                                    let result = if disable { d.disable() } else { d.enable() };
                                    config::set(&d.config_key(), disable as i64);
                                    keys::invalidate_status();
                                    if let Err(e) = result {
                                        crate::win::message_box(&format!("{}:\n{e}", d.name), false);
                                    }
                                    busy.lock().unwrap()[i] = false;
                                    *slot.lock().unwrap() = Some(load());
                                    app::repaint();
                                });
                            }
                        });
                    });
                });
            }
        });

        // ---------- Updates ----------
        heading(ui, "Updates");
        card(ui, t, |ui| self.updates_section(ui, t));

        // ---------- About ----------
        heading(ui, "About");
        card(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width() - 110.0);
                    ui.label(RichText::new(format!("{}  ·  {}", m.model, m.machine_type)).size(13.5));
                    ui.label(RichText::new(format!("BIOS {}  ·  {}", m.bios, m.cpu)).size(12.0).color(t.text2));
                    ui.label(
                        RichText::new(format!("L-Helper {}  ·  {:?} gen {}, smart fan v{}", env!("CARGO_PKG_VERSION"), m.platform, m.generation, m.smart_fan_version))
                            .size(12.0)
                            .color(t.text2),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_button(ui, t, "Open log", Vec2::new(96.0, 30.0)).clicked() {
                        let _ = std::process::Command::new(crate::win::system_exe("notepad.exe")).arg(crate::log::log_file()).spawn();
                    }
                });
            });
        });
        ui.add_space(4.0);
    }
}
