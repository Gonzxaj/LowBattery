//! Glorious, Hitscan, G-Wolves and WLMouse wireless mice using the same
//! supplier protocol (G-Wolves calls it "XVI"; its other models use the
//! Kysona protocol in kysona.rs instead):
//!   request: 65-byte feature report (id 0), bytes [3]=02 [4]=02 [6]=83
//!   reply:   [1]=status, [6]=83, [7]=charging (1), [8]=percent
//!   status A1 = awake, A4 = asleep, A0 = waking up, others = unreachable.
//! hidapi does not expose report lengths, so every collection of the known
//! VIDs is tried vendor-page first; the strict reply match filters the rest.
//! Protocol: github.com/ikajdan/glorious-indicator,
//! github.com/louis4craft/glorious-ctl, Hitscan's web configurator
//! (control.hitscan.com) and G-Wolves' (mouse.xyz, Config/env-models.json);
//! reimplemented here, no code taken.

use crate::hid::{feature_exchange, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

/// (vid, key prefix, reply delay ms)
const VENDORS: &[(u16, &str, u64)] = &[(0x258A, "glorious", 50), (0x3770, "hitscan", 200), (0x33E4, "gwolves", 100), (0x36A7, "wlmouse", 100)];

pub fn vendor(vid: u16) -> Option<&'static str> {
    VENDORS.iter().find(|v| v.0 == vid).map(|v| v.1)
}

pub fn supported(vid: u16, _pid: u16) -> bool {
    vendor(vid).is_some()
}

fn delay(vid: u16) -> u64 {
    VENDORS.iter().find(|v| v.0 == vid).map_or(100, |v| v.2)
}

/// Model name for known G-Wolves PIDs (mouse.xyz device list); dongles shared
/// by several models have no specific name.
pub fn gwolves_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x3808 | 0x3817 => "G-Wolves HTM Plus",
        0x6808 | 0x6817 => "G-Wolves HSK Pro 2.0",
        0x5608 | 0x5617 | 0x5618 | 0x5619 => "G-Wolves HTXU",
        0x3608 | 0x3617 | 0x3619 => "G-Wolves Fenrir Pro",
        0x3908 | 0x3917 => "G-Wolves VUK",
        0x7904 | 0x7913 => "G-Wolves HT-S2",
        0x3708 | 0x3717 => "G-Wolves Fenrir Max",
        0x5308 | 0x5317 => "G-Wolves HTS Ultra",
        0x7704 | 0x7713 => "G-Wolves HTR",
        0x3508 | 0x3517 => "G-Wolves Fenrir",
        0x7908 | 0x7917 => "G-Wolves HT-S2 Pro",
        0x5808 | 0x5817 => "G-Wolves HSK Pro",
        0x2708 | 0x2717 | 0x2719 => "G-Wolves HTX Mini",
        0x5408 | 0x5417 | 0x5418 | 0x5419 => "G-Wolves HTS Plus",
        0x5708 | 0x5717 => "G-Wolves HTX",
        0x5908 | 0x5917 => "G-Wolves HSK Plus",
        0x7204 | 0x7203 => "G-Wolves HSK Lite",
        0x7708 | 0x7717 => "G-Wolves HTR Pro",
        0x5804 | 0x5803 => "G-Wolves HSK Pro ACE",
        0x5404 | 0x5403 => "G-Wolves HTS Plus ACE",
        0x5704 | 0x5703 => "G-Wolves HTX ACE",
        0x5904 | 0x5903 => "G-Wolves HSK Plus ACE",
        0x4718 | 0x4719 => "G-Wolves Lycan",
        0x4219 => "G-Wolves WARG",
        0x5219 => "G-Wolves HTS Plus Pro",
        0x3519 => "G-Wolves Fenrir Asym",
        0x2619 => "G-Wolves HTX Mini Asym",
        0x1619 => "G-Wolves Fenrir Max Asym",
        0x2119 => "G-Wolves VUK Mini",
        _ => return None,
    })
}

/// Wired product ids: the mouse itself on the USB cable.
pub fn gwolves_cable(pid: u16) -> bool {
    matches!(
        pid,
        0x1619 | 0x2119 | 0x2619 | 0x2708 | 0x2719 | 0x3508 | 0x3519 | 0x3608 | 0x3619 | 0x3708 | 0x3808 | 0x3908
            | 0x4219 | 0x4718 | 0x4719 | 0x5219 | 0x5308 | 0x5404 | 0x5408 | 0x5418 | 0x5419 | 0x5608 | 0x5618 | 0x5619
            | 0x5704 | 0x5708 | 0x5804 | 0x5808 | 0x5904 | 0x5908 | 0x6808 | 0x7204 | 0x7704 | 0x7708 | 0x7904 | 0x7908
    )
}

