//! Classic Windows interface: the same controls as the modern (egui) window, drawn by Windows itself. A tab control
//! with property-sheet style pages, group boxes, radio buttons, combo boxes and a status bar, in the system's UI font
//! (Segoe UI) with visual styles, like Control Panel or a ThinkPad utility. Plain Win32 through the `windows` crate
//! (no extra dependency; native-windows-gui is unmaintained and brings a second copy of the Windows bindings).
//!
//! Turned on with `lhelper.exe -native`, or Settings → "Classic Windows interface" (config ui_style = native).
//! Like the egui window it only reads `app::state()` and queues work with `app::job` / `app::set_*`; a 1 s timer
//! refreshes the controls while the window is on screen, and `poke()` after state changes.
//!
//! Not in this interface yet: the fan curve editor, power limit sliders, GPU tuning and the lighting preset editor
//! (the modern interface has them: Settings → "Modern interface").

mod controls;

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{RedrawWindow, COLOR_BTNFACE, HBRUSH, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app;
use crate::config;
use crate::gpu::GpuMode;
use controls::*;

/// Window handle for `poke()` from any thread
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);

const WM_REFRESH: u32 = WM_APP + 10;
const TIMER_REFRESH: usize = 1;
const TIMER_SHOTS: usize = 2;

// Control ids
const ID_MODE: i32 = 100; // + index
const ID_GPU: i32 = 120;
const ID_BAT: i32 = 140;
const ID_WHITE: i32 = 150;
const ID_AUTO: i32 = 160;
const ID_RATE: i32 = 161;
const ID_RES: i32 = 162;
const ID_OD: i32 = 163;
const ID_OLED: i32 = 164;
const ID_PRESET: i32 = 170;
const ID_BRIGHT: i32 = 171;
const ID_LIGHT: i32 = 172;
const ID_NEXT: i32 = 173;
const ID_LOGO: i32 = 174;
const ID_STARTUP: i32 = 200;
const ID_TRAY: i32 = 201;
const ID_MINIMIZED: i32 = 202;
const ID_OSD: i32 = 203;
const ID_OSDPOS: i32 = 204;
const ID_OVERLAY: i32 = 205;
const ID_MODERN: i32 = 206;
const ID_AUTOMODE: i32 = 207;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Performance,
    Gpu,
    Display,
    Keyboard,
    Battery,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::Performance => "Performance",
            Page::Gpu => "Graphics",
            Page::Display => "Display",
            Page::Keyboard => "Keyboard",
            Page::Battery => "Battery",
            Page::Settings => "Settings",
        }
    }
}

#[derive(Clone, Default)]
struct Ctrls {
    modes: Vec<HWND>,
    mode_hint: HWND,
    sensors: [HWND; 4],
    overlay: HWND,
    gpus: Vec<HWND>,
    gpu_hint: HWND,
    auto: HWND,
    rate: HWND,
    res: HWND,
    od: HWND,
    oled: HWND,
    preset: HWND,
    bright: HWND,
    bright_val: HWND,
    light: HWND,
    logo: HWND,
    whites: Vec<HWND>,
    bats: Vec<HWND>,
    bat_info: HWND,
    startup: HWND,
    tray: HWND,
    minimized: HWND,
    osd: HWND,
    osd_pos: HWND,
    auto_mode: HWND,
}

#[derive(Clone)]
struct Ui {
    tab: HWND,
    status: HWND,
    /// Tab order and each page's dialog window
    pages: Vec<(Page, HWND)>,
    c: Ctrls,
    modes: Vec<crate::mode::Mode>,
    gpu_modes: Vec<GpuMode>,
    battery_modes: Vec<BatteryChoice>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BatteryChoice(crate::lenovo::energy::BatteryMode);

/// What the combo boxes currently list, so a refresh only repopulates them when the laptop's lists change
#[derive(Default)]
struct Lists {
    rates: Vec<u32>,
    resolutions: Vec<(u32, u32)>,
    presets: usize,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static LISTS: RefCell<Lists> = RefCell::new(Lists::default());
    static SHOTS: RefCell<Option<(std::path::PathBuf, usize)>> = const { RefCell::new(None) };
}

fn ui() -> Option<Ui> {
    UI.with(|u| u.borrow().clone())
}

/// Wakes the window to re-read `app::state()` (any thread)
pub fn poke() {
    let h = HWND_RAW.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as _)), WM_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

