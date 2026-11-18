//! AULA F75/F87 Pro/F99 and MCHOSE (K99 V2, G75 Pro, X75...) wireless
//! keyboards on "BY" 2.4 GHz receivers: 3554:FA09 (Compx, shared by both
//! brands) and MCHOSE's own 41E4:2001. Vendor collection FF02 or FF04
//! usage 02, 20-byte reports:
//!   request  [13][cmd][packets][0 ...][sum]  (sum = bytes 0..18 & 0xFF)
//!   reply    [13][cmd|0x80][..][level@5][state@6]...
//!   cmd 0x4A = battery: level 1..100, state high nibble = charging.
//!   cmd 0x05 = six-byte model code (M HUB receivers only).
//! Protocols from PyFlat/Device-Battery-Info, womier-l65-linux,
//! sanshoku#36 and M HUB's web driver; no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const RECEIVERS: [(u16, u16); 2] = [(0x3554, 0xFA09), (0x41E4, 0x2001)];

pub fn supported(vid: u16, pid: u16) -> bool {
    RECEIVERS.contains(&(vid, pid))
}

fn request(cmd: u8, packets: u8) -> [u8; 20] {
    let mut req = [0u8; 20];
    req[0] = 0x13;
    req[1] = cmd;
    req[2] = packets;
    req[19] = req[..19].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    req
}

/// Battery exchange; the receiver also pushes unrelated 0x13 frames, so the
/// reply must echo the command (top bit masked) and carry a valid trailer.
fn exchange(io: &dyn HidIo, cmd: u8, timeout_ms: u64) -> Option<Vec<u8>> {
    query(io, &request(cmd, 0), &[20], timeout_ms, |r| {
        if r.len() < 20 || r[0] != 0x13 || (r[1] & 0x7F) != cmd {
            return None;
        }
        if r[4..19].iter().all(|&b| b == 0) {
            return None; // echo of the request
        }
        let sum = r[..19].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        let checksum = sum == r[19];
        let mchose = r[19] == 0 && r[2] == 1 && r[3] == 0;
        (checksum || mchose).then(|| r[..20].to_vec())
    })
    .ok()
    .flatten()
}

/// Model code -> name (M HUB's keyboardPwds table).
fn model_name(code: &[u8]) -> Option<&'static str> {
    let m = |c: &[u8; 6]| code == c;
    Some(match () {
        _ if m(&[0x03, 0, 0, 0, 0x02, 0x1C]) => "MCHOSE G87",
        _ if m(&[0x03, 0, 0, 0, 0, 0x77]) => "MCHOSE K99",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0xDF]) => "MCHOSE K99 V2",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0x32]) => "MCHOSE G98",
        _ if m(&[0x03, 0, 0, 0, 0, 0xD6]) => "MCHOSE X75",
        _ if m(&[0x0A, 0, 0, 0, 0, 0x03]) => "MCHOSE G98 Pro",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0xC6]) => "MCHOSE X75 V2",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0x15]) => "MCHOSE G75 Pro",
        _ if m(&[0x0A, 0, 0, 0, 0, 0x18]) => "MCHOSE G98 V2",
        _ if m(&[0x0A, 0, 0, 0, 0, 0x2B]) => "MCHOSE UT98",
        _ if m(&[0x0A, 0, 0, 0, 0, 0x01]) => "MCHOSE Z75",
        _ if m(&[0x0A, 0, 0, 0, 0, 0x09]) => "MCHOSE Z75S",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0xC7]) => "MCHOSE KX75",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0x74]) => "MCHOSE GX87",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0xED]) => "MCHOSE GX87 V2",
        _ if m(&[0x03, 0, 0, 0, 0x01, 0x35]) => "MCHOSE K87",
        _ if m(&[0x03, 0, 0, 0, 0x03, 0x35]) => "MCHOSE G75 V2",
        _ if m(&[0x0A, 0, 0, 0, 0, 0x34]) => "MCHOSE G87 V2",
        _ => return None,
    })
}

/// Model code from a cmd-0x05 reply (bytes 5..10, or 4..9 in one capture).
pub fn identify(io: &dyn HidIo) -> Option<&'static str> {
    let r = exchange(io, 0x05, 1300)?;
    for off in [5usize, 4] {
        if r.len() >= off + 6 {
            if let Some(n) = model_name(&r[off..off + 6]) {
                return Some(n);
            }
        }
    }
    None
}

