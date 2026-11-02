//! GPU modes: Eco (iGPU only), Standard (hybrid), Optimized (firmware auto), Ultimate (dGPU via MUX, needs restart)

use crate::config;
use crate::lenovo::{gamezone, machine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuMode {
    Eco,
    Standard,
    Optimized,
    Ultimate,
}

impl GpuMode {
    pub fn name(self) -> &'static str {
        match self {
            GpuMode::Eco => "Eco",
            GpuMode::Standard => "Standard",
            GpuMode::Optimized => "Optimized",
            GpuMode::Ultimate => "Ultimate",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            GpuMode::Eco => "iGPU only, dGPU disconnected for best battery life",
            GpuMode::Standard => "Hybrid: iGPU drives the screen, dGPU on demand",
            GpuMode::Optimized => "Hybrid on AC, iGPU only on battery (firmware auto)",
            GpuMode::Ultimate => "dGPU only through the MUX switch. Requires restart",
        }
    }
}

pub fn supported() -> bool {
    let m = machine::get();
    m.supports_igpu_mode || m.supports_gsync
}

pub fn available() -> Vec<GpuMode> {
    let m = machine::get();
    let mut v = Vec::new();
    if m.supports_igpu_mode {
        v.extend([GpuMode::Eco, GpuMode::Standard, GpuMode::Optimized]);
    } else if m.supports_gsync {
        v.push(GpuMode::Standard);
    }
    if m.supports_gsync {
        v.push(GpuMode::Ultimate);
    }
    v
}

pub fn get() -> Option<GpuMode> {
    let m = machine::get();
    if m.supports_gsync && gamezone::gsync().ok()? == 1 {
        return Some(GpuMode::Ultimate);
    }
    if !m.supports_igpu_mode {
        return Some(GpuMode::Standard);
    }
    Some(match gamezone::igpu_mode().ok()? {
        1 => GpuMode::Eco,
        2 => GpuMode::Optimized,
        _ => GpuMode::Standard,
    })
}

/// Returns true when a restart is needed
pub fn set(mode: GpuMode) -> Result<bool, String> {
    let m = machine::get();
    crate::log!("GPU mode: {:?} -> {mode:?}", get());
    let mut reboot = false;
    if m.supports_gsync {
        let want = (mode == GpuMode::Ultimate) as i32;
        if gamezone::gsync()? != want {
            gamezone::set_gsync(want)?;
            reboot = true;
        }
    }
    if m.supports_igpu_mode && mode != GpuMode::Ultimate {
        let want = match mode {
            GpuMode::Eco => 1,
            GpuMode::Optimized => 2,
            _ => 0,
        };
        if gamezone::igpu_mode()? != want {
            gamezone::set_igpu_mode(want)?;
            if !reboot {
                notify_later();
            }
        }
    }
    config::set("gpu_mode", mode as i64);
    Ok(reboot)
}

/// Like LLT's DGPUNotify: a few seconds after switching, tell firmware whether the dGPU is still present
fn notify_later() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let available = dgpu_present();
        match gamezone::notify_dgpu(available as i32) {
            Ok(_) => crate::log!("dGPU notify: {available}"),
            Err(e) => crate::log!("dGPU notify failed: {e}"),
        }
    });
}

/// NVIDIA / discrete AMD display adapter currently enumerated and started
pub fn dgpu_present() -> bool {
    let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).open_subkey(r"SYSTEM\CurrentControlSet\Services\nvlddmkm\Enum") else {
        return false;
    };
    key.get_value::<u32, _>("Count").unwrap_or(0) > 0
}
