//! Razer BlackShark headsets and the Barracuda X — vendor protocols that
//! are NOT the standard 90-byte Razer feature report (handled by razer.rs).
//!
//!  * V3 / V3 Pro (1532:0576/0577/0579/057A), iface 5: 64-byte report 02,
//!    XOR checksum; handshake then GET class 0x21 (battery) / reply [13],
//!    [14] charging. Queried at most once per 5 minutes (the headset drops
//!    its 2.4 GHz link if polled faster); last reading reused.
//!  * V2 HyperSpeed dongle (1532:0565), FF14/FF00 usage 1: same 02 frame
//!    shape with a rolling sequence byte; cmd 0x21 battery / 0x2A charge.
//!  * V2 Pro 2023 (1532:0555), FF00: "PA" envelope; remote mode on/off
//!    around each query; type 03 cmd 0x21 battery / 0x2A cable.
//!  * Barracuda X (1532:0550), FF00:0001: YS-Tech bridge, CRC-16/XMODEM
//!    frames carrying Airoha commands; customer-data GET group 08 cmd 21.
//!  * V3 X (1532:057D dongle / 057C cable), iface 0: feature report 07,
//!    class 07 cmd 0x80 battery / 0x84 charging.
//!
//! Protocols from OpenRazer PRs #2794/#2862/#2899, HaloBattery,
//! justik13's V2 webhid driver and mehdibouchami/nightdevx's Barracuda X
//! docs; no code taken.

use crate::hid::{query, read_matching, HidIo};
use crate::{BatteryStatus, Charge, Link};
use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

pub const VID: u16 = 0x1532;
const V3_PIDS: [u16; 4] = [0x0576, 0x0577, 0x0579, 0x057A];
const HS_PID: u16 = 0x0565;
const V2PRO_PID: u16 = 0x0555;
const BARRACUDA_PID: u16 = 0x0550;
const V3X_PIDS: [u16; 2] = [0x057D, 0x057C];

fn name(pid: u16) -> &'static str {
    match pid {
        0x0576 | 0x0577 => "Razer BlackShark V3 Pro",
        0x0579 | 0x057A => "Razer BlackShark V3",
        HS_PID => "Razer BlackShark V2 HyperSpeed",
        V2PRO_PID => "Razer BlackShark V2 Pro (2023)",
        BARRACUDA_PID => "Razer Barracuda X (2022)",
        0x057D | 0x057C => "Razer BlackShark V3 X",
        _ => "Razer headset",
    }
}

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && (V3_PIDS.contains(&pid) || pid == HS_PID || pid == V2PRO_PID || pid == BARRACUDA_PID || V3X_PIDS.contains(&pid))
}

fn st(level: Option<u8>, charging: bool) -> BatteryStatus {
    BatteryStatus { level, charge: if charging { Charge::Charging } else { Charge::Discharging }, link: Link::Connected }
}

// ---------- V3 / V3 Pro (iface 5, report 02, XOR checksum) ----------

fn v3_packet(dir: u8, cls: u8) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x02;
    b[2] = 0x60;
    b[6] = 0x04;
    b[9] = dir;
    b[10] = cls;
    let crc = b[..62].iter().fold(0u8, |a, &x| a ^ x);
    b[62] = crc;
    b
}

fn v3_get(io: &dyn HidIo, cls: u8) -> io::Result<Option<Vec<u8>>> {
    query(io, &v3_packet(0x80, cls), &[64], 600, move |r| {
        (r.len() >= 15 && r[0] == 0x02 && r[2] == 0x60 && r[10] == cls).then(|| r.to_vec())
    })
}

fn poll_v3(io: &dyn HidIo) -> Option<BatteryStatus> {
    // Handshake the headset expects before reads.
    let _ = io.write(&v3_packet(0x00, 0x02));
    let _ = io.write(&v3_packet(0x00, 0x2A));
    let _ = io.write(&v3_packet(0x80, 0x2A));
    std::thread::sleep(Duration::from_millis(250));
    let r = v3_get(io, 0x21).ok().flatten()?;
    if r[13] > 100 {
        return None;
    }
    Some(st(Some(r[13]), r[14] != 0))
}

// ---------- V2 HyperSpeed (FF14/FF00 usage 1, rolling seq) ----------

fn hs_seq() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static SEQ: AtomicU8 = AtomicU8::new(0);
    0x60 | (SEQ.fetch_add(1, Ordering::Relaxed) & 0x1F)
}

fn hs_frame(cmd: u8, seq: u8) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x02;
    b[2] = seq;
    b[6] = 0x04;
    b[9] = 0x80;
    b[10] = cmd;
    let crc = b[..62].iter().fold(0u8, |a, &x| a ^ x);
    b[62] = crc;
    b
}

