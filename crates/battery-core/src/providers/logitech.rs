//! Logitech HID++ 2.0 over the vendor HID collection: Unifying / Bolt /
//! Lightspeed receivers (device index 1..=6) and direct USB/Bluetooth (0xFF).
//! Only long reports (0x11) are used, so one HID collection is enough.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind};

pub const VID: u16 = 0x046D;
const LONG: u8 = 0x11;
const SHORT: u8 = 0x10;
const SW_ID: u8 = 0x0B;
const TIMEOUT_MS: u64 = 300;

pub const F_ROOT: u16 = 0x0000;
pub const F_DEVICE_INFO: u16 = 0x0003;
pub const F_DEVICE_NAME: u16 = 0x0005;
pub const F_BATTERY_STATUS: u16 = 0x1000;
pub const F_BATTERY_VOLTAGE: u16 = 0x1001;
pub const F_UNIFIED_BATTERY: u16 = 0x1004;

#[derive(Debug, PartialEq)]
pub enum Error {
    /// No answer: device asleep / out of range / slot empty.
    Timeout,
    /// HID++ error code (2.0 `0xFF` or 1.0 `0x8F` reply).
    Protocol(u8),
    Io,
}

/// One request/response on `feature_index`. Returns the 16 parameter bytes.
pub fn request(io: &dyn HidIo, index: u8, feature_index: u8, function: u8, params: &[u8]) -> Result<[u8; 16], Error> {
    let fsw = (function << 4) | SW_ID;
    let mut out = [0u8; 20];
    out[..4].copy_from_slice(&[LONG, index, feature_index, fsw]);
    out[4..4 + params.len()].copy_from_slice(params);
    io.write(&out).map_err(|_| Error::Io)?;
    let reply = read_matching(io, TIMEOUT_MS, |r| {
        // Bluetooth devices may answer with index ^ 0xFF (0x00 for 0xFF).
        if r.len() < 7 || !(r[0] == LONG || r[0] == SHORT) || (r[1] != index && r[1] != index ^ 0xFF) {
            return None;
        }
        match r[2] {
            0xFF if r[3] == feature_index && r[4] == fsw => Some(Err(Error::Protocol(r[5]))),
            0x8F if r[3] == feature_index && r[4] == fsw => Some(Err(Error::Protocol(r[5]))),
            f if f == feature_index && r[3] == fsw => {
                let mut p = [0u8; 16];
                let n = (r.len() - 4).min(16);
                p[..n].copy_from_slice(&r[4..4 + n]);
                Some(Ok(p))
            }
            _ => None,
        }
    })
    .map_err(|_| Error::Io)?;
    reply.unwrap_or(Err(Error::Timeout))
}

/// IRoot.getFeature: index of `feature` on this device, None if unsupported.
pub fn feature_index(io: &dyn HidIo, index: u8, feature: u16) -> Result<Option<u8>, Error> {
    let p = request(io, index, 0, 0, &feature.to_be_bytes())?;
    Ok((p[0] != 0).then_some(p[0]))
}

/// IRoot.ping (getProtocolVersion). Ok(major) when a HID++ 2.0+ device answers.
pub fn ping(io: &dyn HidIo, index: u8) -> Result<u8, Error> {
    request(io, index, 0, 1, &[0, 0, 0x5A]).map(|p| p[0])
}