/// Runs the window on this thread until it closes. `screenshots`: capture every page to PNGs in that folder and exit.
pub fn run(title: &str, start_hidden: bool, screenshots: Option<std::path::PathBuf>) {
    unsafe {
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_TAB_CLASSES | ICC_BAR_CLASSES | ICC_STANDARD_CLASSES });
        crate::win::set_menu_theme(crate::win::dark_mode());

        let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let class = w!("LHelperNative");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as isize as _),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let wide_title = wide(title);
        // Fixed-size dialog-style window: caption, system menu, minimize button
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = match CreateWindowExW(WS_EX_CONTROLPARENT, class, PCWSTR(wide_title.as_ptr()), style, CW_USEDEFAULT, CW_USEDEFAULT, 520, 520, None, None, Some(instance.into()), None) {
            Ok(h) => h,
            Err(e) => {
                crate::log!("Native window failed: {e}");
                return;
            }
        };
        HWND_RAW.store(hwnd.0 as isize, Ordering::Relaxed);
        app::set_hwnd(hwnd.0 as isize);
        crate::win::set_window_app_id(hwnd);
        crate::logo::apply_window_icons(hwnd);
        crate::win::set_dark_title_bar(hwnd, false);

        layout(hwnd);
        refresh();
        SetTimer(Some(hwnd), TIMER_REFRESH, 1000, None);
        if let Some(dir) = screenshots {
            SHOTS.with(|s| *s.borrow_mut() = Some((dir, 0)));
            SetTimer(Some(hwnd), TIMER_SHOTS, 2500, None);
        }
        if !start_hidden {
            crate::win::show_near_tray(hwnd);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab / arrow keys between controls, Esc and Enter like a dialog
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// (Re)builds the controls for the window's current DPI
unsafe fn layout(hwnd: HWND) {
    if let Some(old) = ui() {
        let _ = DestroyWindow(old.tab);
        let _ = DestroyWindow(old.status);
        for (_, p) in &old.pages {
            let _ = DestroyWindow(*p);
        }
    }
    LISTS.with(|l| *l.borrow_mut() = Lists::default());
    let dpi = GetDpiForWindow(hwnd).max(96);
    let b = Builder::new(hwnd, dpi);

    // Client area: 8 px margin, tab control, status bar
    let (client_w, client_h) = (500, 400);
    let mut frame = RECT { left: 0, top: 0, right: b.s(client_w), bottom: b.s(client_h) };
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let _ = AdjustWindowRectExForDpi(&mut frame, style, false, WS_EX_CONTROLPARENT, dpi);
    let _ = SetWindowPos(hwnd, None, 0, 0, frame.right - frame.left, frame.bottom - frame.top, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);

    let status = b.status_bar();
    let status_h = b.height_of(status);
    let tab = b.window("SysTabControl32", "", (WS_TABSTOP.0 | WS_CLIPSIBLINGS.0) | 0, b.s(8), b.s(8), b.s(client_w - 16), b.s(client_h) - status_h - b.s(16), 0, WS_EX_CONTROLPARENT.0);
    b.font(tab);

    let m = crate::lenovo::machine::get();
    let modes = crate::mode::available();
    let gpu_modes = if crate::gpu::supported() { crate::gpu::available() } else { Vec::new() };
    let battery_modes: Vec<BatteryChoice> = if m.has_energy_driver { [crate::lenovo::energy::BatteryMode::Conservation, crate::lenovo::energy::BatteryMode::Normal, crate::lenovo::energy::BatteryMode::RapidCharge].into_iter().map(BatteryChoice).collect() } else { Vec::new() };

    let mut order = vec![Page::Performance];
    if !gpu_modes.is_empty() {
        order.push(Page::Gpu);
    }
    order.push(Page::Display);
    if crate::lighting::is_supported() || crate::lenovo::keyboard::has_spectrum() || m.keyboard_levels > 0 {
        order.push(Page::Keyboard);
    }
    if !battery_modes.is_empty() {
        order.push(Page::Battery);
    }
    order.push(Page::Settings);

    for (i, p) in order.iter().enumerate() {
        let text = wide(p.title());
        let mut item = TCITEMW { mask: TCIF_TEXT, pszText: windows::core::PWSTR(text.as_ptr() as *mut u16), ..Default::default() };
        SendMessageW(tab, TCM_INSERTITEMW, Some(WPARAM(i)), Some(LPARAM(&mut item as *mut _ as isize)));
    }
    // The tab's page area
    let mut area = RECT::default();
    let _ = GetClientRect(tab, &mut area);
    SendMessageW(tab, TCM_ADJUSTRECT, Some(WPARAM(0)), Some(LPARAM(&mut area as *mut _ as isize)));
    let mut tab_rect = RECT::default();
    let _ = GetWindowRect(tab, &mut tab_rect);
    let mut origin = windows::Win32::Foundation::POINT { x: tab_rect.left, y: tab_rect.top };
    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut origin);
    let (px, py, pw, ph) = (origin.x + area.left, origin.y + area.top, area.right - area.left, area.bottom - area.top);

    let mut c = Ctrls::default();
    let mut pages = Vec::new();
    for p in &order {
        let page = b.page(px, py, pw, ph);
        let pb = b.child(page);
        match p {
            Page::Performance => build_performance(&pb, &mut c, &modes),
            Page::Gpu => build_gpu(&pb, &mut c, &gpu_modes),
            Page::Display => build_display(&pb, &mut c),
            Page::Keyboard => build_keyboard(&pb, &mut c),
            Page::Battery => build_battery(&pb, &mut c, &battery_modes),
            Page::Settings => build_settings(&pb, &mut c),
        }
        pages.push((*p, page));
    }
    let _ = SetWindowPos(tab, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);

    UI.with(|u| *u.borrow_mut() = Some(Ui { tab, status, pages, c, modes, gpu_modes, battery_modes }));
    select_page(0);
}

