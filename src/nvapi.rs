//! NVIDIA GPU tuning, ported from LLT's GPUOverclockController: core / memory clock offsets and an undervolt
//! (voltage cap: the V/F curve is flattened above the chosen voltage, so the GPU never boosts past it).
//!
//! Two driver paths, as in LLT:
//! - V/F curve (private NVAPI ClockBoost functions) when the driver exposes a graphics curve range. Layouts and
//!   call protocol read from LLT's NvAPIWrapper fork (LLT.NvAPIWrapper.Net 1.1.28, metadata and IL), which
//!   differs from upstream NvAPIWrapper on current GPUs (255 table entries, 254 curve points, packed indices).
//! - P-state 2.0 offsets (public NvAPI_GPU_Get/SetPstates20, NV_GPU_PERF_PSTATES20_INFO in nvapi.h, cross-checked
//!   with upstream NvAPIWrapper, LGPL-3.0) otherwise; no undervolt there.
//!
//! Every call wakes the dGPU, so nothing here runs unless the user reads the limits, applies,
//! or turned on "apply at startup".

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{s, w};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::config;

const ID_INITIALIZE: u32 = 0x0150E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC921F;
const ID_GET_PSTATES20: u32 = 0x6FF81213;
const ID_SET_PSTATES20: u32 = 0x0F4DAE6B;
const ID_ENABLE_OVERCLOCKED_PSTATES: u32 = 0xB23B70EE;
const ID_GET_CLOCK_BOOST_MASK: u32 = 0x507B4B59;
const ID_GET_CLOCK_BOOST_RANGES: u32 = 0x64B43A6A;
const ID_GET_CLOCK_BOOST_TABLE: u32 = 0x23F1B133;
const ID_SET_CLOCK_BOOST_TABLE: u32 = 0x0733E009;
const ID_GET_VF_POINTS: u32 = 0x21537AD4;
const ID_SET_CLOCK_BOOST_LOCK: u32 = 0x39442CFB;

const DOMAIN_GRAPHICS: u32 = 0;
const DOMAIN_MEMORY: u32 = 4;
const DOMAIN_VOLTAGE: u32 = 6;
const PSTATE_P0: u32 = 0;

// NV_GPU_PERF_PSTATES20_INFO: 20 byte header, 16 P-states of 456 bytes (id, flags, 8 clocks of 44 bytes, 4 base
// voltages of 24 bytes); V2/V3 add the over-volting block (4 + 4 * 24 bytes)
const HEADER: usize = 20;
const PSTATE_SIZE: usize = 456;
const CLOCK_SIZE: usize = 44;
const SIZE_V1: usize = HEADER + 16 * PSTATE_SIZE;
const SIZE_V2: usize = SIZE_V1 + 4 + 4 * 24;

// Private ClockBoost structures (LLT fork)
const MASKS_SIZE: usize = 6188; // version, masks[8] @4, unknown1[8] @36, ...
const RANGES_SIZE: usize = 2344; // version, domain mask, unknown[8], ranges[32] of 72 bytes @40
const RANGE_SIZE: usize = 72; // domain @4, max kHz @40, min kHz @44, first/last point index bytes @48/@49
const TABLE_SIZE: usize = 9248; // version, masks[8] @4, unknown1[8] @36, deltas[255] of 36 bytes @68
const DELTA_SIZE: usize = 36; // point mode @0, frequency delta kHz @20
const DELTAS: usize = 255;
const POINTS_SIZE: usize = 7208; // version, header[17] @4 (masks go in the first 16), points[254] of 28 bytes @72
const POINT_SIZE: usize = 28; // frequency kHz @0, voltage uV @4
const POINTS: usize = 254;
const LOCK_SIZE: usize = 780; // version 2, unknown, count @8, locks[32] of 24 bytes @12

type QueryInterface = unsafe extern "C" fn(u32) -> *const c_void;
type Initialize = unsafe extern "C" fn() -> i32;
type EnumPhysicalGpus = unsafe extern "C" fn(*mut [*mut c_void; 64], *mut u32) -> i32;
type Call = unsafe extern "C" fn(*mut c_void, *mut u8) -> i32;
type EnableOverclocked = unsafe extern "C" fn(*mut c_void) -> i32;

