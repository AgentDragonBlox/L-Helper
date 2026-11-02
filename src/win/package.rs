//! Registers the sparse package that makes L-Helper a Dynamic Lighting background controller, from the
//! LHelper.msix and LHelper.cer next to the exe. No Windows SDK or script needed on testers' PCs.
//!
//! The package is signed with L-Helper's own self-signed certificate, so registering means trusting that
//! certificate for packages on this PC (LocalMachine\TrustedPeople). The user opts in: by running the MSI
//! (packaging/installer, which trusts the certificate) or from Settings with the zip.

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Foundation::Uri;
use windows::Management::Deployment::{AddPackageOptions, PackageManager, RemovalOptions};

use super::secure;

const NAME: &str = "LHelper";
const PUBLISHER: &str = "CN=LHelper";

/// LHelper.msix and LHelper.cer next to the running exe (the release zip's layout)
pub fn bundle() -> Option<(PathBuf, PathBuf)> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let (msix, cer) = (dir.join("LHelper.msix"), dir.join("LHelper.cer"));
    (msix.is_file() && cer.is_file()).then_some((msix, cer))
}

/// Full name of the registered package for this user, if any
pub fn registered() -> Option<String> {
    let pm = PackageManager::new().ok()?;
    let found = pm.FindPackagesByUserSecurityIdNamePublisher(&HSTRING::new(), &NAME.into(), &PUBLISHER.into()).ok()?;
    let first = found.into_iter().next()?;
    first.Id().and_then(|id| id.FullName()).ok().map(|n| n.to_string())
}

/// Copy the exe to Program Files, trust the certificate, register the package against that folder and add the
/// Start menu shortcut. Then L-Helper has to be restarted from Program Files to run with the package identity.
pub fn register() -> Result<(), String> {
    let (msix, cer) = bundle().ok_or("LHelper.msix and LHelper.cer must be next to lhelper.exe (unzip the whole release)")?;
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
    Ok(())
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
    let exe = secure::install_dir().join("lhelper.exe");
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
        super::terminate_other_instances();
        remove_for_all_users();
        if let Some((_, cer)) = bundle() {
            untrust_certificate(&cer);
        }
        super::startup::unschedule_all();
        crate::log!("MSI: removed the package, certificate and startup tasks");
        return Some(0);
    }
    None
}

/// Every user's registration of the package (uninstall runs as SYSTEM, not as any of them)
fn remove_for_all_users() {
    let Ok(pm) = PackageManager::new() else { return };
    let Ok(found) = pm.FindPackagesByNamePublisher(&NAME.into(), &PUBLISHER.into()) else { return };
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
        if subject != NAME {
            return Err(format!("LHelper.cer is issued to \"{subject}\", not LHelper"));
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
    let _ = std::fs::remove_file(programs.join("LHelper.lnk"));
    // The MSI's own shortcut: leave it, the installer removes it on uninstall
    if programs.join("L-Helper.lnk").exists() {
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
        link.SetDescription(&wide("L-Helper")).map_err(e)?;

        let store: IPropertyStore = link.cast().map_err(e)?;
        super::set_string_property(&store, &PKEY_AppUserModel_ID, crate::AUMID).map_err(e)?;
        store.Commit().map_err(e)?;

        let file: IPersistFile = link.cast().map_err(e)?;
        file.Save(PCWSTR(wide(&programs.join("L-Helper.lnk").to_string_lossy()).as_ptr()), true).map_err(e)?;
    }
    Ok(())
}