fn build_performance(b: &Builder, c: &mut Ctrls, modes: &[crate::mode::Mode]) {
    b.group("Performance mode", 8, 6, 438, 96);
    let names: Vec<&str> = modes.iter().map(|m| m.name()).collect();
    c.modes = b.radio_row(&names, 20, 28, 414, ID_MODE);
    c.mode_hint = b.label("", 20, 62, 414, 32);
    if modes.is_empty() {
        b.set_text(c.mode_hint, "This laptop doesn't report performance modes.");
    }
    b.group("Sensors", 8, 110, 438, 112);
    for (i, name) in ["Processor", "Graphics", "Fans", "Power"].iter().enumerate() {
        b.label(name, 20, 132 + i as i32 * 22, 90, 18);
        c.sensors[i] = b.label("-", 116, 132 + i as i32 * 22, 318, 18);
    }
    c.overlay = b.check("Show the game overlay (Ctrl+Shift+O)", 8, 234, 400, 22, ID_OVERLAY);
    if crate::lenovo::machine::get().supports_custom {
        b.label("Fan curves and power limits are in the modern interface (Settings, below).", 8, 266, 438, 32);
    }
}

fn build_gpu(b: &Builder, c: &mut Ctrls, modes: &[GpuMode]) {
    b.group("GPU mode", 8, 6, 438, 96);
    let names: Vec<&str> = modes.iter().map(|g| g.name()).collect();
    c.gpus = b.radio_row(&names, 20, 28, 414, ID_GPU);
    c.gpu_hint = b.label("", 20, 62, 414, 32);
    b.label("Switching to or from Ultimate changes the display MUX and needs a restart.", 8, 114, 438, 32);
}

