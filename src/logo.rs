//! The L-Helper logo: blue (the brand, for the window, Alt-Tab and the exe), black and white (tray and taskbar,
//! following the taskbar's light / dark theme the way Windows' own icons do)

use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Logo {
    Blue,
    Black,
    White,
}

fn source(kind: Logo) -> &'static [u8] {
    match kind {
        Logo::Blue => include_bytes!("../assets/logo_blue.png"),
        Logo::Black => include_bytes!("../assets/logo_black.png"),
        Logo::White => include_bytes!("../assets/logo_white.png"),
    }
}

/// RGBA pixels at `size` x `size` (cached per kind and size)
pub fn rgba(kind: Logo, size: u32) -> Vec<u8> {
    static CACHE: Mutex<Vec<((Logo, u32), Vec<u8>)>> = Mutex::new(Vec::new());
    let mut cache = CACHE.lock().unwrap();
    if let Some((_, px)) = cache.iter().find(|(k, _)| *k == (kind, size)) {
        return px.clone();
    }
    let px = image::load_from_memory(source(kind))
        .map(|img| image::imageops::resize(&img.to_rgba8(), size, size, image::imageops::FilterType::Lanczos3).into_raw())
        .unwrap_or_else(|_| vec![0; (size * size * 4) as usize]);
    cache.push(((kind, size), px.clone()));
    px
}

/// Taskbar / tray theme ("Choose your default Windows mode"), which can differ from the apps theme
#[cfg(windows)]
pub fn taskbar_is_light() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("SystemUsesLightTheme"))
        .map(|v| v == 1)
        .unwrap_or(false)
}

/// Monochrome logo that reads on the current taskbar: black on light, white on dark
#[cfg(windows)]
pub fn for_taskbar() -> Logo {
    if taskbar_is_light() { Logo::Black } else { Logo::White }
}

/// Windows icon handle from the logo (32-bit color with alpha); the caller keeps it for the window's lifetime
#[cfg(windows)]
pub fn hicon(kind: Logo, size: u32) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
    let rgba = rgba(kind, size);
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len());
        for (d, s) in dst.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
        }
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() }).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// The main window's icons: the taskbar (big icon) gets the black / white logo for the taskbar theme, the title
/// bar (small icon) the blue one. Posted, so any thread can call it. The few icons replaced on theme
/// changes are left to the process.
#[cfg(windows)]
pub fn apply_window_icons(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, ICON_BIG, ICON_SMALL, WM_SETICON};
    if hwnd.0.is_null() {
        return;
    }
    unsafe {
        if let Some(big) = hicon(for_taskbar(), 64) {
            let _ = PostMessageW(Some(hwnd), WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(big.0 as isize));
        }
        if let Some(small) = hicon(Logo::Blue, 32) {
            let _ = PostMessageW(Some(hwnd), WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(small.0 as isize));
        }
    }
}
