//! Small Win32 control helpers for native/mod.rs: creating controls at DPI-scaled positions with the system UI font,
//! property-sheet style page dialogs, and reading / writing control state without redundant repaints.

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

const BS_PUSHBUTTON: u32 = 0;
const BS_AUTOCHECKBOX: u32 = 3;
const BS_RADIOBUTTON: u32 = 4;
const BS_GROUPBOX: u32 = 7;
const SS_NOPREFIX: u32 = 0x80;
const CBS_DROPDOWNLIST: u32 = 3;
const TBS_AUTOTICKS: u32 = 1;
const BM_GETCHECK: u32 = 0xF0;
const BM_SETCHECK: u32 = 0xF1;
const CB_ADDSTRING: u32 = 0x143;
const CB_GETCURSEL: u32 = 0x147;
const CB_RESETCONTENT: u32 = 0x14B;
const CB_SETCURSEL: u32 = 0x14E;
const CB_GETDROPPEDSTATE: u32 = 0x157;
const TBM_GETPOS: u32 = WM_USER;
const TBM_SETPOS: u32 = WM_USER + 5;
const TBM_SETRANGE: u32 = WM_USER + 6;
const TBM_SETTICFREQ: u32 = WM_USER + 20;
const TBM_SETPAGESIZE: u32 = WM_USER + 21;
const SB_SETPARTS: u32 = WM_USER + 4;
const SB_SETTEXTW: u32 = WM_USER + 11;
const DS_CONTROL: u32 = 0x0400;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Creates controls inside `parent` at DPI-scaled positions. Coordinates passed to the helper methods are in
/// 96-dpi pixels (what a dialog layout would use); `window` takes physical pixels.
pub struct Builder {
    pub parent: HWND,
    pub dpi: u32,
    pub font: HFONT,
}

impl Builder {
    /// With the system message font (Segoe UI 9 pt) at this DPI
    pub fn new(parent: HWND, dpi: u32) -> Builder {
        let mut ncm = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let font = unsafe {
            let ok = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as *mut _), 0, dpi).is_ok();
            if ok { CreateFontIndirectW(&ncm.lfMessageFont) } else { HFONT(GetStockObject(DEFAULT_GUI_FONT).0) }
        };
        Builder { parent, dpi, font }
    }

    /// Same font and DPI, another parent (a page)
    pub fn child(&self, parent: HWND) -> Builder {
        Builder { parent, dpi: self.dpi, font: self.font }
    }

    pub fn s(&self, v: i32) -> i32 {
        (v as i64 * self.dpi as i64 / 96) as i32
    }

    pub fn font(&self, h: HWND) {
        unsafe {
            SendMessageW(h, WM_SETFONT, Some(WPARAM(self.font.0 as usize)), Some(LPARAM(1)));
        }
    }

    /// Physical pixels; always a visible child
    #[allow(clippy::too_many_arguments)]
    pub fn window(&self, class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32, ex: u32) -> HWND {
        let (class, text) = (wide(class), wide(text));
        unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(ex),
                PCWSTR(class.as_ptr()),
                PCWSTR(text.as_ptr()),
                WINDOW_STYLE(style | WS_CHILD.0 | WS_VISIBLE.0),
                x,
                y,
                w,
                h,
                Some(self.parent),
                Some(HMENU(id as usize as _)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default();
            if !hwnd.0.is_null() && !self.font.0.is_null() {
                self.font(hwnd);
            }
            hwnd
        }
    }

    fn ctl(&self, class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
        self.window(class, text, style, self.s(x), self.s(y), self.s(w), self.s(h), id, 0)
    }

    pub fn group(&self, text: &str, x: i32, y: i32, w: i32, h: i32) -> HWND {
        self.ctl("BUTTON", text, BS_GROUPBOX, x, y, w, h, 0)
    }

    pub fn label(&self, text: &str, x: i32, y: i32, w: i32, h: i32) -> HWND {
        self.ctl("STATIC", text, SS_NOPREFIX, x, y, w, h, 0)
    }

    /// A radio button the program checks itself (the state comes from the laptop, not from the click)
    #[allow(clippy::too_many_arguments)]
    pub fn radio(&self, text: &str, x: i32, y: i32, w: i32, h: i32, id: i32, first: bool) -> HWND {
        let group = if first { WS_GROUP.0 } else { 0 };
        self.ctl("BUTTON", text, BS_RADIOBUTTON | WS_TABSTOP.0 | group, x, y, w, h, id)
    }

    /// A row of radio buttons across `width`, each as wide as its text needs (relative to the others)
    pub fn radio_row(&self, labels: &[&str], x: i32, y: i32, width: i32, first_id: i32) -> Vec<HWND> {
        let weight = |t: &&str| t.chars().count() as i32 + 6;
        let total: i32 = labels.iter().map(weight).sum::<i32>().max(1);
        let mut at = x;
        labels
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let w = width * weight(t) / total;
                let h = self.radio(t, at, y, w - 2, 22, first_id + i as i32, i == 0);
                at += w;
                h
            })
            .collect()
    }

    pub fn check(&self, text: &str, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
        self.ctl("BUTTON", text, BS_AUTOCHECKBOX | WS_TABSTOP.0, x, y, w, h, id)
    }

    pub fn button(&self, text: &str, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
        self.ctl("BUTTON", text, BS_PUSHBUTTON | WS_TABSTOP.0, x, y, w, h, id)
    }

    /// Drop-down list; `h` is the height with the list open
    pub fn combo(&self, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
        self.ctl("COMBOBOX", "", CBS_DROPDOWNLIST | WS_VSCROLL.0 | WS_TABSTOP.0, x, y, w, h, id)
    }

    pub fn add_item(&self, combo: HWND, text: &str) {
        add_item(combo, text);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn trackbar(&self, x: i32, y: i32, w: i32, h: i32, id: i32, min: i32, max: i32) -> HWND {
        let t = self.ctl("msctls_trackbar32", "", TBS_AUTOTICKS | WS_TABSTOP.0, x, y, w, h, id);
        unsafe {
            SendMessageW(t, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM(((max as isize) << 16) | (min as isize & 0xFFFF))));
            SendMessageW(t, TBM_SETTICFREQ, Some(WPARAM(((max - min) / 10).max(1) as usize)), Some(LPARAM(0)));
            SendMessageW(t, TBM_SETPAGESIZE, Some(WPARAM(0)), Some(LPARAM(((max - min) / 10).max(1) as isize)));
        }
        t
    }

    pub fn set_text(&self, h: HWND, text: &str) {
        set_text(h, text);
    }

    pub fn status_bar(&self) -> HWND {
        let bar = self.window("msctls_statusbar32", "", 0, 0, 0, 0, 0, 0, 0);
        unsafe {
            // Two parts: model, then the last message
            let parts = [self.s(330), -1];
            SendMessageW(bar, SB_SETPARTS, Some(WPARAM(2)), Some(LPARAM(parts.as_ptr() as isize)));
            SendMessageW(bar, WM_SIZE, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        bar
    }

    pub fn height_of(&self, h: HWND) -> i32 {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(h, &mut r);
        }
        r.bottom - r.top
    }

    /// A tab page: a child dialog without a template, so Windows draws the tab-textured background behind its
    /// controls and themes their text, like a property sheet page. Notifications go to this builder's parent.
    pub fn page(&self, x: i32, y: i32, w: i32, h: i32) -> HWND {
        // DLGTEMPLATE: style, extended style, item count, x, y, cx, cy, then empty menu, class and title
        let style = WS_CHILD.0 | WS_CLIPCHILDREN.0 | DS_CONTROL;
        let template: [u16; 12] = [style as u16, (style >> 16) as u16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let page = CreateDialogIndirectParamW(Some(instance.into()), template.as_ptr() as *const DLGTEMPLATE, Some(self.parent), Some(page_proc), LPARAM(0)).unwrap_or_default();
            let _ = SetWindowPos(page, Some(HWND_TOP), x, y, w, h, SWP_NOACTIVATE);
            page
        }
    }
}

