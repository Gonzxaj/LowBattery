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

/// Logitech G435 LIGHTSPEED headset on the newer 046D:0ADF dongle. It does
/// not speak HID++: its vendor collection (FF00:0001, unnumbered 64-byte
/// reports) carries CRC-16 framed messages, and "PA" commands inside them
/// reach the headset's radio SoC. The battery is read with the OTA
/// GET_BATTERY command (millivolts). The older 046D:0ACB dongle uses a
/// different transport and is not handled here.
///
/// Session, as captured by portyporty: open (84 01), session init (85 01,
/// OTA IOCTL_READ_MAX_LEN), REMOTE_GET_MODE / REMOTE_SET_MODE 01 /
/// REMOTE_GET_MODE so the query is forwarded to the headset rather than
/// answered by the dongle, GET_BATTERY, close (85 02). SET_MODE 01 only
/// selects where the dongle routes the command; it is not a user setting and
/// does not persist. The fixed waits between steps come from the original
/// capture (about 1.2 s in total). Unlike the 0ACB dongle, a read does not
/// interrupt audio or microphone, so the headset is read on every poll.
/// Protocol: github.com/portyporty/g435-battery-percentage (MIT), ported in
/// github.com/deddroid/logitech-g435-battery; reimplemented here.
pub mod g435 {
    use crate::hid::{read_matching, HidIo};
    use crate::{BatteryStatus, Charge, Link};

    pub const PID: u16 = 0x0ADF;

    const TIMEOUT_OPEN_MS: u64 = 50;
    const TIMEOUT_INIT_MS: u64 = 450;
    const TIMEOUT_PA_MS: u64 = 120;
    const TIMEOUT_BATTERY_MS: u64 = 250;
    const TIMEOUT_CLOSE_MS: u64 = 50;

    const CAT_OTA: u8 = 0x06;
    const CAT_WIRELESS: u8 = 0x0E;
    const GET_MODE: u8 = 0xE0;
    const SET_MODE: u8 = 0xE1;

    /// Session init from the capture (OTA_CMD_IOCTL_READ_MAX_LEN, sequence 1).
    const SESSION_INIT: [u8; 19] = [
        0x50, 0x41, 0x06, 0x01, 0x0D, 0x00, 0x25, 0x34, 0x12, 0x5A, 0x5A, 0x01, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00,
    ];

    pub fn supported(vid: u16, pid: u16) -> bool {
        vid == super::VID && pid == PID
    }

    /// CRC-16/XMODEM (poly 0x1021, init 0).
    pub fn crc16(data: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &b in data {
            crc ^= (b as u16) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
        }
        crc
    }

    /// 64-byte frame: <cmd> <body length> <crc lo> <crc hi> <body...>, zero-padded.
    /// The CRC covers the whole frame with the CRC field zeroed.
    pub fn frame(cmd: u8, body: &[u8]) -> [u8; 64] {
        let mut f = [0u8; 64];
        f[0] = cmd;
        f[1] = body.len() as u8;
        for (i, &b) in body.iter().take(60).enumerate() {
            f[4 + i] = b;
        }
        let crc = crc16(&f);
        f[2] = crc as u8;
        f[3] = (crc >> 8) as u8;
        f
    }

    /// "PA" command for the headset's radio SoC:
    /// 50 41 <category> <seq> <length field> <payload...>.
    pub fn pa(category: u8, seq: u8, len: u8, payload: &[u8]) -> Vec<u8> {
        let mut cmd = vec![0x50, 0x41, category, seq, len];
        cmd.extend_from_slice(payload);
        cmd
    }

    /// Battery voltage from the GET_BATTERY reply (pattern
    /// 00 07 00 06 <seq> 00 <mV lo> <mV hi>). None when absent or not a
    /// plausible Li-Po voltage.
    pub fn parse_millivolts(data: &[u8]) -> Option<i32> {
        for i in 0..data.len().saturating_sub(7) {
            if data[i] == 0x00 && data[i + 1] == 0x07 && data[i + 2] == 0x00 && data[i + 3] == 0x06 && data[i + 5] == 0x00 {
                let mv = data[i + 6] as i32 | ((data[i + 7] as i32) << 8);
                if (2500..=4500).contains(&mv) {
                    return Some(mv);
                }
            }
        }
        None
    }

    /// Voltage -> % for the G435 Li-Po cell, piecewise-linear.
    pub fn percent(mv: i32) -> u8 {
        const V: [i32; 16] = [4200, 4150, 4100, 4050, 4000, 3950, 3900, 3850, 3800, 3750, 3700, 3650, 3600, 3550, 3500, 3400];
        const P: [i32; 16] = [100, 95, 90, 85, 80, 75, 70, 62, 55, 45, 35, 25, 15, 10, 5, 0];
        if mv >= V[0] {
            return 100;
        }
        for i in 1..16 {
            if mv >= V[i] {
                let span = V[i - 1] - V[i];
                let t = if span > 0 { (mv - V[i]) as f64 / span as f64 } else { 0.0 };
                return (P[i] as f64 + t * (P[i - 1] - P[i]) as f64).round().clamp(0.0, 100.0) as u8;
            }
        }
        0
    }

