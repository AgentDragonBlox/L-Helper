//! "Fans + Power" (gaming) / "Power" (IdeaPad): Custom mode fan curve and power limits,
//! with separate "Plugged in" and "On battery" profiles

use egui::{RichText, Vec2, ViewportId};
use std::sync::{Arc, Mutex};

use super::widgets::{bold, card, card_title, fan_curve, mode_button, primary_button, rgb, secondary_button, toggle, Theme};
use crate::app;
use crate::config;
use crate::lenovo::fans::{self, FanTableData, DEFAULT_TABLE};
use crate::lenovo::gamezone::{self, Cap};
use crate::lenovo::machine;
use crate::mode::{self, Mode, Profile};

const AC: usize = 0;
const DC: usize = 1;
const PROFILES: [Profile; 2] = [Profile::Ac, Profile::Battery];

#[derive(Clone)]
struct PowerRow {
    cap: Cap,
    name: &'static str,
    unit: &'static str,
    gpu: bool,
    min: i32,
    max: i32,
    step: i32,
    steps: Vec<i32>,
    /// [plugged in, on battery]
    defaults: [i32; 2],
    values: [i32; 2],
}

#[derive(Clone, Default)]
struct Data {
    fan_data: Vec<FanTableData>,
    rows: Vec<PowerRow>,
    tables: [[i32; 10]; 2],
    full_speed: [bool; 2],
    is_custom: bool,
}

pub struct FansWindow {
    open: bool,
    loaded: Arc<Mutex<Option<Data>>>,
    data: Option<Data>,
    drag: Option<usize>,
    status: Arc<Mutex<String>>,
    profile: usize,
}

const DEFINITIONS: [(Cap, &str, &str, bool); 12] = [
    (Cap::CpuLongTerm, "CPU Sustained (PL1 / SPL)", "W", false),
    (Cap::CpuShortTerm, "CPU Boost (PL2 / sPPT)", "W", false),
    (Cap::CpuPeak, "CPU Peak (PL4 / fPPT)", "W", false),
    (Cap::ApuSppt, "APU sPPT", "W", false),
    (Cap::CpuCrossLoading, "CPU Cross-load limit", "W", false),
    (Cap::CpuPl2Tau, "Boost time window", "s", false),
    (Cap::CpuTempLimit, "CPU Temperature limit", "°C", false),
    (Cap::GpuTgp, "GPU TGP", "W", true),
    (Cap::GpuPowerBoost, "GPU Dynamic Boost", "W", true),
    (Cap::GpuTppOffset, "GPU Total power offset", "W", true),
    (Cap::GpuToCpuBoost, "GPU → CPU Dynamic Boost", "W", true),
    (Cap::GpuTempLimit, "GPU Temperature limit", "°C", true),
];

const TAU_STEPS: [i32; 13] = [20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160];

fn saved(profile: Profile, cap: Cap) -> Option<i32> {
    let v = config::get(&profile.cap_key(cap), i64::MIN);
    (v != i64::MIN).then_some(v as i32)
}

