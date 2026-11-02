//! Tray icon on its own message thread, so it stays responsive while the egui window is hidden.
//! The icon is the L-Helper logo in black or white, following the taskbar theme; left click toggles the window,
//! right click shows quick actions.

use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app;
use crate::gpu;
use crate::lenovo::sensors::Reading;
use crate::lighting;
use crate::logo::{self, Logo};
use crate::mode::{self, Mode};

enum Cmd {
    Icon,
    Tooltip(String),
}

const WM_TRAY_CMD: u32 = WM_APP + 3;

static THREAD: OnceCell<(u32, Mutex<Sender<Cmd>>)> = OnceCell::new();

fn send(cmd: Cmd) {
    if let Some((id, tx)) = THREAD.get() {
        let _ = tx.lock().unwrap().send(cmd);
        unsafe {
            let _ = PostThreadMessageW(*id, WM_TRAY_CMD, WPARAM(0), LPARAM(0));
        }
    }
}

/// Re-checks the taskbar theme (called with every state refresh; the icon only changes when the theme does)
pub fn refresh_icon() {
    send(Cmd::Icon);
}

pub fn update_tooltip(r: &Reading, mode: Option<Mode>) {
    let mut lines = vec![format!("L-Helper · {}", mode.map(|m| m.name()).unwrap_or("-"))];
    let temps: Vec<String> = [r.cpu_temp.map(|t| format!("CPU {t}°C")), r.gpu_temp.map(|t| format!("GPU {t}°C"))].into_iter().flatten().collect();
    if !temps.is_empty() {
        lines.push(temps.join("  "));
    }
    if let Some(p) = r.battery_percent {
        lines.push(format!("Battery {p}%"));
    }
    if let Some(power) = r.power_text() {
        lines.push(power.replace("  ·  ", "  "));
    }
    let mut text = lines.join("\n");
    text.truncate(127);
    send(Cmd::Tooltip(text));
}

/// Tray icon: the monochrome logo for the current taskbar theme, at the small-icon size for the screen's DPI
fn icon(kind: Logo) -> Option<Icon> {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.clamp(16, 64) as u32;
    Icon::from_rgba(logo::rgba(kind, size), size, size).ok()
}

pub fn start(on_toggle: impl Fn() + Send + Sync + 'static, on_quit: impl Fn() + Send + Sync + 'static) {
    let (tx, rx) = channel::<Cmd>();
    let (id_tx, id_rx) = channel();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || unsafe {
            let _ = id_tx.send(GetCurrentThreadId());
            run(rx, on_toggle, on_quit);
        })
        .ok();
    if let Ok(id) = id_rx.recv() {
        let _ = THREAD.set((id, Mutex::new(tx)));
    }
}

unsafe fn run(rx: Receiver<Cmd>, on_toggle: impl Fn() + Send + Sync + 'static, on_quit: impl Fn() + Send + Sync + 'static) {
    let menu = Menu::new();
    let mut actions: HashMap<String, Box<dyn Fn() + Send + Sync>> = HashMap::new();
    let mut add = |menu: &Menu, text: &str, f: Box<dyn Fn() + Send + Sync>| {
        let item = MenuItem::new(text, true, None);
        actions.insert(item.id().0.clone(), f);
        let _ = menu.append(&item);
    };

    for m in mode::available() {
        add(&menu, m.name(), Box::new(move || app::set_mode(m)));
    }
    if gpu::supported() {
        let _ = menu.append(&PredefinedMenuItem::separator());
        for g in gpu::available() {
            add(&menu, &format!("GPU: {}", g.name()), Box::new(move || app::set_gpu(g)));
        }
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    if lighting::is_supported() {
        add(&menu, "Next lighting preset", Box::new(app::cycle_lighting));
    }
    add(&menu, "Game overlay (Ctrl+Shift+O)", Box::new(crate::win::overlay::toggle));
    let on_toggle = std::sync::Arc::new(on_toggle);
    let toggle = on_toggle.clone();
    add(&menu, "Open", Box::new(move || toggle()));
    add(&menu, "Quit", Box::new(on_quit));

    let actions = std::sync::Arc::new(actions);
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if let Some(f) = actions.get(&e.id.0) {
            f();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            on_toggle();
        }
    }));

    let mut shown = logo::for_taskbar();
    let Ok(tray) = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("L-Helper")
        .with_icon(icon(shown).expect("icon"))
        .build()
    else {
        crate::log!("Tray icon failed");
        return;
    };

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if msg.message == WM_TRAY_CMD {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    Cmd::Icon => {
                        let wanted = logo::for_taskbar();
                        if wanted != shown {
                            shown = wanted;
                            let _ = tray.set_icon(icon(wanted));
                            logo::apply_window_icons(crate::app::hwnd());
                        }
                    }
                    Cmd::Tooltip(t) => {
                        let _ = tray.set_tooltip(Some(t));
                    }
                }
            }
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}
