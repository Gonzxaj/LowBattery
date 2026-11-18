//! Kysona, Pulsar and ATK / VXE wireless mice (VID 3554, ATK dongles 373B),
//! G-Wolves models on this protocol (VID 33E4), the original Hitscan
//! Hyperlight (VID 3770), Pulsar's newer XS-1 generation (VID 3710),
//! Redragon's Compx-based mice and HECATE (VID 35BB). They share one
//! protocol on 17-byte report id 08:
//!   [08][cmd][0 x14][checksum], checksum = 0x55 - sum(bytes 0..15)
//!   cmd 03 = online status -> reply [6] = 1 when the mouse is connected
//!   cmd 04 = battery       -> reply [6] = percent, [7] = charging,
//!                             [8..9] = millivolts
//! The ATK N9 headsets speak a serial protocol and are skipped. The battery
//! channel is FF02:0002 where it exists; vendor collections are tried in
//! that order first.
//! Protocol: Linux drivers/hid/hid-kysona.c (Lode Willems), hid-pulsar patch
//! discussion, github.com/S1berla/vxe-r1-pro-max-battery,
//! github.com/orcunturkol/BatteryTray; reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VIDS: [u16; 6] = [0x3554, 0x373B, 0x3770, 0x33E4, 0x3710, 0x35BB];
/// ATK N9 Pro / N9 Ultra headsets: serial protocol, never send mouse frames.
const ATK_HEADSET_PIDS: [u16; 4] = [0x11CA, 0x11CB, 0x1203, 0x1204];

pub fn supported(vid: u16, pid: u16) -> bool {
    VIDS.contains(&vid) && !(vid == 0x373B && ATK_HEADSET_PIDS.contains(&pid))
}

fn checksum(b: &[u8]) -> u8 {
    (0x55u16.wrapping_sub(b[..16].iter().map(|&x| x as u16).sum::<u16>() % 256) % 256) as u8
}

/// Sends `cmd` and waits for its checksummed reply. Returns reply payload.
fn query17(io: &dyn HidIo, cmd: u8) -> Option<Vec<u8>> {
    let mut req = [0u8; 17];
    req[0] = 0x08;
    req[1] = cmd;
    req[16] = checksum(&req);
    query(io, &req, &[32, 64], 500, |r| {
        (r.len() >= 17 && r[0] == 0x08 && r[1] == cmd && r[16] == checksum(r)).then(|| r[..17].to_vec())
    })
    .ok()
    .flatten()
}

/// One collection: online check then battery. None = not this protocol.
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let mut answered = false;
    let mut online = false;
    if let Some(r) = query17(io, 0x03) {
        answered = true;
        online = r[6] != 0;
    }
    if !answered || online {
        if let Some(r) = query17(io, 0x04) {
            let pct = r[6];
            return Some(if pct <= 100 {
                BatteryStatus {
                    level: Some(pct),
                    charge: if r[7] != 0 { Charge::Charging } else { Charge::Discharging },
                    link: Link::Connected,
                }
            } else {
                BatteryStatus { level: None, charge: Charge::Unknown, link: Link::Connected }
            });
        }
    }
    if !answered {
        return None;
    }
    // The receiver answered but the mouse did not: asleep or switched off.
    let _ = online;
    Some(BatteryStatus::ASLEEP)
}

/// Display name for a (vid,pid): model tables where the PID is shared,
/// else the cleaned product string handled by the caller.
pub fn fixed_name(vid: u16, pid: u16) -> Option<&'static str> {
    if vid == 0x33E4 {
        return Some(crate::providers::glorious::gwolves_name(pid).unwrap_or("G-Wolves wireless mouse"));
    }
    if vid == 0x3554 && matches!(pid, 0xF55D | 0xF5D5 | 0xF55E) {
        return Some("Redragon wireless mouse"); // shared by several models
    }
    if vid == 0x3554 && matches!(pid, 0xF58A | 0xF58C | 0xF58E | 0xF58F | 0xF505 | 0xF506) {
        return Some("VXE Dragonfly R1");
    }
    if vid == 0x3770 {
        return Some("Hitscan Hyperlight"); // the receiver only says "Hitscan USB Receiver"
    }
    if vid == 0x35BB {
        return Some("HECATE mouse");
    }
    None
}

/// Cable PIDs (the mouse itself on USB).
pub fn is_cable(vid: u16, pid: u16) -> bool {
    (vid == 0x33E4 && crate::providers::glorious::gwolves_cable(pid))
        || (vid == 0x35BB && matches!(pid, 0xD3FA | 0xD3FC | 0xD3FD))
        || (vid == 0x3554 && matches!(pid, 0xF55E | 0xF58C | 0xF58F | 0xF506))
}

