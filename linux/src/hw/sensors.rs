//! Temperatures, fans, battery draw and the NVIDIA GPU. hwmon: k10temp / coretemp for the CPU, whichever driver
//! reports fans (lenovo-wmi-other, legion-laptop). NVML only while the dGPU is awake: querying a suspended
//! NVIDIA GPU wakes it and costs battery.

use once_cell::sync::{Lazy, OnceCell};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct Reading {
    pub cpu_temp: Option<i32>,
    pub gpu_temp: Option<i32>,
    pub cpu_fan: Option<u32>,
    pub gpu_fan: Option<u32>,
    pub gpu_power: Option<f32>,
    pub gpu_asleep: bool,
    pub battery_percent: Option<i32>,
    /// Watts, negative while discharging
    pub battery_rate: Option<f32>,
}

struct Paths {
    cpu_temp: Option<PathBuf>,
    fans: Vec<PathBuf>,
}

static PATHS: Lazy<Paths> = Lazy::new(|| {
    let mut cpu_temp = None;
    let mut fans = Vec::new();
    for h in super::dirs("/sys/class/hwmon") {
        let name = super::read(h.join("name")).unwrap_or_default();
        match name.as_str() {
            // Tctl on AMD
            "k10temp" | "zenpower" if cpu_temp.is_none() => cpu_temp = Some(h.join("temp1_input")),
            // "Package id 0" on Intel
            "coretemp" if cpu_temp.is_none() => {
                cpu_temp = (1..10).map(|i| h.join(format!("temp{i}_label"))).find(|l| super::read(l).is_some_and(|t| t.starts_with("Package"))).map(|l| PathBuf::from(l.to_string_lossy().replace("_label", "_input")));
            }
            _ => {}
        }
        // The nvidia / amdgpu fans (if any) are the GPU's own, not the laptop's
        if !matches!(name.as_str(), "nvidia" | "amdgpu") {
            for i in 1..=4 {
                let f = h.join(format!("fan{i}_input"));
                if f.exists() {
                    fans.push(f);
                }
            }
        }
    }
    crate::log!("Sensors: CPU {:?}, fans {:?}", cpu_temp, fans);
    Paths { cpu_temp, fans }
});

/// NVIDIA dGPU on the PCI bus (vendor 10de, display class)
static NVIDIA: Lazy<Option<PathBuf>> = Lazy::new(|| {
    super::dirs("/sys/bus/pci/devices")
        .into_iter()
        .find(|d| super::read(d.join("vendor")).as_deref() == Some("0x10de") && super::read(d.join("class")).is_some_and(|c| c.starts_with("0x03")))
});

pub fn nvidia_present() -> bool {
    NVIDIA.is_some()
}

/// Runtime-suspended (D3cold) or bound to no driver
pub fn gpu_asleep() -> bool {
    match NVIDIA.as_ref() {
        Some(d) => super::read(d.join("power/runtime_status")).as_deref() == Some("suspended"),
        None => true,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GpuReading {
    pub temp: Option<i32>,
    pub power: Option<f32>,
    /// 0..1
    pub usage: Option<f32>,
}

/// NVIDIA temperature, power and usage, only while the dGPU is awake anyway (proprietary driver). NVML is opened
/// per reading and closed again: a process holding the NVIDIA device open keeps it from suspending.
pub fn nvidia() -> Option<GpuReading> {
    static LOGGED: OnceCell<()> = OnceCell::new();
    if gpu_asleep() || !std::path::Path::new("/proc/driver/nvidia/version").exists() {
        return None;
    }
    let nvml = nvml_wrapper::Nvml::init().map_err(|e| LOGGED.get_or_init(|| crate::log!("NVML not available: {e}"))).ok()?;
    let device = nvml.device_by_index(0).ok()?;
    Some(GpuReading {
        temp: device.temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu).ok().map(|t| t as i32),
        power: device.power_usage().ok().map(|mw| mw as f32 / 1000.0),
        usage: device.utilization_rates().ok().map(|u| u.gpu as f32 / 100.0),
    })
}

fn power_supplies() -> Vec<PathBuf> {
    super::dirs("/sys/class/power_supply")
}

pub fn plugged_in() -> bool {
    let supplies = power_supplies();
    let mains: Vec<&PathBuf> = supplies.iter().filter(|p| super::read(p.join("type")).as_deref() == Some("Mains")).collect();
    if mains.is_empty() {
        return true;
    }
    mains.iter().any(|p| super::read_i64(p.join("online")) == Some(1))
}

pub fn read() -> Reading {
    let mut r = Reading::default();
    r.cpu_temp = PATHS.cpu_temp.as_ref().and_then(super::read_i64).map(|m| (m / 1000) as i32);
    let fans: Vec<u32> = PATHS.fans.iter().filter_map(super::read_i64).map(|v| v as u32).collect();
    r.cpu_fan = fans.first().copied();
    r.gpu_fan = fans.get(1).copied();

    if nvidia_present() {
        r.gpu_asleep = gpu_asleep();
        if let Some(g) = nvidia() {
            r.gpu_temp = g.temp;
            r.gpu_power = g.power;
        }
    }

    if let Some(bat) = super::battery::battery() {
        r.battery_percent = super::read_i64(bat.join("capacity")).map(|v| v as i32);
        let watts = super::read_i64(bat.join("power_now")).map(|uw| uw as f32 / 1_000_000.0).or_else(|| {
            let (ua, uv) = (super::read_i64(bat.join("current_now"))?, super::read_i64(bat.join("voltage_now"))?);
            Some(ua as f32 * uv as f32 / 1e12)
        });
        let discharging = super::read(bat.join("status")).as_deref() == Some("Discharging");
        r.battery_rate = watts.map(|w| if discharging { -w.abs() } else { w.abs() });
    }
    r
}
