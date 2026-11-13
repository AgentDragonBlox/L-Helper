//! Installs L-Helper and registers the sparse package that makes it a Dynamic Lighting background controller.
//! The package (LHelper.msix) and its certificate (LHelper.cer) are built into the exe, so the release can be one
//! lhelper.exe; the MSI and the zip also put them next to it. No Windows SDK or script needed on testers' PCs.
//!
//! The package is signed with L-Helper's own self-signed certificate, so registering means trusting that
//! certificate for packages on this PC (LocalMachine\TrustedPeople). The user opts in: by running the MSI
//! (packaging/installer, which trusts the certificate), by accepting the install offer the single exe makes on
//! its first start, or from Settings.
//!
//! Names (package, folders, exe) come from identity.rs, so the test build doesn't touch a regular install.

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Foundation::Uri;
use windows::Management::Deployment::{AddPackageOptions, PackageManager, RemovalOptions};

use super::secure;
use crate::identity;

// Written by build.rs from packaging\build (or build-test); empty in builds made without packaging\pack.ps1
static EMBEDDED_MSIX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/LHelper.msix"));
static EMBEDDED_CER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/LHelper.cer"));

/// Apps list entry of an install made by the single exe (the MSI has its own)
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

fn exe_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

/// LHelper.msix and LHelper.cer: next to the running exe (MSI and zip layout), else the copies built into the
/// exe, unpacked to the admin-only data folder
pub fn bundle() -> Option<(PathBuf, PathBuf)> {
    let dir = exe_dir()?;
    let (msix, cer) = (dir.join("LHelper.msix"), dir.join("LHelper.cer"));
    if msix.is_file() && cer.is_file() {
        return Some((msix, cer));
    }
    if EMBEDDED_MSIX.is_empty() || EMBEDDED_CER.is_empty() {
        return None;
    }
    let dir = crate::log::app_dir().join("package");
    let (msix, cer) = (dir.join("LHelper.msix"), dir.join("LHelper.cer"));
    // Rewritten when this exe carries another version (an update)
    for (path, bytes) in [(&msix, EMBEDDED_MSIX), (&cer, EMBEDDED_CER)] {
        if std::fs::read(path).ok().as_deref() != Some(bytes) {
            std::fs::create_dir_all(&dir).ok()?;
            if let Err(e) = std::fs::write(path, bytes) {
                crate::log!("Couldn't unpack {}: {e}", path.display());
                return None;
            }
        }
    }
    Some((msix, cer))
}

/// Full name of the registered package for this user, if any
pub fn registered() -> Option<String> {
    let pm = PackageManager::new().ok()?;
    let found = pm.FindPackagesByUserSecurityIdNamePublisher(&HSTRING::new(), &identity::PACKAGE.into(), &identity::publisher().into()).ok()?;
    let first = found.into_iter().next()?;
    first.Id().and_then(|id| id.FullName()).ok().map(|n| n.to_string())
}

/// Copy the exe to Program Files, trust the certificate, register the package against that folder and add the
/// Start menu shortcut (and, for an install the MSI didn't make, an Apps list entry to uninstall it).
/// Then L-Helper has to be restarted from Program Files to run with the package identity.
pub fn register() -> Result<(), String> {
    let (msix, cer) = bundle().ok_or("This build has no Dynamic Lighting package: use a release build of L-Helper")?;
    let exe = super::startup::protected_exe().ok_or("Couldn't copy L-Helper to Program Files")?;
    let dir = exe.parent().ok_or("No install folder")?.to_path_buf();

    trust_certificate(&cer)?;

    let pm = PackageManager::new().map_err(|e| e.message())?;
    let options = AddPackageOptions::new().map_err(|e| e.message())?;
    let uri = |p: &Path| Uri::CreateUri(&HSTRING::from(p.to_string_lossy().as_ref())).map_err(|e| e.message());
    options.SetExternalLocationUri(&uri(&dir)?).map_err(|e| e.message())?;
    options.SetForceUpdateFromAnyVersion(true).map_err(|e| e.message())?;
    let result = pm.AddPackageByUriAsync(&uri(&msix)?, &options).and_then(|op| op.join());
    match result {
        Ok(r) if r.IsRegistered().unwrap_or(false) => {}
        Ok(r) => return Err(r.ErrorText().map(|t| t.to_string()).unwrap_or_else(|_| "Registration failed".into())),
        Err(e) => return Err(format!("Registration failed: {}", e.message())),
    }
    crate::log!("Registered {} -> {}", registered().unwrap_or_default(), dir.display());
    crate::config::remove("dl_package_removed");

    if let Err(e) = shortcut(&exe) {
        crate::log!("Start menu shortcut: {e}");
    }
    // The MSI puts the package next to the exe and has its own Apps entry
    if !dir.join("LHelper.msix").exists() {
        if let Err(e) = add_uninstall_entry(&exe) {
            crate::log!("Apps list entry: {e}");
        }
    }
    Ok(())
}

