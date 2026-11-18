//! What the OS exposes, written to the log so a user can send it in when a
//! peripheral is missing. No network, no API: the log file is the channel.

use crate::monitor::DeviceState;
use crate::{Device, DeviceKind};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

thread_local! {
    /// Per-poll memos. `Monitor::poll` bumps the HID epoch once per cycle
    /// (`hid::native::refresh`), so `candidates()` and `usb_keys()` build
    /// once per poll instead of once per caller. Standalone callers that
    /// never refresh just keep epoch 0 and memoize forever — same staleness
    /// the HID snapshot itself already has off the monitor thread.
    static MEMO: RefCell<PollMemo> = RefCell::new(PollMemo::default());
}

#[derive(Default)]
struct PollMemo {
    epoch: Option<u64>,
    candidates: Option<Rc<Vec<Candidate>>>,
    keys: HashMap<String, Vec<String>>,
}

#[cfg(any(windows, target_os = "linux"))]
fn poll_epoch() -> u64 {
    crate::hid::native::epoch()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn poll_epoch() -> u64 {
    0
}

/// Runs `f` against the per-epoch memo, resetting it when the HID snapshot
/// was rebuilt. `f` gets `&mut PollMemo` and must return the memoized value.
fn with_memo<T>(f: impl FnOnce(&mut PollMemo) -> T) -> T {
    let epoch = poll_epoch();
    MEMO.with(|cell| {
        let mut m = cell.borrow_mut();
        if m.epoch != Some(epoch) {
            m.epoch = Some(epoch);
            m.candidates = None;
            m.keys.clear();
        }
        f(&mut m)
    })
}

/// Full snapshot of vendor HID interfaces and the OS battery nodes
/// (Bluetooth devnodes on Windows, power_supply on Linux).
pub fn diagnostics() -> Value {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let hid: Vec<Value> = crate::hid::native::enumerate()
            .iter()
            .filter(|d| {
                use crate::providers::keyboards::{USAGE, USAGE_PAGE};
                VENDORS.contains(&d.vendor_id()) || (d.usage_page(), d.usage()) == (USAGE_PAGE, USAGE)
            })
            .map(|d| {
                json!({
                    "vid": format!("{:04x}", d.vendor_id()), "pid": format!("{:04x}", d.product_id()),
                    "usage_page": format!("{:04x}", d.usage_page()), "usage": format!("{:04x}", d.usage()),
                    "interface": d.interface_number(), "product": d.product_string(),
                })
            })
            .collect();
        #[cfg(windows)]
        return json!({ "hid": hid, "bluetooth": crate::providers::bluetooth::diagnostics() });
        #[cfg(target_os = "linux")]
        return json!({ "hid": hid, "power_supply": crate::providers::power_supply::diagnostics() });
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    json!({ "hid": [], "bluetooth": [] })
}

/// Vendors with a dedicated provider covering the whole VID.
pub const VENDORS: [u16; 3] = [crate::providers::logitech::VID, crate::providers::steelseries::VID, crate::providers::razer::VID];

/// True when a dedicated provider speaks for this USB id: a whole vendor
/// (logitech/steelseries/razer) or a listed model on a shared VID (Sony pads,
/// HP/Kingston HyperX, Corsair, Cooler Master). The generic fallbacks and the
/// unsupported-device report skip claimed ids so nothing shows twice.
pub fn claimed(vid: u16, pid: u16) -> bool {
    VENDORS.contains(&vid)
        || (vid == crate::providers::playstation::VID && crate::providers::playstation::PIDS.contains(&pid))
        || (vid == crate::providers::coolermaster::VID && crate::providers::coolermaster::PIDS.contains(&pid))
        || crate::providers::hyperx::supported(vid, pid)
        || (vid == crate::providers::corsair::VID && crate::providers::corsair::supported(vid, pid))
        || crate::providers::jabra::supported(vid, pid)
        || crate::providers::jbl::supported(vid, pid)
        || crate::providers::fractal::supported(vid, pid)
        || crate::providers::sony_inzone::supported(vid, pid)
        || crate::providers::glorious::supported(vid, pid)
        || crate::providers::kysona::supported(vid, pid)
        || crate::providers::pulsar::supported(vid, pid)
        || crate::providers::atk_bitmouse::supported(vid, pid)
        || crate::providers::lamzu::supported(vid, pid)
        || crate::providers::keychron::supported(vid, pid)
        || crate::providers::keychron::nape_supported(vid, pid)
        || crate::providers::mchose::mchose_supported(vid, pid)
        || crate::providers::mchose::g3_supported(vid, pid)
        || crate::providers::nordic::supported(vid, pid)
        || crate::providers::dareu::supported(vid, pid)
        || crate::providers::areson::supported(vid, pid)
        || crate::providers::flydigi::supported(vid, pid)
        || crate::providers::sonix::supported(vid, pid)
        || crate::providers::royal_kludge::supported(vid, pid)
        || crate::providers::rongyuan::supported(vid, pid)
        || crate::providers::endgame::supported(vid, pid)
        || crate::providers::gamesir::nova_supported(vid, pid)
        || crate::providers::gamesir::cyclone_supported(vid, pid)
        || (vid == crate::providers::zowie::VID)
        || crate::providers::rapoo::supported(vid, pid)
        || crate::providers::aula::supported(vid, pid)
        || crate::providers::fantech::supported(vid, pid)
        || crate::providers::rog::supported(vid, pid)
        || crate::providers::razer::headsets::supported(vid, pid)
        || crate::providers::other_mice::supported(vid, pid)
}

/// USB `vid_xxxx&pid_xxxx` key of a Windows HID path, lower-case.
pub fn usb_key(path: &str) -> Option<String> {
    let p = path.to_ascii_lowercase();
    let i = p.find("vid_")?;
    let key = p.get(i..i + 17)?; // "vid_046d&pid_c54d"
    (key.as_bytes()[8] == b'&' && key[9..].starts_with("pid_")).then(|| key.to_string())
}

/// Every USB key in a locator. Windows HID paths carry `vid_xxxx&pid_xxxx`;
/// Linux sysfs paths name each HID node `BBBB:VVVV:PPPP.NNNN` (a receiver
/// and its paired device both appear), and `/dev/hidrawN` resolves to one.
pub fn usb_keys(locator: &str) -> Vec<String> {
    with_memo(|m| {
        if let Some(k) = m.keys.get(locator) {
            return k.clone();
        }
        let v = usb_keys_fresh(locator);
        m.keys.insert(locator.to_string(), v.clone());
        v
    })
}

fn usb_keys_fresh(locator: &str) -> Vec<String> {
    let mut out = Vec::new();
    for field in locator.split('\t') {
        out.extend(usb_key(field));
        let sys = field.strip_prefix("/dev/").and_then(|n| std::fs::canonicalize(format!("/sys/class/hidraw/{n}")).ok());
        let path = sys.map_or_else(|| field.to_string(), |p| p.to_string_lossy().into_owned());
        for seg in path.split('/') {
            let b = seg.as_bytes();
            if b.len() == 19 && b[4] == b':' && b[9] == b':' && b[14] == b'.' && seg.is_ascii() {
                out.push(format!("vid_{}&pid_{}", seg[5..9].to_ascii_lowercase(), seg[10..14].to_ascii_lowercase()));
            }
        }
    }
    out
}

/// Logs, once per run, every keyboard / mouse / gamepad / headset-like HID
/// device that no provider turned into a card, with what's needed to add it.
#[derive(Default)]
pub struct UnsupportedReporter {
    reported: HashSet<String>,
}

impl UnsupportedReporter {
    pub fn check(&mut self, shown: &[DeviceState]) {
        // Generic cards carry no battery protocol either: keep reporting them.
        let handled: HashSet<String> =
            shown.iter().filter(|s| s.device.provider != GENERIC).flat_map(|s| usb_keys(&s.device.locator)).collect();
        for c in candidates().iter() {
            if !handled.contains(&c.key) && !has_protocol(c) && self.reported.insert(c.key.clone()) {
                log::info!(
                    "UNSUPPORTED peripheral {} \"{}\" by \"{}\" collections [{}] - detected but no battery protocol for it yet. \
                     If it is wireless and should show up, send this log file to add support.",
                    c.key,
                    c.product,
                    c.manufacturer,
                    c.cols.join(", ")
                );
            }
        }
    }
}

/// Provider name of the "connected, no battery info" cards.
pub const GENERIC: &str = "generic";

/// Wireless-looking peripherals no provider claimed. They get a card that only
/// says they're connected: cheap 2.4G receivers (YICHIP and the like) expose
/// no battery at all, but the user still sees the device is there.
pub fn generic_devices(shown: &[Device]) -> Vec<Device> {
    let handled: HashSet<String> = shown.iter().flat_map(|d| usb_keys(&d.locator)).collect();
    candidates()
        .iter()
        .filter(|c| !handled.contains(&c.key) && !has_protocol(c) && looks_wireless(&c.product))
        .map(|c| Device {
            id: format!("{GENERIC}:{}", c.key),
            kind: generic_kind(&c.product, &c.cols),
            name: c.product.clone(),
            vendor: c.manufacturer.clone(),
            model: None,
            provider: GENERIC,
            locator: c.key.clone(),
        })
        .collect()
}

/// A device some provider speaks to, only silent right now (asleep, out of
/// range): it must not turn into a "no battery info" card.
fn has_protocol(c: &Candidate) -> bool {
    let parsed = c
        .key
        .strip_prefix("vid_")
        .and_then(|ids| ids.split_once("&pid_"))
        .and_then(|(v, p)| Some((u16::from_str_radix(v, 16).ok()?, u16::from_str_radix(p, 16).ok()?)));
    let Some((vid, pid)) = parsed else { return false };
    claimed(vid, pid) || c.cols.iter().any(|col| col.ends_with("ff1c/0092") || col.ends_with("ffff/0002"))
}

/// shortcut: judged by the product string; wired gear named like this would show too.
pub fn looks_wireless(product: &str) -> bool {
    let p = product.to_lowercase();
    ["2.4g", "wireless", "receiver", "dongle", "nano"].iter().any(|w| p.contains(w))
}

/// Name first; else the collections. Receivers often add a keyboard
/// interface for macro keys, so a mouse collection wins.
pub fn generic_kind(product: &str, cols: &[String]) -> DeviceKind {
    let has = |u: &str| cols.iter().any(|c| c.ends_with(u));
    match crate::guess_kind(product) {
        DeviceKind::Other if has("0001/0002") => DeviceKind::Mouse,
        DeviceKind::Other if has("0001/0006") => DeviceKind::Keyboard,
        DeviceKind::Other if has("0001/0004") || has("0001/0005") => DeviceKind::Controller,
        k => k,
    }
}

/// A peripheral-looking HID device, one per USB id.
pub struct Candidate {
    pub key: String,
    pub product: String,
    pub manufacturer: String,
    /// "if<interface>:<page>/<usage>" per collection.
    pub cols: Vec<String>,
}

/// Peripheral candidates for this poll, shared between `generic_devices`
/// and `UnsupportedReporter::check` (both run once per `Monitor::poll`).
fn candidates() -> Rc<Vec<Candidate>> {
    with_memo(|m| {
        if let Some(c) = &m.candidates {
            return c.clone();
        }
        let c = Rc::new(candidates_fresh());
        m.candidates = Some(c.clone());
        c
    })
}

#[cfg(any(windows, target_os = "linux"))]
fn candidates_fresh() -> Vec<Candidate> {
    use std::collections::BTreeMap;
    let mut by_key: BTreeMap<String, Candidate> = BTreeMap::new();
    for d in crate::hid::native::enumerate().iter() {
        #[cfg(windows)]
        let Some(key) = usb_key(&d.path().to_string_lossy()) else { continue };
        // hidraw paths carry no ids; hidapi reads them from sysfs.
        #[cfg(target_os = "linux")]
        let key = format!("vid_{:04x}&pid_{:04x}", d.vendor_id(), d.product_id());
        let entry = by_key.entry(key.clone()).or_insert_with(|| Candidate {
            key,
            product: d.product_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("?").to_string(),
            manufacturer: d.manufacturer_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("?").to_string(),
            cols: Vec::new(),
        });
        entry.cols.push(format!("if{}:{:04x}/{:04x}", d.interface_number(), d.usage_page(), d.usage()));
    }
    by_key.into_values().filter(|c| c.cols.iter().any(|col| PERIPHERAL.iter().any(|p| col.ends_with(p)))).collect()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn candidates_fresh() -> Vec<Candidate> {
    Vec::new()
}

/// Generic Desktop mouse / joystick / gamepad / keyboard, Consumer Control (headsets).
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const PERIPHERAL: [&str; 5] = ["0001/0002", "0001/0004", "0001/0005", "0001/0006", "000c/0001"];

#[cfg(test)]
mod tests {
    #[test]
    fn usb_key_from_windows_paths() {
        let p = r"\\?\HID#VID_046D&PID_C54D&MI_02&Col02#8&2b2&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(super::usb_key(p).as_deref(), Some("vid_046d&pid_c54d"));
        assert_eq!(super::usb_key("{f7a14153-b985-5303-bf2a-9b2455087e18}"), None);
        assert_eq!(super::usb_key("vid_12"), None);
    }

    #[test]
    fn generic_receivers() {
        use crate::DeviceKind;
        // The YICHIP 3151:3020 mouse receiver seen on a real machine.
        let cols: Vec<String> = ["if1:0001/0002", "if1:0001/0001", "if1:000c/0001", "if1:ff00/000e", "if0:0001/0006"].map(String::from).into();
        assert!(super::looks_wireless("2.4G Receiver"));
        assert!(!super::looks_wireless("SYNA32D8:00 06CB:CEE7"));
        assert_eq!(super::generic_kind("2.4G Receiver", &cols), DeviceKind::Mouse);
        assert_eq!(super::generic_kind("Wireless Keyboard", &cols), DeviceKind::Keyboard);
        assert_eq!(super::generic_kind("USB Receiver", &cols[4..]), DeviceKind::Keyboard);
        let c = |key: &str, col: &str| super::Candidate { key: key.into(), product: "2.4G Receiver".into(), manufacturer: "?".into(), cols: vec![col.into()] };
        assert!(!super::has_protocol(&c("vid_3151&pid_3020", "if1:ff00/000e")));
        assert!(super::has_protocol(&c("vid_320f&pid_5088", "if1:ff1c/0092"))); // silent FF1C keyboard
        assert!(super::has_protocol(&c("vid_046d&pid_c54d", "if2:0001/0002"))); // Logitech
    }

    #[test]
    fn usb_keys_from_sysfs_paths() {
        let p = "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.2/0003:046D:C54D.0003/0003:046D:4082.0007/power_supply/hidpp_battery_0";
        assert_eq!(super::usb_keys(p), ["vid_046d&pid_c54d", "vid_046d&pid_4082"]);
        assert_eq!(super::usb_keys("1838\t/dev/hidraw-missing"), Vec::<String>::new());
    }
}
