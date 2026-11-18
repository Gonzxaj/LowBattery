//! JBL Quantum 520 / 650 / 850 / 950 (and X variants) headsets, vendor 0ECB,
//! "HIDV3" protocol on the vendor collection with usage page FF12 (64-byte
//! reports: input 86, output 87, feature 88).
//!   request (feature 88): [88][00 DD][01 00][01][00][02 00][feature lo][feature hi]
//!            identifier DD00 = the device itself, command 0001 = get device
//!            info, 1 packet, index 0, payload = the 2-byte feature id
//!   reply   (input 86):   [86][00 DD][01 00][01][00][len lo][len hi]
//!                         [status lo][status hi][feature lo][feature hi]
//!                         [size lo][size hi][value...]
//!            status bit 15 set = error
//! Battery = feature 000D, one byte: bit 7 charging, bits 0..6 percent.
//! With the headset switched off the dongle does not answer at all, so a
//! timeout means "not there".
//! Read from JBL QuantumENGINE 2.9.4 (HIDV3Device / PowerFeature.Battery)
//! and checked upstream against a Quantum 650 Wireless on its 2.4 GHz dongle;
//! reimplemented here, no code taken.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x0ECB;
const BATTERY_FEATURE: u16 = 0x000D;
const TIMEOUT_MS: u64 = 800;
/// Feature-report sizes tried in order: the descriptor length is not exposed
/// by hidapi, and Windows wants the exact size.
const FEAT_SIZES: [usize; 3] = [64, 32, 11];

pub const PIDS: [u16; 14] = [
    0x2125, 0x2126, 0x213B, 0x213D, 0x2127, 0x2128, 0x213E, 0x2140, 0x218D, 0x218E, 0x21A7, 0x21A9, 0x219A, 0x219B,
];

pub struct Product {
    pub name: &'static str,
    /// 2.4 GHz dongle PID, then the headset's own USB cable PID.
    pub dongle: u16,
    pub cable: u16,
}

pub const PRODUCTS: &[Product] = &[
    Product { name: "JBL Quantum 650 Wireless", dongle: 0x2125, cable: 0x2126 },
    Product { name: "JBL Quantum 650X Wireless", dongle: 0x213B, cable: 0x213D },
    Product { name: "JBL Quantum 950 Wireless", dongle: 0x2127, cable: 0x2128 },
    Product { name: "JBL Quantum 950X Wireless", dongle: 0x213E, cable: 0x2140 },
    Product { name: "JBL Quantum 520 Wireless", dongle: 0x218D, cable: 0x218E },
    Product { name: "JBL Quantum 520X Wireless", dongle: 0x21A7, cable: 0x21A9 },
    Product { name: "JBL Quantum 850 Wireless", dongle: 0x219A, cable: 0x219B },
];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

/// Product owning `pid`, if any.
pub fn product(vid: u16, pid: u16) -> Option<&'static Product> {
    (vid == VID).then(|| PRODUCTS.iter().find(|p| p.dongle == pid || p.cable == pid)).flatten()
}

/// Feature-report request for the battery feature.
fn request() -> Vec<u8> {
    vec![
        0x88, 0x00, 0xDD, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, BATTERY_FEATURE as u8, (BATTERY_FEATURE >> 8) as u8,
    ]
}