fn add_uninstall_entry(exe: &Path) -> Result<(), String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let (key, _) = winreg::RegKey::predef(HKEY_LOCAL_MACHINE).create_subkey(format!(r"{UNINSTALL_KEY}\{}", identity::FOLDER)).map_err(|e| e.to_string())?;
    let exe_s = exe.to_string_lossy().to_string();
    let size_kb = std::fs::metadata(exe).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    let set = |name: &str, value: &str| key.set_value(name, &value.to_string()).map_err(|e| e.to_string());
    set("DisplayName", identity::DISPLAY)?;
    set("DisplayVersion", env!("CARGO_PKG_VERSION"))?;
    set("Publisher", "L-Helper")?;
    set("DisplayIcon", &format!("{exe_s},0"))?;
    set("InstallLocation", &exe.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default())?;
    set("UninstallString", &format!("\"{exe_s}\" -uninstall"))?;
    set("URLInfoAbout", "https://github.com/AgentDragonBlox/L-Helper")?;
    for name in ["NoModify", "NoRepair"] {
        key.set_value(name, &1u32).map_err(|e| e.to_string())?;
    }
    key.set_value("EstimatedSize", &size_kb).map_err(|e| e.to_string())?;
    Ok(())
}

/// First start of the single exe from outside Program Files (Downloads): offer to install. True when the installed
/// copy was started and this process should exit.
pub fn offer_install() -> bool {
    if EMBEDDED_MSIX.is_empty() || crate::config::exists("install_declined") || crate::lighting::lamparray::package_name().is_some() {
        return false;
    }
    let Ok(current) = std::env::current_exe() else { return false };
    if secure::is_protected_location(&current) || registered().is_some() {
        return false;
    }
    let dir = secure::install_dir();
    let text = format!(
        "Install {name} on this PC?\n\n{name} copies itself to {dir}, adds a Start menu entry and an entry in Settings → Apps to uninstall it, \
         and registers with Windows Dynamic Lighting. That trusts {name}'s own signing certificate (CN={package}) for app packages on this PC.\n\n\
         No: {name} runs from here, without Dynamic Lighting background control. You can install later from Settings → Dynamic Lighting → Register.",
        name = identity::DISPLAY,
        dir = dir.display(),
        package = identity::PACKAGE,
    );
    if !super::message_box(&text, true) {
        crate::config::set("install_declined", 1);
        return false;
    }
    match register() {
        Ok(()) => spawn_installed(),
        Err(e) => {
            crate::log!("Install failed: {e}");
            super::message_box(&format!("{} couldn't be installed:\n\n{e}\n\nIt will run from here instead.", identity::DISPLAY), false);
            false
        }
    }
}

/// Remove the package registration and stop trusting the certificate (the Program Files copy stays, for startup)
pub fn unregister() -> Result<(), String> {
    if let Some(name) = registered() {
        let pm = PackageManager::new().map_err(|e| e.message())?;
        pm.RemovePackageAsync(&HSTRING::from(&name)).and_then(|op| op.join()).map_err(|e| e.message())?;
        crate::log!("Removed package {name}");
    }
    if let Some((_, cer)) = bundle() {
        untrust_certificate(&cer);
    }
    // Removed on purpose: don't register again at the next start after an MSI install
    crate::config::set("dl_package_removed", 1);
    Ok(())
}

/// Start the Program Files copy (which runs with the package identity) once this process has exited
pub fn restart_installed() {
    if spawn_installed() {
        crate::app::quit();
    }
}

fn spawn_installed() -> bool {
    let exe = secure::install_dir().join(identity::EXE);
    let pid = std::process::id().to_string();
    match std::process::Command::new(&exe).args(["-after", &pid]).spawn() {
        Ok(_) => true,
        Err(e) => {
            crate::log!("Couldn't start {}: {e}", exe.display());
            false
        }
    }
}

