//! Smaller mouse brands, each with a community-documented battery protocol:
//!  * Motospeed X6 (Realtek 0BDA, receivers FFE0/FFF2, cable FFF1): output
//!    B5 [06], reply report B4, byte 20 = level (+128 while charging).
//!  * Xenta receivers (Delux M900 Pro, Attack Shark X6/R1/X11/X3/V3;
//!    1D57): passive input report 03 on page 000A every ~2 s:
//!    [03][model][40][state][percent]; two agreeing frames required.
//!  * Ajazz AJ179 (249A receiver / 248A cable, iface 2, 33-byte reports):
//!    command 10 reply carries model, charging, percent, checksum.
//!  * AJAZZ AJ-series (3151, page FFFF usage 02, 65-byte feature):
//!    SET_FEATURE status poll brings the link up; GET_FEATURE 05 -> pct.
//!  * AJAZZ AJ139 V2 MC (A8A5 dongle / A8A4 cable, pid 2255, FF01:0010):
//!    [55 30 A5 chk ..] request, [AA 30 ..] reply, [8]=pct [9]=charging.
//!  * Wraith W1 + Incott G23 V2/G24 (093A:522C/622C, FF05): 8F identifies
//!    W1 (model 18/20) vs Incott; W1 uses the 86 per-button read that
//!    pushes status report 09, Incott answers 89 via GET_FEATURE 09.
//!  * Delux M800 Pro (248A:5B2E cable / 5B2F receiver, FF00:0001):
//!    feature report 0C, [18]=percent [19]=state, seq in byte 4.
//!
//! Protocol docs from the projects cited per section; no code taken.

use crate::hid::{feature_exchange, read_matching, HidIo};
use crate::{BatteryStatus, Charge, Link};
use std::io;
use std::time::Duration;

// ---------- Motospeed X6 ----------

pub const MOTOSPEED_VID: u16 = 0x0BDA;
pub const MOTOSPEED_PIDS: [u16; 3] = [0xFFE0, 0xFFF2, 0xFFF1];

/// iface 4, outLen >= 21. Cable FFF1 only reports "charging".
pub fn poll_motospeed(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    if cable {
        return Some(BatteryStatus { level: None, charge: Charge::Charging, link: Link::Connected });
    }
    io.write(&[0xB5, 0x06]).ok()?;
    let r = read_matching(io, 1500, |r| (r.len() > 20 && r[0] == 0xB4).then(|| r.to_vec())).ok().flatten()?;
    let raw = r[20];
    let charging = raw >= 128;
    let level = (if charging { raw - 128 } else { raw }).min(100);
    Some(BatteryStatus { level: (level > 0).then_some(level), charge: if charging { Charge::Charging } else { Charge::Discharging }, link: Link::Connected })
}

// ---------- Xenta (passive push) ----------

pub const XENTA_VID: u16 = 0x1D57;
pub const XENTA_PIDS: [u16; 4] = [0xFA60, 0xFA65, 0xFA55, 0x2155];

struct XentaModel {
    code: u8,
    name: &'static str,
    tens: bool,
}
const XENTA_MODELS: &[XentaModel] = &[
    XentaModel { code: 0x3D, name: "Delux M900 Pro", tens: false },
    XentaModel { code: 0x55, name: "Attack Shark X11", tens: false },
    XentaModel { code: 0x4E, name: "Attack Shark X3 Max", tens: false },
    XentaModel { code: 0xE5, name: "Attack Shark V3 Pro", tens: false },
    XentaModel { code: 0x10, name: "Attack Shark X6 / R1", tens: true },
];

/// Reads two consecutive [03][model][40][state][level] frames (~2 s
/// cadence) and only trusts them when they agree — link-up always starts
/// with a false 100%.
pub fn poll_xenta(io: &dyn HidIo, cable: bool) -> Option<(BatteryStatus, &'static str)> {
    let frame = |io: &dyn HidIo, ms: u64| -> io::Result<Option<Vec<u8>>> {
        read_matching(io, ms, |r| (r.len() >= 5 && r[0] == 0x03 && r[2] == 0x40).then(|| r[..5].to_vec()))
    };
    let a = frame(io, 5000).ok().flatten()?;
    let b = frame(io, 3500).ok().flatten()?;
    xenta_status(&a, &b, cable)
}

