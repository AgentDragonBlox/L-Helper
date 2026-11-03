//! Log in ~/.local/state/lhelper/log.txt, settings in ~/.config/lhelper (the user's own files: hardware writes go
//! through lhelper-helper, which checks every value itself)

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join(fallback))
        .join("lhelper")
}

/// Settings folder (config.json), used by the shared config module
pub fn app_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn log_file() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("log.txt")
}

pub fn write(message: &str) {
    let _guard = LOCK.lock().unwrap();
    let line = format!("{}: {}\n", now(), message);
    #[cfg(debug_assertions)]
    eprint!("{line}");
    let path = log_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
    // Keeps the log from growing forever (checked occasionally)
    if rand::random::<u8>() == 0 {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let lines: Vec<&str> = text.lines().collect();
            if lines.len() > 2000 {
                let _ = std::fs::write(&path, lines[lines.len() - 1000..].join("\n") + "\n");
            }
        }
    }
}

fn now() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}-{:02}-{:04} {:02}:{:02}:{:02}", tm.tm_mday, tm.tm_mon + 1, tm.tm_year + 1900, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
