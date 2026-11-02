//! BIOS / driver update check against Lenovo's catalog (download.lenovo.com/catalog/<MT>_win11.xml), the same
//! source Vantage and LLT use. Package detection rules are ported from LLT's VantagePackageUpdateDetector.
//! Check only: nothing is downloaded or installed. Packages that Lenovo detects by downloading and running a
//! checker program (_ExternalDetection, e.g. GPU VBIOS updates) are listed as "check manually" instead.

use std::collections::HashMap;

use roxmltree::{Document, Node};
use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinInet::*;
use winreg::enums::*;
use winreg::RegKey;

use crate::lenovo::machine;

const CATALOG_URL: &str = "https://download.lenovo.com/catalog";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Lenovo's detection rules say this machine needs it
    Available,
    /// Applies to this machine, but Lenovo detects it with its own checker program
    CheckManually,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub title: String,
    pub category: String,
    pub version: String,
    pub release_date: String,
    pub size_mb: f64,
    pub restart: bool,
    pub file_url: String,
    pub readme_url: Option<String>,
    pub status: Status,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub machine_type: String,
    pub checked: usize,
    pub packages: Vec<Package>,
}

// ---------------- HTTP (WinInet: Windows' own TLS and proxy settings) ----------------

struct Handle(*mut std::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = InternetCloseHandle(self.0);
            }
        }
    }
}

fn http_get(url: &str) -> Result<String, String> {
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let session = Handle(InternetOpenW(w!("LHelper"), INTERNET_OPEN_TYPE_PRECONFIG.0, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err("No internet connection".into());
        }
        let request = Handle(InternetOpenUrlW(session.0, PCWSTR(wide.as_ptr()), None, INTERNET_FLAG_RELOAD | INTERNET_FLAG_NO_CACHE_WRITE | INTERNET_FLAG_SECURE, None));
        if request.0.is_null() {
            return Err(format!("Can't reach {url}: {}", windows::core::Error::from_thread()));
        }
        let mut status = 0u32;
        let mut len = 4u32;
        if HttpQueryInfoW(request.0, HTTP_QUERY_STATUS_CODE | HTTP_QUERY_FLAG_NUMBER, Some(&mut status as *mut u32 as _), &mut len, None).is_ok() && status != 200 {
            return Err(format!("{url}: HTTP {status}"));
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            InternetReadFile(request.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut read).map_err(|e| format!("{url}: {e}"))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        Ok(text.trim_start_matches('\u{feff}').to_string())
    }
}

// ---------------- what's installed ----------------

struct Driver {
    device_id: String,
    hardware_id: String,
    version: Option<Vec<u64>>,
    /// yyyymmdd
    date: Option<u32>,
}

struct Context {
    drivers: Vec<Driver>,
    bios_prefix: String,
    bios_version: Option<u32>,
    windows_11: bool,
}

/// "25.020.5.0021^" -> [25, 20, 5, 21]
fn parse_version(s: &str) -> Option<Vec<u64>> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let parts: Option<Vec<u64>> = cleaned.split('.').filter(|p| !p.is_empty()).map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

fn version_greater(a: &[u64], b: &[u64]) -> bool {
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// "2025-12-12" or WMI "20251212000000.******+000" -> 20251212
fn parse_date(s: &str) -> Option<u32> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).take(8).collect();
    (digits.len() == 8).then(|| digits.parse().ok()).flatten()
}

/// "R8CN26WW" / "R8CN25W*" -> ("R8CN", 26)
fn parse_bios(s: &str) -> (String, Option<u32>) {
    let prefix: String = s.chars().take(4).collect::<String>().to_ascii_uppercase();
    let version: String = s.chars().skip(4).take_while(|c| c.is_ascii_digit()).take(2).collect();
    (prefix, version.parse().ok())
}

fn context() -> Context {
    let drivers = crate::wmi::query_in("root\\cimv2", "SELECT DeviceID, HardWareID, DriverVersion, DriverDate FROM Win32_PnPSignedDriver")
        .unwrap_or_else(|e| {
            crate::log!("Updates: can't read installed drivers: {e}");
            Vec::new()
        })
        .iter()
        .map(|o| Driver {
            device_id: o.get("DeviceID").as_str().unwrap_or_default().to_ascii_uppercase(),
            hardware_id: o.get("HardWareID").as_str().unwrap_or_default().to_ascii_uppercase(),
            version: o.get("DriverVersion").as_str().and_then(parse_version),
            date: o.get("DriverDate").as_str().and_then(parse_date),
        })
        .collect();
    let (bios_prefix, bios_version) = parse_bios(&machine::get().bios);
    Context { drivers, bios_prefix, bios_version, windows_11: windows_11() }
}

fn windows_11() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .is_some_and(|b| b >= 22000)
}

