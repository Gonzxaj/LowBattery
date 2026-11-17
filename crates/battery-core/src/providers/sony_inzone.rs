//! Sony INZONE H9 II (054C:0FA8) and H5 (054C:0EBF) headsets and the INZONE
//! Buds dongle (054C:0EC2): a Sony command layer tunnelled through HID
//! (report 02, 64 bytes) on the vendor collection with usage page FF04.
//!   command: [02][len][01][00 FC][8+n][96 C3][addr][event][type][tid lo]
//!            [tid hi][payload][sum]
//!   event:   [02][len][04][FF][..][00][96 C3][addr][event][type][tid lo]
//!            [tid hi][payload][sum]
//! Battery = event 04, type 01 (GET) to the headset (addr 0x41). Both
//! directions carry a checksum and a transaction id.
//!   Headsets: reply payload [charging][percent], percent FF = headset off.
//!   Buds:     reply payload [status][percent] for the left bud, right bud,
//!             then case; percent FF = bud is in the case. With both buds in
//!             the case the dongle does not respond. No charging flag.
//! Protocol from github.com/Sapd/HeadsetControl (sony_inzone_protocol.hpp,
//! sony_inzone_buds.hpp); buds reply layout from the inzone-buds-battery-tray
//! projects; reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};
use std::sync::atomic::{AtomicU16, Ordering};

pub const VID: u16 = 0x054C;
pub const PIDS: [u16; 3] = [0x0FA8, 0x0EBF, 0x0EC2];

const TIMEOUT_MS: u64 = 1500;

pub struct Model {
    pub pid: u16,
    pub name: &'static str,
    /// Earbuds: 3x [status][percent] payload, no charging flag.
    pub buds: bool,
    /// Minimum reply payload length accepted.
    pub min_payload: usize,
}

pub const MODELS: &[Model] = &[
    Model { pid: 0x0FA8, name: "Sony INZONE H9 II", buds: false, min_payload: 2 },
    Model { pid: 0x0EBF, name: "Sony INZONE H5", buds: false, min_payload: 2 },
    Model { pid: 0x0EC2, name: "Sony INZONE Buds", buds: true, min_payload: 6 },
];

pub fn model(pid: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.pid == pid)
}

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

/// tid 1 is used by the dongle's own notifications; skip it.
static TID: AtomicU16 = AtomicU16::new(2);

struct Event {
    id: u8,
    r#type: u8,
    tid: u16,
    payload: Vec<u8>,
}

fn parse_event(b: &[u8]) -> Option<Event> {
    if b.len() < 15 || b[0] != 0x02 {
        return None;
    }
    let len = b[1] as usize;
    if !(12..=62).contains(&len) || b.len() < len + 2 {
        return None;
    }
    if b[2] != 0x04 || b[3] != 0xFF || b[5] != 0x00 || b[6] != 0x96 || b[7] != 0xC3 {
        return None;
    }
    if b[8] >> 4 != 0x1 {
        return None; // must be addressed to the PC
    }
    if b[5..=len].iter().fold(0u8, |a, &x| a.wrapping_add(x)) != b[len + 1] {
        return None;
    }
    Some(Event {
        id: b[9],
        r#type: b[10],
        tid: b[11] as u16 | ((b[12] as u16) << 8),
        payload: b[13..len + 1].to_vec(),
    })
}

/// 64-byte battery GET command (HCI command 0xFC00, event 04 / type 01).
fn request(tid: u16) -> [u8; 64] {
    let mut req = [0u8; 64];
    req[0] = 0x02;
    req[1] = 12; // HCI byte count (no payload)
    req[2] = 0x01; // HCI command
    req[4] = 0xFC; // vendor opcode 0xFC00
    req[5] = 8;
    req[6] = 0x96;
    req[7] = 0xC3;
    req[8] = 0x41; // PC -> headset
    req[9] = 0x04; // battery info
    req[10] = 0x01; // GET
    req[11] = tid as u8;
    req[12] = (tid >> 8) as u8;
    req[13] = req[6..=12].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    req
}