/// First start after an MSI install: register the package for this user (packages are per user, and the
/// installer runs as SYSTEM, having already trusted the certificate). True when a restart with the package
/// identity was started and this process should exit.
pub fn register_after_install() -> bool {
    if crate::lighting::lamparray::package_name().is_some() || crate::config::is("dl_package_removed") {
        return false;
    }
    let in_install_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.to_string_lossy().to_lowercase()))
        .is_some_and(|d| d == secure::install_dir().to_string_lossy().to_lowercase());
    let Some((_, cer)) = bundle() else { return false };
    if !in_install_dir || !is_trusted(&cer) || registered().is_some() {
        return false;
    }
    crate::log!("Registering for Dynamic Lighting (installed by the MSI)");
    match register() {
        Ok(()) => spawn_installed(),
        Err(e) => {
            crate::log!("Dynamic Lighting registration failed: {e}");
            false
        }
    }
}

/// Commands the MSI runs as SYSTEM (packaging/installer/LHelper.wxs); returns the exit code for msiexec
pub fn installer_command(args: &[String]) -> Option<i32> {
    if args.iter().any(|a| a == "-msi-install") {
        let result = bundle().ok_or_else(|| "LHelper.cer missing".to_string()).and_then(|(_, cer)| trust_certificate(&cer));
        return Some(match result {
            Ok(()) => {
                crate::log!("MSI: certificate trusted");
                0
            }
            Err(e) => {
                crate::log!("MSI: {e}");
                1
            }
        });
    }
    if args.iter().any(|a| a == "-msi-uninstall") {
        remove_everything();
        crate::log!("MSI: removed the package, certificate and startup tasks");
        return Some(0);
    }
    // Settings → Apps → Uninstall, for an install made by the single exe
    if args.iter().any(|a| a == "-uninstall") {
        if secure::install_dir().join("LHelper.msix").exists() {
            super::message_box(&format!("{} was installed with the MSI: uninstall it from Settings → Apps.", identity::DISPLAY), false);
            return Some(1);
        }
        if !super::message_box(&format!("Uninstall {}?\n\nSettings and the log in {} are kept.", identity::DISPLAY, crate::log::app_dir().display()), true) {
            return Some(1);
        }
        remove_everything();
        if let Ok(programs) = std::env::var("ProgramData").map(|d| PathBuf::from(d).join(r"Microsoft\Windows\Start Menu\Programs")) {
            let _ = std::fs::remove_file(programs.join(identity::shortcut_name()));
        }
        let _ = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).delete_subkey_all(format!(r"{UNINSTALL_KEY}\{}", identity::FOLDER));
        let _ = std::fs::remove_dir_all(crate::log::app_dir().join("package"));
        remove_install_dir_after_exit();
        crate::log!("Uninstalled");
        return Some(0);
    }
    None
}

/// Other running copies, every user's package registration, the certificate and the startup tasks
fn remove_everything() {
    super::terminate_other_instances();
    remove_for_all_users();
    if let Some((_, cer)) = bundle() {
        untrust_certificate(&cer);
    }
    super::startup::unschedule_all();
}

/// The installed exe may be this process: cmd deletes the folder a moment after it exits
fn remove_install_dir_after_exit() {
    use std::os::windows::process::CommandExt;
    let dir = secure::install_dir();
    if !secure::is_protected_location(&dir) || !dir.ends_with(identity::FOLDER) {
        return;
    }
    let cmd = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join(r"System32\cmd.exe");
    let _ = std::process::Command::new(cmd)
        .raw_arg(format!("/d /c \"ping -n 3 127.0.0.1 >nul & rmdir /s /q \"{}\"\"", dir.display()))
        .creation_flags(0x08000000)
        .spawn();
}

/// Every user's registration of the package (uninstall runs as SYSTEM, not as any of them)
fn remove_for_all_users() {
    let Ok(pm) = PackageManager::new() else { return };
    let Ok(found) = pm.FindPackagesByNamePublisher(&identity::PACKAGE.into(), &identity::publisher().into()) else { return };
    for package in found {
        let Ok(name) = package.Id().and_then(|id| id.FullName()) else { continue };
        let result = pm.RemovePackageWithOptionsAsync(&name, RemovalOptions::RemoveForAllUsers).and_then(|op| op.join());
        crate::log!("MSI: removed {name}: {}", result.map(|_| "OK".to_string()).unwrap_or_else(|e| e.message()));
    }
}

