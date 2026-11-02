//! Disables Lenovo software that fights over the same firmware settings, the way LLT does:
//! services set to Disabled, scheduled tasks disabled, Run entries marked disabled in StartupApproved, processes killed.
//! Everything is reversible.

use windows::core::{BSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Services::*;
use windows::Win32::System::TaskScheduler::*;
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_TERMINATE, PROCESS_SYNCHRONIZE};
use windows::Win32::System::Variant::VARIANT;
use winreg::enums::*;
use winreg::RegKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotFound,
    /// Running
    Enabled,
    /// Installed, not running, would start again
    Stopped,
    /// Services set to Disabled
    Disabled,
}

pub struct Disabler {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub task_folders: &'static [&'static str],
    pub services: &'static [&'static str],
    pub processes: &'static [&'static str],
    pub startup_names: &'static [&'static str],
    /// Run entries whose exe lives under these folders (relative to Program Files / ProgramData)
    pub startup_roots: &'static [&'static str],
    pub uwp_startup: Option<(&'static str, &'static str)>,
}

pub static VANTAGE: Disabler = Disabler {
    key: "vantage",
    name: "Lenovo Vantage + ImController",
    description: "Vantage app, Vantage service and the ImController plugin host",
    task_folders: &["Lenovo\\BatteryGauge", "Lenovo\\ImController", "Lenovo\\ImController\\Plugins", "Lenovo\\ImController\\TimeBasedEvents", "Lenovo\\UDC", "Lenovo\\Vantage", "Lenovo\\Vantage\\Schedule"],
    services: &["ImControllerService", "LenovoVantageService"],
    processes: &["BGHelper", "Lenovo.Modern.ImController", "Lenovo.Vantage", "LenovoVantage", "QSHelper", "ScheduleEventAction"],
    startup_names: &["LenovoVantageToolbar", "LenovoVantage"],
    startup_roots: &["%ProgramData%\\Lenovo\\Vantage"],
    uwp_startup: None,
};

pub static LEGION_ZONE: Disabler = Disabler {
    key: "legionzone",
    name: "Legion Zone",
    description: "Legion Zone and its background services",
    task_folders: &[],
    services: &["LZService", "StreamingService"],
    processes: &["BorderlessSpace", "ContextMenuInstaller", "DoudouAI", "LegionZone", "LZAgent", "LZInstall", "LZMain", "lzolhelp64", "LZService", "LZStrategy", "LZTray", "LZUpdate", "NvOcScanner", "StreamingDiagnosis", "StreamingHost", "StreamingService"],
    startup_names: &[],
    startup_roots: &["%ProgramFiles(x86)%\\Lenovo\\LegionZone", "%ProgramFiles%\\Lenovo\\LegionZone"],
    uwp_startup: None,
};

pub static LEGION_SPACE: Disabler = Disabler {
    key: "legionspace",
    name: "Legion Space",
    description: "Legion Space and its DAService",
    task_folders: &[],
    services: &["DAService"],
    processes: &["Bino3D", "LegionGameWidget", "LegionSpace", "LegionSpaceComponent", "LegionSpaceToast", "LSDaemon"],
    startup_names: &[],
    startup_roots: &[],
    uwp_startup: None,
};

pub static FN_KEYS: Disabler = Disabler {
    key: "fnkeys",
    name: "Lenovo Hotkeys (Fn keys)",
    description: "Lenovo Fn and function keys service. L-Helper handles Fn keys itself when this is disabled",
    task_folders: &[],
    services: &["LenovoFnAndFunctionKeys"],
    processes: &["LenovoUtilityUI", "LenovoUtilityService", "LenovoSmartKey"],
    startup_names: &[],
    startup_roots: &[],
    uwp_startup: Some(("LenovoUtility", "LenovoUtilityID")),
};

pub static ALL: [&Disabler; 4] = [&VANTAGE, &LEGION_ZONE, &LEGION_SPACE, &FN_KEYS];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn expand(path: &str) -> String {
    let mut out = path.to_string();
    for var in ["ProgramData", "ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(v) = std::env::var_os(var) {
            out = out.replace(&format!("%{var}%"), &v.to_string_lossy());
        }
    }
    out
}

