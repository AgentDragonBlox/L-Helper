//! Settings window: key / USB switches, app behaviour and a diagnostics card (what the kernel offers this laptop)

use egui::{RichText, Vec2, ViewportId};

use super::widgets::{card, card_title, divider, secondary_button, toggle_row, Theme};
use crate::app;
use crate::config;
use crate::hw::battery::Switch;
use crate::hw::read;

pub struct SettingsWindow {
    open: bool,
    diagnostics: Vec<(String, String)>,
}

impl SettingsWindow {
    pub fn new() -> Self {
        SettingsWindow { open: false, diagnostics: Vec::new() }
    }

    pub fn id() -> ViewportId {
        ViewportId::from_hash_of("settings")
    }

    pub fn open(&mut self) {
        if self.open {
            super::bring_to_front(Self::id());
            return;
        }
        self.diagnostics = diagnostics();
        self.open = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, theme: &Theme) {
        if !self.open {
            return;
        }
        let t = *theme;
        ctx.show_viewport_immediate(Self::id(), super::child_builder("Settings", [500.0, 680.0]), |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.open = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::central_panel(&ui.ctx().global_style()).inner_margin(16).fill(t.bg)).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.contents(ui, &t));
            });
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui, t: &Theme) {
        let s = app::state().clone();

        if s.fn_lock.is_some() || s.usb_charging.is_some() {
            card(ui, t, |ui| {
                card_title(ui, t, "Keys and USB", "");
                if let Some(mut v) = s.fn_lock {
                    if toggle_row(ui, t, "Fn lock", "F1-F12 without holding Fn", &mut v).changed() {
                        app::set_switch(Switch::FnLock, v);
                    }
                }
                if let Some(mut v) = s.usb_charging {
                    divider(ui, t);
                    if toggle_row(ui, t, "Always-on USB", "Charge devices from USB while the laptop sleeps or is off", &mut v).changed() {
                        app::set_switch(Switch::UsbCharging, v);
                    }
                }
            });
            ui.add_space(10.0);
        }

        card(ui, t, |ui| {
            card_title(ui, t, "L-Helper", "");
            let mut tray = config::is_not_false("minimize_to_tray");
            if toggle_row(ui, t, "Keep running in the tray", "Closing the window leaves L-Helper in the tray (lighting effects keep running)", &mut tray).changed() {
                config::set("minimize_to_tray", tray as i64);
            }
            divider(ui, t);
            let mut osd = config::is_not_false("osd");
            if toggle_row(ui, t, "Popups", "Show a notification when Fn+Q or the power source changes the mode", &mut osd).changed() {
                config::set("osd", osd as i64);
            }
            divider(ui, t);
            let mut auto = config::is_not_false("auto_mode");
            if toggle_row(ui, t, "Mode per power source", "Use the mode you last chose on battery / plugged in", &mut auto).changed() {
                config::set("auto_mode", auto as i64);
            }
            if !crate::tray::available() {
                ui.add_space(4.0);
                ui.label(RichText::new("No tray icon: on GNOME, turn on the AppIndicator extension (Ubuntu has it on by default).").size(12.0).color(t.text2));
            }
        });
        ui.add_space(10.0);

        card(ui, t, |ui| {
            card_title(ui, t, "This laptop", "");
            egui::Grid::new("diagnostics").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for (k, v) in &self.diagnostics {
                    ui.label(RichText::new(k).size(12.5).color(t.text2));
                    ui.label(RichText::new(v).size(12.5));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if secondary_button(ui, t, "Open log folder", Vec2::new(150.0, 30.0)).clicked() {
                    if let Some(dir) = crate::log::log_file().parent() {
                        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                    }
                }
                if secondary_button(ui, t, "Refresh", Vec2::new(90.0, 30.0)).clicked() {
                    self.diagnostics = diagnostics();
                }
            });
        });
    }
}

fn loaded(module: &str) -> bool {
    std::path::Path::new("/sys/module").join(module).exists()
}

/// Model, kernel and which Lenovo drivers are present: what a bug report needs
pub fn diagnostics() -> Vec<(String, String)> {
    let dmi = |f: &str| read(format!("/sys/class/dmi/id/{f}")).unwrap_or_default();
    let drivers: Vec<&str> = ["lenovo_wmi_gamezone", "lenovo_wmi_other", "ideapad_laptop", "legion_laptop", "nvidia"].into_iter().filter(|m| loaded(m)).collect();
    vec![
        ("Model".into(), format!("{} ({})", dmi("product_version"), dmi("product_name"))),
        ("BIOS".into(), dmi("bios_version")),
        ("Kernel".into(), read("/proc/sys/kernel/osrelease").unwrap_or_default()),
        ("Profiles".into(), read("/sys/firmware/acpi/platform_profile_choices").unwrap_or_else(|| "none".into())),
        ("Drivers".into(), if drivers.is_empty() { "none".into() } else { drivers.join(", ") }),
        ("Power limits".into(), firmware_summary()),
        ("Keyboard".into(), keyboard_summary()),
        ("Desktop".into(), format!("{} ({})", std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(), std::env::var("XDG_SESSION_TYPE").unwrap_or_default())),
        ("Tray".into(), if crate::tray::available() { "shown".into() } else { "no tray host".into() }),
        ("Version".into(), env!("CARGO_PKG_VERSION").into()),
    ]
}

fn firmware_summary() -> String {
    let attrs = crate::hw::firmware::attributes();
    if attrs.is_empty() { "none".into() } else { attrs.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ") }
}

fn keyboard_summary() -> String {
    if crate::lighting::is_supported() {
        "24-zone RGB".into()
    } else if crate::lighting::hidraw::permission_denied() {
        "24-zone RGB (no access: udev rule missing)".into()
    } else if crate::hw::battery::keyboard_light().is_some() {
        "white backlight".into()
    } else {
        "not found".into()
    }
}
