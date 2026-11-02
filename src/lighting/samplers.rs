// Inputs for reactive presets, adapted from LOQ-24-zone-RGB-controller
// (audio_sampler.rs, presets/ambient.rs, input_handler.rs, presets/thermalStatus.rs)
// Copyright (c) 2026 Dhruv Goel, MIT License. See THIRD_PARTY_NOTICES.md

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetCurrentThreadId, GetSystemTimes};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::zone::{Color, NUM_ZONES};

pub const AMBIENT_HEIGHT: usize = 12;

#[derive(Debug, Clone, Copy, Default)]
pub struct RgbF {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl RgbF {
    pub const BLACK: RgbF = RgbF { r: 0.0, g: 0.0, b: 0.0 };
    pub fn luminance(&self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
    pub fn add(self, o: RgbF) -> RgbF {
        RgbF { r: self.r + o.r, g: self.g + o.g, b: self.b + o.b }
    }
    pub fn scale(self, s: f32) -> RgbF {
        RgbF { r: self.r * s, g: self.g * s, b: self.b * s }
    }
    pub fn lerp(self, t: RgbF, a: f32) -> RgbF {
        RgbF { r: self.r + (t.r - self.r) * a, g: self.g + (t.g - self.g) * a, b: self.b + (t.b - self.b) * a }
    }
    pub fn saturate(self, amount: f32) -> RgbF {
        let l = self.luminance();
        let f = |c: f32| (l + (c - l) * amount).clamp(0.0, 1.0);
        RgbF { r: f(self.r), g: f(self.g), b: f(self.b) }
    }
    pub fn powf(self, p: f32) -> RgbF {
        RgbF { r: self.r.powf(p), g: self.g.powf(p), b: self.b.powf(p) }
    }
    pub fn to_color(self) -> Color {
        let f = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
        Color::new(f(self.r), f(self.g), f(self.b))
    }
}

// ---------------- audio loopback (RMS, smoothed 0.8 / 0.2) ----------------

static INTENSITY: AtomicU32 = AtomicU32::new(0);
static AUDIO_USERS: AtomicUsize = AtomicUsize::new(0);
static AUDIO_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn audio_intensity() -> f32 {
    f32::from_bits(INTENSITY.load(Ordering::Relaxed))
}

/// RAII user of the loopback capture; capture runs while at least one user exists
pub struct AudioUser;

impl AudioUser {
    pub fn new() -> Self {
        if AUDIO_USERS.fetch_add(1, Ordering::SeqCst) == 0 {
            AUDIO_RUNNING.store(true, Ordering::SeqCst);
            std::thread::Builder::new()
                .name("audio-loopback".into())
                .spawn(|| {
                    if let Err(e) = unsafe { capture_loop() } {
                        crate::log!("Audio loopback failed: {e}");
                    }
                    INTENSITY.store(0, Ordering::Relaxed);
                })
                .ok();
        }
        AudioUser
    }
}

impl Drop for AudioUser {
    fn drop(&mut self) {
        if AUDIO_USERS.fetch_sub(1, Ordering::SeqCst) == 1 {
            AUDIO_RUNNING.store(false, Ordering::SeqCst);
        }
    }
}

unsafe fn capture_loop() -> windows::core::Result<()> {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let format = client.GetMixFormat()?;
    let wfx = &*format;
    let channels = wfx.nChannels as usize;
    let bits = wfx.wBitsPerSample;
    let is_float = if wfx.wFormatTag == 0xFFFE {
        let ext = &*(format as *const WAVEFORMATEXTENSIBLE);
        std::ptr::addr_of!(ext.SubFormat).read_unaligned() == windows::core::GUID::from_u128(0x00000003_0000_0010_8000_00AA00389B71)
    } else {
        wfx.wFormatTag == 3
    };
    client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, format, None)?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;

    while AUDIO_RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        while let Ok(size) = capture.GetNextPacketSize() {
            if size == 0 {
                break;
            }
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            if capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                break;
            }
            let total = frames as usize * channels;
            if flags & 0x2 == 0 && total > 0 {
                let rms = if is_float && bits == 32 {
                    let s = std::slice::from_raw_parts(data as *const f32, total);
                    (s.iter().map(|v| v * v).sum::<f32>() / total as f32).sqrt()
                } else if !is_float && bits == 16 {
                    let s = std::slice::from_raw_parts(data as *const i16, total);
                    (s.iter().map(|v| (*v as f32 / 32767.0).powi(2)).sum::<f32>() / total as f32).sqrt()
                } else {
                    0.0
                };
                let smoothed = audio_intensity() * 0.8 + rms * 0.2;
                INTENSITY.store(smoothed.to_bits(), Ordering::Relaxed);
            }
            let _ = capture.ReleaseBuffer(frames);
        }
    }
    let _ = client.Stop();
    CoTaskMemFree(Some(format as _));
    Ok(())
}

