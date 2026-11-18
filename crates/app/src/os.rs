//! OS integration: tray icon, toast notifications, autostart, single instance.
//! Real implementations on Windows and Linux; inert stubs elsewhere.

use battery_core::config::{Config, Lang};
use battery_core::monitor::{Command, DeviceState};
use battery_core::{Charge, Link};
use std::sync::mpsc;

#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))] // only real trays emit these
pub enum TrayAction {
    Open,
    Refresh,
    ToggleWidget,
    OpenLog,
    Quit,
}

/// Coalesces hotplug bursts (plugging a hub fires many events at once) into
/// one monitor refresh ~700ms after the last event.
#[derive(Clone, Default)]
pub struct Hotplug(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl Hotplug {
    /// Call on each OS device event.
    pub fn trigger(&self, cmd: &mpsc::Sender<Command>) {
        use std::sync::atomic::Ordering;
        let gen = self.0.fetch_add(1, Ordering::Relaxed) + 1;
        let (this, cmd) = (self.clone(), cmd.clone());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(700));
            if this.0.load(Ordering::Relaxed) == gen {
                log::info!("device hotplug: refreshing");
                let _ = cmd.send(Command::Refresh);
            }
        });
    }
}

/// Listens for device plug/unplug and triggers an early monitor poll on each
/// burst. The periodic poll stays the source of truth: events are only a hint
/// (they can be lost), so nothing depends on them arriving.
pub fn watch_hotplug(cmd: mpsc::Sender<Command>) {
    #[cfg(windows)]
    win::watch_hotplug(cmd);
    #[cfg(target_os = "linux")]
    linux::watch_hotplug(cmd);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = cmd;
}

/// Window app ID: Linux docks match it to our .desktop file (name, icon).
pub fn app_id() -> Option<String> {
    #[cfg(target_os = "linux")]
    return Some(APP_ID.to_owned());
    #[cfg(not(target_os = "linux"))]
    None
}

/// True on a Linux Wayland session with XWayland available while the app still
/// runs on Wayland: keeping the widget on top needs a restart (see `prefer_x11`).
pub fn on_top_needs_restart(on_top: bool) -> bool {
    cfg!(target_os = "linux") && on_top && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some()
}

/// Lowest live level drives the tray glyph (the device you need to charge).
fn worst(states: &[DeviceState]) -> Option<u8> {
    states.iter().filter(|s| s.status.link == Link::Connected).filter_map(|s| s.status.level).min()
}

/// "Name 78%" per connected device that has a battery, "⚡" while charging.
/// Feeds the tray tooltip (Windows, KDE...) and the Linux tray menu (GNOME
/// draws no tooltips).
fn device_lines(states: &[DeviceState]) -> Vec<String> {
    states
        .iter()
        .filter(|s| s.status.link == Link::Connected && s.device.provider != battery_core::diag::GENERIC)
        .map(device_line)
        .collect()
}

fn device_line(s: &DeviceState) -> String {
    let bolt = if matches!(s.status.charge, Charge::Charging | Charge::Full) { " ⚡" } else { "" };
    match s.status.level {
        Some(l) => format!("{} {l}%{bolt}", s.device.name),
        None => format!("{} —", s.device.name),
    }
}

/// One tray icon per device pinned in settings: (device id, tooltip,
/// 32x32 RGBA). Opt-in because each icon costs a D-Bus connection on Linux.
/// Offline devices keep their last level, grayed.
fn device_icons(states: &[DeviceState], cfg: &Config) -> Vec<(String, String, Vec<u8>)> {
    states
        .iter()
        .filter(|s| s.device.provider != battery_core::diag::GENERIC && cfg.tray.shown.contains(&s.device.id))
        .map(|s| {
            let live = s.status.link == Link::Connected && s.status.level.is_some();
            let level = s.status.level.or(s.last_level);
            let color = level_color(level.filter(|_| live), cfg.low_threshold);
            (s.device.id.clone(), device_line(s), crate::icons::tray_icon(s.device.kind, level, color))
        })
        .collect()
}

/// Gray when unknown, then red / amber / green like the main window.
fn level_color(level: Option<u8>, threshold: u8) -> [u8; 4] {
    match level {
        None => [0x8A, 0x8C, 0x91, 0xFF],
        Some(l) if l <= threshold => [0xFF, 0x63, 0x5C, 0xFF],
        Some(l) if l <= 50 => [0xF2, 0xB5, 0x4A, 0xFF],
        Some(_) => [0x5C, 0xD6, 0x9A, 0xFF],
    }
}