fn load() -> Data {
    let m = machine::get();
    let is_custom = mode::get() == Some(Mode::Custom);
    let battery_saved = Profile::Battery.is_saved();
    let mut rows = Vec::new();

    if m.is_ideapad() {
        // Non-gaming firmware has no capability ranges: LLT's NonGaming table, power capped at 100 W, only what the firmware answers for
        for (cap, min, max, def) in [(Cap::CpuLongTerm, 0, 100, 0), (Cap::CpuShortTerm, 0, 100, 0), (Cap::CpuTempLimit, 60, 100, 100), (Cap::CpuPl2Tau, 20, 160, 56)] {
            let current = gamezone::try_feature(cap);
            crate::log!("IdeaPad {cap:?}: {current:?}");
            if current.is_none() && !m.simulate_ideapad {
                continue;
            }
            if cap == Cap::CpuPl2Tau && !current.map(|c| TAU_STEPS.contains(&c)).unwrap_or(false) {
                continue; // 0 = unused by this firmware; don't offer a value nobody chose
            }
            let (_, name, unit, gpu) = DEFINITIONS.iter().find(|d| d.0 == cap).copied().unwrap();
            let default = current.unwrap_or(def);
            let ac = saved(Profile::Ac, cap).unwrap_or(default);
            let dc = if battery_saved { saved(Profile::Battery, cap).unwrap_or(default) } else { default };
            let steps = if cap == Cap::CpuPl2Tau { TAU_STEPS.to_vec() } else { Vec::new() };
            rows.push(PowerRow { cap, name, unit, gpu, min, max, step: 1, steps, defaults: [default, default], values: [ac, dc] });
        }
    } else {
        let ranges = gamezone::read_ranges();
        let discrete = gamezone::read_discrete();
        for (cap, name, unit, gpu) in DEFINITIONS {
            let Some(r) = gamezone::find_range(&ranges, cap, 0xFF) else { continue };
            let steps = discrete.get(&r.id).cloned().unwrap_or_default();
            if (r.step == 0 && steps.is_empty()) || (r.min == r.max && steps.is_empty()) {
                continue;
            }
            // Battery profile starts from the firmware's own Quiet mode limits (mode byte 1), kept inside Custom's range
            let quiet = gamezone::find_range(&ranges, cap, 1).map(|q| q.default.clamp(r.min, r.max)).unwrap_or(r.default);
            let live = if is_custom { gamezone::try_feature(cap) } else { None };
            let ac = saved(Profile::Ac, cap).or(live).unwrap_or(r.default);
            let dc = if battery_saved { saved(Profile::Battery, cap).unwrap_or(quiet) } else { quiet };
            rows.push(PowerRow { cap, name, unit, gpu, min: r.min, max: r.max, step: r.step.max(1), steps, defaults: [r.default, quiet], values: [ac, dc] });
        }
    }

    let fan_data = if m.is_ideapad() { Vec::new() } else { fans::read_table_data() };
    let ac_table = config::get_array("fan_table")
        .or_else(|| if is_custom && !m.is_ideapad() { fans::get_table(1, 1) } else { None })
        .map(|t| fans::sanitize(&t))
        .unwrap_or(DEFAULT_TABLE);
    let dc_table = config::get_array(&Profile::Battery.table_key()).map(|t| fans::sanitize(&t)).unwrap_or(ac_table);

    Data {
        fan_data,
        rows,
        tables: [ac_table, dc_table],
        full_speed: [config::is("fan_full_speed"), config::is(&Profile::Battery.full_speed_key())],
        is_custom,
    }
}

impl FansWindow {
    pub fn new(open: bool) -> Self {
        let mut w = FansWindow { open: false, loaded: Arc::new(Mutex::new(None)), data: None, drag: None, status: Arc::new(Mutex::new(String::new())), profile: AC };
        if open {
            w.open();
        }
        w
    }

    pub fn id() -> ViewportId {
        ViewportId::from_hash_of("fans")
    }

    fn set_status(&self, text: &str) {
        *self.status.lock().unwrap() = text.to_string();
    }

