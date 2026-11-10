#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod logfile;
mod os;
mod ui;

use battery_core::config::{Config, Lang};
use battery_core::monitor::{Command, DeviceState, LowBatteryAlerts, Monitor};
use battery_core::providers;
use creamui::{AppBuilder, AppHandle, CloseBehavior, Color, Signal, WindowHandle, WindowOptions};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Deliver items produced by a blocking `next()` to `on` on the UI thread.
/// `next()` returning None ends the stream.
/// shortcut: re-arms one `spawn_background` per item because creamui 0.3.0 has no
/// `spawn_stream`; switch to it once a release ships it.
pub fn forward<T: Send + 'static>(handle: &AppHandle, next: Arc<dyn Fn() -> Option<T> + Send + Sync>, on: Rc<dyn Fn(T)>) {
    let again = handle.clone();
    let n = next.clone();
    handle.spawn_background(move || n(), move |item| {
        if let Some(item) = item {
            on(item);
            forward(&again, next, on);
        }
    });
}

/// UI-thread state. Cheap to clone (Rc / Signal handles).
#[derive(Clone)]
pub struct App {
    pub config: Signal<Config>,
    pub devices: Signal<Vec<DeviceState>>,
    pub tab: Signal<u8>,
    /// Persists across rebuilds; knows the real scroll range from the last paint.
    pub scroll: creamui::ScrollController,
    /// Decoded once; widgets only clone the Arc'd pixels.
    pub logo: Option<creamui::ImageData>,
    cmd: mpsc::Sender<Command>,
    handle: Rc<RefCell<Option<AppHandle>>>,
    main_win: Rc<RefCell<Option<WindowHandle>>>,
    widget_win: Rc<RefCell<Option<WindowHandle>>>,
    tray: Rc<RefCell<Option<os::Tray>>>,
}

fn open(w: &Rc<RefCell<Option<WindowHandle>>>) -> Option<WindowHandle> {
    w.borrow().clone().filter(WindowHandle::is_open)
}

impl App {
    pub fn refresh(&self) {
        let _ = self.cmd.send(Command::Refresh);
    }

    /// Single entry point for settings: mutate, persist, apply side effects.
    pub fn set_config(&self, f: impl FnOnce(&mut Config)) {
        let old = self.config.get();
        let mut new = old.clone();
        f(&mut new);
        let new = new.sanitized();
        if new == old {
            return;
        }
        if let Err(e) = new.save() {
            log::error!("saving config: {e}");
        }
        self.config.set(new.clone());

        if new.poll_secs != old.poll_secs {
            let _ = self.cmd.send(Command::SetInterval(Duration::from_secs(new.poll_secs)));
        }
        if new.autostart != old.autostart {
            if let Err(e) = os::set_autostart(new.autostart) {
                log::error!("autostart: {e}");
            }
        }
        if new.widget.visible != old.widget.visible {
            if new.widget.visible {
                self.open_widget();
            } else if let Some(w) = open(&self.widget_win) {
                w.close();
            }
        }
        if let Some(w) = open(&self.widget_win) {
            if new.widget.always_on_top != old.widget.always_on_top {
                w.set_always_on_top(new.widget.always_on_top);
            }
            if new.widget.hidden != old.widget.hidden {
                self.fit_widget();
            }
        }
        if new.lang != old.lang {
            if let Some(tray) = self.tray.borrow().as_ref() {
                tray.set_lang(new.lang);
            }
        }
        if new.dark != old.dark {
            let theme = ui::Palette::get(new.dark).theme(new.dark);
            for w in [&self.main_win, &self.widget_win] {
                if let Some(w) = open(w) {
                    w.set_theme(theme);
                }
            }
        }
    }

    pub fn show_main(&self) {
        if let Some(w) = open(&self.main_win) {
            w.show();
            return;
        }
        let Some(handle) = self.handle.borrow().clone() else { return };
        let cfg = self.config.get();
        let ready = self.main_win.clone();
        let app = self.clone();
        handle.append_window(
            WindowOptions {
                title: "lowbattery".into(),
                width: 460,
                height: 640,
                theme: ui::Palette::get(cfg.dark).theme(cfg.dark),
                // Closing hides to the tray; the process lives on.
                close_behavior: CloseBehavior::Hide,
                ..Default::default()
            },
            ui::Palette::get(cfg.dark).bg,
            move |w| {
                os::set_window_icons();
                *ready.borrow_mut() = Some(w)
            },
            move |size| ui::main_window(&app, size),
        );
    }

    pub fn open_widget(&self) {
        if open(&self.widget_win).is_some() {
            return;
        }
        let Some(handle) = self.handle.borrow().clone() else { return };
        let cfg = self.config.get();
        let ready = self.widget_win.clone();
        let app = self.clone();
        let on_top = cfg.widget.always_on_top;
        handle.append_window(
            WindowOptions {
                title: ui::WIDGET_TITLE.into(),
                width: ui::WIDGET_W,
                height: ui::widget_height(ui::widget_rows(self).len()),
                position: cfg.widget.position,
                resizable: false,
                decorations: false,
                transparent: true,
                theme: ui::Palette::get(cfg.dark).theme(cfg.dark),
                ..Default::default()
            },
            Color::rgba(0, 0, 0, 0),
            move |w| {
                w.set_always_on_top(on_top);
                os::set_window_icons();
                if cfg.widget.position.is_none() {
                    if let Some((mw, _)) = w.monitor_size() {
                        w.set_position(mw - ui::WIDGET_W as i32 - 24, 24);
                    }
                }
                *ready.borrow_mut() = Some(w);
            },
            move |size| ui::widget(&app, size),
        );
    }

