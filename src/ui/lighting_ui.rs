//! 24-zone lighting window, laid out like LOQ-24-zone-RGB-controller's Home page:
//! keyboard visualizer + "ACTIVE LIGHTING PRESET" on the left, a bordered preset card with parameters on the right.

use egui::{Color32, CornerRadius, FontId, RichText, Stroke, Vec2, ViewportId};

use super::widgets::{keyboard_visualizer, Theme, KEYBOARD_ASPECT};
use crate::app;
use crate::config;
use crate::lighting::{self, presets, zone::Color};

// The original controller's palette (Theme.cs)
const BACKGROUND: Color32 = Color32::from_rgb(5, 5, 5);
const CARD: Color32 = Color32::from_rgb(9, 9, 11);
const BORDER: Color32 = Color32::from_rgb(24, 24, 27);
const TEXT: Color32 = Color32::from_rgb(255, 255, 255);
const TEXT2: Color32 = Color32::from_rgb(161, 161, 170);
const MUTED: Color32 = Color32::from_rgb(82, 82, 91);

const LEFT_WIDTH: f32 = 500.0;

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

fn caption(text: &str) -> RichText {
    RichText::new(text).font(mono(12.0)).color(TEXT2)
}

/// Value as a percentage of its range, like CalculatePercentage in the original
fn percent(v: f32, min: f32, max: f32) -> i32 {
    if max <= min {
        return 0;
    }
    (((v - min) / (max - min)) * 100.0).round() as i32
}

pub struct LightingWindow {
    open: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
    recording_hotkey: bool,
    /// "Type here to test" box, so typing effects can be tried without leaving the window
    test: String,
}

impl LightingWindow {
    pub fn new(open: bool) -> Self {
        let mut w = LightingWindow { open: false, recording_hotkey: false, test: String::new() };
        if open {
            w.open();
        }
        w
    }

    pub fn id() -> ViewportId {
        ViewportId::from_hash_of("lighting")
    }

    pub fn open(&mut self) {
        if self.open {
            super::bring_to_front(Self::id());
            return;
        }
        self.open = true;
        super::child_opened();
    }

