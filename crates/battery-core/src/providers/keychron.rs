//! Keychron mice (VID 3434). Keychron keyboards only report batteries over
//! Bluetooth, so every 3434 device here is probed for the mouse protocol:
//!   Classic/8K: output report 0, 65 bytes [0x00][cmd][00][81][sub]...
//!   checksum = 0xA1 - sum(payload 0..62) at payload[63].
//!   Query 0x01 returns a tag (0x3434 = classic dongle, 0x362D = wired?) and
//!   battery fields; 0x41 distinguishes 8K/4K receivers; 0x40 with sub 0
//!   returns the paired product id used for naming.
//!   4K receivers live on page FF0A usage 0001 and answer the same 0x41
//!   query; Nape (Link-KM receiver 0xD026, Nape USB 0x0440) uses raw HID
//!   page FF60 usage 61 with 32-byte payloads and an A7 31 battery tunnel.
//! Protocol from github.com/printempire/keychron-mouse-battery and the
//! keychron-linux-battery project; reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x3434;
/// Link-KM receiver and Nape Pro cable.
pub const NAPE_PIDS: [u16; 2] = [0xD026, 0x0440];
pub const NAPE_PAGE: u16 = 0xFF60;
pub const NAPE_USAGE: u16 = 0x0061;

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && !NAPE_PIDS.contains(&pid) // Nape handled separately
}

pub fn nape_supported(vid: u16, pid: u16) -> bool {
    vid == VID && NAPE_PIDS.contains(&pid)
}

/// Build a 65-byte output report for (cmd, sub).
fn request(cmd: u8, sub: u8) -> [u8; 65] {
    let mut req = [0u8; 65];
    req[1] = cmd;
    req[3] = 0x81;
    req[4] = sub;
    let mut p = [0u8; 64];
    p.copy_from_slice(&req[1..]);
    let sum: u16 = p[..63].iter().map(|&b| b as u16).sum();
    let cksum = (0xA1u16.wrapping_sub(sum % 256) % 256) as u8;
    req[64] = cksum;
    req
}

/// Query cmd; returns the 64-byte payload (report-id byte stripped if the
/// stack kept it). Reply must start with `cmd` at payload[0].
fn query_cmd(io: &dyn HidIo, cmd: u8, sub: u8, timeout_ms: u64) -> Option<Vec<u8>> {
    let req = request(cmd, sub);
    query(io, &req, &[65, 64], timeout_ms, move |r| {
        // Tolerate an optional leading report-id byte.
        for off in 0..=1usize {
            if r.len() >= off + 6 && r[off] == cmd && r.get(off + 3) == Some(&sub) {
                return Some(r[off..].to_vec());
            }
        }
        None
    })
    .ok()
    .flatten()
}

/// One collection on the mouse protocol; None = try next.
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let r1 = query_cmd(io, 0x01, 0x01, 800)?;
    if r1.len() < 8 {
        return None;
    }
    let tag = u16::from_le_bytes([r1[6], r1[7]]);
    // 4K receiver: battery comes from the 0x41 query.
    if tag != 0x3434 && tag != 0x362D {
        if let Some(r) = query_cmd(io, 0x41, 0x01, 800) {
            if r.len() > 11 && (1..=100).contains(&r[6]) {
                return Some(BatteryStatus {
                    level: Some(r[6]),
                    charge: if r.get(5).copied().unwrap_or(0) != 0 { Charge::Charging } else { Charge::Discharging },
                    link: Link::Connected,
                });
            }
        }
        return None;
    }
    // 8K receiver reply carries a generation nibble + percent.
    if r1.len() > 11 && r1[10] <= 3 && (1..=100).contains(&r1[11]) {
        return Some(BatteryStatus {
            level: Some(r1[11]),
            charge: if r1[10] == 1 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        });
    }
    // Classic dongle / wired: percent at payload[11].
    if r1.len() > 11 && r1[11] <= 100 {
        let pct = r1[11];
        return Some(BatteryStatus {
            level: Some(if pct == 0 { 1 } else { pct }),
            charge: if r1.get(10).copied().unwrap_or(0) != 0 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        });
    }
    None
}

// ---- Nape (Link-KM) ----

fn nape_request(cmd: &[u8]) -> Vec<u8> {
    let mut req = vec![0u8; 33]; // unnumbered, 32-byte payload + id byte
    req[1..1 + cmd.len()].copy_from_slice(cmd);
    req
}

fn nape_query(io: &dyn HidIo, cmd: &[u8], timeout_ms: u64) -> Option<Vec<u8>> {
    query(io, &nape_request(cmd), &[33], timeout_ms, move |r| {
        for off in 0..=1usize {
            if r.len() >= off + cmd.len() && &r[off..off + cmd.len()] == cmd {
                return Some(r[off..].to_vec());
            }
        }
        None
    })
    .ok()
    .flatten()
}

