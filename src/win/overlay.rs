//! Game overlay, NVIDIA-overlay style: CPU, GPU, RAM and VRAM usage, temperatures and power in a corner.
//! A native click-through, always-on-top window on its own thread that never takes focus. Drawn with per-pixel
//! alpha (UpdateLayeredWindow), so the background can fade to fully transparent while the text stays solid, with a
//! soft shadow that keeps it readable over bright scenes. Shows over windowed and borderless games (exclusive
//! fullscreen draws over everything). Toggle: Ctrl+Shift+O, the tray menu or Settings.
//!
//! Settings: overlay_bg (background opacity %, default 80), overlay_scale (size %, default 100),
//! overlay_corner (0 top right, 1 top left, 2 bottom right, 3 bottom left), overlay_layout (0 row, 1 column).

use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentThreadId, GetSystemTimes};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config;

const WM_TOGGLE: u32 = WM_APP + 3;
const WM_REDRAW: u32 = WM_APP + 4;
const TIMER_UPDATE: usize = 1;

const BACK: (f32, f32, f32) = (18.0, 18.0, 18.0);
const LABEL: (f32, f32, f32) = (165.0, 165.0, 165.0);
const VALUE: (f32, f32, f32) = (255.0, 255.0, 255.0);
const DIVIDER: (f32, f32, f32) = (95.0, 95.0, 95.0);

static THREAD_ID: OnceCell<u32> = OnceCell::new();
static VISIBLE: AtomicBool = AtomicBool::new(false);
static CPU_TIMES: Mutex<Option<(u64, u64)>> = Mutex::new(None);

pub fn visible() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

fn post(msg: u32) {
    let id = *THREAD_ID.get_or_init(start);
    unsafe {
        let _ = PostThreadMessageW(id, msg, WPARAM(0), LPARAM(0));
    }
}

pub fn toggle() {
    post(WM_TOGGLE);
}

/// Redraw now with the current settings (size, transparency, position, layout)
pub fn refresh() {
    if visible() {
        post(WM_REDRAW);
    }
}

fn start() -> u32 {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("overlay".into()).spawn(move || unsafe { run(tx) }).ok();
    rx.recv().unwrap_or(0)
}

/// Reports the thread id only once the window (and so the message queue) exists: a message posted to a thread
/// without a queue is lost
unsafe fn run(ready: std::sync::mpsc::Sender<u32>) {
    let mut probe = MSG::default();
    let _ = PeekMessageW(&mut probe, None, 0, 0, PM_NOREMOVE);
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let class = w!("LHelperOverlay");
    let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
    RegisterClassW(&wc);
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        class,
        w!("LHelper Overlay"),
        WS_POPUP,
        0,
        0,
        10,
        10,
        None,
        None,
        Some(instance.into()),
        None,
    ) else {
        let _ = ready.send(0);
        return;
    };
    let _ = ready.send(GetCurrentThreadId());

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        match msg.message {
            WM_TOGGLE => {
                let show = !visible();
                VISIBLE.store(show, Ordering::SeqCst);
                if show {
                    update(hwnd);
                    SetTimer(Some(hwnd), TIMER_UPDATE, 1000, None);
                } else {
                    let _ = KillTimer(Some(hwnd), TIMER_UPDATE);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                crate::log!("Overlay: {show}");
            }
            WM_REDRAW => update(hwnd),
            _ => {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wparam.0 == TIMER_UPDATE {
        update(hwnd);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ---------------- readings ----------------

fn cpu_load() -> Option<u32> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    let (idle, total) = (ft(idle), ft(kernel) + ft(user));
    let mut last = CPU_TIMES.lock().unwrap();
    let load = last.map(|(li, lt)| {
        let (di, dt) = (idle.saturating_sub(li), total.saturating_sub(lt));
        if dt == 0 { 0 } else { (100 - di * 100 / dt) as u32 }
    });
    *last = Some((idle, total));
    load
}

fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
}

/// (label, values) per group: CPU, GPU, RAM, VRAM
fn readings() -> Vec<(&'static str, Vec<String>)> {
    let s = crate::app::state().sensors.clone();
    let mut groups = Vec::new();

    let mut cpu = Vec::new();
    if let Some(l) = cpu_load() {
        cpu.push(format!("{l}%"));
    }
    if let Some(t) = s.cpu_temp {
        cpu.push(format!("{t}°C"));
    }
    if let Some(w) = s.cpu_power {
        cpu.push(format!("{w:.1} W"));
    }
    groups.push(("CPU", cpu));

    let usage = crate::lenovo::sensors::nvidia_usage();
    let mut gpu = Vec::new();
    if s.gpu_asleep {
        gpu.push("asleep".into());
    } else {
        if let Some((load, _, _)) = usage {
            gpu.push(format!("{load}%"));
        }
        if let Some(t) = s.gpu_temp {
            gpu.push(format!("{t}°C"));
        }
        if let Some(w) = s.gpu_power {
            gpu.push(format!("{w:.1} W"));
        }
    }
    groups.push(("GPU", gpu));

    let mut mem = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut mem) }.is_ok() {
        groups.push(("RAM", vec![format!("{}/{} GB", gb(mem.ullTotalPhys - mem.ullAvailPhys), gb(mem.ullTotalPhys))]));
    }
    if let Some((_, used, total)) = usage {
        groups.push(("VRAM", vec![format!("{}/{} GB", gb(used), gb(total))]));
    }
    groups.retain(|g| !g.1.is_empty());
    groups
}

