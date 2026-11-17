//! Rapoo VT gaming mice (VID 24AE). Battery is pushed periodically — no
//! commands needed. Collection FF00:0002:
//!   Nordic 54L (19-byte reports): [07][mode+conn][..][state@7][percent@8]
//!     [.. mouse-pid@13..14]; conn nibble: 0 RF, 1 BT, 2 cable.
//!   Legacy VT3 Pro / VT1 Pro Max (7-byte reports on pids 1215/4415/1233/
//!   4433): [BB][B0][..][..][state@5][percent@6]; state 1=discharging,
//!   2=charging, percent 7F while charging on old firmware.
//! Sources: Rapoo Hub decoder/manifests, PyFlat/Device-Battery-Info and
//! issue #136 captures; no code taken.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x24AE;

/// (cable pid, receiver pid, name)
const MODELS: [(u16, u16, &str); 17] = [
    (0x4610, 0x1410, "Rapoo VT0"),
    (0x4620, 0x1420, "Rapoo VT0 Max"),
    (0x4615, 0x1415, "Rapoo VT1"),
    (0x4644, 0x1444, "Rapoo VT1 Max"),
    (0x4622, 0x1422, "Rapoo VT2"),
    (0x4623, 0x1423, "Rapoo VT2 Max"),
    (0x4612, 0x1412, "Rapoo VT3"),
    (0x4617, 0x1417, "Rapoo VT3 Max (Gen 2)"),
    (0x4B06, 0x1B06, "Rapoo VT3 Max V2"),
    (0x4611, 0x1411, "Rapoo VT3S"),
    (0x4621, 0x1421, "Rapoo VT3S Max"),
    (0x4613, 0x1413, "Rapoo VT7"),
    (0x4616, 0x1416, "Rapoo VT7 Max"),
    (0x4614, 0x1414, "Rapoo VT9"),
    (0x4618, 0x1418, "Rapoo VT9 Max"),
    (0x4415, 0x1215, "Rapoo VT3 Pro"),
    (0x4433, 0x1233, "Rapoo VT1 Pro Max"),
];

const LEGACY_PIDS: [u16; 4] = [0x1215, 0x4415, 0x1233, 0x4433];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && (MODELS.iter().any(|m| m.0 == pid || m.1 == pid) || LEGACY_PIDS.contains(&pid))
}

/// Passive read: status frames arrive every few seconds, so the deadline
/// is generous and unrelated reports are skipped.
pub fn poll(io: &dyn HidIo, legacy: bool) -> Option<BatteryStatus> {
    read_matching(io, 5000, |r| {
        let (state, pct) = if !legacy && r.len() >= 9 && r[0] == 0x07 && (r[1] & 0x0F) <= 2 {
            (r[7], r[8])
        } else if legacy && r.len() >= 7 && r[0] == 0xBB && r[1] == 0xB0 {
            (r[5], r[6])
        } else {
            return None;
        };
        if state != 1 && state != 2 {
            return None;
        }
        let pct_ok = pct <= 100 || (legacy && state == 2 && pct == 0x7F);
        if !pct_ok {
            return None;
        }
        Some(BatteryStatus {
            level: (pct <= 100).then_some(pct),
            charge: if state == 2 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        })
    })
    .ok()
    .flatten()
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Rapoo;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Rapoo {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    fn model(pid: u16) -> Option<&'static str> {
        MODELS.iter().find(|m| m.0 == pid || m.1 == pid).map(|m| m.2)
    }

    impl BatteryProvider for Rapoo {
        fn name(&self) -> &'static str {
            "rapoo"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in enumerate() {
                if supported(d.vendor_id(), d.product_id()) && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                let legacy = LEGACY_PIDS.contains(&pid);
                let found = enumerate()
                    .into_iter()
                    .filter(|d| {
                        d.vendor_id() == VID
                            && d.product_id() == pid
                            && d.usage_page() == 0xFF00
                            && d.usage() == 0x0002
                    })
                    .find_map(|d| {
                        let path = d.path().to_string_lossy().into_owned();
                        let io = open(&path)?;
                        let s = poll(&io, legacy);
                        log::debug!("rapoo {pid:04x}: {s:?}");
                        s.map(|status| (path, status))
                    });
                match found {
                    Some((path, _)) => {
                        let product = enumerate()
                            .into_iter()
                            .find(|d| d.vendor_id() == VID && d.product_id() == pid)
                            .and_then(|d| d.product_string().map(str::to_string))
                            .unwrap_or_default();
                        let name = model(pid)
                            .map(str::to_string)
                            .unwrap_or_else(|| if product.trim().is_empty() { "Rapoo wireless mouse".into() } else { product });
                        let dev = Device {
                            id: format!("rapoo:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Rapoo".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "rapoo",
                            locator: format!("{path}\t{VID:04x}\t{pid:04x}"),
                        };
                        known.insert(pid, dev.clone());
                        out.push(dev);
                    }
                    None => out.extend(known.get(&pid).cloned()),
                }
            }
            known.retain(|pid, _| pids.contains(pid));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let pid = device.locator.split('\t').nth(2).and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll(&io, LEGACY_PIDS.contains(&pid)).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn nordic_frame() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 19];
            r[0] = 0x07;
            r[1] = 0x00; // RF connection
            r[7] = 2; // charging
            r[8] = 95;
            vec![r]
        });
        io.write(&[0]).unwrap();
        let s = poll(&io, false).unwrap();
        assert_eq!((s.level, s.charge), (Some(95), Charge::Charging));
    }

    #[test]
    fn legacy_frame() {
        let io = FakeHid::new(|_| vec![vec![0xBB, 0xB0, 0, 0, 0, 1, 80]]);
        io.write(&[0]).unwrap();
        let s = poll(&io, true).unwrap();
        assert_eq!((s.level, s.charge), (Some(80), Charge::Discharging));
    }

    #[test]
    fn invalid_state_rejected() {
        let io = FakeHid::new(|_| vec![vec![0x07, 0, 0, 0, 0, 0, 0, 9, 50]]);
        io.write(&[0]).unwrap();
        assert_eq!(poll(&io, false), None);
    }
}
