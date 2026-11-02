//! Global hotkeys on their own message thread: lighting preset cycle (rgb_hotkey) and the game overlay
//! (overlay_hotkey, Ctrl+Shift+O by default)

use once_cell::sync::OnceCell;
use std::sync::Mutex;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_REREGISTER: u32 = WM_APP + 2;
const ID: i32 = 0x4C48;
const OVERLAY_ID: i32 = 0x4C49;
pub const OVERLAY_DEFAULT: &str = "Ctrl+Shift+O";

static THREAD_ID: OnceCell<u32> = OnceCell::new();
static HANDLER: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

/// Parses "Ctrl+Alt+L" style text into (modifiers, virtual key)
pub fn parse(text: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = MOD_NOREPEAT;
    let mut vk = None;
    for part in text.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
            k if k.len() == 1 => vk = k.chars().next().map(|c| c.to_ascii_uppercase() as u32),
            k if k.starts_with('f') => vk = k[1..].parse::<u32>().ok().filter(|n| (1..=24).contains(n)).map(|n| VK_F1.0 as u32 + n - 1),
            _ => return None,
        }
    }
    vk.map(|v| (mods, v))
}

pub fn start(handler: impl Fn() + Send + 'static) {
    *HANDLER.lock().unwrap() = Some(Box::new(handler));
    THREAD_ID.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || unsafe {
                let _ = tx.send(GetCurrentThreadId());
                register();
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    match msg.message {
                        WM_HOTKEY if msg.wParam.0 as i32 == OVERLAY_ID => super::overlay::toggle(),
                        WM_HOTKEY => {
                            if let Some(h) = HANDLER.lock().unwrap().as_ref() {
                                h();
                            }
                        }
                        WM_REREGISTER => register(),
                        _ => {}
                    }
                }
            })
            .ok();
        rx.recv().unwrap_or(0)
    });
}

/// Re-reads rgb_hotkey from config
pub fn reload() {
    if let Some(id) = THREAD_ID.get() {
        unsafe {
            let _ = PostThreadMessageW(*id, WM_REREGISTER, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe fn register() {
    let _ = UnregisterHotKey(None, OVERLAY_ID);
    let overlay = crate::config::get_string("overlay_hotkey").unwrap_or_else(|| OVERLAY_DEFAULT.into());
    if let Some((mods, vk)) = parse(&overlay) {
        let ok = RegisterHotKey(None, OVERLAY_ID, mods, vk).is_ok();
        crate::log!("Overlay hotkey {overlay}: {}", if ok { "registered" } else { "failed (in use by another app)" });
    }

    let _ = UnregisterHotKey(None, ID);
    let Some(text) = crate::config::get_string("rgb_hotkey") else { return };
    match parse(&text) {
        Some((mods, vk)) => {
            let ok = RegisterHotKey(None, ID, mods, vk).is_ok();
            crate::log!("Lighting hotkey {text}: {}", if ok { "registered" } else { "failed" });
        }
        None => crate::log!("Can't parse hotkey {text}"),
    }
}
