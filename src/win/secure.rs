//! Hardening for an app that runs as administrator and can start with Windows without a UAC prompt:
//!
//! - DLLs load only from System32 (no planting next to the exe or in the working directory), and the process
//!   refuses DLLs from network shares or low-integrity locations.
//! - Settings and log live in %ProgramData%\LHelper, writable only by administrators and SYSTEM. Programs running
//!   without admin rights can read them but can't change the power limits, fan curve or GPU overclock that
//!   LHelper applies to the hardware.
//! - The startup task only ever points at a copy in %ProgramFiles%\LHelper (see startup.rs), so it can't be used
//!   to run a swapped exe as administrator.

use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};
use windows::Win32::System::LibraryLoader::{SetDefaultDllDirectories, SetDllDirectoryW, LOAD_LIBRARY_SEARCH_SYSTEM32};

/// Administrators and SYSTEM: full control; Users: read only. Inherited by files created inside.
const ADMIN_ONLY_WRITE: PCWSTR = w!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)");

/// Call first thing in main
pub fn harden_process() {
    unsafe {
        // Later LoadLibrary calls (NVIDIA, uxtheme, powrprof...) search System32 only; no current directory
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
        let _ = SetDllDirectoryW(w!(""));

        // Image load policy: no DLLs from remote shares or low-integrity files, prefer System32 copies
        use windows::Win32::System::Threading::{ProcessImageLoadPolicy, SetProcessMitigationPolicy};
        // PROCESS_MITIGATION_IMAGE_LOAD_POLICY flags: NoRemoteImages (bit 0), NoLowMandatoryLabelImages (bit 1),
        // PreferSystem32Images (bit 2)
        let flags: u32 = 0b111;
        let _ = SetProcessMitigationPolicy(ProcessImageLoadPolicy, &flags as *const u32 as _, 4);
    }
}

/// Applies the admin-only-write ACL to a folder (and what's created inside it from now on)
pub fn lock_down(dir: &Path) -> bool {
    let wide: Vec<u16> = dir.as_os_str().encode_wide_null();
    unsafe {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(ADMIN_ONLY_WRITE, SDDL_REVISION_1, &mut sd, None).is_err() {
            return false;
        }
        let ok = SetFileSecurityW(PCWSTR(wide.as_ptr()), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, sd).as_bool();
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        ok
    }
}

/// Also locks each existing file, since files created before the folder ACL keep their own
pub fn lock_down_tree(dir: &Path) {
    if !lock_down(dir) {
        crate::log!("Couldn't secure {}", dir.display());
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            lock_down(&e.path());
        }
    }
}

/// Folders only administrators can write to: Program Files and Windows
pub fn is_protected_location(path: &Path) -> bool {
    let p = path.to_string_lossy().to_ascii_lowercase();
    ["ProgramFiles", "ProgramW6432", "SystemRoot"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .any(|root| !root.is_empty() && p.starts_with(&root.to_ascii_lowercase()))
}

/// %ProgramFiles%\LHelper: the protected copy the startup task and the package point at
pub fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var("ProgramW6432").or_else(|_| std::env::var("ProgramFiles")).unwrap_or_else(|_| "C:\\Program Files".into())).join("LHelper")
}

/// Download links from the update catalog may only go to Lenovo's download server
pub fn is_lenovo_download(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.starts_with("https://download.lenovo.com/") || u.starts_with("https://pcsupport.lenovo.com/")
}

trait EncodeWideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}
impl EncodeWideNull for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}