    /// Writes one frame (report id 0) and drains the input reports that
    /// arrive within `wait_ms`. Returns false only when the write fails.
    fn transact(io: &dyn HidIo, cmd: u8, body: &[u8], wait_ms: u64) -> bool {
        let f = frame(cmd, body);
        let mut out = vec![0u8; 65];
        out[1..].copy_from_slice(&f);
        if io.write(&out).is_err() {
            return false;
        }
        let _ = read_matching(io, wait_ms, |_: &[u8]| -> Option<i32> { None });
        true
    }

    /// Same, but returns the first battery millivolts seen in the replies.
    fn transact_mv(io: &dyn HidIo, cmd: u8, body: &[u8], wait_ms: u64) -> Option<i32> {
        let f = frame(cmd, body);
        let mut out = vec![0u8; 65];
        out[1..].copy_from_slice(&f);
        io.write(&out).ok()?;
        read_matching(io, wait_ms, parse_millivolts).ok().flatten()
    }

    /// Full session; always closes (85 02) once it was opened. Intermediate
    /// step failures are ignored like upstream: only the final battery reply
    /// matters.
    fn read_millivolts(io: &dyn HidIo) -> Option<i32> {
        if !transact(io, 0x84, &[0x01], TIMEOUT_OPEN_MS) {
            return None;
        }
        let inner = {
            transact(io, 0x85, &[0x01], TIMEOUT_OPEN_MS);
            transact(io, 0x80, &SESSION_INIT, TIMEOUT_INIT_MS);
            let mut seq = 2u8;
            let mut pa_cmd = |cat: u8, len: u8, payload: &[u8]| {
                let c = pa(cat, seq, len, payload);
                seq = seq % 127 + 1;
                c
            };
            transact(io, 0x80, &pa_cmd(CAT_WIRELESS, 1, &[GET_MODE]), TIMEOUT_PA_MS);
            transact(io, 0x80, &pa_cmd(CAT_WIRELESS, 2, &[SET_MODE, 0x01]), TIMEOUT_PA_MS);
            transact(io, 0x80, &pa_cmd(CAT_WIRELESS, 1, &[GET_MODE]), TIMEOUT_PA_MS);
            transact_mv(io, 0x80, &pa_cmd(CAT_OTA, 1, &[0x00, 0x31]), TIMEOUT_BATTERY_MS)
        };
        transact(io, 0x85, &[0x02], TIMEOUT_CLOSE_MS);
        inner
    }

