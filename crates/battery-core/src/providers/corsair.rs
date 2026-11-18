//! Corsair wireless headsets, mice and keyboards (VID 1B1C).
//!  * Void / Void Pro / Void Elite / HS70 / HS80 family: output C9 64 ->
//!    [2] % (bit 7 = mic up), [4] status (0 offline, 1-2 ok, 4-5 charging).
//!  * "Bragi" devices (Virtuoso, Void V2, HS80 MAX, Slipstream receivers):
//!    GET property over vendor reports; battery is property 0F (0..1000),
//!    charge state 0x10, paired-device product id 0x12. Targets 09+ are
//!    devices paired to a receiver, 08 is the device/receiver itself.
//!    GET is read-only; devices are never switched into software mode.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind};

pub const VID: u16 = 0x1B1C;
const TIMEOUT_MS: u64 = 300;
const PADS: [usize; 2] = [64, 32];

const VOID_PIDS: &[u16] = &[
    0x1B1C, 0x1B27, 0x0A14, 0x0A16, 0x0A17, 0x0A1D, 0x0A1A, 0x1B2A, 0x1B23, 0x1B29, 0x0A55, 0x0A51, 0x0A52, 0x0A38,
    0x0A4F, 0x0A2B, 0x0A75, 0x0A56,
];
const BRAGI_PIDS: &[u16] = &[0x2A08, 0x2A02, 0x0A97, 0x0A64, 0x0A62, 0x0A3E, 0x0A3D, 0x2B00];

/// Paired devices seen behind a receiver, identified by reported product id.
const PAIRED_MODELS: &[(u16, &str, DeviceKind)] = &[(0x1B7E, "Corsair Dark Core RGB Pro SE", DeviceKind::Mouse)];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && (VOID_PIDS.contains(&pid) || BRAGI_PIDS.contains(&pid))
}

// --- Void family -----------------------------------------------------------------

fn void_poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    query(io, &[0xC9, 0x64], &PADS, TIMEOUT_MS, |r| {
        if r.len() < 5 || r[4] > 5 || (r[2] & 0x7F) > 100 {
            return None;
        }
        let st = r[4];
        if st == 0 {
            return Some(BatteryStatus::ASLEEP);
        }
        let charge = if st == 4 || st == 5 {
            Charge::Charging
        } else {
            Charge::Discharging
        };
        Some(BatteryStatus::connected(r[2] & 0x7F, charge))
    })
    .ok()
    .flatten()
}

// --- Bragi ------------------------------------------------------------------------

/// GET(target, property) -> u32 value. Three request framings exist in the
/// wild (prefixed unnumbered, prefixed numbered, plain); whichever answers
/// first wins. The reply is scanned for [source, 0x02, status=0, value LE]
/// at a small offset so a leading report id doesn't matter.
fn bragi_get(io: &dyn HidIo, target: u8, prop: u8) -> Option<u32> {
    let source = if target == 0x08 { 0x00 } else { 0x01 };
    let variants: [&[u8]; 3] = [
        &[0x00, 0x02, target, 0x02, prop],
        &[0x02, target, 0x02, prop],
        &[0x00, target, 0x02, prop],
    ];
    for req in variants {
        let found = query(io, req, &PADS, TIMEOUT_MS, |r| {
            for o in 0..=2usize {
                if r.len() >= o + 7 && r[o] == source && r[o + 1] == 0x02 && r[o + 2] == 0x00 {
                    let v = u32::from_le_bytes([r[o + 3], r[o + 4], r[o + 5], r[o + 6]]);
                    return Some(v);
                }
            }
            None
        });
        if let Ok(Some(v)) = found {
            return Some(v);
        }
    }
    None
}