struct Api {
    get_pstates20: Call,
    set_pstates20: Call,
    enable_overclocked: Option<EnableOverclocked>,
    get_boost_mask: Option<Call>,
    get_boost_ranges: Option<Call>,
    get_boost_table: Option<Call>,
    set_boost_table: Option<Call>,
    get_vf_points: Option<Call>,
    set_boost_lock: Option<Call>,
    gpu: usize,
}

impl Api {
    fn call(&self, f: Call, buf: &mut [u8]) -> i32 {
        unsafe { f(self.gpu as *mut c_void, buf.as_mut_ptr()) }
    }
}

/// One clock's offset and the range the driver allows, in MHz
#[derive(Debug, Clone, Copy, Default)]
pub struct Offset {
    pub current: i32,
    pub min: i32,
    pub max: i32,
    pub editable: bool,
}

/// Voltage cap (undervolt) state, in mV; cap 0 = stock curve
#[derive(Debug, Clone, Copy, Default)]
pub struct Undervolt {
    pub cap: i32,
    pub min: i32,
    pub max: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Offsets {
    pub core: Offset,
    pub memory: Offset,
    /// Only with the V/F curve path
    pub undervolt: Option<Undervolt>,
}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let lib = LoadLibraryW(w!("nvapi64.dll")).map_err(|_| "NVIDIA driver (nvapi64.dll) not found".to_string())?;
        let query: QueryInterface = std::mem::transmute(GetProcAddress(lib, s!("nvapi_QueryInterface")).ok_or("nvapi_QueryInterface missing")?);
        let get = |id: u32| {
            let p = query(id);
            (!p.is_null()).then_some(p)
        };
        let call = |id: u32| get(id).map(|p| std::mem::transmute::<*const c_void, Call>(p));
        let initialize: Initialize = std::mem::transmute(get(ID_INITIALIZE).ok_or("NvAPI_Initialize missing")?);
        let status = initialize();
        if status != 0 {
            return Err(format!("NvAPI_Initialize failed ({status})"));
        }
        let enumerate: EnumPhysicalGpus = std::mem::transmute(get(ID_ENUM_PHYSICAL_GPUS).ok_or("NvAPI_EnumPhysicalGPUs missing")?);
        let mut handles = [std::ptr::null_mut(); 64];
        let mut count = 0u32;
        let status = enumerate(&mut handles, &mut count);
        if status != 0 || count == 0 {
            return Err(format!("No NVIDIA GPU found ({status})"));
        }
        Ok(Api {
            get_pstates20: call(ID_GET_PSTATES20).ok_or("NvAPI_GPU_GetPstates20 missing")?,
            set_pstates20: call(ID_SET_PSTATES20).ok_or("NvAPI_GPU_SetPstates20 missing")?,
            enable_overclocked: get(ID_ENABLE_OVERCLOCKED_PSTATES).map(|p| std::mem::transmute::<*const c_void, EnableOverclocked>(p)),
            get_boost_mask: call(ID_GET_CLOCK_BOOST_MASK),
            get_boost_ranges: call(ID_GET_CLOCK_BOOST_RANGES),
            get_boost_table: call(ID_GET_CLOCK_BOOST_TABLE),
            set_boost_table: call(ID_SET_CLOCK_BOOST_TABLE),
            get_vf_points: call(ID_GET_VF_POINTS),
            set_boost_lock: call(ID_SET_CLOCK_BOOST_LOCK),
            gpu: handles[0] as usize,
        })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    u32_at(b, at) as i32
}

fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Zeroed structure with NVAPI's version word: size | (version << 16)
fn structure(size: usize, version: u32) -> Vec<u8> {
    let mut b = vec![0u8; size];
    put(&mut b, 0, size as u32 | (version << 16));
    b
}

// ---------------- P-state 2.0 ----------------

/// Reads the P-state table, trying the newest structure version first
fn read_pstates(api: &Api) -> Result<Vec<u8>, String> {
    let mut last = 0;
    for (size, version) in [(SIZE_V2, 3u32), (SIZE_V2, 2), (SIZE_V1, 1)] {
        let mut buf = structure(size, version);
        let status = api.call(api.get_pstates20, &mut buf);
        if status == 0 {
            return Ok(buf);
        }
        last = status;
    }
    Err(format!("Can't read GPU clocks (NVAPI status {last}). The GPU may be off (Eco mode)."))
}

