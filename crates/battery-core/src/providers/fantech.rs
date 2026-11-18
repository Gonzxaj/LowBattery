//! Fantech Helios Go XD5 wireless mouse (Areson/SinoWealth MCU).
//! VID 25A7: wired FA7B, 2.4 GHz dongle FA7C. Vendor configuration uses
//! 17-byte feature reports id 0x08: [08][cmd][0 x14][checksum], sum of the
//! 17 bytes == 0x55 mod 256. cmd 04 -> reply [6]=percent, [7]=charging.
//! The command goes to one collection and the reply can arrive on ANY input
//! collection of the device (or as a feature report), so discovery passes
//! every reader and the locator keeps all their paths.
//! Protocol from jarjut/xd5-go-web (PROTOCOL_SPECIFICATION.md); no code taken.

use crate::hid::HidIo;
use crate::{BatteryStatus, Charge, Link};
use std::io;
use std::time::{Duration, Instant};

pub const VID: u16 = 0x25A7;
pub const PIDS: [u16; 2] = [0xFA7B, 0xFA7C];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

fn packet(cmd: u8) -> Vec<u8> {
    let mut p = vec![0u8; 17];
    p[0] = 0x08;
    p[1] = cmd;
    let sum: u16 = p[..16].iter().map(|&b| b as u16).sum();
    p[16] = (0x55u16.wrapping_sub(sum)) as u8;
    p
}

/// Parses an input/feature reply: report id may or may not be present;
/// documented layouts put percent/charging at [5]/[6] (with id) or
/// [4]/[5] (without), and [6]/[7] when cmd sits at [0] with id stripped.
fn parse(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 7 {
        return None;
    }
    let plausible = |power: usize, charge: usize| {
        let (p, c) = (*r.get(power)?, *r.get(charge)?);
        ((1..=100).contains(&p) && c <= 1).then_some((p, c != 0))
    };
    if r[0] == 0x04 {
        plausible(5, 6).or_else(|| plausible(4, 5))
    } else if r.len() >= 8 && r[1] == 0x04 {
        plausible(6, 7)
    } else {
        None
    }
}

/// Sends cmd 04 on `cmd` then watches `readers` for the pushed reply,
/// falling back to a feature-report read on `cmd` itself.
pub fn poll(cmd: &dyn HidIo, readers: &[&dyn HidIo]) -> io::Result<Option<(u8, bool)>> {
    cmd.send_feature(&packet(0x04))?;
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut buf = [0u8; 64];
    loop {
        let left = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
        if left <= 0 {
            break;
        }
        // Round-robin a short read on every reader.
        for r in readers {
            let n = r.read_timeout(&mut buf, left.min(40))?;
            if n > 0 {
                if let Some(v) = parse(&buf[..n]) {
                    return Ok(Some(v));
                }
            }
        }
        // Some firmware answers with the feature report instead.
        buf[0] = 0x08;
        if let Ok(n) = cmd.get_feature(&mut buf) {
            if n > 0 {
                if let Some(v) = parse(&buf[..n]) {
                    return Ok(Some(v));
                }
            }
        }
    }
    Ok(None)
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Fantech;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open, Dev};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Fantech {
        known: Mutex<BTreeMap<String, Device>>,
    }

    impl BatteryProvider for Fantech {
        fn name(&self) -> &'static str {
            "fantech"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut seen_pids = std::collections::HashSet::new();
            for info in enumerate().iter() {
                if info.vendor_id() != VID || !PIDS.contains(&info.product_id()) || !seen_pids.insert(info.product_id()) {
                    continue;
                }
                let pid = info.product_id();
                let cols: Vec<_> = enumerate()
                    .iter()
                    .filter(|d| d.vendor_id() == VID && d.product_id() == pid)
                    .cloned().collect();
                // (path, opened handle) per collection.
                let mut ios: Vec<(String, Option<Dev>)> = Vec::new();
                for c in &cols {
                    let p = c.path().to_string_lossy().into_owned();
                    ios.push((p.clone(), open(&p)));
                }
                // Command handle: try each collection until one accepts the
                // feature report; the reply may arrive on any of them.
                let mut found = None;
                for (i, (_, io)) in ios.iter().enumerate() {
                    let Some(io) = io else { continue };
                    let readers: Vec<&dyn HidIo> = ios.iter().filter_map(|(_, d)| d.as_ref().map(|d| -> &dyn HidIo { d })).collect();
                    if let Ok(Some(_)) = poll(io, &readers) {
                        found = Some(i);
                        break;
                    }
                }
                let Some(cmd_i) = found else { continue };
                // Locator: command path first, then every sibling collection.
                let mut paths: Vec<String> = vec![ios[cmd_i].0.clone()];
                paths.extend(ios.iter().enumerate().filter(|(i, _)| *i != cmd_i).map(|(_, (p, _))| p.clone()));
                let dev = Device {
                    id: format!("fantech:{pid:04x}"),
                    name: "Fantech Helios Go XD5".into(),
                    vendor: "Fantech".into(),
                    model: Some("Helios Go XD5".into()),
                    kind: crate::DeviceKind::Mouse,
                    provider: "fantech",
                    // cmd path first, then every sibling collection.
                    locator: paths.join("\t"),
                };
                known.insert(dev.id.clone(), dev.clone());
                out.push(dev);
            }
            let live: std::collections::HashSet<String> = out.iter().map(|d| d.id.clone()).collect();
            for (id, d) in known.iter() {
                if !live.contains(id) {
                    out.push(d.clone());
                }
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut paths = device.locator.split('\t');
            let cmd_path = paths.next().unwrap_or("");
            let Some(cmd) = open(cmd_path) else { return BatteryStatus::GONE };
            let cmd: &dyn HidIo = &cmd;
            let mut siblings: Vec<Dev> = Vec::new();
            for p in paths {
                if let Some(io) = open(p) {
                    siblings.push(io);
                }
            }
            let mut readers: Vec<&dyn HidIo> = vec![cmd];
            readers.extend(siblings.iter().map(|d| -> &dyn HidIo { d }));
            match poll(cmd, &readers) {
                Ok(Some((pct, charging))) => BatteryStatus {
                    level: Some(pct),
                    charge: if charging { Charge::Charging } else { Charge::Discharging },
                    link: Link::Connected,
                },
                Ok(None) => BatteryStatus::ASLEEP,
                Err(_) => BatteryStatus::GONE,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(percent: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; 17];
        r[0] = 0x08;
        r[1] = 0x04;
        r[6] = percent;
        r[7] = charging;
        let sum: u16 = r[..16].iter().map(|&b| b as u16).sum();
        r[16] = (0x55u16.wrapping_sub(sum)) as u8;
        r
    }

    #[test]
    fn parses_report_id_layout() {
        assert_eq!(parse(&reply(64, 1)), Some((64, true)));
    }

    #[test]
    fn parses_without_report_id() {
        let mut r = vec![0u8; 17];
        r[0] = 0x04;
        r[4] = 42;
        r[5] = 0;
        let sum: u16 = r[..16].iter().map(|&b| b as u16).sum();
        r[16] = (0x55u16.wrapping_sub(sum)) as u8;
        assert_eq!(parse(&r), Some((42, false)));
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(parse(&reply(0, 0)).is_none());
        assert!(parse(&reply(101, 0)).is_none());
        assert!(parse(&reply(50, 2)).is_none());
    }

    #[test]
    fn packet_checksum() {
        let p = packet(0x04);
        let sum: u16 = p.iter().map(|&b| b as u16).sum();
        assert_eq!(sum & 0xFF, 0x55);
        assert_eq!((p[0], p[1]), (0x08, 0x04));
    }
}
