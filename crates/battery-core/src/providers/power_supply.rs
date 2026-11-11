//! Linux: peripheral batteries the kernel already publishes in
//! /sys/class/power_supply (scope "Device"). Covers Logitech HID++
//! (hid-logitech-hidpp: Unifying, Bolt, Lightspeed, Bluetooth), Bluetooth HID
//! and any HID device declaring "Battery Strength" (hid-input), plus game
//! controllers. Plain sysfs reads: no root, no udev rule.

use crate::{BatteryStatus, Charge, Link};

pub const ROOT: &str = "/sys/class/power_supply";

/// Status from a power_supply node's attributes (`attr("capacity")` -> file content).
pub fn status(attr: impl Fn(&str) -> Option<String>) -> BatteryStatus {
    // hidpp sets online=0 while the device sleeps or is out of range.
    if attr("online").as_deref() == Some("0") || attr("present").as_deref() == Some("0") {
        return BatteryStatus::ASLEEP;
    }
    let charge = match attr("status").as_deref() {
        Some("Charging") => Charge::Charging,
        Some("Full") => Charge::Full,
        Some("Discharging" | "Not charging") => Charge::Discharging,
        _ => Charge::Unknown,
    };
    // shortcut: devices reporting only capacity_level get a coarse midpoint;
    // fine for a low-battery alert, upgrade if users want exact steps.
    let level = attr("capacity").and_then(|c| c.parse::<u8>().ok()).or_else(|| match attr("capacity_level")?.as_str() {
        "Full" => Some(100),
        "High" => Some(80),
        "Normal" => Some(55),
        "Low" => Some(20),
        "Critical" => Some(5),
        _ => None,
    });
    BatteryStatus { level: level.map(|l| l.min(100)), charge, link: Link::Connected }
}

#[cfg(target_os = "linux")]
pub use linux::{diagnostics, PowerSupply};

#[cfg(target_os = "linux")]
mod linux {
    use crate::{BatteryProvider, BatteryStatus, Device};
    use serde_json::{json, Value};
    use std::path::Path;

    pub struct PowerSupply;

    fn attr(dir: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Peripheral nodes as (canonical sysfs dir, name); laptop/UPS batteries are scope "System".
    fn nodes() -> Vec<(std::path::PathBuf, String)> {
        let Ok(dir) = std::fs::read_dir(super::ROOT) else { return Vec::new() };
        let mut out: Vec<_> = dir
            .flatten()
            .filter_map(|e| Some((std::fs::canonicalize(e.path()).ok()?, e.file_name().to_string_lossy().into_owned())))
            .filter(|(p, _)| attr(p, "scope").as_deref() == Some("Device"))
            .collect();
        out.sort();
        out
    }

    /// SteelSeries models the hidraw provider already reads (kernel hid-steelseries
    /// also publishes some): one card per device.
    fn handled_elsewhere(dir: &Path) -> bool {
        crate::diag::usb_keys(&dir.to_string_lossy()).iter().any(|k| {
            let ss = format!("vid_{:04x}&pid_", crate::providers::steelseries::VID);
            k.strip_prefix(&ss).and_then(|p| u16::from_str_radix(p, 16).ok()).is_some_and(|pid| crate::providers::steelseries::model(pid).is_some())
        })
    }

    impl BatteryProvider for PowerSupply {
        fn name(&self) -> &'static str {
            "power-supply"
        }

        fn discover(&self) -> Vec<Device> {
            let mut out: Vec<Device> = Vec::new();
            for (dir, node) in nodes() {
                if handled_elsewhere(&dir) {
                    continue;
                }
                let name = attr(&dir, "model_name").unwrap_or_else(|| node.clone());
                // hidpp_battery_N / hid-...-battery renumber on reconnect; serial + name don't.
                let id = format!("ps:{}:{name}", attr(&dir, "serial_number").unwrap_or_default());
                if out.iter().any(|d| d.id == id) {
                    continue;
                }
                out.push(Device {
                    id,
                    kind: crate::guess_kind(&name),
                    vendor: attr(&dir, "manufacturer").unwrap_or_default(),
                    model: Some(name.clone()),
                    name,
                    provider: "power-supply",
                    locator: dir.to_string_lossy().into_owned(),
                });
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let dir = Path::new(&device.locator);
            if !dir.exists() {
                return BatteryStatus::GONE;
            }
            super::status(|a| attr(dir, a))
        }
    }

    pub fn diagnostics() -> Value {
        nodes()
            .into_iter()
            .map(|(dir, node)| {
                let a = |n| attr(&dir, n);
                json!({ "node": node, "path": dir, "model": a("model_name"), "capacity": a("capacity"),
                        "capacity_level": a("capacity_level"), "status": a("status"), "online": a("online") })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn st(pairs: &[(&str, &str)]) -> BatteryStatus {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        status(|a| m.get(a).cloned())
    }

    #[test]
    fn reads_sysfs_attributes() {
        assert_eq!(st(&[("capacity", "42"), ("status", "Discharging"), ("online", "1")]), BatteryStatus::connected(42, Charge::Discharging));
        assert_eq!(st(&[("capacity_level", "Low"), ("status", "Charging")]), BatteryStatus::connected(20, Charge::Charging));
        assert_eq!(st(&[("capacity", "90"), ("online", "0")]), BatteryStatus::ASLEEP);
        assert_eq!(st(&[("status", "Unknown")]).level, None);
    }
}