fn hs_query(io: &dyn HidIo, cmd: u8) -> Option<u8> {
    let seq = hs_seq();
    io.write(&hs_frame(cmd, seq)).ok()?;
    read_matching(io, 650, |r| {
        if r.len() < 64 || r[0] != 0x02 || r[2] != seq || r[10] != cmd || r[12] < 1 {
            return None;
        }
        let chk = r[1..62].iter().fold(0u8, |a, &x| a ^ x);
        (chk == r[62]).then_some(r[13])
    })
    .ok()
    .flatten()
}

fn poll_hs(io: &dyn HidIo) -> Option<BatteryStatus> {
    let level = hs_query(io, 0x21)?;
    if level > 100 {
        return None;
    }
    let charging = hs_query(io, 0x2A).map(|v| v > 0).unwrap_or(false);
    Some(st(Some(level), charging))
}

// ---------- V2 Pro 2023 (FF00, PA envelope) ----------

fn pa_frame(total: u8, inner: u8, ty: u8, cmd: u8, flag: u8) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[0] = 0x02;
    b[1] = 0x80;
    b[2] = total;
    b[5] = b'P';
    b[6] = b'A';
    b[7] = inner;
    b[9] = ty;
    b[10] = cmd;
    b[11] = flag;
    b
}

fn pa_remote(io: &dyn HidIo, on: bool) -> bool {
    io.write(&pa_frame(0x07, 0x0E, 0x02, 0xE1, u8::from(on))).is_ok()
}

fn pa_query(io: &dyn HidIo, cmd: u8) -> Option<u8> {
    pa_remote(io, true); // sacrificial wake
    for _ in 0..3 {
        pa_remote(io, true);
        if io.write(&pa_frame(0x08, 0x08, 0x03, cmd, 0)).is_err() {
            break;
        }
        if let Ok(Some(v)) = read_matching(io, 150, |r| {
            (r.len() >= 16 && r[0] == 0x02 && r[12] == cmd && r[13] == 0x01 && r[14] >= 1).then_some(r[15])
        }) {
            pa_remote(io, false);
            return Some(v);
        }
    }
    pa_remote(io, false);
    None
}

fn poll_v2pro(io: &dyn HidIo) -> Option<BatteryStatus> {
    let level = pa_query(io, 0x21)?;
    if level > 100 {
        return None;
    }
    let charging = pa_query(io, 0x2A).map(|v| v > 0).unwrap_or(false);
    Some(st(Some(level), charging))
}

// ---------- Barracuda X (FF00:0001, YS-Tech bridge, CRC-16/XMODEM) ----------

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

struct Barracuda<'a> {
    io: &'a dyn HidIo,
    stream: Vec<u8>,
    seq: u8,
}

impl<'a> Barracuda<'a> {
    fn command(&mut self, group: u8, body: &[u8]) -> bool {
        self.seq = self.seq % 127 + 1;
        let mut payload = vec![0x50, 0x41, group, self.seq];
        payload.extend_from_slice(body);
        let mut f = vec![0u8; 64];
        f[0] = 0x02;
        f[1] = 0x80;
        f[2] = payload.len() as u8;
        f[5..5 + payload.len()].copy_from_slice(&payload);
        let crc = crc16(&f);
        f[3] = (crc & 0xFF) as u8;
        f[4] = (crc >> 8) as u8;
        // Bridge busy-retry: the GetInputReport('O'/'F') ack is Windows-only;
        // on hidraw we rely on the stream await below to filter.
        self.io.write(&f).is_ok()
    }

    /// Waits for the first [50 49] stream frame accepted by `match`.
    fn await_frame(&mut self, timeout_ms: u64, want: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut buf = [0u8; 64];
        loop {
            // Drain complete frames from the reassembly buffer.
            while self.stream.len() >= 2 {
                let Some(start) = self.stream.windows(2).position(|w| w == [0x50, 0x49]) else {
                    self.stream.clear();
                    break;
                };
                self.stream.drain(..start);
                if self.stream.len() < 10 {
                    break;
                }
                let len = 10 + (self.stream[8] as usize | ((self.stream[9] as usize) << 8));
                if self.stream.len() < len {
                    break;
                }
                let frame: Vec<u8> = self.stream[..len].to_vec();
                self.stream.drain(..len);
                if want(&frame) {
                    return Some(frame);
                }
            }
            let left = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
            if left <= 0 {
                return None;
            }
            let n = self.io.read_timeout(&mut buf, left).ok()?;
            if n < 2 {
                return None;
            }
            // Input reports are [02][len][stream..].
            if buf[0] == 0x02 && (buf[1] as usize) + 2 <= n {
                self.stream.extend_from_slice(&buf[2..2 + buf[1] as usize]);
            }
        }
    }

