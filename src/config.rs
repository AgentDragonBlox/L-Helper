//! Flat key/value JSON config (same keys as the C# version) in %ProgramData%\LHelper\config.json, which only
//! administrators can write (win/secure.rs). Writes are debounced on a background thread.

use once_cell::sync::Lazy;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

struct Store {
    path: PathBuf,
    values: Map<String, Value>,
    /// False while config.json exists but can't be read (locked or access denied). Saving then would replace the
    /// user's settings with defaults, so writes wait until it can be read and merged.
    readable: bool,
}

/// Settings on disk; None when the file exists but can't be read yet
fn load(path: &Path) -> Option<Map<String, Value>> {
    // Unparseable content counts as empty (it gets rewritten); unreadable doesn't
    let read = |p: &Path| std::fs::read_to_string(p).ok().map(|s| serde_json::from_str::<Map<String, Value>>(&s).unwrap_or_default());
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    // A newer config.json.tmp means the last save couldn't replace config.json: recover from it
    let tmp = path.with_extension("json.tmp");
    if modified(&tmp) > modified(path) {
        if let Some(v) = read(&tmp).filter(|v| !v.is_empty()) {
            crate::log!("Recovered settings from {}", tmp.display());
            return Some(v);
        }
    }
    if !path.exists() {
        return Some(Map::new());
    }
    read(path)
}

static STORE: Lazy<Mutex<Store>> = Lazy::new(|| {
    // Only the admin-only data folder: a config next to the exe could be edited by any program
    let path = crate::log::app_dir().join("config.json");
    let loaded = load(&path);
    let readable = loaded.is_some();
    if !readable {
        // Keep retrying in the background (flush reloads first)
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(1000));
            mark_dirty();
        });
    }
    Mutex::new(Store { path, values: loaded.unwrap_or_default(), readable })
});

static DIRTY: AtomicBool = AtomicBool::new(false);
static WRITER: Lazy<()> = Lazy::new(|| {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(1000));
        if DIRTY.swap(false, Ordering::SeqCst) {
            flush();
        }
    });
});

pub fn flush() {
    let (path, json) = {
        let mut store = STORE.lock().unwrap();
        if !store.readable {
            let Some(mut saved) = load(&store.path) else {
                DIRTY.store(true, Ordering::SeqCst); // still locked: try again later
                return;
            };
            // Settings changed this session win over the saved ones
            for (k, v) in std::mem::take(&mut store.values) {
                saved.insert(k, v);
            }
            store.values = saved;
            store.readable = true;
            crate::log!("Settings file readable again, reloaded");
        }
        (store.path.clone(), serde_json::to_string_pretty(&store.values).unwrap_or_default())
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write-then-rename keeps the old file intact if we crash mid-write. Replacing can fail (e.g. config.json
    // encrypted with EFS), so fall back to writing the file in place rather than silently losing settings.
    let tmp = path.with_extension("json.tmp");
    let replaced = std::fs::write(&tmp, &json).is_ok() && std::fs::rename(&tmp, &path).is_ok();
    if !replaced {
        match std::fs::write(&path, &json) {
            Ok(_) => {
                let _ = std::fs::remove_file(&tmp);
            }
            Err(e) => {
                crate::log!("Can't save settings to {}: {e}", path.display());
                DIRTY.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn mark_dirty() {
    Lazy::force(&WRITER);
    DIRTY.store(true, Ordering::SeqCst);
}

pub fn exists(name: &str) -> bool {
    STORE.lock().unwrap().values.contains_key(name)
}

/// Integer value or `empty`
pub fn get(name: &str, empty: i64) -> i64 {
    let store = STORE.lock().unwrap();
    match store.values.get(name) {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(empty),
        Some(Value::String(s)) => s.parse().unwrap_or(empty),
        Some(Value::Bool(b)) => *b as i64,
        _ => empty,
    }
}

pub fn is(name: &str) -> bool {
    get(name, -1) == 1
}

pub fn is_not_false(name: &str) -> bool {
    get(name, -1) != 0
}

pub fn get_string(name: &str) -> Option<String> {
    let store = STORE.lock().unwrap();
    match store.values.get(name) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

pub fn get_array(name: &str) -> Option<Vec<i32>> {
    let s = get_string(name)?;
    s.split(',').map(|x| x.trim().parse::<i32>().ok()).collect()
}

pub fn set(name: &str, value: i64) {
    STORE.lock().unwrap().values.insert(name.to_string(), Value::from(value));
    mark_dirty();
}

pub fn set_string(name: &str, value: &str) {
    STORE.lock().unwrap().values.insert(name.to_string(), Value::from(value));
    mark_dirty();
}

pub fn set_array(name: &str, values: &[i32]) {
    let s = values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",");
    set_string(name, &s);
}

pub fn remove(name: &str) {
    STORE.lock().unwrap().values.remove(name);
    mark_dirty();
}