fn build_display(b: &Builder, c: &mut Ctrls) {
    c.auto = b.check("Automatic: lowest refresh rate on battery, highest when plugged in", 8, 8, 440, 22, ID_AUTO);
    b.label("Refresh rate:", 8, 44, 130, 18);
    c.rate = b.combo(140, 40, 140, 200, ID_RATE);
    b.label("Resolution:", 8, 76, 130, 18);
    c.res = b.combo(140, 72, 180, 300, ID_RES);
    c.od = b.check("Panel overdrive (faster pixel response)", 8, 112, 400, 22, ID_OD);
    c.oled = b.check("OLED burn-in protection (hide the taskbar, turn the screen off sooner)", 8, 140, 440, 22, ID_OLED);
}

fn build_keyboard(b: &Builder, c: &mut Ctrls) {
    use crate::lenovo::{keyboard, machine};
    if crate::lighting::is_supported() {
        b.label("Preset:", 8, 14, 90, 18);
        c.preset = b.combo(104, 10, 250, 300, ID_PRESET);
        b.label("Brightness:", 8, 50, 90, 18);
        c.bright = b.trackbar(100, 44, 270, 28, ID_BRIGHT, 0, 100);
        c.bright_val = b.label("", 376, 50, 60, 18);
        c.light = b.check("L-Helper controls the keyboard lighting", 8, 88, 400, 22, ID_LIGHT);
        b.button("Next preset", 8, 120, 120, 26, ID_NEXT);
        b.label("Preset parameters and colors are in the modern interface.", 8, 160, 438, 20);
    } else if keyboard::has_spectrum() {
        b.label("Brightness:", 8, 14, 90, 18);
        c.bright = b.trackbar(100, 8, 270, 28, ID_BRIGHT, 0, 9);
        c.bright_val = b.label("", 376, 14, 60, 18);
        c.logo = b.check("Logo light", 8, 52, 300, 22, ID_LOGO);
    } else {
        b.group("Keyboard backlight", 8, 6, 438, 60);
        let max = machine::get().keyboard_levels.clamp(1, 5);
        let width = 414 / (max + 1);
        for i in 0..=max {
            let name = match (i, max) {
                (0, _) => "Off".to_string(),
                (1, 2) => "Low".to_string(),
                (2, 2) => "High".to_string(),
                _ => i.to_string(),
            };
            c.whites.push(b.radio(&name, 20 + i * width, 28, width - 4, 22, ID_WHITE + i, i == 0));
        }
    }
}

fn build_battery(b: &Builder, c: &mut Ctrls, modes: &[BatteryChoice]) {
    b.group("Charging mode", 8, 6, 438, 112);
    let text = |m: &BatteryChoice| match m.0 {
        crate::lenovo::energy::BatteryMode::Conservation => "Conservation: keeps the battery around 80% to extend its lifespan",
        crate::lenovo::energy::BatteryMode::Normal => "Normal: charges to 100%",
        crate::lenovo::energy::BatteryMode::RapidCharge => "Rapid Charge: charges faster, more battery wear",
    };
    for (i, m) in modes.iter().enumerate() {
        c.bats.push(b.radio(text(m), 20, 28 + i as i32 * 26, 414, 22, ID_BAT + i as i32, i == 0));
    }
    c.bat_info = b.label("", 8, 130, 438, 20);
}