fn glorious_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x2022 | 0x2011 => "Glorious Model O Wireless",
        0x2023 | 0x2012 => "Glorious Model D Wireless",
        0x2024 | 0x2013 => "Glorious Model O- Wireless",
        0x2025 | 0x2014 => "Glorious Model D- Wireless",
        0x2033 => "Glorious Model O 2 Wireless",
        0x2034 | 0x2036 | 0x201A => "Glorious Model D 2 PRO",
        _ => return None, // another brand on the same Sinowealth chip
    })
}

fn hitscan_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x0136 => "Hitscan Hyperlight X",
        0x0194 => "Hitscan Hyperlight N",
        _ => "Hitscan Hyperlight",
    })
}

fn hitscan_cable(pid: u16) -> bool {
    pid == 0x0136 || pid == 0x0194
}

/// Display name for a (vid,pid); None = use the product string.
pub fn model_name(vid: u16, pid: u16) -> Option<String> {
    match vid {
        0x258A => glorious_name(pid).map(str::to_string),
        0x3770 => Some(hitscan_name(pid).unwrap_or("Hitscan Hyperlight").to_string()),
        0x33E4 => Some(gwolves_name(pid).unwrap_or("G-Wolves wireless mouse").to_string()),
        0x36A7 => (pid == 0xA874).then(|| "WLMouse YING".to_string()),
        _ => None,
    }
}

/// True when `pid` is the device itself on USB cable (charging while plugged).
pub fn is_cable(vid: u16, pid: u16) -> bool {
    match vid {
        0x258A => matches!(pid, 0x2011 | 0x2012 | 0x2013 | 0x2014 | 0x201A),
        0x3770 => hitscan_cable(pid),
        0x33E4 => gwolves_cable(pid),
        _ => false,
    }
}

/// Parse a reply at either alignment (report id byte present or not).
/// Returns (status, charging, percent).
fn parse(r: &[u8]) -> Option<(u8, u8, u8)> {
    for off in [1usize, 0] {
        if r.len() >= off + 9 && r[off + 5] == 0x83 && r[off] & 0xF0 == 0xA0 {
            return Some((r[off], r[off + 6], r[off + 7]));
        }
    }
    None
}

fn request() -> [u8; 65] {
    let mut w = [0u8; 65];
    w[3] = 0x02;
    w[4] = 0x02;
    w[6] = 0x83;
    w
}

