//! Temperatures and fan speeds from LENOVO_OTHER_METHOD, battery rate from the battery class driver
//! (IOCTL_BATTERY_QUERY_STATUS, as LLT does) with root\WMI BatteryStatus as fallback

use std::sync::Mutex;

use super::gamezone::{self, Cap};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::Properties::DEVPROPTYPE;
use windows::Win32::Foundation::{CloseHandle, DEVPROPKEY, HANDLE};
use windows::Win32::System::Performance::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::IO::DeviceIoControl;

#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub cpu_temp: Option<i32>,
    pub gpu_temp: Option<i32>,
    pub cpu_fan: Option<i32>,
    pub gpu_fan: Option<i32>,
    /// Watts, + charging / - discharging
    pub battery_rate: Option<f64>,
    pub battery_percent: Option<i32>,
    /// CPU package power in W (Windows' RAPL energy meter)
    pub cpu_power: Option<f64>,
    /// NVIDIA GPU board power in W, only read while the GPU is awake (reading it would wake it)
    pub gpu_power: Option<f64>,
    /// NVIDIA GPU present and asleep (D3): it draws next to nothing
    pub gpu_asleep: bool,
}

impl Reading {
    /// CPU + GPU in W; an asleep GPU counts as 0. None unless the CPU meter works.
    pub fn total_power(&self) -> Option<f64> {
        let gpu = match (self.gpu_power, self.gpu_asleep) {
            (Some(w), _) => w,
            (None, true) => 0.0,
            (None, false) => return None,
        };
        Some(self.cpu_power? + gpu)
    }

    /// "CPU 9.2 W · GPU 31.0 W · Total 40.2 W" (GPU "asleep" when it's off). None when nothing is measurable.
    pub fn power_text(&self) -> Option<String> {
        let gpu = match (self.gpu_power, self.gpu_asleep) {
            (Some(w), _) => Some(format!("GPU {w:.1} W")),
            (None, true) => Some("GPU asleep".to_string()),
            _ => None,
        };
        let total = self.total_power().map(|w| format!("Total {w:.1} W"));
        let parts: Vec<String> = [self.cpu_power.map(|w| format!("CPU {w:.1} W")), gpu, total].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join("  ·  "))
    }
}

fn temp(v: Option<i32>) -> Option<i32> {
    v.filter(|t| (1..150).contains(t))
}

pub fn read() -> Reading {
    let mut r = Reading {
        cpu_temp: temp(gamezone::try_feature(Cap::CpuTemp)),
        gpu_temp: temp(gamezone::try_feature(Cap::GpuTemp)),
        cpu_fan: gamezone::try_feature(Cap::CpuFanSpeed),
        gpu_fan: gamezone::try_feature(Cap::GpuFanSpeed),
        ..Default::default()
    };

    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.BatteryLifePercent <= 100 {
        r.battery_percent = Some(status.BatteryLifePercent as i32);
    }

    r.battery_rate = battery_rate().or_else(wmi_battery_rate);
    r.cpu_power = cpu_power();
    match nvidia_awake() {
        Some(true) => r.gpu_power = gpu_power(),
        Some(false) => r.gpu_asleep = true,
        None => {}
    }
    r
}

// ---------------- CPU package power: "\Energy Meter(RAPL_Package0_PKG)\Power" (mW) ----------------

struct Pdh {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}
unsafe impl Send for Pdh {}

static PDH: Mutex<Option<Option<Pdh>>> = Mutex::new(None);

fn cpu_power() -> Option<f64> {
    let mut slot = PDH.lock().unwrap();
    let pdh = slot.get_or_insert_with(|| unsafe {
        let mut query = PDH_HQUERY::default();
        let mut counter = PDH_HCOUNTER::default();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
            return None;
        }
        if PdhAddEnglishCounterW(query, windows::core::w!("\\Energy Meter(RAPL_Package0_PKG)\\Power"), 0, &mut counter) != 0 {
            crate::log!("No CPU package power counter (Energy Meter)");
            let _ = PdhCloseQuery(query);
            return None;
        }
        // A rate counter needs two samples: take the first now so the reading below has an interval
        let _ = PdhCollectQueryData(query);
        std::thread::sleep(std::time::Duration::from_millis(250));
        Some(Pdh { query, counter })
    });
    let pdh = pdh.as_ref()?;
    unsafe {
        if PdhCollectQueryData(pdh.query) != 0 {
            return None;
        }
        let mut value = PDH_FMT_COUNTERVALUE::default();
        if PdhGetFormattedCounterValue(pdh.counter, PDH_FMT_DOUBLE, None, &mut value) != 0 {
            return None;
        }
        let mw = value.Anonymous.doubleValue;
        (mw > 0.0).then(|| mw / 1000.0)
    }
}

// ---------------- NVIDIA GPU power ----------------

const GUID_DEVCLASS_DISPLAY: GUID = GUID::from_u128(0x4d36e968_e325_11ce_bfc1_08002be10318);
const DEVPKEY_DEVICE_POWER_DATA: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 32 };

/// Power state of the NVIDIA display adapter without touching the GPU: Some(true) in D0, Some(false) asleep,
/// None when there's no NVIDIA adapter (or Eco mode removed it)
fn nvidia_awake() -> Option<bool> {
    unsafe {
        let set = SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), PCWSTR::null(), None, DIGCF_PRESENT).ok()?;
        let mut result = None;
        let mut index = 0;
        loop {
            let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            if SetupDiEnumDeviceInfo(set, index, &mut info).is_err() {
                break;
            }
            index += 1;
            let mut ids = [0u8; 2048];
            if SetupDiGetDeviceRegistryPropertyW(set, &info, SPDRP_HARDWAREID, None, Some(&mut ids), None).is_err() {
                continue;
            }
            let wide: Vec<u16> = ids.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            if !String::from_utf16_lossy(&wide).to_ascii_uppercase().contains("VEN_10DE") {
                continue;
            }
            // CM_POWER_DATA: PD_Size, then PD_MostRecentPowerState (1 = D0)
            let mut data = [0u8; 64];
            let mut kind = DEVPROPTYPE::default();
            if SetupDiGetDevicePropertyW(set, &info, &DEVPKEY_DEVICE_POWER_DATA, &mut kind, Some(&mut data), None, 0).is_ok() {
                result = Some(u32::from_le_bytes([data[4], data[5], data[6], data[7]]) == 1);
            }
            break;
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        result
    }
}

