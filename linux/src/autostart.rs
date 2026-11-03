//! "Run at login": an XDG autostart entry (~/.config/autostart/lhelper.desktop) that starts L-Helper in the tray

use std::path::PathBuf;

fn entry() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    config.join("autostart").join("lhelper.desktop")
}

pub fn enabled() -> bool {
    std::fs::read_to_string(entry()).is_ok_and(|s| !s.contains("Hidden=true"))
}

pub fn set(on: bool) -> Result<(), String> {
    let path = entry();
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().ok().filter(|e| e.starts_with("/usr")).map(|e| e.display().to_string()).unwrap_or_else(|| "lhelper".into());
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=L-Helper\nComment=Lenovo laptop control in the tray\nExec={exe} --tray\nIcon=lhelper\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())
}