/// One XVI exchange; A0 (waking) gets one retry like upstream. On the cable
/// the mouse is always charging.
pub fn poll(io: &dyn HidIo, vid: u16, pid: u16) -> Option<BatteryStatus> {
    for attempt in 0..2 {
        let r = feature_exchange(io, &request(), 1, delay(vid), 65, parse)?;
        let (status, charging, pct) = r;
        if status == 0xA0 && attempt == 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            continue;
        }
        return Some(match status {
            0xA1 => BatteryStatus {
                level: (pct <= 100).then_some(if pct == 0 { 1 } else { pct }),
                charge: if charging == 1 || is_cable(vid, pid) { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            },
            _ => BatteryStatus::ASLEEP,
        });
    }
    None
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::GloriousMice;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct GloriousMice {
        known: Mutex<BTreeMap<(u16, u16), Device>>,
    }

    /// Keyboard-looking product or >=2 keyboard collections on the same id:
    /// Sinowealth also sells keyboards on 258A.
    fn kind(product: &str, infos: &[hidapi::DeviceInfo], vid: u16, pid: u16) -> DeviceKind {
        match crate::guess_kind(product) {
            DeviceKind::Other => {
                let keyboards = infos.iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid && d.usage_page() == 0x01 && d.usage() == 0x06).count();
                if keyboards >= 2 {
                    DeviceKind::Keyboard
                } else {
                    DeviceKind::Mouse
                }
            }
            k => k,
        }
    }

    impl BatteryProvider for GloriousMice {
        fn name(&self) -> &'static str {
            "glorious"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate().into_iter().filter(|d| supported(d.vendor_id(), d.product_id())).collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut ids: Vec<(u16, u16)> = infos.iter().map(|d| (d.vendor_id(), d.product_id())).collect();
            ids.sort_unstable();
            ids.dedup();
            for &(vid, pid) in &ids {
                // WLMouse YING only answers on its battery collection.
                let mut cols: Vec<_> = infos
                    .iter()
                    .filter(|d| {
                        d.vendor_id() == vid
                            && d.product_id() == pid
                            && (vid != 0x36A7 || (d.usage_page() == 0xFFFF && d.usage() == 0))
                    })
                    .collect();
                // Vendor pages first: the protocol lives there; standard
                // collections are only a fallback for odd firmwares.
                cols.sort_by_key(|d| (if d.usage_page() >= 0xFF00 { 0 } else { 1 }, d.interface_number()));
                let product = cols
                    .iter()
                    .find_map(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    let s = poll(&io, vid, pid);
                    log::debug!("glorious {vid:04x}:{pid:04x} if{} {:04x}/{:04x}: {s:?}", d.interface_number(), d.usage_page(), d.usage());
                    s.map(|status| (path, status))
                });
                match found {
                    Some((path, _)) => {
                        let name = model_name(vid, pid).unwrap_or_else(|| {
                            let n = product.trim();
                            if n.is_empty() { format!("{} wireless device", vendor(vid).unwrap_or("mouse")) } else { n.to_string() }
                        });
                        let dev = Device {
                            id: format!("{}:{vid:04x}:{pid:04x}", vendor(vid).unwrap_or("mouse")),
                            name: name.clone(),
                            vendor: vendor(vid).unwrap_or("Mouse").into(),
                            model: Some(name),
                            kind: kind(&product, &infos, vid, pid),
                            provider: "glorious",
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
            let mut parts = device.locator.split('\t');
            let Some(path) = parts.next() else { return BatteryStatus::GONE };
            let (Some(vid), Some(pid)) = (
                parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()),
                parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()),
            ) else {
                return BatteryStatus::GONE;
            };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll(&io, vid, pid).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// XVI dongle: echoes a reply for the feature request.
    fn dongle(status: u8, charging: u8, pct: u8) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 7 || req[3] != 0x02 || req[4] != 0x02 || req[6] != 0x83 {
                return vec![];
            }
            let mut r = vec![0u8; 65];
            r[0] = 0x00; // report id
            r[1] = status;
            r[6] = 0x83;
            r[7] = charging;
            r[8] = pct;
            vec![r]
        })
    }

    #[test]
    fn awake_reads_level() {
        let s = poll(&dongle(0xA1, 1, 64), 0x33E4, 0x3808).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(64), Charge::Charging, Link::Connected));
    }

    #[test]
    fn asleep_status() {
        assert_eq!(poll(&dongle(0xA4, 0, 0), 0x258A, 0x2022), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn cable_forces_charging() {
        // GWolves HTM Plus cable pid 0x3808 is wired per the table? No: 3808 is dongle.
        // Use a known cable pid: Glorious Model O cable 0x2011.
        let s = poll(&dongle(0xA1, 0, 80), 0x258A, 0x2011).unwrap();
        assert_eq!(s.charge, Charge::Charging);
    }

    #[test]
    fn waking_retries_once() {
        // First reply A0, second A1: the FakeHid handler counts writes.
        use std::cell::Cell;
        use std::rc::Rc;
        let n = Rc::new(Cell::new(0));
        let n2 = n.clone();
        let io = FakeHid::new(move |_| {
            n2.set(n2.get() + 1);
            let mut r = vec![0u8; 65];
            r[1] = if n2.get() == 1 { 0xA0 } else { 0xA1 };
            r[6] = 0x83;
            r[8] = 55;
            vec![r]
        });
        let s = poll(&io, 0x33E4, 0x3808).unwrap();
        assert_eq!(s.level, Some(55));
        assert_eq!(n.get(), 2);
    }

    #[test]
    fn tables() {
        assert_eq!(model_name(0x33E4, 0x3808).as_deref(), Some("G-Wolves HTM Plus"));
        assert_eq!(model_name(0x3770, 0x0136).as_deref(), Some("Hitscan Hyperlight X"));
        assert_eq!(model_name(0x36A7, 0xA874).as_deref(), Some("WLMouse YING"));
        assert!(is_cable(0x258A, 0x2011));
        assert!(!is_cable(0x258A, 0x2022));
        assert!(supported(0x33E4, 0x0001));
        assert!(!supported(0x1234, 0x0001));
    }
}
