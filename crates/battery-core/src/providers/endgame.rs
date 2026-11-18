//! Endgame Gear 4K wireless mice on the shared dongle 3367:1970 (OP1w 4k
//! v1/v2, XM2w 4k v1/v2). Vendor collection FF01:0002, feature report A1
//! (64 bytes, no checksum): [A1][cmd][target][len][00 ...].
//!   B4 = battery -> reply[16] = percent
//!   0E = mouse info -> reply[16..17]=6733, [18..19]=mouse PID (LE)
//!   0F = probe -> only sent after a failed command (keeps the mouse awake)
//! Reply byte 1: 01 ok, 03 busy, 08/11 asleep or out of range.
//! Protocol from johanneszab/vole and Janlhin's configurator; no code taken.

use crate::hid::{feature_exchange, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x3367;
pub const PID: u16 = 0x1970;

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && pid == PID
}

fn request(cmd: u8, target: u8) -> [u8; 64] {
    let mut req = [0u8; 64];
    req[0] = 0xA1;
    req[1] = cmd;
    req[2] = target;
    req
}

fn name_of(mouse_pid: u16) -> Option<&'static str> {
    Some(match mouse_pid {
        0x1972 => "Endgame Gear OP1w 4k",
        0x1984 => "Endgame Gear OP1w 4k v2",
        0x1968 => "Endgame Gear XM2w 4k",
        0x1982 => "Endgame Gear XM2w 4k v2",
        _ => return None,
    })
}

/// One command exchange; returns (status, reply payload at [16..]).
fn command(io: &dyn HidIo, cmd: u8, target: u8) -> Option<(u8, Vec<u8>)> {
    feature_exchange(io, &request(cmd, target), 1, 150, 64, |r| {
        (r.len() >= 20 && r[0] == 0xA1).then(|| (r[1], r[16..].to_vec()))
    })
}

/// Battery status for the mouse behind the dongle.
pub fn poll(io: &dyn HidIo) -> Option<BatteryStatus> {
    let mut st = command(io, 0xB4, 0x00);
    if !matches!(st.as_ref().map(|s| s.0), Some(0x01 | 0x08 | 0x11)) {
        // Probe the dongle and mouse once, then retry.
        let _ = command(io, 0x0F, 0x01);
        let _ = command(io, 0x0F, 0x0F);
        st = command(io, 0xB4, 0x00);
    }
    let (status, reply) = st?;
    match status {
        0x08 | 0x11 => Some(BatteryStatus::ASLEEP),
        0x01 if reply.first().is_some_and(|&p| p <= 100) => {
            let level = reply[0];
            Some(BatteryStatus {
                level: Some(level),
                charge: Charge::Discharging, // no charging flag decoded for this family
                link: Link::Connected,
            })
        }
        _ => None,
    }
}

/// Mouse info command: returns the model name when known.
pub fn mouse_name(io: &dyn HidIo) -> Option<&'static str> {
    let (st, r) = command(io, 0x0E, 0x00)?;
    if st != 0x01 || r.len() < 4 || r[0] != 0x67 || r[1] != 0x33 {
        return None;
    }
    name_of(u16::from_le_bytes([r[2], r[3]]))
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Endgame;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Endgame {
        /// Dongle path -> last mouse name (only identifiable while awake).
        names: Mutex<HashMap<String, String>>,
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for Endgame {
        fn name(&self) -> &'static str {
            "endgame"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut names = self.names.lock().unwrap();
            let mut out = Vec::new();
            for d in enumerate().iter() {
                if !(supported(d.vendor_id(), d.product_id()) && d.usage_page() == 0xFF01 && d.usage() == 0x0002) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let Some(io) = open(&path) else { continue };
                let cached = names.get(&path).cloned().unwrap_or_else(|| "Endgame Gear 4k wireless mouse".into());
                match poll(&io) {
                    Some(_) => {
                        if let Some(n) = mouse_name(&io) {
                            names.insert(path.clone(), n.to_string());
                        }
                        let name = names.get(&path).cloned().unwrap_or(cached);
                        let dev = Device {
                            id: "endgame:1970".into(),
                            name: name.clone(),
                            vendor: "Endgame Gear".into(),
                            model: Some(name),
                            kind: DeviceKind::Mouse,
                            provider: "endgame",
                            locator: format!("{path}\t{VID:04x}\t{PID:04x}"),
                        };
                        *known = Some(dev.clone());
                        out.push(dev);
                    }
                    None => out.extend(known.take()),
                }
                break; // one 4K dongle
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

    fn reply(_cmd: u8, status: u8, payload: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0xA1;
        r[1] = status;
        r[16..16 + payload.len()].copy_from_slice(payload);
        r
    }

    #[test]
    fn battery_reply() {
        let io = FakeHid::new(move |_| vec![reply(0xB4, 0x01, &[77])]);
        let s = poll(&io).unwrap();
        assert_eq!((s.level, s.link), (Some(77), Link::Connected));
    }

    #[test]
    fn asleep_status() {
        let io = FakeHid::new(move |_| vec![reply(0xB4, 0x08, &[0])]);
        assert_eq!(poll(&io), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn mouse_name_from_info() {
        let io = FakeHid::new(move |req| {
            if req.len() >= 2 && req[1] == 0x0E {
                return vec![reply(0x0E, 0x01, &[0x67, 0x33, 0x72, 0x19])]; // pid 0x1972
            }
            vec![reply(0xB4, 0x01, &[50])]
        });
        assert_eq!(mouse_name(&io), Some("Endgame Gear OP1w 4k"));
    }
}