fn build_settings(b: &Builder, c: &mut Ctrls) {
    c.startup = b.check("Run L-Helper when I sign in (admin, no prompt)", 8, 8, 440, 22, ID_STARTUP);
    c.minimized = b.check("Start minimized to the tray", 8, 34, 440, 22, ID_MINIMIZED);
    c.tray = b.check("Close the window to the tray (off: closing exits)", 8, 60, 440, 22, ID_TRAY);
    c.osd = b.check("Show a popup when the mode changes", 8, 86, 440, 22, ID_OSD);
    c.auto_mode = b.check("Use the mode last chosen on battery / plugged in", 8, 112, 440, 22, ID_AUTOMODE);
    b.label("Popup position:", 8, 150, 130, 18);
    c.osd_pos = b.combo(140, 146, 160, 200, ID_OSDPOS);
    for p in crate::win::toast::POSITIONS {
        b.add_item(c.osd_pos, p);
    }
    b.button("Modern interface (restart)", 8, 196, 220, 28, ID_MODERN);
    b.label(&format!("L-Helper {}", env!("CARGO_PKG_VERSION")), 8, 240, 300, 18);
}

fn select_page(index: usize) {
    let Some(u) = ui() else { return };
    unsafe {
        for (i, (_, page)) in u.pages.iter().enumerate() {
            let _ = ShowWindow(*page, if i == index { SW_SHOW } else { SW_HIDE });
        }
        SendMessageW(u.tab, TCM_SETCURSEL, Some(WPARAM(index)), Some(LPARAM(0)));
    }
}

// ---------------- reading the state into the controls ----------------

