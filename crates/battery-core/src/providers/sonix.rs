//! SONiX keyboards on their 2.4 GHz dongles: AJAZZ AK820 MAX / AULA F108
//! Pro (05AC:024F — the dongle reuses Apple's vendor id) and AJAZZ AK980 V2
//! (0C45:FEFE). The vendor collection is interface 3, page FF60 usage 61,
//! 33-byte reports (id 0 + 32-byte payload):
//!   request [20 01 00 ... sum], reply [20 01 00 level ... sum],
//!   sum = byte sum of the preceding bytes & 0xFF. `level` is percent;
//!   0 means the keyboard is off, out of range or in another mode.
//! Protocol from donglover69/sonix-keyboard-studio; no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const DONGLES: [(u16, u16); 2] = [(0x05AC, 0x024F), (0x0C45, 0xFEFE)];
const LEN: usize = 32; // payload bytes after the report id; last is checksum

pub fn supported(vid: u16, pid: u16) -> bool {
    DONGLES.contains(&(vid, pid))
}

fn request() -> [u8; 33] {
    let mut req = [0u8; 33];
    req[1] = 0x20;
    req[2] = 0x01;
    req[LEN] = req[1..LEN].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    req
}

/// Parse a reply tolerating an optional leading report-id byte.
fn parse(r: &[u8]) -> Option<u8> {
    for off in 0..=1usize {
        if r.len() >= off + LEN && r[off] == 0x20 && r[off + 1] == 0x01 {
            let p = &r[off..off + LEN];
            let sum = p[..LEN - 1].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            if sum == p[LEN - 1] && p[3] <= 100 {
                return Some(p[3]);
            }
        }
    }
    None
}

/// One query with the documented 500 ms deadline; retry once on silence.
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let req = request();
    for _ in 0..2 {
        if let Some(level) = query(io, &req, &[], 500, parse).ok().flatten() {
            return Some(if level == 0 {
                BatteryStatus::ASLEEP // off, out of range or another mode
            } else {
                BatteryStatus {
                    level: Some(level),
                    charge: Charge::Discharging,
                    link: Link::Connected,
                }
            });
        }
    }
    None
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Sonix;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Sonix {
        known: Mutex<BTreeMap<(u16, u16), Device>>,
    }

    impl BatteryProvider for Sonix {
        fn name(&self) -> &'static str {
            "sonix"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let infos: Vec<_> = enumerate().into_iter().filter(|d| supported(d.vendor_id(), d.product_id())).collect();
            let mut ids: Vec<(u16, u16)> = infos.iter().map(|d| (d.vendor_id(), d.product_id())).collect();
            ids.sort_unstable();
            ids.dedup();
            for &(vid, pid) in &ids {
                let product = infos
                    .iter()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
                    .and_then(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let mut name = product.trim().to_string();
                if name.ends_with(" 2.4G") {
                    name.truncate(name.len() - 5);
                }
                if name.is_empty() {
                    name = "Wireless keyboard".into();
                }
                let found = infos
                    .iter()
                    .filter(|d| {
                        d.vendor_id() == vid
                            && d.product_id() == pid
                            && d.interface_number() == 3
                            && d.usage_page() == 0xFF60
                            && d.usage() == 0x0061
                    })
                    .find_map(|d| {
                        let path = d.path().to_string_lossy().into_owned();
                        let io = open(&path)?;
                        let s = poll(&io);
                        log::debug!("sonix {vid:04x}:{pid:04x}: {s:?}");
                        s.map(|status| (path, status))
                    });
                match found {
                    Some((path, _)) => {
                        let dev = Device {
                            id: format!("sonix:{vid:04x}:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Keyboard".into(),
                            model: Some(name),
                            kind: DeviceKind::Keyboard,
                            provider: "sonix",
                            locator: format!("{path}\t{vid:04x}\t{pid:04x}"),
                        };
                        known.insert((vid, pid), dev.clone());
                        out.push(dev);
                    }
                    None => out.extend(known.get(&(vid, pid)).cloned()),
                }
            }
            known.retain(|id, _| ids.contains(id));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn reply(level: u8) -> Vec<u8> {
        let mut r = vec![0u8; LEN];
        r[0] = 0x20;
        r[1] = 0x01;
        r[3] = level;
        r[LEN - 1] = r[..LEN - 1].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        r
    }

    #[test]
    fn level_and_checksum() {
        let io = FakeHid::new(move |_| vec![reply(73)]);
        let s = poll(&io).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(73), Charge::Discharging, Link::Connected));
    }

    #[test]
    fn zero_is_asleep() {
        let io = FakeHid::new(move |_| vec![reply(0)]);
        assert_eq!(poll(&io), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn bad_checksum_rejected() {
        let io = FakeHid::new(|_| {
            let mut r = reply(50);
            r[LEN - 1] ^= 0xFF;
            vec![r]
        });
        assert_eq!(poll(&io), None);
    }
}
