//! BenQ ZOWIE wireless mice (VID 04A5). Community-reverse-engineered
//! read-only vendor commands; write-capable commands are never sent.
//!   EC-CW series (cable 8005, receiver 8006): unnumbered 64-byte output
//!   [00 A5 03], reply [mV lo][mV hi][percent]; mV = 0 means mouse not
//!   linked. The Enhanced Receiver declares the collection on page 00FF.
//!   DW series + U2 (receiver 800A, U2 receiver 8008): output report 08
//!   [B4 15] on FF03, reply input report 09 on FF04: mV = ([1]*256+[2])*10,
//!   [4] must be C8.
//! Voltage -> % anchored on the mouse LED thresholds; charging on the
//! receiver is inferred from the voltage trend (rises >=50 mV above the
//! trough = docked, drops >=20 mV below the peak = undocked).
//! Protocols from yleshchuk/zowie-battery and IgnacioBarb/zowie-battery-tray;
//! no code taken.

use crate::hid::{query, read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x04A5;

/// mV -> percent via the LED threshold anchors.
pub fn percent_of_mv(mv: u16) -> u8 {
    const V: [u16; 6] = [4200, 4000, 3890, 3760, 3680, 3500];
    const P: [u8; 6] = [100, 75, 50, 25, 10, 0];
    for i in 1..V.len() {
        if mv >= V[i] {
            let (hi_v, lo_v) = (V[i - 1], V[i]);
            let (hi_p, lo_p) = (P[i - 1], P[i]);
            return lo_p + ((mv - lo_v) * (hi_p - lo_p) as u16 / (hi_v - lo_v).max(1)) as u8;
        }
    }
    0
}

/// Charging guess from the voltage trend of one receiver.
#[derive(Default)]
pub struct ChargeTrend {
    has: bool,
    charging: bool,
    peak: u16,
    trough: u16,
}

impl ChargeTrend {
    pub fn update(&mut self, mv: u16) -> bool {
        if !self.has {
            self.has = true;
            self.charging = mv >= 4250;
            self.peak = mv;
            self.trough = mv;
        } else if self.charging {
            self.peak = self.peak.max(mv);
            if mv <= self.peak - 20 {
                self.charging = false;
                self.trough = mv;
            }
        } else {
            self.trough = self.trough.min(mv);
            if mv >= self.trough + 50 {
                self.charging = true;
                self.peak = mv;
            }
        }
        self.charging
    }
}

/// EC-CW: write [00 A5 03], read [mV lo][mV hi][percent].
pub fn poll_cw(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    let r = query(io, &[0x00, 0xA5, 0x03], &[64, 3], 1000, |r| {
        (r.len() >= 3).then(|| r[..r.len().min(8)].to_vec())
    })
    .ok()
    .flatten()?;
    let mv = u16::from_le_bytes([r[0], r[1]]);
    if mv != 0 && !(3000..=4500).contains(&mv) {
        return None;
    }
    Some(if mv == 0 {
        BatteryStatus::ASLEEP // mouse not linked
    } else {
        let pct = if (1..=100).contains(&r[2]) { r[2] } else { percent_of_mv(mv) };
        BatteryStatus {
            level: Some(pct),
            charge: if cable { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }
    })
}

/// DW/U2: write report 08 [B4 15], read report 09 [mV hi][mV mid][mV lo?]...
/// Actually: mV = (r[1]*256 + r[2]) * 10, r[4] == 0xC8.
pub fn poll_dw(cmd: &dyn HidIo, input: &dyn HidIo, cable: bool, trend: &mut ChargeTrend) -> Option<BatteryStatus> {
    cmd.write(&[0x08, 0xB4, 0x15]).ok()?;
    let r = read_matching(input, 1500, |r| {
        (r.len() >= 5 && r[0] == 0x09 && r[4] == 0xC8 && r[1] != 0xE6).then(|| r[..8].to_vec())
    })
    .ok()
    .flatten()?;
    let mv = (u16::from_be_bytes([r[1], r[2]])) * 10;
    Some(if mv == 0 {
        BatteryStatus::ASLEEP
    } else {
        BatteryStatus {
            level: Some(percent_of_mv(mv)),
            charge: if cable || trend.update(mv) { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        }
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Zowie;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Zowie {
        known: Mutex<BTreeMap<u16, Device>>,
        trends: Mutex<BTreeMap<u16, ChargeTrend>>,
    }

    /// EC-CW collection: any vendor-ish page with 65-byte output.
    fn cw_path(vid: u16, pid: u16) -> Option<String> {
        enumerate()
            .iter()
            .find(|d| {
                d.vendor_id() == vid
                    && d.product_id() == pid
                    && (d.usage_page() >= 0xFF00 || d.usage_page() == 0x00FF)
            })
            .map(|d| d.path().to_string_lossy().into_owned())
    }

    /// DW: command collection FF03 (16B out) + reply FF04 (16B in).
    fn dw_paths(vid: u16, pid: u16) -> Option<(String, String)> {
        let infos: Vec<_> = enumerate().iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).cloned().collect();
        let cmd = infos.iter().find(|d| d.usage_page() == 0xFF03)?;
        let rep = infos.iter().find(|d| d.usage_page() == 0xFF04)?;
        Some((cmd.path().to_string_lossy().into_owned(), rep.path().to_string_lossy().into_owned()))
    }

    impl BatteryProvider for Zowie {
        fn name(&self) -> &'static str {
            "zowie"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut trends = self.trends.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in enumerate().iter() {
                if d.vendor_id() == VID && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                let is_cw = pid == 0x8005 || pid == 0x8006;
                let cable = pid == 0x8005;
                let found = if is_cw {
                    cw_path(VID, pid).and_then(|p| {
                        let io = open(&p)?;
                        poll_cw(&io, cable).map(|s| (p, s))
                    })
                } else {
                    dw_paths(VID, pid).and_then(|(cp, rp)| {
                        let (Some(cmd), Some(input)) = (open(&cp), open(&rp)) else { return None };
                        let trend = trends.entry(pid).or_default();
                        poll_dw(&cmd, &input, pid != 0x800A && pid != 0x8008, trend).map(|s| (cp, s))
                    })
                };
                match found {
                    Some((path, _)) => {
                        let name = if pid == 0x8008 {
                            "ZOWIE U2".to_string()
                        } else if is_cw {
                            "ZOWIE EC-CW".into()
                        } else {
                            let p = enumerate()
                                .iter()
                                .find(|d| d.vendor_id() == VID && d.product_id() == pid)
                                .and_then(|d| d.product_string().map(str::to_string))
                                .unwrap_or_default();
                            if p.is_empty() || p.contains("Receiver") { "ZOWIE DW mouse".into() } else { p }
                        };
                        let dev = Device {
                            id: format!("zowie:{pid:04x}"),
                            name: name.clone(),
                            vendor: "ZOWIE".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "zowie",
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
            let pid = device.locator.split('\t').nth(2).and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
            let is_cw = pid == 0x8005 || pid == 0x8006;
            let cable_cw = pid == 0x8005;
            let cable_dw = pid != 0x800A && pid != 0x8008;
            if is_cw {
                let Some(io) = open(path) else { return BatteryStatus::GONE };
                return poll_cw(&io, cable_cw).unwrap_or(BatteryStatus::ASLEEP);
            }
            let mut parts = device.locator.split('\t');
            let Some(cp) = parts.next() else { return BatteryStatus::GONE };
            // DW locator stores the command path; find the reply sibling.
            let Some(rp) = dw_paths(VID, pid).map(|(_, r)| r) else { return BatteryStatus::GONE };
            let (Some(cmd), Some(input)) = (open(cp), open(&rp)) else { return BatteryStatus::GONE };
            let mut trends = self.trends.lock().unwrap();
            let trend = trends.entry(pid).or_default();
            poll_dw(&cmd, &input, cable_dw, trend).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn mv_curve() {
        assert_eq!(percent_of_mv(4200), 100);
        assert_eq!(percent_of_mv(3500), 0);
        assert!(percent_of_mv(3890) > percent_of_mv(3680));
    }

    #[test]
    fn trend_detects_charging() {
        let mut t = ChargeTrend::default();
        assert!(!t.update(3700));
        assert!(t.update(3800)); // +100 mV above trough -> docked
        assert!(!t.update(3700)); // 20 mV below the 3800 peak -> came off
    }

    #[test]
    fn cw_reply() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 3];
            r[0] = 0xD0; // 7200? too low; use 3800 = 0xED8 -> lo ED hi 0E
            r[0] = 0xD8;
            r[1] = 0x0E;
            r[2] = 60;
            vec![r]
        });
        let s = poll_cw(&io, false).unwrap();
        assert_eq!(s.level, Some(60));
        let io0 = FakeHid::new(|_| vec![vec![0u8, 0, 0]]);
        assert_eq!(poll_cw(&io0, false), Some(BatteryStatus::ASLEEP));
    }
}
