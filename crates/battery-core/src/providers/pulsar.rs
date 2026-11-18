//! Pulsar X2 V3 / X2 V3 eS and other Sonix-based Pulsar mice on vendor id
//! 3710 (X2 V3 cable 3402, X2 V3 eS cable 3406, 1K dongle 5402, 8K dongle
//! 5403; the dongles are shared with other Pulsar mice). These do not speak
//! the 17-byte report 08 protocol of kysona.rs.
//!   Feature report 0, 64-byte packet: [00][category][register][sub]
//!   [00 ...][sum lo][sum hi], sum = 16-bit sum of packet bytes 0..61.
//!   Battery read = 08 81 01. SET_FEATURE, then GET_FEATURE every ~50 ms
//!   until the reply echoes the command:
//!   [status][08][81][01][..][..][percent] (packet byte 6, FF = no reading).
//!   Status 01 = answered, 02 = pending, 05 = mouse not linked (kept polling
//!   on it like upstream; it only means "off" when it is the last answer).
//!   No reliable charging register: on the cable the mouse is charging;
//!   0% from a dongle means the mouse is off or out of range.
//! Protocol from github.com/darthsoup/PulsarBattery and
//! github.com/bluesceada/pulsar-mouse-linux; reimplemented here, no code.

use crate::hid::HidIo;
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x3710;

pub struct Product {
    pub pid: u16,
    pub cable: bool,
    pub name: Option<&'static str>,
}

pub const PRODUCTS: &[Product] = &[
    Product { pid: 0x3402, cable: true, name: Some("Pulsar X2 V3") },
    Product { pid: 0x3406, cable: true, name: Some("Pulsar X2 V3 eS") },
    Product { pid: 0x5402, cable: false, name: None }, // 1K dongle
    Product { pid: 0x5403, cable: false, name: None }, // 8K dongle
];

pub fn product(pid: u16) -> Option<&'static Product> {
    PRODUCTS.iter().find(|p| p.pid == pid)
}

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && product(pid).is_some()
}

/// 64-byte packet (report id 0 first) for register 08 81 01.
fn request() -> [u8; 65] {
    let mut req = [0u8; 65];
    req[2] = 0x08;
    req[3] = 0x81;
    req[4] = 0x01;
    let sum: u16 = req[1..63].iter().map(|&b| b as u16).sum();
    req[63] = (sum & 0xFF) as u8;
    req[64] = (sum >> 8) as u8;
    req
}

/// Parse a feature reply at either alignment (report id present or not).
/// Returns (status, percent).
fn parse(r: &[u8]) -> Option<(u8, u8)> {
    for off in [1usize, 0] {
        if r.len() >= off + 8 && r[off + 1] == 0x08 && r[off + 2] == 0x81 && r[off + 3] == 0x01 {
            return Some((r[off], r[off + 6]));
        }
    }
    None
}

enum Reply {
    Battery(u8),
    Offline,
    None,
}

fn read_battery(io: &dyn HidIo) -> Reply {
    let req = request();
    let mut saw_offline = false;
    for _ in 0..3 {
        if io.send_feature(&req).is_err() {
            return Reply::None;
        }
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut buf = [0u8; 65];
            buf[0] = 0;
            let Ok(n) = io.get_feature(&mut buf) else { return Reply::None };
            if n < 8 {
                continue;
            }
            let Some((status, pct)) = parse(&buf[..n]) else { continue };
            match status {
                0x01 => {
                    if pct > 100 {
                        return Reply::None; // FF = no reading
                    }
                    return Reply::Battery(pct);
                }
                0x05 => saw_offline = true, // keep polling: 05 can also mean pending
                _ => {}
            }
        }
    }
    if saw_offline { Reply::Offline } else { Reply::None }
}

/// `cable` = the mouse itself on USB (always charging).
pub fn poll(io: &dyn HidIo, prod: &Product) -> Option<BatteryStatus> {
    match read_battery(io) {
        Reply::Battery(pct) if prod.cable || pct > 0 => Some(BatteryStatus {
            level: Some(pct),
            charge: if prod.cable { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }),
        Reply::Battery(_) | Reply::Offline => Some(BatteryStatus::ASLEEP),
        Reply::None => None,
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Pulsar;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Pulsar {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for Pulsar {
        fn name(&self) -> &'static str {
            "pulsar"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> =
                enumerate().iter().filter(|d| supported(d.vendor_id(), d.product_id())).cloned().collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in &pids {
                let Some(prod) = product(*pid) else { continue };
                // The vendor collection (FFFF:0001) sits on higher
                // interfaces; interface 2 rejects SET_FEATURE.
                let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == *pid).collect();
                cols.sort_by_key(|d| (if d.usage_page() >= 0xFF00 { 0 } else { 1 }, std::cmp::Reverse(d.interface_number())));
                let product_str = cols.iter().find_map(|d| d.product_string().map(str::to_string)).unwrap_or_default();
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io, prod);
                    log::debug!("pulsar {pid:04x} if{}: {s:?}", d.interface_number());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let generic = product_str.is_empty() || product_str.contains("Dongle") || product_str.contains("Receiver");
                        let name = prod
                            .name
                            .map(str::to_string)
                            .unwrap_or_else(|| if generic { "Pulsar wireless mouse".into() } else { product_str.clone() });
                        let dev = Device {
                            id: format!("pulsar:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Pulsar".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "pulsar",
                            locator: format!("{path}\t{VID:04x}\t{pid:04x}"),
                        };
                        known.insert(*pid, dev.clone());
                        out.push(dev);
                    }
                    None => out.extend(known.get(pid).cloned()),
                }
            }
            known.retain(|pid, _| pids.contains(pid));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let Some(path) = parts.next() else { return BatteryStatus::GONE };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            let pid = parts.nth(1).and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
            match product(pid) {
                Some(prod) => poll(&io, prod).unwrap_or(BatteryStatus::ASLEEP),
                None => BatteryStatus::GONE,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// Pulsar dongle answering the battery register.
    fn dongle(status: u8, pct: u8) -> FakeHid {
        FakeHid::new(move |req| {
            // req[0] = report id 0; packet at req[1..]: [2]=08,[3]=81,[4]=01
            if req.len() < 5 || req[2] != 0x08 || req[3] != 0x81 || req[4] != 0x01 {
                return vec![];
            }
            let mut r = vec![0u8; 65];
            r[0] = 0x00;
            r[1] = status;
            r[2] = 0x08;
            r[3] = 0x81;
            r[4] = 0x01;
            r[7] = pct;
            vec![r]
        })
    }

    #[test]
    fn battery_level() {
        let s = poll(&dongle(0x01, 77), &PRODUCTS[2]).unwrap();
        assert_eq!((s.level, s.link), (Some(77), Link::Connected));
        assert_eq!(s.charge, Charge::Discharging);
    }

    #[test]
    fn cable_is_charging() {
        let s = poll(&dongle(0x01, 40), &PRODUCTS[0]).unwrap();
        assert_eq!(s.charge, Charge::Charging);
    }

    #[test]
    fn offline_is_asleep() {
        // status 05 kept polling then gave up -> offline
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 65];
            r[1] = 0x05;
            r[2] = 0x08;
            r[3] = 0x81;
            r[4] = 0x01;
            vec![r]
        });
        assert_eq!(poll(&io, &PRODUCTS[2]), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn supported_tables() {
        assert!(supported(0x3710, 0x5402));
        assert!(!supported(0x3710, 0x1234));
        assert!(!supported(0x1234, 0x5402));
    }
}
