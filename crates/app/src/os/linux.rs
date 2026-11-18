//! Linux desktop integration over freedesktop standards, so it works the same
//! on GNOME, KDE, XFCE, Cinnamon... and inside Flatpak / Snap:
//! StatusNotifierItem tray, org.freedesktop.Notifications, XDG autostart
//! (Background portal under Flatpak), xdg-open, a flock for single instance.

use super::*;
use creamui::AppHandle;
use ksni::blocking::TrayMethods;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use zbus::zvariant::Value;

/// Reverse-DNS id shared by the .desktop file, metainfo, icon, Flatpak and Snap.
pub const APP_ID: &str = "io.github.gonzxaj.LowBattery";
/// Name of the XDG autostart entry; snapcraft.yaml `autostart:` must match.
const AUTOSTART_FILE: &str = "lowbattery.desktop";

fn sandboxed() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some()
}

fn session() -> Result<zbus::blocking::Connection, String> {
    zbus::blocking::Connection::session().map_err(|e| e.to_string())
}

/// $XDG_CONFIG_HOME, which Snap and Flatpak already point inside the sandbox.
fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// XDG autostart entry. Flatpak can't write the host's ~/.config, so it asks
/// the Background portal; under Snap, snapd starts the app the yaml names.
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if std::env::var_os("FLATPAK_ID").is_some() {
        let mut opts: HashMap<&str, Value> = HashMap::new();
        opts.insert("reason", Value::from("Show peripheral battery levels after login"));
        opts.insert("autostart", Value::from(enabled));
        opts.insert("commandline", Value::from(vec!["lowbattery", "--minimized"]));
        // shortcut: the answer comes on a Request signal we don't wait for; a denial
        // only shows in the portal's own UI. Wait for it if users report confusion.
        return session()?
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Background"),
                "RequestBackground",
                &("", opts),
            )
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    let file = config_home().join("autostart").join(AUTOSTART_FILE);
    if !enabled {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", file.display())),
            _ => Ok(()),
        };
    }
    let exe = if std::env::var_os("SNAP").is_some() {
        "lowbattery".to_string()
    } else {
        format!("\"{}\"", std::env::current_exe().map_err(|e| e.to_string())?.display())
    };
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=LowBattery\nExec={exe} --minimized\nIcon={APP_ID}\nX-GNOME-Autostart-enabled=true\n"
    );
    std::fs::create_dir_all(file.parent().unwrap_or(&file))
        .and_then(|()| std::fs::write(&file, entry))
        .map_err(|e| format!("{}: {e}", file.display()))
}

/// The session's Wayland socket, kept for the apps we launch after `prefer_x11`.
static WAYLAND_DISPLAY: std::sync::OnceLock<std::ffi::OsString> = std::sync::OnceLock::new();

/// Wayland lets no client keep a window above the others (or place it), so a
/// pinned widget runs the whole app on XWayland, where GNOME/KDE honor both.
/// Must run before the first window: winit picks Wayland whenever
/// WAYLAND_DISPLAY is set. No-op without XWayland (DISPLAY unset, e.g. Flatpak's
/// fallback-x11), where the widget just isn't kept on top.
pub fn prefer_x11(on_top: bool) {
    if !super::on_top_needs_restart(on_top) {
        return;
    }
    if let Some(wl) = std::env::var_os("WAYLAND_DISPLAY") {
        let _ = WAYLAND_DISPLAY.set(wl);
    }
    log::info!("widget kept on top: running on X11 (XWayland); Wayland can't keep windows above");
    // Single-threaded here: called first thing in main.
    std::env::remove_var("WAYLAND_DISPLAY");
}

/// Linux apps get `--minimized` from the autostart entry itself.
pub fn launched_at_startup() -> bool {
    false
}

