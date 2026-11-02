//! GPU Tuning window: NVIDIA core / memory clock offsets and undervolt (voltage cap), opened from the GPU section

use egui::{RichText, Vec2, ViewportId};
use std::sync::{Arc, Mutex};

use super::widgets::{bold, card, card_title, primary_button, rgb, secondary_button, toggle_row, Theme};
use crate::app;
use crate::config;
use crate::nvapi;

#[derive(Clone)]
enum State {
    Busy(&'static str),
    Ready(nvapi::Offsets),
    Failed(String),
}

pub struct GpuTuneWindow {
    open: bool,
    state: Arc<Mutex<State>>,
    core: i32,
    memory: i32,
    /// mV, 0 = stock curve
    cap: i32,
}

impl GpuTuneWindow {
    pub fn new(open: bool) -> Self {
        let mut w = GpuTuneWindow { open: false, state: Arc::new(Mutex::new(State::Busy(""))), core: 0, memory: 0, cap: 0 };
        if open {
            w.open();
        }
        w
    }

    pub fn id() -> ViewportId {
        ViewportId::from_hash_of("gpu_tune")
    }

    /// Reading the limits wakes the dGPU, which is why it only happens when this window opens
    pub fn open(&mut self) {
        if self.open {
            super::bring_to_front(Self::id());
            return;
        }
        self.open = true;
        (self.core, self.memory, self.cap) = nvapi::saved();
        self.run("Reading the GPU's limits…", || {
            if !nvapi::firmware_allows() {
                return Err("The BIOS doesn't allow GPU overclocking on this laptop".into());
            }
            nvapi::read()
        });
        super::child_opened();
    }

    fn run(&self, what: &'static str, f: impl FnOnce() -> Result<nvapi::Offsets, String> + Send + 'static) {
        *self.state.lock().unwrap() = State::Busy(what);
        let slot = self.state.clone();
        std::thread::spawn(move || {
            *slot.lock().unwrap() = match f() {
                Ok(o) => State::Ready(o),
                Err(e) => State::Failed(e),
            };
            app::repaint();
        });
    }

    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.open {
            return;
        }
        let t = *theme;
        ctx.show_viewport_immediate(Self::id(), super::child_builder("GPU Tuning", [460.0, 500.0]), |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(16).fill(t.bg)).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.contents(ui, &t));
            });
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let state = self.state.lock().unwrap().clone();
        let offsets = match state {
            State::Busy(what) => {
                ui.label(RichText::new(what).color(t.text2));
                return;
            }
            State::Failed(e) => {
                ui.label(RichText::new(e).color(rgb(app::COLOR_PERFORMANCE)));
                if secondary_button(ui, t, "Try again", Vec2::new(120.0, 28.0)).clicked() {
                    self.open = false;
                    self.open();
                }
                return;
            }
            State::Ready(o) => o,
        };

        let slider = |ui: &mut egui::Ui, name: &str, shown: String, active: Option<String>, value: &mut i32, range: std::ops::RangeInclusive<i32>, step: f64| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).size(13.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(shown).font(bold(13.5)));
                    if let Some(a) = active {
                        ui.label(RichText::new(format!("active {a}")).size(12.0).color(t.text2));
                    }
                });
            });
            ui.spacing_mut().slider_width = ui.available_width() - 8.0;
            ui.add(egui::Slider::new(value, range).step_by(step).show_value(false));
        };
        let offset_text = |v: i32| if v == 0 { "Stock".to_string() } else { format!("{v:+} MHz") };

        let (core, memory) = (offsets.core, offsets.memory);
        card(ui, t, |ui| {
            card_title(ui, t, "Clocks", "");
            if core.editable && core.min < core.max {
                let active = (core.current != self.core).then(|| offset_text(core.current));
                slider(ui, "Core offset", offset_text(self.core), active, &mut self.core, core.min.max(-500)..=core.max.min(1000), 5.0);
            }
            if memory.editable && memory.min < memory.max {
                let active = (memory.current != self.memory).then(|| offset_text(memory.current));
                slider(ui, "Memory offset", offset_text(self.memory), active, &mut self.memory, memory.min.max(-1000)..=memory.max.min(3000), 10.0);
            }
        });

        card(ui, t, |ui| {
            card_title(ui, t, "Undervolt", "");
            match offsets.undervolt {
                Some(u) => {
                    // The slider's top end is "off"; anything lower caps the voltage (curve flattened there)
                    let min = u.min.max(650);
                    if self.cap != 0 && self.cap < min {
                        self.cap = min;
                    }
                    let mut value = if self.cap == 0 { u.max } else { self.cap };
                    let shown = if self.cap == 0 { "Off".to_string() } else { format!("{} mV", self.cap) };
                    let active = (u.cap != self.cap).then(|| if u.cap == 0 { "off".to_string() } else { format!("{} mV", u.cap) });
                    slider(ui, "Maximum voltage", shown, active, &mut value, min..=u.max, 5.0);
                    self.cap = if value >= u.max { 0 } else { value };
                    ui.label(
                        RichText::new("The GPU stops boosting above this voltage: cooler and quieter. Raise the core offset to keep more clock at it; 850–950 mV with +50…+150 MHz is typical.")
                            .size(12.0)
                            .color(t.text2),
                    );
                }
                None => {
                    ui.label(RichText::new("Not available: this GPU or driver doesn't expose its voltage curve.").size(12.0).color(t.text2));
                }
            }
        });

        card(ui, t, |ui| {
            let mut at_startup = config::is("gpu_oc");
            if toggle_row(ui, t, "Apply at startup", "The NVIDIA driver forgets these when Windows restarts or wakes", &mut at_startup).changed() {
                config::set("gpu_oc", at_startup as i64);
            }
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Small steps, one at a time. Artifacts or crashes: reset.").size(12.0).color(t.text2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_button(ui, t, "Apply", Vec2::new(96.0, 32.0)).clicked() {
                    let (core, memory, cap) = (self.core, self.memory, self.cap);
                    config::set("gpu_oc_core", core as i64);
                    config::set("gpu_oc_memory", memory as i64);
                    config::set("gpu_oc_cap", cap as i64);
                    self.run("Applying…", move || nvapi::apply(core, memory, cap));
                }
                if secondary_button(ui, t, "Reset", Vec2::new(84.0, 32.0)).clicked() {
                    (self.core, self.memory, self.cap) = (0, 0, 0);
                    for key in ["gpu_oc_core", "gpu_oc_memory", "gpu_oc_cap"] {
                        config::set(key, 0);
                    }
                    self.run("Resetting…", || nvapi::apply(0, 0, 0));
                }
            });
        });
    }
}
