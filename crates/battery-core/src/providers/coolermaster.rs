//! Cooler Master wireless mice (VID 2516): MM712 (cable 0169), MM731 (cable
//! 0167) and the shared "Cooler Master Mice Dongle" (01B7). Vendor interface
//! 3, usage page FF0A. Request [46][81][00] -> status [47][81][model] ...
//! [10] percent, [11] charging, [12] link. The mouse also pushes the status
//! report when the level changes; a sleeping mouse does not answer.
//! MM712 verified on hardware upstream; MM731 bytes are unverified.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind};

pub const VID: u16 = 0x2516;
pub const PIDS: [u16; 3] = [0x01B7, 0x0169, 0x0167];
const USAGE_PAGE: u16 = 0xFF0A;
const TIMEOUT_MS: u64 = 500;
/// Output-report sizes retried when the bare request gets no answer.
const PADS: [usize; 2] = [64, 32];

fn model_name(pid: u16, model: u8) -> &'static str {
    if pid == 0x0169 || (pid == 0x01B7 && model == 0x50) {
        "Cooler Master MM712"
    } else if pid == 0x0167 || (pid == 0x01B7 && model == 0x70) {
        "Cooler Master MM731"
    } else {
        "Cooler Master wireless mouse"
    }
}

/// Status report -> (model byte, battery). None = not a status reply.
pub fn decode(r: &[u8]) -> Option<(u8, BatteryStatus)> {
    if r.len() < 13 || r[0] != 0x47 || r[1] != 0x81 {
        return None;
    }
    let status = BatteryStatus {
        level: (1..=100).contains(&r[10]).then_some(r[10]),
        charge: if r[11] != 0 { Charge::Charging } else { Charge::Discharging },
        link: crate::Link::Connected,
    };
    Some((r[2], status))
}

/// One status exchange. Ok(None) = silent (asleep / out of range).
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    query(io, &[0x00, 0x46, 0x81, 0x00], &PADS, TIMEOUT_MS, |r| decode(r).map(|(_, s)| s))
        .ok()
        .flatten()
}

/// (model name, status) of one answered collection.
pub fn probe(io: &dyn HidIo, pid: u16) -> Option<(String, BatteryStatus)> {
    query(io, &[0x00, 0x46, 0x81, 0x00], &PADS, TIMEOUT_MS, |r| {
        decode(r).map(|(model, s)| (model_name(pid, model).to_string(), s))
    })
    .ok()
    .flatten()
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::CoolerMaster;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// One card per mouse model: cable and dongle links of the same mouse
    /// would otherwise show twice. `known` keeps the card (asleep) while the
    /// mouse is silent but its collections are still plugged in.
    #[derive(Default)]
    pub struct CoolerMaster {
        known: Mutex<BTreeMap<String, Device>>,
    }

    /// Every FF0A collection on a Cooler Master id, dongle last so a cable
    /// link (which names the real model) is probed first.
    fn collections() -> Vec<hidapi::DeviceInfo> {
        let mut out: Vec<_> = enumerate()
            .into_iter()
            .filter(|d| d.vendor_id() == VID && PIDS.contains(&d.product_id()) && d.usage_page() == USAGE_PAGE)
            .collect();
        out.sort_by_key(|d| (d.product_id() == 0x01B7, d.interface_number() != 3));
        out
    }

    impl BatteryProvider for CoolerMaster {
        fn name(&self) -> &'static str {
            "coolermaster"
        }

        fn discover(&self) -> Vec<Device> {
            let cols = collections();
            let mut known = self.known.lock().unwrap();
            // model name -> every path that answered as that model
            let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut present = Vec::new();
            for d in &cols {
                let path = d.path().to_string_lossy().into_owned();
                present.push(path.clone());
                if let Some(io) = open(&path) {
                    if let Some((name, _)) = probe(&io, d.product_id()) {
                        by_name.entry(name).or_default().push(path);
                    }
                }
            }
            let mut out = Vec::new();
            for (name, mut paths) in by_name {
                paths.sort();
                paths.dedup();
                let device = Device {
                    id: format!("coolermaster:{}", name.to_lowercase().replace(' ', "-")),
                    name: name.clone(),
                    vendor: "Cooler Master".into(),
                    model: Some(name.clone()),
                    kind: DeviceKind::Mouse,
                    provider: "coolermaster",
                    locator: paths.join("\t"),
                };
                known.insert(name, device.clone());
                out.push(device);
            }
            // Silent but still plugged in: keep last cards as asleep.
            known.retain(|_, dev| dev.locator.split('\t').any(|p| present.iter().any(|q| q == p)));
            for dev in known.values() {
                if !out.iter().any(|d| d.id == dev.id) {
                    out.push(dev.clone());
                }
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            for path in device.locator.split('\t') {
                if let Some(io) = open(path) {
                    if let Some(s) = poll(&io) {
                        return s;
                    }
                }
            }
            BatteryStatus::ASLEEP
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;
    use crate::Link;

    fn status(model: u8, pct: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0x47, 0x81, model];
        r.resize(13, 0);
        r[10] = pct;
        r[11] = charging;
        r[12] = 1;
        r
    }

    #[test]
    fn exchange_and_decode() {
        let io = FakeHid::new(|req| {
            assert_eq!(&req[..4], &[0x00, 0x46, 0x81, 0x00]);
            vec![vec![0x02, 0x00], status(0x50, 65, 1)]
        });
        let (name, s) = probe(&io, 0x01B7).unwrap();
        assert_eq!(name, "Cooler Master MM712");
        assert_eq!((s.level, s.charge, s.link), (Some(65), Charge::Charging, Link::Connected));
        assert_eq!(model_name(0x0167, 0), "Cooler Master MM731");
        assert_eq!(model_name(0x01B7, 0x70), "Cooler Master MM731");
        assert_eq!(decode(&status(0x50, 0, 0)).unwrap().1.level, None);
    }

    #[test]
    fn silent_is_none() {
        let io = FakeHid::new(|_| vec![]);
        assert_eq!(poll(&io), None);
    }
}