/// X11 only (the app runs there when the widget is kept on top): gives the
/// window our icon (`_NET_WM_ICON`, used by some taskbars and Alt+Tab) and,
/// for the floating widget (`tool`), keeps it out of the dock/taskbar and
/// Alt+Tab (`_NET_WM_STATE_SKIP_TASKBAR` / `_SKIP_PAGER`, plus `_ABOVE` when
/// `on_top`). Wayland has no equivalent: docks take the icon from the .desktop
/// file via the app_id.
///
/// creamui calls this before the window is first mapped, so the state goes in
/// the property the window manager reads at map. Sent later as a client
/// message, Ubuntu Dock had already counted the widget as an open window and
/// kept showing the app as running (and its "Quit" skipped the widget).
pub fn decorate_window(w: &creamui::WindowHandle, tool: bool, on_top: bool) {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    use x11_dl::xlib;
    let (Some(RawWindowHandle::Xlib(window)), Some(RawDisplayHandle::Xlib(display))) = (w.raw_window_handle(), w.raw_display_handle())
    else {
        return;
    };
    let (Some(display), Ok(x)) = (display.display, xlib::Xlib::open()) else { return };
    // winit's own Xlib connection, used from its (the UI) thread.
    let dpy = display.as_ptr().cast::<xlib::Display>();
    let window = window.window;
    unsafe {
        let atom = |name: &std::ffi::CStr| (x.XInternAtom)(dpy, name.as_ptr(), xlib::False);
        if let Some(icon) = icon_cardinals() {
            (x.XChangeProperty)(dpy, window, atom(c"_NET_WM_ICON"), xlib::XA_CARDINAL, 32, xlib::PropModeReplace, icon.as_ptr().cast(), icon.len() as i32);
        }
        if tool {
            // Atoms are C longs for format 32. winit's own always-on-top request
            // is a client message, which the WM drops for a not-yet-mapped window.
            let mut state: Vec<std::os::raw::c_ulong> = vec![atom(c"_NET_WM_STATE_SKIP_TASKBAR"), atom(c"_NET_WM_STATE_SKIP_PAGER")];
            if on_top {
                state.push(atom(c"_NET_WM_STATE_ABOVE"));
            }
            (x.XChangeProperty)(dpy, window, atom(c"_NET_WM_STATE"), xlib::XA_ATOM, 32, xlib::PropModeAppend, state.as_ptr().cast(), state.len() as i32);
        }
        (x.XFlush)(dpy);
    }
}

/// `_NET_WM_ICON` data: width, height, then one ARGB pixel per CARDINAL
/// (a C `long`, as Xlib wants for format 32).
fn icon_cardinals() -> Option<Vec<std::os::raw::c_ulong>> {
    let logo = creamui::ImageData::from_bytes(include_bytes!("../../../../assets/logo-96.png")).ok()?;
    let mut out = vec![logo.width() as std::os::raw::c_ulong, logo.height() as std::os::raw::c_ulong];
    out.extend(logo.pixels().as_chunks::<4>().0.iter().map(|&px| straight_argb(px) as std::os::raw::c_ulong));
    Some(out)
}

/// Premultiplied RGBA (creamui's decode) -> straight ARGB (what EWMH expects).
fn straight_argb([r, g, b, a]: [u8; 4]) -> u32 {
    let straight = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) };
    (a as u32) << 24 | straight(r) << 16 | straight(g) << 8 | straight(b)
}

/// shortcut: a second launch just exits; it does not ask the first one to show its window.
pub fn single_instance() -> bool {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let Ok(file) = std::fs::File::create(dir.join("lowbattery.lock")) else { return true };
    match file.try_lock() {
        Ok(()) => {
            std::mem::forget(file); // the lock lives as long as the process
            true
        }
        Err(std::fs::TryLockError::WouldBlock) => false,
        Err(_) => true, // filesystem without locks: better two copies than none
    }
}

pub fn notify(title: &str, body: &str) {
    log::info!("notify: {title} / {body}");
    let (title, body) = (title.to_string(), body.to_string());
    // Own thread: a slow or missing notification daemon must not stall the UI.
    std::thread::spawn(move || {
        let sent = session().and_then(|c| {
            c.call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &("LowBattery", 0u32, APP_ID, title.as_str(), body.as_str(), Vec::<&str>::new(), HashMap::<&str, Value>::new(), -1i32),
            )
            .map_err(|e| e.to_string())
        });
        if let Err(e) = sent {
            log::error!("notify: {e}");
        }
    });
}

/// Opens a file with the desktop's default handler.
pub fn open_file(path: &std::path::Path) {
    let path = path.to_owned();
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&path);
    if let Some(wl) = WAYLAND_DISPLAY.get() {
        cmd.env("WAYLAND_DISPLAY", wl); // the editor shouldn't inherit our X11 fallback
    }
    std::thread::spawn(move || match cmd.status() {
        Ok(s) if s.success() => {}
        r => log::error!("xdg-open {}: {r:?}", path.display()),
    });
}