/// P0 core / memory offsets and the driver's allowed ranges
fn pstate_offsets(api: &Api) -> Result<(Offset, Offset), String> {
    let buf = read_pstates(api)?;
    let pstates = u32_at(&buf, 8).min(16) as usize;
    let clocks = u32_at(&buf, 12).min(8) as usize;
    let (mut core, mut memory) = (Offset::default(), Offset::default());
    for p in 0..pstates {
        let base = HEADER + p * PSTATE_SIZE;
        if u32_at(&buf, base) != PSTATE_P0 {
            continue;
        }
        for c in 0..clocks {
            let at = base + 8 + c * CLOCK_SIZE;
            let offset = Offset {
                editable: u32_at(&buf, at + 8) & 1 == 1,
                current: i32_at(&buf, at + 12) / 1000,
                min: i32_at(&buf, at + 16) / 1000,
                max: i32_at(&buf, at + 20) / 1000,
            };
            match u32_at(&buf, at) {
                DOMAIN_GRAPHICS => core = offset,
                DOMAIN_MEMORY => memory = offset,
                _ => {}
            }
        }
    }
    Ok((core, memory))
}

fn apply_pstates(api: &Api, core: Offset, memory: Offset, core_mhz: i32, memory_mhz: i32) -> Result<(), String> {
    let mut entries = Vec::new();
    for (domain, offset, wanted) in [(DOMAIN_GRAPHICS, core, core_mhz), (DOMAIN_MEMORY, memory, memory_mhz)] {
        if offset.editable && offset.min < offset.max {
            entries.push((domain, wanted.clamp(offset.min, offset.max)));
        }
    }
    if entries.is_empty() {
        return Err("This GPU doesn't allow clock offsets".into());
    }
    if let Some(enable) = api.enable_overclocked {
        let _ = unsafe { enable(api.gpu as *mut c_void) };
    }
    // V1 structure with one P-state (P0) holding only the clocks we change, as NvAPIWrapper / LLT build it
    let mut buf = structure(SIZE_V1, 1);
    put(&mut buf, 8, 1); // P-states
    put(&mut buf, 12, entries.len() as u32); // clocks
    put(&mut buf, HEADER, PSTATE_P0);
    for (i, (domain, mhz)) in entries.iter().enumerate() {
        let at = HEADER + 8 + i * CLOCK_SIZE;
        put(&mut buf, at, *domain);
        put(&mut buf, at + 12, (mhz * 1000) as u32);
    }
    match api.call(api.set_pstates20, &mut buf) {
        0 => Ok(()),
        status => Err(format!("The driver refused the offsets (NVAPI status {status})")),
    }
}

// ---------------- V/F curve ----------------

#[derive(Debug, Clone, Copy)]
struct Range {
    first: usize,
    last: usize,
    min_khz: i32,
    max_khz: i32,
}

struct Curve {
    /// 2 = adjusted clock curve (current GPUs), 1 = default
    version: u32,
    /// (frequency kHz, voltage uV)
    points: Vec<(u32, u32)>,
    /// Current ClockBoost table as read (masks included), modified in place to apply
    table: Vec<u8>,
    graphics: Range,
    memory: Option<Range>,
}

impl Curve {
    fn delta(&self, i: usize) -> i32 {
        i32_at(&self.table, 68 + i * DELTA_SIZE + 20)
    }
    fn mode(&self, i: usize) -> u32 {
        u32_at(&self.table, 68 + i * DELTA_SIZE)
    }
    fn set_delta(&mut self, i: usize, khz: i32) {
        put(&mut self.table, 68 + i * DELTA_SIZE + 20, khz as u32);
    }
    fn stock_khz(&self, i: usize) -> i32 {
        self.points[i].0 as i32 - self.delta(i)
    }
    /// First delta in the range the driver lets us set (point mode 0), kHz
    fn settable_delta(&self, r: Range) -> Option<i32> {
        (r.first..=r.last.min(DELTAS - 1)).find(|&i| self.mode(i) == 0).map(|i| self.delta(i))
    }
    /// Voltage range of the graphics curve, mV
    fn voltage_range(&self) -> Option<(i32, i32)> {
        let mv: Vec<i32> = (self.graphics.first..=self.graphics.last.min(self.points.len() - 1)).map(|i| self.points[i].1).filter(|v| *v > 0).map(|v| (v / 1000) as i32).collect();
        Some((*mv.iter().min()?, *mv.iter().max()?))
    }
    /// Voltage of the point where the curve was flattened, mV (0 = stock)
    fn current_cap(&self) -> i32 {
        let last = self.graphics.last.min(self.points.len() - 1);
        for i in self.graphics.first + 1..=last {
            if self.delta(i) < self.delta(i - 1) && self.points[i - 1].1 > 0 {
                return (self.points[i - 1].1 / 1000) as i32;
            }
        }
        0
    }
    /// LLT: clocks are doubled in the default curve structure unless the range says otherwise
    fn multiplier(&self, max_core_khz: i32) -> i32 {
        if self.version == 2 || self.graphics.max_khz != 2 * max_core_khz { 1 } else { 2 }
    }
}

