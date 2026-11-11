//! Keyboards with the vendor battery channel FF1C:0092 (64-byte report 04),
//! whatever the brand: a common Telink / Evision firmware family sold as
//! FEKER, VGN, Rainy and others, over its 2.4G receiver or USB cable.
//! Matched by that collection, not by VID, so new brands work without a
//! table; a device that doesn't speak the protocol never gets a card.
//! Battery query per the VGN HUB web driver as documented by MyFinder Device
//! Battery (AGPL; no code taken); charge byte found and both links checked on
//! a real FEKER IK65 (receiver 320F:5088, cable 320F:5055).

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge};

pub const USAGE_PAGE: u16 = 0xFF1C;
pub const USAGE: u16 = 0x0092;
const REPORT: u8 = 0x04;
const CMD: [u8; 2] = [0x1A, 0x06];
/// Byte 3 when the receiver answers for a keyboard that is asleep / off.
const ASLEEP: u8 = 0xFF;

/// Reply: [04, 00, 00, 1A|FF, 06, 00, 00, 00, level, charge, ...].
/// None = no valid answer (not this protocol, or I/O failure).
pub fn query(io: &dyn HidIo) -> Option<BatteryStatus> {
    let mut req = [0u8; 64];
    req[0] = REPORT;
    req[3..5].copy_from_slice(&CMD);
    io.write(&req).ok()?;
    // Other reports on this interface (keys, media) are skipped by read_matching.
    read_matching(io, 500, |r| {
        if r.len() < 10 || r[0] != REPORT || r[4] != CMD[1] {
            return None;
        }
        match r[3] {
            ASLEEP => Some(BatteryStatus::ASLEEP),
            0x1A if r[8] <= 100 => {
                // Seen on a FEKER IK65: 0 on 2.4G, 1 charging by cable, 2 at 100 % on cable.
                let charge = match r[9] {
                    0 => Charge::Discharging,
                    1 => Charge::Charging,
                    2 => Charge::Full,
                    _ => Charge::Unknown,
                };
                Some(BatteryStatus::connected(r[8], charge))
            }
            _ => None,
        }
    })
    .ok()
    .flatten()
}

/// Of every link that answered, the one to show: a live reading beats an asleep receiver.
pub fn best<T>(answers: Vec<(T, BatteryStatus)>) -> Option<(T, BatteryStatus)> {
    answers.into_iter().max_by_key(|(_, s)| s.level.is_some())
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Ff1cKeyboard;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device, DeviceKind};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Generic product string of the 2.4G receivers; the cable link names the real model.
    const RECEIVER_NAME: &str = "Wireless Gaming Keyboard";

    /// One card per VID, whichever link the keyboard uses: cable and receiver
    /// have different PIDs, and the receiver keeps answering "asleep" while
    /// the board is on the cable.
    /// shortcut: two keyboards with the same VID at once would share a card;
    /// key by serial/model if anyone reports owning two.
    #[derive(Default)]
    pub struct Ff1cKeyboard {
        /// VID -> model name learned from the cable link, kept for the receiver.
        models: Mutex<BTreeMap<u16, String>>,
        /// VID -> card seen this run: a receiver that stops answering
        /// (keyboard off or deep asleep) keeps it, shown as asleep.
        known: Mutex<BTreeMap<u16, Device>>,
    }

    struct Found {
        path: String,
        product: String,
        vendor: String,
    }

    /// Everything seen for one VID in a discovery pass.
    #[derive(Default)]
    struct Links {
        answered: Vec<(Found, BatteryStatus)>,
        paths: Vec<String>,
    }

    impl BatteryProvider for Ff1cKeyboard {
        fn name(&self) -> &'static str {
            "ff1c-keyboard"
        }

        fn discover(&self) -> Vec<Device> {
            let mut by_vid: BTreeMap<u16, Links> = BTreeMap::new();
            let mut models = self.models.lock().unwrap();
            for d in enumerate() {
                if (d.usage_page(), d.usage()) != (USAGE_PAGE, USAGE) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let answer = open(&path).and_then(|io| query(&io));
                log::debug!("ff1c {:04x}:{:04x} {path}: {answer:?}", d.vendor_id(), d.product_id());
                let product = d.product_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(RECEIVER_NAME).to_string();
                if product != RECEIVER_NAME {
                    models.insert(d.vendor_id(), product.clone());
                }
                let entry = by_vid.entry(d.vendor_id()).or_default();
                entry.paths.push(path.clone());
                if let Some(status) = answer {
                    let vendor = d.manufacturer_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("HID").to_string();
                    entry.answered.push((Found { path, product, vendor }, status));
                }
            }
            let mut known = self.known.lock().unwrap();
            known.retain(|vid, _| by_vid.contains_key(vid)); // unplugged: forget
            let mut out = Vec::new();
            for (vid, Links { answered, mut paths }) in by_vid {
                let Some((link, _)) = best(answered) else {
                    out.extend(known.get(&vid).cloned());
                    continue;
                };
                let name = models.get(&vid).cloned().unwrap_or(link.product);
                // Every path of the VID rides along so the unsupported-device report skips all links.
                paths.retain(|p| *p != link.path);
                paths.insert(0, link.path);
                let device = Device {
                    id: format!("ff1c:{vid:04x}"),
                    kind: match crate::guess_kind(&name) {
                        DeviceKind::Other => DeviceKind::Keyboard, // FF1C:0092 is a keyboard protocol
                        k => k,
                    },
                    model: models.get(&vid).cloned(),
                    name,
                    vendor: link.vendor,
                    provider: "ff1c-keyboard",
                    locator: paths.join("\t"),
                };
                known.insert(vid, device.clone());
                out.push(device);
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or_default();
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            query(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn board(byte3: u8, level: u8, charge: u8) -> FakeHid {
        FakeHid::new(move |out| {
            assert_eq!((out.len(), &out[..5]), (64, &[0x04, 0, 0, 0x1A, 0x06][..]));
            let key = vec![0x01, 0, 0x04, 0, 0, 0, 0, 0, 0]; // a keypress report in between
            let mut r = vec![0u8; 64];
            r[..5].copy_from_slice(&[0x04, 0, 0, byte3, 0x06]);
            r[8] = level;
            r[9] = charge;
            vec![key, r]
        })
    }

    #[test]
    fn reads_level_charge_and_sleep() {
        // Real FEKER IK65 frames: 2.4G 4f 00, cable 63 01, cable 64 02.
        assert_eq!(query(&board(0x1A, 0x4F, 0)), Some(BatteryStatus::connected(79, Charge::Discharging)));
        assert_eq!(query(&board(0x1A, 0x63, 1)), Some(BatteryStatus::connected(99, Charge::Charging)));
        assert_eq!(query(&board(0x1A, 0x64, 2)), Some(BatteryStatus::connected(100, Charge::Full)));
        assert_eq!(query(&board(0xFF, 0, 0)), Some(BatteryStatus::ASLEEP));
        assert_eq!(query(&board(0x1A, 200, 0)), None);
        assert_eq!(query(&board(0x20, 50, 0)), None);
    }

    #[test]
    fn live_link_beats_asleep_receiver() {
        let live = BatteryStatus::connected(99, Charge::Charging);
        assert_eq!(best(vec![("dongle", BatteryStatus::ASLEEP), ("cable", live)]), Some(("cable", live)));
        assert_eq!(best(vec![("dongle", BatteryStatus::ASLEEP)]), Some(("dongle", BatteryStatus::ASLEEP)));
        assert_eq!(best::<&str>(vec![]), None);
    }
}
