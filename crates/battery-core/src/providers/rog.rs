//! ASUS ROG wireless keyboards, mice and the Pelta headset (VID 0B05),
//! including the ROG Omni Receiver (0B05:1ACE).
//!   Query: [report id][12][sub]; sub 01 = keyboard battery, sub 07 = mouse.
//!   Reply: keyboard [offset]=% (Falchion: 11), [9]==1 charging;
//!          mouse    [offset]=level x scale (older 0-4 gauges x25), [10] charging.
//!   [1..2] == FF AA means the device behind the receiver did not answer.
//! Omni-channel entries use numbered reports (2 for keyboards, 3 for mice)
//! on interface 2; receiver/wired entries are unnumbered on the interfaces
//! in the table. Only query commands are sent.
//! Protocol from SteveWang92/peripheral-battery-tray and 1gcat/BatteryTool;
//! no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x0B05;
pub const OMNI_PID: u16 = 0x1ACE;
pub const PELTA_PID: u16 = 0x1B84;

#[derive(Clone, Copy)]
pub struct Model {
    pub pid: u16,
    pub name: &'static str,
    pub keyboard: bool,
    pub report_id: u8, // 0 = unnumbered
    pub iface: i32,    // -1 = any vendor collection
    pub offset: usize,
    pub scale: u16,
}

const KB: bool = true;
const MS: bool = false;

/// (pid, name, kind, reportId, iface, offset, scale)
const MODELS: &[Model] = &[
    Model { pid: 0x1A85, name: "ROG Azoth", keyboard: KB, report_id: 2, iface: 2, offset: 6, scale: 1 },
    Model { pid: 0x1A85, name: "ROG Azoth", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1A83, name: "ROG Azoth", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1B42, name: "ROG Azoth Extreme", keyboard: KB, report_id: 2, iface: 2, offset: 6, scale: 1 },
    Model { pid: 0x1B3F, name: "ROG Azoth Extreme", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1CF1, name: "ROG Azoth Extreme SE", keyboard: KB, report_id: 2, iface: 2, offset: 6, scale: 1 },
    Model { pid: 0x1CEF, name: "ROG Azoth Extreme SE", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1C25, name: "ROG Azoth X", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1C24, name: "ROG Azoth X", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x193E, name: "ROG Falchion", keyboard: KB, report_id: 0, iface: 1, offset: 11, scale: 1 },
    Model { pid: 0x193C, name: "ROG Falchion", keyboard: KB, report_id: 0, iface: 1, offset: 11, scale: 1 },
    Model { pid: 0x1B04, name: "ROG Falchion RX", keyboard: KB, report_id: 0, iface: -1, offset: 11, scale: 1 },
    Model { pid: 0x1B06, name: "ROG Falchion RX Low Profile", keyboard: KB, report_id: 2, iface: 2, offset: 11, scale: 1 },
    Model { pid: 0x1AB0, name: "ROG Strix Scope II 96", keyboard: KB, report_id: 2, iface: 2, offset: 6, scale: 1 },
    Model { pid: 0x1AAE, name: "ROG Strix Scope II 96", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1B7A, name: "ROG Strix Scope II 96 RX", keyboard: KB, report_id: 2, iface: 2, offset: 6, scale: 1 },
    Model { pid: 0x1B78, name: "ROG Strix Scope II 96 RX", keyboard: KB, report_id: 0, iface: 1, offset: 6, scale: 1 },
    Model { pid: 0x1A07, name: "ROG Strix Scope RX TKL", keyboard: KB, report_id: 0, iface: -1, offset: 6, scale: 1 },
    Model { pid: 0x18E5, name: "ROG Chakram", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x18E3, name: "ROG Chakram", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1A1A, name: "ROG Chakram X", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1A18, name: "ROG Chakram X", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x18A0, name: "ROG Gladius II Wireless", keyboard: MS, report_id: 0, iface: 2, offset: 5, scale: 25 },
    Model { pid: 0x197F, name: "ROG Gladius III Wireless", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x197D, name: "ROG Gladius III Wireless", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1A72, name: "ROG Gladius III Aimpoint", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1A72, name: "ROG Gladius III Aimpoint", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1A70, name: "ROG Gladius III Aimpoint", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1B0C, name: "ROG Gladius III Aimpoint EVA-02", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1B0A, name: "ROG Gladius III Aimpoint EVA-02", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1A94, name: "ROG Harpe Ace Aim Lab Edition", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1A94, name: "ROG Harpe Ace Aim Lab Edition", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1A92, name: "ROG Harpe Ace Aim Lab Edition", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1B68, name: "ROG Harpe Ace Extreme", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1B69, name: "ROG Harpe Ace Extreme", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1B67, name: "ROG Harpe Ace Extreme", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1B65, name: "ROG Harpe Ace Mini", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1B63, name: "ROG Harpe Ace Mini", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1AD0, name: "ROG Harpe II Ace", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1C69, name: "ROG Harpe II Ace", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1960, name: "ROG Keris Wireless", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x195E, name: "ROG Keris Wireless", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1A59, name: "ROG Keris Wireless EVA Edition", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1A57, name: "ROG Keris Wireless EVA Edition", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1A68, name: "ROG Keris Wireless Aimpoint", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1A6A, name: "ROG Keris Wireless Aimpoint", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1A68, name: "ROG Keris Wireless Aimpoint", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1A66, name: "ROG Keris Wireless Aimpoint", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1B1A, name: "ROG Keris II Ace", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1B18, name: "ROG Keris II Ace", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1B16, name: "ROG Keris II Ace", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1C0E, name: "ROG Keris II Origin", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1C0C, name: "ROG Keris II Origin", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1D4E, name: "ROG Keris II Origin KJP", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
    Model { pid: 0x1D4C, name: "ROG Keris II Origin KJP", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1908, name: "ROG Pugio II", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1906, name: "ROG Pugio II", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1979, name: "ROG Spatha X", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x1977, name: "ROG Spatha X", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 1 },
    Model { pid: 0x18B4, name: "ROG Strix Carry", keyboard: MS, report_id: 0, iface: 1, offset: 7, scale: 25 },
    Model { pid: 0x1949, name: "ROG Strix Impact II Wireless", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1947, name: "ROG Strix Impact II Wireless", keyboard: MS, report_id: 0, iface: 0, offset: 5, scale: 25 },
    Model { pid: 0x1AD7, name: "ROG Strix Impact III Wireless", keyboard: MS, report_id: 3, iface: 2, offset: 5, scale: 1 },
];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && (pid == OMNI_PID || pid == PELTA_PID || MODELS.iter().any(|m| m.pid == pid))
}