// ---------------- drawing ----------------

/// A piece of text placed on the canvas
struct Run {
    text: String,
    bold: bool,
    color: (f32, f32, f32),
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn font(size: i32, bold: bool) -> HFONT {
    unsafe {
        let weight = if bold { FW_SEMIBOLD.0 } else { FW_NORMAL.0 };
        // Grayscale antialiasing: the coverage becomes the text's alpha (ClearType needs an opaque background)
        CreateFontW(-size, 0, 0, 0, weight as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Segoe UI"))
    }
}

unsafe fn measure(hdc: HDC, text: &str, size: i32, bold: bool) -> (i32, i32) {
    let f = font(size, bold);
    let old = SelectObject(hdc, f.into());
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut s = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &w, &mut s);
    SelectObject(hdc, old);
    let _ = DeleteObject(f.into());
    (s.cx, s.cy)
}

/// Places the runs for a row or a column layout; returns the content size
unsafe fn layout(hdc: HDC, groups: &[(&'static str, Vec<String>)], size: i32, column: bool) -> (Vec<Run>, i32, i32) {
    let gap = size / 2;
    let mut runs = Vec::new();
    let (mut x, mut y, mut width, mut height) = (0, 0, 0, 0);
    let label_w = if column { groups.iter().map(|g| measure(hdc, g.0, size, false).0).max().unwrap_or(0) + gap } else { 0 };
    for (i, (label, values)) in groups.iter().enumerate() {
        if column {
            x = 0;
        } else if i > 0 {
            let (w, h) = measure(hdc, "|", size, false);
            runs.push(Run { text: "|".into(), bold: false, color: DIVIDER, x, y, w, h });
            x += w + gap;
        }
        let (w, h) = measure(hdc, label, size, false);
        runs.push(Run { text: (*label).into(), bold: false, color: LABEL, x, y, w, h });
        x += if column { label_w } else { w + gap };
        let mut line_h = h;
        for (j, v) in values.iter().enumerate() {
            let (w, h) = measure(hdc, v, size, true);
            runs.push(Run { text: v.clone(), bold: true, color: VALUE, x, y, w, h });
            x += w + if j + 1 < values.len() { gap } else { 0 };
            line_h = line_h.max(h);
        }
        width = width.max(x);
        height = y + line_h;
        if column {
            y += line_h + size / 5;
        } else {
            x += gap;
        }
    }
    (runs, width, height)
}

fn dpi_scale(hwnd: HWND) -> f32 {
    let s = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) } as f32 / 96.0;
    if s <= 0.0 { 1.0 } else { s }
}

/// Coverage (0..1) of a rounded rectangle at a pixel, antialiased over one pixel
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = x.clamp(r, w - r);
    let cy = y.clamp(r, h - r);
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0.0, 1.0)
}

