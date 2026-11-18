//! Sony DualShock 4 / DualSense / DualSense Edge (VID 054C), USB, Bluetooth
//! or the DualShock 4 wireless adapter. The battery rides in the regular
//! input report (layout per Linux hid-playstation.c):
//!   DualShock 4: report 01 byte 30, Bluetooth report 11 byte 32:
//!                bits 0-3 level, bit 4 cable
//!   DualSense:   report 01 byte 53, Bluetooth report 31 byte 54:
//!                bits 0-3 level (0-10), bits 4-7 state
//! Over Bluetooth the controller sends reduced reports until feature report
//! 05 is read once, which is what games do too.

use crate::hid::{HidIo, pad_report};
use crate::{BatteryStatus, Charge, DeviceKind};

pub const VID: u16 = 0x054C;
pub const PIDS: [u16; 5] = [0x05C4, 0x09CC, 0x0BA0, 0x0CE6, 0x0DF2];
const TIMEOUT_MS: u64 = 500;
/// Report sizes tried for the feature read; hidapi does not expose the
/// descriptor's feature-report length and stacks disagree on buffer size.
const FEATURE_SIZES: [usize; 4] = [64, 78, 37, 32];

fn is_ds4(pid: u16) -> bool {
    matches!(pid, 0x05C4 | 0x09CC | 0x0BA0)
}

fn name(pid: u16) -> &'static str {
    if is_ds4(pid) {
        "DualShock 4"
    } else if pid == 0x0DF2 {
        "DualSense Edge"
    } else {
        "DualSense"
    }
}

/// The wireless adapter (0x0BA0) reports a zeroed battery byte when no
/// controller is paired to it.
#[derive(Debug, PartialEq)]
pub enum Seen {
    /// Adapter present but no controller behind it: no card at all.
    EmptyAdapter,
    Status(BatteryStatus),
}

/// One input report -> battery, or None when it isn't a gamepad report.
/// `bt` mirrors the C++ guard: once a Bluetooth-format report (11/31) is
/// seen, short USB-format report 01 is ignored.
pub fn decode(pid: u16, bt: &mut bool, r: &[u8]) -> Option<Seen> {
    if is_ds4(pid) {
        let at = match r.first()? {
            0x01 => {
                if *bt {
                    return None;
                }
                30
            }
            0x11 => {
                *bt = true;
                32
            }
            _ => return None,
        };
        let b = *r.get(at)?;
        if pid == 0x0BA0 && b == 0 {
            return Some(Seen::EmptyAdapter);
        }
        let level = (b & 0x0F) as u16;
        let cable = b & 0x10 != 0;
        let pct = if cable {
            if level >= 11 {
                100
            } else {
                level * 10
            }
        } else {
            level * 10 + 5
        };
        let charge = if cable && level < 11 { Charge::Charging } else { Charge::Discharging };
        Some(Seen::Status(BatteryStatus::connected(pct.min(100) as u8, charge)))
    } else {
        let at = match r.first()? {
            0x01 => {
                if *bt {
                    return None;
                }
                53
            }
            0x31 => {
                *bt = true;
                54
            }
            _ => return None,
        };
        let b = *r.get(at)?;
        let level = (b & 0x0F) as u16;
        let state = b >> 4;
        let charge = match state {
            0x0 => Charge::Discharging,
            0x1 => Charge::Charging,
            0x2 => Charge::Full,
            _ => Charge::Unknown,
        };
        let pct = if state == 0x2 { 100 } else { (level * 10 + 5).min(100) };
        Some(Seen::Status(BatteryStatus::connected(pct as u8, charge)))
    }
}

/// Feature report 05 once: on Bluetooth it switches the pad to full-size
/// reports. Harmless over USB (games read it for calibration too).
fn wake(io: &dyn HidIo) {
    for size in FEATURE_SIZES {
        let mut buf = pad_report(&[0x05], size);
        if io.get_feature(&mut buf).is_ok() {
            return;
        }
    }
}

/// Wait for one parseable gamepad report. Ok(None) = timeout (asleep).
pub fn poll(io: &dyn HidIo, pid: u16) -> Option<Seen> {
    wake(io);
    let mut bt = false;
    crate::hid::read_matching(io, TIMEOUT_MS, |r| decode(pid, &mut bt, r))
        .ok()
        .flatten()
}

