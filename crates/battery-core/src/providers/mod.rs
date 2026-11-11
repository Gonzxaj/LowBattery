pub mod bluetooth;
pub mod demo;
pub mod ff1c_keyboard;
pub mod hid_battery;
pub mod logitech;
pub mod power_supply;
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
            Box::new(ff1c_keyboard::Ff1cKeyboard::default()),
            // Last: universal fallback for any brand using the standard HID battery usage.
            Box::new(hid_battery::HidBattery::default()),
        ]
    }
    // Logitech, Bluetooth and standard HID batteries come from the kernel's
    // power_supply class; vendor protocols it lacks go through hidraw.
    #[cfg(target_os = "linux")]
    {
        vec![
            Box::new(power_supply::PowerSupply),
            Box::new(steelseries::SteelSeries::default()),
            Box::new(razer::Razer::default()),
            Box::new(ff1c_keyboard::Ff1cKeyboard::default()),
        ]
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        vec![Box::new(demo::Demo::default())]
    }
}
