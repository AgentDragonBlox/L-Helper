//! Linux hardware layer: kernel driver files in /sys. Reads happen directly; writes need root and go through
//! lhelper-helper (src/bin/lhelper-helper.rs), started with pkexec. The polkit policy lets the signed-in user
//! run it without a password, and the helper only accepts the settings and ranges the drivers advertise.

pub mod battery;
pub mod display;
pub mod firmware;
pub mod profile;
pub mod sensors;

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read_i64(path: impl AsRef<Path>) -> Option<i64> {
    read(path)?.parse().ok()
}

/// Sub-directories of a /sys folder (devices are symlinks to directories)
pub fn dirs(path: impl AsRef<Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(path).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    out.sort();
    out
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

const INSTALLED_HELPER: &str = "/usr/libexec/lhelper/lhelper-helper";

/// The packaged helper, or the one next to this exe (cargo build); pkexec needs an absolute path
fn helper() -> PathBuf {
    if Path::new(INSTALLED_HELPER).exists() {
        return INSTALLED_HELPER.into();
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("lhelper-helper"))).unwrap_or_else(|| INSTALLED_HELPER.into())
}

/// Runs one lhelper-helper command as root
pub fn privileged(args: &[&str]) -> Result<(), String> {
    let helper = helper();
    let mut cmd = if is_root() {
        Command::new(&helper)
    } else {
        let mut c = Command::new("pkexec");
        c.arg(&helper);
        c
    };
    let output = cmd.args(args).output().map_err(|e| format!("Can't start {}: {e}", if is_root() { helper.display().to_string() } else { "pkexec".into() }))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    crate::log!("lhelper-helper {}: exit {:?}: {stderr}", args.join(" "), output.status.code());
    Err(match output.status.code() {
        // pkexec: dismissed or not authorized / helper missing
        Some(126) => "Not authorized to change hardware settings".into(),
        Some(127) if !Path::new(INSTALLED_HELPER).exists() => "lhelper-helper isn't installed (install the package)".into(),
        _ if !stderr.is_empty() => stderr.lines().last().unwrap_or_default().to_string(),
        _ => "Hardware setting failed".into(),
    })
}
