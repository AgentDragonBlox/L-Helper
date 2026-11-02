#![allow(non_snake_case)]
//! Windows-side actions behind Lenovo Fn keys: mic mute, airplane mode, snipping tool

use windows::core::{interface, IUnknown, IUnknown_Vtbl, HRESULT, GUID};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

fn com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Mutes / unmutes every active capture device. Returns true when now muted.
pub fn toggle_microphone() -> Result<bool, String> {
    com();
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let devices = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).map_err(|e| e.to_string())?;
        let count = devices.GetCount().map_err(|e| e.to_string())?;
        let mut volumes = Vec::new();
        for i in 0..count {
            if let Ok(device) = devices.Item(i) {
                if let Ok(v) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    volumes.push(v);
                }
            }
        }
        let all_muted = volumes.iter().all(|v| v.GetMute().map(|m| m.as_bool()).unwrap_or(false));
        for v in &volumes {
            let _ = v.SetMute(!all_muted, std::ptr::null());
        }
        crate::log!("Microphone muted: {}", !all_muted);
        Ok(!all_muted)
    }
}

// Undocumented RadioManager COM object, same as LLT
#[interface("DB3AFBFB-08E6-46C6-AA70-BF9A34C30AB7")]
unsafe trait IRadioManager: IUnknown {
    fn slot3(&self) -> HRESULT;
    fn slot4(&self) -> HRESULT;
    fn GetSystemRadioState(&self, state: *mut i32, a: *mut i32, b: *mut i32) -> HRESULT;
    fn SetSystemRadioState(&self, state: i32) -> HRESULT;
}

/// Returns true when airplane mode is now on
pub fn toggle_airplane_mode() -> Result<bool, String> {
    com();
    const CLSID: GUID = GUID::from_u128(0x581333F6_28DB_41BE_BC7A_FF201F12F3F6);
    unsafe {
        let manager: IRadioManager = CoCreateInstance(&CLSID, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let (mut state, mut a, mut b) = (0, 0, 0);
        manager.GetSystemRadioState(&mut state, &mut a, &mut b).ok().map_err(|e| e.to_string())?;
        let new_state = if state == 0 { 1 } else { 0 };
        manager.SetSystemRadioState(new_state).ok().map_err(|e| e.to_string())?;
        crate::log!("Airplane mode: {}", new_state == 0);
        Ok(new_state == 0)
    }
}

pub fn open_snipping_tool() {
    let _ = std::process::Command::new(super::system_exe("explorer.exe")).arg("ms-screenclip:").spawn();
}
