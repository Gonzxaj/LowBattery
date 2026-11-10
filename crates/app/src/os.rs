//! OS integration: tray icon, toast notifications, autostart, single instance.
//! Real implementations on Windows; inert stubs elsewhere (demo/dev builds).

use battery_core::config::Lang;
use battery_core::monitor::DeviceState;
use battery_core::Link;

#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))] // only the Windows tray emits these
pub enum TrayAction {
    Open,
    Refresh,
    ToggleWidget,
    OpenLog,
    Quit,
}

/// Lowest live level drives the tray glyph (the device you need to charge).
fn worst(states: &[DeviceState]) -> Option<u8> {
    states.iter().filter(|s| s.status.link == Link::Connected).filter_map(|s| s.status.level).min()
}

fn tooltip(states: &[DeviceState]) -> String {
    let mut t = String::from("lowbattery");
    for s in states.iter().filter(|s| s.status.link == Link::Connected) {
        let line = match s.status.level {
            Some(l) => format!("\n{} {l}%", s.device.name),
            None => format!("\n{} —", s.device.name),
        };
        if t.len() + line.len() > 127 {
            break; // Windows truncates tray tooltips at 128 UTF-16 units
        }
        t.push_str(&line);
    }
    t
}

/// 32x32 RGBA battery glyph filled to `level`.
pub fn battery_icon(level: Option<u8>, threshold: u8) -> Vec<u8> {
    const S: usize = 32;
    let mut px = vec![0u8; S * S * 4];
    let outline = [0xEC, 0xEB, 0xE6, 0xFF];
    let fill = match level {
        None => [0x8A, 0x8C, 0x91, 0xFF],
        Some(l) if l <= threshold => [0xFF, 0x63, 0x5C, 0xFF],
        Some(l) if l <= 50 => [0xF2, 0xB5, 0x4A, 0xFF],
        Some(_) => [0x5C, 0xD6, 0x9A, 0xFF],
    };
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

    /// Puts the exe's icon (resource 1) on every top-level window of the UI
    /// thread and keeps the widget out of the taskbar. creamui 0.3 has no
    /// window-icon / tool-window option and doesn't expose the HWND.
    pub fn set_window_icons() {
        use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        unsafe extern "system" fn apply(hwnd: HWND, _: LPARAM) -> BOOL {
            let module = GetModuleHandleW(std::ptr::null());
            for (which, metric_x, metric_y) in [(ICON_BIG, SM_CXICON, SM_CYICON), (ICON_SMALL, SM_CXSMICON, SM_CYSMICON)] {
                // MAKEINTRESOURCE(1)
                let icon = LoadImageW(module, 1 as _, IMAGE_ICON, GetSystemMetrics(metric_x), GetSystemMetrics(metric_y), LR_DEFAULTCOLOR);
                if !icon.is_null() {
                    SendMessageW(hwnd, WM_SETICON, which as usize, icon as isize);
                }
            }
            // The floating widget is a tool window: no taskbar button, no Alt+Tab entry.
            let mut title = [0u16; 64];
            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            if String::from_utf16_lossy(&title[..n.max(0) as usize]) == crate::ui::WIDGET_TITLE {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let tool = (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
                if tool != ex {
                    // The taskbar only drops the button if the style changes while hidden.
                    let visible = IsWindowVisible(hwnd) != 0;
                    if visible {
                        ShowWindow(hwnd, SW_HIDE);
                    }
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, tool);
                    if visible {
                        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                }
            }
            1
        }
        unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(apply), 0) };
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
        items: [MenuItem; 5],
        ids: [MenuId; 5],
        last: std::cell::Cell<Option<Option<u8>>>,
    }

    impl Tray {
        fn labels(lang: Lang) -> [&'static str; 5] {
            [
                lang.t("Open lowbattery", "Abrir lowbattery"),
                lang.t("Refresh now", "Actualizar ahora"),
                lang.t("Show/hide widget", "Mostrar/ocultar widget"),
                lang.t("View log", "Ver registro (log)"),
                lang.t("Quit", "Salir"),
            ]
        }

        pub fn set_lang(&self, lang: Lang) {
            for (item, label) in self.items.iter().zip(Self::labels(lang)) {
                item.set_text(label);
            }
        }

        pub fn new(lang: Lang) -> Result<Self, String> {
            let [open, refresh, widget, log, quit] = Self::labels(lang).map(|l| MenuItem::new(l, true, None));
            let menu = Menu::new();
            menu.append_items(&[&open, &refresh, &widget, &log, &PredefinedMenuItem::separator(), &quit])
                .map_err(|e| e.to_string())?;
            let icon = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
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

        pub fn update(&self, states: &[DeviceState], threshold: u8) {
            let _ = self.icon.set_tooltip(Some(tooltip(states)));
            let level = worst(states);
            if self.last.get() != Some(level) {
                self.last.set(Some(level));
                if let Ok(icon) = Icon::from_rgba(battery_icon(level, threshold), 32, 32) {
                    let _ = self.icon.set_icon(Some(icon));
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub use stub::*;

#[cfg(not(windows))]
mod stub {
    use super::*;

    pub fn set_autostart(_: bool) -> Result<(), String> {
        Err("autostart is only implemented on Windows".into())
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
    pub fn set_window_icons() {}
    pub fn launched_at_startup() -> bool {
        false
    }
    pub struct Tray;
    impl Tray {
        pub fn set_lang(&self, _: Lang) {}
        pub fn new(_: Lang) -> Result<Self, String> {
            Err("tray is only implemented on Windows".into())
        }
        pub fn listen(&self, _: &creamui::AppHandle, _: impl Fn(TrayAction) + 'static) {}
        pub fn update(&self, states: &[DeviceState], threshold: u8) {
            let _ = (worst(states), tooltip(states), battery_icon(None, threshold));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_fill_tracks_level() {
        let filled = |px: &[u8]| (5..24).filter(|x| px[(15 * 32 + x) * 4 + 3] != 0).count();
        assert_eq!(filled(&battery_icon(Some(0), 20)), 0);
        assert_eq!(filled(&battery_icon(Some(50), 20)), 10);
        assert_eq!(filled(&battery_icon(Some(100), 20)), 19);
    }
}