pub fn device_name(io: &dyn HidIo, index: u8, fi: u8) -> Option<String> {
    let len = request(io, index, fi, 0, &[]).ok()?[0] as usize;
    let mut name = Vec::with_capacity(len);
    while name.len() < len {
        let chunk = request(io, index, fi, 1, &[name.len() as u8]).ok()?;
        let take = (len - name.len()).min(16);
        name.extend_from_slice(&chunk[..take]);
    }
    Some(String::from_utf8_lossy(&name).trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
}

/// DEVICE_FW_VERSION.getDeviceInfo: [entity count, unit id (4), ...]. The unit
/// id is the device's serial: identical over a receiver, USB cable or Bluetooth,
/// so one physical device keeps one identity when the user switches link.
pub fn unit_id(io: &dyn HidIo, index: u8) -> Option<u32> {
    let fi = feature_index(io, index, F_DEVICE_INFO).ok().flatten()?;
    let p = request(io, index, fi, 0, &[]).ok()?;
    let id = u32::from_be_bytes([p[1], p[2], p[3], p[4]]);
    (id != 0 && id != u32::MAX).then_some(id)
}

/// Stable device id: unit id when the device reports one, else its name
/// (never the receiver PID or slot, which change with the connection).
pub fn stable_id(unit: Option<u32>, name: &str) -> String {
    match unit {
        Some(u) => format!("logitech:{u:08x}"),
        None => format!("logitech:{}", name.to_lowercase()),
    }
}

/// DEVICE_NAME.getDeviceType
pub fn device_kind(io: &dyn HidIo, index: u8, fi: u8) -> DeviceKind {
    match request(io, index, fi, 2, &[]).map(|p| p[0]) {
        Ok(0 | 2) => DeviceKind::Keyboard,
        Ok(3..=5) => DeviceKind::Mouse,
        _ => DeviceKind::Other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryFeature {
    Status,
    Voltage,
    Unified,
}

impl BatteryFeature {
    pub const PREFERENCE: [(u16, BatteryFeature); 3] = [
        (F_UNIFIED_BATTERY, BatteryFeature::Unified),
        (F_BATTERY_STATUS, BatteryFeature::Status),
        (F_BATTERY_VOLTAGE, BatteryFeature::Voltage),
    ];

    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn from_code(c: u8) -> Option<Self> {
        [Self::Status, Self::Voltage, Self::Unified].into_iter().find(|f| f.code() == c)
    }
}

pub fn read_battery(io: &dyn HidIo, index: u8, fi: u8, feature: BatteryFeature) -> Result<BatteryStatus, Error> {
    Ok(match feature {
        BatteryFeature::Status => decode_status(&request(io, index, fi, 0, &[])?),
        BatteryFeature::Unified => decode_unified(&request(io, index, fi, 1, &[])?),
        BatteryFeature::Voltage => decode_voltage(&request(io, index, fi, 0, &[])?),
    })
}

/// 0x1000 getBatteryLevelStatus: [level %, next level %, status]
pub fn decode_status(p: &[u8; 16]) -> BatteryStatus {
    let charge = match p[2] {
        0 => Charge::Discharging,
        1 | 2 | 4 => Charge::Charging,
        3 => Charge::Full,
        _ => Charge::Unknown,
    };
    let level = match (p[0], charge) {
        (0, Charge::Full) => Some(100),
        (0, _) => None, // many devices report 0 while charging = unknown
        (l, _) => Some(l.min(100)),
    };
    BatteryStatus { level, charge, link: crate::Link::Connected }
}

/// 0x1004 get_status: [state of charge %, level flags, charging status, ext power]
pub fn decode_unified(p: &[u8; 16]) -> BatteryStatus {
    let charge = match p[2] {
        0 => Charge::Discharging,
        1 | 2 => Charge::Charging,
        3 => Charge::Full,
        _ => Charge::Unknown,
    };
    // Devices without percentage support report soc=0 and only level flags.
    let level = if p[0] > 0 {
        Some(p[0].min(100))
    } else {
        match p[1] {
            f if f & 0x08 != 0 => Some(90),
            f if f & 0x04 != 0 => Some(50),
            f if f & 0x02 != 0 => Some(20),
            f if f & 0x01 != 0 => Some(5),
            _ => None,
        }
    };
    BatteryStatus { level, charge, link: crate::Link::Connected }
}

/// 0x1001 getBatteryVoltage: [mV hi, mV lo, flags]. Bit 7 of flags = external
/// power; bits 0..2 then give the phase: 0 charging, 1 full, 2 not charging.
pub fn decode_voltage(p: &[u8; 16]) -> BatteryStatus {
    let mv = u16::from_be_bytes([p[0], p[1]]);
    let charge = match (p[2] & 0x80 != 0, p[2] & 0x07) {
        (false, _) => Charge::Discharging,
        (true, 1) => Charge::Full,
        (true, 0) => Charge::Charging,
        (true, _) => Charge::Unknown,
    };
    BatteryStatus { level: voltage_to_percent(mv), charge, link: crate::Link::Connected }
}

/// Typical single-cell Li-ion discharge curve (mV, %), linear in between.
pub fn voltage_to_percent(mv: u16) -> Option<u8> {
    const CURVE: [(u16, u8); 13] = [
        (4186, 100), (4067, 90), (3989, 80), (3922, 70), (3859, 60), (3811, 50), (3778, 40),
        (3751, 30), (3717, 20), (3671, 10), (3646, 5), (3579, 2), (3500, 0),
    ];
    if mv == 0 {
        return None;
    }
    if mv >= CURVE[0].0 {
        return Some(100);
    }
    for w in CURVE.windows(2) {
        let ((hv, hp), (lv, lp)) = (w[0], w[1]);
        if mv >= lv {
            let t = (mv - lv) as f32 / (hv - lv) as f32;
            return Some((lp as f32 + t * (hp - lp) as f32).round() as u8);
        }
    }
    Some(0)
}

#[cfg(windows)]
pub use win::Logitech;

#[cfg(windows)]
mod win {
    use super::*;
    use crate::hid::win::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Receivers expose up to 6 paired devices; anything else is direct (0xFF).
    const RECEIVER_SLOTS: std::ops::RangeInclusive<u8> = 1..=6;

    #[derive(Default)]
    pub struct Logitech {
        /// Devices seen while their receiver is plugged in: a silent one is asleep, not gone.
        known: Mutex<BTreeMap<String, Device>>,
    }

    fn is_hidpp_collection(d: &hidapi::DeviceInfo) -> bool {
        d.vendor_id() == VID
            && matches!((d.usage_page(), d.usage()), (0xFF00, 0x0002) | (0xFF43, 0x0202))
    }

    /// Unifying, Bolt, Lightspeed and Nano receivers all live in 0xC5xx.
    fn is_receiver(d: &hidapi::DeviceInfo) -> bool {
        (0xC500..=0xC5FF).contains(&d.product_id())
    }

    impl Logitech {
        fn probe(&self, io: &dyn HidIo, path: &str, pid: u16, index: u8, fallback: &str) -> Option<Device> {
            ping(io, index).ok()?;
            let (fi, kind) = BatteryFeature::PREFERENCE
                .iter()
                .find_map(|&(f, k)| feature_index(io, index, f).ok().flatten().map(|fi| (fi, k)))?;
            let name_fi = feature_index(io, index, F_DEVICE_NAME).ok().flatten();
            let name = name_fi.and_then(|n| device_name(io, index, n)).unwrap_or_else(|| fallback.to_string());
            let dkind = match name_fi.map_or(DeviceKind::Other, |n| device_kind(io, index, n)) {
                DeviceKind::Other => crate::guess_kind(&name), // headsets have no device type
                k => k,
            };
            let unit = unit_id(io, index);
            log::debug!("logitech {pid:04x} {path} idx {index}: \"{name}\" unit {unit:08x?} {dkind:?}, battery feature {kind:?} at {fi}");
            Some(Device {
                id: stable_id(unit, &name),
                name: name.clone(),
                vendor: "Logitech".into(),
                model: Some(name),
                kind: dkind,
                provider: "logitech",
                locator: format!("{path}\t{index}\t{fi}\t{}", kind.code()),
            })
        }
    }

    impl BatteryProvider for Logitech {
        fn name(&self) -> &'static str {
            "logitech"
        }

        fn discover(&self) -> Vec<Device> {
            // Devices that answered now, then silent receiver slots shown as asleep.
            let (mut found, mut silent): (Vec<Device>, Vec<Device>) = (Vec::new(), Vec::new());
            let mut known = self.known.lock().unwrap();
            let interfaces: Vec<_> = enumerate().into_iter().filter(is_hidpp_collection).collect();
            for info in &interfaces {
                let path = info.path().to_string_lossy().into_owned();
                let Some(io) = open(&path) else { continue };
                let product = info.product_string().unwrap_or("Logitech device").to_string();
                let receiver = is_receiver(info);
                let slots: Vec<u8> = if receiver { RECEIVER_SLOTS.collect() } else { vec![0xFF] };
                for index in slots {
                    let prefix = format!("{path}\t{index}\t");
                    match self.probe(&io, &path, info.product_id(), index, &product) {
                        Some(dev) => {
                            known.insert(prefix, dev.clone());
                            found.push(dev);
                        }
                        // Receiver still here but the device is silent: keep it, reads will say Asleep.
                        None if receiver => silent.extend(known.get(&prefix).cloned()),
                        None => {}
                    }
                }
            }
            // Forget devices whose receiver/interface is gone.
            known.retain(|k, _| interfaces.iter().any(|i| k.starts_with(&*i.path().to_string_lossy())));
            // Same device on two links (e.g. charging by cable while still paired to the
            // receiver): keep the link that answered; one card per physical device.
            for d in silent {
                if !found.iter().any(|f| f.id == d.id) {
                    found.push(d);
                }
            }
            let mut seen = std::collections::HashSet::new();
            found.retain(|d| seen.insert(d.id.clone()));
            found
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let (Some(path), Some(index), Some(fi), Some(kind)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
                return BatteryStatus::GONE;
            };
            let (Ok(index), Ok(fi), Some(kind)) =
                (index.parse::<u8>(), fi.parse::<u8>(), kind.parse().ok().and_then(BatteryFeature::from_code))
            else {
                return BatteryStatus::GONE;
            };
            let Some(io) = open(path) else { return BatteryStatus::GONE };
            match super::read_battery(&io, index, fi, kind) {
                Ok(s) => s,
                Err(Error::Timeout) | Err(Error::Protocol(_)) => BatteryStatus::ASLEEP,
                Err(Error::Io) => BatteryStatus::GONE,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    /// Simulated receiver with one mouse on slot 1 supporting UNIFIED_BATTERY at index 6.
    fn mouse() -> FakeHid {
        FakeHid::new(|req| {
            if req[0] != LONG || req[1] != 1 {
                return vec![]; // empty slot: receiver stays silent on the long channel
            }
            let (fi, fsw) = (req[2], req[3]);
            let mut r = vec![LONG, 1, fi, fsw];
            r.resize(20, 0);
            match (fi, fsw >> 4) {
                (0, 1) => r[4] = 4,                                            // ping: HID++ 4.x
                (0, 0) if req[4..6] == [0x10, 0x04] => r[4] = 6,               // getFeature(0x1004)
                (0, 0) if req[4..6] == [0x00, 0x05] => r[4] = 3,               // getFeature(0x0005)
                (0, 0) if req[4..6] == [0x00, 0x03] => r[4] = 4,               // getFeature(0x0003)
                (4, 0) => r[4..9].copy_from_slice(&[1, 0xDE, 0xAD, 0xBE, 0xEF]), // unit id
                (0, 0) => r[4] = 0,                                            // unsupported
                (6, 1) => r[4..8].copy_from_slice(&[73, 0x04, 1, 1]),           // 73 %, charging
                (3, 0) => r[4] = 3,                                            // name length
                (3, 1) => r[4..7].copy_from_slice(b"G42"),
                (3, 2) => r[4] = 3,                                            // mouse
                _ => {
                    // HID++ 2.0 error: invalid function
                    r = vec![LONG, 1, 0xFF, fi, fsw, 0x07];
                    r.resize(20, 0);
                }
            }
            // Noise first: an unrelated mouse report must be skipped.
            vec![vec![0x02, 0, 0, 0, 0, 0, 0], r]
        })
    }

    #[test]
    fn full_exchange_against_simulated_device() {
        let io = mouse();
        assert_eq!(ping(&io, 1), Ok(4));
        assert_eq!(ping(&io, 2), Err(Error::Timeout));
        assert_eq!(feature_index(&io, 1, F_UNIFIED_BATTERY), Ok(Some(6)));
        assert_eq!(feature_index(&io, 1, F_BATTERY_STATUS), Ok(None));
        assert_eq!(device_name(&io, 1, 3).as_deref(), Some("G42"));
        assert_eq!(device_kind(&io, 1, 3), DeviceKind::Mouse);
        let s = read_battery(&io, 1, 6, BatteryFeature::Unified).unwrap();
        assert_eq!((s.level, s.charge), (Some(73), Charge::Charging));
        assert_eq!(request(&io, 1, 9, 0, &[]), Err(Error::Protocol(0x07)));
        // Same unit id -> same identity whatever receiver/slot/cable it arrives on.
        assert_eq!(stable_id(unit_id(&io, 1), "PRO X 2"), "logitech:deadbeef");
        assert_eq!(stable_id(None, "PRO X 2"), "logitech:pro x 2");
    }

    #[test]
    fn decoders() {
        let mut p = [0u8; 16];
        p[..3].copy_from_slice(&[0, 0, 3]);
        assert_eq!(decode_status(&p).level, Some(100));
        p[..3].copy_from_slice(&[0, 0, 1]);
        assert_eq!(decode_status(&p).level, None);
        p[..3].copy_from_slice(&[0, 0x08, 4]); // full flag, charging error
        assert_eq!((decode_unified(&p).level, decode_unified(&p).charge), (Some(90), Charge::Unknown));
        p[..3].copy_from_slice(&[0, 0x02, 0]);
        assert_eq!(decode_unified(&p).level, Some(20));
        p[..3].copy_from_slice(&[0x0F, 0x1E, 0x00]); // 3870 mV
        let v = decode_voltage(&p);
        assert_eq!((v.level, v.charge), (Some(62), Charge::Discharging));
        assert_eq!(voltage_to_percent(4300), Some(100));
        assert_eq!(voltage_to_percent(3400), Some(0));
    }
}
