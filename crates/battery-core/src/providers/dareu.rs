//! DAREU EM901X receiver (260D:1074). Unlike the Kysona transport, the
//! feature report 08 goes to the FF02:0002 collection and the input report
//! 09 arrives on the sibling FF01:0000 collection of interface 1. Only
//! command 04 (battery) is sent: [08][04]...[16]=0x49, reply
//! [09][04][status=0]...[6=percent][7=charging], 17 bytes.
//! Protocol from the official EM901X driver analysis in
//! myfinder-device-battery; reimplemented here, no code taken.

use crate::hid::{read_matching, HidIo};
use crate::{BatteryStatus, Charge, DeviceKind, Link};

pub const VID: u16 = 0x260D;
pub const PID: u16 = 0x1074;

pub fn supported(vid: u16, pid: u16) -> bool {
    vid == VID && pid == PID
}

const REQUEST: [u8; 17] = {
    let mut r = [0u8; 17];
    r[0] = 0x08;
    r[1] = 0x04;
    r[16] = 0x49; // 0x55 - (0x08 + 0x04)
    r
};

/// `cmd` writes the feature request, `input` reads the reply. The reply has
/// no established checksum rule; strict frame match only.
pub fn poll(cmd: &dyn HidIo, input: &dyn HidIo) -> Option<BatteryStatus> {
    cmd.send_feature(&REQUEST).ok()?;
    read_matching(input, 1000, |r| {
        (r.len() >= 17 && r[0] == 0x09 && r[1] == 0x04 && r[2] == 0 && r[6] <= 100).then(|| {
            BatteryStatus {
                level: Some(r[6]),
                charge: if r[7] != 0 { Charge::Charging } else { Charge::Discharging },
                link: Link::Connected,
            }
        })
    })
    .ok()
    .flatten()
}

#[cfg(any(windows, target_os = "linux"))]
pub use native::Dareu;

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::hid::native::{enumerate, open};
    use crate::{BatteryProvider, Device};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Dareu {
        known: Mutex<Option<Device>>,
    }

    impl BatteryProvider for Dareu {
        fn name(&self) -> &'static str {
            "dareu"
        }

        fn discover(&self) -> Vec<Device> {
            let mut known = self.known.lock().unwrap();
            let mut out = Vec::new();
            let infos: Vec<_> = enumerate().iter().filter(|d| supported(d.vendor_id(), d.product_id())).cloned().collect();
            if infos.is_empty() {
                return out;
            }
            // Exactly one input (FF01:0000) and one feature (FF02:0002)
            // collection on interface 1, like upstream.
            let inputs: Vec<_> = infos.iter().filter(|d| d.interface_number() == 1 && d.usage_page() == 0xFF01 && d.usage() == 0).collect();
            let feats: Vec<_> = infos.iter().filter(|d| d.interface_number() == 1 && d.usage_page() == 0xFF02 && d.usage() == 2).collect();
            if inputs.len() == 1 && feats.len() == 1 {
                let ipath = inputs[0].path().to_string_lossy().into_owned();
                let fpath = feats[0].path().to_string_lossy().into_owned();
                let (Some(input), Some(cmd)) = (open(&ipath), open(&fpath)) else {
                    out.extend(known.take());
                    return out;
                };
                if poll(&cmd, &input).is_some() {
                    let product = inputs[0].product_string().unwrap_or("DAREU EM901X");
                    let dev = Device {
                        id: format!("dareu:{VID:04x}:{PID:04x}"),
                        name: product.to_string(),
                        vendor: "DAREU".into(),
                        model: Some(product.to_string()),
                        kind: DeviceKind::Mouse,
                        provider: "dareu",
                        locator: format!("{fpath}\t{ipath}\t{VID:04x}\t{PID:04x}"),
                    };
                    *known = Some(dev.clone());
                    out.push(dev);
                } else {
                    out.extend(known.take());
                }
            }
            out
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut parts = device.locator.split('\t');
            let (Some(f), Some(i)) = (parts.next(), parts.next()) else { return BatteryStatus::GONE };
            let (Some(cmd), Some(input)) = (open(f), open(i)) else { return BatteryStatus::GONE };
            poll(&cmd, &input).unwrap_or(BatteryStatus::ASLEEP)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::fake::FakeHid;

    #[test]
    fn battery_reply() {
        let cmd = FakeHid::new(|_| vec![]);
        let input = FakeHid::new(|_| {
            let mut r = vec![0u8; 17];
            r[0] = 0x09;
            r[1] = 0x04;
            r[6] = 88;
            r[7] = 1;
            vec![r]
        });
        input.write(&[0]).unwrap(); // queue the reply the dongle would push
        let s = poll(&cmd, &input).unwrap();
        assert_eq!((s.level, s.charge), (Some(88), Charge::Charging));
    }

    #[test]
    fn silence_is_none() {
        let silent = FakeHid::new(|_| vec![]);
        assert_eq!(poll(&silent, &silent), None);
    }
}
