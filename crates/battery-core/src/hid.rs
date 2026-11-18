//! Minimal HID I/O surface used by vendor protocols. Lets tests drive the
//! protocol code with a simulated device (see `tests::FakeHid` in providers).

use std::io;

pub trait HidIo {
    /// Output report; `data[0]` is the report id (0 if the device uses none).
    fn write(&self, data: &[u8]) -> io::Result<()>;
    /// Input report into `buf` (report id first). Ok(0) = timed out.
    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> io::Result<usize>;
    fn send_feature(&self, data: &[u8]) -> io::Result<()>;
    /// `buf[0]` must hold the report id. Returns bytes read.
    fn get_feature(&self, buf: &mut [u8]) -> io::Result<usize>;
}

/// Zero-pad an output report (report id first) to `len` total bytes.
pub fn pad_report(data: &[u8], len: usize) -> Vec<u8> {
    let mut v = data.to_vec();
    v.resize(len.max(v.len()), 0);
    v
}

/// Write `req` (report id first) and read until `accept` matches. Some stacks
/// require the output report to have its exact descriptor length, which hidapi
/// does not expose; `pads` are extra total sizes retried when the bare request
/// gets no answer. Write errors on a wrong size just move to the next pad.
pub fn query<T>(
    io: &dyn HidIo,
    req: &[u8],
    pads: &[usize],
    timeout_ms: u64,
    mut accept: impl FnMut(&[u8]) -> Option<T>,
) -> io::Result<Option<T>> {
    for size in std::iter::once(req.len()).chain(pads.iter().copied()) {
        let out = pad_report(req, size);
        if io.write(&out).is_err() {
            continue;
        }
        if let Some(v) = read_matching(io, timeout_ms, &mut accept)? {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

/// Send a feature report (`req[0]` = report id) then poll feature reads until
/// `accept` takes one or `reads` attempts pass, sleeping `delay_ms` between
/// reads. Devices need a moment to prepare the answer; dongles relay over
/// the air. Returns the first accepted value.
pub fn feature_exchange<T>(
    io: &dyn HidIo,
    req: &[u8],
    reads: usize,
    delay_ms: u64,
    size: usize,
    mut accept: impl FnMut(&[u8]) -> Option<T>,
) -> Option<T> {
    io.send_feature(req).ok()?;
    let mut buf = vec![0u8; size];
    for _ in 0..reads {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        buf[0] = req[0];
        let Ok(n) = io.get_feature(&mut buf) else { return None };
        if n > 0 {
            if let Some(v) = accept(&buf[..n]) {
                return Some(v);
            }
        }
    }
    None
}

/// Read until `accept` returns Some or the deadline passes. Unrelated
/// reports (mouse motion, other software's replies) are skipped.
pub fn read_matching<T>(
    io: &dyn HidIo,
    timeout_ms: u64,
    mut accept: impl FnMut(&[u8]) -> Option<T>,
) -> io::Result<Option<T>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut buf = [0u8; 64];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now()).as_millis() as i32;
        if left == 0 {
            return Ok(None);
        }
        let n = io.read_timeout(&mut buf, left)?;
        if n == 0 {
            return Ok(None);
        }
        if let Some(v) = accept(&buf[..n]) {
            return Ok(Some(v));
        }
    }
}

/// hidapi backend: windows-native on Windows, hidraw on Linux.
#[cfg(any(windows, target_os = "linux"))]
pub mod native {
    use super::HidIo;
    use hidapi::{DeviceInfo, HidApi, HidDevice};
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;

    thread_local! {
        // Providers only run on the monitor thread; one enumerator for all.
        static API: RefCell<Option<HidApi>> = const { RefCell::new(None) };
        /// Interface list shared by every provider during one monitor poll;
        /// `refresh()` rebuilds it so ~50 discover() calls cost one sysfs scan
        /// and zero clones (callers share the `Rc`).
        static SNAPSHOT: RefCell<Option<Rc<Vec<DeviceInfo>>>> = const { RefCell::new(None) };
        /// Bumped by `refresh()`; `diag` memos key off it.
        static EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    /// Re-scan HID interfaces and cache the list for this poll. `Monitor::poll`
    /// calls it once; `enumerate()` then hands out the shared snapshot.
    pub fn refresh() {
        SNAPSHOT.with(|s| *s.borrow_mut() = Some(Rc::new(enumerate_fresh())));
        EPOCH.with(|e| e.set(e.get() + 1));
    }

    /// Poll counter for per-cycle memos (see `diag::candidates`).
    pub fn epoch() -> u64 {
        EPOCH.with(|e| e.get())
    }

    fn enumerate_fresh() -> Vec<DeviceInfo> {
        API.with(|cell| {
            let mut slot = cell.borrow_mut();
            match slot.as_mut() {
                Some(api) => {
                    if let Err(e) = api.refresh_devices() {
                        log::warn!("hid refresh: {e}");
                    }
                }
                None => match HidApi::new() {
                    Ok(api) => *slot = Some(api),
                    Err(e) => {
                        log::warn!("hidapi init: {e}");
                        return Vec::new();
                    }
                },
            }
            slot.as_ref().map(|a| a.device_list().cloned().collect()).unwrap_or_default()
        })
    }

    /// HID interfaces for this poll (see `refresh`). Shared, read-only:
    /// cloning the `Rc` is a refcount bump, not a device-list copy.
    pub fn enumerate() -> Rc<Vec<DeviceInfo>> {
        if let Some(list) = SNAPSHOT.with(|s| s.borrow().clone()) {
            return list;
        }
        let list = Rc::new(enumerate_fresh());
        SNAPSHOT.with(|s| *s.borrow_mut() = Some(list.clone()));
        list
    }

    pub fn open(path: &str) -> Option<Dev> {
        let c = std::ffi::CString::new(path).ok()?;
        API.with(|cell| {
            let slot = cell.borrow();
            let api = slot.as_ref()?;
            api.open_path(&c).map(Dev).map_err(|e| warn_open(path, &e.to_string())).ok()
        })
    }

    fn warn_open(path: &str, e: &str) {
        thread_local! {
            static WARNED: RefCell<std::collections::HashSet<String>> = RefCell::default();
        }
        // Linux hidraw nodes are root-only until the packaged udev rule grants access.
        if cfg!(target_os = "linux") && e.contains("Permission denied") && WARNED.with(|w| w.borrow_mut().insert(path.to_string())) {
            log::warn!("no permission to open {path}: install the udev rule 70-lowbattery.rules (see README, Linux)");
        } else {
            log::debug!("open {path}: {e}");
        }
    }

    pub struct Dev(HidDevice);

    fn err(e: hidapi::HidError) -> io::Error {
        io::Error::other(e.to_string())
    }

    impl HidIo for Dev {
        fn write(&self, data: &[u8]) -> io::Result<()> {
            self.0.write(data).map(|_| ()).map_err(err)
        }
        fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> io::Result<usize> {
            self.0.read_timeout(buf, timeout_ms).map_err(err)
        }
        fn send_feature(&self, data: &[u8]) -> io::Result<()> {
            self.0.send_feature_report(data).map_err(err)
        }
        fn get_feature(&self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.get_feature_report(buf).map_err(err)
        }
    }
}
