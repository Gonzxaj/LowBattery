//! HyperX/HP and Kingston wireless headsets: vendor HID report protocols.
//! Protocol facts per the upstream C++ monitor's research notes (Cloud III
//! verified on hardware; the rest from HyperXWM, HyperHeadset and
//! HyperBatteryMonitor); reimplemented here, no code taken.
//!
//! hidapi does not expose report lengths, so each model lists candidate
//! usage pages and every collection is tried until one answers — the reply
//! matchers are strict enough that a wrong collection simply times out.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind};

const TIMEOUT_MS: u64 = 500;
/// Extra output-report sizes retried when the bare request gets no answer.
const PADS: [usize; 2] = [64, 32];

/// Ask `req` and return the first reply `accept` takes. Ok(None) = silence.
fn ask<T>(io: &dyn HidIo, req: &[u8], accept: impl FnMut(&[u8]) -> Option<T>) -> Option<T> {
    query(io, req, &PADS, TIMEOUT_MS, accept).ok().flatten()
}

fn connected(level: Option<u8>, charge: Charge) -> BatteryStatus {
    BatteryStatus { level, charge, link: crate::Link::Connected }
}

// --- Cloud III Wireless / Cloud II Core Wireless ---------------------------------
// report 66 <cmd>; 82 -> [2] connected, 8A -> [2] charging, 89 -> [2..3] mV, [4] %.
fn cloud3(io: &dyn HidIo) -> Option<BatteryStatus> {
    let r = ask(io, &[0x66, 0x82], |r| (r.len() >= 3 && r[0] == 0x66 && r[1] == 0x82).then_some(r[2]))?;
    if r == 0 {
        return Some(BatteryStatus::ASLEEP);
    }
    let charge = match ask(io, &[0x66, 0x8A], |r| (r.len() >= 3 && r[0] == 0x66 && r[1] == 0x8A).then_some(r[2])) {
        Some(1) => Charge::Charging,
        Some(_) => Charge::Discharging,
        None => Charge::Unknown,
    };
    let level = ask(io, &[0x66, 0x89], |r| {
        (r.len() >= 5 && r[0] == 0x66 && r[1] == 0x89 && r[4] <= 100).then_some(r[4])
    });
    Some(connected(level, charge))
}

// --- Cloud II Wireless (HP) --------------------------------------------------------
// report 06 FF BB <cmd>; 02 -> [7] %, 03 -> [4] charging.
fn cloud2(io: &dyn HidIo) -> Option<BatteryStatus> {
    let level = ask(io, &[0x06, 0xFF, 0xBB, 0x02], |r| {
        (r.len() >= 8 && r[0] == 0x06 && r[1] == 0xFF && r[2] == 0xBB && r[3] == 0x02 && r[7] <= 100).then_some(r[7])
    })?;
    let charge = match ask(io, &[0x06, 0xFF, 0xBB, 0x03], |r| {
        (r.len() >= 8 && r[0] == 0x06 && r[1] == 0xFF && r[2] == 0xBB && r[3] == 0x03).then_some(r[4] == 1)
    }) {
        Some(true) => Charge::Charging,
        Some(false) => Charge::Discharging,
        None => Charge::Unknown,
    };
    Some(connected(Some(level), charge))
}

// --- Cloud II Wireless (Kingston 0951:1718) ---------------------------------------
// fixed 62-byte frame, reply 0B 00 BB <cmd>.
fn cloud2_kingston(io: &dyn HidIo) -> Option<BatteryStatus> {
    let level = ask(io, &[0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00, 0xBB, 0x02, 0x00], |r| {
        (r.len() >= 8 && r[0] == 0x0B && r[2] == 0xBB && r[3] == 0x02 && r[7] <= 100).then_some(r[7])
    })?;
    let charge = match ask(io, &[0x06, 0x00, 0x02, 0x00, 0x9A, 0x00, 0x00, 0x68, 0x4A, 0x8E, 0x0A, 0x00, 0x00, 0x00, 0xBB, 0x03, 0x00], |r| {
        (r.len() >= 8 && r[0] == 0x0B && r[2] == 0xBB && r[3] == 0x03).then_some(r[4] == 1)
    }) {
        Some(true) => Charge::Charging,
        Some(false) => Charge::Discharging,
        None => Charge::Unknown,
    };
    Some(connected(Some(level), charge))
}