/// The DER certificate, checked to be L-Helper's (and nothing else's) before it's trusted
fn read_certificate(cer: &Path) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::*;
    let der = std::fs::read(cer).map_err(|e| e.to_string())?;
    unsafe {
        let ctx = CertCreateCertificateContext(X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, &der);
        if ctx.is_null() {
            return Err("LHelper.cer isn't a certificate".into());
        }
        let mut name = [0u16; 256];
        let len = CertGetNameStringW(ctx, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut name));
        let _ = CertFreeCertificateContext(Some(ctx));
        let subject = String::from_utf16_lossy(&name[..(len as usize).saturating_sub(1)]);
        if subject != identity::PACKAGE {
            return Err(format!("LHelper.cer is issued to \"{subject}\", not {}", identity::PACKAGE));
        }
    }
    Ok(der)
}

fn trusted_people() -> Result<windows::Win32::Security::Cryptography::HCERTSTORE, String> {
    use windows::Win32::Security::Cryptography::*;
    unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            CERT_QUERY_ENCODING_TYPE(0),
            None,
            CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE),
            Some(windows::core::w!("TrustedPeople").as_ptr() as *const _),
        )
        .map_err(|e| format!("Certificate store: {}", e.message()))
    }
}

fn trust_certificate(cer: &Path) -> Result<(), String> {
    use windows::Win32::Security::Cryptography::*;
    let der = read_certificate(cer)?;
    let store = trusted_people()?;
    unsafe {
        let added = CertAddEncodedCertificateToStore(Some(store), X509_ASN_ENCODING, &der, CERT_STORE_ADD_REPLACE_EXISTING, None);
        let _ = CertCloseStore(Some(store), 0);
        added.map_err(|e| format!("Couldn't trust the certificate: {}", e.message()))
    }
}

/// Runs `f` on the TrustedPeople copy of the certificate, if it's there
fn with_trusted<T>(cer: &Path, f: impl FnOnce(*const windows::Win32::Security::Cryptography::CERT_CONTEXT) -> T) -> Option<T> {
    use windows::Win32::Security::Cryptography::*;
    let der = read_certificate(cer).ok()?;
    let store = trusted_people().ok()?;
    let mut result = None;
    unsafe {
        let ctx = CertCreateCertificateContext(X509_ASN_ENCODING, &der);
        if !ctx.is_null() {
            let found = CertFindCertificateInStore(store, X509_ASN_ENCODING, 0, CERT_FIND_EXISTING, Some(ctx as *const _), None);
            if !found.is_null() {
                result = Some(f(found));
            }
            let _ = CertFreeCertificateContext(Some(ctx));
        }
        let _ = CertCloseStore(Some(store), 0);
    }
    result
}

fn is_trusted(cer: &Path) -> bool {
    with_trusted(cer, |found| unsafe {
        let _ = windows::Win32::Security::Cryptography::CertFreeCertificateContext(Some(found));
    })
    .is_some()
}

fn untrust_certificate(cer: &Path) {
    // Deleting frees the context
    with_trusted(cer, |found| unsafe {
        let _ = windows::Win32::Security::Cryptography::CertDeleteCertificateFromStore(found);
    });
}

/// All users' Start menu shortcut, with the same taskbar ID as the running window so they group together
fn shortcut(exe: &Path) -> Result<(), String> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let programs = PathBuf::from(std::env::var("ProgramData").map_err(|e| e.to_string())?).join(r"Microsoft\Windows\Start Menu\Programs");
    let _ = std::fs::remove_file(programs.join(format!("{}.lnk", identity::FOLDER)));
    // The MSI's own shortcut (same name): leave it, the installer removes it on uninstall
    let path = programs.join(identity::shortcut_name());
    if path.exists() {
        return Ok(());
    }
    let wide = |s: &str| HSTRING::from(s);
    let e = |e: windows::core::Error| e.message();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        link.SetPath(&wide(&exe.to_string_lossy())).map_err(e)?;
        link.SetWorkingDirectory(&wide(&exe.parent().unwrap_or(exe).to_string_lossy())).map_err(e)?;
        link.SetIconLocation(&wide(&exe.to_string_lossy()), 0).map_err(e)?;
        link.SetDescription(&wide(identity::DISPLAY)).map_err(e)?;

        let store: IPropertyStore = link.cast().map_err(e)?;
        super::set_string_property(&store, &PKEY_AppUserModel_ID, crate::AUMID).map_err(e)?;
        store.Commit().map_err(e)?;

        let file: IPersistFile = link.cast().map_err(e)?;
        file.Save(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), true).map_err(e)?;
    }
    Ok(())
}