/// Strip receiver-ish suffixes from a product string.
pub fn clean_name(mut n: String, vid: u16, pid: u16) -> String {
    if let Some(fixed) = fixed_name(vid, pid) {
        return fixed.to_string();
    }
    for suffix in [" Dongle", " Receiver", " 2.4G", " Wireless Receiver"] {
        if n.len() > suffix.len() && n.ends_with(suffix) {
            n.truncate(n.len() - suffix.len());
        }
    }
    if n.trim().is_empty() {
        n = if vid == 0x373B { "ATK wireless mouse".into() } else { "Wireless mouse".into() };
    }
    n
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::KysonaMice;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct KysonaMice {
        known: Mutex<BTreeMap<(u16, u16), Device>>,
    }

    impl BatteryProvider for KysonaMice {
        fn name(&self) -> &'static str {
            "kysona"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> =
                enumerate().iter().filter(|d| supported(d.vendor_id(), d.product_id())).cloned().collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut seen_names: Vec<String> = Vec::new();
            let mut ids: Vec<(u16, u16)> = infos.iter().map(|d| (d.vendor_id(), d.product_id())).collect();
            ids.sort_unstable();
            ids.dedup();
            for &(vid, pid) in &ids {
                let product = infos
                    .iter()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
                    .and_then(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let name = clean_name(product.clone(), vid, pid);
                // A mouse plugged in for charging can show up twice
                // (cable + dongle); keep one card per physical mouse.
                if seen_names.contains(&name) {
                    continue;
                }
                let mut cols: Vec<_> = infos.iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).collect();
                // The battery channel is FF02:0002 where it exists; vendor
                // pages first, then the rest.
                cols.sort_by_key(|d| {
                    let pri = if d.usage_page() == 0xFF02 && d.usage() == 0x0002 {
                        0
                    } else if d.usage_page() >= 0xFF00 {
                        1
                    } else {
                        2
                    };
                    (pri, d.interface_number())
                });
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io);
                    log::debug!("kysona {vid:04x}:{pid:04x} if{} {:04x}/{:04x}: {s:?}", d.interface_number(), d.usage_page(), d.usage());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, status)) => {
                        if status.link != Link::Connected {
                            // Receiver answered "mouse not there": keep any
                            // previous card asleep instead of duplicating.
                            out.extend(known.get(&(vid, pid)).cloned());
                            continue;
                        }
                        seen_names.push(name.clone());
                        let kind = match crate::guess_kind(&name) {
                            DeviceKind::Other => DeviceKind::Mouse,
                            k => k,
                        };
                        let dev = Device {
                            id: format!("kysona:{vid:04x}:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Mouse".into(),
                            model: Some(name),
                            kind,
                            provider: "kysona",
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

    fn reply(cmd: u8, b6: u8, b7: u8, b8: u8, b9: u8) -> Vec<u8> {
        let mut r = vec![0u8; 17];
        r[0] = 0x08;
        r[1] = cmd;
        r[6] = b6;
        r[7] = b7;
        r[8] = b8;
        r[9] = b9;
        r[16] = checksum(&r);
        r
    }

    fn dongle(online: u8, pct: u8, charging: u8) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 17 || req[0] != 0x08 || req[16] != checksum(req) {
                return vec![];
            }
            match req[1] {
                0x03 => vec![reply(0x03, online, 0, 0, 0)],
                0x04 => vec![reply(0x04, pct, charging, 0x0E, 0xDC)], // 3800 mV
                _ => vec![],
            }
        })
    }

    #[test]
    fn online_battery() {
        let s = poll(&dongle(1, 73, 1)).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(73), Charge::Charging, Link::Connected));
    }

    #[test]
    fn offline_mouse_is_asleep() {
        assert_eq!(poll(&dongle(0, 0, 0)), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn silence_is_none() {
        assert_eq!(poll(&FakeHid::new(|_| vec![])), None);
    }

    #[test]
    fn checksum_vector() {
        let mut b = [0u8; 17];
        b[0] = 0x08;
        b[1] = 0x04;
        assert_eq!(checksum(&b), (0x55u16.wrapping_sub(0x0C) % 256) as u8);
    }

    #[test]
    fn tables() {
        assert!(supported(0x3554, 0xF55D));
        assert!(!supported(0x373B, 0x11CA)); // ATK headset
        assert_eq!(fixed_name(0x3554, 0xF55D), Some("Redragon wireless mouse"));
        assert!(is_cable(0x3554, 0xF55E));
        assert!(!is_cable(0x3554, 0xF55D));
    }
}