/// One battery query; Asleep when the receiver answered FF AA.
fn query_model(io: &dyn HidIo, m: &Model) -> Option<BatteryStatus> {
    let sub = if m.keyboard { 0x01 } else { 0x07 };
    let req = [m.report_id, 0x12, sub];
    query(io, &req, &[16, 64], 500, move |r| {
        // Normalize: unnumbered stacks lack the report-id slot.
        let r: Vec<u8> = if m.report_id == 0 && r.first() != Some(&0) { std::iter::once(0).chain(r.iter().copied()).collect() } else { r.to_vec() };
        if r.len() < 12 {
            return None;
        }
        if r[1] == 0xFF && r[2] == 0xAA {
            return Some(BatteryStatus::ASLEEP);
        }
        if r[1] != 0x12 || r[2] != sub {
            return None;
        }
        Some(if m.keyboard {
            let pct = r[m.offset];
            BatteryStatus {
                level: (pct <= 100).then_some(pct),
                charge: if r[9] == 1 { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            }
        } else {
            let v = r[m.offset] as u16 * m.scale;
            BatteryStatus {
                level: (v > 0 && v <= 100).then_some(v as u8),
                charge: if r[10] != 0 { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            }
        })
    })
    .ok()
    .flatten()
}

/// Pelta headset: report CC on FF00:0001 iface 3; [5]=sleep,[6]=% (FE=?)
/// [7]=low alert; cmd 08 -> [5]=charging. Asleep on FF AA.
fn poll_pelta(io: &dyn HidIo) -> Option<BatteryStatus> {
    let r = query(io, &[0xCC, 0x12, 0x07], &[64], 1200, |r| {
        (r.len() >= 8 && r[0] == 0xCC && r[1] == 0x12 && r[2] == 0x07).then(|| r.to_vec())
    })
    .ok()
    .flatten()?;
    if r[1] == 0xFF && r[2] == 0xAA || (r[5] == 0xFF && r[6] == 0xAA) {
        return Some(BatteryStatus::ASLEEP);
    }
    let pct = r[6];
    let level = (pct <= 100).then_some(pct);
    let charging = query(io, &[0xCC, 0x12, 0x08], &[64], 1200, |r| {
        (r.len() >= 8 && r[0] == 0xCC && r[1] == 0x12 && r[2] == 0x08).then(|| r[5] == 1)
    })
    .ok()
    .flatten()
    .unwrap_or(false);
    Some(if level.is_none() {
        BatteryStatus::ASLEEP // FE = level not known yet
    } else {
        BatteryStatus { level, charge: if charging { Charge::Charging } else { Charge::Discharging }, link: Link::Connected }
    })
}

/// Omni receiver: [01 A0 00] on interface 2 lists paired product ids in
/// 4-byte slots from byte 5.
pub fn omni_pids(io: &dyn HidIo) -> Vec<u16> {
    let mut out = Vec::new();
    if let Ok(Some(r)) = query(io, &[0x01, 0xA0, 0x00], &[16, 64], 500, |r| {
        (r.len() >= 6 && r[1] == 0xA0).then(|| r.to_vec())
    }) {
        for at in (5..r.len().saturating_sub(1)).step_by(4) {
            let pid = u16::from_le_bytes([r[at], r[at + 1]]);
            if pid == 0 {
                break;
            }
            out.push(pid);
        }
    }
    out
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Rog;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Rog {
        /// pid -> (Omni-channel devices already awake once).
        awake: Mutex<std::collections::HashSet<u16>>,
        known: Mutex<BTreeMap<String, Device>>,
    }

    fn collections(vid: u16, pid: u16) -> Vec<hidapi::DeviceInfo> {
        let mut cols: Vec<_> = enumerate().into_iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid && d.usage_page() >= 0xFF00).collect();
        cols.sort_by_key(|d| d.interface_number());
        cols
    }

    impl BatteryProvider for Rog {
        fn name(&self) -> &'static str {
            "rog"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut awake = self.awake.lock().unwrap();
            let mut out = Vec::new();
            let mut names: Vec<String> = Vec::new();
            // Pelta headset.
            for d in enumerate() {
                if d.vendor_id() == VID && d.product_id() == PELTA_PID && d.interface_number() == 3 && d.usage_page() == 0xFF00 && d.usage() == 1 {
                    let path = d.path().to_string_lossy().into_owned();
                    let Some(io) = open(&path) else { continue };
                    if poll_pelta(&io).is_some() {
                        let dev = Device {
                            id: "rog:1b84:hs".into(),
                            name: "ROG Pelta".into(),
                            vendor: "ASUS ROG".into(),
                            model: Some("ROG Pelta".into()),
                            kind: DeviceKind::Headset,
                            provider: "rog",
                            locator: format!("{path}\tpelta"),
                        };
                        known.insert(dev.id.clone(), dev.clone());
                        out.push(dev);
                    }
                    break;
                }
            }
            // Omni receiver: list paired devices, read each Omni-channel entry.
            let omni_cols = collections(VID, OMNI_PID);
            if !omni_cols.is_empty() {
                let path = omni_cols[0].path().to_string_lossy().into_owned();
                if let Some(io) = open(&path) {
                    for pid in omni_pids(&io) {
                        // Only the Omni-channel (reportId != 0) entry of that model.
                        let Some(m) = MODELS.iter().find(|m| m.pid == pid && m.report_id != 0) else { continue };
                        let found = collections(VID, OMNI_PID)
                            .into_iter()
                            .filter(|d| d.interface_number() == 2)
                            .find_map(|d| {
                                let p = d.path().to_string_lossy().into_owned();
                                let io = open(&p)?;
                                query_model(&io, m).map(|s| (p, s))
                            });
                        if let Some((p, s)) = found {
                            if s.link == Link::Connected {
                                awake.insert(pid);
                            } else if !awake.contains(&pid) {
                                continue; // receiver channel answers "asleep" with nothing paired too
                            }
                            let key = format!("rog:{pid:04x}:omni");
                            if names.contains(&m.name.to_string()) {
                                continue;
                            }
                            names.push(m.name.to_string());
                            let dev = Device {
                                id: key,
                                name: m.name.into(),
                                vendor: "ASUS ROG".into(),
                                model: Some(m.name.into()),
                                kind: if m.keyboard { DeviceKind::Keyboard } else { DeviceKind::Mouse },
                                provider: "rog",
                                locator: format!("{p}\tomni\t{pid:04x}"),
                            };
                            known.insert(dev.id.clone(), dev.clone());
                            out.push(dev);
                        }
                    }
                }
            }
            // Direct devices (receiver or cable) by model table.
            let mut pids: Vec<u16> = Vec::new();
            for d in enumerate() {
                if d.vendor_id() == VID && MODELS.iter().any(|m| m.pid == d.product_id()) && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for pid in pids {
                let mut devs_for_pid = Vec::new();
                for m in MODELS.iter().filter(|m| m.pid == pid) {
                    let found = collections(VID, pid)
                        .into_iter()
                        .filter(|d| m.iface < 0 || d.interface_number() == m.iface)
                        .find_map(|d| {
                            let p = d.path().to_string_lossy().into_owned();
                            let io = open(&p)?;
                            query_model(&io, m).map(|s| (p, s))
                        });
                    let Some((path, s)) = found else { continue };
                    if m.report_id != 0 {
                        if s.link == Link::Connected {
                            awake.insert(pid);
                        } else if !awake.contains(&pid) {
                            continue;
                        }
                    }
                    let slot = if m.keyboard { "kb" } else { "ms" };
                    let dev = Device {
                        id: format!("rog:{pid:04x}:{slot}"),
                        name: m.name.into(),
                        vendor: "ASUS ROG".into(),
                        model: Some(m.name.into()),
                        kind: if m.keyboard { DeviceKind::Keyboard } else { DeviceKind::Mouse },
                        provider: "rog",
                        locator: format!("{path}\t{pid:04x}\t{slot}"),
                    };
                    devs_for_pid.push(dev);
                    break; // first answering entry per kind is enough; both kinds may appear
                }
                // A pid can appear as both keyboard and mouse entries? No: kind is per pid.
                for dev in devs_for_pid {
                    if names.contains(&dev.name) {
                        continue;
                    }
                    names.push(dev.name.clone());
                    known.insert(dev.id.clone(), dev.clone());
                    out.push(dev);
                }
            }
            known.retain(|_, d| out.iter().any(|o| o.id == d.id) || d.provider == "rog");
            // Keep previously seen devices whose collections went silent.
            let live: std::collections::HashSet<String> = out.iter().map(|d| d.id.clone()).collect();
            for d in known.values() {
                if !live.contains(&d.id) {
                    out.push(d.clone());
                }
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let fields: Vec<&str> = device.locator.split('\t').collect();
            let Some(&path) = fields.first() else { return BatteryStatus::GONE };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            if fields.contains(&"pelta") {
                return poll_pelta(&io).unwrap_or(BatteryStatus::ASLEEP);
            }
            let keyboard = device.kind == DeviceKind::Keyboard;
            let omni = fields.contains(&"omni");
            // pid is the field after "omni" or the first hex field.
            let pid = fields
                .iter()
                .skip(1)
                .filter_map(|f| u16::from_str_radix(f, 16).ok())
                .next()
                .unwrap_or(0);
            let m = MODELS
                .iter()
                .find(|m| m.pid == pid && m.keyboard == keyboard && (!omni || m.report_id != 0))
                .or_else(|| MODELS.iter().find(|m| m.pid == pid && m.keyboard == keyboard));
            let Some(m) = m else { return BatteryStatus::GONE };
            query_model(&io, m).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn kb_reply(offset: usize, pct: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; 16];
        r[1] = 0x12;
        r[2] = 0x01;
        r[offset] = pct;
        r[9] = charging;
        r
    }

    #[test]
    fn keyboard_battery() {
        let m = &MODELS[0]; // Azoth Omni entry
        let io = FakeHid::new(move |_| vec![kb_reply(m.offset, 77, 1)]);
        let s = query_model(&io, m).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(77), Charge::Charging, Link::Connected));
    }

    #[test]
    fn mouse_scaled() {
        let m = MODELS.iter().find(|m| m.pid == 0x18E5).unwrap(); // Chakram x25
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 16];
            r[1] = 0x12;
            r[2] = 0x07;
            r[5] = 3; // 3*25 = 75
            r[10] = 0;
            vec![r]
        });
        let s = query_model(&io, m).unwrap();
        assert_eq!(s.level, Some(75));
    }

    #[test]
    fn ff_aa_is_asleep() {
        let m = &MODELS[0];
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 16];
            r[1] = 0xFF;
            r[2] = 0xAA;
            vec![r]
        });
        assert_eq!(query_model(&io, m), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn tables() {
        assert!(supported(VID, 0x1A85));
        assert!(supported(VID, OMNI_PID));
        assert!(supported(VID, PELTA_PID));
        assert!(!supported(VID, 0x1234));
        assert!(!supported(0x1234, 0x1A85));
    }
}
