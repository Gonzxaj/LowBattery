//! Redragon wireless mice on Areson/CompX receivers (25A7:FA07, FA08),
//! e.g. M913 Impact Elite and M915RGB-WL. Read-only GetPower exchange:
//!   feature report 08 on FF02:0002 (iface 1): [08][04][0 x14][checksum]
//!   reply input report on FF01:0000 (iface 1):
//!   [09][04][00][..][..][..][level 0..10][charging][0 ...] — the whole
//!   frame sums to 0x55 mod 256; level * 10 = percent.
//! The FA7B/FA7C family reports on a different scale and is handled by
//! fantech.rs instead. Protocol from Qehbr/m913-ctl, the M913 battery-tray
//! project and a capture of Redragon's official software; no code taken.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x25A7;
pub const PIDS: [u16; 2] = [0xFA07, 0xFA08];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

fn request() -> [u8; 17] {
    let mut req = [0u8; 17];
    req[0] = 0x08;
    req[1] = 0x04;
    let sum: u16 = req[..16].iter().map(|&b| b as u16).sum();
    req[16] = (0x55u16.wrapping_sub(sum) % 256) as u8;
    req
}

fn parse(r: &[u8]) -> Option<BatteryStatus> {
    if r.len() < 17 || r[0] != 0x09 || r[1] != 0x04 || r[2] != 0 || r[6] > 10 {
        return None;
    }
    let total: u32 = r.iter().map(|&b| b as u32).sum();
    (total & 0xFF == 0x55).then(|| BatteryStatus {
        level: Some(r[6] * 10),
        charge: if r[7] != 0 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

/// `cmd` writes the feature request, `input` reads the reply; the mouse
/// stays silent while asleep (caller keeps the last reading).
pub fn poll(cmd: &dyn HidIo, input: &dyn HidIo) -> Option<BatteryStatus> {
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        cmd.send_feature(&request()).ok()?;
        if let Some(s) = read_matching(input, 1800, parse).ok().flatten() {
            return Some(s);
        }
    }
    None
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Areson;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Areson {
        known: Mutex<BTreeMap<u16, Device>>,
    }

    fn collections(vid: u16, pid: u16) -> Option<(String, String)> {
        let infos: Vec<_> = enumerate().iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).cloned().collect();
        let feat = infos.iter().find(|d| {
            d.interface_number() == 1 && d.usage_page() == 0xFF02 && d.usage() == 2
        })?;
        let input = infos.iter().find(|d| {
            d.interface_number() == 1 && d.usage_page() == 0xFF01 && d.usage() == 0
        })?;
        Some((feat.path().to_string_lossy().into_owned(), input.path().to_string_lossy().into_owned()))
    }

    impl BatteryProvider for Areson {
        fn name(&self) -> &'static str {
            "areson"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = Vec::new();
            for d in enumerate().iter() {
                if supported(d.vendor_id(), d.product_id()) && !pids.contains(&d.product_id()) {
                    pids.push(d.product_id());
                }
            }
            for &pid in &pids {
                let Some((fpath, ipath)) = collections(VID, pid) else { continue };
                let (Some(cmd), Some(input)) = (open(&fpath), open(&ipath)) else { continue };
                match poll(&cmd, &input) {
                    Some(_) => {
                        let product = enumerate()
                            .iter()
                            .find(|d| d.product_id() == pid)
                            .and_then(|d| d.product_string().map(str::to_string))
                            .unwrap_or_default();
                        let name = if product.trim().is_empty() { "Redragon wireless mouse".into() } else { product };
                        let dev = Device {
                            id: format!("areson:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Redragon".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "areson",
                            locator: format!("{fpath}\t{ipath}\t{VID:04x}\t{pid:04x}"),
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
            let (Some(f), Some(i)) = (parts.next(), parts.next()) else { return BatteryStatus::GONE };
            let (Some(cmd), Some(input)) = (open(f), open(i)) else { return BatteryStatus::GONE };
            poll(&cmd, &input).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn reply(level: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; 17];
        r[0] = 0x09;
        r[1] = 0x04;
        r[6] = level;
        r[7] = charging;
        // Whole 17-byte frame must sum to 0x55 mod 256.
        let sum: u32 = r[..16].iter().map(|&b| b as u32).sum();
        r[16] = (0x55u32.wrapping_sub(sum) & 0xFF) as u8;
        r
    }

    #[test]
    fn level_is_times_ten() {
        let cmd = FakeHid::new(|_| vec![]);
        let input = FakeHid::new(|_| vec![reply(8, 1)]);
        input.write(&[0]).unwrap();
        let s = poll(&cmd, &input).unwrap();
        assert_eq!((s.level, s.charge), (Some(80), Charge::Charging));
    }

    #[test]
    fn bad_checksum_rejected() {
        let cmd = FakeHid::new(|_| vec![]);
        let mut bad = reply(8, 0);
        bad[16] ^= 0xFF;
        let input = FakeHid::new(move |_| vec![bad.clone()]);
        input.write(&[0]).unwrap();
        assert_eq!(poll(&cmd, &input), None);
    }
}