    pub fn show(&mut self, ctx: &egui::Context, _theme: &Theme) {
        if !self.open {
            return;
        }
        ctx.show_viewport_immediate(Self::id(), super::child_builder("Keyboard Lighting", [900.0, 620.0]), |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(40)); // live keyboard
            egui::CentralPanel::default().frame(egui::Frame::new().fill(BACKGROUND).inner_margin(16)).show(ui, |ui| {
                style(ui);
                self.contents(ui);
            });
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        let current = lighting::current();
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(LEFT_WIDTH);
                self.left(ui, &current);
            });
            ui.add_space(16.0);
            egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, BORDER)).inner_margin(16).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(height - 34.0);
                ui.vertical(|ui| self.card(ui, &current));
            });
        });
    }

    // ---------------- left column ----------------

    fn left(&mut self, ui: &mut egui::Ui, current: &presets::Preset) {
        keyboard_visualizer(ui, &lighting::frame(), Vec2::new(LEFT_WIDTH, LEFT_WIDTH / KEYBOARD_ASPECT), BACKGROUND);

        ui.add_space(16.0);
        ui.label(caption("ACTIVE LIGHTING PRESET"));
        let all = presets::all();
        let mut selected = all.iter().position(|p| p.id == current.id).unwrap_or(0);
        egui::ComboBox::from_id_salt("rgb_preset")
            .width(LEFT_WIDTH)
            .height(420.0)
            .selected_text(RichText::new(all[selected].name).font(mono(13.0)).color(TEXT2))
            .show_ui(ui, |ui| {
                for (i, p) in all.iter().enumerate() {
                    if ui.selectable_value(&mut selected, i, RichText::new(p.name).font(mono(13.0))).clicked() {
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

        ui.add_space(18.0);
        let mut brightness = lighting::brightness();
        ui.label(caption(&format!("BRIGHTNESS: {brightness}%")));
        ui.spacing_mut().slider_width = LEFT_WIDTH;
        if ui.add(egui::Slider::new(&mut brightness, 0..=100).show_value(false)).changed() {
            lighting::set_brightness(brightness);
        }

        ui.add_space(18.0);
        ui.label(caption("CONTROL"));
        let mut software = lighting::is_running();
        if ui
            .checkbox(&mut software, RichText::new("L-Helper controls the keyboard lighting").color(TEXT2))
            .on_hover_text("Off: lighting goes back to the keyboard firmware (Fn+Space and Legion Space effects)")
            .changed()
        {
            app::job(move || if software { lighting::start() } else { lighting::stop(true) });
        }
        #[cfg(windows)]
        if software && !lighting::dynamic_lighting_enabled() {
            ui.add_space(4.0);
            egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, Color32::from_rgb(120, 90, 20))).inner_margin(10).show(ui, |ui| {
                ui.set_width(LEFT_WIDTH - 22.0);
                ui.label(RichText::new("WINDOWS DYNAMIC LIGHTING IS OFF").font(mono(12.0)).color(Color32::from_rgb(250, 190, 60)));
                ui.label(
                    RichText::new("L-Helper's effects need it: with Dynamic Lighting off, Windows hands the keyboard to its firmware and presets don't change. Turn on \"Use Dynamic Lighting on my devices\".")
                        .size(12.0)
                        .color(TEXT2),
                );
                if ui.add(egui::Button::new(RichText::new("OPEN DYNAMIC LIGHTING SETTINGS").font(mono(12.0)).color(TEXT2)).fill(BACKGROUND).stroke(Stroke::new(1.0, BORDER))).clicked() {
                    crate::win::open_url("ms-settings:personalization-lighting");
                }
            });
        }

        let mut cycle = lighting::cycle_list();
        let mut in_cycle = cycle.iter().any(|c| c == current.id);
        if ui.checkbox(&mut in_cycle, RichText::new("Include this preset in the cycle (hotkey / Fn keys)").color(TEXT2)).changed() {
            cycle.retain(|c| c != current.id);
            if in_cycle {
                cycle.push(current.id.to_string());
            }
            lighting::set_cycle_list(&cycle);
        }

        // Linux build: Wayland gives apps no global hotkeys; desktops bind a shortcut to `lhelper --next-lighting`
        #[cfg(not(windows))]
        ui.label(RichText::new("Cycle from a keyboard shortcut: bind `lhelper --next-lighting` in your desktop's keyboard settings").font(mono(11.0)).color(MUTED));
        #[cfg(windows)]
        ui.add_space(6.0);
        #[cfg(windows)]
        ui.horizontal(|ui| {
            ui.label(caption("CYCLE HOTKEY"));
            let text = if self.recording_hotkey { "PRESS KEYS…".to_string() } else { config::get_string("rgb_hotkey").unwrap_or_else(|| "NONE".into()).to_uppercase() };
            if ui.add_sized([170.0, 26.0], egui::Button::new(RichText::new(text).font(mono(12.0)).color(TEXT2)).fill(CARD).stroke(Stroke::new(1.0, BORDER))).clicked() {
                self.recording_hotkey = !self.recording_hotkey;
            }
            if self.recording_hotkey {
                self.capture_hotkey(ui);
            }
        });
        #[cfg(windows)]
        ui.label(RichText::new("e.g. Ctrl+Alt+L · Backspace clears").font(mono(11.0)).color(MUTED));

        ui.add_space(14.0);
        ui.label(caption("TRY IT"));
        ui.add(
            egui::TextEdit::singleline(&mut self.test)
                .desired_width(LEFT_WIDTH)
                .font(mono(13.0))
                .text_color(TEXT)
                .background_color(CARD)
                .hint_text(RichText::new("Type here to test typing effects…").font(mono(12.0)).color(MUTED)),
        );
        if self.test.len() > 200 {
            self.test.clear();
        }
    }

    // ---------------- preset card ----------------

    fn card(&mut self, ui: &mut egui::Ui, current: &presets::Preset) {
        ui.label(RichText::new(current.name).size(19.0).color(TEXT));
        ui.label(RichText::new(current.description).size(12.5).color(TEXT2));
        ui.add_space(14.0);

        if current.params.is_empty() {
            ui.add_space(ui.available_height() / 2.0 - 30.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("NO PARAMETERS FOR THIS PRESET").font(mono(12.0)).color(MUTED)));
            return;
        }

        let mut values = lighting::values(current);
        let mut changed = false;
        egui::ScrollArea::vertical().max_height(ui.available_height() - 44.0).show(ui, |ui| {
            for param in &current.params {
                match param.color {
                    Some(_) => {
                        let c = values.c(current, param.name);
                        let mut rgb = [c.r, c.g, c.b];
                        ui.horizontal(|ui| {
                            ui.label(caption(&param.label.to_uppercase()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().interact_size = Vec2::new(50.0, 28.0);
                                if ui.color_edit_button_srgb(&mut rgb).changed() {
                                    values.colors.insert(param.name.to_string(), Color::new(rgb[0], rgb[1], rgb[2]));
                                    changed = true;
                                }
                                ui.label(RichText::new(Color::new(rgb[0], rgb[1], rgb[2]).hex()).font(mono(12.0)).color(TEXT2));
                            });
                        });
                        ui.add_space(10.0);
                    }
                    None => {
                        let mut v = values.f(current, param.name);
                        ui.horizontal(|ui| {
                            ui.label(caption(&param.label.to_uppercase()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let text = if param.min == param.max { "FIXED".to_string() } else { format!("{}%", percent(v, param.min, param.max)) };
                                ui.label(RichText::new(text).font(mono(12.0)).color(TEXT2));
                            });
                        });
                        ui.spacing_mut().slider_width = ui.available_width();
                        let response = ui.add(egui::Slider::new(&mut v, param.min..=param.max).step_by(param.step as f64).show_value(false));
                        if response.changed() {
                            // like the original, every scroll step applies live and is saved as a tweak
                            values.floats.insert(param.name.to_string(), v);
                            changed = true;
                        }
                        ui.add_space(10.0);
                    }
                }
            }
        });
        if changed {
            lighting::save_values(current, &values);
            lighting::apply(current, Some(&values), false);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            let reset = egui::Button::new(RichText::new("RESET PRESET").font(mono(12.0)).color(TEXT2)).fill(BACKGROUND).stroke(Stroke::new(1.0, BORDER));
            if ui.add_sized([140.0, 28.0], reset).clicked() {
                config::remove(&format!("rgb_params_{}", current.id));
                lighting::apply(current, None, false);
            }
        });
    }

    #[cfg(windows)]
    fn capture_hotkey(&mut self, ui: &egui::Ui) {
        let events = ui.ctx().input(|i| i.events.clone());
        for e in events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                if matches!(key, egui::Key::Backspace | egui::Key::Delete | egui::Key::Escape) {
                    config::remove("rgb_hotkey");
                } else if modifiers.ctrl || modifiers.alt || modifiers.shift {
                    let mut parts = Vec::new();
                    if modifiers.ctrl {
                        parts.push("Ctrl".to_string());
                    }
                    if modifiers.alt {
                        parts.push("Alt".to_string());
                    }
                    if modifiers.shift {
                        parts.push("Shift".to_string());
                    }
                    parts.push(key.name().to_string());
                    config::set_string("rgb_hotkey", &parts.join("+"));
                } else {
                    continue; // require a modifier so normal typing isn't captured
                }
                self.recording_hotkey = false;
                crate::win::hotkey::reload();
                break;
            }
        }
    }
}

/// Local widget styling with the original's palette (scoped to this window's Ui)
fn style(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.override_text_color = Some(TEXT);
    v.extreme_bg_color = CARD;
    v.selection.bg_fill = Color32::from_rgb(228, 228, 231);
    v.selection.stroke = Stroke::new(1.0, TEXT);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.bg_fill = Color32::from_rgb(39, 39, 42); // slider rails / checkbox boxes
        w.weak_bg_fill = CARD;
        w.bg_stroke = Stroke::new(1.0, BORDER);
        w.corner_radius = CornerRadius::same(2);
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT2);
}