/// Battery of one Bragi target. Ok(None) = target silent / absent.
fn bragi_battery(io: &dyn HidIo, target: u8) -> Option<BatteryStatus> {
    let mut level = bragi_get(io, target, 0x0F);
    for _ in 0..2 {
        if !level.is_some_and(|v| v == 0 || v > 1000) {
            break;
        }
        level = bragi_get(io, target, 0x0F);
    }
    let level = level.filter(|&v| v <= 1000)?;
    let charge = match bragi_get(io, target, 0x10) {
        Some(1) => Charge::Charging,
        Some(3) => Charge::Full,
        Some(_) => Charge::Discharging,
        None => Charge::Unknown,
    };
    Some(BatteryStatus::connected(((level + 5) / 10).min(100) as u8, charge))
}

pub struct BragiDevice {
    pub target: u8,
    pub name: String,
    pub kind: DeviceKind,
    pub status: BatteryStatus,
}

/// Everything a receiver (or wired device) reports: one entry per live target.
pub fn bragi_scan(io: &dyn HidIo, product: &str, self_pid: u16) -> Vec<BragiDevice> {
    let mut out = Vec::new();
    for target in [0x09u8, 0x0A, 0x0B, 0x08] {
        if target == 0x08 && !out.is_empty() {
            break; // the device itself only when nothing paired answered
        }
        let Some(status) = bragi_battery(io, target) else { continue };
        let mut name = if product.is_empty() { "Corsair wireless device".to_string() } else { product.to_string() };
        let mut kind = crate::guess_kind(&name);
        // Property 12 is the paired device's product id; Corsair headsets use 0Axx.
        if target != 0x08 {
            if let Some(dev_pid) = bragi_get(io, target, 0x12) {
                if let Some(&(_, n, k)) = PAIRED_MODELS.iter().find(|(p, _, _)| *p == dev_pid as u16) {
                    name = n.to_string();
                    kind = k;
                } else if (dev_pid >> 8) == 0x0A {
                    kind = DeviceKind::Headset;
                    if name.contains("Receiver") {
                        name = "Corsair wireless headset".into();
                    }
                }
            }
        }
        // Corsair headsets and their receivers use product ids 0Axx (the HS80
        // RGB receiver names itself "... Wireless Gaming Receiver").
        if kind == DeviceKind::Other && (self_pid >> 8) == 0x0A {
            kind = DeviceKind::Headset;
        }
        out.push(BragiDevice { target, name, kind, status });
    }
    out
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Corsair;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// key (vid,pid) -> devices last seen, kept as asleep while silent.
    #[derive(Default)]
    pub struct Corsair {
        known: Mutex<BTreeMap<(u16, u16), Vec<Device>>>,
    }

    fn product(d: &hidapi::DeviceInfo) -> String {
        d.product_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Corsair wireless device").to_string()
    }

    impl BatteryProvider for Corsair {
        fn name(&self) -> &'static str {
            "corsair"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate().iter().filter(|d| d.vendor_id() == VID).cloned().collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut keys: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            keys.sort_unstable();
            keys.dedup();
            for &pid in &keys {
                if VOID_PIDS.contains(&pid) {
                    // Preferred collections: FFC5 / FF00, then any vendor page.
                    let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == pid).collect();
                    cols.sort_by_key(|d| (!matches!(d.usage_page(), 0xFFC5 | 0xFF00), d.usage_page() < 0xFF00));
                    let found = cols.iter().find_map(|d| {
                        let io = open(&d.path().to_string_lossy())?;
                        void_poll(&io).map(|s| (d.path().to_string_lossy().into_owned(), s))
                    });
                    match found {
                        Some((path, _)) => {
                            let name = product(cols.iter().find(|d| d.path().to_string_lossy() == path).unwrap_or(&cols[0]));
                            let dev = Device {
                                id: format!("corsair:{pid:04x}"),
                                kind: DeviceKind::Headset,
                                name,
                                vendor: "Corsair".into(),
                                model: None,
                                provider: "corsair",
                                locator: format!("v\t{path}\t{pid:04x}\t00"),
                            };
                            known.insert((VID, pid), vec![dev.clone()]);
                            out.push(dev);
                        }
                        None => out.extend(known.get(&(VID, pid)).cloned().unwrap_or_default()),
                    }
                } else if BRAGI_PIDS.contains(&pid) {
                    let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == pid).collect();
                    cols.sort_by_key(|d| {
                        let rank = if d.usage_page() == 0xFF42 { 0 } else if d.usage_page() >= 0xFF00 { 1 } else { 2 };
                        (rank, d.interface_number())
                    });
                    let mut devices = Vec::new();
                    'outer: for d in &cols {
                        let Some(io) = open(&d.path().to_string_lossy()) else { continue };
                        let prod = product(d);
                        for dev in bragi_scan(&io, &prod, pid) {
                            devices.push(Device {
                                id: format!("corsair:{pid:04x}:{:02x}", dev.target),
                                kind: dev.kind,
                                name: dev.name,
                                vendor: "Corsair".into(),
                                model: None,
                                provider: "corsair",
                                locator: format!("b\t{}\t{pid:04x}\t{:02x}", d.path().to_string_lossy(), dev.target),
                            });
                        }
                        if !devices.is_empty() {
                            break 'outer;
                        }
                    }
                    if devices.is_empty() {
                        out.extend(known.get(&(VID, pid)).cloned().unwrap_or_default());
                    } else {
                        known.insert((VID, pid), devices.clone());
                        out.extend(devices);
                    }
                }
            }
            known.retain(|(v, p), _| *v == VID && keys.contains(p));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let (Some(kind), Some(path), Some(_pid), Some(target)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return BatteryStatus::GONE;
            };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            if kind == "v" {
                return void_poll(&io).unwrap_or(BatteryStatus::ASLEEP);
            }
            let Ok(target) = u8::from_str_radix(target, 16) else { return BatteryStatus::GONE };
            bragi_battery(&io, target).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;
    use crate::Link;

    /// Simulated Bragi device: answers GET for target 0x08 only.
    fn bragi() -> FakeHid {
        FakeHid::new(|req| {
            // Accept any of the three framings; reply [00, source, 02, 00, value LE].
            let (target, prop) = match req {
                [0x00, 0x02, t, 0x02, p, ..] | [0x02, t, 0x02, p, ..] => (*t, *p),
                [0x00, t, 0x02, p, ..] => (*t, *p),
                _ => return vec![],
            };
            if target != 0x08 {
                return vec![];
            }
            let source = 0x00u8;
            let value: u32 = match prop {
                0x0F => 730,  // 73 %
                0x10 => 0,    // not charging
                0x12 => 0x0A64,
                _ => return vec![],
            };
            let mut r = vec![0u8; 16];
            r[0] = 0x00;
            r[1] = source;
            r[2] = 0x02;
            r[3] = 0x00;
            r[4..8].copy_from_slice(&value.to_le_bytes());
            vec![r]
        })
    }

    #[test]
    fn bragi_get_and_battery() {
        let io = bragi();
        assert_eq!(bragi_get(&io, 0x08, 0x0F), Some(730));
        let s = bragi_battery(&io, 0x08).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(73), Charge::Discharging, Link::Connected)); // (730+5)/10
    }

    #[test]
    fn void_family() {
        let io = FakeHid::new(|_| vec![vec![0xC9, 0x64, 80, 0, 2, 0]]);
        let s = void_poll(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(80), Charge::Discharging));
        let io = FakeHid::new(|_| vec![vec![0xC9, 0x64, 0x85, 0, 4, 0]]);
        let s = void_poll(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(5), Charge::Charging)); // bit7 masked out
        let io = FakeHid::new(|_| vec![vec![0xC9, 0x64, 50, 0, 0, 0]]);
        assert_eq!(void_poll(&io), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn supported_tables() {
        assert!(supported(VID, 0x2B00));
        assert!(supported(VID, 0x1B1C));
        assert!(!supported(VID, 0x1234));
        assert!(!supported(0x1234, 0x2B00));
    }
}
