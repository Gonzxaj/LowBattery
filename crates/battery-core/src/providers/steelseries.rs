//! SteelSeries wireless mice and Arctis headsets over vendor HID reports.
//! Mouse commands follow rivalcfg (WTFPL). Headset byte layouts are protocol
//! facts documented by HeadsetControl (GPL-3; no code taken from it).
//! Read indexes assume hidapi semantics: report id 0 is stripped on read.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind};

pub const VID: u16 = 0x1038;
const TIMEOUT_MS: u64 = 500;

pub struct Model {
    pub pids: &'static [u16],
    pub name: &'static str,
    pub kind: DeviceKind,
    pub interface: i32,
    /// Collection to use on that interface; None = any vendor page (>= 0xFF00).
    pub usage_page: Option<u16>,
    /// Output report, report id first. Zero-padded to `pad`.
    pub request: &'static [u8],
    pub pad: usize,
    pub min_reply: usize,
    /// First byte a genuine reply starts with; other input reports are skipped.
    pub reply_prefix: Option<u8>,
    pub decode: fn(&[u8]) -> BatteryStatus,
}

/// Aerox / Prime: byte 1 = charging bit 7 | (level/5 + 1). Out of range = mouse off.
fn aerox(d: &[u8]) -> BatteryStatus {
    let b = d[1];
    let level = ((b & 0x7F) as i16 - 1) * 5;
    if !(0..=100).contains(&level) {
        return BatteryStatus::ASLEEP;
    }
    BatteryStatus::connected(level as u8, if b & 0x80 != 0 { Charge::Charging } else { Charge::Discharging })
}

/// Rival 3 Wireless: [level 0..100, _, charging]
fn rival3(d: &[u8]) -> BatteryStatus {
    BatteryStatus::connected(d[0], if d[2] != 0 { Charge::Charging } else { Charge::Discharging })
}

/// Arctis 7 / Pro 2019: byte 2 = level 0..100, no charge info.
fn arctis7(d: &[u8]) -> BatteryStatus {
    BatteryStatus::connected(d[2], Charge::Unknown)
}

/// Arctis 7P / 7X / 1: byte 2 == 1 offline, byte 3 = level.
fn arctis7p(d: &[u8]) -> BatteryStatus {
    if d[2] == 0x01 {
        return BatteryStatus::ASLEEP;
    }
    BatteryStatus::connected(d[3], Charge::Unknown)
}

/// Arctis 7+ family: byte 1 == 1 offline, byte 2 = level 0..4, byte 3 == 1 charging.
fn arctis7plus(d: &[u8]) -> BatteryStatus {
    if d[1] == 0x01 {
        return BatteryStatus::ASLEEP;
    }
    BatteryStatus::connected(d[2].min(4) * 25, if d[3] == 0x01 { Charge::Charging } else { Charge::Discharging })
}

/// Nova 7: byte 3 = 0 offline, 1|2 charging; byte 2 = level (0..4 on older firmware).
fn nova7(d: &[u8], steps: bool) -> BatteryStatus {
    let charge = match d[3] {
        0x00 => return BatteryStatus::ASLEEP,
        0x01 | 0x02 => Charge::Charging,
        _ => Charge::Discharging,
    };
    BatteryStatus::connected(if steps { d[2].min(4) * 25 } else { d[2] }, charge)
}

/// Nova 5 / 5X: reply [B0, state, _, level %, charging]; state 0x02 = headset off.
fn nova5(d: &[u8]) -> BatteryStatus {
    if d[1] == 0x02 {
        return BatteryStatus::ASLEEP;
    }
    BatteryStatus::connected(d[3], if d[4] == 0x01 { Charge::Charging } else { Charge::Discharging })
}

/// Nova Pro Wireless base station: byte 6 = level 0..8, byte 15 = state.
fn nova_pro(d: &[u8]) -> BatteryStatus {
    let charge = match d[15] {
        0x01 => return BatteryStatus::ASLEEP,
        0x02 => Charge::Charging,
        _ => Charge::Discharging,
    };
    BatteryStatus::connected((d[6].min(8) as u16 * 100 / 8) as u8, charge)
}

const MOUSE_PAD: usize = 2;

