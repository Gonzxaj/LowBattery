//! Hardware-facing core: device model, provider trait, polling monitor, config.
//! Everything except the Windows I/O compiles and tests on Linux.

pub mod config;
pub mod diag;
pub mod hid;
pub mod monitor;
pub mod providers;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Controller,
    Other,
}

/// Best-effort type from a product name, for transports that don't report one.
pub fn guess_kind(name: &str) -> DeviceKind {
    let n = name.to_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| n.contains(x));
    if has(&["headset", "headphone", "buds", "airpods", "wh-", "wf-", "arctis", "hyperx cloud", "pro x 2", "astro", "g733", "g435", "g535", "g935"]) {
        DeviceKind::Headset
    } else if has(&["keyboard", "keys", "k380", "k780", "mx mechanical"]) {
        DeviceKind::Keyboard
    } else if has(&["mouse", "mx master", "mx anywhere", "superlight", "g502", "g305", "g703", "g903", "lift"]) {
        DeviceKind::Mouse
    } else if has(&["controller", "gamepad", "xbox", "dualsense", "dualshock"]) {
        DeviceKind::Controller
    } else {
        DeviceKind::Other
    }
}

/// A device as found by a provider. `locator` is provider-private
/// (HID path + index, PnP instance id...) and never shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Device {
    /// Stable across polls and restarts; used as key in config.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub model: Option<String>,
    pub kind: DeviceKind,
    pub provider: &'static str,
    pub locator: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Charge {
    Discharging,
    Charging,
    Full,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    Connected,
    /// Known to the transport (paired / receiver slot) but not answering.
    Asleep,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BatteryStatus {
    /// None = the device answered but battery is unknown, or it did not answer.
    pub level: Option<u8>,
    pub charge: Charge,
    pub link: Link,
}

impl BatteryStatus {
    pub const ASLEEP: Self = Self { level: None, charge: Charge::Unknown, link: Link::Asleep };
    pub const GONE: Self = Self { level: None, charge: Charge::Unknown, link: Link::Disconnected };

    pub fn connected(level: u8, charge: Charge) -> Self {
        Self { level: Some(level.min(100)), charge, link: Link::Connected }
    }
}

/// One protocol or vendor. Providers are independent: adding one means
/// writing a new module and listing it in `providers::all()`.
///
/// Both calls block (HID round-trips) and run on the monitor thread only,
/// one device at a time, so a device never sees concurrent requests.
pub trait BatteryProvider: Send {
    fn name(&self) -> &'static str;
    fn discover(&self) -> Vec<Device>;
    fn read_battery(&self, device: &Device) -> BatteryStatus;
}