    pub fn open(&mut self) {
        if self.open {
            super::bring_to_front(Self::id());
            return;
        }
        self.open = true;
        self.data = None;
        // Start on the profile for the current power source
        self.profile = if Profile::current() == Profile::Battery { DC } else { AC };
        self.set_status("Reading firmware limits…");
        let slot = self.loaded.clone();
        app::job(move || {
            *slot.lock().unwrap() = Some(load());
        });
        super::child_opened();
    }

    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.open {
            return;
        }
        if let Some(d) = self.loaded.lock().unwrap().take() {
            let status = if d.rows.is_empty() && !machine::get().is_ideapad() {
                "Can't read power limit ranges from firmware (see log)"
            } else if d.is_custom {
                "Custom mode active"
            } else {
                "Changes apply when Custom mode is selected"
            };
            self.set_status(status);
            self.data = Some(d);
        }
        let ideapad = machine::get().is_ideapad();
        let (title, size) = if ideapad { ("Power", [520.0, 600.0]) } else { ("Fans + Power", [880.0, 660.0]) };
        let t = *theme;
        ctx.show_viewport_immediate(Self::id(), super::child_builder(title, size), |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(16).fill(t.bg)).show(ui, |ui| {
                self.contents(ui, &t, ideapad);
            });
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui, t: &Theme, ideapad: bool) {
        let status = self.status.lock().unwrap().clone();
        let Some(data) = &mut self.data else {
            ui.label(RichText::new(status).color(t.text2));
            return;
        };

        // ---------- profile switch ----------
        let now = Profile::current();
        ui.horizontal(|ui| {
            for (i, (label, accent)) in [("Plugged in", app::COLOR_BALANCED), ("On battery", app::COLOR_QUIET)].into_iter().enumerate() {
                let text = if PROFILES[i] == now { format!("{label}  ·  now") } else { label.to_string() };
                if mode_button(ui, t, "", &text, self.profile == i, rgb(accent), Vec2::new(170.0, 36.0)).clicked() {
                    self.profile = i;
                }
            }
            ui.add_space(6.0);
            let hint = if self.profile == DC { "Custom mode on battery. Starts from your Quiet limits." } else { "Custom mode when plugged in." };
            ui.label(RichText::new(hint).size(12.5).color(t.text2));
        });
        // The mode remembered per power source decides what LHelper switches to on (un)plugging
        let (key, label, fallback) = if self.profile == DC {
            ("performance_mode_dc", "Switch to Custom when I unplug", Mode::Quiet)
        } else {
            ("performance_mode_ac", "Switch to Custom when I plug in", Mode::Balanced)
        };
        let mut auto = config::get(key, -1) == Mode::Custom as i64;
        ui.horizontal(|ui| {
            if toggle(ui, t, &mut auto).changed() {
                config::set(key, if auto { Mode::Custom as i64 } else { fallback as i64 });
                if auto {
                    config::set("auto_mode", 1);
                }
            }
            ui.label(RichText::new(label).size(13.0));
        });
        ui.add_space(2.0);

        let p = self.profile;
        let height = ui.available_height() - 52.0;

        ui.horizontal_top(|ui| {
            // ---------- fan curve ----------
            if !ideapad {
                ui.allocate_ui(Vec2::new(ui.available_width() * 0.56, height), |ui| {
                    card(ui, t, |ui| {
                        ui.set_min_height(height - 26.0);
                        card_title(ui, t, "Fan curve", "Drag the points · hover for RPM");
                        ui.allocate_ui(Vec2::new(ui.available_width(), height - 150.0), |ui| {
                            fan_curve(ui, t, &mut data.tables[p], &data.fan_data, !data.full_speed[p], &mut self.drag);
                        });
                        ui.horizontal(|ui| {
                            toggle(ui, t, &mut data.full_speed[p]);
                            ui.label(RichText::new("Full speed").size(13.0));
                            ui.add_space(12.0);
                            let mut allow = fans::allow_zero();
                            if toggle(ui, t, &mut allow)
                                .on_hover_text("Lets the low-temperature points go down to level 0 (LLT's legacy minimum). Plugged in the fans then stop; on battery the firmware keeps a minimum speed.")
                                .changed()
                            {
                                config::set("fan_allow_zero", allow as i64);
                                if !allow {
                                    // back to the normal floor
                                    data.tables = [fans::sanitize(&data.tables[AC]), fans::sanitize(&data.tables[DC])];
                                }
                            }
                            ui.label(RichText::new("Allow fans to stop").size(13.0));
                        });
                        ui.horizontal(|ui| {
                            if secondary_button(ui, t, "Default curve", Vec2::new(112.0, 28.0)).clicked() {
                                data.tables[p] = DEFAULT_TABLE;
                            }
                            if fans::allow_zero() && secondary_button(ui, t, "Lowest curve", Vec2::new(112.0, 28.0)).clicked() {
                                data.tables[p] = fans::ZERO_MINIMUM_TABLE;
                            }
                            if p == DC && secondary_button(ui, t, "Copy plugged-in", Vec2::new(124.0, 28.0)).clicked() {
                                data.tables[DC] = data.tables[AC];
                            }
                        });
                    });
                });
            }

            // ---------- power limits ----------
            ui.allocate_ui(Vec2::new(ui.available_width(), height), |ui| {
                card(ui, t, |ui| {
                    ui.set_min_height(height - 26.0);
                    card_title(ui, t, "Power limits", "");
                    egui::ScrollArea::vertical().max_height(height - (if ideapad { 90.0 } else { 60.0 })).show(ui, |ui| {
                        let mut group = None;
                        for row in data.rows.iter_mut() {
                            let g = if row.gpu { "GPU" } else { "CPU" };
                            if group != Some(g) {
                                group = Some(g);
                                ui.add_space(2.0);
                                ui.label(RichText::new(g).font(bold(12.0)).color(t.text2));
                            }
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(row.name).size(13.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(format!("{} {}", row.values[p], row.unit)).font(bold(13.0)));
                                });
                            });
                            ui.scope(|ui| {
                                ui.spacing_mut().slider_width = ui.available_width() - 8.0;
                                if row.steps.is_empty() {
                                    ui.add(egui::Slider::new(&mut row.values[p], row.min..=row.max).step_by(row.step as f64).show_value(false));
                                } else {
                                    let mut index = row.steps.iter().position(|s| *s >= row.values[p]).unwrap_or(0);
                                    if ui.add(egui::Slider::new(&mut index, 0..=row.steps.len() - 1).show_value(false)).changed() {
                                        row.values[p] = row.steps[index];
                                    }
                                }
                            });
                        }
                        if data.rows.is_empty() {
                            ui.label(RichText::new("No adjustable power limits reported by firmware.").color(t.text2));
                        }
                    });
                    if ideapad {
                        ui.horizontal(|ui| {
                            toggle(ui, t, &mut data.full_speed[p]);
                            ui.label(RichText::new("Full speed fans").size(13.0));
                        });
                    }
                });
            });
        });

        // ---------- bottom bar ----------
        ui.add_space(4.0);
        let mut new_status = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&status).size(12.5).color(t.text2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if p == DC { "Save battery profile" } else { "Apply to Custom mode" };
                if primary_button(ui, t, label, Vec2::new(180.0, 32.0)).clicked() {
                    let profile = PROFILES[p];
                    for row in &data.rows {
                        config::set(&profile.cap_key(row.cap), row.values[p] as i64);
                    }
                    if !ideapad {
                        config::set_array(&profile.table_key(), &data.tables[p]);
                    }
                    config::set(&profile.full_speed_key(), data.full_speed[p] as i64);
                    if profile == Profile::Battery {
                        config::set("custom_dc_saved", 1);
                    }

                    if profile == Profile::current() {
                        // This profile is the active one: apply it now (switching to Custom if needed)
                        new_status = Some("Applying…".to_string());
                        let status = self.status.clone();
                        app::job(move || {
                            let result = if mode::get() == Some(Mode::Custom) {
                                mode::apply_custom();
                                Ok(())
                            } else {
                                mode::set(Mode::Custom)
                            };
                            *status.lock().unwrap() = match &result {
                                Ok(()) => "Applied to Custom mode".to_string(),
                                Err(e) => e.clone(),
                            };
                            app::report(result);
                            app::refresh_state();
                        });
                    } else {
                        new_status = Some(match profile {
                            Profile::Battery => "Saved. Used when you're on battery in Custom mode".to_string(),
                            Profile::Ac => "Saved. Used when you're plugged in and in Custom mode".to_string(),
                        });
                    }
                }
                if secondary_button(ui, t, "Reset", Vec2::new(90.0, 32.0)).clicked() {
                    let profile = PROFILES[p];
                    for row in data.rows.iter_mut() {
                        config::remove(&profile.cap_key(row.cap));
                        row.values[p] = row.defaults[p];
                    }
                    config::remove(&profile.table_key());
                    config::set(&profile.full_speed_key(), 0);
                    if profile == Profile::Battery {
                        config::remove("custom_dc_saved");
                    }
                    data.tables[p] = DEFAULT_TABLE;
                    data.full_speed[p] = false;
                    new_status = Some(if p == DC { "Battery defaults (Quiet limits) loaded, press Save".into() } else { "Defaults loaded, press Apply".into() });
                }
            });
        });
        if let Some(s) = new_status {
            self.set_status(&s);
        }
    }
}
