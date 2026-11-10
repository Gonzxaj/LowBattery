//! Simulated devices so the UI can be developed and screenshotted without
//! hardware (default provider on non-Windows builds, `--demo` on Windows).
use crate::{BatteryProvider, BatteryStatus, Charge, Device, DeviceKind};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Default)]
pub struct Demo {
    tick: AtomicU32,
}

const DEVICES: [(&str, &str, DeviceKind); 4] = [
    ("MX Master 3S", "Logitech", DeviceKind::Mouse),
    ("Arctis Nova 7", "SteelSeries", DeviceKind::Headset),
    ("Xbox Wireless Controller", "Microsoft", DeviceKind::Controller),
    ("K380", "Logitech", DeviceKind::Keyboard),
];

impl BatteryProvider for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }
    fn discover(&self) -> Vec<Device> {
        self.tick.fetch_add(1, Ordering::Relaxed);
        DEVICES
            .iter()
            .enumerate()
            .map(|(i, (name, vendor, kind))| Device {
                id: format!("demo:{i}"),
                name: name.to_string(),
                vendor: vendor.to_string(),
                model: None,
                kind: *kind,
                provider: "demo",
                locator: i.to_string(),
            })
            .collect()
    }
    fn read_battery(&self, device: &Device) -> BatteryStatus {
        let t = self.tick.load(Ordering::Relaxed);
        match device.locator.as_str() {
            "0" => BatteryStatus::connected(82u32.saturating_sub(t) as u8, Charge::Discharging),
            "1" => BatteryStatus::connected(15, Charge::Discharging),
            "2" => BatteryStatus::connected((40 + t * 3).min(100) as u8, Charge::Charging),
            _ => BatteryStatus::ASLEEP,
        }
    }
}