fn refresh() {
    let Some(u) = ui() else { return };
    let s = app::state().clone();
    let c = &u.c;
    let r = &s.sensors;
    let m = crate::lenovo::machine::get();

    // Performance
    for (i, mode) in u.modes.iter().enumerate() {
        set_check(c.modes[i], s.mode == Some(*mode));
    }
    set_text(c.mode_hint, s.mode.map(|md| md.hint()).unwrap_or(""));
    let mut cpu = Vec::new();
    if let Some(t) = r.cpu_temp {
        cpu.push(format!("{t} °C"));
    }
    if let Some(f) = r.cpu_fan {
        cpu.push(format!("fan {f} RPM"));
    }
    set_text(c.sensors[0], &or_dash(cpu.join("   ")));
    let mut gpu = Vec::new();
    if r.gpu_asleep {
        gpu.push("asleep".to_string());
    } else if let Some(t) = r.gpu_temp {
        gpu.push(format!("{t} °C"));
    }
    if let Some(f) = r.gpu_fan {
        gpu.push(format!("fan {f} RPM"));
    }
    set_text(c.sensors[1], &or_dash(gpu.join("   ")));
    set_text(c.sensors[2], &or_dash([r.cpu_fan, r.gpu_fan].iter().flatten().map(|f| format!("{f} RPM")).collect::<Vec<_>>().join(" / ")));
    set_text(c.sensors[3], &or_dash(r.power_text().unwrap_or_default()));
    set_check(c.overlay, crate::win::overlay::visible());

    // Graphics
    for (i, g) in u.gpu_modes.iter().enumerate() {
        set_check(c.gpus[i], s.gpu == Some(*g));
    }
    set_text(c.gpu_hint, s.gpu.map(|g| g.hint()).unwrap_or(""));

    // Display
    let auto = config::is("screen_auto");
    set_check(c.auto, auto);
    if let Some(screen_rates) = (!s.rates.is_empty()).then_some(&s.rates) {
        LISTS.with(|l| {
            let mut l = l.borrow_mut();
            if l.rates != *screen_rates {
                reset_combo(c.rate);
                for hz in screen_rates {
                    add_item(c.rate, &format!("{hz} Hz"));
                }
                l.rates = screen_rates.clone();
            }
            if l.resolutions != s.resolutions {
                reset_combo(c.res);
                for (i, (w, h)) in s.resolutions.iter().enumerate() {
                    add_item(c.res, &if i == 0 { format!("{w} × {h} (native)") } else { format!("{w} × {h}") });
                }
                l.resolutions = s.resolutions.clone();
            }
        });
        set_sel(c.rate, s.rate.and_then(|hz| screen_rates.iter().position(|r| *r == hz)));
        set_sel(c.res, s.resolution.and_then(|res| s.resolutions.iter().position(|r| *r == res)));
    }
    let has_screen = s.screen.is_some() && !s.rates.is_empty();
    for h in [c.auto, c.rate, c.res] {
        enable(h, has_screen);
    }
    show(c.od, s.overdrive.is_some());
    set_check(c.od, s.overdrive == Some(true));
    show(c.oled, s.oled);
    set_check(c.oled, s.oled && crate::win::oled::enabled());

    // Keyboard
    if !c.preset.0.is_null() {
        let all = crate::lighting::presets::all();
        LISTS.with(|l| {
            let mut l = l.borrow_mut();
            if l.presets != all.len() {
                reset_combo(c.preset);
                for p in &all {
                    add_item(c.preset, p.name);
                }
                l.presets = all.len();
            }
        });
        let current = crate::lighting::current();
        set_sel(c.preset, all.iter().position(|p| p.id == current.id));
        set_check(c.light, crate::lighting::is_running());
        set_trackbar(c.bright, crate::lighting::brightness());
        set_text(c.bright_val, &format!("{}%", crate::lighting::brightness()));
    } else if !c.bright.0.is_null() {
        if let Some(b) = s.spectrum_brightness {
            set_trackbar(c.bright, b);
            set_text(c.bright_val, &b.to_string());
        }
        show(c.logo, s.logo.is_some());
        set_check(c.logo, s.logo == Some(true));
    }
    for (i, h) in c.whites.iter().enumerate() {
        set_check(*h, s.white_level == Some(i as i32));
    }

    // Battery
    for (i, b) in u.battery_modes.iter().enumerate() {
        set_check(c.bats[i], s.battery == Some(b.0));
    }
    let mut info = Vec::new();
    if let Some(p) = r.battery_percent {
        info.push(format!("Charge: {p}%"));
    }
    match r.battery_rate.filter(|x| x.abs() > 0.1) {
        Some(rate) if rate < 0.0 => info.push(format!("{rate:.1} W")),
        Some(rate) => info.push(format!("+{rate:.1} W")),
        None => {}
    }
    set_text(c.bat_info, &info.join("   "));

    // Settings
    set_check(c.startup, s.startup);
    set_check(c.minimized, config::is_not_false("start_minimized"));
    set_check(c.tray, config::is_not_false("minimize_to_tray"));
    set_check(c.osd, config::is_not_false("osd"));
    set_check(c.auto_mode, config::is_not_false("auto_mode"));
    set_sel(c.osd_pos, Some((config::get("osd_position", 0).max(0) as usize).min(crate::win::toast::POSITIONS.len() - 1)));

    // Status bar: model and the last error
    set_status(u.status, &m.model, &s.status);
}

fn or_dash(s: String) -> String {
    if s.is_empty() { "-".into() } else { s }
}

// ---------------- reacting to the controls ----------------