/// (exists, running, start type disabled)
fn service_state(name: &str) -> Option<(bool, bool)> {
    unsafe {
        let manager = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).ok()?;
        let w = wide(name);
        let result = match OpenServiceW(manager, PCWSTR(w.as_ptr()), SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG) {
            Ok(h) => {
                let mut status = SERVICE_STATUS::default();
                let running = QueryServiceStatus(h, &mut status).is_ok() && status.dwCurrentState != SERVICE_STOPPED;
                let mut needed = 0u32;
                let _ = QueryServiceConfigW(h, None, 0, &mut needed);
                let mut buffer = vec![0u8; needed.max(64) as usize];
                let config = buffer.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
                let disabled = QueryServiceConfigW(h, Some(config), buffer.len() as u32, &mut needed).is_ok() && (*config).dwStartType == SERVICE_DISABLED;
                let _ = CloseServiceHandle(h);
                Some((running, disabled))
            }
            Err(_) => None,
        };
        let _ = CloseServiceHandle(manager);
        result
    }
}

fn set_service(name: &str, enabled: bool) -> Result<(), String> {
    unsafe {
        let manager = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS).map_err(|e| e.to_string())?;
        let w = wide(name);
        let h = match OpenServiceW(manager, PCWSTR(w.as_ptr()), SERVICE_CHANGE_CONFIG | SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS) {
            Ok(h) => h,
            Err(_) => {
                let _ = CloseServiceHandle(manager);
                return Ok(()); // not installed
            }
        };
        let start = if enabled { SERVICE_AUTO_START } else { SERVICE_DISABLED };
        let r = ChangeServiceConfigW(h, ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE), start, SERVICE_ERROR(SERVICE_NO_CHANGE), PCWSTR::null(), PCWSTR::null(), None, PCWSTR::null(), PCWSTR::null(), PCWSTR::null(), PCWSTR::null());
        if enabled {
            let _ = StartServiceW(h, None);
        } else {
            let mut status = SERVICE_STATUS::default();
            let _ = ControlService(h, SERVICE_CONTROL_STOP, &mut status);
            for _ in 0..30 {
                if QueryServiceStatus(h, &mut status).is_ok() && status.dwCurrentState == SERVICE_STOPPED {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        let _ = CloseServiceHandle(h);
        let _ = CloseServiceHandle(manager);
        crate::log!("  Service {name}: {}", if enabled { "auto" } else { "disabled" });
        r.map_err(|e| format!("ChangeServiceConfig {name}: {e}"))
    }
}

/// (pid, exe name without .exe)
fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            out.push((entry.th32ProcessID, name.trim_end_matches(".exe").trim_end_matches(".EXE").to_string()));
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

impl Disabler {
    pub fn config_key(&self) -> String {
        format!("disable_{}", self.key)
    }

    fn running_processes(&self) -> Vec<(u32, String)> {
        processes()
            .into_iter()
            .filter(|(_, n)| self.processes.iter().any(|p| n.to_lowercase().starts_with(&p.to_lowercase())))
            .collect()
    }

    pub fn status(&self) -> Status {
        let states: Vec<(bool, bool)> = self.services.iter().filter_map(|s| service_state(s)).collect();
        if states.iter().any(|(running, _)| *running) || !self.running_processes().is_empty() {
            return Status::Enabled;
        }
        if states.is_empty() {
            return Status::NotFound;
        }
        if states.iter().all(|(_, disabled)| *disabled) {
            Status::Disabled
        } else {
            Status::Stopped
        }
    }

    pub fn disable(&self) -> Result<(), String> {
        self.apply(false)
    }

    pub fn enable(&self) -> Result<(), String> {
        self.apply(true)
    }

    fn apply(&self, enabled: bool) -> Result<(), String> {
        crate::log!("{} {}", if enabled { "Enabling" } else { "Disabling" }, self.name);
        let mut errors = Vec::new();
        for folder in self.task_folders {
            if let Err(e) = set_tasks(folder, enabled) {
                errors.push(format!("tasks {folder}: {e}"));
            }
        }
        for s in self.services {
            if let Err(e) = set_service(s, enabled) {
                errors.push(e);
            }
        }
        if !enabled {
            for (pid, name) in self.running_processes() {
                unsafe {
                    if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) {
                        let _ = TerminateProcess(h, 1);
                        let _ = WaitForSingleObject(h, 5000);
                        let _ = CloseHandle(h);
                        crate::log!("  Killed {name}");
                    }
                }
            }
        }
        if let Err(e) = self.set_startup_entries(enabled) {
            errors.push(e);
        }
        if let Some((pattern, sub)) = self.uwp_startup {
            set_uwp_startup(pattern, sub, enabled);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Marks matching Run entries enabled / disabled in StartupApproved (what Task Manager's Startup tab does)
    fn set_startup_entries(&self, enabled: bool) -> Result<(), String> {
        if self.startup_names.is_empty() && self.startup_roots.is_empty() {
            return Ok(());
        }
        let roots: Vec<String> = self.startup_roots.iter().map(|r| expand(r).to_lowercase().trim_end_matches('\\').to_string() + "\\").collect();
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let root = RegKey::predef(hive);
            let Ok(run) = root.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run") else { continue };
            for (name, value) in run.enum_values().flatten() {
                let command = value.to_string().trim_matches('"').to_lowercase();
                let matches = self.startup_names.iter().any(|n| n.eq_ignore_ascii_case(&name)) || roots.iter().any(|r| command.starts_with(r.as_str()));
                if !matches {
                    continue;
                }
                let (approved, _) = root
                    .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run")
                    .map_err(|e| e.to_string())?;
                let mut bytes = vec![if enabled { 0x02 } else { 0x03 }, 0, 0, 0];
                bytes.extend_from_slice(&[0u8; 8]);
                let _ = approved.set_raw_value(&name, &winreg::RegValue { bytes: bytes.into(), vtype: REG_BINARY });
                crate::log!("  Startup entry {name}: {}", if enabled { "enabled" } else { "disabled" });
            }
        }
        Ok(())
    }
}

/// Scheduled tasks through the Task Scheduler COM API
fn set_tasks(folder: &str, enabled: bool) -> Result<(), String> {
    unsafe {
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        service.Connect(&VARIANT::default(), &VARIANT::default(), &VARIANT::default(), &VARIANT::default()).map_err(|e| e.to_string())?;
        let Ok(folder) = service.GetFolder(&BSTR::from(format!("\\{folder}"))) else {
            return Ok(()); // not installed
        };
        let tasks = folder.GetTasks(1).map_err(|e| e.to_string())?; // TASK_ENUM_HIDDEN
        let count = tasks.Count().map_err(|e| e.to_string())?;
        for i in 1..=count {
            let mut index = VARIANT::default();
            (*index.Anonymous.Anonymous).vt = windows::Win32::System::Variant::VT_I4;
            (*index.Anonymous.Anonymous).Anonymous.lVal = i;
            if let Ok(task) = tasks.get_Item(&index) {
                let _ = task.SetEnabled(if enabled { VARIANT_TRUE } else { VARIANT_FALSE });
                if let Ok(path) = task.Path() {
                    crate::log!("  Task {path}: {}", if enabled { "enabled" } else { "disabled" });
                }
            }
        }
    }
    Ok(())
}

/// Store apps: SystemAppData\<package>\<id>\State = 2 enabled / 1 disabled
fn set_uwp_startup(pattern: &str, sub: &str, enabled: bool) {
    let root = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData";
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(root) else { return };
    let Some(package) = key.enum_keys().flatten().find(|k| k.to_lowercase().contains(&pattern.to_lowercase())) else { return };
    if let Ok((app, _)) = hkcu.create_subkey(format!("{root}\\{package}\\{sub}")) {
        let _ = app.set_value("State", &(if enabled { 2u32 } else { 1u32 }));
    }
}

pub fn running() -> Vec<&'static Disabler> {
    ALL.iter().copied().filter(|d| d.status() == Status::Enabled).collect()
}

/// "Keep disabled": re-disable anything a Lenovo update turned back on
pub fn enforce_configured() {
    for d in ALL {
        if crate::config::is(&d.config_key()) && matches!(d.status(), Status::Enabled | Status::Stopped) {
            let _ = d.disable();
        }
    }
}