// --- Cloud Alpha Wireless ---------------------------------------------------------
// 21 BB <cmd>; 03 -> [3]==1 offline, 0C -> [3]==1 charging, 0B -> [3] %.
fn cloud_alpha(io: &dyn HidIo) -> Option<BatteryStatus> {
    let state = ask(io, &[0x21, 0xBB, 0x03], |r| {
        (r.len() >= 4 && r[0] == 0x21 && r[2] == 0x03).then_some(r[3])
    })?;
    if state == 0x01 {
        return Some(BatteryStatus::ASLEEP);
    }
    let charge = match ask(io, &[0x21, 0xBB, 0x0C], |r| {
        (r.len() >= 4 && r[0] == 0x21 && r[2] == 0x0C).then_some(r[3] == 0x01)
    }) {
        Some(true) => Charge::Charging,
        Some(false) => Charge::Discharging,
        None => Charge::Unknown,
    };
    let level = ask(io, &[0x21, 0xBB, 0x0B], |r| {
        (r.len() >= 4 && r[0] == 0x21 && r[2] == 0x0B && r[3] <= 100).then_some(r[3])
    });
    Some(connected(level, charge))
}

// --- Cloud Alpha 2 Wireless -------------------------------------------------------
// [cmd][sub]; reply [cmd+1][sub]...; 52 01 -> [2] connected, 50 02 -> [2] %, [3] charging.
fn cloud_alpha2(io: &dyn HidIo) -> Option<BatteryStatus> {
    let state = ask(io, &[0x52, 0x01], |r| {
        (r.len() >= 8 && r[0] == 0x53 && r[1] == 0x01).then_some(r[2])
    })?;
    if state == 0 {
        return Some(BatteryStatus::ASLEEP);
    }
    let (level, charge) = match ask(io, &[0x50, 0x02], |r| {
        (r.len() >= 8 && r[0] == 0x51 && r[1] == 0x02 && r[2] <= 100).then_some((r[2], r[3] == 0x01))
    }) {
        Some((l, true)) => (Some(l), Charge::Charging),
        Some((l, false)) => (Some(l), Charge::Discharging),
        None => (None, Charge::Unknown),
    };
    Some(connected(level, charge))
}

// --- Cloud Flight / Cloud Flight S -----------------------------------------------
// 21 FF 05 -> [3..4] mV (>0x100B means charging); percent from a fitted curve.
fn cloud_flight(io: &dyn HidIo) -> Option<BatteryStatus> {
    let mv = ask(io, &[0x21, 0xFF, 0x05], |r| (r.len() >= 5).then(|| ((r[3] as u16) << 8) | r[4] as u16))?;
    let charging = mv > 0x100B;
    let level = if charging {
        None
    } else if mv <= 3648 {
        Some((mv as u32 / 800).min(100) as u8) // 0.00125 * mv
    } else if mv > 3975 {
        Some(100)
    } else {
        // Quartic fit of the discharge curve (HyperBatteryMonitor).
        let v = mv as f64;
        let pct = 0.00000002547505 * v.powi(4) - 0.0003900299 * v.powi(3) + 2.238321 * v * v - 5706.256 * v + 5452299.0;
        Some(pct.round().clamp(0.0, 100.0) as u8)
    };
    Some(connected(level, if charging { Charge::Charging } else { Charge::Discharging }))
}

// --- Cloud III S Wireless ---------------------------------------------------------
// report 0C [02 03 01 00 <cmd>]; reply [5] = cmd, [6] = value (FF = invalid).
// 02 -> [6] == 2 connected, 06 -> [6] %, 48 -> [6] 0 discharging / 1 charging / 2 full.
fn cloud3s(io: &dyn HidIo) -> Option<BatteryStatus> {
    let q = |cmd: u8| {
        ask(io, &[0x0C, 0x02, 0x03, 0x01, 0x00, cmd], |r| {
            (r.len() >= 7 && r[0] == 0x0C && r[5] == cmd && r[6] != 0xFF).then_some(r[6])
        })
    };
    if q(0x02)? != 2 {
        return Some(BatteryStatus::ASLEEP);
    }
    let level = q(0x06).filter(|&v| v <= 100);
    let charge = match q(0x48) {
        Some(1) => Charge::Charging,
        Some(2) => Charge::Full,
        Some(_) => Charge::Discharging,
        None => Charge::Unknown,
    };
    Some(connected(level, charge))
}

