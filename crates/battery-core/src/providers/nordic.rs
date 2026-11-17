//! Ninjutso Sora V2 (1915:AE1C) and Sprime PM1 (1915:AC11/AC12/AC13 cable,
//! AC1C/AD8C receivers) share one status protocol on collection FFA0:0001:
//!   SET_FEATURE report 05: [05][15][00][00][01]... (32 bytes)
//!   GET_FEATURE report 05: [05][15] ... [9]=percent (0 = 100 on Ninjutso),
//!   [10]=charging, [11]=fully charged (Sprime), [12]=mouse online.
//! Protocol from NinjaForce's web configurator and sprime.pro's web driver;
//! reimplemented here, no code taken.

use crate::hid::{feature_exchange, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x1915;

pub fn product(pid: u16) -> Option<(&'static str, bool)> {
    // (name, is_cable)
    Some(match pid {
        0xAE1C => ("Ninjutso Sora V2", false),
        0xAC11..=0xAC13 => ("Sprime PM1", true),
        0xAC1C | 0xAD8C => ("Sprime PM1", false),
        _ => return None,
    })
}

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && product(pid).is_some()
}

fn request() -> [u8; 32] {
    let mut req = [0u8; 32];
    req[0] = 0x05;
    req[1] = 0x15;
    req[4] = 0x01;
    req
}

/// Parse at either alignment (report id present or not).
/// Returns (pct raw, charging, full, online).
fn parse(r: &[u8]) -> Option<(u8, u8, u8, u8)> {
    for off in 0..=1usize {
        if r.len() >= off + 13 && r[off] == 0x05 && r[off + 1] == 0x15 {
            return Some((r[off + 9], r[off + 10], r[off + 11], r[off + 12]));
        }
    }
    None
}

/// `cable` forces connected even when the online byte is 0.
pub fn poll(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    let (pct, charging, full, online) = feature_exchange(io, &request(), 4, 15, 32, |r| {
        parse(r).filter(|&(p, _, _, _)| p <= 100)
    })?;
    let connected = cable || online != 0;
    Some(if !connected {
        BatteryStatus::ASLEEP
    } else if full != 0 {
        BatteryStatus::connected(100, if charging != 0 { Charge::Charging } else { Charge::Full })
    } else {
        let level = if pct == 0 { 100 } else { pct }; // Ninjutso: 0 means 100%
        BatteryStatus {
            level: Some(level),
            charge: if charging != 0 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Nordic;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Nordic {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for Nordic {
        fn name(&self) -> &'static str {
            "nordic"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate().into_iter().filter(|d| supported(d.vendor_id(), d.product_id())).collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            pids.sort_unstable();
            pids.dedup();
            for &pid in &pids {
                let Some((name, cable)) = product(pid) else { continue };
                let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == pid && d.usage_page() == 0xFFA0 && d.usage() == 1).collect();
                cols.sort_by_key(|d| d.interface_number());
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io, cable);
                    log::debug!("nordic {pid:04x}: {s:?}");
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let dev = Device {
                            id: format!("nordic:{pid:04x}"),
                            name: name.into(),
                            vendor: "Mouse".into(),
                            model: Some(name.into()),
                            kind: DeviceKind::Mouse,
                            provider: "nordic",
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

    fn reply(pct: u8, chg: u8, full: u8, online: u8) -> Vec<Vec<u8>> {
        let mut r = vec![0u8; 32];
        r[0] = 0x05;
        r[1] = 0x15;
        r[9] = pct;
        r[10] = chg;
        r[11] = full;
        r[12] = online;
        vec![r]
    }

    #[test]
    fn ninjutso_level() {
        let s = poll(&FakeHid::new(move |_| reply(64, 0, 0, 1)), false).unwrap();
        assert_eq!((s.level, s.charge), (Some(64), Charge::Discharging));
        // 0% on Ninjutso means 100%.
        let s = poll(&FakeHid::new(move |_| reply(0, 0, 0, 1)), false).unwrap();
        assert_eq!(s.level, Some(100));
    }

    #[test]
    fn receiver_mouse_off_is_asleep() {
        assert_eq!(poll(&FakeHid::new(move |_| reply(0, 0, 0, 0)), false), Some(BatteryStatus::ASLEEP));
        // Cable stays connected even with online=0.
        let s = poll(&FakeHid::new(move |_| reply(30, 1, 0, 0)), true).unwrap();
        assert_eq!(s.charge, Charge::Charging);
    }

    #[test]
    fn full_flag_wins() {
        let s = poll(&FakeHid::new(move |_| reply(40, 0, 1, 1)), false).unwrap();
        assert_eq!(s.charge, Charge::Full);
        assert_eq!(s.level, Some(100));
    }
}