fn ranges(api: &Api) -> Option<(Range, Option<Range>)> {
    let mut buf = structure(RANGES_SIZE, 1);
    if api.call(api.get_boost_ranges?, &mut buf) != 0 {
        return None;
    }
    let range = |domain: u32| {
        (0..32).map(|r| 40 + r * RANGE_SIZE).find(|&at| u32_at(&buf, at + 4) == domain).map(|at| Range {
            first: buf[at + 48] as usize,
            last: buf[at + 49] as usize,
            max_khz: i32_at(&buf, at + 40),
            min_khz: i32_at(&buf, at + 44),
        })
    };
    let graphics = range(DOMAIN_GRAPHICS).filter(|g| g.last != 0)?;
    Some((graphics, range(DOMAIN_MEMORY)))
}

/// Reads the V/F curve and the ClockBoost table; None when this GPU / driver has no graphics curve
fn curve(api: &Api) -> Option<Curve> {
    let (graphics, memory) = ranges(api)?;
    let mut masks = structure(MASKS_SIZE, 1);
    if api.call(api.get_boost_mask?, &mut masks) != 0 {
        return None;
    }
    // The fork copies the 8 masks and the 8 words after them into each request
    let mask_words = masks[4..68].to_vec();
    for version in [2u32, 1] {
        let mut points = structure(POINTS_SIZE, version);
        points[4..68].copy_from_slice(&mask_words);
        if api.call(api.get_vf_points?, &mut points) != 0 {
            continue;
        }
        let mut table = structure(TABLE_SIZE, version);
        table[4..68].copy_from_slice(&mask_words);
        if api.call(api.get_boost_table?, &mut table) != 0 {
            return None;
        }
        let points = (0..POINTS).map(|i| (u32_at(&points, 72 + i * POINT_SIZE), u32_at(&points, 72 + i * POINT_SIZE + 4))).collect::<Vec<_>>();
        if graphics.first >= points.len() || graphics.last >= points.len().min(DELTAS) {
            crate::log!("GPU OC: unexpected curve range {}..{}", graphics.first, graphics.last);
            return None;
        }
        return Some(Curve { version, points, table, graphics, memory });
    }
    None
}

/// LLT ApplyClockCurve: offsets on every settable point, and with a cap, points above it flattened to the cap's clock
fn apply_curve(api: &Api, mut c: Curve, core_khz: i32, memory_khz: i32, cap_mv: i32, max_core_khz: i32) -> Result<(), String> {
    let core_offset = core_khz * c.multiplier(max_core_khz);
    let anchor = (cap_mv > 0)
        .then(|| (c.graphics.first..=c.graphics.last).find(|&i| c.points[i].1 > 0 && (c.points[i].1 / 1000) as i32 >= cap_mv))
        .flatten();
    if cap_mv > 0 && anchor.is_none() {
        return Err(format!("No curve point at or above {cap_mv} mV"));
    }
    let anchor_stock = anchor.map(|a| c.stock_khz(a)).unwrap_or(0);
    let target = anchor_stock + core_offset;

    let mut flattened = 0;
    for i in 0..DELTAS.min(c.points.len()) {
        let delta = if c.memory.is_some_and(|m| i >= m.first && i <= m.last) {
            memory_khz
        } else if i >= c.graphics.first && i <= c.graphics.last {
            if anchor.is_some() && c.stock_khz(i) > anchor_stock {
                flattened += 1;
                (target - c.stock_khz(i)).max(c.graphics.min_khz)
            } else {
                core_offset
            }
        } else {
            continue;
        };
        // Like the fork's SetClockBoostTable: points the driver owns (non-zero point mode) keep their delta
        if c.mode(i) == 0 {
            c.set_delta(i, delta);
        }
    }
    let set = api.set_boost_table.ok_or("NvAPI_GPU_SetClockBoostTable missing")?;
    let status = api.call(set, &mut c.table);
    if status != 0 {
        return Err(format!("The driver refused the curve (NVAPI status {status})"));
    }
    if let Some(a) = anchor {
        crate::log!("GPU OC: curve flattened at point #{a} ({} mV, {} MHz), {flattened} points", c.points[a].1 / 1000, target / 1000);
    }
    // LLT resets any voltage lock on each apply
    if let Some(lock) = api.set_boost_lock {
        let mut buf = structure(LOCK_SIZE, 2);
        put(&mut buf, 8, 1);
        put(&mut buf, 12, DOMAIN_VOLTAGE);
        let _ = api.call(lock, &mut buf);
    }
    Ok(())
}