// --- Cloud Stinger Core Wireless ---------------------------------------------------
// Feature report FF: SetFeature(command), wait, GetFeature(FF).
//   connection reply [11] == 1 means the headset is on.
//   battery reply [9] = 03 discharging / 05 charging / 06 full, [11..12] = mV LE.
// The 0A93 (PlayStation) receiver can hand back the previous report, so it is
// re-read until the reply header looks right.
const STINGER_CONNECTION: [u8; 20] = [
    0xFF, 0x07, 0x00, 0xFD, 0x04, 0x00, 0x00, 0x02, 0x82, 0x03, 0x05, 0x29, 0x0F, 0x00, 0x00, 0x03, 0x68, 0x10, 0x00, 0x00,
];
const STINGER_BATTERY: [u8; 12] = [
    0xFF, 0x07, 0x00, 0xFD, 0x04, 0x0C, 0xF1, 0x02, 0x01, 0x04, 0xF0, 0x0C,
];
const STINGER_SIZES: [usize; 5] = [32, 64, 65, 16, 20];

fn stinger_fresh(r: &[u8]) -> bool {
    !(r.len() > 2 && r[1] == 0x01 && r[2] == 0x00)
}
fn stinger_battery(r: &[u8]) -> bool {
    r.len() >= 13 && r[0] == 0xFF && r[1] == 0x12 && r[2] == 0x05
}

/// Set the feature report (padded to a plausible size) then read it back.
/// `good` decides whether a reply is the wanted one; the PS receiver needs
/// several reads because it can return the previous report first.
fn stinger_exchange(io: &dyn HidIo, cmd: &[u8], ps: bool, good: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    for size in STINGER_SIZES {
        let req = crate::hid::pad_report(cmd, size);
        if io.send_feature(&req).is_err() {
            continue;
        }
        for _ in 0..if ps { 10 } else { 1 } {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let mut buf = vec![0u8; size];
            buf[0] = 0xFF;
            let Ok(n) = io.get_feature(&mut buf) else { break };
            let r = &buf[..n.min(size)];
            if r.len() >= 13 && (!ps || good(r)) {
                return Some(r.to_vec());
            }
        }
    }
    None
}

/// mV -> % in 10% steps (charging and discharging curves).
fn stinger_percent(mv: i32, charging: bool) -> u8 {
    const PCT: [i32; 11] = [100, 90, 80, 70, 60, 50, 40, 30, 20, 10, 0];
    const CHARGING_MV: [i32; 11] = [4130, 4090, 4050, 4000, 3967, 3946, 3930, 3916, 3871, 3831, 3400];
    const DISCHARGING_MV: [i32; 11] = [4121, 4042, 3954, 3905, 3838, 3785, 3763, 3738, 3700, 3651, 3517];
    let table = if charging { &CHARGING_MV } else { &DISCHARGING_MV };
    // Tables are sorted by descending voltage; interpolate between points.
    if mv >= table[0] {
        return PCT[0] as u8;
    }
    for i in 1..table.len() {
        if mv >= table[i] {
            let span = table[i - 1] - table[i];
            let t = if span > 0 { (mv - table[i]) as f64 / span as f64 } else { 0.0 };
            return (PCT[i] as f64 + t * (PCT[i - 1] - PCT[i]) as f64).round().clamp(0.0, 100.0) as u8;
        }
    }
    *PCT.last().unwrap() as u8
}

fn stinger_core(io: &dyn HidIo, ps: bool) -> Option<BatteryStatus> {
    // The connection reply's header isn't documented; after stale-looking
    // reads the last one is used on the PS receiver.
    let r = stinger_exchange(io, &STINGER_CONNECTION, ps, stinger_fresh)
        .or_else(|| ps.then(|| stinger_exchange(io, &STINGER_CONNECTION, ps, |_| true)).flatten())?;
    if r.len() <= 11 || r[11] != 0x01 {
        return Some(BatteryStatus::ASLEEP);
    }
    let Some(rb) = stinger_exchange(io, &STINGER_BATTERY, ps, stinger_battery) else {
        return Some(connected(None, Charge::Unknown));
    };
    let code = rb[9];
    if !matches!(code, 0x03 | 0x05 | 0x06) {
        return Some(connected(None, Charge::Unknown));
    }
    let mv = rb[11] as i32 | ((rb[12] as i32) << 8);
    let charging = code == 0x05;
    let level = if code == 0x06 {
        Some(100)
    } else {
        Some(stinger_percent(mv, charging))
    };
    let charge = match code {
        0x05 => Charge::Charging,
        0x06 => Charge::Full,
        _ => Charge::Discharging,
    };
    Some(connected(level, charge))
}