/// One feature exchange. Returns the battery info byte, or None when the
/// device does not answer / reports an error / an empty feature value.
fn battery(io: &dyn HidIo) -> Option<BatteryStatus> {
    for size in FEAT_SIZES {
        let req = crate::hid::pad_report(&request(), size);
        if io.send_feature(&req).is_err() {
            continue;
        }
        let info = read_matching(io, TIMEOUT_MS, |r| {
            // Only the reply to "get device info" sent to the device itself.
            if r.len() < 16 || r[0] != 0x86 || r[1] != 0x00 || r[2] != 0xDD || r[3] != 0x01 || r[4] != 0x00 || r[5] != 0x01 {
                return None;
            }
            if r[10] & 0x80 != 0 {
                return Some(u8::MAX); // error status (bit 15): stop waiting
            }
            if r[11] != BATTERY_FEATURE as u8 || r[12] != (BATTERY_FEATURE >> 8) as u8 {
                return None;
            }
            if r[13] == 0 && r[14] == 0 {
                return None; // empty feature value
            }
            Some(r[15])
        })
        .ok()
        .flatten();
        if let Some(info) = info {
            let level = info & 0x7F;
            if level > 100 {
                return None;
            }
            return Some(BatteryStatus {
                level: Some(level),
                charge: if info & 0x80 != 0 { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            });
        }
    }
    None
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Jbl;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Jbl {
        /// Dongle plugged in but headset silent: keep the card as asleep.
        known: Mutex<BTreeMap<&'static str, Device>>,
    }

    impl BatteryProvider for Jbl {
        fn name(&self) -> &'static str {
            "jbl"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate()
                .iter()
                .filter(|d| supported(d.vendor_id(), d.product_id()) && d.usage_page() == 0xFF12)
                .cloned().collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            for p in PRODUCTS {
                // Cable (headset itself) first, then the 2.4 GHz dongle: both
                // speak the same protocol and report the same battery.
                let mut cols: Vec<_> = infos
                    .iter()
                    .filter(|d| d.product_id() == p.cable || d.product_id() == p.dongle)
                    .collect();
                cols.sort_by_key(|d| if d.product_id() == p.cable { 0 } else { 1 });
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = battery(&io);
                    log::debug!("jbl {} {:04x} if{}: {s:?}", p.name, d.product_id(), d.interface_number());
                    s.map(|status| (path, d.product_id(), status))
                });
                match found {
                    Some((path, pid, _)) => {
                        let dev = Device {
                            id: format!("jbl:{}", p.name.to_lowercase().replace(' ', "-")),
                            name: p.name.into(),
                            vendor: "JBL".into(),
                            model: Some(p.name.into()),
                            kind: DeviceKind::Headset,
                            provider: "jbl",
                            locator: format!("{path}\t{VID:04x}\t{pid:04x}"),
                        };
                        known.insert(p.name, dev.clone());
                        out.push(dev);
                    }
                    None => out.extend(known.get(p.name).cloned()),
                }
            }
            known.retain(|name, _| PRODUCTS.iter().any(|p| p.name == *name));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let Some(path) = parts.next() else { return BatteryStatus::GONE };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            battery(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// HIDV3 dongle answering the battery feature (0x000D) with `info`.
    fn dongle(info: u8) -> FakeHid {
        FakeHid::new(move |req| {
            // Feature writes arrive padded; the 11-byte request leads.
            if req.len() < 11 || req[0] != 0x88 || req[2] != 0xDD {
                return vec![];
            }
            // [86][00 DD][01 00][01][00][len][status][feature 000D][size 1][value]
            let mut r = vec![0x86, 0x00, 0xDD, 0x01, 0x00, 0x01, 0x00, 0x09, 0x00, 0x00, 0x00, 0x0D, 0x00, 0x01, 0x00, info];
            r.resize(64, 0);
            vec![r]
        })
    }

    #[test]
    fn battery_connected() {
        // 0x80 | 73 = charging at 73 %
        let s = battery(&dongle(0x80 | 73)).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(73), Charge::Charging, Link::Connected));
        let s = battery(&dongle(42)).unwrap();
        assert_eq!((s.level, s.charge), (Some(42), Charge::Discharging));
    }

    #[test]
    fn invalid_level_rejected() {
        assert_eq!(battery(&dongle(0x7F)), None); // 127 > 100
    }

    #[test]
    fn silence_is_none() {
        assert_eq!(battery(&FakeHid::new(|_| vec![])), None);
    }

    #[test]
    fn supported_tables() {
        assert!(supported(0x0ECB, 0x2125));
        assert!(supported(0x0ECB, 0x219B));
        assert!(!supported(0x0ECB, 0x1234));
        assert!(!supported(0x1234, 0x2125));
        assert_eq!(product(0x0ECB, 0x2126).map(|p| p.name), Some("JBL Quantum 650 Wireless"));
    }
}