// ---------------- public ----------------

/// Current offsets, undervolt and allowed ranges
pub fn read() -> Result<Offsets, String> {
    let api = api()?;
    let (mut core, mut memory) = pstate_offsets(api)?;
    let mut undervolt = None;
    if let Some(c) = curve(api) {
        crate::log!(
            "GPU OC curve: struct v{}, graphics points {}..{} ({}..{} kHz), memory {:?}, voltages {:?} mV, cap {} mV",
            c.version,
            c.graphics.first,
            c.graphics.last,
            c.graphics.min_khz,
            c.graphics.max_khz,
            c.memory.map(|m| (m.first, m.last)),
            c.voltage_range(),
            c.current_cap()
        );
        // With a curve, the active offsets live in the ClockBoost table rather than the P-states
        let multiplier = c.multiplier(core.max * 1000);
        if let Some(d) = c.settable_delta(c.graphics) {
            core.current = d / 1000 / multiplier;
        }
        if let Some(d) = c.memory.and_then(|m| c.settable_delta(m)) {
            memory.current = d / 1000;
        }
        undervolt = c.voltage_range().map(|(min, max)| Undervolt { cap: c.current_cap(), min, max });
    }
    Ok(Offsets { core, memory, undervolt })
}

/// Applies core / memory offsets (MHz) and a voltage cap (mV, 0 = none), clamped to the driver's ranges
pub fn apply(core_mhz: i32, memory_mhz: i32, cap_mv: i32) -> Result<Offsets, String> {
    let api = api()?;
    let (core, memory) = pstate_offsets(api)?;
    let clamp = |o: Offset, v: i32| if o.min < o.max { v.clamp(o.min, o.max) } else { 0 };
    let (core_mhz, memory_mhz) = (clamp(core, core_mhz), clamp(memory, memory_mhz));
    match curve(api) {
        Some(c) => {
            let cap = match (cap_mv, c.voltage_range()) {
                (0, _) | (_, None) => 0,
                (v, Some((min, max))) => v.clamp(min, max),
            };
            apply_curve(api, c, core_mhz * 1000, memory_mhz * 1000, cap, core.max * 1000)?;
        }
        None => {
            if cap_mv > 0 {
                return Err("This GPU / driver doesn't expose its voltage curve, so it can't be undervolted".into());
            }
            apply_pstates(api, core, memory, core_mhz, memory_mhz)?;
        }
    }
    let now = read()?;
    crate::log!(
        "GPU OC applied: core {} MHz, memory {} MHz, cap {} mV (requested {core_mhz} / {memory_mhz} / {cap_mv})",
        now.core.current,
        now.memory.current,
        now.undervolt.map(|u| u.cap).unwrap_or(0)
    );
    Ok(now)
}

/// Lenovo firmware's own GPU overclock switch (LLT checks the same before offering it)
pub fn firmware_allows() -> bool {
    crate::wmi::try_call_int("LENOVO_GAMEZONE_DATA", "IsSupportGpuOC", "Data", &[], false).is_some_and(|v| v > 0)
}

pub fn saved() -> (i32, i32, i32) {
    (config::get("gpu_oc_core", 0) as i32, config::get("gpu_oc_memory", 0) as i32, config::get("gpu_oc_cap", 0) as i32)
}

/// Re-applies saved settings (startup, resume); the driver forgets them on restart
pub fn apply_saved() {
    if !config::is("gpu_oc") {
        return;
    }
    let (core, memory, cap) = saved();
    if core == 0 && memory == 0 && cap == 0 {
        return;
    }
    if let Err(e) = apply(core, memory, cap) {
        crate::log!("GPU OC not applied: {e}");
    }
}
