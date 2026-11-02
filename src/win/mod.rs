//! Windows-side helpers

pub mod display;
pub mod hotkey;
pub mod oled;
pub mod overlay;
pub mod package;
pub mod power;
pub mod secure;
pub mod startup;
pub mod toast;
pub mod toggles;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Windows apps theme (AppsUseLightTheme = 0 → dark), unless overridden in config ui_mode
pub fn dark_mode() -> bool {
    match crate::config::get_string("ui_mode").as_deref() {
        Some("dark") => return true,
        Some("light") => return false,
        _ => {}
    }
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(true)
}

/// Dark or light Win32 popup menus (the tray menu) for the whole process: uxtheme's SetPreferredAppMode
/// (ordinal 135; 2 = force dark, 3 = force light) and FlushMenuThemes (ordinal 136), as Explorer and Notepad use
pub fn set_menu_theme(dark: bool) {
    use windows::core::{w, PCSTR};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    unsafe {
        let Ok(lib) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) else { return };
        if let Some(p) = GetProcAddress(lib, PCSTR(135usize as *const u8)) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(p);
            set_mode(if dark { 2 } else { 3 });
        }
        if let Some(p) = GetProcAddress(lib, PCSTR(136usize as *const u8)) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(p);
            flush();
        }
    }
}

pub fn set_dark_title_bar(hwnd: HWND, dark: bool) {
    use windows::Win32::Graphics::Dwm::*;
    let value: i32 = dark as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const i32 as _, 4);
    }
}

pub fn show_window(hwnd: HWND, visible: bool) {
    unsafe {
        // SW_RESTORE also brings back a window that was minimized before it was hidden
        let _ = ShowWindow(hwnd, if visible { SW_RESTORE } else { SW_HIDE });
        if visible {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Shows the window in the bottom-right corner of the work area (next to the tray), G-Helper style
pub fn show_near_tray(hwnd: HWND) {
    use windows::Win32::Foundation::RECT;
    unsafe {
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let margin = 12;
        let _ = SetWindowPos(hwnd, None, work.right - w - margin, work.bottom - h - margin, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
    }
    show_window(hwnd, true);
}

/// Work area (screen minus taskbar) of the monitor the window is on, in physical pixels
fn work_area(hwnd: HWND) -> Option<windows::Win32::Foundation::RECT> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(monitor, &mut info).as_bool().then_some(info.rcWork)
    }
}

/// Tallest client area (physical pixels) that still fits the window, title bar included, on its monitor
pub fn max_client_height(hwnd: HWND) -> Option<i32> {
    use windows::Win32::Foundation::RECT;
    let work = work_area(hwnd)?;
    let (mut window, mut client) = (RECT::default(), RECT::default());
    unsafe {
        GetWindowRect(hwnd, &mut window).ok()?;
        GetClientRect(hwnd, &mut client).ok()?;
    }
    let frame = (window.bottom - window.top) - (client.bottom - client.top);
    Some((work.bottom - work.top) - frame - 2 * 12)
}

/// Moves the window back inside its monitor's work area if a resize pushed it past an edge
pub fn keep_on_screen(hwnd: HWND) {
    use windows::Win32::Foundation::RECT;
    let Some(work) = work_area(hwnd) else { return };
    let mut r = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut r).is_err() || !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        let margin = 12;
        let h = r.bottom - r.top;
        let mut top = r.top;
        if r.bottom > work.bottom - margin {
            top = work.bottom - margin - h;
        }
        top = top.max(work.top + margin.min((work.bottom - work.top - h).max(0)));
        if top != r.top {
            let _ = SetWindowPos(hwnd, None, r.left, top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

/// Dark title bar for a child window found by its title
pub fn dark_title_for(title: &str, dark: bool) {
    let w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if let Ok(hwnd) = FindWindowW(windows::core::PCWSTR::null(), windows::core::PCWSTR(w.as_ptr())) {
            set_dark_title_bar(hwnd, dark);
        }
    }
}

/// Captures a top-level window by title (PrintWindow with PW_RENDERFULLCONTENT works for GL windows). Returns (w, h, RGBA).
pub fn capture_window(title: &str) -> Option<(u32, u32, Vec<u8>)> {
    let w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let hwnd = unsafe { FindWindowW(windows::core::PCWSTR::null(), windows::core::PCWSTR(w.as_ptr())).ok()? };
    capture_hwnd(hwnd)
}

pub fn capture_hwnd(hwnd: HWND) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).ok()?;
        let (width, height) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: width as i32, biHeight: -(height as i32), biPlanes: 1, biBitCount: 32, ..Default::default() };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(mem, bitmap.into());
        let ok = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(2)).as_bool(); // PW_RENDERFULLCONTENT
        let mut rgba = Vec::new();
        if ok {
            let data = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
            rgba.reserve(data.len());
            for px in data.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        ok.then_some((width, height, rgba))
    }
}

