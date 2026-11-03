//! The ITE 24-zone keyboard controller (VID 048D, interface 1) over /dev/hidraw: devices from /sys/class/hidraw,
//! feature report sizes from the report descriptor, writes with the HIDIOCSFEATURE ioctl. The udev rule
//! (packaging/70-lhelper.rules) gives the signed-in user access to these nodes, so no root is needed.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const VID: u32 = 0x048D;
/// The lighting reports are 51 bytes on a LOQ 15AHP10; the controller's other interfaces have smaller ones
const MIN_FEATURE: usize = 51;

static DENIED: AtomicBool = AtomicBool::new(false);

struct Node {
    dev: PathBuf,
    pid: u32,
    interface: Option<u32>,
    /// (report ID, bytes including the ID)
    features: Vec<(u8, usize)>,
}

impl Node {
    fn max_feature(&self) -> usize {
        self.features.iter().map(|f| f.1).max().unwrap_or(0)
    }
}

fn nodes() -> Vec<Node> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/sys/class/hidraw") else { return out };
    for entry in dir.flatten() {
        let sys = entry.path();
        let uevent = std::fs::read_to_string(sys.join("device/uevent")).unwrap_or_default();
        // HID_ID=0003:0000048D:0000C693 (bus:vendor:product)
        let Some(id) = uevent.lines().find_map(|l| l.strip_prefix("HID_ID=")) else { continue };
        let parts: Vec<u32> = id.split(':').filter_map(|p| u32::from_str_radix(p, 16).ok()).collect();
        if parts.len() != 3 || parts[1] != VID {
            continue;
        }
        let interface = std::fs::read_to_string(sys.join("device/../bInterfaceNumber")).ok().and_then(|s| u32::from_str_radix(s.trim(), 16).ok());
        let features = std::fs::read(sys.join("device/report_descriptor")).map(|d| feature_sizes(&d)).unwrap_or_default();
        out.push(Node { dev: PathBuf::from("/dev").join(entry.file_name()), pid: parts[2], interface, features });
    }
    out.sort_by(|a, b| a.dev.cmp(&b.dev));
    out
}

fn keyboard_node() -> Option<Node> {
    nodes().into_iter().find(|n| n.interface == Some(1) && n.max_feature() >= MIN_FEATURE)
}

pub fn keyboard_present() -> bool {
    keyboard_node().is_some()
}

/// The keyboard exists but its node isn't ours to open (udev rule missing, or not the active session)
pub fn permission_denied() -> bool {
    DENIED.load(Ordering::SeqCst)
}

pub fn open_keyboard() -> Option<HidDevice> {
    let node = keyboard_node()?;
    match OpenOptions::new().read(true).write(true).open(&node.dev) {
        Ok(file) => {
            DENIED.store(false, Ordering::SeqCst);
            crate::log!("24-zone keyboard: {:04X}:{:04X} at {} (feature reports {:?})", VID, node.pid, node.dev.display(), node.features);
            Some(HidDevice { file, features: node.features })
        }
        Err(e) => {
            // Reconnects retry every second: log a denied node once
            let denied = e.kind() == std::io::ErrorKind::PermissionDenied;
            if !(denied && DENIED.swap(true, Ordering::SeqCst)) {
                crate::log!("24-zone keyboard: can't open {}: {e}", node.dev.display());
            }
            None
        }
    }
}

/// Feature report sizes per report ID from a HID report descriptor (short items only; enough for this controller)
fn feature_sizes(desc: &[u8]) -> Vec<(u8, usize)> {
    let (mut size, mut count, mut id) = (0u32, 0u32, 0u8);
    let mut bits: Vec<(u8, u32)> = Vec::new();
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // long item: size byte, tag byte, data
            i += 3 + desc.get(i + 1).copied().unwrap_or(0) as usize;
            continue;
        }
        let len = [0, 1, 2, 4][(prefix & 3) as usize];
        let value = (0..len).fold(0u32, |v, b| v | (desc.get(i + 1 + b).copied().unwrap_or(0) as u32) << (8 * b));
        match prefix & 0xFC {
            0x74 => size = value,
            0x94 => count = value,
            0x84 => id = value as u8,
            0xB0 => match bits.iter_mut().find(|(r, _)| *r == id) {
                Some(entry) => entry.1 += size * count,
                None => bits.push((id, size * count)),
            },
            _ => {}
        }
        i += 1 + len;
    }
    bits.into_iter().map(|(r, b)| (r, (b as usize).div_ceil(8) + usize::from(r != 0))).collect()
}

/// _IOWR('H', 0x06, len): HIDIOCSFEATURE
fn hidiocsfeature(len: usize) -> u64 {
    (3u64 << 30) | ((len as u64) << 16) | ((b'H' as u64) << 8) | 0x06
}

pub struct HidDevice {
    file: File,
    features: Vec<(u8, usize)>,
}

impl HidDevice {
    /// Pads / truncates to the report's size, as hidapi does on Windows (the first byte is the report ID)
    pub fn send_feature_report(&self, data: &[u8]) -> std::io::Result<()> {
        let id = data.first().copied().unwrap_or(0);
        let len = self.features.iter().find(|f| f.0 == id).map(|f| f.1).unwrap_or_else(|| self.features.iter().map(|f| f.1).max().unwrap_or(data.len()));
        let mut buf = data.to_vec();
        buf.resize(len, 0);
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), hidiocsfeature(len) as _, buf.as_mut_ptr()) };
        if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_with_two_feature_reports() {
        // Usage page (vendor), collection, report ID 4: 8 bits x 50, feature; report ID 5: 8 bits x 50, feature
        let desc = [0x06, 0x89, 0xFF, 0xA1, 0x01, 0x85, 0x04, 0x75, 0x08, 0x95, 0x32, 0xB1, 0x02, 0x85, 0x05, 0x95, 0x32, 0xB1, 0x02, 0xC0];
        assert_eq!(feature_sizes(&desc), vec![(4, 51), (5, 51)]);
    }

    #[test]
    fn ioctl_number() {
        // HIDIOCSFEATURE(65) as the kernel headers define it
        assert_eq!(hidiocsfeature(65), 0xC0414806);
    }
}