static NVML: Mutex<Option<nvml_wrapper::Nvml>> = Mutex::new(None);

/// NVIDIA GPU load (%) and VRAM used / total (bytes), for the overlay. None while the GPU sleeps (asking would wake it).
pub fn nvidia_usage() -> Option<(u32, u64, u64)> {
    if nvidia_awake() != Some(true) {
        return None;
    }
    let mut nvml = NVML.lock().unwrap();
    if nvml.is_none() {
        *nvml = nvml_wrapper::Nvml::init().ok();
    }
    let device = nvml.as_ref()?.device_by_index(0).ok()?;
    let load = device.utilization_rates().ok()?.gpu;
    let memory = device.memory_info().ok()?;
    Some((load, memory.used, memory.total))
}

/// NVML board power; only called while the GPU is already awake
fn gpu_power() -> Option<f64> {
    let mut nvml = NVML.lock().unwrap();
    if nvml.is_none() {
        *nvml = nvml_wrapper::Nvml::init().ok();
    }
    let mw = nvml.as_ref()?.device_by_index(0).ok()?.power_usage().ok()?;
    Some(mw as f64 / 1000.0)
}

/// Battery rate in W from root\WMI: Windows refreshes it only every 3-15 s
fn wmi_battery_rate() -> Option<f64> {
    let rows = crate::wmi::query("SELECT ChargeRate, DischargeRate FROM BatteryStatus").ok()?;
    let row = rows.first()?;
    let charge = row.get("ChargeRate").as_i64().unwrap_or(0);
    let discharge = row.get("DischargeRate").as_i64().unwrap_or(0);
    Some(if discharge > 0 { -(discharge as f64) / 1000.0 } else { charge as f64 / 1000.0 })
}

pub fn plugged_in() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.ACLineStatus == 1
}

// ---------------- battery class driver (refreshes every 1-3 s) ----------------

const GUID_DEVCLASS_BATTERY: GUID = GUID::from_u128(0x72631e54_78a4_11d0_bcf7_00aa00b7b32a);
const IOCTL_BATTERY_QUERY_TAG: u32 = 0x294040;
const IOCTL_BATTERY_QUERY_STATUS: u32 = 0x29404C;
const BATTERY_UNKNOWN_RATE: i32 = i32::MIN;

struct Battery {
    handle: HANDLE,
    tag: u32,
}
unsafe impl Send for Battery {}

static BATTERY: Mutex<Option<Battery>> = Mutex::new(None);

/// Device path of the first battery, NUL terminated
fn battery_path() -> Option<Vec<u16>> {
    unsafe {
        let set = SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_BATTERY), PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE).ok()?;
        let mut data = SP_DEVICE_INTERFACE_DATA { cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32, ..Default::default() };
        let mut path = None;
        if SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVCLASS_BATTERY, 0, &mut data).is_ok() {
            let mut size = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut size), None);
            // u64 backing keeps the detail struct aligned
            let mut buf = vec![0u64; size as usize / 8 + 2];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), size, None, None).is_ok() {
                let p = (*detail).DevicePath.as_ptr();
                let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                path = Some(std::slice::from_raw_parts(p, len + 1).to_vec());
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        path
    }
}

fn ioctl(handle: HANDLE, code: u32, input: &[u8], output: &mut [u8]) -> bool {
    let mut returned = 0u32;
    unsafe { DeviceIoControl(handle, code, Some(input.as_ptr() as _), input.len() as u32, Some(output.as_mut_ptr() as _), output.len() as u32, Some(&mut returned), None) }.is_ok()
}

fn open_battery() -> Option<Battery> {
    let path = battery_path()?;
    let handle = unsafe {
        CreateFileW(PCWSTR(path.as_ptr()), (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None).ok()?
    };
    let mut tag = [0u8; 4];
    let tag = if ioctl(handle, IOCTL_BATTERY_QUERY_TAG, &0u32.to_le_bytes(), &mut tag) { u32::from_le_bytes(tag) } else { 0 };
    if tag == 0 {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(Battery { handle, tag })
}

/// Battery rate in W from IOCTL_BATTERY_QUERY_STATUS (BATTERY_STATUS.Rate, mW, negative when discharging)
fn battery_rate() -> Option<f64> {
    let mut guard = BATTERY.lock().unwrap();
    if guard.is_none() {
        *guard = open_battery();
    }
    let battery = guard.as_ref()?;
    // BATTERY_WAIT_STATUS: tag, timeout 0 (don't wait), power state / thresholds unused
    let mut input = [0u8; 20];
    input[..4].copy_from_slice(&battery.tag.to_le_bytes());
    let mut status = [0u8; 16];
    if !ioctl(battery.handle, IOCTL_BATTERY_QUERY_STATUS, &input, &mut status) {
        // Tag changes when the battery is re-enumerated: reopen next time
        unsafe {
            let _ = CloseHandle(battery.handle);
        }
        *guard = None;
        return None;
    }
    let rate = i32::from_le_bytes(status[12..16].try_into().unwrap());
    (rate != BATTERY_UNKNOWN_RATE).then(|| rate as f64 / 1000.0)
}