fn on_command(id: i32, code: u32) {
    let Some(u) = ui() else { return };
    let c = &u.c;
    let clicked = code == BN_CLICKED;
    let selchange = code == CBN_SELCHANGE;
    match id {
        i if (ID_MODE..ID_MODE + 8).contains(&i) && clicked => {
            if let Some(m) = u.modes.get((i - ID_MODE) as usize) {
                for (k, h) in c.modes.iter().enumerate() {
                    set_check(*h, k == (i - ID_MODE) as usize);
                }
                app::set_mode(*m);
            }
        }
        i if (ID_GPU..ID_GPU + 8).contains(&i) && clicked => {
            if let Some(g) = u.gpu_modes.get((i - ID_GPU) as usize) {
                app::set_gpu(*g);
            }
        }
        i if (ID_BAT..ID_BAT + 4).contains(&i) && clicked => {
            if let Some(b) = u.battery_modes.get((i - ID_BAT) as usize) {
                for (k, h) in c.bats.iter().enumerate() {
                    set_check(*h, k == (i - ID_BAT) as usize);
                }
                app::set_battery(b.0);
            }
        }
        i if (ID_WHITE..ID_WHITE + 8).contains(&i) && clicked => {
            let level = i - ID_WHITE;
            for (k, h) in c.whites.iter().enumerate() {
                set_check(*h, k as i32 == level);
            }
            app::set_white_level(level);
        }
        ID_OVERLAY if clicked => crate::win::overlay::toggle(),
        ID_AUTO if clicked => {
            if is_checked(c.auto) {
                app::set_screen_auto();
            } else {
                config::set("screen_auto", 0);
            }
        }
        ID_RATE if selchange => {
            let rates = LISTS.with(|l| l.borrow().rates.clone());
            if let Some(hz) = get_sel(c.rate).and_then(|i| rates.get(i).copied()) {
                set_check(c.auto, false);
                app::set_refresh_rate(hz);
            }
        }
        ID_RES if selchange => {
            let list = LISTS.with(|l| l.borrow().resolutions.clone());
            if let Some((w, h)) = get_sel(c.res).and_then(|i| list.get(i).copied()) {
                app::set_resolution(w, h);
            }
        }
        ID_OD if clicked => {
            let on = is_checked(c.od);
            app::job(move || {
                let _ = crate::lenovo::gamezone::set_overdrive(on as i32);
                app::refresh_state();
            });
        }
        ID_OLED if clicked => app::set_oled_care(is_checked(c.oled)),
        ID_PRESET if selchange => {
            if let Some(p) = get_sel(c.preset).and_then(|i| crate::lighting::presets::all().get(i).cloned()) {
                app::job(move || {
                    if !crate::lighting::is_running() {
                        crate::lighting::start();
                    }
                    crate::lighting::apply(&p, None, true);
                });
            }
        }
        ID_LIGHT if clicked => {
            let on = is_checked(c.light);
            app::job(move || if on { crate::lighting::start() } else { crate::lighting::stop(true) });
        }
        ID_NEXT if clicked => app::cycle_lighting(),
        ID_LOGO if clicked => {
            let on = is_checked(c.logo);
            app::job(move || {
                crate::lenovo::keyboard::set_logo(on);
                app::refresh_state();
            });
        }
        ID_STARTUP if clicked => {
            let on = is_checked(c.startup);
            app::job(move || {
                if on {
                    crate::win::startup::schedule();
                } else {
                    crate::win::startup::unschedule();
                }
                app::refresh_state();
            });
        }
        ID_TRAY if clicked => config::set("minimize_to_tray", is_checked(c.tray) as i64),
        ID_MINIMIZED if clicked => config::set("start_minimized", is_checked(c.minimized) as i64),
        ID_OSD if clicked => config::set("osd", is_checked(c.osd) as i64),
        ID_AUTOMODE if clicked => config::set("auto_mode", is_checked(c.auto_mode) as i64),
        ID_OSDPOS if selchange => {
            if let Some(i) = get_sel(c.osd_pos) {
                config::set("osd_position", i as i64);
                crate::win::toast::show("Popup position", app::COLOR_BALANCED);
            }
        }
        ID_MODERN if clicked => {
            config::set_string("ui_style", "modern");
            restart();
        }
        _ => {}
    }
}

fn on_scroll(from: HWND, code: u32) {
    let Some(u) = ui() else { return };
    if from != u.c.bright {
        return;
    }
    let pos = get_trackbar(from);
    if crate::lighting::is_supported() {
        crate::lighting::set_brightness(pos);
        set_text(u.c.bright_val, &format!("{pos}%"));
    } else {
        set_text(u.c.bright_val, &pos.to_string());
        // The keyboard's own brightness: write once the thumb is released (TB_ENDTRACK) or a key / wheel step
        if code == 8 || code != 5 {
            app::job(move || {
                crate::lenovo::keyboard::set_spectrum_brightness(pos);
                app::refresh_state();
            });
        }
    }
}

