//! Tray icon (StatusNotifierItem, through ksni): KDE Plasma natively, GNOME with the AppIndicator extension
//! (on by default in Ubuntu). Left click opens the window; the menu has the modes, next lighting preset and Quit.

use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};

use ksni::blocking::TrayMethods;
use ksni::menu::{RadioGroup, RadioItem, StandardItem};
use ksni::MenuItem;

use crate::app;
use crate::hw::profile::Mode;
use crate::lighting;

static HANDLE: OnceCell<ksni::blocking::Handle<Tray>> = OnceCell::new();
static AVAILABLE: AtomicBool = AtomicBool::new(false);

struct Tray;

/// RGBA -> ARGB32 in network byte order, as StatusNotifierItem wants
fn pixmap(size: u32) -> ksni::Icon {
    let rgba = crate::logo::rgba(crate::logo::Logo::Blue, size);
    let data = rgba.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
    ksni::Icon { width: size as i32, height: size as i32, data }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "lhelper".into()
    }

    fn title(&self) -> String {
        "L-Helper".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![pixmap(22), pixmap(32), pixmap(48)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let s = app::state().clone();
        let r = &s.sensors;
        let mut lines = vec![format!("Mode: {}", s.mode.map(|m| m.name()).unwrap_or("-"))];
        if let Some(c) = r.cpu_temp {
            lines.push(format!("CPU: {c}°C"));
        }
        match (r.gpu_asleep, r.gpu_temp) {
            (true, _) if crate::hw::sensors::nvidia_present() => lines.push("GPU: asleep".into()),
            (_, Some(c)) => lines.push(format!("GPU: {c}°C")),
            _ => {}
        }
        if let Some(p) = r.battery_percent {
            lines.push(format!("Battery: {p}%"));
        }
        ksni::ToolTip { title: "L-Helper".into(), description: lines.join("\n"), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        crate::ui::request_open();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = app::state().clone();
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem { label: "Open L-Helper".into(), activate: Box::new(|_| crate::ui::request_open()), ..Default::default() }.into(),
            MenuItem::Separator,
        ];
        if !s.modes.is_empty() {
            let modes: Vec<Mode> = s.modes.clone();
            let selected = s.mode.and_then(|m| modes.iter().position(|x| *x == m)).unwrap_or(usize::MAX);
            items.push(
                RadioGroup {
                    selected,
                    select: Box::new(move |_, i| {
                        if let Some(m) = modes.get(i) {
                            app::set_mode(*m);
                        }
                    }),
                    options: s.modes.iter().map(|m| RadioItem { label: m.name().into(), ..Default::default() }).collect(),
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }
        if lighting::is_supported() {
            items.push(StandardItem { label: "Next lighting preset".into(), activate: Box::new(|_| app::cycle_lighting()), ..Default::default() }.into());
            items.push(MenuItem::Separator);
        }
        items.push(StandardItem { label: "Quit".into(), icon_name: "application-exit".into(), activate: Box::new(|_| app::quit()), ..Default::default() }.into());
        items
    }

    fn watcher_online(&self) {
        AVAILABLE.store(true, Ordering::SeqCst);
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        crate::log!("Tray host gone: {reason:?}");
        AVAILABLE.store(false, Ordering::SeqCst);
        true
    }
}

/// `wait`: at login the tray host may not be up yet; keep the icon registered until it is
pub fn start(wait: bool) {
    // Set first: a host that's missing at start reports through watcher_offline during spawn
    AVAILABLE.store(true, Ordering::SeqCst);
    match Tray.assume_sni_available(wait).spawn() {
        Ok(handle) => {
            let _ = HANDLE.set(handle);
            crate::log!("Tray icon registered (host {})", if available() { "online" } else { "not running yet" });
        }
        Err(e) => {
            AVAILABLE.store(false, Ordering::SeqCst);
            crate::log!("No tray icon ({e}): on GNOME, turn on the AppIndicator extension");
        }
    }
}

/// A tray host is showing the icon, so closing the window can leave L-Helper running
pub fn available() -> bool {
    AVAILABLE.load(Ordering::SeqCst)
}

/// Re-reads the menu and tooltip
pub fn refresh() {
    if let Some(h) = HANDLE.get() {
        h.update(|_| {});
    }
}