// ---------------- screen sampling (GDI StretchBlt into 24 x 12, HALFTONE averages each cell) ----------------

pub struct ScreenSampler {
    left: f32,
    width: f32,
    last: [[RgbF; AMBIENT_HEIGHT]; NUM_ZONES],
}

impl ScreenSampler {
    pub fn new(left: f32, width: f32) -> Self {
        ScreenSampler { left, width, last: [[RgbF::BLACK; AMBIENT_HEIGHT]; NUM_ZONES] }
    }

    pub fn sample(&mut self) -> [[RgbF; AMBIENT_HEIGHT]; NUM_ZONES] {
        unsafe {
            let sw = GetSystemMetrics(SM_CXSCREEN);
            let sh = GetSystemMetrics(SM_CYSCREEN);
            let sx = (sw as f32 * self.left.clamp(0.0, 1.0)) as i32;
            let swidth = ((sw as f32 * self.width.clamp(0.01, 1.0)) as i32).max(1);

            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: NUM_ZONES as i32,
                biHeight: -(AMBIENT_HEIGHT as i32),
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            if let Ok(bitmap) = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                let old = SelectObject(mem, bitmap.into());
                SetStretchBltMode(mem, HALFTONE);
                if StretchBlt(mem, 0, 0, NUM_ZONES as i32, AMBIENT_HEIGHT as i32, Some(screen), sx, 0, swidth, sh, SRCCOPY).as_bool() {
                    let data = std::slice::from_raw_parts(bits as *const u8, NUM_ZONES * AMBIENT_HEIGHT * 4);
                    for y in 0..AMBIENT_HEIGHT {
                        for x in 0..NUM_ZONES {
                            let p = (y * NUM_ZONES + x) * 4;
                            self.last[x][y] = RgbF { r: data[p + 2] as f32 / 255.0, g: data[p + 1] as f32 / 255.0, b: data[p] as f32 / 255.0 };
                        }
                    }
                }
                SelectObject(mem, old);
                let _ = DeleteObject(bitmap.into());
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
        self.last
    }
}

// ---------------- global keyboard hook → zone (Typing Ripple) ----------------

static KEY_ZONES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static KEY_USERS: AtomicUsize = AtomicUsize::new(0);
static KEY_THREAD: AtomicU32 = AtomicU32::new(0);

pub struct KeyHookUser;

impl KeyHookUser {
    pub fn new() -> Self {
        if KEY_USERS.fetch_add(1, Ordering::SeqCst) == 0 {
            std::thread::Builder::new()
                .name("key-hook".into())
                .spawn(|| unsafe {
                    KEY_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
                    let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0);
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
                    if let Ok(h) = hook {
                        let _ = UnhookWindowsHookEx(h);
                    }
                })
                .ok();
        }
        KeyHookUser
    }

    pub fn drain(&self) -> Vec<usize> {
        std::mem::take(&mut *KEY_ZONES.lock().unwrap())
    }
}

