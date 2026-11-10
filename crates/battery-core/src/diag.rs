//! What Windows exposes, written to the log so a user can send it in when a
//! peripheral is missing. No network, no API: the log file is the channel.

use crate::monitor::DeviceState;
use serde_json::{json, Value};
use std::collections::HashSet;

/// Full snapshot of vendor HID interfaces and Bluetooth battery nodes.
pub fn diagnostics() -> Value {
    #[cfg(windows)]
    {
        let hid: Vec<Value> = crate::hid::win::enumerate()
            .into_iter()
            .filter(|d| VENDORS.contains(&d.vendor_id()))
            .map(|d| {
                json!({
                    "vid": format!("{:04x}", d.vendor_id()), "pid": format!("{:04x}", d.product_id()),
                    "usage_page": format!("{:04x}", d.usage_page()), "usage": format!("{:04x}", d.usage()),
                    "interface": d.interface_number(), "product": d.product_string(),
                })
            })
            .collect();
        json!({ "hid": hid, "bluetooth": crate::providers::bluetooth::diagnostics() })
    }
    #[cfg(not(windows))]
    json!({ "hid": [], "bluetooth": [] })
}

/// Vendors with a dedicated provider.
pub const VENDORS: [u16; 3] = [crate::providers::logitech::VID, crate::providers::steelseries::VID, crate::providers::razer::VID];

/// USB `vid_xxxx&pid_xxxx` key of a Windows HID path, lower-case.
pub fn usb_key(path: &str) -> Option<String> {
    let p = path.to_ascii_lowercase();
    let i = p.find("vid_")?;
    let key = p.get(i..i + 17)?; // "vid_046d&pid_c54d"
    (key.as_bytes()[8] == b'&' && key[9..].starts_with("pid_")).then(|| key.to_string())
}

/// Logs, once per run, every keyboard / mouse / gamepad / headset-like HID
/// device that no provider turned into a card, with what's needed to add it.
#[derive(Default)]
pub struct UnsupportedReporter {
    reported: HashSet<String>,
}

impl UnsupportedReporter {
    pub fn check(&mut self, shown: &[DeviceState]) {
        let handled: HashSet<String> = shown.iter().filter_map(|s| usb_key(&s.device.locator)).collect();
        for (key, summary) in candidates() {
            if !handled.contains(&key) && self.reported.insert(key.clone()) {
                log::info!(
                    "UNSUPPORTED peripheral {key} {summary} - detected but no battery protocol for it yet. \
                     If it is wireless and should show up, send this log file to add support."
                );
            }
        }
    }
}

/// (usb key, human summary) of peripheral-looking HID devices, one per device.
#[cfg(windows)]
fn candidates() -> Vec<(String, String)> {
    use std::collections::BTreeMap;
    let mut by_key: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for d in crate::hid::win::enumerate() {
        let path = d.path().to_string_lossy().into_owned();
        let Some(key) = usb_key(&path) else { continue };
        let entry = by_key.entry(key).or_insert_with(|| {
            let name = format!("\"{}\" by \"{}\"", d.product_string().unwrap_or("?"), d.manufacturer_string().unwrap_or("?"));
            (name, Vec::new())
        });
        entry.1.push(format!("if{}:{:04x}/{:04x}", d.interface_number(), d.usage_page(), d.usage()));
    }
    by_key
        .into_iter()
        .filter(|(_, (_, cols))| cols.iter().any(|c| PERIPHERAL.iter().any(|p| c.ends_with(p))))
        .map(|(k, (name, cols))| (k, format!("{name} collections [{}]", cols.join(", "))))
        .collect()
}

#[cfg(not(windows))]
fn candidates() -> Vec<(String, String)> {
    Vec::new()
}

/// Generic Desktop mouse / joystick / gamepad / keyboard, Consumer Control (headsets).
#[cfg_attr(not(windows), allow(dead_code))]
const PERIPHERAL: [&str; 5] = ["0001/0002", "0001/0004", "0001/0005", "0001/0006", "000c/0001"];

#[cfg(test)]
mod tests {
    #[test]
    fn usb_key_from_windows_paths() {
        let p = r"\\?\HID#VID_046D&PID_C54D&MI_02&Col02#8&2b2&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(super::usb_key(p).as_deref(), Some("vid_046d&pid_c54d"));
        assert_eq!(super::usb_key("{f7a14153-b985-5303-bf2a-9b2455087e18}"), None);
        assert_eq!(super::usb_key("vid_12"), None);
    }
}
