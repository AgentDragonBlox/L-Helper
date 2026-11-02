//! G-Helper style on-screen notification. A tiny native window on its own thread, so it works while the UI is hidden
//! and never steals focus.

use once_cell::sync::OnceCell;
use std::sync::Mutex;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_SHOW_TOAST: u32 = WM_APP + 1;
const TIMER_HIDE: usize = 1;
const KEY: COLORREF = COLORREF(0x00FF00FF); // magenta = transparent

static THREAD_ID: OnceCell<u32> = OnceCell::new();
static PENDING: Mutex<Option<(String, (u8, u8, u8))>> = Mutex::new(None);
static CURRENT: Mutex<Option<(String, (u8, u8, u8))>> = Mutex::new(None);

pub fn show(text: &str, accent: (u8, u8, u8)) {
    if crate::config::get("osd", 1) == 0 {
        return;
    }
    *PENDING.lock().unwrap() = Some((text.to_string(), accent));
    let id = *THREAD_ID.get_or_init(start);
    unsafe {
        let _ = PostThreadMessageW(id, WM_SHOW_TOAST, WPARAM(0), LPARAM(0));
    }
}

fn start() -> u32 {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("toast".into())
        .spawn(move || unsafe {
            // Create the message queue before reporting the id, or the first toast posted to it is lost
            let mut probe = MSG::default();
            let _ = PeekMessageW(&mut probe, None, 0, 0, PM_NOREMOVE);
            let _ = tx.send(GetCurrentThreadId());
            run();
        })
        .ok();
    rx.recv().unwrap_or(0)
}

unsafe fn run() {
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let class = w!("LHelperToast");
    let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
    RegisterClassW(&wc);
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        class,
        w!("LHelper OSD"),
        WS_POPUP,
        0,
        0,
        300,
        60,
        None,
        None,
        Some(instance.into()),
        None,
    ) else {
        return;
    };
    let _ = SetLayeredWindowAttributes(hwnd, KEY, 0, LWA_COLORKEY);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if msg.message == WM_SHOW_TOAST {
            if let Some(p) = PENDING.lock().unwrap().take() {
                *CURRENT.lock().unwrap() = Some(p);
                present(hwnd);
            }
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// Popup positions for config `osd_position`, in Settings order
pub const POSITIONS: [&str; 6] = ["Bottom center", "Bottom left", "Bottom right", "Top center", "Top left", "Top right"];

/// Top-left corner of a `width` x `height` popup at `position` in the work area
fn place(position: i64, work: &RECT, width: i32, height: i32, scale: f32) -> (i32, i32) {
    let side = (24.0 * scale) as i32;
    let left = work.left + side;
    let center = work.left + (work.right - work.left - width) / 2;
    let right = work.right - width - side;
    let top = work.top + side;
    let bottom = work.bottom - height - (60.0 * scale) as i32;
    match position {
        1 => (left, bottom),
        2 => (right, bottom),
        3 => (center, top),
        4 => (left, top),
        5 => (right, top),
        _ => (center, bottom),
    }
}

fn font(scale: f32) -> HFONT {
    unsafe {
        CreateFontW((-20.0 * scale) as i32, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"))
    }
}

unsafe fn present(hwnd: HWND) {
    let Some((text, _)) = CURRENT.lock().unwrap().clone() else { return };
    let scale = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) as f32 / 96.0;
    let scale = if scale <= 0.0 { 1.0 } else { scale };

    // Measure text
    let hdc = GetDC(Some(hwnd));
    let f = font(scale);
    let old = SelectObject(hdc, f.into());
    let wtext: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wtext, &mut size);
    SelectObject(hdc, old);
    let _ = DeleteObject(f.into());
    ReleaseDC(Some(hwnd), hdc);

    let width = size.cx + (60.0 * scale) as i32;
    let height = size.cy + (28.0 * scale) as i32;

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(monitor, &mut info);
    let work = info.rcWork;
    let (x, y) = place(crate::config::get("osd_position", 0), &work, width, height, scale);

    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    let _ = InvalidateRect(Some(hwnd), None, true);
    let _ = UpdateWindow(hwnd);
    SetTimer(Some(hwnd), TIMER_HIDE, 2000, None);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let scale = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) as f32 / 96.0;

            let key = CreateSolidBrush(KEY);
            FillRect(hdc, &rc, key);
            let _ = DeleteObject(key.into());

            if let Some((text, (r, g, b))) = CURRENT.lock().unwrap().clone() {
                let accent = COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16);
                let back = CreateSolidBrush(COLORREF(0x00202020));
                let pen = CreatePen(PS_SOLID, (2.0 * scale) as i32, accent);
                let old_b = SelectObject(hdc, back.into());
                let old_p = SelectObject(hdc, pen.into());
                let radius = (24.0 * scale) as i32;
                let _ = RoundRect(hdc, rc.left + 1, rc.top + 1, rc.right - 1, rc.bottom - 1, radius, radius);

                let dot = CreateSolidBrush(accent);
                SelectObject(hdc, dot.into());
                let cy = rc.bottom / 2;
                let d = (5.0 * scale) as i32;
                let x0 = (16.0 * scale) as i32;
                let _ = Ellipse(hdc, x0, cy - d, x0 + 2 * d, cy + d);

                SelectObject(hdc, old_b);
                SelectObject(hdc, old_p);
                let _ = DeleteObject(back.into());
                let _ = DeleteObject(pen.into());
                let _ = DeleteObject(dot.into());

                let f = font(scale);
                let old_f = SelectObject(hdc, f.into());
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FFFFFF));
                let mut text_rc = RECT { left: (36.0 * scale) as i32, top: 0, right: rc.right, bottom: rc.bottom };
                let mut wtext: Vec<u16> = text.encode_utf16().collect();
                DrawTextW(hdc, &mut wtext, &mut text_rc, DT_SINGLELINE | DT_VCENTER | DT_LEFT);
                SelectObject(hdc, old_f);
                let _ = DeleteObject(f.into());
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_HIDE => {
            let _ = KillTimer(Some(hwnd), TIMER_HIDE);
            let _ = ShowWindow(hwnd, SW_HIDE);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