/// FNV-1a of the HID path: stable device id without exposing the path.
pub fn path_hash(path: &str) -> u32 {
    path.bytes().fold(0x811C_9DC5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::PlayStation;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};

    pub struct PlayStation;

    fn collections() -> Vec<hidapi::DeviceInfo> {
        enumerate()
            .iter()
            .filter(|d| {
                d.vendor_id() == VID
                    && PIDS.contains(&d.product_id())
                    && (d.usage_page(), d.usage()) == (0x0001, 0x0005)
            })
            .cloned().collect()
    }

    impl BatteryProvider for PlayStation {
        fn name(&self) -> &'static str {
            "playstation"
        }

        fn discover(&self) -> Vec<Device> {
            let mut out = Vec::new();
            for d in collections() {
                let path = d.path().to_string_lossy().into_owned();
                // A quick probe tells an empty wireless adapter (no card)
                // from a sleeping controller (card, shown asleep).
                let empty = open(&path).is_some_and(|io| matches!(poll(&io, d.product_id()), Some(Seen::EmptyAdapter)));
                if empty {
                    continue;
                }
                out.push(Device {
                    id: format!("ps:{:04x}:{:08x}", d.product_id(), path_hash(&path)),
                    name: name(d.product_id()).into(),
                    vendor: "Sony".into(),
                    model: Some(name(d.product_id()).into()),
                    kind: DeviceKind::Controller,
                    provider: "playstation",
                    locator: format!("{}\t{:04x}", path, d.product_id()),
                });
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let (Some(path), Some(pid)) = (parts.next(), parts.next().and_then(|p| u16::from_str_radix(p, 16).ok())) else {
                return BatteryStatus::GONE;
            };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            match poll(&io, pid) {
                Some(Seen::Status(s)) => s,
                // Empty adapter or silence: asleep, last level is kept.
                _ => BatteryStatus::ASLEEP,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn dualsense_usb_and_bluetooth_reports() {
        // USB report 01, byte 53: level 7 -> 75 %, state 1 charging.
        let mut r = vec![0x01u8; 64];
        r[53] = 0x17;
        let mut bt = false;
        let s = match decode(0x0CE6, &mut bt, &r) {
            Some(Seen::Status(s)) => s,
            _ => panic!(),
        };
        assert_eq!((s.level, s.charge), (Some(75), Charge::Charging));
        // Bluetooth report 31, byte 54: level 10, state 2 -> 100 % full.
        let mut r = vec![0x31u8; 78];
        r[54] = 0x2A; // level 10, state 2 -> 100 % full
        let s = match decode(0x0CE6, &mut bt, &r) {
            Some(Seen::Status(s)) => s,
            _ => panic!(),
        };
        assert_eq!((s.level, s.charge), (Some(100), Charge::Full));
        // After a BT report, a stray USB-format report is ignored.
        assert_eq!(decode(0x0CE6, &mut bt, &[0x01u8; 64]), None);
    }

    #[test]
    fn dualshock4_levels() {
        // Report 01 byte 30: level 5 on battery -> 55 %.
        let mut r = vec![0x01u8; 64];
        r[30] = 0x05;
        let mut bt = false;
        let s = match decode(0x05C4, &mut bt, &r) {
            Some(Seen::Status(s)) => s,
            _ => panic!(),
        };
        assert_eq!((s.level, s.charge), (Some(55), Charge::Discharging));
        // Same level on cable, below 11 -> charging, 50 %.
        r[30] = 0x15;
        let s = match decode(0x05C4, &mut bt, &r) {
            Some(Seen::Status(s)) => s,
            _ => panic!(),
        };
        assert_eq!((s.level, s.charge), (Some(50), Charge::Charging));
        // Adapter with no controller paired reports a zeroed byte.
        r[30] = 0;
        assert_eq!(decode(0x0BA0, &mut bt, &r), Some(Seen::EmptyAdapter));
    }

    #[test]
    fn bt_report_switches_format() {
        // DualShock 4 over Bluetooth: report 11, byte 32.
        let mut r = vec![0x11u8; 78];
        r[32] = 0x0A | 0x10; // level 10 on cable -> 100 %
        let mut bt = false;
        let s = match decode(0x09CC, &mut bt, &r) {
            Some(Seen::Status(s)) => s,
            _ => panic!(),
        };
        assert_eq!((s.level, s.charge), (Some(100), Charge::Charging));
        assert!(bt);
    }

    #[test]
    fn silence_is_asleep() {
        let io = FakeHid::new(|_| vec![]);
        assert_eq!(poll(&io, 0x0CE6), None);
    }
}
