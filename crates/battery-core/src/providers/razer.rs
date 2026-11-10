//! Razer wireless mice/keyboards (and their dongles) via the 90-byte Razer
//! feature report. Protocol facts per OpenRazer (GPL-2; no code taken).
//! Model-agnostic: any Razer HID collection that answers the battery query
//! with a valid reply becomes a device, so new models work without a table.
//! Razer headsets use other protocols and are not covered.

use crate::{BatteryStatus, Charge};

pub const VID: u16 = 0x1532;
const LEN: usize = 90;
/// Different generations use different transaction ids; probed in this order.
pub const TRANSACTION_IDS: [u8; 3] = [0x1F, 0x3F, 0xFF];

const STATUS_OK: u8 = 0x02;
const STATUS_BUSY: u8 = 0x01;
/// The dongle answers but the device behind it does not (asleep / off).
const STATUS_TIMEOUT: u8 = 0x04;

/// [status, txn, remaining(2), protocol, size, class, id, args(80), crc, reserved]
pub fn request(txn: u8, class: u8, id: u8, size: u8) -> [u8; LEN] {
    let mut r = [0u8; LEN];
    r[1] = txn;
    r[5] = size;
    r[6] = class;
    r[7] = id;
    r[88] = crc(&r);
    r
}

/// XOR of bytes 2..88.
pub fn crc(r: &[u8; LEN]) -> u8 {
    r[2..88].iter().fold(0, |a, b| a ^ b)
}

#[derive(Debug, PartialEq)]
pub enum Reply {
    Ok([u8; LEN]),
    Asleep,
    Invalid,
}

/// Validate a reply against the request it answers.
pub fn check(req: &[u8; LEN], reply: &[u8]) -> Reply {
    let Ok(r) = <[u8; LEN]>::try_from(reply.get(..LEN).unwrap_or(&[])) else { return Reply::Invalid };
    if r[1] != req[1] || r[6] != req[6] || r[7] != req[7] {
        return Reply::Invalid;
    }
    match r[0] {
        STATUS_OK => Reply::Ok(r),
        STATUS_TIMEOUT => Reply::Asleep,
        _ => Reply::Invalid,
    }
}

/// Battery: class 0x07 id 0x80, arg[1] = 0..255. Charging: class 0x07 id 0x84, arg[1] = 1.
pub const BATTERY: (u8, u8) = (0x07, 0x80);
pub const CHARGING: (u8, u8) = (0x07, 0x84);

pub fn level_percent(raw: u8) -> u8 {
    ((raw as u16 * 100 + 127) / 255) as u8
}

/// One exchange over any feature-report transport.
pub fn exchange(io: &dyn crate::hid::HidIo, txn: u8, (class, id): (u8, u8)) -> Reply {
    let req = request(txn, class, id, 0x02);
    for _ in 0..3 {
        let mut out = [0u8; LEN + 1]; // report id 0 + payload
        out[1..].copy_from_slice(&req);
        if io.send_feature(&out).is_err() {
            return Reply::Invalid;
        }
        // Devices need a moment to prepare the answer; dongles relay over the air.
        std::thread::sleep(std::time::Duration::from_millis(60));
        let mut buf = [0u8; LEN + 1];
        match io.get_feature(&mut buf) {
            Ok(n) if n >= LEN => {
                // Some stacks return the report id first, some don't.
                let body = if n > LEN { &buf[1..] } else { &buf[..LEN] };
                if body.first() == Some(&STATUS_BUSY) {
                    continue;
                }
                return check(&req, body);
            }
            _ => return Reply::Invalid,
        }
    }
    Reply::Invalid
}

pub fn read(io: &dyn crate::hid::HidIo, txn: u8) -> Option<BatteryStatus> {
    let level = match exchange(io, txn, BATTERY) {
        Reply::Ok(r) => level_percent(r[9]),
        Reply::Asleep => return Some(BatteryStatus::ASLEEP),
        Reply::Invalid => return None,
    };
    let charge = match exchange(io, txn, CHARGING) {
        Reply::Ok(r) if r[9] == 1 => Charge::Charging,
        Reply::Ok(_) => Charge::Discharging,
        _ => Charge::Unknown,
    };
    Some(BatteryStatus::connected(level, charge))
}

#[cfg(windows)]
pub use win::Razer;