impl Drop for KeyHookUser {
    fn drop(&mut self) {
        if KEY_USERS.fetch_sub(1, Ordering::SeqCst) == 1 {
            let id = KEY_THREAD.swap(0, Ordering::SeqCst);
            if id != 0 {
                unsafe {
                    let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
            KEY_ZONES.lock().unwrap().clear();
        }
    }
}

/// Every key-down starts a ripple, auto-repeat included (as in the original controller): holding a key keeps
/// sending ripples, so the keyboard stays lit around it until it's released
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && (wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize) {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let mut zones = KEY_ZONES.lock().unwrap();
        zones.push(map_key_to_zone(VIRTUAL_KEY(info.vkCode as u16)));
        if zones.len() > 10 {
            zones.remove(0);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Same layout mapping as input_handler.rs
fn map_key_to_zone(k: VIRTUAL_KEY) -> usize {
    let ch = |c: char| VIRTUAL_KEY(c as u16);
    let is = |list: &[VIRTUAL_KEY]| list.contains(&k);
    if is(&[VK_ESCAPE, VK_OEM_3, VK_TAB, VK_CAPITAL, VK_LSHIFT, VK_LCONTROL]) { 0 }
    else if is(&[VK_F1, ch('1')]) { 1 }
    else if is(&[VK_F2, ch('2'), ch('Q'), ch('A')]) { 2 }
    else if is(&[ch('W'), ch('Z'), VK_LWIN]) { 3 }
    else if is(&[VK_F3, ch('3'), ch('S'), ch('X'), VK_LMENU]) { 4 }
    else if is(&[VK_F4, ch('4'), ch('E'), ch('D')]) { 5 }
    else if is(&[VK_F5, ch('5'), ch('R'), ch('F'), ch('C')]) { 6 }
    else if is(&[VK_F6, ch('T'), ch('V')]) { 7 }
    else if is(&[VK_F7, ch('6'), ch('G'), ch('B'), VK_SPACE]) { 8 }
    else if is(&[ch('7'), ch('Y'), ch('H')]) { 9 }
    else if is(&[VK_F8, ch('U'), ch('N')]) { 10 }
    else if is(&[VK_F9, ch('8'), ch('J'), ch('M')]) { 11 }
    else if is(&[VK_F10, ch('9'), ch('I'), ch('K'), VK_OEM_COMMA, VK_RMENU]) { 12 }
    else if is(&[VK_F11, ch('0'), ch('O'), ch('L')]) { 13 }
    else if is(&[VK_F12, ch('P'), VK_OEM_PERIOD, VK_RWIN]) { 14 }
    else if is(&[VK_OEM_MINUS, VK_OEM_1, VK_OEM_2, VK_LEFT]) { 15 }
    else if is(&[VK_INSERT, VK_OEM_PLUS, VK_OEM_4, VK_OEM_7, VK_UP, VK_DOWN]) { 16 }
    else if is(&[VK_SNAPSHOT, VK_BACK, VK_OEM_6, VK_RETURN, VK_RSHIFT]) { 17 }
    else if is(&[VK_DELETE, VK_OEM_5, VK_RIGHT]) { 18 }
    else if is(&[VK_HOME, VK_NUMLOCK, VK_NUMPAD7, VK_NUMPAD4, VK_NUMPAD1, VK_NUMPAD0]) { 20 }
    else if is(&[VK_END, VK_DIVIDE, VK_NUMPAD8, VK_NUMPAD5, VK_NUMPAD2]) { 21 }
    else if is(&[VK_PRIOR, VK_MULTIPLY, VK_NUMPAD9, VK_NUMPAD6, VK_NUMPAD3, VK_DECIMAL]) { 22 }
    else if is(&[VK_NEXT, VK_SUBTRACT, VK_ADD]) { 23 }
    else { 12 }
}

// ---------------- CPU / memory / NVIDIA GPU utilisation ----------------

pub struct SystemMonitor {
    last_idle: u64,
    last_total: u64,
    nvml: Option<nvml_wrapper::Nvml>,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl SystemMonitor {
    pub fn new() -> Self {
        let nvml = nvml_wrapper::Nvml::init().map_err(|e| crate::log!("NVML not available: {e}")).ok();
        SystemMonitor { last_idle: 0, last_total: 0, nvml }
    }

    pub fn read(&mut self) -> (f32, f32, f32) {
        let mut cpu = 0.0;
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_ok() {
            let total = ft(kernel) + ft(user); // kernel time includes idle
            let (d_total, d_idle) = (total.saturating_sub(self.last_total), ft(idle).saturating_sub(self.last_idle));
            if self.last_total != 0 && d_total > 0 {
                cpu = (1.0 - d_idle as f32 / d_total as f32).clamp(0.0, 1.0);
            }
            self.last_total = total;
            self.last_idle = ft(idle);
        }
        let mut mem = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        let memory = if unsafe { GlobalMemoryStatusEx(&mut mem) }.is_ok() { mem.dwMemoryLoad as f32 / 100.0 } else { 0.0 };
        let gpu = self
            .nvml
            .as_ref()
            .and_then(|n| n.device_by_index(0).ok())
            .and_then(|d| d.utilization_rates().ok())
            .map(|u| u.gpu as f32 / 100.0)
            .unwrap_or(0.0);
        (cpu, memory, gpu)
    }
}