/// inotify on the sysfs class dirs: kernfs emits create/delete when a HID
/// device or a power_supply node appears/disappears (USB, Bluetooth). The
/// events are only a hint — the periodic poll stays the source of truth.
pub fn watch_hotplug(cmd: mpsc::Sender<Command>) {
    std::thread::Builder::new()
        .name("hotplug".into())
        .spawn(move || unsafe {
            let fd = libc::inotify_init1(libc::IN_CLOEXEC);
            if fd < 0 {
                log::error!("hotplug: inotify_init1: {}", std::io::Error::last_os_error());
                return;
            }
            for path in ["/sys/class/hidraw", "/sys/class/power_supply"] {
                let Ok(c) = std::ffi::CString::new(path) else { continue };
                let mask = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_ONLYDIR;
                if libc::inotify_add_watch(fd, c.as_ptr(), mask) < 0 {
                    log::warn!("hotplug: watching {path}: {}", std::io::Error::last_os_error());
                }
            }
            let hotplug = super::Hotplug::default();
            let mut buf = [0u8; 4096];
            loop {
                let n = libc::read(fd, buf.as_mut_ptr().cast(), buf.len());
                if n < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    log::error!("hotplug: inotify read: {e}");
                    return;
                }
                // inotify_event: i32 wd, u32 mask, u32 cookie, u32 len, name[len]
                let mut off = 0usize;
                while off + 16 <= n as usize {
                    let mask = u32::from_ne_bytes(buf[off + 4..off + 8].try_into().unwrap());
                    let len = u32::from_ne_bytes(buf[off + 12..off + 16].try_into().unwrap()) as usize;
                    const TRIGGER: u32 = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_Q_OVERFLOW;
                    if mask & TRIGGER != 0 {
                        hotplug.trigger(&cmd);
                    }
                    off += 16 + len;
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|e| log::error!("hotplug thread: {e}"));
}

/// SNI wants ARGB32 in network byte order; battery_icon is RGBA.
fn argb(mut px: Vec<u8>) -> Vec<u8> {
    px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.rotate_right(1));
    px
}

/// State the tray service thread renders; menu clicks go back over `tx`.
struct Model {
    /// SNI id: "lowbattery" for the aggregate icon, "lowbattery-<device>" per device.
    id: String,
    /// Passive items are hidden by GNOME's AppIndicator and tucked away by KDE.
    passive: bool,
    lang: Lang,
    tooltip: String,
    /// Shown at the top of the menu: GNOME's AppIndicator draws no tooltip.
    devices: Vec<String>,
    icon: Vec<u8>,
    tx: mpsc::Sender<TrayAction>,
}

impl ksni::Tray for Model {
    fn id(&self) -> String {
        self.id.clone()
    }
    fn status(&self) -> ksni::Status {
        if self.passive { ksni::Status::Passive } else { ksni::Status::Active }
    }
    fn title(&self) -> String {
        "LowBattery".into()
    }
    fn category(&self) -> ksni::Category {
        ksni::Category::Hardware
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![ksni::Icon { width: 32, height: 32, data: self.icon.clone() }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        let (title, description) = self.tooltip.split_once('\n').unwrap_or((&self.tooltip, ""));
        ksni::ToolTip { title: title.into(), description: description.into(), ..Default::default() }
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.tx.send(TrayAction::Open);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let item = |label: &str, action: TrayAction| -> ksni::MenuItem<Self> {
            ksni::menu::StandardItem {
                label: label.into(),
                activate: Box::new(move |m: &mut Self| {
                    let _ = m.tx.send(action);
                }),
                ..Default::default()
            }
            .into()
        };
        let [open, refresh, widget, log, quit] = tray_labels(self.lang);
        let info = |label: String| -> ksni::MenuItem<Self> { ksni::menu::StandardItem { label, enabled: false, ..Default::default() }.into() };
        let mut menu: Vec<ksni::MenuItem<Self>> = if self.devices.is_empty() {
            vec![info(self.lang.t("No devices", "Sin dispositivos").into())]
        } else {
            // A literal "_" would be read as a mnemonic marker.
            self.devices.iter().map(|d| info(d.replace('_', "__"))).collect()
        };
        menu.push(ksni::MenuItem::Separator);
        menu.extend([
            item(open, TrayAction::Open),
            item(refresh, TrayAction::Refresh),
            item(widget, TrayAction::ToggleWidget),
            item(log, TrayAction::OpenLog),
            ksni::MenuItem::Separator,
            item(quit, TrayAction::Quit),
        ]);
        menu
    }
}

pub struct Tray {
    handle: ksni::blocking::Handle<Model>,
    rx: Mutex<Option<mpsc::Receiver<TrayAction>>>,
    last: std::cell::Cell<Option<Option<u8>>>,
    /// Per-device icons: device id, tray service, last RGBA pixels drawn.
    devices: std::cell::RefCell<Vec<DeviceIcon>>,
    tx: mpsc::Sender<TrayAction>,
    lang: std::cell::Cell<Lang>,
}

type DeviceIcon = (String, ksni::blocking::Handle<Model>, Vec<u8>);

fn spawn(model: Model) -> Result<ksni::blocking::Handle<Model>, String> {
    // Sandboxes can't own the well-known org.kde.StatusNotifierItem-<pid> name.
    model.disable_dbus_name(sandboxed()).spawn().map_err(|e| e.to_string())
}

impl Tray {
    /// Fails when no StatusNotifierWatcher runs (GNOME without the
    /// AppIndicator extension); main.rs then keeps the window visible.
    pub fn new(lang: Lang) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let model = Model {
            id: "lowbattery".into(),
            passive: false,
            lang,
            tooltip: "lowbattery".into(),
            devices: Vec::new(),
            icon: argb(battery_icon(None, 0)),
            tx: tx.clone(),
        };
        let handle = spawn(model)?;
        Ok(Self { handle, rx: Mutex::new(Some(rx)), last: Default::default(), devices: Default::default(), tx, lang: lang.into() })
    }

    pub fn set_lang(&self, lang: Lang) {
        self.lang.set(lang);
        self.handle.update(|m| m.lang = lang);
        for (_, h, _) in self.devices.borrow().iter() {
            h.update(|m| m.lang = lang);
        }
    }

    /// Clicks arrive on the ksni thread; forward them to the UI thread.
    pub fn listen(&self, handle: &AppHandle, on: impl Fn(TrayAction) + 'static) {
        let Some(rx) = self.rx.lock().ok().and_then(|mut r| r.take()) else { return };
        let rx = Mutex::new(rx);
        crate::forward(handle, Arc::new(move || rx.lock().ok()?.recv().ok()), Rc::new(on));
    }

    pub fn update(&self, states: &[DeviceState], cfg: &Config) {
        let tip = tooltip(states);
        let lines = device_lines(states);
        let level = worst(states);
        let icon = (self.last.get() != Some(level)).then(|| {
            self.last.set(Some(level));
            argb(battery_icon(level, cfg.low_threshold))
        });

        let wanted = device_icons(states, cfg);
        let mut devices = self.devices.borrow_mut();
        devices.retain(|(id, h, _)| {
            let keep = wanted.iter().any(|(w, ..)| w == id);
            if !keep {
                let _ = h.shutdown(); // drops the item from the panel; no need to wait
            }
            keep
        });
        for (id, dev_tip, px) in wanted {
            if let Some((_, h, last)) = devices.iter_mut().find(|(d, ..)| *d == id) {
                let icon = (*last != px).then(|| argb(px.clone()));
                *last = px;
                let lines = lines.clone();
                h.update(move |m| {
                    m.tooltip = dev_tip;
                    m.devices = lines;
                    if let Some(icon) = icon {
                        m.icon = icon;
                    }
                });
                continue;
            }
            let model = Model {
                id: format!("lowbattery-{id}"),
                passive: false,
                lang: self.lang.get(),
                tooltip: dev_tip,
                devices: lines.clone(),
                icon: argb(px.clone()),
                tx: self.tx.clone(),
            };
            match spawn(model) {
                Ok(h) => devices.push((id, h, px)),
                Err(e) => log::error!("tray icon for {id}: {e}"),
            }
        }
        // The aggregate icon only fills in while no device has its own.
        let passive = !devices.is_empty();
        self.handle.update(move |m| {
            m.tooltip = tip;
            m.devices = lines;
            m.passive = passive;
            if let Some(icon) = icon {
                m.icon = icon;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_pixels_are_unpremultiplied_argb() {
        assert_eq!(super::straight_argb([128, 64, 0, 128]), 0x80FF_8000);
        assert_eq!(super::straight_argb([0, 0, 0, 0]), 0);
        assert_eq!(super::straight_argb([255, 255, 255, 255]), 0xFFFF_FFFF);
        let icon = super::icon_cardinals().expect("logo decodes");
        assert_eq!(icon.len(), 2 + (icon[0] * icon[1]) as usize);
    }

    #[test]
    fn argb_moves_alpha_first() {
        assert_eq!(super::argb(vec![1, 2, 3, 4]), [4, 1, 2, 3]);
    }
}