#[cfg(windows)]
mod win {
    use super::*;
    use crate::hid::win::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Razer {
        /// pid -> device seen this run (so a sleeping mouse stays as "asleep").
        known: Mutex<BTreeMap<u16, Device>>,
    }

    impl BatteryProvider for Razer {
        fn name(&self) -> &'static str {
            "razer"
        }

        fn discover(&self) -> Vec<Device> {
            let infos: Vec<_> = enumerate().into_iter().filter(|d| d.vendor_id() == VID).collect();
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let mut pids: Vec<u16> = infos.iter().map(|d| d.product_id()).collect();
            pids.sort_unstable();
            pids.dedup();
            for pid in pids {
                // Already found and its collection is still there: no re-probing (probes cost ~60 ms each).
                if let Some(dev) = known.get(&pid) {
                    let path = dev.locator.split('\t').next().unwrap_or("");
                    if infos.iter().any(|d| d.path().to_string_lossy() == path) {
                        out.push(dev.clone());
                        continue;
                    }
                }
                // Control collection is usually on interface 0 (mouse/keyboard); try those first.
                let mut cols: Vec<_> = infos.iter().filter(|d| d.product_id() == pid).collect();
                cols.sort_by_key(|d| (d.interface_number() != 0, d.usage_page() != 0x0001));
                let found = cols.iter().find_map(|d| {
                    let path = d.path().to_string_lossy().into_owned();
                    let io = open(&path)?;
                    TRANSACTION_IDS.iter().find_map(|&txn| {
                        let r = read(&io, txn);
                        log::debug!("razer {pid:04x} if{} {:04x}/{:04x} txn {txn:02x}: {r:?}", d.interface_number(), d.usage_page(), d.usage());
                        r.map(|_| (path.clone(), txn, d.product_string().unwrap_or("Razer device").to_string()))
                    })
                });
                match found {
                    Some((path, txn, product)) => {
                        let name = product.trim().to_string();
                        let dev = Device {
                            id: format!("razer:{pid:04x}"),
                            kind: crate::guess_kind(&name),
                            name,
                            vendor: "Razer".into(),
                            model: None,
                            provider: "razer",
                            locator: format!("{path}\t{txn}"),
                        };
                        known.insert(pid, dev.clone());
                        out.push(dev);
                    }
                    // Dongle still plugged in but silent: keep the last known card.
                    None => out.extend(known.get(&pid).cloned()),
                }
            }
            known.retain(|pid, _| infos.iter().any(|d| d.product_id() == *pid));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let Some((path, txn)) = device.locator.split_once('\t') else { return BatteryStatus::GONE };
            let (Some(io), Ok(txn)) = (open(path), txn.parse::<u8>()) else { return BatteryStatus::GONE };
            read(&io, txn).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;
    use crate::Link;

    /// Simulated Razer mouse answering only transaction id 0x1F, 80 % and charging.
    fn mouse() -> FakeHid {
        FakeHid::new(|out| {
            let req: [u8; LEN] = out[1..].try_into().unwrap();
            assert_eq!(req[88], crc(&req), "request crc");
            if req[1] != 0x1F {
                return vec![vec![0u8; LEN + 1]]; // status 0 = not handled
            }
            let mut r = req;
            r[0] = STATUS_OK;
            r[9] = match (req[6], req[7]) {
                BATTERY => 204, // 80 %
                CHARGING => 1,
                _ => 0,
            };
            r[88] = crc(&r);
            let mut with_id = vec![0u8];
            with_id.extend_from_slice(&r);
            vec![with_id]
        })
    }

    #[test]
    fn reads_battery_and_charging() {
        let io = mouse();
        assert_eq!(read(&io, 0x3F), None);
        let s = read(&io, 0x1F).unwrap();
        assert_eq!((s.level, s.charge, s.link), (Some(80), Charge::Charging, Link::Connected));
    }

    #[test]
    fn dongle_timeout_means_asleep_and_mismatch_is_invalid() {
        let req = request(0x1F, 0x07, 0x80, 0x02);
        let mut r = req;
        r[0] = STATUS_TIMEOUT;
        assert_eq!(check(&req, &r), Reply::Asleep);
        r[0] = STATUS_OK;
        r[7] = 0x81; // answer to another command
        assert_eq!(check(&req, &r), Reply::Invalid);
        assert_eq!(level_percent(255), 100);
        assert_eq!(level_percent(0), 0);
    }
}