// ---------------- rules ----------------

/// None = can't tell without running Lenovo's checker program
type Tri = Option<bool>;

fn all(results: impl Iterator<Item = Tri>) -> Tri {
    let mut unknown = false;
    for r in results {
        match r {
            Some(false) => return Some(false),
            None => unknown = true,
            _ => {}
        }
    }
    if unknown { None } else { Some(true) }
}

fn any(results: impl Iterator<Item = Tri>) -> Tri {
    let mut unknown = false;
    for r in results {
        match r {
            Some(true) => return Some(true),
            None => unknown = true,
            _ => {}
        }
    }
    if unknown { None } else { Some(false) }
}

fn children<'a>(node: Node<'a, 'a>) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children().filter(|n| n.is_element())
}

fn texts(node: Node, name: &str) -> Vec<String> {
    children(node).filter(|n| n.has_tag_name(name)).filter_map(|n| n.text()).map(|t| t.trim().to_string()).collect()
}

fn text(node: Node, name: &str) -> Option<String> {
    texts(node, name).into_iter().next()
}

fn find_driver<'c>(ctx: &'c Context, ids: &[String]) -> Option<&'c Driver> {
    ctx.drivers.iter().find(|d| ids.iter().any(|id| d.device_id.starts_with(&id.to_ascii_uppercase()) || d.hardware_id.starts_with(&id.to_ascii_uppercase())))
}

/// LLT DriverPackageRule.VerifyByDateVersion: the package is newer than what's installed
fn driver_needs_update(node: Node, d: &Driver) -> bool {
    let version = text(node, "Version").and_then(|v| parse_version(&v));
    let date = text(node, "Date").and_then(|v| parse_date(&v));
    match (version, &d.version) {
        (Some(v), Some(installed)) => {
            if let (Some(date), Some(installed_date)) = (date, d.date) {
                if date < installed_date {
                    return false;
                }
            }
            version_greater(&v, installed)
        }
        _ => false,
    }
}

fn registry_key(path: &str) -> Option<(RegKey, String)> {
    let (hive, rest) = path.split_once('\\')?;
    let root = match hive.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKCR" | "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        "HKU" | "HKEY_USERS" => HKEY_USERS,
        _ => return None,
    };
    Some((RegKey::predef(root), rest.to_string()))
}

fn registry_value(node: Node) -> Option<(Vec<u64>, Option<String>)> {
    let key = text(node, "Key")?;
    let name = text(node, "KeyName")?;
    let version = parse_version(&text(node, "Version")?)?;
    let (root, path) = registry_key(&key)?;
    let value = root.open_subkey(path).ok().and_then(|k| k.get_value::<String, _>(name).ok());
    Some((version, value))
}

/// One rule element. `deps` picks the Dependencies meaning (applies to this machine) over the
/// DetectInstall meaning (install needed). Unknown or malformed rules are skipped, as in LLT.
fn eval(node: Node, ctx: &Context, deps: bool) -> Option<Tri> {
    Some(match node.tag_name().name() {
        "And" => all(children(node).filter_map(|n| eval(n, ctx, deps))),
        "Or" => any(children(node).filter_map(|n| eval(n, ctx, deps))),
        "Not" => children(node).find_map(|n| eval(n, ctx, deps))?.map(|v| !v),
        "_OS" => {
            let oses = texts(node, "OS");
            if oses.is_empty() {
                return None;
            }
            let current = if ctx.windows_11 { "WIN11" } else { "WIN10" };
            Some(oses.iter().any(|o| o.to_ascii_uppercase().starts_with(current)))
        }
        "_CPUAddressWidth" => Some(text(node, "AddressWidth")?.trim() == "64"),
        "_ExternalDetection" => None,
        "_PnPID" => {
            let id = node.text()?.trim().to_string();
            Some(find_driver(ctx, &[id]).is_some())
        }
        "_Driver" => {
            let ids = texts(node, "HardwareID");
            if ids.is_empty() {
                return None;
            }
            Some(match find_driver(ctx, &ids) {
                None => false,
                Some(d) if deps => !driver_needs_update(node, d),
                Some(d) => driver_needs_update(node, d),
            })
        }
        "_Bios" => {
            let levels: Vec<(String, Option<u32>)> = texts(node, "Level").iter().map(|l| parse_bios(l)).collect();
            if levels.is_empty() {
                return None;
            }
            let current = ctx.bios_version?;
            Some(if deps {
                levels.iter().any(|(p, v)| *p == ctx.bios_prefix && *v == Some(current))
            } else {
                levels.iter().all(|(p, v)| *p == ctx.bios_prefix && v.is_some_and(|v| v > current))
            })
        }
        "_RegistryKey" => {
            let (root, path) = registry_key(&text(node, "Key")?)?;
            Some(root.open_subkey(path).is_ok())
        }
        "_RegistryKeyValue" => {
            let (version, value) = registry_value(node)?;
            let installed = value.as_deref().and_then(|v| parse_version(v.split_whitespace().next().unwrap_or(v)));
            Some(match (deps, installed, value.is_some()) {
                (true, _, false) => true,
                (true, Some(i), true) => !version_greater(&version, &i),
                (false, Some(i), true) => version_greater(&version, &i),
                _ => false,
            })
        }
        // _WindowsBuildVersion lists builds (LLT skips it too); anything else is unknown
        _ => return None,
    })
}

