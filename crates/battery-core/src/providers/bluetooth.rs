//! Standard Bluetooth battery as Windows itself reports it (Settings > Bluetooth):
//! LE Battery Service and classic HFP battery, exposed as an undocumented
//! BYTE devnode property on BTHLE / BTHENUM nodes. Works for any vendor.
//!
//! Windows keeps that value after a disconnect, so the connection state is
//! read separately from the device container (DEVPKEY_DeviceContainer_IsConnected).

/// Braced, lower-case GUID string as the device-query API expects.
pub fn guid_string(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> String {
    format!(
        "{{{data1:08x}-{data2:04x}-{data3:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        data4[0], data4[1], data4[2], data4[3], data4[4], data4[5], data4[6], data4[7]
    )
}

#[cfg(windows)]
pub use win::{diagnostics, Bluetooth};

#[cfg(windows)]
mod win {
    use super::guid_string;
    use crate::{BatteryProvider, BatteryStatus, Charge, Device, Link};
    use std::collections::HashMap;
    use windows_sys::core::GUID;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows_sys::Win32::Devices::DeviceQuery::{DevFreeObjectProperties, DevGetObjectProperties, DevObjectTypeDeviceContainer, DevQueryFlagNone};
    use windows_sys::Win32::Devices::Properties::*;

    const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
    const CONTAINER_CONNECTED: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x78c34fc8_104a_4aca_9ea4_524d52996e57), pid: 55 };
    const ITEM_NAME: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac), pid: 10 };

    pub struct Bluetooth;

    struct Node {
        container: String,
        name: String,
        level: u8,
    }

    unsafe fn prop(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY, buf: &mut [u8]) -> Option<(DEVPROPTYPE, usize)> {
        let (mut ty, mut size) = (0u32, 0u32);
        let ok = SetupDiGetDevicePropertyW(set, data, key, &mut ty, buf.as_mut_ptr(), buf.len() as u32, &mut size, 0);
        (ok != 0).then_some((ty, size as usize))
    }

    fn utf16(buf: &[u8]) -> String {
        let w: Vec<u16> = buf.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&c| c != 0).collect();
        String::from_utf16_lossy(&w)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// All BT devnodes under `enumerator` that carry a battery level.
    fn scan(enumerator: &str, out: &mut Vec<Node>) {
        let e = wide(enumerator);
        unsafe {
            let set = SetupDiGetClassDevsW(std::ptr::null(), e.as_ptr(), std::ptr::null_mut(), DIGCF_ALLCLASSES | DIGCF_PRESENT);
            if set == -1 {
                return; // INVALID_HANDLE_VALUE
            }
            let mut buf = [0u8; 512];
            let mut i = 0;
            loop {
                let mut data: SP_DEVINFO_DATA = std::mem::zeroed();
                data.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiEnumDeviceInfo(set, i, &mut data) == 0 {
                    break;
                }
                i += 1;
                let level = match prop(set, &data, &BATTERY, &mut buf) {
                    Some((DEVPROP_TYPE_BYTE, 1)) if buf[0] <= 100 => buf[0],
                    _ => continue,
                };
                let Some((DEVPROP_TYPE_GUID, 16)) = prop(set, &data, &DEVPKEY_Device_ContainerId, &mut buf) else { continue };
                let g: GUID = std::ptr::read_unaligned(buf.as_ptr().cast());
                let container = guid_string(g.data1, g.data2, g.data3, g.data4);
                let name = match prop(set, &data, &DEVPKEY_Device_FriendlyName, &mut buf) {
                    Some((DEVPROP_TYPE_STRING, n)) => utf16(&buf[..n]),
                    _ => String::new(),
                };
                out.push(Node { container, name, level });
            }
            SetupDiDestroyDeviceInfoList(set);
        }
    }

    /// (is_connected, display name) of a device container.
    fn container_info(id: &str) -> (Option<bool>, Option<String>) {
        let keys = [CONTAINER_CONNECTED, ITEM_NAME].map(|k| DEVPROPCOMPKEY { Key: k, Store: DEVPROP_STORE_SYSTEM, LocaleName: std::ptr::null() });
        let id = wide(id);
        let (mut n, mut props) = (0u32, std::ptr::null_mut::<DEVPROPERTY>());
        unsafe {
            if DevGetObjectProperties(DevObjectTypeDeviceContainer, id.as_ptr(), DevQueryFlagNone as u32, 2, keys.as_ptr(), &mut n, &mut props) < 0 || props.is_null() {
                return (None, None);
            }
            let (mut connected, mut name) = (None, None);
            for p in std::slice::from_raw_parts(props, n as usize) {
                if p.Buffer.is_null() {
                    continue;
                }
                let bytes = std::slice::from_raw_parts(p.Buffer as *const u8, p.BufferSize as usize);
                let k = p.CompKey.Key;
                if k.pid == CONTAINER_CONNECTED.pid && p.Type == DEVPROP_TYPE_BOOLEAN && !bytes.is_empty() {
                    connected = Some(bytes[0] != 0);
                } else if k.pid == ITEM_NAME.pid && p.Type == DEVPROP_TYPE_STRING {
                    name = Some(utf16(bytes)).filter(|s| !s.is_empty());
                }
            }
            DevFreeObjectProperties(n, props);
            (connected, name)
        }
    }

    /// Every battery-bearing BT node, unfiltered, for `GET /diag`.
    pub fn diagnostics() -> Vec<serde_json::Value> {
        let mut nodes = Vec::new();
        scan("BTHLE", &mut nodes);
        scan("BTHENUM", &mut nodes);
        nodes
            .into_iter()
            .map(|n| {
                let (connected, container_name) = container_info(&n.container);
                serde_json::json!({
                    "node_name": n.name, "container": n.container, "level": n.level,
                    "connected": connected, "container_name": container_name,
                })
            })
            .collect()
    }

    impl BatteryProvider for Bluetooth {
        fn name(&self) -> &'static str {
            "bluetooth"
        }

        fn discover(&self) -> Vec<Device> {
            let mut nodes = Vec::new();
            scan("BTHLE", &mut nodes);
            scan("BTHENUM", &mut nodes);
            // A headset has several profile nodes; one card per physical device.
            let mut by_container: HashMap<String, Node> = HashMap::new();
            for n in nodes {
                by_container.entry(n.container.clone()).or_insert(n);
            }
            by_container
                .into_values()
                .map(|n| {
                    let name = container_info(&n.container).1.unwrap_or(n.name);
                    Device {
                        id: format!("bt:{}", n.container),
                        kind: crate::guess_kind(&name),
                        name,
                        vendor: "Bluetooth".into(),
                        model: None,
                        provider: "bluetooth",
                        locator: n.container,
                    }
                })
                .collect()
        }

        fn read_battery(&self, device: &Device) -> BatteryStatus {
            let mut nodes = Vec::new();
            scan("BTHLE", &mut nodes);
            scan("BTHENUM", &mut nodes);
            let Some(node) = nodes.into_iter().find(|n| n.container == device.locator) else {
                return BatteryStatus::GONE;
            };
            let connected = container_info(&device.locator).0;
            log::debug!("bt {}: node level {} container connected {:?}", device.name, node.level, connected);
            match connected {
                // Stale value Windows kept from the last session: don't present it as live.
                Some(false) => BatteryStatus::GONE,
                // Standard BT battery has no charging flag.
                _ => BatteryStatus { level: Some(node.level), charge: Charge::Unknown, link: Link::Connected },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn guid_format() {
        let s = super::guid_string(0x8c7ed206, 0x3f8a, 0x4827, [0xb3, 0xab, 0xae, 0x9e, 0x1f, 0xae, 0xfc, 0x6c]);
        assert_eq!(s, "{8c7ed206-3f8a-4827-b3ab-ae9e1faefc6c}");
    }
}