/// Starts this exe again (it waits for this process to exit first) and quits
fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(["-after", &std::process::id().to_string()]).spawn();
        app::quit();
    }
}

// ---------------- window procedure ----------------

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let (id, code) = ((wparam.0 & 0xFFFF) as i32, ((wparam.0 >> 16) & 0xFFFF) as u32);
            on_command(id, code);
            LRESULT(0)
        }
        WM_HSCROLL => {
            on_scroll(HWND(lparam.0 as _), (wparam.0 & 0xFFFF) as u32);
            LRESULT(0)
        }
        WM_NOTIFY => {
            let header = &*(lparam.0 as *const NMHDR);
            if let Some(u) = ui() {
                if header.hwndFrom == u.tab && header.code == TCN_SELCHANGE {
                    let index = SendMessageW(u.tab, TCM_GETCURSEL, None, None).0 as usize;
                    select_page(index);
                    refresh();
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_REFRESH if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() => refresh(),
                TIMER_SHOTS => screenshot_step(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_REFRESH => {
            if IsWindowVisible(hwnd).as_bool() {
                refresh();
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            layout(hwnd);
            refresh();
            LRESULT(0)
        }
        WM_CLOSE => {
            if config::is_not_false("minimize_to_tray") {
                crate::win::show_window(hwnd, false);
            } else {
                app::quit();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// `-screenshot`: one PNG per page, then exit
unsafe fn screenshot_step(hwnd: HWND) {
    let Some(u) = ui() else { return };
    let Some((dir, index)) = SHOTS.with(|s| s.borrow().clone()) else { return };
    if index >= u.pages.len() {
        let _ = std::fs::copy(crate::log::log_file(), dir.join("log.txt"));
        std::process::exit(0);
    }
    let _ = std::fs::create_dir_all(&dir);
    select_page(index);
    refresh();
    let _ = RedrawWindow(Some(hwnd), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW);
    let name = format!("native-{}.png", u.pages[index].0.title().to_lowercase());
    // Let the page paint before PrintWindow reads it
    SetTimer(Some(hwnd), TIMER_SHOTS, 400, None);
    pump_paint(hwnd);
    if let Some((w, h, rgba)) = capture_screen(hwnd) {
        if let Some(img) = image::RgbaImage::from_raw(w, h, rgba) {
            let _ = img.save(dir.join(name));
        }
    }
    SHOTS.with(|s| *s.borrow_mut() = Some((dir, index + 1)));
}

/// Dispatches pending paint messages once
unsafe fn pump_paint(hwnd: HWND) {
    let mut msg = MSG::default();
    while PeekMessageW(&mut msg, Some(hwnd), WM_PAINT, WM_PAINT, PM_REMOVE).as_bool() {
        DispatchMessageW(&msg);
    }
}

/// The window as it looks on screen (BitBlt from the desktop), so it's what a user sees: PrintWindow can miss
/// themed children. Brings the window to the front first.
unsafe fn capture_screen(hwnd: HWND) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::*;
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    let mut r = RECT::default();
    GetWindowRect(hwnd, &mut r).ok()?;
    let (w, h) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
    let screen = GetDC(None);
    let mem = CreateCompatibleDC(Some(screen));
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w as i32, biHeight: -(h as i32), biPlanes: 1, biBitCount: 32, ..Default::default() };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    let old = SelectObject(mem, bitmap.into());
    // Let DWM present the repaint
    std::thread::sleep(std::time::Duration::from_millis(250));
    let ok = BitBlt(mem, 0, 0, w as i32, h as i32, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok();
    let mut rgba = Vec::new();
    if ok {
        let data = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
        for px in data.chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    SelectObject(mem, old);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(mem);
    ReleaseDC(None, screen);
    ok.then_some((w, h, rgba))
}
