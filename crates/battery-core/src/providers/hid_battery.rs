//! Universal fallback: any HID device that declares the standard
//! "Battery Strength" value (Generic Device Controls page 0x06, usage 0x20)
//! in a feature report, whatever the brand. Uses Windows' own HID parser
//! (HidP_*), so no per-model tables. Vendors with a dedicated provider are
//! skipped to avoid duplicate cards.

/// Map a raw value onto 0..=100 using the descriptor's logical range.
pub fn scale(raw: i64, min: i64, max: i64) -> Option<u8> {
    if max <= min || raw < min || raw > max {
        return None;
    }
    Some((((raw - min) * 100 + (max - min) / 2) / (max - min)) as u8)
}

#[cfg(windows)]
pub use win::HidBattery;

#[cfg(windows)]
mod win {
    use super::scale;
    use crate::{BatteryProvider, BatteryStatus, Charge, Device, DeviceKind, Link};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use windows_sys::Win32::Devices::HumanInterfaceDevice::*;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};

    const PAGE_GENERIC_DEVICE: u16 = 0x06;
    const USAGE_BATTERY_STRENGTH: u16 = 0x20;

    #[derive(Clone, Copy, Debug)]
    struct Cap {
        report_id: u8,
        link: u16,
        min: i32,
        max: i32,
        len: u16,
    }

    #[derive(Default)]
    pub struct HidBattery {
        /// path -> battery capability (None = parsed, has none). Descriptors don't change.
        caps: Mutex<HashMap<String, Option<Cap>>>,
    }

    /// Zero-access handle: enough for feature reports, even on keyboards and
    /// mice that Windows holds exclusively. Closed on drop.
    struct Handle(HANDLE);
    impl Handle {
        fn open(path: &str) -> Option<Self> {
            let w: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            let h = unsafe { CreateFileW(w.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
            (h != INVALID_HANDLE_VALUE).then_some(Self(h))
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    struct Preparsed(PHIDP_PREPARSED_DATA);
    impl Preparsed {
        fn get(h: &Handle) -> Option<Self> {
            let mut pp: PHIDP_PREPARSED_DATA = 0;
            (unsafe { HidD_GetPreparsedData(h.0, &mut pp) } != 0).then_some(Self(pp))
        }
    }
    impl Drop for Preparsed {
        fn drop(&mut self) {
            unsafe { HidD_FreePreparsedData(self.0) };
        }
    }

    fn find_cap(path: &str) -> Option<Cap> {
        let h = Handle::open(path)?;
        let pp = Preparsed::get(&h)?;
        unsafe {
            let mut caps: HIDP_CAPS = std::mem::zeroed();
            if HidP_GetCaps(pp.0, &mut caps) != HIDP_STATUS_SUCCESS || caps.FeatureReportByteLength == 0 {
                return None;
            }
            let mut vc: [HIDP_VALUE_CAPS; 4] = std::mem::zeroed();
            let mut n = vc.len() as u16;
            let st = HidP_GetSpecificValueCaps(HidP_Feature, PAGE_GENERIC_DEVICE, 0, USAGE_BATTERY_STRENGTH, vc.as_mut_ptr(), &mut n, pp.0);
            if st != HIDP_STATUS_SUCCESS || n == 0 {
                return None;
            }
            let c = &vc[0];
            Some(Cap { report_id: c.ReportID, link: c.LinkCollection, min: c.LogicalMin, max: c.LogicalMax, len: caps.FeatureReportByteLength })
        }
    }

    fn read(path: &str, cap: Cap) -> Option<u8> {
        let h = Handle::open(path)?;
        let pp = Preparsed::get(&h)?;
        let mut buf = vec![0u8; cap.len as usize];
        buf[0] = cap.report_id;
        unsafe {
            if HidD_GetFeature(h.0, buf.as_mut_ptr().cast(), buf.len() as u32) == 0 {
                return None;
            }
            let mut v = 0u32;
            let st = HidP_GetUsageValue(HidP_Feature, PAGE_GENERIC_DEVICE, cap.link, USAGE_BATTERY_STRENGTH, &mut v, pp.0, buf.as_ptr(), buf.len() as u32);
            if st != HIDP_STATUS_SUCCESS {
                return None;
            }
            // Unsigned when the logical range is non-negative (the common case).
            scale(v as i64, cap.min as i64, cap.max as i64)
        }
    }

    impl BatteryProvider for HidBattery {
        fn name(&self) -> &'static str {
            "hid-battery"
        }

        fn discover(&self) -> Vec<Device> {
            let mut caps = self.caps.lock().unwrap();
            let mut out: Vec<Device> = Vec::new();
            for d in crate::hid::native::enumerate() {
                if crate::diag::VENDORS.contains(&d.vendor_id()) {
                    continue;
                }
                let path = d.path().to_string_lossy().into_owned();
                let cap = *caps.entry(path.clone()).or_insert_with(|| find_cap(&path));
                let Some(cap) = cap else { continue };
                let id = format!("hid:{}", crate::diag::usb_key(&path).unwrap_or_else(|| path.clone()));
                if out.iter().any(|o| o.id == id) {
                    continue; // one card per physical device
                }
                let name = d.product_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Dispositivo HID").to_string();
                let kind = match (d.usage_page(), d.usage()) {
                    (0x01, 0x02) => DeviceKind::Mouse,
                    (0x01, 0x06) => DeviceKind::Keyboard,
                    (0x01, 0x04 | 0x05) => DeviceKind::Controller,
                    _ => crate::guess_kind(&name),
                };
                log::debug!("hid-battery: {name} {path} {cap:?}");
                out.push(Device {
                    id,
                    kind,
                    name,
                    vendor: d.manufacturer_string().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("HID").to_string(),
                    model: None,
                    provider: "hid-battery",
                    locator: format!("{path}\t{}\t{}\t{}\t{}\t{}", cap.report_id, cap.link, cap.min, cap.max, cap.len),
                });
            }
            // Forget unplugged paths so the cache can't grow forever.
            let present: std::collections::HashSet<String> = crate::hid::native::enumerate().iter().map(|d| d.path().to_string_lossy().into_owned()).collect();
            caps.retain(|p, _| present.contains(p));
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let f: Vec<&str> = device.locator.split('\t').collect();
            let [path, rid, link, min, max, len] = f[..] else { return BatteryStatus::GONE };
            let (Ok(report_id), Ok(link), Ok(min), Ok(max), Ok(len)) = (rid.parse(), link.parse(), min.parse(), max.parse(), len.parse()) else {
                return BatteryStatus::GONE;
            };
            match read(path, Cap { report_id, link, min, max, len }) {
                // The standard usage carries no charging flag.
                Some(level) => BatteryStatus { level: Some(level), charge: Charge::Unknown, link: Link::Connected },
                None => BatteryStatus::ASLEEP,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scaling() {
        assert_eq!(super::scale(100, 0, 100), Some(100));
        assert_eq!(super::scale(128, 0, 255), Some(50));
        assert_eq!(super::scale(3, 1, 5), Some(50));
        assert_eq!(super::scale(7, 0, 5), None);
        assert_eq!(super::scale(1, 1, 1), None);
    }
}
