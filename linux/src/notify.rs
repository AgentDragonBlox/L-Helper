//! Desktop notifications (org.freedesktop.Notifications). The OSD ones (Fn+Q, lighting, power source) replace
//! each other instead of piling up, like the Windows popup.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use zbus::zvariant::Value;

static LAST_OSD: AtomicU32 = AtomicU32::new(0);
static CONNECTION: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);

fn send(summary: &str, body: &str, replaces: u32, transient: bool) -> Option<u32> {
    let mut slot = CONNECTION.lock().unwrap();
    if slot.is_none() {
        *slot = zbus::blocking::Connection::session().map_err(|e| crate::log!("Notifications: {e}")).ok();
    }
    let conn = slot.as_ref()?;
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("transient", Value::from(transient));
    hints.insert("desktop-entry", Value::from("lhelper"));
    let actions: Vec<&str> = Vec::new();
    let reply = conn
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("L-Helper", replaces, "lhelper", summary, body, actions, hints, if transient { 2500i32 } else { -1 }),
        )
        .map_err(|e| crate::log!("Notification failed: {e}"));
    let Ok(reply) = reply else {
        *slot = None;
        return None;
    };
    reply.body().deserialize::<u32>().ok()
}

/// Short popup for a setting change; off with the "osd" setting
pub fn osd(text: &str) {
    if !crate::config::is_not_false("osd") {
        return;
    }
    let text = text.to_string();
    std::thread::spawn(move || {
        if let Some(id) = send(&text, "", LAST_OSD.load(Ordering::SeqCst), true) {
            LAST_OSD.store(id, Ordering::SeqCst);
        }
    });
}

/// Error or notice that stays in the notification list
pub fn show(summary: &str, body: &str) {
    let (summary, body) = (summary.to_string(), body.to_string());
    std::thread::spawn(move || {
        send(&summary, &body, 0, false);
    });
}