/// Two agreeing frames -> status + model name.
pub fn xenta_status(a: &[u8], b: &[u8], cable: bool) -> Option<(BatteryStatus, &'static str)> {
    if a != b || a.len() < 5 {
        return None;
    }
    let model = XENTA_MODELS.iter().find(|m| m.code == a[1]);
    let name = model.map(|m| m.name).unwrap_or("Wireless mouse");
    let st = a[3];
    let level = a[4];
    let mut s = BatteryStatus { level: None, charge: Charge::Unknown, link: Link::Connected };
    if cable || st == 0x02 || st == 0x03 {
        s.charge = Charge::Charging; // level frozen on external power
        if st == 0x02 && !cable {
            s.level = Some(100);
        }
    } else if let Some(m) = model {
        if m.tens {
            if (1..=10).contains(&level) {
                s.level = Some(level * 10);
            }
        } else if st == 0x01 && (1..=100).contains(&level) {
            s.level = Some(level);
        }
    }
    if s.level.is_none() && s.charge == Charge::Unknown {
        s.link = Link::Asleep; // shown with the last known value
    }
    Some((s, name))
}

// ---------- Ajazz AJ179 (249A/248A, iface 2, 33-byte reports) ----------

pub const AJ179_VIDS: [u16; 2] = [0x249A, 0x248A];

pub fn poll_ajazz(io: &dyn HidIo, receiver: bool) -> Option<(BatteryStatus, String)> {
    io.write(&[0x00, 0x10]).ok()?;
    let r = read_matching(io, 1500, |r| {
        if r.len() < 32 || r[0] != 0x10 || r[1] != 0x00 {
            return None;
        }
        let sum: u16 = r[4..=30].iter().map(|&b| b as u16).sum();
        (sum as u8 == r[31]).then(|| r.to_vec())
    })
    .ok()
    .flatten()?;
    if r[13] > 100 {
        return None;
    }
    let model: String = r[4..8].iter().take_while(|&&b| (0x20..0x7F).contains(&b)).map(|&b| b as char).collect();
    let name = if model.len() == 4 && model.starts_with('M') {
        format!("Ajazz AJ{}", &model[1..])
    } else {
        "Ajazz mouse".into()
    };
    let connected = !receiver || r[14] != 0;
    Some((
        BatteryStatus {
            level: connected.then_some(r[13]),
            charge: if r[12] != 0 { Charge::Charging } else { Charge::Discharging },
            link: Link::Connected,
        },
        name,
    ))
}

// ---------- AJAZZ AJ-series (3151, FFFF:0002, 65-byte feature) ----------

pub fn poll_ajazz_aj(io: &dyn HidIo) -> Option<BatteryStatus> {
    let mut req = vec![0u8; 65];
    req[1] = 0xF7; // status poll: brings the 2.4 GHz telemetry link up
    let pct = feature_exchange(io, &req, 8, 30, 65, |r| {
        (r.len() >= 4 && r[0] == 0x05 && r[1] == 0 && r[2] == 0 && r[3] <= 100).then_some(r[3])
    })?;
    Some(BatteryStatus {
        level: (pct > 0).then_some(pct),
        charge: Charge::Unknown,
        link: if pct > 0 { Link::Connected } else { Link::Asleep },
    })
}

// ---------- AJAZZ AJ139 V2 MC (A8A5/A8A4 pid 2255, FF01:0010) ----------

pub const AJ139_VIDS: [u16; 2] = [0xA8A5, 0xA8A4];
pub const AJ139_PID: u16 = 0x2255;

