//! Power limits window: the firmware's Custom-mode settings (hw/firmware.rs) as sliders inside their ranges

use egui::{RichText, Vec2, ViewportId};

use super::widgets::{bold, card, card_title, primary_button, secondary_button, Theme};
use crate::app;
use crate::config;
use crate::hw::firmware::{self, Attribute};

pub struct PowerWindow {
    open: bool,
    /// Attributes with the value each slider shows
    rows: Vec<(Attribute, i64)>,
}

impl PowerWindow {
    pub fn new() -> Self {
        PowerWindow { open: false, rows: Vec::new() }
    }

    pub fn id() -> ViewportId {
        ViewportId::from_hash_of("power")
    }

    pub fn open(&mut self) {
        if self.open {
            super::bring_to_front(Self::id());
            return;
        }
        self.reload();
        self.open = true;
    }

    /// Saved Custom values where there are any, else what the firmware has now
    fn reload(&mut self) {
        self.rows = firmware::attributes().into_iter().map(|a| {
            let v = config::get(&a.config_key(), a.value).clamp(a.min, a.max);
            (a, v)
        }).collect();
    }

    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.open {
            return;
        }
        let t = *theme;
        let height = (190.0 + self.rows.len() as f32 * 62.0).min(720.0);
        ctx.show_viewport_immediate(Self::id(), super::child_builder("Power limits", [460.0, height]), |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(16).fill(t.bg)).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.contents(ui, &t));
            });
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui, t: &Theme) {
        card(ui, t, |ui| {
            card_title(ui, t, "Custom mode", "");
            ui.label(RichText::new("The firmware uses these in Custom mode. Apply switches to Custom, and L-Helper applies them again whenever Custom is selected (Fn+Q included).").size(12.5).color(t.text2));
        });
        ui.add_space(8.0);
        if self.rows.is_empty() {
            ui.label(RichText::new("No power limits from the kernel (lenovo-wmi-other, Linux 6.16+).").color(t.text2));
            return;
        }
        card(ui, t, |ui| {
            for (a, v) in self.rows.iter_mut() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&a.label).font(bold(13.5)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("{v}{}", a.unit())).size(13.0).color(t.text2));
                    });
                });
                ui.spacing_mut().slider_width = ui.available_width();
                ui.add(egui::Slider::new(v, a.min..=a.max).step_by(a.step as f64).show_value(false))
                    .on_hover_text(format!("{} · {}..{}{} · firmware now {}", a.name, a.min, a.max, a.unit(), a.value));
                ui.add_space(6.0);
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if primary_button(ui, t, "Apply", Vec2::new(120.0, 32.0)).clicked() {
                app::apply_power_limits(self.rows.clone());
            }
            if secondary_button(ui, t, "Firmware values", Vec2::new(150.0, 32.0)).on_hover_text("Back to what the firmware has now (not saved until Apply)").clicked() {
                self.rows = firmware::attributes().into_iter().map(|a| {
                    let v = a.value;
                    (a, v)
                }).collect();
            }
        });
    }
}
