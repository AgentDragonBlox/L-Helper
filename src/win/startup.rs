//! Run on startup through a Task Scheduler task with highest privileges (no UAC prompt at logon).
//!
//! A no-prompt admin task is only safe if nobody else can change what it runs, so the task always points at the
//! copy in %ProgramFiles%\LHelper (admin-only), never at a build folder or Downloads. The task XML is written to
//! the admin-only data folder and schtasks is called by its full System32 path.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use super::secure;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn task_name() -> String {
    format!("LHelper_{}", std::env::var("USERNAME").unwrap_or_default())
}

fn schtasks(args: &[&str]) -> bool {
    let exe = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32").join("schtasks.exe");
    Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn is_scheduled() -> bool {
    schtasks(&["/Query", "/TN", &task_name()])
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The exe the task runs: this one if it's already in a protected folder, else a copy in %ProgramFiles%\LHelper
pub fn protected_exe() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    if secure::is_protected_location(&current) {
        return Some(current);
    }
    let dir = secure::install_dir();
    std::fs::create_dir_all(&dir).ok()?;
    secure::lock_down(&dir);
    let target = dir.join("lhelper.exe");
    let newer = |a: &PathBuf, b: &PathBuf| {
        let m = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        m(a) > m(b)
    };
    if !target.exists() || newer(&current, &target) {
        // Fails if the installed copy is running; the existing copy is used then
        match std::fs::copy(&current, &target) {
            Ok(_) => crate::log!("Installed {} -> {}", current.display(), target.display()),
            Err(e) => crate::log!("Couldn't update {}: {e}", target.display()),
        }
    }
    target.exists().then_some(target)
}

pub fn schedule() {
    let Some(exe) = protected_exe() else {
        crate::log!("Startup not scheduled: no protected copy of LHelper");
        return;
    };
    let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>LHelper autostart</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>-autostart</Arguments></Exec></Actions>
</Task>"#,
        user = escape(&user),
        exe = escape(&exe.display().to_string())
    );
    // Admin-only folder: nothing can swap the XML between writing and schtasks reading it
    let file = crate::log::app_dir().join("task.xml");
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    if std::fs::write(&file, bytes).is_ok() {
        let ok = schtasks(&["/Create", "/TN", &task_name(), "/XML", &file.display().to_string(), "/F"]);
        crate::log!("Startup scheduled: {ok} ({})", exe.display());
        let _ = std::fs::remove_file(file);
    }
}

/// An existing task from an older version may point at a writable folder: re-create it on the protected copy
pub fn secure_existing_task() {
    if !is_scheduled() {
        return;
    }
    let out = Command::new(PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("System32").join("schtasks.exe"))
        .args(["/Query", "/TN", &task_name(), "/XML"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return };
    let xml = String::from_utf8_lossy(&out.stdout).to_string();
    let command = xml.split("<Command>").nth(1).and_then(|s| s.split("</Command>").next()).unwrap_or_default();
    if !secure::is_protected_location(std::path::Path::new(command.trim_matches('"'))) {
        crate::log!("Startup task points at a writable folder ({command}): moving it to the protected copy");
        schedule();
    }
}

pub fn unschedule() {
    let ok = schtasks(&["/Delete", "/TN", &task_name(), "/F"]);
    crate::log!("Startup removed: {ok}");
}