pub const MODELS: &[Model] = &[
    // --- mice (rivalcfg), interface 3, report id 0
    Model { pids: &[0x1838, 0x1878], name: "Aerox 3 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0xD2], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x183A, 0x187A], name: "Aerox 3 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0x92], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x1852, 0x185C, 0x1860], name: "Aerox 5 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0xD2], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x1854, 0x185E, 0x1862], name: "Aerox 5 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0x92], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x1858, 0x1874], name: "Aerox 9 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0xD2], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x185A, 0x1876], name: "Aerox 9 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0x92], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x1840, 0x1848], name: "Prime Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0xD2], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x1842, 0x184A], name: "Prime Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0x92], pad: MOUSE_PAD, min_reply: 2, reply_prefix: None, decode: aerox },
    Model { pids: &[0x1830, 0x1872], name: "Rival 3 Wireless", kind: DeviceKind::Mouse, interface: 3, usage_page: None, request: &[0x00, 0xAA, 0x01], pad: 3, min_reply: 3, reply_prefix: None, decode: rival3 },
    // --- headsets
    Model { pids: &[0x1260, 0x12AD, 0x1252, 0x1280], name: "Arctis 7", kind: DeviceKind::Headset, interface: 5, usage_page: Some(0xFF43), request: &[0x06, 0x18], pad: 2, min_reply: 3, reply_prefix: None, decode: arctis7 },
    Model { pids: &[0x12D5, 0x12D7, 0x12B3, 0x12B6], name: "Arctis 7P", kind: DeviceKind::Headset, interface: 3, usage_page: Some(0xFF43), request: &[0x06, 0x12], pad: 2, min_reply: 4, reply_prefix: None, decode: arctis7p },
    Model { pids: &[0x220E, 0x2212, 0x2216, 0x2236], name: "Arctis 7+", kind: DeviceKind::Headset, interface: 3, usage_page: Some(0xFFC0), request: &[0x00, 0xB0], pad: 2, min_reply: 4, reply_prefix: None, decode: arctis7plus },
    Model { pids: &[0x2202, 0x2206, 0x220A, 0x223A, 0x227A, 0x22A4], name: "Arctis Nova 7", kind: DeviceKind::Headset, interface: 3, usage_page: Some(0xFFC0), request: &[0x00, 0xB0], pad: 2, min_reply: 4, reply_prefix: None, decode: |d| nova7(d, true) },
    Model { pids: &[0x22A1, 0x227E, 0x2258, 0x229E, 0x22AD, 0x22A9, 0x22A5, 0x22A7], name: "Arctis Nova 7", kind: DeviceKind::Headset, interface: 3, usage_page: Some(0xFFC0), request: &[0x00, 0xB0], pad: 2, min_reply: 4, reply_prefix: None, decode: |d| nova7(d, false) },
    Model { pids: &[0x2232, 0x2253], name: "Arctis Nova 5", kind: DeviceKind::Headset, interface: 3, usage_page: Some(0xFFC0), request: &[0x00, 0xB0], pad: 2, min_reply: 16, reply_prefix: Some(0xB0), decode: nova5 },
    Model { pids: &[0x12E0, 0x12E5], name: "Arctis Nova Pro Wireless", kind: DeviceKind::Headset, interface: 4, usage_page: None, request: &[0x06, 0xB0], pad: 31, min_reply: 16, reply_prefix: None, decode: nova_pro },
];

pub fn model(pid: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.pids.contains(&pid))
}

