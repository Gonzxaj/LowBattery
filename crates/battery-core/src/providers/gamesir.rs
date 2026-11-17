//! GameSir controllers. Two vendor protocols on VID 3537:
//!   Nova Lite / Nova 2 / Nova 2 Lite / Nova Lite 2 (pids 1098 receiver,
//!   100F cable, 1040 receiver): collection FF7A, write [00 01 01], reply
//!   [0]=01 ... [18]=battery (0..100, FF = charging on cable).
//!   Cyclone 2 (pid 100B): collection FFF0:0040, write [0F 03] (report 0F),
//!   reply input report 12: [35]=cable flag, [36]=percent.
//! Protocols from GamesirBatteryTray, Gamesir-Nova-2-Lite-Battery-Detection
//! and vdemonchy/cyclone2-linux; no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x3537;
pub const NOVA_PIDS: [u16; 3] = [0x1098, 0x100F, 0x1040];
pub const CYCLONE2_PID: u16 = 0x100B;

pub fn nova_supported(vid: u16, pid: u16) -> bool {
    vid == VID && NOVA_PIDS.contains(&pid)
}
pub fn cyclone_supported(vid: u16, pid: u16) -> bool {
    vid == VID && pid == CYCLONE2_PID
}

pub fn poll_nova(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    query(io, &[0x00, 0x01, 0x01], &[8], 800, |r| {
        // Skip unsolicited F4 00 state frames and anything not starting 01.
        if r.len() < 19 || r[0] != 0x01 {
            return None;
        }
        let b = r[18];
        if b == 0xFF {
            Some(BatteryStatus {
                level: None, // charging: keep last value
                charge: Charge::Charging,
                link: Link::Connected,
            })
        } else if b <= 100 {
            let _ = cable;
            Some(BatteryStatus {
                level: Some(b),
                charge: Charge::Discharging,
                link: Link::Connected,
            })
        } else {
            None
        }
    })
    .ok()
    .flatten()
}

pub fn poll_cyclone2(io: &dyn HidIo) -> Option<BatteryStatus> {
    query(io, &[0x0F, 0x03], &[13, 8], 1500, |r| {
        // Report id 0x12 at [0]; fields at absolute [35]/[36]. Tolerate a
        // stack that strips the id (fields shift to [34]/[35]).
        let b = if r.first() == Some(&0x12) { &r[1..] } else { r };
        if b.len() >= 36 && b[34] <= 1 && b[35] <= 100 {
            return Some(BatteryStatus {
                level: Some(b[35]),
                charge: if b[34] == 1 { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            });
        }
        None
    })
    .ok()
    .flatten()
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::GameSir;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct GameSir {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for GameSir {
        fn name(&self) -> &'static str {
            "gamesir"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in enumerate() {
                if d.vendor_id() == VID && (NOVA_PIDS.contains(&d.product_id()) || d.product_id() == CYCLONE2_PID) && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                let is_nova = NOVA_PIDS.contains(&pid);
                let (page, usage) = if is_nova { (0xFF7A, 0) } else { (0xFFF0, 0x0040) };
                let found = enumerate()
                    .into_iter()
                    .filter(|d| d.vendor_id() == VID && d.product_id() == pid && d.usage_page() == page)
                    .find_map(|d| {
                        if !is_nova && d.usage() != usage {
                            return None;
                        }
                        let path = d.path().to_string_lossy().into_owned();
                        let io = open(&path)?;
                        let s = if is_nova { poll_nova(&io, pid == 0x100F) } else { poll_cyclone2(&io) };
                        log::debug!("gamesir {pid:04x}: {s:?}");
                        s.map(|status| (path, status))
                    });
                match found {
                    Some((path, _)) => {
                        let name = if is_nova { "GameSir Nova" } else { "GameSir Cyclone 2" };
                        let dev = Device {
                            id: format!("gamesir:{pid:04x}"),
                            name: name.into(),
                            vendor: "GameSir".into(),
                            model: Some(name.into()),
                            kind: DeviceKind::Controller,
                            provider: "gamesir",
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
            let s = if NOVA_PIDS.contains(&pid) { poll_nova(&io, pid == 0x100F) } else { poll_cyclone2(&io) };
            s.unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn nova_level() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 19];
            r[0] = 0x01;
            r[18] = 73;
            vec![r]
        });
        let s = poll_nova(&io, false).unwrap();
        assert_eq!((s.level, s.charge), (Some(73), Charge::Discharging));
    }

    #[test]
    fn nova_ff_is_charging() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 19];
            r[0] = 0x01;
            r[18] = 0xFF;
            vec![r]
        });
        let s = poll_nova(&io, true).unwrap();
        assert_eq!((s.level, s.charge), (None, Charge::Charging));
    }

    #[test]
    fn cyclone2_fields() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 37];
            r[0] = 0x12;
            r[35] = 1;
            r[36] = 64;
            vec![r]
        });
        let s = poll_cyclone2(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(64), Charge::Charging));
    }
}