fn package(xml: &str, category: &str, location: &str, ctx: &Context) -> Option<Package> {
    let doc = Document::parse(xml).ok()?;
    let root = doc.root_element();
    let child = |name: &str| children(root).find(|n| n.has_tag_name(name));

    // Dependencies: does it apply here at all (LLT: no rules = doesn't apply)
    let deps = match child("Dependencies").filter(|n| children(*n).next().is_some()) {
        Some(n) => all(children(n).filter_map(|r| eval(r, ctx, true))),
        None => Some(false),
    };
    if deps == Some(false) {
        return None;
    }
    let needed = match child("DetectInstall").filter(|n| children(*n).next().is_some()) {
        Some(n) => all(children(n).filter_map(|r| eval(r, ctx, false))),
        None => Some(true),
    };
    let status = match (deps, needed) {
        (_, Some(false)) => return None,
        (Some(true), Some(true)) => Status::Available,
        _ => Status::CheckManually,
    };

    let base = &location[..location.rfind('/')?];
    let files = child("Files")?;
    let file = |kind: &str| children(files).find(|n| n.has_tag_name(kind)).and_then(|n| children(n).find(|n| n.has_tag_name("File")));
    let installer = file("Installer")?;
    let title = child("Title").and_then(|t| children(t).find(|n| n.has_tag_name("Desc"))).and_then(|n| n.text()).unwrap_or("Lenovo package").trim().to_string();
    Some(Package {
        title,
        category: category.to_string(),
        version: root.attribute("version").unwrap_or_default().to_string(),
        release_date: child("ReleaseDate").and_then(|n| n.text()).unwrap_or_default().trim().to_string(),
        size_mb: text(installer, "Size").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) / 1024.0 / 1024.0,
        restart: child("Reboot").and_then(|n| n.attribute("type")).is_some_and(|t| t != "0"),
        file_url: format!("{base}/{}", text(installer, "Name")?),
        readme_url: file("Readme").and_then(|f| text(f, "Name")).map(|n| format!("{base}/{n}")),
        status,
    })
}

/// Fetches the catalog and every package description (in parallel) and returns what applies to this laptop
pub fn check() -> Result<Report, String> {
    let m = machine::get();
    let machine_type: String = m.machine_type.chars().take(4).collect::<String>().to_ascii_uppercase();
    if machine_type.len() != 4 {
        return Err("Can't tell this laptop's machine type".into());
    }
    let os = if windows_11() { "win11" } else { "win10" };
    let catalog = http_get(&format!("{CATALOG_URL}/{machine_type}_{os}.xml")).map_err(|e| {
        if e.contains("HTTP 404") {
            format!("Lenovo has no update catalog for machine type {machine_type}")
        } else {
            e
        }
    })?;
    let doc = Document::parse(&catalog).map_err(|e| format!("Catalog: {e}"))?;
    let entries: Vec<(String, String)> = doc
        .descendants()
        .filter(|n| n.has_tag_name("package"))
        .filter_map(|p| Some((text(p, "location")?, text(p, "category").unwrap_or_default())))
        .collect();

    let ctx = context();
    let fetched: HashMap<usize, String> = std::thread::scope(|s| {
        let handles: Vec<_> = entries.iter().enumerate().map(|(i, (location, _))| s.spawn(move || (i, http_get(location)))).collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter_map(|(i, r)| match r {
                Ok(xml) => Some((i, xml)),
                Err(e) => {
                    crate::log!("Updates: {e}");
                    None
                }
            })
            .collect()
    });

    let mut packages: Vec<Package> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, (location, category))| package(fetched.get(&i)?, category, location, &ctx))
        .collect();
    packages.sort_by(|a, b| (a.status as u8, &a.category, &a.title).cmp(&(b.status as u8, &b.category, &b.title)));
    crate::log!(
        "Updates for {machine_type}: {} of {} packages apply: {}",
        packages.len(),
        entries.len(),
        packages.iter().map(|p| format!("{} {} ({:?})", p.title, p.version, p.status)).collect::<Vec<_>>().join(" | ")
    );
    Ok(Report { machine_type, checked: entries.len(), packages })
}