/// Ends other lhelper.exe processes (used when a previous instance stopped responding)
pub fn terminate_other_instances() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::Threading::*;
    unsafe {
        let me = GetCurrentProcessId();
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snapshot, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            if name == "lhelper.exe" && entry.th32ProcessID != me {
                if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, entry.th32ProcessID) {
                    let _ = TerminateProcess(h, 1);
                    let _ = WaitForSingleObject(h, 2000);
                    let _ = CloseHandle(h);
                    crate::log!("Ended unresponsive LHelper (PID {})", entry.th32ProcessID);
                }
            }
            ok = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
}

pub fn is_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// A Windows program by its full path (System32 / Windows folder), never through PATH
pub fn system_exe(name: &str) -> std::path::PathBuf {
    let root = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()));
    let in_system32 = root.join("System32").join(name);
    if in_system32.exists() { in_system32 } else { root.join(name) }
}

/// Opens a link in the default browser. Going through explorer keeps the browser un-elevated (LHelper runs as admin).
/// Explorer would also run a local program given its path, so only Lenovo's HTTPS download links (from the update
/// catalog) and Windows Settings pages are accepted.
/// Set a string property (VT_LPWSTR) on a shell property store; the store copies the value
pub fn set_string_property(
    store: &windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore,
    key: &windows::Win32::Foundation::PROPERTYKEY,
    value: &str,
) -> windows::core::Result<()> {
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Variant::VT_LPWSTR;
    let mut text: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let mut prop = PROPVARIANT::default();
    unsafe {
        (*prop.Anonymous.Anonymous).vt = VT_LPWSTR;
        (*prop.Anonymous.Anonymous).Anonymous.pwszVal = windows::core::PWSTR(text.as_mut_ptr());
        let result = store.SetValue(key, &prop);
        // `text` is ours, not COM's: don't let PROPVARIANT's drop free it
        std::mem::forget(prop);
        result
    }
}

/// Give the main window its own taskbar identity (ID, relaunch command, icon and name), so the taskbar shows the
/// window's icon instead of looking it up in the Dynamic Lighting package, whose logo it can't resolve (the
/// package has no app list entry) and draws as a blank square. Window properties win over the package's ID.
pub fn set_window_app_id(hwnd: HWND) {
    use windows::Win32::Storage::EnhancedStorage::*;
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let result = unsafe {
        SHGetPropertyStoreForWindow::<IPropertyStore>(hwnd).and_then(|store| {
            // Relaunch properties first: the taskbar reads them when the ID changes
            set_string_property(&store, &PKEY_AppUserModel_RelaunchCommand, &format!("\"{exe}\""))?;
            set_string_property(&store, &PKEY_AppUserModel_RelaunchIconResource, &format!("{exe},-1"))?;
            set_string_property(&store, &PKEY_AppUserModel_RelaunchDisplayNameResource, "L-Helper")?;
            set_string_property(&store, &PKEY_AppUserModel_ID, crate::AUMID)
        })
    };
    match result {
        Ok(()) => crate::log!("Taskbar: window ID {}", crate::AUMID),
        Err(e) => crate::log!("Taskbar: couldn't set the window ID: {}", e.message()),
    }
}

pub fn open_url(url: &str) {
    if !(secure::is_lenovo_download(url) || url.starts_with("ms-settings:")) {
        crate::log!("Refused to open {url}");
        return;
    }
    let explorer = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("explorer.exe");
    let _ = std::process::Command::new(explorer).arg(url).spawn();
}

pub fn message_box(text: &str, yes_no: bool) -> bool {
    let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "L-Helper\0".encode_utf16().collect();
    let style = if yes_no { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONWARNING };
    unsafe { MessageBoxW(None, windows::core::PCWSTR(w.as_ptr()), windows::core::PCWSTR(title.as_ptr()), style | MB_TOPMOST) == IDYES }
}
