//! Flydigi Vader 5 Pro gamepad (37D7:2401), vendor collection FFA0:0001.
//! Unnumbered output report: [0][5A][A5][cmd][0][0].
//!   cmd 07 = architecture query (reply decides "modern" framing),
//!   cmd 01 sub 02 = device info: reply [5A][A5][01]... [5]=0x82,
//!   [6]=connection (1 = 2.4 GHz, 2 = cable), [11] = state<<4 | level(0..5)
//!   -> percent = level*20, state 2 = full (100%), state 1 = charging.
//! Protocol from SDL's Flydigi HIDAPI driver and HaloBattery issue #191;
//! reimplemented here, no code taken.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x37D7;
pub const PID: u16 = 0x2401;

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && pid == PID
}

/// Reply payload after [5A][A5][cmd]; some firmware prefixes 0x04.
fn parse(r: &[u8], cmd: u8) -> Option<Vec<u8>> {
    for off in 0..=1usize {
        if r.len() >= off + 5 && r[off] == 0x5A && r[off + 1] == 0xA5 && r[off + 2] == cmd {
            return Some(r[off..].to_vec());
        }
    }
    None
}

fn exchange(io: &dyn HidIo, out: &[u8; 6], cmd: u8, timeout_ms: u64) -> Option<Vec<u8>> {
    io.write(out).ok()?;
    read_matching(io, timeout_ms, |r| parse(r, cmd)).ok().flatten()
}

pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let modern = exchange(io, &[0, 0x5A, 0xA5, 0x07, 0, 0], 0x07, 800)
        .is_some_and(|r| r.len() >= 5 && r[3] == 1 && r[4] == 0);
    let r = exchange(io, &[0, 0x5A, 0xA5, 0x01, 2, 0], 0x01, 800)?;
    if r.len() < 12 || r[5] != 130 {
        return None; // not the confirmed Vader 5 Pro identity
    }
    let level = r[11] & 0x0F;
    let state = r[11] >> 4;
    if state > 2 || level > 5 {
        return None;
    }
    let connection = r[6];
    let on_receiver = if modern { connection == 1 } else { connection == 2 };
    if !(connection == 1 || connection == 2) && !on_receiver {
        return None;
    }
    Some(BatteryStatus {
        level: Some(if state == 2 { 100 } else { level * 20 }),
        charge: if state == 1 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Flydigi;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Flydigi {
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for Flydigi {
        fn name(&self) -> &'static str {
            "flydigi"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            for d in enumerate().iter() {
                if !(d.vendor_id() == VID && d.product_id() == PID && d.usage_page() == 0xFFA0 && d.usage() == 1) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let Some(io) = open(&path) else { continue };
                if poll(&io).is_some() {
                    let dev = Device {
                        id: format!("flydigi:{path}"),
                        name: "Flydigi Vader 5 Pro".into(),
                        vendor: "Flydigi".into(),
                        model: Some("Flydigi Vader 5 Pro".into()),
                        kind: DeviceKind::Controller,
                        provider: "flydigi",
                        locator: format!("{path}\t{VID:04x}\t{PID:04x}"),
                    };
                    *known = Some(dev.clone());
                    out.push(dev);
                } else {
                    out.extend(known.take());
                }
                break;
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let path = device.locator.split('\t').next().unwrap_or("");
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll(&io).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    fn info_reply(level: u8, state: u8, conn: u8) -> Vec<u8> {
        let mut r = vec![0u8; 12];
        r[0] = 0x5A;
        r[1] = 0xA5;
        r[2] = 0x01;
        r[5] = 130;
        r[6] = conn;
        r[11] = (state << 4) | level;
        r
    }

    #[test]
    fn level_times_twenty() {
        // 07 arch reply: modern (r[3]=1,r[4]=0) then 01 info.
        let io = FakeHid::new(|req| {
            if req.len() >= 4 && req[3] == 0x07 {
                let mut r = vec![0u8; 5];
                r[0] = 0x5A;
                r[1] = 0xA5;
                r[2] = 0x07;
                r[3] = 1;
                r[4] = 0;
                return vec![r];
            }
            vec![info_reply(3, 0, 1)]
        });
        let s = poll(&io).unwrap();
        assert_eq!((s.level, s.link), (Some(60), Link::Connected));
    }

    #[test]
    fn full_is_hundred() {
        let io = FakeHid::new(|_| vec![info_reply(0, 2, 2)]);
        let s = poll(&io).unwrap();
        assert_eq!(s.level, Some(100));
    }
}
