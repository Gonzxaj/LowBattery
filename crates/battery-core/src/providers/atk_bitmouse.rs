//! ATK / VXE NearLink "BITMOUSE" dongle (VID 373B, PID 1216) used by the
//! VXE R1 / Dragonfly R1 family and some ATK mice. Unlike the other ATK
//! gear it does not speak the 17-byte report 08 protocol of kysona.rs.
//!   Output report 08, 64 bytes: [08][checksum][72][02][len][target][cmd]
//!   [expected-reply-len]..., checksum = sum(bytes 2..63) & 0xFF.
//!   Input reply: [08][72][status][seq][00][cmd][01][value]; a timeout
//!   means the mouse is asleep. Commands: 07 = battery level (1..100),
//!   0F = charging (1 = charging). No reply at all -> keep last value with
//!   a long grace (the monitor already does that via `known`).
//! Protocol from github.com/BigSmoke666/ATK-N9-Battery (BITMOUSE frames)
//! and the ATK controller specs; reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x373B;
pub const PID: u16 = 0x1216;
pub const PAGE: u16 = 0xFF05;
pub const USAGE: u16 = 0x0001;

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && pid == PID
}

fn request(cmd: u8) -> [u8; 64] {
    let mut req = [0u8; 64];
    req[0] = 0x08; // report id
    req[2] = 0x72;
    req[3] = 0x02; // request length
    req[5] = 0x01; // target
    req[6] = cmd;
    req[7] = 0x01; // expected reply length
    req[1] = req[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    req
}

fn command(io: &dyn HidIo, cmd: u8, timeout_ms: u64) -> Option<u8> {
    let req = request(cmd);
    query(io, &req, &[64], timeout_ms, |r| {
        // Reply: [08][72][status][seq][00][cmd][01][value]
        if r.len() >= 8 && r[0] == 0x08 && r[1] == 0x72 && r[5] == cmd && r[2] != 0xFF {
            return Some(r[7]);
        }
        None
    })
    .ok()
    .flatten()
}

/// Battery level then charging flag; a missing reply means asleep.
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let level = command(io, 0x07, 700).filter(|&v| (1..=100).contains(&v))?;
    let charging = command(io, 0x0F, 700).is_some_and(|v| v == 1);
    Some(BatteryStatus {
        level: Some(level),
        charge: if charging { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

/// No answer at all -> the mouse is off; the caller keeps the last card.
pub fn poll_or_asleep(io: &dyn HidIo) -> BatteryStatus {
    poll(io).unwrap_or(BatteryStatus::ASLEEP)
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::AtkBitmouse;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct AtkBitmouse {
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for AtkBitmouse {
        fn name(&self) -> &'static str {
            "atk-bitmouse"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            for d in enumerate() {
                if !(d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == PAGE && d.usage() == USAGE) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let Some(io) = open(&path) else { continue };
                if poll(&io).is_some() {
                    let product = d.product_string().unwrap_or("ATK wireless mouse");
                    let dev = Device {
                        id: "atknk:1216".into(),
                        name: product.to_string(),
                        vendor: "ATK".into(),
                        model: Some(product.to_string()),
                        kind: DeviceKind::Mouse,
                        provider: "atk-bitmouse",
                        locator: format!("{path}\t{VID:04x}\t{PID:04x}"),
                    };
                    *known = Some(dev.clone());
                    out.push(dev);
                } else {
                    out.extend(known.take());
                }
                break; // one BITMOUSE dongle
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll_or_asleep(&io)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn reply(cmd: u8, value: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x08;
        r[1] = 0x72;
        r[2] = 0x00;
        r[5] = cmd;
        r[6] = 0x01;
        r[7] = value;
        r
    }

    fn dongle(level: u8, charging: u8) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 8 || req[0] != 0x08 || req[6] > 0x0F {
                return vec![];
            }
            match req[6] {
                0x07 => vec![reply(0x07, level)],
                0x0F => vec![reply(0x0F, charging)],
                _ => vec![],
            }
        })
    }

    #[test]
    fn level_and_charging() {
        let s = poll(&dongle(62, 1)).unwrap();
        assert_eq!((s.level, s.charge), (Some(62), Charge::Charging));
        let s = poll(&dongle(62, 0)).unwrap();
        assert_eq!(s.charge, Charge::Discharging);
    }

    #[test]
    fn silence_is_asleep() {
        assert_eq!(poll_or_asleep(&FakeHid::new(|_| vec![])), BatteryStatus::ASLEEP);
    }

    #[test]
    fn checksum_is_sum_of_tail() {
        let req = request(0x07);
        let expect: u8 = req[2..].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        assert_eq!(req[1], expect);
        assert_eq!(req[0], 0x08);
        assert_eq!(req[2], 0x72);
        assert_eq!(req[6], 0x07);
    }
}
