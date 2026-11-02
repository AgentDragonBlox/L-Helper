//! LENOVO_GAMEZONE_DATA (power mode, GPU switching, key locks, overdrive) and LENOVO_OTHER_METHOD capabilities

use crate::wmi::{self, Arg};

const GZ: &str = "LENOVO_GAMEZONE_DATA";
const OTHER: &str = "LENOVO_OTHER_METHOD";

fn data(v: i32) -> [(&'static str, Arg); 1] {
    [("Data", Arg::I32(v))]
}

fn get(method: &str) -> Result<i32, String> {
    wmi::call_int(GZ, method, "Data", &[])
}

fn supported(method: &str) -> bool {
    wmi::try_call_int(GZ, method, "Data", &[], false).unwrap_or(0) > 0
}

fn set(method: &str, value: i32) -> Result<(), String> {
    wmi::call(GZ, method, &data(value)).map(|_| ())
}

pub fn exists() -> bool {
    wmi::exists(GZ)
}

// Power mode: 1 Quiet, 2 Balanced, 3 Performance, 224 Extreme, 255 Custom
pub fn smart_fan_version() -> Option<i32> {
    wmi::try_call_int(GZ, "IsSupportSmartFan", "Data", &[], true)
}
pub fn smart_fan_mode() -> Result<i32, String> {
    get("GetSmartFanMode")
}
pub fn set_smart_fan_mode(mode: i32) -> Result<(), String> {
    set("SetSmartFanMode", mode)
}

// GPU: G-Sync (dGPU only via MUX) and iGPU mode (0 hybrid, 1 iGPU only, 2 auto)
pub fn supports_gsync() -> bool {
    supported("IsSupportGSync")
}
pub fn gsync() -> Result<i32, String> {
    get("GetGSyncStatus")
}
pub fn set_gsync(v: i32) -> Result<(), String> {
    set("SetGSyncStatus", v)
}
pub fn supports_igpu_mode() -> bool {
    supported("IsSupportIGPUMode")
}
pub fn igpu_mode() -> Result<i32, String> {
    get("GetIGPUModeStatus")
}
pub fn set_igpu_mode(mode: i32) -> Result<(), String> {
    wmi::call(GZ, "SetIGPUModeStatus", &[("mode", Arg::I32(mode))]).map(|_| ())
}
pub fn notify_dgpu(status: i32) -> Result<(), String> {
    wmi::call(GZ, "NotifyDGPUStatus", &[("Status", Arg::I32(status))]).map(|_| ())
}

// Overdrive (panel response time), as LLT's OverDriveFeature: the OverDrive capability (LENOVO_OTHER_METHOD) on
// newer models, else GameZone's IsSupportOD / Get/SetODStatus
static OD_VIA_CAPABILITY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `exclude_capability`: LLT skips the capability on Legion 7 / Pro 7 gen 10+
pub fn supports_overdrive(exclude_capability: bool) -> bool {
    if !exclude_capability && capability_listed(Cap::OverDrive) {
        OD_VIA_CAPABILITY.store(true, std::sync::atomic::Ordering::Relaxed);
        return true;
    }
    supported("IsSupportOD")
}
pub fn overdrive() -> Result<i32, String> {
    if OD_VIA_CAPABILITY.load(std::sync::atomic::Ordering::Relaxed) {
        return feature(Cap::OverDrive);
    }
    get("GetODStatus")
}
pub fn set_overdrive(v: i32) -> Result<(), String> {
    if OD_VIA_CAPABILITY.load(std::sync::atomic::Ordering::Relaxed) {
        return set_feature(Cap::OverDrive, v);
    }
    set("SetODStatus", v)
}

/// The firmware lists a capability as supported (LENOVO_CAPABILITY_DATA_00, non-zero), like LLT's Features[id]
pub fn capability_listed(id: Cap) -> bool {
    wmi::query("SELECT IDs, Capability FROM LENOVO_CAPABILITY_DATA_00")
        .map(|rows| rows.iter().any(|r| r.get("IDs").as_i64() == Some(id as u32 as i64) && r.get("Capability").as_i64().unwrap_or(0) != 0))
        .unwrap_or(false)
}
pub fn supports_winkey_lock() -> bool {
    supported("IsSupportDisableWinKey")
}
pub fn winkey_lock() -> Result<i32, String> {
    get("GetWinKeyStatus")
}
pub fn set_winkey_lock(v: i32) -> Result<(), String> {
    set("SetWinKeyStatus", v)
}
pub fn supports_touchpad_lock() -> bool {
    supported("IsSupportDisableTP")
}
pub fn touchpad_lock() -> Result<i32, String> {
    get("GetTPStatus")
}
pub fn set_touchpad_lock(v: i32) -> Result<(), String> {
    set("SetTPStatus", v)
}

/// 0 = firmware owns keyboard lighting, 1 = software (used by the 24-zone engine)
pub fn set_light_control_owner(software: bool) -> Result<(), String> {
    set("SetLightControlOwner", software as i32)
}

// ---------------- LENOVO_OTHER_METHOD capabilities ----------------

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Cap {
    SupportedPowerModes = 0x00070000,
    OverDrive = 0x001A0000,
    LegionZoneSupportVersion = 0x00090000,
    CpuShortTerm = 0x0101FF00,
    CpuLongTerm = 0x0102FF00,
    CpuPeak = 0x0103FF00,
    CpuTempLimit = 0x0104FF00,
    ApuSppt = 0x0105FF00,
    CpuCrossLoading = 0x0106FF00,
    CpuPl2Tau = 0x0107FF00,
    GpuPowerBoost = 0x0201FF00,
    GpuTgp = 0x0202FF00,
    GpuTempLimit = 0x0203FF00,
    GpuTppOffset = 0x0204FF00,
    GpuToCpuBoost = 0x020BFF00,
    FanFullSpeed = 0x04020000,
    CpuFanSpeed = 0x04030001,
    GpuFanSpeed = 0x04030002,
    CpuTemp = 0x05040000,
    GpuTemp = 0x05050000,
}

/// The mode byte (bits 8-15) of power limit ids is masked out when calling, same as LLT
const ID_MASK: u32 = 0xFFFF00FF;

pub fn feature(id: Cap) -> Result<i32, String> {
    wmi::call_int(OTHER, "GetFeatureValue", "Value", &[("IDs", Arg::U32(id as u32 & ID_MASK))])
}

pub fn try_feature(id: Cap) -> Option<i32> {
    feature(id).ok()
}

pub fn set_feature(id: Cap, value: i32) -> Result<(), String> {
    wmi::call(OTHER, "SetFeatureValue", &[("IDs", Arg::U32(id as u32 & ID_MASK)), ("value", Arg::I32(value))]).map(|_| ())
}

pub fn support_thermal_mode() -> Option<i32> {
    wmi::try_call_int(OTHER, "GetSupportThermalMode", "mode", &[], false)
}

pub fn legion_zone_version() -> Option<i32> {
    try_feature(Cap::LegionZoneSupportVersion).or_else(|| wmi::try_call_int(OTHER, "Get_Support_LegionZone_Version", "Version", &[], false))
}

#[derive(Debug, Clone, Copy)]
pub struct Range {
    pub id: u32,
    pub default: i32,
    pub min: i32,
    pub max: i32,
    pub step: i32,
}

/// LENOVO_CAPABILITY_DATA_01 rows keyed by raw id (mode byte included)
pub fn read_ranges() -> std::collections::HashMap<u32, Range> {
    let mut out = std::collections::HashMap::new();
    match wmi::query("SELECT * FROM LENOVO_CAPABILITY_DATA_01") {
        Ok(rows) => {
            for r in rows {
                let id = r.get("IDs").as_i64().unwrap_or(0) as u32;
                out.insert(id, Range {
                    id,
                    default: r.i32("DefaultValue").unwrap_or(0),
                    min: r.i32("MinValue").unwrap_or(0),
                    max: r.i32("MaxValue").unwrap_or(0),
                    step: r.i32("Step").unwrap_or(0),
                });
            }
        }
        Err(e) => crate::log!("Can't read capability data 01: {e}"),
    }
    out
}

/// Range of a capability in a given power mode (0xFF = Custom)
pub fn find_range(ranges: &std::collections::HashMap<u32, Range>, id: Cap, mode: u32) -> Option<Range> {
    ranges.get(&((id as u32 & ID_MASK) | (mode << 8))).copied()
}

/// LENOVO_DISCRETE_DATA: allowed values for capabilities without a step
pub fn read_discrete() -> std::collections::HashMap<u32, Vec<i32>> {
    let mut out: std::collections::HashMap<u32, Vec<i32>> = std::collections::HashMap::new();
    if let Ok(rows) = wmi::query("SELECT * FROM LENOVO_DISCRETE_DATA") {
        for r in rows {
            let id = r.get("IDs").as_i64().unwrap_or(0) as u32;
            if let Some(v) = r.i32("Value") {
                out.entry(id).or_default().push(v);
            }
        }
    }
    for v in out.values_mut() {
        v.sort();
    }
    out
}
