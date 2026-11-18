//! Fractal Design Scape headset (VID 36BC): 2.4 GHz dongle (PID 0001) or the
//! headset itself on its USB-C cable (PID 2001). Both expose a vendor
//! collection on usage page FF00 / usage 01.
//!
//! Requests are output report 0x02: [0x02][device type][command]. The reply is
//! an input report 0x02 echoing the device type and command, then the payload.
//!   GET_DEVICE_STATE = 0x21, sent to device type 0x11 (dongle) or 0xF1
//!   (headset).
//!   Dongle reply:  payload byte 3 = 1 when the headset is linked to it.
//!   Headset reply: payload byte 14 = battery level in percent. Through the
//!                  dongle the request is relayed over the air; on the cable
//!                  it is answered directly.
//! Byte offsets count from the device type, so with the report id included
//! they land one byte later in the hidapi buffer (r[4], r[15], r[23]).
//! Payload byte 22 reads 1 with the headset on its USB cable and 0 over the
//! air; it is used as the charging bit. The headset unlinks from the dongle
//! while it sits on its charging stand, so it then shows as off.
//! Source: Fractal's own WebHID configurator, Adjust Pro
//! (adjust.fractal-design.com); reimplemented here, no code taken.

use crate::hid::{query, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x36BC;
pub const PIDS: [u16; 2] = [0x0001, 0x2001];
const CABLE: u16 = 0x2001;

const REPORT: u8 = 0x02;
const TYPE_DONGLE: u8 = 0x11;
const TYPE_HEADSET: u8 = 0xF1;
const GET_STATE: u8 = 0x21;
const TIMEOUT_MS: u64 = 1000;
/// Output report is at least 4 bytes; retry padded to plausible sizes.
const PADS: [usize; 2] = [4, 64];

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && PIDS.contains(&pid)
}

/// GET_DEVICE_STATE to `ty`; returns the reply payload area (report bytes).
fn get_state(io: &dyn HidIo, ty: u8) -> Option<Vec<u8>> {
    query(io, &[REPORT, ty, GET_STATE], &PADS, TIMEOUT_MS, |r| {
        (r.len() >= 24 && r[0] == REPORT && r[1] == ty && r[2] == GET_STATE).then(|| r[..24].to_vec())
    })
    .ok()
    .flatten()
}

/// `cable` = the headset's own USB interface (PID 2001); otherwise the
/// 2.4 GHz dongle relays the query to the paired headset.
pub fn poll(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    if !cable {
        let link = get_state(io, TYPE_DONGLE)?;
        if link[4] != 1 {
            // Dongle answers, headset off, out of range or on its stand.
            return Some(BatteryStatus::ASLEEP);
        }
    }
    let r = get_state(io, TYPE_HEADSET)?;
    if r[15] > 100 {
        return None;
    }
    Some(BatteryStatus {
        level: Some(r[15]),
        charge: if r[23] == 1 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Fractal;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::sync::Mutex;

    /// One Scape per machine: the dongle and the cable are the same headset.
    #[derive(Default)]
    pub struct Fractal {
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for Fractal {
        fn name(&self) -> &'static str {
            "fractal"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate()
                .iter()
                .filter(|d| supported(d.vendor_id(), d.product_id()) && d.usage_page() == 0xFF00 && d.usage() == 0x0001)
                .cloned().collect();
            if infos.is_empty() {
                self.known.lock().unwrap().take();
                return Vec::new();
            }
            // Cable first: it answers directly and doesn't depend on the air link.
            let mut cols: Vec<_> = infos.iter().collect();
            cols.sort_by_key(|d| if d.product_id() == CABLE { 0 } else { 1 });
            let found = cols.iter().find_map(|d| {
                let path = d.path().to_string_lossy().into_owned();
                let io = open(&path)?;
                let cable = d.product_id() == CABLE;
                let s = poll(&io, cable);
                log::debug!("fractal {:04x} if{} cable={cable}: {s:?}", d.product_id(), d.interface_number());
                s.map(|status| (path, d.product_id(), cable, status))
            });
            match found {
                Some((path, pid, cable, _)) => {
                    let dev = Device {
                        id: "fractal:scape".into(),
                        name: "Fractal Scape".into(),
                        vendor: "Fractal Design".into(),
                        model: Some("Scape".into()),
                        kind: DeviceKind::Headset,
                        provider: "fractal",
                        locator: format!("{path}\t{VID:04x}\t{pid:04x}\t{}", u8::from(cable)),
                    };
                    *self.known.lock().unwrap() = Some(dev.clone());
                    vec![dev]
                }
                None => self.known.lock().unwrap().clone().into_iter().collect(),
            }
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let Some(path) = parts.next() else { return BatteryStatus::GONE };
            let cable = parts.nth(2).and_then(|v| v.parse::<u8>().ok()) == Some(1);
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            poll(&io, cable).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// Reply to GET_DEVICE_STATE for `ty`: link byte and headset payload.
    fn reply(ty: u8, linked: u8, level: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; 24];
        r[0] = REPORT;
        r[1] = ty;
        r[2] = GET_STATE;
        r[4] = linked;
        r[15] = level;
        r[23] = charging;
        r
    }

    fn scape(linked: u8, level: u8, charging: u8) -> FakeHid {
        FakeHid::new(move |req| {
            if req.len() < 3 || req[0] != REPORT || req[2] != GET_STATE {
                return vec![];
            }
            match req[1] {
                TYPE_DONGLE => vec![reply(TYPE_DONGLE, linked, 0, 0)],
                TYPE_HEADSET => vec![reply(TYPE_HEADSET, 0, level, charging)],
                _ => vec![],
            }
        })
    }

    #[test]
    fn dongle_linked_reads_headset() {
        let s = poll(&scape(1, 64, 0), false).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(64), Charge::Discharging, Link::Connected));
    }

    #[test]
    fn dongle_unlinked_is_asleep() {
        assert_eq!(poll(&scape(0, 0, 0), false), Some(BatteryStatus::ASLEEP));
    }

    #[test]
    fn cable_reads_directly() {
        let s = poll(&scape(0, 88, 1), true).unwrap();
        assert_eq!((s.level, s.charge), (Some(88), Charge::Charging));
    }

    #[test]
    fn silence_is_none() {
        assert_eq!(poll(&FakeHid::new(|_| vec![]), true), None);
    }

    #[test]
    fn supported_tables() {
        assert!(supported(0x36BC, 0x0001));
        assert!(supported(0x36BC, 0x2001));
        assert!(!supported(0x36BC, 0x2002));
        assert!(!supported(0x1234, 0x2001));
    }
}
