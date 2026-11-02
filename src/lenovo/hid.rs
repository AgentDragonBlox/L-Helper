//! Raw HID helpers (feature reports and capability lookup) for the Lenovo ITE keyboard controller (VID 048D)

use windows::core::PCWSTR;
use windows::Win32::Devices::HumanInterfaceDevice::*;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::*;

pub const LENOVO_VID: u16 = 0x048D;

pub struct HidInfo {
    pub path: String,
    pub pid: u16,
    #[allow(dead_code)]
    pub interface: i32,
    pub feature_len: u16,
}

/// All HID interfaces of a vendor with their feature report length
pub fn enumerate(vid: u16) -> Vec<HidInfo> {
    let mut out = Vec::new();
    let Ok(api) = hidapi::HidApi::new() else { return out };
    for d in api.device_list().filter(|d| d.vendor_id() == vid) {
        let path = d.path().to_string_lossy().to_string();
        let feature_len = feature_length(&path).unwrap_or(0);
        out.push(HidInfo { path, pid: d.product_id(), interface: d.interface_number(), feature_len });
    }
    out
}

fn open(path: &str, access: u32) -> Option<HANDLE> {
    let w: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { CreateFileW(PCWSTR(w.as_ptr()), access, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None).ok() }
}

pub fn feature_length(path: &str) -> Option<u16> {
    let handle = open(path, 0)?;
    unsafe {
        let mut preparsed = PHIDP_PREPARSED_DATA::default();
        let ok = HidD_GetPreparsedData(handle, &mut preparsed);
        let result = if ok {
            let mut caps = HIDP_CAPS::default();
            let r = HidP_GetCaps(preparsed, &mut caps);
            let _ = HidD_FreePreparsedData(preparsed);
            if r.is_ok() { Some(caps.FeatureReportByteLength) } else { None }
        } else {
            None
        };
        let _ = CloseHandle(handle);
        result
    }
}

/// An open HID device for feature reports
pub struct HidDevice(HANDLE);
unsafe impl Send for HidDevice {}

impl HidDevice {
    pub fn open(path: &str) -> Option<HidDevice> {
        open(path, (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0).map(HidDevice)
    }

    pub fn set_feature(&self, data: &[u8]) -> bool {
        unsafe { HidD_SetFeature(self.0, data.as_ptr() as _, data.len() as u32) }
    }

    pub fn get_feature(&self, buffer: &mut [u8]) -> bool {
        unsafe { HidD_GetFeature(self.0, buffer.as_mut_ptr() as _, buffer.len() as u32) }
    }
}

impl Drop for HidDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