/// Battery poll; `mchose` receivers get M HUB's exact frame semantics.
pub fn poll(io: &dyn HidIo, mchose: bool) -> Option<BatteryStatus> {
    let r = exchange(io, 0x4A, 4000)?;
    let level = r[5];
    if !(1..=100).contains(&level) {
        return None;
    }
    let _ = mchose;
    Some(BatteryStatus {
        level: Some(level),
        charge: if (r[6] >> 4) == 1 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Aula;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Aula {
        /// (vid,pid) -> resolved model name (queried once per pairing).
        names: Mutex<BTreeMap<(u16, u16), String>>,
        known: Mutex<BTreeMap<(u16, u16), Device>>,
    }

    impl BatteryProvider for Aula {
        fn name(&self) -> &'static str {
            "aula"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut names = self.names.lock().unwrap();
            let mut out = Vec::new();
            let mut ids: Vec<(u16, u16)> = Vec::new();
            for d in enumerate().iter() {
                if supported(d.vendor_id(), d.product_id()) && !ids.contains(&(d.vendor_id(), d.product_id())) {
                    ids.push((d.vendor_id(), d.product_id()));
                }
            }
            for &(vid, pid) in &ids {
                let product = enumerate()
                    .iter()
                    .find(|d| d.vendor_id() == vid && d.product_id() == pid)
                    .and_then(|d| d.product_string().map(str::to_string))
                    .unwrap_or_default();
                let found = enumerate()
                    .iter()
                    .filter(|d| {
                        d.vendor_id() == vid
                            && d.product_id() == pid
                            && (d.usage_page() == 0xFF02 || d.usage_page() == 0xFF04)
                            && d.usage() == 0x0002
                    })
                    .find_map(|d| {
                        let path = d.path().to_string_lossy().into_owned();
                        let io = open(&path)?;
                        let generic = product.contains("2.4G Wireless Receiver") || product.contains("MCHOSE 2.4G Wireless");
                        let mchose = vid == 0x41E4 || (generic && names.get(&(vid, pid)).is_none_or(|n| n.starts_with("MCHOSE")));
                        if mchose && !names.contains_key(&(vid, pid)) {
                            if let Some(n) = identify(&io) {
                                names.insert((vid, pid), n.to_string());
                            }
                        }
                        let s = poll(&io, mchose)?;
                        Some((path, s))
                    });
                match found {
                    Some((path, _)) => {
                        let name = names
                            .get(&(vid, pid))
                            .cloned()
                            .unwrap_or_else(|| if vid == 0x41E4 { "MCHOSE wireless keyboard".into() } else { "AULA wireless keyboard".into() });
                        let dev = Device {
                            id: format!("aula:{vid:04x}:{pid:04x}"),
                            name: name.clone(),
                            vendor: "Keyboard".into(),
                            model: Some(name),
                            kind: DeviceKind::Keyboard,
                            provider: "aula",
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
            let mchose = device.provider == "aula" && device.name.starts_with("MCHOSE");
            poll(&io, mchose).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn battery_reply(level: u8, state: u8) -> Vec<u8> {
        let mut r = vec![0u8; 20];
        r[0] = 0x13;
        r[1] = 0x4A;
        r[2] = 1;
        r[5] = level;
        r[6] = state;
        r[19] = r[..19].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        r
    }

    #[test]
    fn level_and_charge_nibble() {
        let io = FakeHid::new(move |_| vec![battery_reply(64, 0x10)]);
        let s = poll(&io, false).unwrap();
        assert_eq!((s.level, s.charge), (Some(64), Charge::Charging));
        let io2 = FakeHid::new(move |_| vec![battery_reply(64, 0x01)]);
        let s2 = poll(&io2, false).unwrap();
        assert_eq!(s2.charge, Charge::Discharging);
    }

    #[test]
    fn unrelated_frames_rejected() {
        let io = FakeHid::new(|_| {
            let mut noise = vec![0u8; 20];
            noise[0] = 0x13;
            noise[1] = 0x0A; // status frame, not 0x4A
            noise[19] = noise[..19].iter().fold(0u8, |a, &b| a.wrapping_add(b));
            vec![noise, battery_reply(80, 0)]
        });
        let s = poll(&io, false).unwrap();
        assert_eq!(s.level, Some(80));
    }

    #[test]
    fn model_codes() {
        assert_eq!(model_name(&[0x03, 0, 0, 0, 0, 0x77]), Some("MCHOSE K99"));
        assert_eq!(model_name(&[0x0A, 0, 0, 0, 0, 0x34]), Some("MCHOSE G87 V2"));
        assert_eq!(model_name(&[0xFF; 6]), None);
    }
}
