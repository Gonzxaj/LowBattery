pub mod bluetooth;
pub mod demo;
pub mod hid_battery;
pub mod logitech;
pub mod razer;
pub mod steelseries;

#[cfg(test)]
pub mod fake;

use crate::BatteryProvider;

/// Every provider for this platform. Add new vendors here.
pub fn all() -> Vec<Box<dyn BatteryProvider>> {
    #[cfg(windows)]
    {
        vec![
            Box::new(bluetooth::Bluetooth),
            Box::new(logitech::Logitech::default()),
            Box::new(steelseries::SteelSeries::default()),
            Box::new(razer::Razer::default()),
            // Last: universal fallback for any brand using the standard HID battery usage.
            Box::new(hid_battery::HidBattery::default()),
        ]
    }
    #[cfg(not(windows))]
    {
        vec![Box::new(demo::Demo::default())]
    }
}