unsafe extern "system" fn page_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            1
        }
        WM_COMMAND | WM_HSCROLL | WM_NOTIFY => {
            if let Ok(parent) = GetParent(hwnd) {
                SendMessageW(parent, msg, Some(wparam), Some(lparam));
            }
            1
        }
        _ => 0,
    }
}

// ---------------- reading and writing control state ----------------

fn gone(h: HWND) -> bool {
    h.0.is_null()
}

pub fn set_text(h: HWND, text: &str) {
    if gone(h) {
        return;
    }
    unsafe {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(h, &mut buf) as usize;
        if String::from_utf16_lossy(&buf[..len]) != text {
            let w = wide(text);
            let _ = SetWindowTextW(h, PCWSTR(w.as_ptr()));
        }
    }
}

pub fn is_checked(h: HWND) -> bool {
    !gone(h) && unsafe { SendMessageW(h, BM_GETCHECK, None, None).0 == 1 }
}

pub fn set_check(h: HWND, on: bool) {
    if !gone(h) && is_checked(h) != on {
        unsafe {
            SendMessageW(h, BM_SETCHECK, Some(WPARAM(on as usize)), Some(LPARAM(0)));
        }
    }
}

pub fn enable(h: HWND, on: bool) {
    if !gone(h) {
        unsafe {
            let _ = EnableWindow(h, on);
        }
    }
}

pub fn show(h: HWND, on: bool) {
    if !gone(h) {
        unsafe {
            let _ = ShowWindow(h, if on { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

pub fn reset_combo(h: HWND) {
    if !gone(h) {
        unsafe {
            SendMessageW(h, CB_RESETCONTENT, None, None);
        }
    }
}

pub fn add_item(h: HWND, text: &str) {
    if !gone(h) {
        let w = wide(text);
        unsafe {
            SendMessageW(h, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(w.as_ptr() as isize)));
        }
    }
}

pub fn get_sel(h: HWND) -> Option<usize> {
    if gone(h) {
        return None;
    }
    let i = unsafe { SendMessageW(h, CB_GETCURSEL, None, None).0 };
    (i >= 0).then_some(i as usize)
}

/// Leaves a list that's open alone (the user is choosing from it)
pub fn set_sel(h: HWND, index: Option<usize>) {
    if gone(h) || unsafe { SendMessageW(h, CB_GETDROPPEDSTATE, None, None).0 } != 0 || get_sel(h) == index {
        return;
    }
    unsafe {
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(index.map(|i| i as usize).unwrap_or(usize::MAX))), Some(LPARAM(0)));
    }
}

pub fn get_trackbar(h: HWND) -> i32 {
    if gone(h) { 0 } else { unsafe { SendMessageW(h, TBM_GETPOS, None, None).0 as i32 } }
}

/// Not while the thumb is being dragged
pub fn set_trackbar(h: HWND, pos: i32) {
    if gone(h) || unsafe { GetCapture() } == h || get_trackbar(h) == pos {
        return;
    }
    unsafe {
        SendMessageW(h, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(pos as isize)));
    }
}

pub fn set_status(bar: HWND, left: &str, right: &str) {
    if gone(bar) {
        return;
    }
    for (part, text) in [(0usize, left), (1usize, right)] {
        let w = wide(text);
        unsafe {
            SendMessageW(bar, SB_SETTEXTW, Some(WPARAM(part)), Some(LPARAM(w.as_ptr() as isize)));
        }
    }
}