/// Battery query. Ok(None) = no usable reply; buds treat that as "both buds
/// in the case" and report asleep, headsets report nothing new.
fn battery(io: &dyn HidIo, m: &Model) -> Option<BatteryStatus> {
    let tid = TID.fetch_add(1, Ordering::Relaxed);
    let payload = query(io, &request(tid), &[], TIMEOUT_MS, |r| {
        let e = parse_event(r)?;
        if e.id != 0x04 || e.tid != tid || e.payload.len() < m.min_payload {
            return None;
        }
        // RET or active notification only.
        (e.r#type == 0x10 || e.r#type == 0xA0).then_some(e.payload)
    })
    .ok()
    .flatten()?;

    if m.buds {
        // Lowest bud charge; FF = docked, values > 100 invalid.
        let lvl = [1usize, 3]
            .into_iter()
            .filter(|&i| payload[i] <= 100)
            .map(|i| payload[i])
            .min();
        return Some(match lvl {
            Some(l) => BatteryStatus { level: Some(l), charge: Charge::Unknown, link: Link::Connected },
            None => BatteryStatus::ASLEEP,
        });
    }
    let offline = payload[1] == 0xFF;
    if offline {
        return Some(BatteryStatus::ASLEEP);
    }
    Some(BatteryStatus {
        level: (payload[1] <= 100).then_some(payload[1]),
        charge: if payload[0] != 0 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

/// NoReply from the buds dongle means both buds are docked: still a card.
fn poll(io: &dyn HidIo, m: &Model) -> Option<BatteryStatus> {
    match battery(io, m) {
        Some(s) => Some(s),
        None if m.buds => Some(BatteryStatus::ASLEEP),
        None => None,
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::SonyInzone;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct SonyInzone {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for SonyInzone {
        fn name(&self) -> &'static str {
            "sony-inzone"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate()
                .into_iter()
                .filter(|d| supported(d.vendor_id(), d.product_id()) && d.usage_page() == 0xFF04)
                .collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            pids.sort_unstable();
            pids.dedup();
            for &pid in &pids {
                let Some(m) = model(pid) else { continue };
                let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == pid).collect();
                cols.sort_by_key(|d| d.interface_number());
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io, m);
                    log::debug!("sony-inzone {} {:04x} if{}: {s:?}", m.name, pid, d.interface_number());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let dev = Device {
                            id: format!("sony-inzone:{pid:04x}"),
                            name: m.name.into(),
                            vendor: "Sony".into(),
                            model: Some(m.name.into()),
                            kind: DeviceKind::Headset,
                            provider: "sony-inzone",
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
            let mut parts = device.locator.split('\t');
            let Some(path) = parts.next() else { return BatteryStatus::GONE };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            let pid = parts.nth(1).and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
            match model(pid) {
                Some(m) => poll(&io, m).unwrap_or(BatteryStatus::ASLEEP),
                None => BatteryStatus::GONE,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// Build an event reply for the battery GET with the given payload.
    fn event(tid: u16, payload: &[u8]) -> Vec<u8> {
        let len = 12 + payload.len();
        let mut b = vec![0u8; len + 2];
        b[0] = 0x02;
        b[1] = len as u8;
        b[2] = 0x04;
        b[3] = 0xFF;
        b[5] = 0x00;
        b[6] = 0x96;
        b[7] = 0xC3;
        b[8] = 0x10; // high nibble 1 = addressed to the PC
        b[9] = 0x04;
        b[10] = 0x10; // RET
        b[11] = tid as u8;
        b[12] = (tid >> 8) as u8;
        b[13..13 + payload.len()].copy_from_slice(payload);
        b[len + 1] = b[5..=len].iter().fold(0u8, |a, &x| a.wrapping_add(x));
        b
    }

    /// Dongle: echoes the tid from the request into the battery reply.
    fn dongle(payload: &'static [u8]) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 14 || req[0] != 0x02 || req[9] != 0x04 || req[10] != 0x01 {
                return vec![];
            }
            let tid = req[11] as u16 | ((req[12] as u16) << 8);
            vec![event(tid, payload)]
        })
    }

    #[test]
    fn headset_charging() {
        let m = &MODELS[0];
        let s = battery(&dongle(&[1, 80]), m).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(80), Charge::Charging, Link::Connected));
    }

    #[test]
    fn headset_offline_is_asleep() {
        let m = &MODELS[1];
        assert_eq!(battery(&dongle(&[0, 0xFF]), m), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn buds_lowest_out() {
        let m = &MODELS[2];
        // left 90, right 40, case 100 -> lowest out = 40
        let s = battery(&dongle(&[0, 90, 0, 40, 0, 100]), m).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(40), Charge::Unknown, Link::Connected));
        // both buds docked (FF) -> asleep
        assert_eq!(battery(&dongle(&[0, 0xFF, 0, 0xFF, 0, 55]), m), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn buds_no_reply_is_asleep_card() {
        let m = &MODELS[2];
        assert_eq!(poll(&FakeHid::new(|_| vec![]), m), Some(BatteryStatus::ASLEEP));
        // headsets stay silent instead
        assert_eq!(poll(&FakeHid::new(|_| vec![]), &MODELS[0]), None);
    }

    #[test]
    fn wrong_tid_rejected() {
        let m = &MODELS[0];
        // reply with a stale tid never matches
        let io = FakeHid::new(|_| vec![event(0x7777, &[0, 50])]);
        assert_eq!(battery(&io, m), None);
    }

    #[test]
    fn supported_tables() {
        assert!(supported(0x054C, 0x0FA8));
        assert!(supported(0x054C, 0x0EC2));
        assert!(!supported(0x054C, 0x05C4)); // DS4 belongs to playstation
        assert!(!supported(0x1234, 0x0FA8));
    }
}
