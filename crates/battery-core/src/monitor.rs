use crate::{BatteryProvider, BatteryStatus, Charge, Device, Link};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DeviceState {
    pub device: Device,
    pub status: BatteryStatus,
    /// Last level actually read; kept while the device sleeps or disconnects.
    pub last_level: Option<u8>,
    pub last_charge: Charge,
    pub last_read: Option<SystemTime>,
}

pub enum Command {
    Refresh,
    SetInterval(Duration),
}

pub struct Monitor {
    providers: Vec<Box<dyn BatteryProvider>>,
    states: BTreeMap<String, DeviceState>,
}

impl Monitor {
    pub fn new(providers: Vec<Box<dyn BatteryProvider>>) -> Self {
        Self { providers, states: BTreeMap::new() }
    }

    /// One full cycle: discover + read every device, sequentially.
    pub fn poll(&mut self) -> Vec<DeviceState> {
        let now = SystemTime::now();
        let mut seen = HashSet::new();
        let mut readings = Vec::new();
        for provider in &self.providers {
            let found = provider.discover();
            log::debug!("{}: {} device(s)", provider.name(), found.len());
            for device in found {
                let status = provider.read_battery(&device);
                readings.push((device, status));
            }
        }
        for (device, status) in readings {
            seen.insert(device.id.clone());
            self.merge(device, status, now);
        }
        for (id, state) in self.states.iter_mut() {
            if !seen.contains(id) {
                if state.status != BatteryStatus::GONE {
                    log::info!("{} \"{}\": no longer found -> disconnected", id, state.device.name);
                }
                state.status = BatteryStatus::GONE;
            }
        }
        self.states.values().cloned().collect()
    }

    fn merge(&mut self, device: Device, status: BatteryStatus, now: SystemTime) {
        match self.states.get(&device.id) {
            None => log::info!("new device {} \"{}\" via {}: {:?}", device.id, device.name, device.provider, status),
            Some(old) if old.status != status => log::info!("{} \"{}\": {:?} -> {:?}", device.id, device.name, old.status, status),
            _ => {}
        }
        let state = self.states.entry(device.id.clone()).or_insert_with(|| DeviceState {
            device: device.clone(),
            status,
            last_level: None,
            last_charge: Charge::Unknown,
            last_read: None,
        });
        state.device = device;
        state.status = status;
        if let Some(level) = status.level {
            state.last_level = Some(level);
            state.last_charge = status.charge;
            state.last_read = Some(now);
        }
    }

    /// Blocking loop for the background thread. Ends when the command
    /// channel's sender is dropped (app exit).
    pub fn run(mut self, mut interval: Duration, commands: Receiver<Command>, mut emit: impl FnMut(Vec<DeviceState>)) {
        loop {
            emit(self.poll());
            match commands.recv_timeout(interval) {
                Ok(Command::Refresh) | Err(RecvTimeoutError::Timeout) => {}
                Ok(Command::SetInterval(d)) => interval = d,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

/// Fires once when a device drops to the threshold; re-arms after charging
/// or climbing 5 points above it, so a level hovering at the edge doesn't spam.
#[derive(Default)]
pub struct LowBatteryAlerts {
    fired: HashMap<String, bool>,
}

impl LowBatteryAlerts {
    pub fn check<'a>(&mut self, states: &'a [DeviceState], threshold: u8) -> Vec<&'a DeviceState> {
        let mut out = Vec::new();
        for s in states {
            if s.status.link != Link::Connected {
                continue;
            }
            let Some(level) = s.status.level else { continue };
            let fired = self.fired.entry(s.device.id.clone()).or_insert(false);
            let charging = matches!(s.status.charge, Charge::Charging | Charge::Full);
            if charging || level > threshold.saturating_add(5) {
                *fired = false;
            } else if level <= threshold && !*fired {
                *fired = true;
                out.push(s);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::demo::Demo;
    use crate::DeviceKind;
    use std::sync::Mutex;

    struct Scripted(Mutex<Vec<Option<BatteryStatus>>>);
    impl BatteryProvider for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }
        fn discover(&self) -> Vec<Device> {
            match self.0.lock().unwrap().first() {
                Some(Some(_)) => vec![dev()],
                _ => vec![],
            }
        }
        fn read_battery(&self, _: &Device) -> BatteryStatus {
            self.0.lock().unwrap().remove(0).unwrap()
        }
    }
    fn dev() -> Device {
        Device {
            id: "x".into(),
            name: "X".into(),
            vendor: "V".into(),
            model: None,
            kind: DeviceKind::Mouse,
            provider: "scripted",
            locator: String::new(),
        }
    }

    #[test]
    fn remembers_last_level_across_sleep_and_disconnect() {
        let script = vec![
            Some(BatteryStatus::connected(55, Charge::Discharging)),
            Some(BatteryStatus::ASLEEP),
            None,
        ];
        let mut m = Monitor::new(vec![Box::new(Scripted(Mutex::new(script)))]);
        assert_eq!(m.poll()[0].last_level, Some(55));
        let s = &m.poll()[0];
        assert_eq!((s.status.link, s.last_level), (Link::Asleep, Some(55)));
        // Script exhausted -> not discovered -> disconnected, level kept.
        let s = &m.poll()[0];
        assert_eq!((s.status.link, s.last_level), (Link::Disconnected, Some(55)));
    }

    #[test]
    fn alert_fires_once_and_rearms_after_charging() {
        let mk = |level, charge| {
            vec![DeviceState {
                device: dev(),
                status: BatteryStatus::connected(level, charge),
                last_level: Some(level),
                last_charge: charge,
                last_read: None,
            }]
        };
        let mut a = LowBatteryAlerts::default();
        assert_eq!(a.check(&mk(30, Charge::Discharging), 20).len(), 0);
        assert_eq!(a.check(&mk(20, Charge::Discharging), 20).len(), 1);
        assert_eq!(a.check(&mk(19, Charge::Discharging), 20).len(), 0);
        assert_eq!(a.check(&mk(22, Charge::Discharging), 20).len(), 0); // hysteresis
        assert_eq!(a.check(&mk(19, Charge::Charging), 20).len(), 0);
        assert_eq!(a.check(&mk(18, Charge::Discharging), 20).len(), 1);
    }

    #[test]
    fn demo_provider_polls() {
        let mut m = Monitor::new(vec![Box::new(Demo::default())]);
        assert!(!m.poll().is_empty());
    }
}