    fn fit_widget(&self) {
        if let Some(w) = open(&self.widget_win) {
            w.resize(ui::WIDGET_W, ui::widget_height(ui::widget_rows(self).len()));
        }
    }

    pub fn reset_widget_position(&self) {
        self.set_config(|c| c.widget.position = None);
        if let Some(w) = open(&self.widget_win) {
            if let Some((mw, _)) = w.monitor_size() {
                w.set_position(mw - ui::WIDGET_W as i32 - 24, 24);
            }
        }
    }

    /// New readings from the monitor thread.
    fn on_snapshot(&self, states: Vec<DeviceState>, alerts: &mut LowBatteryAlerts) {
        let cfg = self.config.get();
        let rows_before = ui::widget_rows(self).len();
        if cfg.notify {
            for s in alerts.check(&states, cfg.low_threshold) {
                let level = s.status.level.unwrap_or(0);
                let (title, body) = match cfg.lang {
                    Lang::En => (format!("{}: low battery", s.device.name), format!("{level}% left. Charge it soon.")),
                    Lang::Es => (format!("{}: batería baja", s.device.name), format!("Queda {level}%. Conviene cargarlo pronto.")),
                };
                os::notify(&title, &body);
            }
        } else {
            alerts.check(&states, cfg.low_threshold); // keep state so enabling later doesn't replay
        }
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.update(&states, cfg.low_threshold);
        }
        self.devices.set(states);
        if ui::widget_rows(self).len() != rows_before {
            self.fit_widget();
        }
        // Persist a moved widget. shortcut: sampled each poll, not on drag end (no move event exposed).
        if let Some(pos) = open(&self.widget_win).and_then(|w| w.position()) {
            if cfg.widget.position != Some(pos) {
                self.set_config(|c| c.widget.position = Some(pos));
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo");
    let minimized = args.iter().any(|a| a == "--minimized") || os::launched_at_startup();
    logfile::init(args.iter().any(|a| a == "--debug"));
    log::info!("lowbattery {} start, args {:?}", env!("CARGO_PKG_VERSION"), &args[1..]);

    if !os::single_instance() {
        log::warn!("already running; exiting");
        return;
    }

    let config = Config::load();
    let interval = Duration::from_secs(config.poll_secs);
    let widget_visible = config.widget.visible;
    let provider_list = if demo { vec![Box::new(providers::demo::Demo::default()) as _] } else { providers::all() };
    let (tx, rx) = mpsc::channel();
    log::info!(
        "config {}: {:?}; providers: {:?}",
        Config::path().display(),
        config,
        provider_list.iter().map(|p| p.name()).collect::<Vec<_>>()
    );
    let monitor = Monitor::new(provider_list);

    let app = App {
        config: Signal::new(config),
        devices: Signal::new(Vec::new()),
        tab: Signal::new(0),
        scroll: creamui::ScrollController::default(),
        logo: creamui::ImageData::from_bytes(include_bytes!("../../../assets/logo-96.png"))
            .map_err(|e| log::error!("logo: {e:?}"))
            .ok(),
        cmd: tx,
        handle: Rc::default(),
        main_win: Rc::default(),
        widget_win: Rc::default(),
        tray: Rc::default(),
    };

    AppBuilder::new()
        .keep_running()
        .on_started(move |handle: AppHandle| {
            *app.handle.borrow_mut() = Some(handle.clone());

            let (snap_tx, snap_rx) = mpsc::channel::<Vec<DeviceState>>();
            std::thread::Builder::new()
                .name("monitor".into())
                .spawn(move || {
                    log::info!("diagnostics: {}", battery_core::diag::diagnostics());
                    let mut unsupported = battery_core::diag::UnsupportedReporter::default();
                    monitor.run(interval, rx, |states| {
                        unsupported.check(&states);
                        let _ = snap_tx.send(states);
                    })
                })
                .expect("spawn monitor thread");
            let snap_rx = std::sync::Mutex::new(snap_rx);
            let ui_app = app.clone();
            let alerts = RefCell::new(LowBatteryAlerts::default());
            forward(
                &handle,
                Arc::new(move || snap_rx.lock().ok()?.recv().ok()),
                Rc::new(move |states| ui_app.on_snapshot(states, &mut alerts.borrow_mut())),
            );

            match os::Tray::new(app.config.get().lang) {
                Ok(tray) => {
                    let ui_app = app.clone();
                    let exit = handle.clone();
                    tray.listen(&handle, move |action| match action {
                        os::TrayAction::Open => ui_app.show_main(),
                        os::TrayAction::Refresh => ui_app.refresh(),
                        os::TrayAction::ToggleWidget => ui_app.set_config(|c| c.widget.visible = !c.widget.visible),
                        os::TrayAction::OpenLog => os::open_file(&logfile::path()),
                        os::TrayAction::Quit => {
                            log::info!("quit from tray");
                            exit.exit()
                        }
                    });
                    *app.tray.borrow_mut() = Some(tray);
                }
                Err(e) => log::error!("tray: {e}"),
            }

            // Without a tray there is no way back to a hidden window, so always show it then.
            if !minimized || app.tray.borrow().is_none() {
                app.show_main();
            }
            if widget_visible {
                app.open_widget();
            }
        })
        .run();
}
