//! Royal Kludge MG6 Pro (VID 2E3C, "QiWang" protocol): dongles 00A6/00E6 on
//! vendor page FF00:0001, cable 00A5/00E5 on FFF2:0001, 65-byte reports.
//!   output report 0: [00 01 01 02 01] (SYS_CTRL_GET_BATTER_VOL, READ)
//!   reply: same first three bytes; [3] bit 4 = access_res (mouse answered),
//!   [4..5] mV, [6..7] percent, [8..9] temperature, [10] USB plugged in.
//! The receiver pushes unrelated notifications, so replies are matched
//! strictly; access_res 0 means the mouse did not answer -> asleep.
//! Protocol from RK's web driver (drive.rkgaming.com); no code taken.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x2E3C;

pub fn product(pid: u16) -> Option<(&'static str, bool)> {
    Some(match pid {
        0x00A5 | 0x00E5 => ("RK MG6 Pro", true),
        0x00A6 | 0x00E6 => ("RK MG6 Pro", false),
        _ => return None,
    })
}

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && product(pid).is_some()
}

const REQUEST: [u8; 5] = [0x00, 0x01, 0x01, 0x02, 0x01];

/// Returns Some(status) when the receiver answered; the mouse bit decides
/// connected vs asleep.
pub fn poll(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        io.write(&REQUEST).ok()?;
        let s = read_matching(io, 600, |r| {
            if r.len() < 11 || (r[0] & 0x1F) != 0x01 || (r[1] & 0x0F) != 0x01 || r[2] != 0x02 {
                return None;
            }
            if (r[3] >> 4) & 1 == 0 {
                return Some(BatteryStatus::ASLEEP); // receiver ok, mouse silent
            }
            let p = u16::from_le_bytes([r[6], r[7]]);
            if p > 100 {
                return None;
            }
            Some(BatteryStatus {
                level: Some(p as u8),
                charge: if r[10] != 0 || cable { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            })
        });
        if let Some(s) = s.ok().flatten() {
            return Some(s);
        }
    }
    None
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::RoyalKludge;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct RoyalKludge {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for RoyalKludge {
        fn name(&self) -> &'static str {
            "royal-kludge"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in enumerate().iter() {
                if supported(d.vendor_id(), d.product_id()) && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                let Some((_, cable)) = product(pid) else { continue };
                let page = if cable { 0xFFF2 } else { 0xFF00 };
                let found = enumerate()
                    .iter()
                    .filter(|d| d.vendor_id() == VID && d.product_id() == pid && d.usage_page() == page && d.usage() == 1)
                    .find_map(|d| {
                        let path = d.path().to_string_lossy().into_owned();
                        let io = open(&path)?;
                        let s = poll(&io, cable);
                        log::debug!("rk {pid:04x}: {s:?}");
                        s.map(|status| (path, status))
                    });
                match found {
                    Some((path, _)) => {
                        let dev = Device {
                            id: format!("rk:{pid:04x}"),
                            name: "RK MG6 Pro".into(),
                            vendor: "Royal Kludge".into(),
                            model: Some("RK MG6 Pro".into()),
                            kind: DeviceKind::Mouse,
                            provider: "royal-kludge",
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
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            let pid = device.locator.split('\t').nth(2).and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
            let cable = product(pid).is_some_and(|p| p.1);
            poll(&io, cable).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn reply(access: u8, pct: u16, plugged: u8) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        r[0] = 0x01;
        r[1] = 0x01;
        r[2] = 0x02;
        r[3] = access << 4;
        r[4..6].copy_from_slice(&3700u16.to_le_bytes());
        r[6..8].copy_from_slice(&pct.to_le_bytes());
        r[10] = plugged;
        r
    }

    #[test]
    fn level_and_plugged() {
        let io = FakeHid::new(move |_| vec![reply(1, 87, 1)]);
        let s = poll(&io, false).unwrap();
        assert_eq!((s.level, s.charge), (Some(87), Charge::Charging));
    }

    #[test]
    fn no_access_is_asleep() {
        let io = FakeHid::new(move |_| vec![reply(0, 0, 0)]);
        assert_eq!(poll(&io, false), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn unrelated_reports_skipped() {
        let io = FakeHid::new(|_| {
            let mut noise = vec![0u8; 65];
            noise[0] = 0x09;
            noise[1] = 0x09;
            vec![noise, reply(1, 55, 0)]
        });
        let s = poll(&io, false).unwrap();
        assert_eq!(s.level, Some(55));
    }
}