pub struct Model {
    pub vid: u16,
    pub pids: &'static [u16],
    pub name: &'static str,
    /// Candidate usage pages, most likely first; empty = vendor pages then all.
    pub pages: &'static [u16],
    pub poll: fn(&dyn HidIo) -> Option<BatteryStatus>,
}

pub const MODELS: &[Model] = &[
    Model { vid: 0x03F0, pids: &[0x05B7, 0x0C9D], name: "HyperX Cloud III Wireless", pages: &[], poll: cloud3 },
    Model { vid: 0x03F0, pids: &[0x069F, 0x0995], name: "HyperX Cloud II Core Wireless", pages: &[], poll: cloud3 },
    Model { vid: 0x03F0, pids: &[0x06BE, 0x02CC], name: "HyperX Cloud III S Wireless", pages: &[0x01C0], poll: cloud3s },
    Model { vid: 0x03F0, pids: &[0x0696, 0x018B, 0x0D93], name: "HyperX Cloud II Wireless", pages: &[0xFF90], poll: cloud2 },
    Model { vid: 0x0951, pids: &[0x1718], name: "HyperX Cloud II Wireless", pages: &[], poll: cloud2_kingston },
    Model { vid: 0x03F0, pids: &[0x098D], name: "HyperX Cloud Alpha Wireless", pages: &[], poll: cloud_alpha },
    Model { vid: 0x03F0, pids: &[0x08BE], name: "HyperX Cloud Alpha 2 Wireless", pages: &[0xFF13], poll: cloud_alpha2 },
    Model { vid: 0x0951, pids: &[0x16C4, 0x1723, 0x1749], name: "HyperX Cloud Flight", pages: &[], poll: cloud_flight },
    Model { vid: 0x03F0, pids: &[0x0E90], name: "HyperX Cloud Flight", pages: &[], poll: cloud_flight },
    // 0x0A93 is the PlayStation receiver: it can return the previous report,
    // so replies are re-read until the header looks right.
    Model { vid: 0x03F0, pids: &[0x0A93], name: "HyperX Cloud Stinger Core Wireless", pages: &[0xFF00], poll: |io| stinger_core(io, true) },
    Model { vid: 0x0951, pids: &[0x170B], name: "HyperX Cloud Stinger Core Wireless", pages: &[0xFF00], poll: |io| stinger_core(io, false) },
];

pub fn model(vid: u16, pid: u16) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.vid == vid && m.pids.contains(&pid))
}

