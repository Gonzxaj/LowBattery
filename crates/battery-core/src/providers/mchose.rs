//! MCHOSE mice. Two unrelated protocols share the brand:
//!   1) VID 5253 (AULA/MCHOSE): vendor collection FF01:0001, feature report
//!      0x12, 65-byte reports where every byte is bitwise-inverted (the
//!      report is sent and received inverted). Request is all 0xFF except
//!      [0]=0x12, [1]=0xF9 (= 0x06 ^ 0xFF). After inversion the reply
//!      payload decodes to VID(2), model(2), firmware(4), flags(1),
//!      battery(1), charging(1). All-zero decoded = mouse asleep.
//!      Up to 5 SET_FEATURE rounds are tried.
//!   2) G3 V2: VID 3837, receiver 0x4245, wired 0x418C, collection
//!      FF01:0010, 65-byte input/output reports starting
//!      [00 55 30 A5 0B 2E 01 01 01]. Reply starts AA 30 with battery at
//!      [8] and charging at [9]. Wired is preferred over the receiver and
//!      both share one card.
//!
//! Protocol from github.com/SlyF0x/mchose-battery-linux and the MCHOSE
//! G3 config notes; reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID_MCHOSE: u16 = 0x5253;
pub const VID_G3: u16 = 0x3837;
pub const G3_WIRED: u16 = 0x418C;
pub const G3_DONGLE: u16 = 0x4245;

// ---- MCHOSE (5253) ----

pub fn mchose_supported(vid: u16, _pid: u16) -> bool {
    vid == VID_MCHOSE
}

fn mchose_request() -> [u8; 65] {
    let mut req = [0u8; 65];
    req.fill(0xFF);
    req[0] = 0x12; // report id
    req[1] = 0x06 ^ 0xFF; // status command, inverted
    req
}

/// Decode one inverted reply; returns (level, charging) or None.
fn mchose_decode(r: &[u8]) -> Option<(u8, u8)> {
    if r.len() < 13 || r[0] != 0x12 || r[1] != 0xF9 {
        return None;
    }
    let p: Vec<u8> = r[2..13].iter().map(|b| b ^ 0xFF).collect();
    if p.iter().all(|&b| b == 0) {
        return Some((0, 0)); // all-zero decoded payload: mouse asleep
    }
    if u16::from_le_bytes([p[0], p[1]]) != VID_MCHOSE {
        return None;
    }
    let level = p[9];
    let charging = p[10];
    (level <= 100).then_some((level, charging))
}

pub fn poll_mchose(io: &dyn HidIo) -> Option<BatteryStatus> {
    let req = mchose_request();
    let mut best = None;
    for _ in 0..5 {
        if io.send_feature(&req).is_err() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
        let mut buf = [0u8; 65];
        buf[0] = 0x12;
        let Ok(n) = io.get_feature(&mut buf) else { break };
        if let Some(v) = mchose_decode(&buf[..n]) {
            best = Some(v);
            if v.0 > 0 {
                break;
            }
        }
    }
    match best {
        Some((0, _)) => Some(BatteryStatus::ASLEEP),
        Some((level, charging)) => Some(BatteryStatus {
            level: Some(level),
            charge: if charging != 0 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }),
        None => None,
    }
}

// ---- MCHOSE G3 V2 (3837) ----

pub fn g3_supported(vid: u16, pid: u16) -> bool {
    vid == VID_G3 && (pid == G3_WIRED || pid == G3_DONGLE)
}

const G3_REQUEST: [u8; 11] = [0x00, 0x55, 0x30, 0xA5, 0x0B, 0x2E, 0x01, 0x01, 0x01, 0x00, 0x00];

/// Reply starts AA 30 after an optional leading report-id byte; battery at
/// [8], charging at [9].
fn g3_parse(r: &[u8]) -> Option<(u8, u8)> {
    for off in 0..=1usize {
        if r.len() >= off + 10 && r[off] == 0xAA && r[off + 1] == 0x30 {
            let level = r[off + 8];
            if level <= 100 {
                return Some((level, r[off + 9]));
            }
        }
    }
    None
}