/// None = no (valid) answer in time.
pub fn query(io: &dyn HidIo, m: &Model) -> Option<BatteryStatus> {
    let mut out = m.request.to_vec();
    out.resize(m.pad.max(out.len()), 0);
    io.write(&out).ok()?;
    read_matching(io, TIMEOUT_MS, |r| {
        let genuine = r.len() >= m.min_reply && m.reply_prefix.is_none_or(|p| r[0] == p);
        genuine.then(|| (m.decode)(r))
    })
    .ok()
    .flatten()
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::SteelSeries;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};

    #[derive(Default)]
    pub struct SteelSeries {
        /// Unsupported PIDs already reported, so the log says it once per run.
        reported: std::sync::Mutex<std::collections::HashSet<u16>>,
    }

    impl BatteryProvider for SteelSeries {
        fn name(&self) -> &'static str {
            "steelseries"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate().into_iter().filter(|d| d.vendor_id() == VID).collect();
            let mut out: Vec<Device> = Vec::new();
            let mut pids: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in pids {
                let Some(m) = model(pid) else {
                    if self.reported.lock().map(|mut r| r.insert(pid)).unwrap_or(false) {
                        let ifaces: Vec<String> = infos
                            .iter()
                            .filter(|d| d.product_id() == pid)
                            .map(|d| format!("if{} page {:04x}/{:04x}", d.interface_number(), d.usage_page(), d.usage()))
                            .collect();
                        log::info!("steelseries {pid:04x} ({}) not supported yet: {ifaces:?}", infos.iter().find(|d| d.product_id() == pid).and_then(|d| d.product_string()).unwrap_or("?"));
                    }
                    continue;
                };
                // Expected collection first, then any other vendor collection on that interface.
                let mut candidates: Vec<_> = infos
                    .iter()
                    .filter(|d| d.product_id() == pid && d.interface_number() == m.interface && d.usage_page() >= 0xFF00)
                    .collect();
                candidates.sort_by_key(|d| m.usage_page != Some(d.usage_page()));
                let mut chosen = None;
                for d in &candidates {
                    let path = d.path().to_string_lossy().into_owned();
                    let answer = open(&path).and_then(|io| query(&io, m));
                    log::debug!("steelseries {pid:04x} {} page {:04x}: {answer:?}", m.name, d.usage_page());
                    if answer.is_some() {
                        chosen = Some(path);
                        break;
                    }
                }
                let path = match (chosen, candidates.first()) {
                    (Some(p), _) => p,
                    // Dongle present but headset silent (off / out of range): show it asleep.
                    (None, Some(first)) => first.path().to_string_lossy().into_owned(),
                    (None, None) => {
                        log::info!("steelseries {pid:04x} {}: no vendor collection on interface {}", m.name, m.interface);
                        continue;
                    }
                };
                out.push(Device {
                    id: format!("steelseries:{pid:04x}"),
                    name: format!("SteelSeries {}", m.name),
                    vendor: "SteelSeries".into(),
                    model: Some(m.name.into()),
                    kind: m.kind,
                    provider: "steelseries",
                    locator: format!("{pid}\t{path}"),
                });
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let Some((pid, path)) = device.locator.split_once('\t') else { return BatteryStatus::GONE };
            let (Some(m), Some(io)) = (pid.parse().ok().and_then(model), open(path)) else {
                return BatteryStatus::GONE;
            };
            query(&io, m).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;
    use crate::Link;

    #[test]
    fn aerox_wireless_exchange() {
        let m = model(0x1838).unwrap();
        // 0x80 | (65/5 + 1) -> 65 %, charging
        let io = FakeHid::new(|req| if req == [0x00, 0xD2] { vec![vec![0xD2, 0x80 | 14]] } else { vec![] });
        let s = query(&io, m).unwrap();
        assert_eq!((s.level, s.charge), (Some(65), Charge::Charging));
        // Out-of-range byte = mouse switched off
        assert_eq!(aerox(&[0, 0x7F]).link, Link::Asleep);
    }

    #[test]
    fn headset_decoders() {
        assert_eq!(nova7(&[0xB0, 0, 3, 0x03], true).level, Some(75));
        assert_eq!(nova7(&[0xB0, 0, 80, 0x01], false).charge, Charge::Charging);
        assert_eq!(nova7(&[0xB0, 0, 80, 0x00], false).link, Link::Asleep);
        let mut d = [0u8; 16];
        d[6] = 4;
        d[15] = 0x08;
        assert_eq!(nova_pro(&d).level, Some(50));
        assert_eq!(arctis7p(&[0, 0, 1, 50]).link, Link::Asleep);
        assert_eq!(arctis7plus(&[0, 0, 2, 1]).level, Some(50));
    }

    #[test]
    fn nova5_exchange_skips_unrelated_reports() {
        let m = model(0x2232).unwrap();
        let io = FakeHid::new(|req| {
            if req != [0x00, 0xB0] {
                return vec![];
            }
            let mut status = vec![0xB0, 0x03, 0x00, 64, 0x01];
            status.resize(16, 0);
            // A chatmix/volume report arrives first and must be ignored.
            let mut other = vec![0x45, 0x10, 0x20];
            other.resize(16, 0);
            vec![other, status]
        });
        let s = query(&io, m).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(64), Charge::Charging, Link::Connected));
        let mut off = vec![0xB0, 0x02, 0, 0, 0];
        off.resize(16, 0);
        assert_eq!(nova5(&off).link, Link::Asleep);
    }

    #[test]
    fn silent_device_is_none() {
        let io = FakeHid::new(|_| vec![]);
        assert!(query(&io, model(0x2202).unwrap()).is_none());
    }

    #[test]
    fn pids_are_unique() {
        let mut all: Vec<u16> = MODELS.iter().flat_map(|m| m.pids.iter().copied()).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n);
    }
}