fn tooltip(states: &[DeviceState]) -> String {
    let mut t = String::from("lowbattery");
    for line in device_lines(states) {
        if t.len() + 1 + line.len() > 127 {
            break; // Windows truncates tray tooltips at 128 UTF-16 units (bytes >= units)
        }
        t.push('\n');
        t.push_str(&line);
    }
    t
}

fn tray_labels(lang: Lang) -> [&'static str; 5] {
    [
        lang.t("Open lowbattery", "Abrir lowbattery"),
        lang.t("Refresh now", "Actualizar ahora"),
        lang.t("Show/hide widget", "Mostrar/ocultar widget"),
        lang.t("View log", "Ver registro (log)"),
        lang.t("Quit", "Salir"),
    ]
}

/// 32x32 RGBA battery glyph filled to `level`.
pub fn battery_icon(level: Option<u8>, threshold: u8) -> Vec<u8> {
    const S: usize = 32;
    let mut px = vec![0u8; S * S * 4];
    let outline = [0xEC, 0xEB, 0xE6, 0xFF];
    let fill = level_color(level, threshold);
    let mut put = |x: usize, y: usize, c: [u8; 4]| px[(y * S + x) * 4..(y * S + x) * 4 + 4].copy_from_slice(&c);
    // body 2..27 x 8..24, 2px border; nub 27..30 x 13..19
    for y in 8..24 {
        for x in 2..27 {
            if !(4..=24).contains(&x) || !(10..=21).contains(&y) {
                put(x, y, outline);
            }
        }
    }
    for y in 13..19 {
        for x in 27..30 {
            put(x, y, outline);
        }
    }
    let inner = 19; // x 5..24 inside a 1px gap
    let w = (inner * level.unwrap_or(0) as usize).div_ceil(100).min(inner);
    for y in 11..21 {
        for x in 5..5 + w {
            put(x, y, fill);
        }
    }
    px
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use creamui::AppHandle;
    use std::rc::Rc;
    use std::sync::Arc;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND};
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    const AUMID: &str = "LowBattery.Monitor";
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn set_string(key: &str, name: &str, value: &str) -> Result<(), String> {
        let (k, n, v) = (wide(key), wide(name), wide(value));
        let rc = unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32)
        };
        if rc == 0 { Ok(()) } else { Err(format!("RegSetKeyValueW({key}) = {rc}")) }
    }

    const STARTUP_TASK: &str = "LowBatteryStartup"; // must match AppxManifest.xml

    /// MSIX: the manifest's StartupTask (HKCU\Run writes are virtualized in a
    /// package). Unpackaged: HKCU Run key. Neither needs admin rights.
    pub fn set_autostart(enabled: bool) -> Result<(), String> {
        if battery_core::config::package_family_name().is_some() {
            return startup_task(enabled);
        }
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            set_string(RUN_KEY, "LowBattery", &format!("\"{}\" --minimized", exe.display()))
        } else {
            let (k, n) = (wide(RUN_KEY), wide("LowBattery"));
            let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
            if rc == 0 || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("RegDeleteKeyValueW = {rc}")) }
        }
    }

    /// Gives a window the exe's icon (resource 1) and, for the floating widget
    /// (`tool`), the tool-window style: no taskbar button, no Alt+Tab entry.
    /// Works on the window's own HWND, which creamui exposes.
    pub fn decorate_window(w: &creamui::WindowHandle, tool: bool, _on_top: bool) {
        use raw_window_handle::RawWindowHandle;
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let Some(RawWindowHandle::Win32(handle)) = w.raw_window_handle() else { return };
        let hwnd = handle.hwnd.get() as HWND;
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            for (which, metric_x, metric_y) in [(ICON_BIG, SM_CXICON, SM_CYICON), (ICON_SMALL, SM_CXSMICON, SM_CYSMICON)] {
                // MAKEINTRESOURCE(1)
                let icon = LoadImageW(module, 1 as _, IMAGE_ICON, GetSystemMetrics(metric_x), GetSystemMetrics(metric_y), LR_DEFAULTCOLOR);
                if !icon.is_null() {
                    SendMessageW(hwnd, WM_SETICON, which as usize, icon as isize);
                }
            }
            if !tool {
                return;
            }
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let tool_style = (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
            if tool_style != ex {
                // The taskbar only drops the button if the style changes while hidden.
                let visible = IsWindowVisible(hwnd) != 0;
                if visible {
                    ShowWindow(hwnd, SW_HIDE);
                }
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, tool_style);
                if visible {
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
    }

    /// Channel + debouncer shared with the hotplug wndproc (see `watch_hotplug`).
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    static HOTPLUG: std::sync::OnceLock<(mpsc::Sender<Command>, super::Hotplug)> = std::sync::OnceLock::new();

    unsafe extern "system" fn hotplug_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        if msg == WM_DEVICECHANGE && matches!(wparam as u32, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE) {
            if let Some((cmd, hotplug)) = HOTPLUG.get() {
                hotplug.trigger(cmd);
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    /// Message-only window + RegisterDeviceNotification: USB arrivals/removals
    /// post WM_DEVICECHANGE to our own wndproc, independent of any creamui
    /// window (the main window may not even exist yet).
    pub fn watch_hotplug(cmd: mpsc::Sender<Command>) {
        use windows_sys::Win32::Devices::Usb::GUID_DEVINTERFACE_USB_DEVICE;
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        if HOTPLUG.set((cmd, super::Hotplug::default())).is_err() {
            return;
        }
        std::thread::spawn(|| unsafe {
            let class = wide("LowBatteryHotplug");
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(hotplug_wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: GetModuleHandleW(std::ptr::null()),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                log::error!("hotplug: RegisterClassW: {}", GetLastError());
                return;
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                wc.hInstance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                log::error!("hotplug: CreateWindowExW: {}", GetLastError());
                return;
            }
            let filter = DEV_BROADCAST_HANDLE {
                dbch_size: std::mem::size_of::<DEV_BROADCAST_HANDLE>() as u32,
                dbch_devicetype: DBT_DEVTYP_DEVICEINTERFACE,
                dbch_reserved: 0,
                dbch_handle: hwnd,
                dbch_hdevnotify: std::ptr::null_mut(),
                dbch_eventguid: GUID_DEVINTERFACE_USB_DEVICE,
                dbch_nameoffset: 0,
                dbch_data: [0],
            };
            if RegisterDeviceNotificationW(hwnd, &filter as *const _ as *const core::ffi::c_void, DEVICE_NOTIFY_WINDOW_HANDLE).is_null() {
                log::error!("hotplug: RegisterDeviceNotificationW: {}", GetLastError());
                return;
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    /// Opens a file with its default handler (Notepad for .log).
    pub fn open_file(path: &std::path::Path) {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let (verb, file) = (wide("open"), wide(&path.to_string_lossy()));
        let rc = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
        if (rc as isize) <= 32 {
            log::error!("open {}: ShellExecuteW = {}", path.display(), rc as isize);
        }
    }

    /// WinRT calls on a fresh MTA thread: blocking `join()` must not run on the UI (STA) thread.
    fn startup_task(enabled: bool) -> Result<(), String> {
        use windows::ApplicationModel::{StartupTask, StartupTaskState};
        std::thread::spawn(move || -> Result<(), String> {
            unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) }
                .map_err(|e| e.to_string())?;
            let task = StartupTask::GetAsync(&STARTUP_TASK.into()).and_then(|op| op.join()).map_err(|e| e.to_string())?;
            if !enabled {
                return task.Disable().map_err(|e| e.to_string());
            }
            match task.RequestEnableAsync().and_then(|op| op.join()).map_err(|e| e.to_string())? {
                StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => Ok(()),
                StartupTaskState::DisabledByUser => Err("disabled by the user in Task Manager > Startup apps".into()),
                s => Err(format!("startup task state {}", s.0)),
            }
        })
        .join()
        .map_err(|_| "startup task thread panicked".to_string())?
    }

    /// True when Windows launched the package at sign-in through its StartupTask
    /// (a StartupTask can't pass `--minimized`).
    pub fn launched_at_startup() -> bool {
        use windows::ApplicationModel::Activation::ActivationKind;
        if battery_core::config::package_family_name().is_none() {
            return false;
        }
        // Own MTA thread: the main thread must stay uninitialized for winit's OleInitialize (STA).
        std::thread::spawn(|| {
            unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) }.ok()?;
            let kind = windows::ApplicationModel::AppInstance::GetActivatedEventArgs().and_then(|a| a.Kind()).ok()?;
            Some(kind == ActivationKind::StartupTask)
        })
        .join()
        .ok()
        .flatten()
        .unwrap_or(false)
    }

    /// shortcut: a second launch just exits; it does not ask the first one to show its window.
    pub fn single_instance() -> bool {
        let name = wide(r"Local\LowBattery.SingleInstance");
        // Handle intentionally leaked: it must live as long as the process.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        !handle.is_null() && unsafe { GetLastError() } != ERROR_ALREADY_EXISTS
    }

    /// Packaged: the package's own AUMID (`<family>!App`). Unpackaged: an
    /// AppUserModelID registered under HKCU (no shortcut/installer step).
    pub fn notify(title: &str, body: &str) {
        use tauri_winrt_notification::Toast;
        let packaged = battery_core::config::package_family_name().map(|pfn| format!("{pfn}!App"));
        let key = format!(r"Software\Classes\AppUserModelId\{AUMID}");
        let aumid = match &packaged {
            Some(id) => id.as_str(),
            None => match set_string(&key, "DisplayName", "LowBattery") {
                Ok(()) => AUMID,
                Err(_) => Toast::POWERSHELL_APP_ID,
            },
        };
        log::info!("toast ({aumid}): {title} / {body}");
        if let Err(e) = Toast::new(aumid).title(title).text1(body).show() {
            log::error!("toast: {e}");
        }
    }

    pub struct Tray {
        icon: TrayIcon,
        menu: Menu,
        items: [MenuItem; 5],
        ids: [MenuId; 5],
        last: std::cell::Cell<Option<Option<u8>>>,
        /// Per-device icons: device id, icon, last pixels drawn.
        devices: std::cell::RefCell<Vec<(String, TrayIcon, Vec<u8>)>>,
    }

    impl Tray {
        pub fn set_lang(&self, lang: Lang) {
            for (item, label) in self.items.iter().zip(tray_labels(lang)) {
                item.set_text(label);
            }
        }

        pub fn new(lang: Lang) -> Result<Self, String> {
            let [open, refresh, widget, log, quit] = tray_labels(lang).map(|l| MenuItem::new(l, true, None));
            let menu = Menu::new();
            menu.append_items(&[&open, &refresh, &widget, &log, &PredefinedMenuItem::separator(), &quit])
                .map_err(|e| e.to_string())?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu.clone()))
                .with_menu_on_left_click(false)
                .with_tooltip("lowbattery")
                .with_icon(Icon::from_rgba(battery_icon(None, 0), 32, 32).map_err(|e| e.to_string())?)
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Self {
                icon,
                ids: [open.id().clone(), refresh.id().clone(), widget.id().clone(), log.id().clone(), quit.id().clone()],
                items: [open, refresh, widget, log, quit],
                last: Default::default(),
                menu,
                devices: Default::default(),
            })
        }

        /// Events arrive on tray-icon's channels; a blocking reader thread per
        /// channel forwards them to the UI thread (see `crate::forward`).
        pub fn listen(&self, handle: &AppHandle, on: impl Fn(TrayAction) + 'static) {
            let on: Rc<dyn Fn(TrayAction)> = Rc::new(on);
            let ids = self.ids.clone();
            crate::forward(
                handle,
                Arc::new(move || loop {
                    let ev = MenuEvent::receiver().recv().ok()?;
                    let actions =
                        [TrayAction::Open, TrayAction::Refresh, TrayAction::ToggleWidget, TrayAction::OpenLog, TrayAction::Quit];
                    if let Some((a, _)) = actions.into_iter().zip(&ids).find(|(_, id)| **id == ev.id) {
                        return Some(a);
                    }
                }),
                on.clone(),
            );
            crate::forward(
                handle,
                Arc::new(|| loop {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                        TrayIconEvent::receiver().recv().ok()?
                    {
                        return Some(TrayAction::Open);
                    }
                }),
                on,
            );
        }

        pub fn update(&self, states: &[DeviceState], cfg: &Config) {
            let _ = self.icon.set_tooltip(Some(tooltip(states)));
            let level = worst(states);
            if self.last.get() != Some(level) {
                self.last.set(Some(level));
                if let Ok(icon) = Icon::from_rgba(battery_icon(level, cfg.low_threshold), 32, 32) {
                    let _ = self.icon.set_icon(Some(icon));
                }
            }
            let wanted = device_icons(states, cfg);
            let mut devices = self.devices.borrow_mut();
            // Dropping a TrayIcon removes it from the taskbar.
            devices.retain(|(id, ..)| wanted.iter().any(|(w, ..)| w == id));
            for (id, tip, px) in wanted {
                if let Some((_, icon, last)) = devices.iter_mut().find(|(d, ..)| *d == id) {
                    let _ = icon.set_tooltip(Some(&tip));
                    if *last != px {
                        if let Ok(i) = Icon::from_rgba(px.clone(), 32, 32) {
                            let _ = icon.set_icon(Some(i));
                        }
                        *last = px;
                    }
                    continue;
                }
                let built = Icon::from_rgba(px.clone(), 32, 32).map_err(|e| e.to_string()).and_then(|i| {
                    TrayIconBuilder::new()
                        .with_menu(Box::new(self.menu.clone()))
                        .with_menu_on_left_click(false)
                        .with_tooltip(&tip)
                        .with_icon(i)
                        .build()
                        .map_err(|e| e.to_string())
                });
                match built {
                    Ok(icon) => devices.push((id, icon, px)),
                    Err(e) => log::error!("tray icon for {id}: {e}"),
                }
            }
            // The aggregate icon only fills in while no device has its own.
            let _ = self.icon.set_visible(devices.is_empty());
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(any(windows, target_os = "linux")))]
pub use stub::*;

#[cfg(not(any(windows, target_os = "linux")))]
mod stub {
    use super::*;

    pub fn set_autostart(_: bool) -> Result<(), String> {
        Err("autostart is not implemented on this OS".into())
    }
    pub fn single_instance() -> bool {
        true
    }
    pub fn notify(title: &str, body: &str) {
        log::info!("[notify] {title}: {body}");
    }
    pub fn open_file(path: &std::path::Path) {
        log::info!("log file: {}", path.display());
    }
    pub fn decorate_window(_: &creamui::WindowHandle, _: bool, _: bool) {}
    pub fn launched_at_startup() -> bool {
        false
    }
    pub struct Tray;
    impl Tray {
        pub fn set_lang(&self, _: Lang) {}
        pub fn new(_: Lang) -> Result<Self, String> {
            Err("tray is not implemented on this OS".into())
        }
        pub fn listen(&self, _: &creamui::AppHandle, _: impl Fn(TrayAction) + 'static) {}
        pub fn update(&self, states: &[DeviceState], cfg: &Config) {
            let _ = (worst(states), tooltip(states), battery_icon(None, cfg.low_threshold), device_icons(states, cfg));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use battery_core::{BatteryStatus, Device, DeviceKind};

    fn state(name: &str, provider: &'static str, status: BatteryStatus) -> DeviceState {
        let device = Device { id: name.into(), name: name.into(), vendor: String::new(), model: None, kind: DeviceKind::Mouse, provider, locator: String::new() };
        DeviceState { device, status, last_level: status.level, last_charge: status.charge, last_read: None }
    }

    #[test]
    fn device_lines_show_charge_and_skip_batteryless() {
        let states = [
            state("Keyboard", "keyboards", BatteryStatus::connected(99, Charge::Charging)),
            state("Headset", "steelseries", BatteryStatus::connected(51, Charge::Discharging)),
            state("Mouse", "razer", BatteryStatus::ASLEEP),
            state("2.4G Receiver", battery_core::diag::GENERIC, BatteryStatus::PRESENT),
        ];
        assert_eq!(device_lines(&states), ["Keyboard 99% ⚡", "Headset 51%"]);
        assert_eq!(tooltip(&states), "lowbattery\nKeyboard 99% ⚡\nHeadset 51%");
    }

    #[test]
    fn icon_fill_tracks_level() {
        let filled = |px: &[u8]| (5..24).filter(|x| px[(15 * 32 + x) * 4 + 3] != 0).count();
        assert_eq!(filled(&battery_icon(Some(0), 20)), 0);
        assert_eq!(filled(&battery_icon(Some(50), 20)), 10);
        assert_eq!(filled(&battery_icon(Some(100), 20)), 19);
    }
}