/// Slots reply B2 lists paired devices; A7 31 tunnels to the Nape.
pub fn poll_nape(io: &dyn HidIo) -> Option<BatteryStatus> {
    // Ensure the Nape slot is claimed: query slots, then ask it directly.
    let _slots = nape_query(io, &[0xB2], 800);
    let r = nape_query(io, &[0xA7, 0x31], 800)?;
    if r.len() < 4 || r[2] > 100 {
        return None;
    }
    Some(BatteryStatus {
        level: Some(r[2]),
        charge: if r[3] != 0 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::{KeychronMice, KeychronNape};

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Collections for one id, vendor pages first.
    fn collections(
        infos: &[hidapi::DeviceInfo],
        vid: u16,
        pid: u16,
    ) -> Vec<&hidapi::DeviceInfo> {
        let mut cols: Vec<_> = infos.iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).collect();
        cols.sort_by_key(|d| (if d.usage_page() >= 0xFF00 { 0 } else { 1 }, d.interface_number()));
        cols
    }

    #[derive(Default)]
    pub struct KeychronMice {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for KeychronMice {
        fn name(&self) -> &'static str {
            "keychron"
        }

        fn discover(&self) -> Vec<Device> {
            let infos = enumerate();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in infos.iter() {
                if d.vendor_id() == VID && !NAPE_PIDS.contains(&d.product_id()) && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                let product = infos
                    .iter()
                    .find(|d| d.product_id() == pid)
                    .and_then(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let found = collections(&infos, VID, pid).into_iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io);
                    log::debug!("keychron {pid:04x} if{} {:04x}/{:04x}: {s:?}", d.interface_number(), d.usage_page(), d.usage());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let name = product.trim().to_string();
                        let name = if name.is_empty() { "Keychron wireless mouse".into() } else { name };
                        let dev = Device {
                            id: format!("keychron:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Keychron".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "keychron",
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
            poll(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }

    #[derive(Default)]
    pub struct KeychronNape {
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for KeychronNape {
        fn name(&self) -> &'static str {
            "keychron-nape"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            for d in enumerate().iter() {
                if !(d.vendor_id() == VID && NAPE_PIDS.contains(&d.product_id()) && d.usage_page() == NAPE_PAGE && d.usage() == NAPE_USAGE) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let Some(io) = open(&path) else { continue };
                if poll_nape(&io).is_some() {
                    let dev = Device {
                        id: "keychron:nape".into(),
                        name: "Keychron Nape".into(),
                        vendor: "Keychron".into(),
                        model: Some("Keychron Nape".into()),
                        kind: DeviceKind::Mouse,
                        provider: "keychron-nape",
                        locator: format!("{path}\t{VID:04x}\t{:04x}", d.product_id()),
                    };
                    *known = Some(dev.clone());
                    out.push(dev);
                } else {
                    out.extend(known.take());
                }
                break;
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll_nape(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// Classic dongle: tag 0x3434, percent at payload[11].
    fn classic(pct: u8, flags: u8) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 5 || req[1] != 0x01 {
                return vec![];
            }
            let mut p = vec![0u8; 64];
            p[0] = 0x01;
            p[3] = 0x01;
            p[6] = 0x34; // tag LE 0x3434
            p[7] = 0x34;
            p[10] = flags;
            p[11] = pct;
            let mut r = vec![0u8];
            r.extend_from_slice(&p);
            vec![r]
        })
    }

    #[test]
    fn classic_battery() {
        let s = poll(&classic(77, 0)).unwrap();
        assert_eq!((s.level, s.charge), (Some(77), Charge::Discharging));
        let s = poll(&classic(50, 1)).unwrap();
        assert_eq!(s.charge, Charge::Charging);
    }

    #[test]
    fn request_checksum() {
        let req = request(0x01, 0x01);
        assert_eq!(req[0], 0x00);
        assert_eq!(req[1], 0x01);
        assert_eq!(req[3], 0x81);
        // checksum = 0xA1 - sum(payload 0..62); recompute independently
        let payload: &[u8] = &req[1..65];
        let sum: u16 = payload[..63].iter().map(|&b| b as u16).sum();
        assert_eq!(payload[63], ((0xA1u16.wrapping_sub(sum % 256)) % 256) as u8);
    }

    #[test]
    fn nape_tunnel() {
        let io = FakeHid::new(|req| {
            if req.len() < 3 {
                return vec![];
            }
            match &req[1..3] {
                [0xB2, _] => vec![], // slots: ignore content
                [0xA7, 0x31] => {
                    let mut r = vec![0u8; 8];
                    r[0] = 0xA7;
                    r[1] = 0x31;
                    r[2] = 66;
                    r[3] = 1;
                    vec![r]
                }
                _ => vec![],
            }
        });
        let s = poll_nape(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(66), Charge::Charging));
    }

    #[test]
    fn supported_split() {
        assert!(supported(VID, 0x1234));
        assert!(!supported(VID, 0xD026));
        assert!(nape_supported(VID, 0xD026));
    }
}
