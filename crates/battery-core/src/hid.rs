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

#[cfg(windows)]
pub mod win {
    use super::HidIo;
    use hidapi::{DeviceInfo, HidApi, HidDevice};
    use std::cell::RefCell;
    use std::io;

    thread_local! {
        // Providers only run on the monitor thread; one enumerator for all.
        static API: RefCell<Option<HidApi>> = const { RefCell::new(None) };
    }

    /// Fresh enumeration of HID interfaces (no admin rights needed).
    pub fn enumerate() -> Vec<DeviceInfo> {
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

    pub fn open(path: &str) -> Option<Dev> {
        let c = std::ffi::CString::new(path).ok()?;
        API.with(|cell| {
            let slot = cell.borrow();
            let api = slot.as_ref()?;
            api.open_path(&c).map(Dev).map_err(|e| log::debug!("open {path}: {e}")).ok()
        })
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