pub fn supported(vid: u16, pid: u16) -> bool {
    model(vid, pid).is_some()
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::HyperX;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Dongle plugged in but headset silent: keep the card as asleep.
    #[derive(Default)]
    pub struct HyperX {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for HyperX {
        fn name(&self) -> &'static str {
            "hyperx"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate().into_iter().filter(|d| supported(d.vendor_id(), d.product_id())).collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut keys: Vec<(u16, u16)> = infos.iter().map(|d| (d.vendor_id(), d.product_id())).collect();
            keys.sort_unstable();
            keys.dedup();
            for &(vid, pid) in &keys {
                let Some(m) = model(vid, pid) else { continue };
                let mut cols: Vec<_> = infos.iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).collect();
                // Candidate usage pages first (in table order), then vendor pages, then the rest.
                cols.sort_by_key(|d| {
                    let page_rank = m.pages.iter().position(|&p| p == d.usage_page()).map(|i| i as i32).unwrap_or(1000);
                    (page_rank, if d.usage_page() >= 0xFF00 { 0 } else { 1 }, d.interface_number())
                });
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = (m.poll)(&io);
                    log::debug!("hyperx {vid:04x}:{pid:04x} if{} {:04x}/{:04x}: {s:?}", d.interface_number(), d.usage_page(), d.usage());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let dev = Device {
                            id: format!("hyperx:{vid:04x}:{pid:04x}"),
                            name: m.name.into(),
                            vendor: "HyperX".into(),
                            model: Some(m.name.into()),
                            kind: DeviceKind::Headset,
                            provider: "hyperx",
                            locator: format!("{}\t{vid:04x}\t{pid:04x}", path),
                        };
                        known.insert(pid, dev.clone());
                        out.push(dev);
                    }
                    None => out.extend(known.get(&pid).cloned()),
                }
            }
            known.retain(|pid, _| keys.iter().any(|&(_, p)| p == *pid));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let Some(path) = parts.next() else { return BatteryStatus::GONE };
            let (Some(vid), Some(pid)) = (parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()), parts.next().and_then(|v| u16::from_str_radix(v, 16).ok())) else {
                return BatteryStatus::GONE;
            };
            let (Some(m), Some(io)) = (model(vid, pid), open(path)) else { return BatteryStatus::GONE };
            (m.poll)(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;
    use crate::Link;

    fn cloud3_device(connected: bool, charging: u8, level: u8) -> FakeHid {
        FakeHid::new(move |req| {
            let cmd = req.get(1).copied().unwrap_or(0);
            match (req.first(), cmd) {
                (Some(0x66), 0x82) => vec![vec![0x66, 0x82, u8::from(connected)]],
                (Some(0x66), 0x8A) => vec![vec![0x66, 0x8A, charging]],
                (Some(0x66), 0x89) => vec![vec![0x66, 0x89, 0, 0, level]],
                _ => vec![],
            }
        })
    }

    #[test]
    fn cloud3_reads_level_and_charge() {
        let s = cloud3(&cloud3_device(true, 1, 73)).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(73), Charge::Charging, Link::Connected));
        assert_eq!(cloud3(&cloud3_device(false, 0, 0)), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn cloud_alpha_flow() {
        let io = FakeHid::new(|req| match req {
            [0x21, 0xBB, 0x03] => vec![vec![0x21, 0xBB, 0x03, 0x00]],
            [0x21, 0xBB, 0x0C] => vec![vec![0x21, 0xBB, 0x0C, 0x01]],
            [0x21, 0xBB, 0x0B] => vec![vec![0x21, 0xBB, 0x0B, 64]],
            _ => vec![],
        });
        let s = cloud_alpha(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(64), Charge::Charging));
    }

    #[test]
    fn cloud_flight_curve() {
        let io = FakeHid::new(|_| vec![vec![0x21, 0xFF, 0x05, 0x0E, 0xAC]]); // 3756 mV
        let s = cloud_flight(&io).unwrap();
        assert!(s.level.is_some() && s.level.unwrap() > 20 && s.level.unwrap() < 60);
        // Above 0x100B = charging flag in the voltage field.
        let io = FakeHid::new(|_| vec![vec![0x21, 0xFF, 0x05, 0x10, 0x10]]);
        assert_eq!(cloud_flight(&io).unwrap().charge, Charge::Charging);
    }

    #[test]
    fn cloud3s_flow() {
        let io = FakeHid::new(|req| {
            let cmd = req.last().copied().unwrap_or(0);
            vec![vec![0x0C, 0x02, 0x03, 0x01, 0x00, cmd, match cmd {
                0x02 => 2,
                0x06 => 55,
                0x48 => 1,
                _ => 0xFF,
            }]]
        });
        let s = cloud3s(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(55), Charge::Charging));
    }

    #[test]
    fn stinger_core_flow() {
        let io = FakeHid::new(|req| {
            if req[0] != 0xFF {
                return vec![];
            }
            if req.get(5) == Some(&0x00) {
                // connection reply: [11] == 1 on
                let mut r = vec![0u8; 20];
                r[0] = 0xFF;
                r[11] = 1;
                return vec![r];
            }
            // battery reply: code 05 charging, 3900 mV
            let mut r = vec![0u8; 32];
            r[0] = 0xFF;
            r[1] = 0x12;
            r[2] = 0x05;
            r[9] = 0x05;
            r[11] = 0x3C;
            r[12] = 0x0F; // 3900
            vec![r]
        });
        let s = stinger_core(&io, false).unwrap();
        assert_eq!(s.link, Link::Connected);
        assert_eq!(s.charge, Charge::Charging);
        assert!(s.level.is_some());
    }

    #[test]
    fn stinger_off_is_asleep() {
        let io = FakeHid::new(|_| vec![{
            let mut r = vec![0u8; 20];
            r[0] = 0xFF;
            r[11] = 0;
            r
        }]);
        assert_eq!(stinger_core(&io, false), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn supported_tables() {
        assert!(supported(0x03F0, 0x05B7));
        assert!(supported(0x0951, 0x170B));
        assert!(!supported(0x03F0, 0x1234));
        assert!(MODELS.iter().all(|m| !m.pids.is_empty()));
    }
}
