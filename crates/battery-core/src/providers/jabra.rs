//! Jabra headsets behind a Jabra Link 380 / 390 dongle: Jabra's GNP protocol
//! on the vendor collection with usage page FF00 (report 05, 64 bytes).
//!   query: [05][dst][src 00][seq][40|len][class][op][payload]
//!   reply: [05][dst 00][src][seq][C0|len][class][op][payload]   class FE = NAK
//! Addresses: 01 = the dongle itself, 04 = the headset behind it.
//! Battery = class 12 / op 02, reply payload [flags][level]; flags bit 0 =
//! charging, bit 7 = per-unit levels follow (earbuds), the level byte is then
//! the combined value. Name = class 02 / op 00, payload [n][n bytes UTF-8].
//! A switched-off headset gets a NAK (or no reply) while the dongle stays
//! present.
//! Protocol from github.com/Watchdog0x/jabridge (cmd/jabridge/jabraApi.go),
//! verified upstream on a Link 390 (0B0E:2E51) with an Evolve3 75;
//! reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};
use std::sync::atomic::{AtomicU8, Ordering};

pub const VID: u16 = 0x0B0E;
/// Link 380a/380c and 390 USB-A/USB-C, UC and MS variants.
pub const PIDS: [u16; 9] = [0x24C7, 0x24C8, 0x24C9, 0x24CA, 0x24E9, 0x2E50, 0x2E51, 0x2E56, 0x2E57];

const HEADSET: u8 = 0x04;
const TIMEOUT_MS: u64 = 700;

static SEQ: AtomicU8 = AtomicU8::new(0x20);

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

enum Reply {
    Ok(Vec<u8>),
    Nak,
}

/// One GNP exchange; Ok(None) = silence, Some(Nak) = headset refused.
fn gnp(io: &dyn HidIo, dst: u8, cls: u8, op: u8, timeout_ms: u64) -> Option<Reply> {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let req = [0x05, dst, 0x00, seq, 0x46, cls, op];
    query(io, &req, &[64], timeout_ms, |r| {
        if r.len() < 6 || r[0] != 0x05 || r[1] != 0x00 || r[2] != dst || r[3] != seq || r[4] & 0xC0 != 0xC0 {
            return None;
        }
        if r[5] == 0xFE {
            return Some(Reply::Nak);
        }
        let len = (r[4] & 0x3F) as usize; // counts from r[1]
        if r.len() < 7 || r[5] != cls || r[6] != op || len < 6 || len + 1 > r.len() {
            return None;
        }
        Some(Reply::Ok(r[7..len + 1].to_vec()))
    })
    .ok()
    .flatten()
}

/// Battery query to the headset at GNP address 4. None = dongle silent.
fn battery(io: &dyn HidIo) -> Option<BatteryStatus> {
    match gnp(io, HEADSET, 0x12, 0x02, TIMEOUT_MS)? {
        Reply::Nak => Some(BatteryStatus::ASLEEP),
        Reply::Ok(p) if p.len() >= 2 => Some(BatteryStatus {
            level: (p[1] <= 100).then_some(p[1]),
            charge: if p[0] & 0x01 != 0 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }),
        Reply::Ok(_) => None,
    }
}

/// Class 02 / op 00 name query; payload [n][UTF-8 name].
fn headset_name(io: &dyn HidIo) -> Option<String> {
    match gnp(io, HEADSET, 0x02, 0x00, 400)? {
        Reply::Ok(p) if !p.is_empty() && p[0] != 0 && (p[0] as usize) < p.len() => {
            let n = p[0] as usize;
            Some(String::from_utf8_lossy(&p[1..1 + n]).trim().to_string()).filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Jabra;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Jabra {
        /// Dongle plugged in but headset silent: keep the card as asleep.
        known: Mutex<BTreeMap<u16, Device>>,
        /// Headset name per dongle PID, asked once while connected and
        /// forgotten on NAK so a newly paired headset is picked up.
        names: Mutex<BTreeMap<u16, String>>,
    }

    impl BatteryProvider for Jabra {
        fn name(&self) -> &'static str {
            "jabra"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> =
                enumerate().iter().filter(|d| supported(d.vendor_id(), d.product_id()) && d.usage_page() == 0xFF00).cloned().collect();
            let mut known = self.known.lock().unwrap();
            let mut names = self.names.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            pids.sort_unstable();
            pids.dedup();
            for &pid in &pids {
                let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == pid).collect();
                cols.sort_by_key(|d| d.interface_number());
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = battery(&io);
                    log::debug!("jabra {pid:04x} if{}: {s:?}", d.interface_number());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, status)) => {
                        if status.link == Link::Asleep {
                            names.remove(&pid); // headset changed while off
                        } else if let std::collections::btree_map::Entry::Vacant(e) = names.entry(pid) {
                            if let Some(io) = open(&path) {
                                e.insert(headset_name(&io).unwrap_or_else(|| "Jabra headset".into()));
                            }
                        }
                        let dev = Device {
                            id: format!("jabra:{pid:04x}"),
                            name: names.get(&pid).cloned().unwrap_or_else(|| "Jabra headset".into()),
                            vendor: "Jabra".into(),
                            model: None,
                            kind: DeviceKind::Headset,
                            provider: "jabra",
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
            battery(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// GNP dongle: battery query to address 4, optional NAK or name reply.
    fn dongle(nak: bool, name: Option<&'static [u8]>) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 7 || req[0] != 0x05 || req[1] != HEADSET || req[4] != 0x46 {
                return vec![];
            }
            let seq = req[3];
            match (req[6], nak) {
                (0x02, false) => {
                    // battery: flags=1 charging, level=80; len counts from r[1] -> end index 9
                    vec![vec![0x05, 0x00, HEADSET, seq, 0xC8, 0x12, 0x02, 0x01, 80]]
                }
                (0x02, true) => vec![vec![0x05, 0x00, HEADSET, seq, 0xC4, 0xFE, 0x00, 0x00]],
                (0x00, false) => {
                    let n = name.map(|s| s.len()).unwrap_or(0) as u8;
                    let mut p = vec![n];
                    p.extend_from_slice(name.unwrap_or(b""));
                    let mut r = vec![0x05, 0x00, HEADSET, seq, 0xC0 | (6 + p.len()) as u8, 0x02, 0x00];
                    r.extend_from_slice(&p);
                    while r.len() < 9 {
                        r.push(0);
                    }
                    vec![r]
                }
                _ => vec![],
            }
        })
    }

    #[test]
    fn battery_connected_charging() {
        let s = battery(&dongle(false, None)).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(80), Charge::Charging, Link::Connected));
    }

    #[test]
    fn nak_is_asleep() {
        assert_eq!(battery(&dongle(true, None)), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn silence_is_none() {
        assert_eq!(battery(&FakeHid::new(|_| vec![])), None);
    }

    #[test]
    fn name_query() {
        assert_eq!(headset_name(&dongle(false, Some(b"Evolve3 75"))).as_deref(), Some("Evolve3 75"));
    }

    #[test]
    fn supported_tables() {
        assert!(supported(0x0B0E, 0x2E51));
        assert!(supported(0x0B0E, 0x24C7));
        assert!(!supported(0x0B0E, 0x1234));
        assert!(!supported(0x1234, 0x2E51));
    }
}