unsafe fn update(hwnd: HWND) {
    let groups = readings();
    let dpi = dpi_scale(hwnd);
    let scale = config::get("overlay_scale", 100).clamp(60, 200) as f32 / 100.0;
    let bg = config::get("overlay_bg", 80).clamp(0, 100) as f32 / 100.0;
    let column = config::get("overlay_layout", 0) == 1;
    let size = (14.5 * dpi * scale).round() as i32;

    let screen = GetDC(None);
    let mem = CreateCompatibleDC(Some(screen));
    let (runs, content_w, content_h) = layout(mem, &groups, size, column);
    let (pad_x, pad_y) = ((size as f32 * 1.0) as i32, (size as f32 * 0.55) as i32);
    let (width, height) = (content_w + 2 * pad_x, content_h + 2 * pad_y);
    if width <= 0 || height <= 0 {
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        return;
    }

    // 32-bit top-down DIB: draw the text white on black to get its coverage
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let Ok(bitmap) = CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        return;
    };
    let old_bitmap = SelectObject(mem, bitmap.into());
    let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (width * height * 4) as usize);
    pixels.fill(0);
    SetBkMode(mem, TRANSPARENT);
    SetTextColor(mem, COLORREF(0x00FFFFFF));
    for r in &runs {
        let f = font(size, r.bold);
        let old = SelectObject(mem, f.into());
        let mut rc = RECT { left: pad_x + r.x, top: pad_y + r.y, right: pad_x + r.x + r.w + 2, bottom: pad_y + r.y + r.h };
        let mut text: Vec<u16> = r.text.encode_utf16().collect();
        DrawTextW(mem, &mut text, &mut rc, DT_SINGLELINE | DT_LEFT | DT_TOP | DT_NOPREFIX);
        SelectObject(mem, old);
        let _ = DeleteObject(f.into());
    }
    let _ = GdiFlush();

    // Text coverage, its color by run, and a soft shadow (spread coverage) for readability without a background
    let (w, h) = (width as usize, height as usize);
    let coverage: Vec<f32> = (0..w * h).map(|i| pixels[i * 4 + 1] as f32 / 255.0).collect();
    let mut color = vec![VALUE; w * h];
    for r in &runs {
        for y in (pad_y + r.y).max(0) as usize..((pad_y + r.y + r.h) as usize).min(h) {
            for x in (pad_x + r.x).max(0) as usize..((pad_x + r.x + r.w + 2) as usize).min(w) {
                color[y * w + x] = r.color;
            }
        }
    }
    let spread = (size / 9).max(1) as i32;
    let shadow_strength = 0.85 * (1.0 - bg);
    let radius = (size as f32 * 0.7).min(height as f32 / 2.0);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut shadow = 0.0f32;
            if shadow_strength > 0.0 {
                for dy in -spread..=spread {
                    for dx in -spread..=spread {
                        let (sx, sy) = (x as i32 + dx, y as i32 + dy);
                        if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                            shadow = shadow.max(coverage[sy as usize * w + sx as usize]);
                        }
                    }
                }
            }
            // background, then shadow, then text (premultiplied)
            let mut a = bg * rounded_rect(x as f32 + 0.5, y as f32 + 0.5, width as f32, height as f32, radius);
            let (mut r, mut g, mut b) = (BACK.0 * a, BACK.1 * a, BACK.2 * a);
            let sa = shadow * shadow_strength;
            r *= 1.0 - sa;
            g *= 1.0 - sa;
            b *= 1.0 - sa;
            a = sa + a * (1.0 - sa);
            let ta = coverage[i];
            let c = color[i];
            r = c.0 * ta + r * (1.0 - ta);
            g = c.1 * ta + g * (1.0 - ta);
            b = c.2 * ta + b * (1.0 - ta);
            a = ta + a * (1.0 - ta);
            pixels[i * 4] = b.round().clamp(0.0, 255.0) as u8;
            pixels[i * 4 + 1] = g.round().clamp(0.0, 255.0) as u8;
            pixels[i * 4 + 2] = r.round().clamp(0.0, 255.0) as u8;
            pixels[i * 4 + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    // Chosen corner of the primary monitor (over games, not just the work area)
    let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(monitor, &mut mi);
    let m = mi.rcMonitor;
    let margin = (12.0 * dpi) as i32;
    let (x, y) = match config::get("overlay_corner", 0) {
        1 => (m.left + margin, m.top + margin),
        2 => (m.right - width - margin, m.bottom - height - margin),
        3 => (m.left + margin, m.bottom - height - margin),
        _ => (m.right - width - margin, m.top + margin),
    };

    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    let position = POINT { x, y };
    let size_px = SIZE { cx: width, cy: height };
    let origin = POINT::default();
    let _ = UpdateLayeredWindow(hwnd, Some(screen), Some(&position), Some(&size_px), Some(mem), Some(&origin), COLORREF(0), Some(&blend), ULW_ALPHA);
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);

    SelectObject(mem, old_bitmap);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(mem);
    ReleaseDC(None, screen);
}