    fn remote_mode(&mut self, on: bool) -> bool {
        if !self.command(0x0E, &[0x02, 0xE1, u8::from(on)]) {
            return false;
        }
        let seq = self.seq;
        self.await_frame(800, |r| r.len() >= 13 && r[10] == 0x0E && r[11] == (0x80 | seq)).is_some_and(|f| f[12] == 0)
    }

    fn get(&mut self, cmd: u8) -> Option<u8> {
        if !self.command(0x08, &[0x03, cmd, 0x00, 0x00]) {
            return None;
        }
        let f = self.await_frame(1000, |r| r.len() >= 13 && r[2] == 0x08 && r[10] == cmd && r[11] == 0x01)?;
        (f[12] >= 1 && f.len() >= 14).then_some(f[13])
    }
}

fn poll_barracuda(io: &dyn HidIo) -> Option<BatteryStatus> {
    let mut link = Barracuda { io, stream: Vec::new(), seq: 0 };
    if !link.remote_mode(true) {
        return None; // headset off / out of range / in Bluetooth mode
    }
    let level = link.get(0x21)?;
    let cable = link.get(0x2A);
    link.remote_mode(false);
    if level > 100 {
        return None;
    }
    Some(st(Some(level), cable == Some(1)))
}

// ---------- V3 X (iface 0, feature report 07) ----------

fn v3x_get(io: &dyn HidIo, cmd: u8) -> Option<u8> {
    use std::sync::atomic::{AtomicU8, Ordering};
    static TXID: AtomicU8 = AtomicU8::new(0x40);
    for _ in 0..4 {
        let txid = TXID.fetch_add(1, Ordering::Relaxed);
        let mut req = vec![0u8; 64];
        req[0] = 0x07;
        req[2] = txid;
        req[6] = 0x02;
        req[7] = 0x07;
        req[8] = cmd;
        let crc = req[3..=60].iter().fold(0u8, |a, &x| a ^ x);
        req[62] = crc;
        io.send_feature(&req).ok()?;
        std::thread::sleep(Duration::from_millis(60));
        let mut r = vec![0u8; 64];
        r[0] = 0x07;
        let Ok(n) = io.get_feature(&mut r) else { continue };
        if n < 63 || r[2] != txid || r[7] != 0x07 || r[8] != cmd || r[6] < 2 {
            continue;
        }
        if r[1] != 0x02 && r[1] != 0x01 {
            return None;
        }
        return Some(r[10]);
    }
    None
}

fn poll_v3x(io: &dyn HidIo) -> Option<BatteryStatus> {
    let level = v3x_get(io, 0x80)?;
    if level > 100 {
        return None;
    }
    let charging = v3x_get(io, 0x84) == Some(1);
    Some(st(Some(level), charging))
}

// ---------- state + I/O for the shared `razer` provider ----------