pub fn poll_g3(io: &dyn HidIo) -> Option<BatteryStatus> {
    query(io, &G3_REQUEST, &[65, 11], 700, g3_parse).ok().flatten().map(|(level, charging)| BatteryStatus {
        level: Some(level),
        charge: if charging != 0 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::{Mchose, MchoseG3};

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Mchose {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for Mchose {
        fn name(&self) -> &'static str {
            "mchose"
        }

        fn discover(&self) -> Vec<Device> {
            let infos = enumerate();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in &infos {
                if d.vendor_id() == VID_MCHOSE && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                // Vendor collection FF01:0001 first.
                let mut cols: Vec<_> = infos.iter().filter(|d| d.vendor_id() == VID_MCHOSE && d.product_id() == pid).collect();
                cols.sort_by_key(|d| (if d.usage_page() == 0xFF01 && d.usage() == 0x0001 { 0 } else { 1 }, d.interface_number()));
                let product = cols
                    .iter()
                    .find_map(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll_mchose(&io);
                    log::debug!("mchose {pid:04x} if{}: {s:?}", d.interface_number());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let name = product.trim().to_string();
                        let name = if name.is_empty() { "MCHOSE wireless mouse".into() } else { name };
                        let dev = Device {
                            id: format!("mchose:{pid:04x}"),
                            name: name.clone(),
                            vendor: "MCHOSE".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "mchose",
                            locator: format!("{path}\t{VID_MCHOSE:04x}\t{pid:04x}"),
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
            poll_mchose(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }

    /// G3 V2: wired and receiver share one card; wired wins when present.
    #[derive(Default)]
    pub struct MchoseG3 {
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for MchoseG3 {
        fn name(&self) -> &'static str {
            "mchose-g3"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut wired = None;
            let mut dongle = None;
            for d in enumerate() {
                if !(d.vendor_id() == VID_G3 && (d.product_id() == G3_WIRED || d.product_id() == G3_DONGLE) && d.usage_page() == 0xFF01 && d.usage() == 0x0010) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let Some(io) = open(&path) else { continue };
                if poll_g3(&io).is_some() {
                    if d.product_id() == G3_WIRED { wired = Some(path) } else { dongle = Some(path) }
                }
            }
            let path = wired.or(dongle);
            match path {
                Some(path) => {
                    let dev = Device {
                        id: "mchose:g3v2".into(),
                        name: "MCHOSE G3 V2".into(),
                        vendor: "MCHOSE".into(),
                        model: Some("MCHOSE G3 V2".into()),
                        kind: DeviceKind::Mouse,
                        provider: "mchose-g3",
                        locator: format!("{path}\t{VID_G3:04x}\t{G3_WIRED:04x}"),
                    };
                    *known = Some(dev.clone());
                    out.push(dev);
                }
                None => out.extend(known.take()),
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll_g3(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn mchose_decode_inverted() {
        // Build an inverted reply for level 80, charging 1.
        let mut p = [0u8; 11];
        p[0..2].copy_from_slice(&VID_MCHOSE.to_le_bytes());
        p[9] = 80;
        p[10] = 1;
        let mut r = vec![0x12u8, 0xF9];
        r.extend(p.iter().map(|b| b ^ 0xFF));
        r.resize(65, 0xFF);
        assert_eq!(mchose_decode(&r), Some((80, 1)));
    }

    #[test]
    fn mchose_all_zero_is_asleep() {
        let mut r = vec![0x12u8, 0xF9];
        r.extend(std::iter::repeat_n(0xFF, 11)); // decodes to all zeros
        r.resize(65, 0xFF);
        assert_eq!(mchose_decode(&r), Some((0, 0)));
        let io = FakeHid::new(move |_| vec![r.clone()]);
        assert_eq!(poll_mchose(&io), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn mchose_poll_level() {
        let io = FakeHid::new(|_| {
            let mut p = [0u8; 11];
            p[0..2].copy_from_slice(&VID_MCHOSE.to_le_bytes());
            p[9] = 55;
            p[10] = 0;
            let mut r = vec![0x12u8, 0xF9];
            r.extend(p.iter().map(|b| b ^ 0xFF));
            r.resize(65, 0xFF);
            vec![r]
        });
        let s = poll_mchose(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(55), Charge::Discharging));
    }

    #[test]
    fn g3_parse_offsets() {
        let mut r = vec![0u8; 10];
        r[0] = 0xAA;
        r[1] = 0x30;
        r[8] = 91;
        r[9] = 1;
        assert_eq!(g3_parse(&r), Some((91, 1)));
        // With leading report-id byte.
        let mut r2 = vec![0u8];
        r2.extend(&r);
        assert_eq!(g3_parse(&r2), Some((91, 1)));
    }

    #[test]
    fn g3_poll() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 10];
            r[0] = 0xAA;
            r[1] = 0x30;
            r[8] = 33;
            r[9] = 0;
            vec![r]
        });
        let s = poll_g3(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(33), Charge::Discharging));
    }
}
