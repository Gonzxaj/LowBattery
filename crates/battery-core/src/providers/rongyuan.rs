//! Rongyuan 2.4 GHz receivers used by Attack Shark, Ajazz and many other
//! mice/keyboards (and some MSI gear). One feature report returns both the
//! keyboard and mouse slots:
//!   request on interface 2, collection FFFF:0002: [00][F7][0 ...]
//!   reply: [00][ok][keyboard %][mouse %][keyboard offline][mouse offline]
//! A slot marked offline is skipped; each receiver can yield two cards.
//! Receiver list and protocol from qu1ncky/BatteryTray (MIT), itself from
//! Attack Shark's official web driver; no code taken.

use crate::hid::{feature_exchange, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind};

/// (vid << 16) | pid of every known Rongyuan receiver.
const RECEIVERS: &[u32] = &[
    0x0DB0197E, 0x0DB08E9A, 0x145F0376, 0x145F038D, 0x14A5C045, 0x25AA2008, 0x25AA4008, 0x25AA4018,
    0x31514011, 0x31514014, 0x31514017, 0x31514021, 0x31514027, 0x3151402D, 0x31514038, 0x31515006,
    0x31515007, 0x31515008, 0x31515020, 0x31515026, 0x3151502B, 0x31515031, 0x31515037, 0x31515038,
    0x3151503D, 0x3151503F, 0x31515040, 0x31515042, 0x31515045, 0x3151504B, 0x3151504E, 0x31515050,
    0x31515052, 0x31515055, 0x3151505A, 0x342DE3E0, 0x342DE3E1, 0x342DE3E2, 0x342DE3E6, 0x342DE3E7,
    0x342DE3E8, 0x342DE3F5, 0x342DE3F6, 0x342DE3F7, 0x342DE3FB, 0x342DE3FC, 0x342DE3FD, 0x347AA302,
    0x347AA305, 0x374AA205, 0x374AA217, 0x374AA220, 0x374AA223, 0x374AA226, 0x374AA229, 0x374AA234,
    0x374AA237, 0x374AA251, 0x374AA263, 0x374AA268, 0x374AA311, 0x374AA314, 0x374AA319, 0x374AA322,
    0x374AA329, 0x374AA355, 0x379A1804, 0x379A1807, 0x38A90002, 0x38EE0000, 0x38EE0002, 0x38EE0039,
    0x391D1A04, 0x391D1A08, 0x391D1A0E, 0x391D1A13, 0x391D1A25, 0x391D1A2B, 0x391D1A34, 0x391D2017,
    0x391D2A05,
];

pub fn supported(vid: u16, pid: u16) -> bool {
    RECEIVERS.binary_search(&(((vid as u32) << 16) | pid as u32)).is_ok()
}

/// Query once; returns the 7+ byte reply (any report-id alignment).
pub fn poll(io: &dyn HidIo) -> Option<Vec<u8>> {
    let mut req = vec![0u8; 16];
    req[0] = 0x00;
    req[1] = 0xF7;
    feature_exchange(io, &req, 3, 20, 16, |r| {
        // Reply payload may sit at offset 0 or 1 (report id byte).
        for off in 0..=1usize {
            if r.len() >= off + 7
                && r[off..off + 5].iter().any(|&b| b != 0)
            {
                return Some(r[off..].to_vec());
            }
        }
        None
    })
}

/// Slots: [2]=kb %, [3]=mouse %, [4]=kb offline, [5]=mouse offline.
pub fn slots(r: &[u8]) -> Vec<(bool, u8)> {
    let mut out = Vec::new();
    for &(keyboard, lvl, off) in &[(true, r[2], r[4]), (false, r[3], r[5])] {
        if off == 0 && (1..=100).contains(&lvl) {
            out.push((keyboard, lvl));
        }
    }
    out
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Rongyuan;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// (vid,pid) -> up to two devices (kb + mouse).
    #[derive(Default)]
    pub struct Rongyuan {
        known: Mutex<BTreeMap<(u16, u16), Vec<Device>>>,
    }

    impl BatteryProvider for Rongyuan {
        fn name(&self) -> &'static str {
            "rongyuan"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut ids: Vec<(u16, u16)> = Vec::new();
            for d in enumerate() {
                if supported(d.vendor_id(), d.product_id()) && !ids.contains(&(d.vendor_id(), d.product_id())) {
                    ids.push((d.vendor_id(), d.product_id()));
                }
            }
            for &(vid, pid) in &ids {
                let found = enumerate()
                    .into_iter()
                    .filter(|d| {
                        d.vendor_id() == vid
                            && d.product_id() == pid
                            && d.interface_number() == 2
                            && d.usage_page() == 0xFFFF
                            && d.usage() == 0x0002
                    })
                    .find_map(|d| {
                        let path = d.path().to_string_lossy().into_owned();
                        let io = open(&path)?;
                        let r = poll(&io)?;
                        Some((path, r))
                    });
                let Some((path, r)) = found else {
                    out.extend(known.get(&(vid, pid)).cloned().unwrap_or_default());
                    continue;
                };
                let product = enumerate()
                    .into_iter()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
                    .and_then(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let generic = product.is_empty() || product.contains("Receiver") || product.contains("2.4G");
                let mut devs = Vec::new();
                for (keyboard, lvl) in slots(&r) {
                    let slot = if keyboard { "kb" } else { "ms" };
                    let kind = if keyboard { DeviceKind::Keyboard } else { DeviceKind::Mouse };
                    let name = if vid == 0x3151 && pid == 0x402D && !keyboard {
                        "Attack Shark R3".to_string()
                    } else if generic {
                        if keyboard { "Wireless keyboard".into() } else { "Wireless mouse".into() }
                    } else {
                        product.clone()
                    };
                    devs.push(Device {
                        id: format!("rongyuan:{vid:04x}:{pid:04x}:{slot}"),
                        name: name.clone(),
                        vendor: "Mouse".into(),
                        model: Some(name),
                        kind,
                        provider: "rongyuan",
                        locator: format!("{path}\t{vid:04x}\t{pid:04x}\t{slot}"),
                    });
                    let _ = lvl;
                }
                out.extend(devs.clone());
                known.insert((vid, pid), devs);
            }
            known.retain(|id, _| ids.contains(id));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            // Re-read the frame and pick this device's slot.
            let slot = device.locator.split('\t').nth(3).unwrap_or("");
            match poll(&io).and_then(|r| slots(&r).into_iter().find(|(kb, _)| (if *kb { "kb" } else { "ms" }) == slot)) {
                Some((_, lvl)) => BatteryStatus::connected(lvl, Charge::Discharging),
                None => BatteryStatus::ASLEEP,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn receiver_table() {
        assert!(supported(0x3151, 0x402D)); // Attack Shark R3
        assert!(supported(0x391D, 0x1A04));
        assert!(!supported(0x1234, 0x5678));
    }

    #[test]
    fn slots_parse() {
        let r = vec![0x00, 0x01, 80, 45, 0, 1, 0];
        let s = slots(&r);
        assert_eq!(s, vec![(true, 80)]);
        let r2 = vec![0x00, 0x01, 0, 0, 1, 0, 0];
        assert!(slots(&r2).is_empty());
    }

    #[test]
    fn poll_rejects_empty() {
        assert_eq!(poll(&FakeHid::new(|_| vec![vec![0u8; 16]])), None);
    }
}
