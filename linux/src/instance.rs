//! Single instance: the first L-Helper listens on $XDG_RUNTIME_DIR/lhelper.sock; a second one (from the app menu
//! or a keyboard shortcut) passes its command there and exits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

fn socket() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir());
    dir.join("lhelper.sock")
}

/// Sends the command to a running L-Helper; false when none is running
pub fn send(command: &str) -> bool {
    match UnixStream::connect(socket()) {
        Ok(mut s) => s.write_all(format!("{command}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Becomes the running instance; commands arrive on a background thread
pub fn listen(handle: impl Fn(&str) + Send + 'static) {
    let path = socket();
    // A socket left by a crashed instance (send() already found nobody listening)
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            crate::log!("Single instance socket {}: {e}", path.display());
            return;
        }
    };
    std::thread::Builder::new()
        .name("instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() {
                    handle(line.trim());
                }
            }
        })
        .ok();
}
