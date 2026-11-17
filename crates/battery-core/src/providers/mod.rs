pub mod areson;
pub mod atk_bitmouse;
pub mod aula;
pub mod bluetooth;
pub mod coolermaster;
pub mod corsair;
pub mod dareu;
pub mod demo;
pub mod endgame;
pub mod keyboards;
pub mod flydigi;
pub mod fractal;
pub mod gamesir;
pub mod glorious;
pub mod hid_battery;
pub mod hyperx;
pub mod jabra;
pub mod jbl;
pub mod keychron;
pub mod kysona;
pub mod lamzu;
pub mod logitech;
pub mod fantech;
pub mod other_mice;
pub mod rog;
pub mod mchose;
pub mod nordic;
pub mod playstation;
pub mod pulsar;
pub mod power_supply;
pub mod rapoo;
pub mod razer;
pub mod rongyuan;
pub mod royal_kludge;
pub mod sony_inzone;
pub mod sonix;
pub mod steelseries;
pub mod zowie;

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
            Box::new(keyboards::Keyboards::default()),
            Box::new(hyperx::HyperX::default()),
            Box::new(corsair::Corsair::default()),
            Box::new(coolermaster::CoolerMaster::default()),
            Box::new(playstation::PlayStation),
            Box::new(jabra::Jabra::default()),
            Box::new(jbl::Jbl::default()),
            Box::new(fractal::Fractal::default()),
            Box::new(sony_inzone::SonyInzone::default()),
            Box::new(glorious::GloriousMice::default()),
            Box::new(kysona::KysonaMice::default()),
            Box::new(pulsar::Pulsar::default()),
            Box::new(atk_bitmouse::AtkBitmouse::default()),
            Box::new(lamzu::Lamzu::default()),
            Box::new(keychron::KeychronMice::default()),
            Box::new(keychron::KeychronNape::default()),
            Box::new(mchose::Mchose::default()),
            Box::new(mchose::MchoseG3::default()),
            Box::new(nordic::Nordic::default()),
            Box::new(dareu::Dareu::default()),
            Box::new(areson::Areson::default()),
            Box::new(flydigi::Flydigi::default()),
            Box::new(sonix::Sonix::default()),
            Box::new(royal_kludge::RoyalKludge::default()),
            Box::new(rongyuan::Rongyuan::default()),
            Box::new(endgame::Endgame::default()),
            Box::new(gamesir::GameSir::default()),
            Box::new(zowie::Zowie::default()),
            Box::new(rapoo::Rapoo::default()),
            Box::new(aula::Aula::default()),
            Box::new(fantech::Fantech::default()),
            Box::new(rog::Rog::default()),
            Box::new(other_mice::OtherMice::default()),
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
            // HID++ batteries come from power_supply; this provider only
            // picks up what the kernel lacks (the G435 dongle protocol).
            Box::new(logitech::Logitech::default()),
            Box::new(steelseries::SteelSeries::default()),
            Box::new(razer::Razer::default()),
            Box::new(keyboards::Keyboards::default()),
            Box::new(hyperx::HyperX::default()),
            Box::new(corsair::Corsair::default()),
            Box::new(coolermaster::CoolerMaster::default()),
            Box::new(playstation::PlayStation),
            Box::new(jabra::Jabra::default()),
            Box::new(jbl::Jbl::default()),
            Box::new(fractal::Fractal::default()),
            Box::new(sony_inzone::SonyInzone::default()),
            Box::new(glorious::GloriousMice::default()),
            Box::new(kysona::KysonaMice::default()),
            Box::new(pulsar::Pulsar::default()),
            Box::new(atk_bitmouse::AtkBitmouse::default()),
            Box::new(lamzu::Lamzu::default()),
            Box::new(keychron::KeychronMice::default()),
            Box::new(keychron::KeychronNape::default()),
            Box::new(mchose::Mchose::default()),
            Box::new(mchose::MchoseG3::default()),
            Box::new(nordic::Nordic::default()),
            Box::new(dareu::Dareu::default()),
            Box::new(areson::Areson::default()),
            Box::new(flydigi::Flydigi::default()),
            Box::new(sonix::Sonix::default()),
            Box::new(royal_kludge::RoyalKludge::default()),
            Box::new(rongyuan::Rongyuan::default()),
            Box::new(endgame::Endgame::default()),
            Box::new(gamesir::GameSir::default()),
            Box::new(zowie::Zowie::default()),
            Box::new(rapoo::Rapoo::default()),
            Box::new(aula::Aula::default()),
            Box::new(fantech::Fantech::default()),
            Box::new(rog::Rog::default()),
            Box::new(other_mice::OtherMice::default()),
        ]
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        vec![Box::new(demo::Demo::default())]
    }
}