    /// Dongle present: a millivolt reading means the headset answered;
    /// silence means it is off or out of range.
    pub fn poll(io: &dyn HidIo) -> BatteryStatus {
        match read_millivolts(io) {
            Some(mv) => BatteryStatus { level: Some(percent(mv)), charge: Charge::Unknown, link: Link::Connected },
            None => BatteryStatus::ASLEEP,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::providers::fake::FakeHid;

        #[test]
        fn crc_known_vector() {
            // CRC-16/XMODEM of "123456789" = 0x31C3.
            assert_eq!(crc16(b"123456789"), 0x31C3);
        }

        #[test]
        fn frame_layout() {
            let f = frame(0x80, &[1, 2, 3]);
            assert_eq!(f[0], 0x80);
            assert_eq!(f[1], 3);
            assert_eq!(&f[4..7], &[1, 2, 3]);
            // CRC field is consistent: recomputing over the frame with it
            // zeroed gives the stored value.
            let mut z = f;
            z[2] = 0;
            z[3] = 0;
            assert_eq!(crc16(&z), u16::from_le_bytes([f[2], f[3]]));
        }

        #[test]
        fn millivolt_pattern() {
            let mut r = vec![0u8; 65];
            r[20] = 0x00;
            r[21] = 0x07;
            r[22] = 0x00;
            r[23] = 0x06;
            r[24] = 0x01;
            r[25] = 0x00;
            r[26] = 0xD8; // 3800 mV LE
            r[27] = 0x0E;
            assert_eq!(parse_millivolts(&r), Some(3800));
            assert_eq!(parse_millivolts(&[0u8; 65]), None);
        }

        #[test]
        fn percent_curve() {
            assert_eq!(percent(4200), 100);
            assert_eq!(percent(3400), 0);
            let p = percent(3800);
            assert!(p > 40 && p < 70);
        }

        /// Dongle answering the full session; battery reply only on the last step.
        fn session(mv: Option<i32>) -> FakeHid {
            FakeHid::new(move |req| {
                // req[0] = report id 0, frame starts at req[1]: cmd, body len, crc, body
                if req.len() < 5 || req[0] != 0x00 {
                    return vec![];
                }
                let cmd = req[1];
                let body = &req[5..5 + req[2] as usize];
                let is_battery = cmd == 0x80 && body.len() >= 5 && &body[..2] == b"PA" && body[2] == CAT_OTA;
                if is_battery {
                    return mv
                        .map(|mv| {
                            let mut r = vec![0u8; 65];
                            r[30] = 0x00;
                            r[31] = 0x07;
                            r[32] = 0x00;
                            r[33] = 0x06;
                            r[34] = 0x01;
                            r[35] = 0x00;
                            r[36] = (mv & 0xFF) as u8;
                            r[37] = ((mv >> 8) & 0xFF) as u8;
                            vec![r]
                        })
                        .unwrap_or_default();
                }
                vec![vec![0u8; 65]] // noise
            })
        }

        #[test]
        fn session_reads_battery() {
            let s = poll(&session(Some(3800)));
            assert_eq!(s.link, Link::Connected);
            assert!(s.level.is_some());
            assert_eq!(s.charge, Charge::Unknown);
        }

        #[test]
        fn session_silence_is_asleep() {
            assert_eq!(poll(&session(None)), BatteryStatus::ASLEEP);
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Logitech;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    #[cfg(windows)]
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// Receivers expose up to 6 paired devices; anything else is direct (0xFF).
    #[cfg(windows)]
    const RECEIVER_SLOTS: std::ops::RangeInclusive<u8> = 1..=6;

    #[derive(Default)]
    pub struct Logitech {
        /// HID++ devices seen while their receiver is plugged in: a silent
        /// one is asleep, not gone. Windows only: on Linux the kernel's
        /// power_supply class already reports HID++ batteries.
        #[cfg(windows)]
        known: Mutex<BTreeMap<String, Device>>,
        /// G435 dongle seen while plugged in.
        g435_known: Mutex<Option<Device>>,
    }

    #[cfg(windows)]
    fn is_hidpp_collection(d: &hidapi::DeviceInfo) -> bool {
        d.vendor_id() == VID
            && matches!((d.usage_page(), d.usage()), (0xFF00, 0x0002) | (0xFF43, 0x0202))
    }

    /// Unifying, Bolt, Lightspeed and Nano receivers all live in 0xC5xx.
    #[cfg(windows)]
    fn is_receiver(d: &hidapi::DeviceInfo) -> bool {
        (0xC500..=0xC5FF).contains(&d.product_id())
    }

    impl Logitech {
        #[cfg(windows)]
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

        /// HID++ devices behind receivers/direct links. Windows only: on
        /// Linux the kernel's power_supply class reports them already.
        #[cfg(windows)]
        fn discover_hidpp(&self) -> Vec<Device> {
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
            found
        }

        #[cfg(not(windows))]
        fn discover_hidpp(&self) -> Vec<Device> {
            Vec::new()
        }

        /// G435 LIGHTSPEED dongle (046D:0ADF): not HID++, works on every
        /// platform through hidraw/hidapi.
        fn discover_g435(&self, out: &mut Vec<Device>) {
            let infos: Vec<_> = enumerate()
                .into_iter()
                .filter(|d| g435::supported(d.vendor_id(), d.product_id()) && d.usage_page() == 0xFF00 && d.usage() == 0x0001)
                .collect();
            if infos.is_empty() {
                self.g435_known.lock().unwrap().take();
                return;
            }
            let found = infos.iter().find_map(|d| {
                let path = d.path().to_string_lossy().into_owned();
                let io = open(&path)?;
                let s = g435::poll(&io);
                log::debug!("logitech g435 if{}: {s:?}", d.interface_number());
                Some((path, s))
            });
            match found {
                Some((path, _)) => {
                    let dev = Device {
                        id: "logitech:g435".into(),
                        name: "Logitech G435".into(),
                        vendor: "Logitech".into(),
                        model: Some("G435 LIGHTSPEED".into()),
                        kind: DeviceKind::Headset,
                        provider: "logitech",
                        locator: format!("g435\t{path}"),
                    };
                    *self.g435_known.lock().unwrap() = Some(dev.clone());
                    out.push(dev);
                }
                None => out.extend(self.g435_known.lock().unwrap().clone()),
            }
        }
    }

    impl BatteryProvider for Logitech {
        fn name(&self) -> &'static str {
            "logitech"
        }

        fn discover(&self) -> Vec<Device> {
            let mut found = self.discover_hidpp();
            self.discover_g435(&mut found);
            // Same device on two links keeps one card.
            let mut seen = std::collections::HashSet::new();
            found.retain(|d| seen.insert(d.id.clone()));
            found
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let Some(first) = parts.next() else { return BatteryStatus::GONE };
            if first == "g435" {
                let Some(path) = parts.next() else { return BatteryStatus::GONE };
                let Some(io) = open(path) else { return BatteryStatus::GONE };
                return g435::poll(&io);
            }
            let (Some(path), Some(index), Some(fi), Some(kind)) =
                (Some(first), parts.next(), parts.next(), parts.next())
            else {
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