#[cfg(any(windows, target_os = "linux"))]
pub use native::{discover, read, Headsets};

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open, Dev};
    use crate::{Device, DeviceKind};
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    const MIN_INTERVAL: Duration = Duration::from_secs(300);

    /// 5-minute cache (V3/V3X) + devices seen this run, owned by `razer`.
    #[derive(Default)]
    pub struct Headsets {
        cache: Mutex<HashMap<u16, (Instant, BatteryStatus)>>,
        known: Mutex<BTreeMap<String, Device>>,
    }

    impl Headsets {
        fn cached(&self, pid: u16) -> Option<BatteryStatus> {
            let c = self.cache.lock().unwrap();
            c.get(&pid).filter(|(at, _)| at.elapsed() < MIN_INTERVAL).map(|(_, s)| *s)
        }

        fn store(&self, pid: u16, s: BatteryStatus) {
            self.cache.lock().unwrap().insert(pid, (Instant::now(), s));
        }
    }

    fn collections(vid: u16, pid: u16) -> Vec<hidapi::DeviceInfo> {
        let mut cols: Vec<_> = enumerate()
            .into_iter()
            .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
            .collect();
        cols.sort_by_key(|d| d.interface_number());
        cols
    }

    /// First collection matching `pred`, opened.
    fn open_matching(vid: u16, pid: u16, pred: impl Fn(&hidapi::DeviceInfo) -> bool) -> Option<Dev> {
        collections(vid, pid).into_iter().find(|d| pred(d)).and_then(|d| open(&d.path().to_string_lossy()))
    }

    fn open_for(pid: u16) -> Option<Dev> {
        if V3X_PIDS.contains(&pid) {
            open_matching(VID, pid, |d| d.interface_number() == 0)
        } else if pid == HS_PID {
            open_matching(VID, pid, |d| (d.usage_page() == 0xFF14 || d.usage_page() == 0xFF00) && d.usage() == 1)
        } else if pid == V2PRO_PID {
            open_matching(VID, pid, |d| d.usage_page() == 0xFF00)
        } else if pid == BARRACUDA_PID {
            open_matching(VID, pid, |d| d.usage_page() == 0xFF00 && d.usage() == 1)
        } else {
            open_matching(VID, pid, |d| d.interface_number() == 5)
        }
    }

    fn poll(state: &Headsets, pid: u16, io: &dyn HidIo) -> Option<BatteryStatus> {
        if V3_PIDS.contains(&pid) {
            state.cached(pid).or_else(|| poll_v3(io).inspect(|s| state.store(pid, *s)))
        } else if V3X_PIDS.contains(&pid) {
            state.cached(pid).or_else(|| poll_v3x(io).inspect(|s| state.store(pid, *s)))
        } else if pid == HS_PID {
            poll_hs(io)
        } else if pid == V2PRO_PID {
            poll_v2pro(io)
        } else {
            poll_barracuda(io)
        }
    }

    /// Headset cards for the `razer` provider's discover().
    pub fn discover(state: &Headsets) -> Vec<Device> {
        let mut known = state.known.lock().unwrap();
        let mut out = Vec::new();
        for &pid in V3_PIDS.iter().chain(V3X_PIDS.iter()).chain([HS_PID, V2PRO_PID, BARRACUDA_PID].iter()) {
            let Some(io) = open_for(pid) else { continue };
            // Probe once so a dead dongle does not leave a ghost card.
            if poll(state, pid, &io).is_none() {
                continue;
            }
            let dev = Device {
                id: format!("razer:hs:{pid:04x}"),
                name: name(pid).into(),
                vendor: "Razer".into(),
                model: Some(name(pid).into()),
                kind: DeviceKind::Headset,
                provider: "razer",
                locator: format!("hs\t{pid:04x}"),
            };
            known.insert(dev.id.clone(), dev.clone());
            out.push(dev);
        }
        let live: std::collections::HashSet<String> = out.iter().map(|d| d.id.clone()).collect();
        for (id, d) in known.iter() {
            if !live.contains(id) {
                out.push(d.clone());
            }
        }
        out
    }

    /// Battery for a headset device (locator "hs\t<pid>").
    pub fn read(state: &Headsets, pid: u16) -> BatteryStatus {
        if let Some(s) = state.cached(pid) {
            return s;
        }
        let Some(io) = open_for(pid) else { return BatteryStatus::ASLEEP };
        poll(state, pid, &io).unwrap_or(BatteryStatus::ASLEEP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn crc16_known() {
        // CRC-16/XMODEM of "123456789" is 0x31C3.
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn v3_packet_checksum() {
        let p = v3_packet(0x80, 0x21);
        assert_eq!(p[62], p[..62].iter().fold(0u8, |a, &x| a ^ x));
        assert_eq!((p[9], p[10]), (0x80, 0x21));
    }

    #[test]
    fn v3_reply_parsed() {
        // After the handshake writes, the battery GET gets a matching reply.
        let io = FakeHid::new(|req| {
            if req.len() > 10 && req[10] == 0x21 {
                let mut r = vec![0u8; 64];
                r[0] = 0x02;
                r[2] = 0x60;
                r[10] = 0x21;
                r[13] = 63;
                r[14] = 1;
                vec![r]
            } else {
                vec![]
            }
        });
        let s = poll_v3(&io).unwrap();
        assert_eq!((s.level, s.charge), (Some(63), Charge::Charging));
    }

    #[test]
    fn hs_frame_checksum() {
        let f = hs_frame(0x21, 0x61);
        // Request checksum covers bytes [0..62) including the report id.
        let chk = f[..62].iter().fold(0u8, |a, &x| a ^ x);
        assert_eq!(chk, f[62]);
        assert_eq!((f[2], f[10]), (0x61, 0x21));
    }

    #[test]
    fn pa_frame_layout() {
        let f = pa_frame(0x07, 0x0E, 0x02, 0xE1, 1);
        assert_eq!((&f[5], &f[6], f[9], f[10], f[11]), (&b'P', &b'A', 0x02, 0xE1, 1));
    }

    #[test]
    fn barracuda_command_frame() {
        let io = FakeHid::new(|_| vec![]);
        let mut b = Barracuda { io: &io, stream: Vec::new(), seq: 0 };
        assert!(b.command(0x0E, &[0x02, 0xE1, 1]));
        // The written frame carries [50 41] payload at offset 5.
    }

    #[test]
    fn supported_pids() {
        assert!(supported(VID, 0x0576));
        assert!(supported(VID, 0x0550));
        assert!(supported(VID, 0x057D));
        assert!(!supported(VID, 0x9999));
        assert!(!supported(0x046D, 0x0576));
    }
}