pub fn poll_aj139(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    let is_battery = |r: &[u8]| r.len() > 9 && r[0] == 0xAA && r[1] == 0x30;
    let mut ok = false;
    let mut r = Vec::new();
    for req in [
        vec![0x00, 0x55, 0x30, 0xA5, 0x2E, 0x2E],
        vec![0x00, 0x55, 0x30, 0xA5, 0x0B, 0x2E, 0x01, 0x01],
    ] {
        io.write(&req).ok()?;
        if let Ok(Some(rep)) = read_matching(io, 800, |b| is_battery(b).then(|| b.to_vec())) {
            r = rep;
            ok = true;
            break;
        }
    }
    if !ok || r[8] > 100 {
        return None;
    }
    let connected = cable || r[8] > 0;
    Some(BatteryStatus {
        level: connected.then_some(r[8]),
        charge: if r[9] != 0 { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

// ---------- PixArt 093A:522C/622C — Wraith W1 vs Incott ----------

pub const PIXART_VID: u16 = 0x093A;
pub const PIXART_PIDS: [u16; 2] = [0x522C, 0x622C]; // dongle, cable

/// SET_FEATURE [09 8F] device-info read; returns the model code
/// (18 = W1 Standard, 20 = W1 Freestyle, Incott codes are other values,
/// -1 = answered but not a W1, None = no answer).
pub fn pixart_identify(io: &dyn HidIo) -> Option<i32> {
    io.send_feature(&[0x09, 0x8F]).ok()?;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(50));
        let mut r = vec![0u8; 16];
        r[0] = 0x09;
        let Ok(n) = io.get_feature(&mut r) else { return None };
        if n >= 4 && r[0] == 0x09 && r[1] == 0x8F {
            return Some(if r[3] == 18 || r[3] == 20 { r[3] as i32 } else { -1 });
        }
    }
    None
}

/// Incott query: SET_FEATURE [09 89], GET_FEATURE 09 echoes it;
/// byte 2 = battery (bit7 charging, low 7 bits percent).
pub fn poll_incott(io: &dyn HidIo, cable: bool) -> Option<BatteryStatus> {
    io.send_feature(&[0x09, 0x89]).ok()?;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(50));
        let mut r = vec![0u8; 16];
        r[0] = 0x09;
        let Ok(n) = io.get_feature(&mut r) else { return None };
        if n >= 4 && r[0] == 0x09 && r[1] == 0x89 && (r[2] | r[3]) != 0 {
            let pct = r[2] & 0x7F;
            if pct > 100 {
                return None;
            }
            let charging = r[2] & 0x80 != 0;
            let connected = cable || pct > 0;
            return Some(BatteryStatus {
                level: connected.then_some(pct),
                charge: if charging { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            });
        }
    }
    None
}

/// W1 per-button read: SET_FEATURE [09 86 btn], GET_FEATURE 09, retry once.
fn w1_read_button(io: &dyn HidIo, button: u8) -> bool {
    io.send_feature(&[0x09, 0x86, button]).is_ok()
}

/// After the 86 sweep the mouse pushes status input report 09:
/// [09][battery | 0x80 charging][rate | slot << 4].
pub fn w1_listen(readers: &[&dyn HidIo], timeout_ms: u64) -> Option<(u8, bool)> {
    for &r in readers {
        if let Ok(Some(rep)) = read_matching(r, timeout_ms, |b| {
            (b.len() >= 2 && b[0] == 0x09 && (b[1] & 0x7F) <= 100).then(|| (b[1] & 0x7F, b[1] & 0x80 != 0))
        }) {
            return Some(rep);
        }
    }
    None
}

pub fn poll_w1(io: &dyn HidIo, readers: &[&dyn HidIo], cable: bool) -> Option<BatteryStatus> {
    // 86 sweep like the web driver's Ri(), then listen for the push.
    std::thread::sleep(Duration::from_millis(200));
    let mut sent = false;
    for b in [0u8, 1, 2, 4, 3] {
        sent |= w1_read_button(io, b);
    }
    if !sent {
        return None;
    }
    let (pct, charging) = w1_listen(readers, 400)?;
    // One keep-alive beat if the first listen window missed.
    let (pct, charging) = match w1_listen(readers, 1500) {
        Some(v) => v,
        None => (pct, charging),
    };
    let connected = cable || pct > 0 || charging;
    Some(BatteryStatus {
        level: connected.then_some(pct),
        charge: if charging { Charge::Charging } else { Charge::Discharging },
        link: Link::Connected,
    })
}

// ---------- Delux M800 Pro (248A:5B2E/5B2F, FF00:0001, feature 0C) ----------

pub fn delux_request(seq: u8) -> Vec<u8> {
    let mut req = vec![0u8; 33];
    req[0] = 0x0C;
    req[1] = 0x01;
    req[2] = 0x20;
    req[4] = seq;
    req
}

pub fn poll_delux(io: &dyn HidIo, seq: &mut u8, receiver: bool) -> Option<BatteryStatus> {
    io.send_feature(&delux_request(*seq)).ok()?;
    *seq = seq.wrapping_add(1);
    std::thread::sleep(Duration::from_millis(20));
    let mut r = vec![0u8; 33];
    r[0] = 0x0C;
    let n = io.get_feature(&mut r).ok()?;
    if n < 20 || r[0] != 0x0C || r[2] != 0x20 {
        return None;
    }
    // The dongle sometimes needs a second read when r[6] == 0.
    if receiver && r[6] == 0 {
        io.send_feature(&delux_request(*seq)).ok()?;
        *seq = seq.wrapping_add(1);
        std::thread::sleep(Duration::from_millis(20));
        let mut r2 = vec![0u8; 33];
        r2[0] = 0x0C;
        if let Ok(n2) = io.get_feature(&mut r2) {
            if n2 >= 20 && r2[0] == 0x0C && r2[2] == 0x20 {
                r = r2;
            }
        }
    }
    let pct = r[18];
    if pct > 100 {
        return None;
    }
    let st = r[19];
    let connected = r[6] != 0 || pct > 0;
    Some(match st {
        2 => BatteryStatus { level: Some(100), charge: Charge::Full, link: Link::Connected },
        1 => BatteryStatus {
            level: (connected && pct > 0).then_some(pct.min(100)),
            charge: Charge::Charging,
            link: Link::Connected,
        },
        _ => BatteryStatus {
            level: (connected && pct > 0).then_some(pct.min(100)),
            charge: Charge::Discharging,
            link: Link::Connected,
        },
    })
}

pub fn supported(vid: u16, pid: u16) -> bool {
    (vid == MOTOSPEED_VID && MOTOSPEED_PIDS.contains(&pid))
        || (vid == XENTA_VID && XENTA_PIDS.contains(&pid))
        || (AJ179_VIDS.contains(&vid) && pid != 0)
        || (AJ139_VIDS.contains(&vid) && pid == AJ139_PID)
        || (vid == PIXART_VID && PIXART_PIDS.contains(&pid))
        || (vid == 0x248A && (pid == 0x5B2E || pid == 0x5B2F))
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::OtherMice;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open, Dev};
    use crate::{BatteryProvider, Device, DeviceKind};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex;

    /// W1 model code per PixArt pid (0 = unknown/not-W1, -1 = answered
    /// but not a W1). Persists for the run like upstream's knownModel.
    fn w1_models() -> &'static Mutex<HashMap<u16, i32>> {
        use std::sync::OnceLock;
        static M: OnceLock<Mutex<HashMap<u16, i32>>> = OnceLock::new();
        M.get_or_init(|| Mutex::new(HashMap::new()))
    }

    #[derive(Default)]
    pub struct OtherMice {
        seq: Mutex<u8>,
        known: Mutex<BTreeMap<String, Device>>,
    }

    fn cols(vid: u16, pid: u16) -> Vec<hidapi::DeviceInfo> {
        let mut v: Vec<_> = enumerate().into_iter().filter(|d| d.vendor_id() == vid && d.product_id() == pid).collect();
        v.sort_by_key(|d| d.interface_number());
        v
    }

    fn first_io(vid: u16, pid: u16, pred: impl Fn(&hidapi::DeviceInfo) -> bool) -> Option<(String, Dev)> {
        cols(vid, pid).into_iter().find(|d| pred(d)).and_then(|d| {
            let p = d.path().to_string_lossy().into_owned();
            open(&p).map(|io| (p, io))
        })
    }

    fn device(id: String, name: &str, kind: DeviceKind, locator: String) -> Device {
        Device {
            id,
            name: name.into(),
            vendor: "Misc".into(),
            model: Some(name.into()),
            kind,
            provider: "other_mice",
            locator,
        }
    }

    impl BatteryProvider for OtherMice {
        fn name(&self) -> &'static str {
            "other_mice"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out: Vec<Device> = Vec::new();
            let push = |d: Device, out: &mut Vec<Device>| {
                if !out.iter().any(|o| o.id == d.id) {
                    out.push(d);
                }
            };

            // Motospeed X6.
            for pid in MOTOSPEED_PIDS {
                let Some((path, io)) = first_io(MOTOSPEED_VID, pid, |d| d.interface_number() == 4) else { continue };
                if poll_motospeed(&io, pid == 0xFFF1).is_none() {
                    continue;
                }
                push(device(format!("motospeed:{pid:04x}"), "Motospeed X6", DeviceKind::Mouse, format!("{path}\tmotospeed\t{pid:04x}")), &mut out);
            }

            // Xenta receivers: passive frames on page 000A.
            for pid in XENTA_PIDS {
                let Some((path, io)) = first_io(XENTA_VID, pid, |d| d.usage_page() == 0x000A) else { continue };
                if let Some((_, name)) = poll_xenta(&io, pid == 0xFA55 || pid == 0x2155) {
                    push(device(format!("xenta:{pid:04x}"), name, DeviceKind::Mouse, format!("{path}\txenta\t{pid:04x}")), &mut out);
                }
            }

            // Ajazz AJ179.
            for vid in AJ179_VIDS {
                for info in enumerate() {
                    if info.vendor_id() != vid {
                        continue;
                    }
                    let pid = info.product_id();
                    let Some((path, io)) = first_io(vid, pid, |d| d.interface_number() == 2) else { continue };
                    if let Some((_, name)) = poll_ajazz(&io, vid == 0x249A) {
                        push(device(format!("ajazz:{vid:04x}:{pid:04x}"), &name, DeviceKind::Mouse, format!("{path}\tajazz\t{vid:04x}\t{pid:04x}")), &mut out);
                    }
                    break; // one device per vid is enough for discovery
                }
            }

            // AJAZZ AJ-series on 3151 (FFFF:0002, 65-byte feature).
            for info in enumerate() {
                if info.vendor_id() != 0x3151 {
                    continue;
                }
                let (vid, pid) = (info.vendor_id(), info.product_id());
                let Some((path, io)) = first_io(vid, pid, |d| d.usage_page() == 0xFFFF && d.usage() == 0x02) else { continue };
                if poll_ajazz_aj(&io).is_none() {
                    continue;
                }
                push(device(format!("ajazzaj:{vid:04x}:{pid:04x}"), "AJAZZ mouse", DeviceKind::Mouse, format!("{path}\tajazzaj")), &mut out);
            }

            // AJ139 V2 MC.
            for vid in AJ139_VIDS {
                let Some((path, io)) = first_io(vid, AJ139_PID, |d| d.usage_page() == 0xFF01 && d.usage() == 0x0010) else { continue };
                if poll_aj139(&io, vid == 0xA8A4).is_none() {
                    continue;
                }
                push(device(format!("ajazz139:{vid:04x}"), "AJAZZ wireless mouse", DeviceKind::Mouse, format!("{path}\taj139\t{vid:04x}")), &mut out);
            }

            // PixArt: Wraith W1 first (it claims the pid once identified),
            // then Incott. Try each FF05 collection until one answers.
            for pid in PIXART_PIDS {
                let cable = pid == 0x622C;
                for feat in cols(PIXART_VID, pid).into_iter().filter(|d| d.usage_page() == 0xFF05).rev() {
                    let feat_path = feat.path().to_string_lossy().into_owned();
                    let Some(feat_io) = open(&feat_path) else { continue };
                    // Identify: cached model, fresh 8F read, or "not seen yet".
                    let mut model = w1_models().lock().unwrap().get(&pid).copied().unwrap_or(0);
                    if model == 0 {
                        match pixart_identify(&feat_io) {
                            Some(m) if m > 0 => model = m,
                            Some(_) => {
                                w1_models().lock().unwrap().insert(pid, -1);
                                model = -1;
                            }
                            None => {}
                        }
                    }
                    if model > 0 {
                        // W1: every FF05 collection can carry the status push.
                        let mut readers: Vec<(String, Dev)> = Vec::new();
                        for d in cols(PIXART_VID, pid) {
                            if d.usage_page() == 0xFF05 {
                                let p = d.path().to_string_lossy().into_owned();
                                if let Some(io) = open(&p) {
                                    readers.push((p, io));
                                }
                            }
                        }
                        let refs: Vec<&dyn HidIo> = readers.iter().map(|(_, io)| -> &dyn HidIo { io }).collect();
                        if poll_w1(&feat_io, &refs, cable).is_none() {
                            continue;
                        }
                        let name = if model == 20 { "Wraith W1 Freestyle" } else { "Wraith W1" };
                        let mut paths = vec![feat_path];
                        paths.extend(readers.iter().map(|(p, _)| p.clone()));
                        push(device(format!("wraithw1:{pid:04x}"), name, DeviceKind::Mouse, format!("w1\t{}", paths.join("\t"))), &mut out);
                    } else {
                        // Incott path (also the fallback for never-seen devices).
                        if poll_incott(&feat_io, cable).is_none() {
                            continue;
                        }
                        push(device(format!("incott:{pid:04x}"), "Incott mouse", DeviceKind::Mouse, format!("{feat_path}\tincott\t{pid:04x}")), &mut out);
                    }
                    break;
                }
            }

            // Delux M800 Pro.
            for pid in [0x5B2E, 0x5B2F] {
                let receiver = pid == 0x5B2F;
                let Some((path, io)) = first_io(0x248A, pid, |d| d.usage_page() == 0xFF00 && d.usage() == 1) else { continue };
                let mut seq = *self.seq.lock().unwrap();
                if poll_delux(&io, &mut seq, receiver).is_none() {
                    continue;
                }
                *self.seq.lock().unwrap() = seq;
                push(device("delux:m800".into(), "Delux M800 Pro", DeviceKind::Mouse, format!("{path}\tdelux\t{pid:04x}")), &mut out);
            }

            // Keep known devices whose collections went silent.
            let live: std::collections::HashSet<String> = out.iter().map(|d| d.id.clone()).collect();
            for (id, d) in known.iter() {
                if !live.contains(id) {
                    out.push(d.clone());
                }
            }
            for d in &out {
                known.insert(d.id.clone(), d.clone());
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let tag = parts.next().unwrap_or("");
            match tag {
                "motospeed" => {
                    let pid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
                    let Some((_, io)) = first_io(MOTOSPEED_VID, pid, |d| d.interface_number() == 4) else { return BatteryStatus::GONE };
                    poll_motospeed(&io, pid == 0xFFF1).unwrap_or(BatteryStatus::ASLEEP)
                }
                "xenta" => {
                    let pid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
                    let Some((_, io)) = first_io(XENTA_VID, pid, |d| d.usage_page() == 0x000A) else { return BatteryStatus::GONE };
                    poll_xenta(&io, pid == 0xFA55 || pid == 0x2155).map(|(s, _)| s).unwrap_or(BatteryStatus::ASLEEP)
                }
                "ajazz" => {
                    let vid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
                    let pid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
                    let Some((_, io)) = first_io(vid, pid, |d| d.interface_number() == 2) else { return BatteryStatus::GONE };
                    poll_ajazz(&io, vid == 0x249A).map(|(s, _)| s).unwrap_or(BatteryStatus::ASLEEP)
                }
                "ajazzaj" => {
                    let path = device.locator.split('\t').nth(1).unwrap_or("");
                    // The locator for ajazzaj is "path\tajazzaj" — recover path.
                    let path = device.locator.strip_suffix("\tajazzaj").unwrap_or(path);
                    let Some(io) = open(path) else { return BatteryStatus::GONE };
                    poll_ajazz_aj(&io).unwrap_or(BatteryStatus::ASLEEP)
                }
                "aj139" => {
                    let vid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
                    let Some((_, io)) = first_io(vid, AJ139_PID, |d| d.usage_page() == 0xFF01 && d.usage() == 0x0010) else { return BatteryStatus::GONE };
                    poll_aj139(&io, vid == 0xA8A4).unwrap_or(BatteryStatus::ASLEEP)
                }
                "w1" => {
                    let paths: Vec<&str> = parts.collect();
                    let Some(io) = open(paths[0]) else { return BatteryStatus::GONE };
                    let mut ios: Vec<Dev> = Vec::new();
                    for p in &paths[1..] {
                        if let Some(d) = open(p) {
                            ios.push(d);
                        }
                    }
                    let pid = device.id.rsplit(':').next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0x522C);
                    let refs: Vec<&dyn HidIo> = ios.iter().map(|d| d as &dyn HidIo).collect();
                    poll_w1(&io, &refs, pid == 0x622C).unwrap_or(BatteryStatus::ASLEEP)
                }
                "incott" => {
                    let path = device.locator.split('\t').next().unwrap_or("");
                    let pid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0x522C);
                    let Some(io) = open(path) else { return BatteryStatus::GONE };
                    poll_incott(&io, pid == 0x622C).unwrap_or(BatteryStatus::ASLEEP)
                }
                "delux" => {
                    let pid = parts.next().and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0);
                    let Some((_, io)) = first_io(0x248A, pid, |d| d.usage_page() == 0xFF00 && d.usage() == 1) else { return BatteryStatus::GONE };
                    let mut seq = *self.seq.lock().unwrap();
                    let r = poll_delux(&io, &mut seq, pid == 0x5B2F);
                    *self.seq.lock().unwrap() = seq;
                    r.unwrap_or(BatteryStatus::ASLEEP)
                }
                _ => {
                    // Locator "path\t<tag>" variants where path is first.
                    let path = device.locator.split('\t').next().unwrap_or("");
                    open(path).map_or(BatteryStatus::GONE, |_| BatteryStatus::ASLEEP)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn delux_request_layout() {
        let r = delux_request(7);
        assert_eq!((r[0], r[1], r[2], r[4]), (0x0C, 0x01, 0x20, 7));
        assert_eq!(r.len(), 33);
    }

    #[test]
    fn pixart_identify_parses_model() {
        let io = FakeHid::new(|req| {
            if req.starts_with(&[0x09, 0x8F]) {
                vec![vec![0x09, 0x8F, 0, 20]]
            } else {
                vec![]
            }
        });
        assert_eq!(pixart_identify(&io), Some(20));
    }

    #[test]
    fn incott_reply() {
        let io = FakeHid::new(|req| {
            if req.starts_with(&[0x09, 0x89]) {
                vec![vec![0x09, 0x89, 55 | 0x80, 0]]
            } else {
                vec![]
            }
        });
        let s = poll_incott(&io, false).unwrap();
        assert_eq!(s.level, Some(55));
        assert_eq!(s.charge, Charge::Charging);
    }

    #[test]
    fn xenta_needs_two_frames() {
        let f = vec![0x03, 0x3D, 0x40, 0x01, 64];
        let (s, name) = xenta_status(&f, &f, false).unwrap();
        assert_eq!(name, "Delux M900 Pro");
        assert_eq!(s.level, Some(64));
        // Disagreement -> no reading.
        let mut g = f.clone();
        g[4] = 10;
        assert!(xenta_status(&f, &g, false).is_none());
        // Full state on receiver freezes at 100%.
        let full = vec![0x03, 0x3D, 0x40, 0x02, 0];
        let (s, _) = xenta_status(&full, &full, false).unwrap();
        assert_eq!((s.level, s.charge), (Some(100), Charge::Charging));
    }

    #[test]
    fn ajazz_checksum() {
        let io = FakeHid::new(|_| {
            let mut r = vec![0u8; 33];
            r[0] = 0x10;
            r[4..8].copy_from_slice(b"M800");
            r[12] = 1;
            r[13] = 42;
            r[14] = 1;
            let sum: u16 = r[4..=30].iter().map(|&b| b as u16).sum();
            r[31] = sum as u8;
            vec![r]
        });
        let (s, name) = poll_ajazz(&io, true).unwrap();
        assert_eq!(name, "Ajazz AJ800");
        assert_eq!(s.level, Some(42));
        assert_eq!(s.charge, Charge::Charging);
    }

    #[test]
    fn supported_ids() {
        assert!(supported(0x0BDA, 0xFFE0));
        assert!(supported(0x1D57, 0xFA60));
        assert!(supported(0x249A, 0x9999));
        assert!(supported(0xA8A4, AJ139_PID));
        assert!(supported(PIXART_VID, 0x522C));
        assert!(supported(0x248A, 0x5B2F));
        assert!(!supported(0x0BDA, 0x1234));
    }
}
