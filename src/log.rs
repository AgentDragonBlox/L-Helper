//! Append-only log in %ProgramData%\LHelper\log.txt (admin-only writable, see win/secure.rs)

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Lines not written yet because the log file couldn't be opened (e.g. locked by another process)
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// %ProgramData%\LHelper: settings and log, writable by administrators only
pub fn app_dir() -> PathBuf {
    std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData")).join("LHelper")
}

/// Creates the data folder with its admin-only ACL, moving settings and log over from %AppData%\LHelper
/// (where versions before the hardening kept them) the first time. Call before any logging or config access.
pub fn init_data_dir() {
    let dir = app_dir();
    let fresh = !dir.exists();
    let _ = std::fs::create_dir_all(&dir);
    if fresh {
        if let Some(old) = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("LHelper")) {
            for name in ["config.json", "log.txt"] {
                let _ = std::fs::copy(old.join(name), dir.join(name));
            }
        }
    }
    crate::win::secure::lock_down_tree(&dir);
}

pub fn log_file() -> PathBuf {
    app_dir().join("log.txt")
}

pub fn write(message: &str) {
    let mut pending = PENDING.lock().unwrap();
    let line = format!("{}: {}\n", now(), message);
    #[cfg(debug_assertions)]
    eprint!("{line}");
    pending.push(line);
    let _ = std::fs::create_dir_all(app_dir());
    match OpenOptions::new().create(true).append(true).open(log_file()) {
        Ok(mut f) => {
            if f.write_all(pending.concat().as_bytes()).is_ok() {
                pending.clear();
            }
        }
        Err(_) if pending.len() > 500 => {
            pending.remove(0);
        }
        Err(_) => {}
    }
    drop(pending);
    trim();
}

/// Keeps the log from growing forever (checked occasionally)
fn trim() {
    if rand::random::<u8>() != 0 {
        return;
    }
    if let Ok(text) = std::fs::read_to_string(log_file()) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > 2000 {
            let _ = std::fs::write(log_file(), lines[lines.len() - 1000..].join("\n") + "\n");
        }
    }
}

fn now() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!("{:02}-{:02}-{:04} {:02}:{:02}:{:02}", t.wDay, t.wMonth, t.wYear, t.wHour, t.wMinute, t.wSecond)
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
