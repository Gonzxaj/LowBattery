//! LAMZU mice (VIDs 373E and 37B0: Maya, Thorn, Lotus, Ignit, etc.).
//! Feature report 0, 65 bytes:
//!   request bytes [3]=02 [4]=02 [6]=83
//!   reply   [1]=0xA1, [4]=02, [6]=83 -> [7]=charging, [8]=percent
//! Some stacks strip the report-id byte, so the reply is also accepted at
//! the shifted alignment [0]=A1 [3]=02 [5]=83 -> [6]/[7].
//! A dongle reply of 0% means the mouse is switched off or out of range;
//! on the cable it stays connected. hidapi does not expose report lengths,
//! so every collection is tried vendor-page first.
//! Protocol from github.com/EngineersBox/lamzu-linux and the LAMZU config
//! tool; reimplemented here, no code taken.

use crate::hid::{feature_exchange, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VIDS: [u16; 2] = [0x373E, 0x37B0];

pub fn supported(vid: u16, _pid: u16) -> bool {
    VIDS.contains(&vid)
}

fn request() -> [u8; 65] {
    let mut w = [0u8; 65];
    w[3] = 0x02;
    w[4] = 0x02;
    w[6] = 0x83;
    w
}

/// Parse a reply at either alignment. Returns (charging, percent).
fn parse(r: &[u8]) -> Option<(u8, u8)> {
    if r.len() >= 9 && r[1] == 0xA1 && r[4] == 0x02 && r[6] == 0x83 {
        return Some((r[7], r[8]));
    }
    if r.len() >= 8 && r[0] == 0xA1 && r[3] == 0x02 && r[5] == 0x83 {
        return Some((r[6], r[7]));
    }
    None
}

/// One LAMZU exchange; None = not this protocol (try next collection).
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let (charging, pct) = feature_exchange(io, &request(), 1, 100, 65, parse)?;
    if pct > 100 {
        return None;
    }
    Some(if pct == 0 && charging == 0 {
        BatteryStatus::ASLEEP // dongle answered but the mouse is not linked
    } else {
        BatteryStatus {
            level: Some(if pct == 0 { 1 } else { pct }),
            charge: if charging != 0 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Lamzu;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Lamzu {
        known: Mutex<BTreeMap<(u16, u16), Device>>,
    }

    impl BatteryProvider for Lamzu {
        fn name(&self) -> &'static str {
            "lamzu"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> =
                enumerate().iter().filter(|d| supported(d.vendor_id(), d.product_id())).cloned().collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut ids: Vec<(u16, u16)> = infos.iter().map(|d| (d.vendor_id(), d.product_id())).collect();
            ids.sort_unstable();
            ids.dedup();
            for &(vid, pid) in &ids {
                let product = infos
                    .iter()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
                    .and_then(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let mut cols: Vec<_> = infos.iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).collect();
                cols.sort_by_key(|d| (if d.usage_page() >= 0xFF00 { 0 } else { 1 }, d.interface_number()));
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io);
                    log::debug!("lamzu {vid:04x}:{pid:04x} if{} {:04x}/{:04x}: {s:?}", d.interface_number(), d.usage_page(), d.usage());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let name = product.trim().to_string();
                        let name = if name.is_empty() { "LAMZU wireless mouse".to_string() } else { name };
                        let dev = Device {
                            id: format!("lamzu:{vid:04x}:{pid:04x}"),
                            name: name.clone(),
                            vendor: "LAMZU".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "lamzu",
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

    fn reply(charging: u8, pct: u8) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        r[0] = 0x00;
        r[1] = 0xA1;
        r[4] = 0x02;
        r[6] = 0x83;
        r[7] = charging;
        r[8] = pct;
        r
    }

    #[test]
    fn awake_level() {
        let s = poll(&FakeHid::new(move |_| vec![reply(1, 88)])).unwrap();
        assert_eq!((s.level, s.charge), (Some(88), Charge::Charging));
    }

    #[test]
    fn dongle_zero_is_asleep() {
        let s = poll(&FakeHid::new(move |_| vec![reply(0, 0)])).unwrap();
        assert_eq!(s, BatteryStatus::ASLEEP);
    }

    #[test]
    fn shifted_alignment() {
        // Stack without report-id byte.
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 64];
            r[0] = 0xA1;
            r[3] = 0x02;
            r[5] = 0x83;
            r[6] = 0;
            r[7] = 42;
            vec![r]
        });
        let s = poll(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(42), Charge::Discharging));
    }

    #[test]
    fn silence_is_none() {
        assert_eq!(poll(&FakeHid::new(|_| vec![])), None);
    }
}
