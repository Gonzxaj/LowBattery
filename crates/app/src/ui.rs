//! Views. Pure functions of (state, palette) -> widget tree; all mutations go
//! through `App` so config persistence and side effects stay in one place.

use crate::App;
use battery_core::config::{Config, Lang};
use battery_core::monitor::DeviceState;
use battery_core::{Charge, DeviceKind, Link};
use creamui::core::layout::{Dimension, Size as TSize, Style as Layout};
use creamui::layout::{Align, Flex, Justify};
use creamui::{BoxedWidget, Button, ButtonSize, ButtonState, ButtonVariant, CUIWindowDragArea, Color, ScrollView, Size, Slider, Styled, Switch, Text, Theme};

const HEADER_H: f32 = 52.0;

/// Fixed width, grows into the remaining height of a column (and may shrink below content).
fn fill_rest(w: f32) -> Layout {
    Layout {
        size: TSize { width: Dimension::Length(w), height: Dimension::Auto },
        min_size: TSize { width: Dimension::Auto, height: Dimension::Length(0.0) },
        flex_grow: 1.0,
        flex_shrink: 1.0,
        ..Default::default()
    }
}
use std::time::SystemTime;

#[cfg(test)]
mod tests {
    #[test]
    fn ellipsize_counts_chars() {
        assert_eq!(super::ellipsize("Xbox Wireless Controller", 20), "Xbox Wireless Contr…");
        assert_eq!(super::ellipsize("ñandú", 20), "ñandú");
    }

    #[test]
    fn monograms() {
        assert_eq!(super::monogram("PRO X 2"), "PX");
        assert_eq!(super::monogram("Xbox Wireless Controller"), "XW");
        assert_eq!(super::monogram("K380"), "K");
        assert_eq!(super::monogram("— (x)"), "");
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub card: Color,
    pub line: Color,
    pub text: Color,
    pub muted: Color,
    pub ok: Color,
    pub mid: Color,
    pub low: Color,
    pub accent: Color,
}

impl Palette {
    pub fn get(dark: bool) -> Self {
        if dark {
            Self {
                bg: Color::rgb(0x0E, 0x0F, 0x11),
                card: Color::rgb(0x17, 0x18, 0x1B),
                line: Color::rgb(0x26, 0x28, 0x2C),
                text: Color::rgb(0xEC, 0xEB, 0xE6),
                muted: Color::rgb(0x8A, 0x8C, 0x91),
                ok: Color::rgb(0x5C, 0xD6, 0x9A),
                mid: Color::rgb(0xF2, 0xB5, 0x4A),
                low: Color::rgb(0xFF, 0x63, 0x5C),
                accent: Color::rgb(0xC8, 0xF0, 0x4C),
            }
        } else {
            Self {
                bg: Color::rgb(0xF3, 0xF2, 0xEE),
                card: Color::rgb(0xFF, 0xFF, 0xFF),
                line: Color::rgb(0xE2, 0xE0, 0xD9),
                text: Color::rgb(0x18, 0x19, 0x1C),
                muted: Color::rgb(0x6B, 0x6D, 0x72),
                ok: Color::rgb(0x1F, 0x9D, 0x62),
                mid: Color::rgb(0xC7, 0x84, 0x10),
                low: Color::rgb(0xD9, 0x3A, 0x33),
                accent: Color::rgb(0x4F, 0x6B, 0x00),
            }
        }
    }

    /// CreamUI theme for the stock controls (switches, sliders, buttons).
    pub fn theme(&self, dark: bool) -> Theme {
        let mut t = if dark { Theme::dark() } else { Theme::light() };
        t.colors.surface = self.bg;
        t.colors.surface_elevated = self.card;
        t.colors.border = self.line;
        t.colors.text_primary = self.text;
        t.colors.text_secondary = self.muted;
        t.colors.accent = self.accent;
        t.colors.selection_text = self.bg; // label on accent-filled buttons
        t.colors.accent_hover = self.accent.mix(self.text, 0.15);
        t.colors.accent_pressed = self.accent.mix(self.bg, 0.2);
        t
    }

    fn level(&self, level: Option<u8>, threshold: u8) -> Color {
        match level {
            None => self.muted,
            Some(l) if l <= threshold => self.low,
            Some(l) if l <= 50 => self.mid,
            Some(_) => self.ok,
        }
    }
}

fn text(s: impl Into<String>, size: f32, color: Color) -> BoxedWidget {
    Box::new(Text::new(s).font_size(size).color(color))
}

fn spacer() -> BoxedWidget {
    Box::new(Flex::row().grow(1.0))
}

/// Thin horizontal bar; `level` None draws an empty track.
fn bar(width: f32, height: f32, level: Option<u8>, fill: Color, track: Color) -> BoxedWidget {
    let w = width * level.unwrap_or(0) as f32 / 100.0;
    Box::new(
        Flex::row()
            .size(width, height)
            .background(track)
            .corner_radius(height / 2.0)
            .child(Box::new(Flex::row().size(w.max(0.0), height).background(fill).corner_radius(height / 2.0))),
    )
}

fn kind_label(l: Lang, k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Keyboard => l.t("Keyboard", "Teclado"),
        DeviceKind::Headset => l.t("Headset", "Auriculares"),
        DeviceKind::Controller => l.t("Controller", "Control"),
        DeviceKind::Other => l.t("Peripheral", "Periférico"),
    }
}

fn ago(l: Lang, t: Option<SystemTime>) -> String {
    let Some(t) = t else { return l.t("no readings", "sin lecturas").into() };
    let s = SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match (s, l) {
        (0..=59, _) => l.t("just read", "leído recién").into(),
        (60..=3599, Lang::En) => format!("read {} min ago", s / 60),
        (60..=3599, Lang::Es) => format!("leído hace {} min", s / 60),
        (_, Lang::En) => format!("read {} h ago", s / 3600),
        (_, Lang::Es) => format!("leído hace {} h", s / 3600),
    }
}

/// Shown level: live if connected, otherwise last known (dimmed by caller).
fn shown_level(s: &DeviceState) -> Option<u8> {
    s.status.level.or(s.last_level)
}

/// The widget row is one line high; long names would wrap and overflow it.
fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

fn pct(level: Option<u8>) -> String {
    level.map_or("—".into(), |l| format!("{l}%"))
}

// ---------------------------------------------------------------- main window

pub fn main_window(app: &App, view: Size) -> BoxedWidget {
    let cfg = app.config.get();
    let p = Palette::get(cfg.dark);
    let tab = app.tab.get();
    let width = view.width;
    let l = cfg.lang;

    let tabs = Flex::row()
        .gap(4.0)
        .padding(4.0)
        .background(p.card)
        .corner_radius(10.0)
        .child(tab_button(app, &p, 0, l.t("Devices", "Dispositivos"), tab == 0))
        .child(tab_button(app, &p, 1, l.t("Settings", "Ajustes"), tab == 1));

    let mut header = Flex::row().full_width().height(HEADER_H).shrink(0.0).align(Align::Center).gap(12.0);
    if let Some(logo) = &app.logo {
        header = header.child(Box::new(creamui::Image::new(logo.clone()).fit(creamui::ImageFit::Contain).width(40.0).height(40.0)));
    }
    let header = header
        .child(Box::new(
            Flex::column()
                .gap(2.0)
                .child(Box::new(Text::new("lowbattery").font_size(22.0).bold(true).color(p.text)))
                .child(text(summary(l, &app.devices.get(), cfg.low_threshold), 12.0, p.muted)),
        ))
        .child(spacer())
        .child(Box::new(tabs));

    let body: BoxedWidget = if tab == 0 { devices(app, &p, &cfg, width - 40.0) } else { settings(app, &p, &cfg, width - 40.0) };

    let mut root = Flex::column()
        .size(view.width, view.height)
        .padding(20.0)
        .gap(18.0)
        .background(p.bg)
        .child(Box::new(header))
        // The list takes whatever height is left; a hand-computed height drifted
        // from the real header size and broke clipping and the scroll range.
        .child(Box::new(ScrollView::controlled(fill_rest(width - 40.0), app.scroll.clone()).child(body)));
    if tab == 0 {
        // Pinned below the list so scrolling ends at the last card.
        root = root.child(devices_footer(app, &p, l));
    }
    Box::new(root)
}

fn devices_footer(app: &App, p: &Palette, l: Lang) -> BoxedWidget {
    let refresh = app.clone();
    Box::new(
        Flex::row()
            .full_width()
            .shrink(0.0)
            .height(36.0)
            .gap(8.0)
            .align(Align::Center)
            .child(text(l.t("Missing one? It's noted in the log.", "¿Falta alguno? Queda en el registro."), 12.0, p.muted))
            .child(spacer())
            .child(Box::new(Button::styled(ButtonVariant::Secondary, ButtonSize::Sm, l.t("View log", "Ver registro"), ButtonState::Normal, || {
                crate::os::open_file(&crate::logfile::path())
            })))
            .child(Box::new(Button::styled(ButtonVariant::Primary, ButtonSize::Sm, l.t("Refresh", "Actualizar"), ButtonState::Normal, move || {
                refresh.refresh()
            }))),
    )
}

fn summary(lang: Lang, states: &[DeviceState], threshold: u8) -> String {
    let connected = states.iter().filter(|s| s.status.link == Link::Connected).count();
    let low = states.iter().filter(|s| s.status.level.is_some_and(|l| l <= threshold)).count();
    match (lang, connected, low) {
        (_, 0, _) => lang.t("Looking for peripherals…", "Buscando periféricos…").into(),
        (Lang::En, n, 0) => format!("{n} connected · all good"),
        (Lang::En, n, l) => format!("{n} connected · {l} low on battery"),
        (Lang::Es, n, 0) => format!("{n} {} · todo en orden", if n == 1 { "conectado" } else { "conectados" }),
        (Lang::Es, n, l) => format!("{n} {} · {l} con batería baja", if n == 1 { "conectado" } else { "conectados" }),
    }
}

fn tab_button(app: &App, _p: &Palette, idx: u8, label: &str, active: bool) -> BoxedWidget {
    let (tab, scroll) = (app.tab.clone(), app.scroll.clone());
    let variant = if active { ButtonVariant::Primary } else { ButtonVariant::Tertiary };
    Box::new(Button::styled(variant, ButtonSize::Sm, label, ButtonState::Normal, move || {
        tab.set(idx);
        scroll.set(0.0);
    }))
}

fn devices(app: &App, p: &Palette, cfg: &Config, width: f32) -> BoxedWidget {
    let states = app.devices.get();
    let l = cfg.lang;
    // Inside the scroll view nothing may shrink (creamui containers shrink by
    // default): squeezed cards overflow their boxes and the scroll range collapses.
    let mut col = Flex::column().gap(10.0).full_width().shrink(0.0);
    if states.is_empty() {
        col = col.child(Box::new(
            Flex::column()
                .full_width()
                .shrink(0.0)
                .padding(24.0)
                .gap(6.0)
                .background(p.card)
                .corner_radius(14.0)
                .child(text(l.t("No peripherals detected yet", "Sin periféricos detectados todavía"), 15.0, p.text))
                .child(text(
                    l.t(
                        "Bluetooth, Logitech, SteelSeries and Razer devices are detected automatically.",
                        "Los dispositivos Bluetooth, Logitech, SteelSeries y Razer se detectan solos.",
                    ),
                    12.0,
                    p.muted,
                )),
        ));
    }
    // Battery-less generic cards go last.
    let generic = |s: &&DeviceState| s.device.provider == battery_core::diag::GENERIC;
    for s in states.iter().filter(|s| !generic(s)).chain(states.iter().filter(generic)) {
        col = col.child(device_card(app, p, cfg, s, width));
    }
    Box::new(col)
}

/// Battery as 10 cells, like a cell-meter, instead of a generic progress bar.
fn cells(width: f32, level: Option<u8>, fill: Color, track: Color) -> BoxedWidget {
    const N: usize = 10;
    const GAP: f32 = 4.0;
    let w = ((width - GAP * (N - 1) as f32) / N as f32).max(1.0);
    // Round up so 1..=10 % still lights the first cell: a sliver of hope is information.
    let lit = level.map_or(0, |l| (l as usize).div_ceil(10).min(N));
    let mut row = Flex::row().gap(GAP);
    for i in 0..N {
        row = row.child(Box::new(Flex::row().size(w, 8.0).background(if i < lit { fill } else { track }).corner_radius(2.5)));
    }
    Box::new(row)
}

/// Two-letter monogram from the device name ("PRO X 2" -> "PX").
fn monogram(name: &str) -> String {
    let mut words = name.split_whitespace().filter(|w| w.chars().next().is_some_and(char::is_alphanumeric));
    let first = words.next().and_then(|w| w.chars().next());
    let second = words.next().and_then(|w| w.chars().next());
    [first, second].into_iter().flatten().flat_map(char::to_uppercase).collect()
}

fn dot(color: Color) -> BoxedWidget {
    Box::new(Flex::row().size(7.0, 7.0).background(color).corner_radius(3.5))
}

fn widget_pill(app: &App, p: &Palette, l: Lang, id: String, on: bool) -> BoxedWidget {
    let toggle = app.clone();
    let (bg, fg, border) = if on { (p.accent.mix(p.card, 0.82), p.accent, p.accent.mix(p.card, 0.55)) } else { (p.card, p.muted, p.line) };
    let label = if on { l.t("In widget", "En widget") } else { l.t("Hidden", "Oculto") };
    Box::new(
        creamui::RawButton::new(
            creamui::Style::new().padding("4px 10px").background(bg).border(border, 1.0).corner_radius(999.0),
            move || {
                let id = id.clone();
                toggle.set_config(move |c| {
                    if !c.widget.hidden.remove(&id) {
                        c.widget.hidden.insert(id);
                    }
                })
            },
        )
        .child(text(label, 11.0, fg)),
    )
}

fn device_card(app: &App, p: &Palette, cfg: &Config, s: &DeviceState, width: f32) -> BoxedWidget {
    let level = shown_level(s);
    let live = s.status.link == Link::Connected && s.status.level.is_some();
    let low = live && level.is_some_and(|l| l <= cfg.low_threshold);
    let charging = live && matches!(s.status.charge, Charge::Charging | Charge::Full);
    let tone = if !live { p.muted } else if charging { p.accent } else { p.level(level, cfg.low_threshold) };
    let card_bg = if low { p.card.mix(p.low, 0.06) } else { p.card };
    let inner = width - 32.0;
    let l = cfg.lang;
    // Cheap receivers with no battery channel: the card only says they're there.
    let generic = s.device.provider == battery_core::diag::GENERIC;
    let bright = live || (generic && s.status.link == Link::Connected);

    let badge = Flex::row()
        .size(40.0, 40.0)
        .align(Align::Center)
        .justify(Justify::Center)
        .background(tone.mix(card_bg, 0.84))
        .corner_radius(12.0)
        .child(Box::new(Text::new(monogram(&s.device.name)).font_size(13.0).bold(true).color(tone)));

    let title = Flex::column()
        .grow(1.0)
        .gap(3.0)
        .child(Box::new(Text::new(s.device.name.clone()).font_size(15.0).bold(true).color(if bright { p.text } else { p.muted })))
        .child(text(format!("{} · {}", s.device.vendor, kind_label(l, s.device.kind)), 12.0, p.muted));

    let figure: BoxedWidget = match level {
        Some(l) => Box::new(
            Flex::row()
                .align(Align::End)
                .gap(2.0)
                .child(Box::new(Text::new(l.to_string()).font_size(30.0).bold(true).color(tone)))
                .child(Box::new(Flex::column().padding("0 0 5px 0").child(text("%", 13.0, p.muted)))),
        ),
        None if generic => Box::new(Flex::row()),
        None => text("—", 26.0, p.muted),
    };

    let (state_color, state) = match (s.status.link, charging, s.status.charge) {
        (Link::Connected, true, Charge::Full) => (p.accent, l.t("Fully charged", "Carga completa")),
        (Link::Connected, true, _) => (p.accent, l.t("Charging", "Cargando")),
        (Link::Connected, _, _) => (p.ok, l.t("Connected", "Conectado")),
        (Link::Asleep, _, _) => (p.mid, l.t("Asleep", "En reposo")),
        (Link::Disconnected, _, _) => (p.muted, l.t("Disconnected", "Desconectado")),
    };
    let when = match (live, s.last_level) {
        _ if generic => String::new(),
        (false, Some(v)) => format!("· {} {v}% · {}", l.t("last", "último"), ago(l, s.last_read)),
        _ => format!("· {}", ago(l, s.last_read)),
    };

    // Fixed row heights: text in flex rows is otherwise measured at min-content
    // width (wrapped per word) and inflates the card with empty space.
    let mut footer = Flex::row()
        .full_width()
        .height(26.0)
        .align(Align::Center)
        .gap(7.0)
        .child(dot(state_color))
        .child(text(state, 12.0, if bright { p.text } else { p.muted }))
        .child(text(when, 12.0, p.muted))
        .child(spacer());
    if !generic {
        footer = footer.child(widget_pill(app, p, l, s.device.id.clone(), !cfg.widget.hidden.contains(&s.device.id)));
    }
    let (middle, middle_h): (BoxedWidget, f32) = if generic {
        let note = l.t(
            "Generic receiver: it reports no battery, only that it's connected.",
            "Receptor genérico: no informa la batería, solo que está conectado.",
        );
        (text(note, 12.0, p.muted), 14.0)
    } else {
        (cells(inner, level, tone, p.line), 8.0)
    };

    Box::new(
        Flex::column()
            .full_width()
            .shrink(0.0)
            // shortcut: explicit height (sum of parts) because creamui 0.3 over-measures
            // this column; drop it once its layout reports the real content height.
            .height(16.0 + 42.0 + 14.0 + middle_h + 14.0 + 26.0 + 16.0)
            .padding(16.0)
            .gap(14.0)
            .background(card_bg)
            .border(if low { p.low.mix(card_bg, 0.45) } else { p.line }, 1.0)
            .corner_radius(16.0)
            .child(Box::new(
                Flex::row().full_width().height(42.0).align(Align::Center).gap(12.0).child(Box::new(badge)).child(Box::new(title)).child(figure),
            ))
            .child(middle)
            .child(Box::new(footer)),
    )
}

// ---------------------------------------------------------------- settings

const INTERVALS: [u64; 6] = [15, 30, 60, 120, 300, 600];

fn row(p: &Palette, title: &str, hint: String, control: BoxedWidget) -> BoxedWidget {
    Box::new(
        Flex::row()
            .full_width()
            .shrink(0.0)
            .height(40.0)
            .align(Align::Center)
            .gap(12.0)
            .child(Box::new(
                Flex::column()
                    .grow(1.0)
                    .gap(2.0)
                    .child(text(title, 14.0, p.text))
                    .child(text(hint, 12.0, p.muted)),
            ))
            .child(control),
    )
}

fn group(p: &Palette, title: &str, rows: Vec<BoxedWidget>) -> BoxedWidget {
    Box::new(
        Flex::column()
            .full_width()
            .shrink(0.0)
            .gap(10.0)
            .child(text(title.to_uppercase(), 11.0, p.muted))
            .child(Box::new(
                Flex::column()
                    .full_width()
                    .shrink(0.0)
                    .padding(18.0)
                    .gap(16.0)
                    .background(p.card)
                    .border(p.line, 1.0)
                    .corner_radius(14.0)
                    .with_children(rows),
            )),
    )
}

fn switch(app: &App, on: bool, f: impl Fn(&mut Config) + 'static) -> BoxedWidget {
    let app = app.clone();
    let f = std::rc::Rc::new(f);
    Box::new(Switch::new(on, move || {
        let f = f.clone();
        app.set_config(move |c| f(c))
    }))
}

/// Two-button segmented control: the active language is the filled one.
fn lang_picker(app: &App, current: Lang) -> BoxedWidget {
    let button = |lang: Lang, label: &str| -> BoxedWidget {
        let a = app.clone();
        let variant = if lang == current { ButtonVariant::Primary } else { ButtonVariant::Tertiary };
        Box::new(Button::styled(variant, ButtonSize::Sm, label, ButtonState::Normal, move || a.set_config(move |c| c.lang = lang)))
    };
    Box::new(Flex::row().gap(4.0).child(button(Lang::En, "English")).child(button(Lang::Es, "Español")))
}

fn settings(app: &App, p: &Palette, cfg: &Config, _width: f32) -> BoxedWidget {
    let interval_pos = INTERVALS.iter().position(|&s| s >= cfg.poll_secs).unwrap_or(2);
    let a = app.clone();
    let interval = Slider::new(interval_pos as f32 / (INTERVALS.len() - 1) as f32, move |v| {
        let secs = INTERVALS[(v * (INTERVALS.len() - 1) as f32).round() as usize];
        a.set_config(move |c| c.poll_secs = secs)
    });
    let a = app.clone();
    let threshold = Slider::new((cfg.low_threshold as f32 - 5.0) / 45.0, move |v| {
        let t = (5.0 + v * 45.0).round() as u8;
        a.set_config(move |c| c.low_threshold = t)
    });
    let a = app.clone();
    let opacity = Slider::new((cfg.widget.opacity - 0.3) / 0.7, move |v| {
        let o = 0.3 + v * 0.7;
        a.set_config(move |c| c.widget.opacity = o)
    });

    let secs = cfg.poll_secs;
    let l = cfg.lang;
    let interval_hint = match (l, secs < 60) {
        (Lang::En, true) => format!("Every {secs} s"),
        (Lang::En, false) => format!("Every {} min", secs / 60),
        (Lang::Es, true) => format!("Cada {secs} s"),
        (Lang::Es, false) => format!("Cada {} min", secs / 60),
    };
    let threshold_hint = match l {
        Lang::En => format!("Warn at {}%", cfg.low_threshold),
        Lang::Es => format!("Avisar al llegar a {}%", cfg.low_threshold),
    };

    let reset = app.clone();
    Box::new(
        Flex::column()
            .full_width()
            .shrink(0.0)
            .gap(18.0)
            .child(group(
                p,
                l.t("Monitoring", "Monitoreo"),
                vec![
                    row(p, l.t("Refresh interval", "Intervalo de actualización"), interval_hint, Box::new(interval)),
                    row(p, l.t("Low battery", "Batería baja"), threshold_hint, Box::new(threshold)),
                    row(
                        p,
                        l.t("Notifications", "Notificaciones"),
                        if cfg!(windows) { l.t("Windows notification once per discharge", "Aviso de Windows una vez por descarga") } else { l.t("Desktop notification once per discharge", "Aviso del escritorio una vez por descarga") }.into(),
                        switch(app, cfg.notify, |c| c.notify = !c.notify),
                    ),
                    row(
                        p,
                        if cfg!(windows) { l.t("Start with Windows", "Iniciar con Windows") } else { l.t("Start at login", "Iniciar con la sesión") },
                        l.t("Starts minimized to the tray", "Arranca minimizado en la bandeja").into(),
                        switch(app, cfg.autostart, |c| c.autostart = !c.autostart),
                    ),
                ],
            ))
            .child(group(
                p,
                l.t("Widget and appearance", "Widget y apariencia"),
                vec![
                    row(
                        p,
                        l.t("Show widget", "Mostrar widget"),
                        l.t("Floating panel; drag it to move it", "Panel flotante; arrastralo para moverlo").into(),
                        switch(app, cfg.widget.visible, |c| c.widget.visible = !c.widget.visible),
                    ),
                    row(
                        p,
                        l.t("Always on top", "Siempre visible"),
                        if crate::os::on_top_needs_restart(cfg.widget.always_on_top) {
                            l.t("Restart the app to apply (Wayland)", "Se aplica al reiniciar la app (Wayland)")
                        } else {
                            l.t("Above other windows", "Por encima de otras ventanas")
                        }
                        .into(),
                        switch(app, cfg.widget.always_on_top, |c| c.widget.always_on_top = !c.widget.always_on_top),
                    ),
                    row(p, l.t("Opacity", "Opacidad"), format!("{:.0}%", cfg.widget.opacity * 100.0), Box::new(opacity)),
                    row(
                        p,
                        l.t("Dark theme", "Tema oscuro"),
                        l.t("Applies to the app and the widget", "Aplica a la app y al widget").into(),
                        switch(app, cfg.dark, |c| c.dark = !c.dark),
                    ),
                    row(p, l.t("Language", "Idioma"), "English / Español".into(), lang_picker(app, l)),
                    row(
                        p,
                        l.t("Widget position", "Posición del widget"),
                        l.t("Back to the default corner", "Volver a la esquina por defecto").into(),
                        Box::new(Button::new(l.t("Reset", "Restablecer"), move || reset.reset_widget_position()).corner_radius(8.0)),
                    ),
                ],
            )),
    )
}

// ---------------------------------------------------------------- widget

pub const WIDGET_W: u32 = 250;
/// Window title; os.rs finds the widget's HWND by it.
pub const WIDGET_TITLE: &str = "lowbattery widget";
const WIDGET_ROW: f32 = 30.0;
const WIDGET_PAD: f32 = 12.0;

pub fn widget_height(rows: usize) -> u32 {
    (WIDGET_PAD * 2.0 + WIDGET_ROW * rows.max(1) as f32) as u32
}

pub fn widget_rows(app: &App) -> Vec<DeviceState> {
    let cfg = app.config.get();
    app.devices
        .get()
        .into_iter()
        // Battery-less generic cards have nothing to show in the widget.
        .filter(|s| s.device.provider != battery_core::diag::GENERIC && !cfg.widget.hidden.contains(&s.device.id))
        .collect()
}

pub fn widget(app: &App, view: Size) -> BoxedWidget {
    let cfg = app.config.get();
    let p = Palette::get(cfg.dark);
    let rows = widget_rows(app);
    let a = (cfg.widget.opacity * 255.0) as u8;
    let bg = Color::rgba(p.bg.r, p.bg.g, p.bg.b, a);

    let mut col = Flex::column().gap(0.0).full_width();
    if rows.is_empty() {
        col = col.child(Box::new(
            Flex::row().height(WIDGET_ROW).align(Align::Center).child(text(cfg.lang.t("No devices", "Sin dispositivos"), 12.0, p.muted)),
        ));
    }
    for s in &rows {
        let level = shown_level(s);
        let live = s.status.link == Link::Connected && s.status.level.is_some();
        let color = if live { p.level(level, cfg.low_threshold) } else { p.muted };
        let charging = live && s.status.charge == Charge::Charging;
        col = col.child(Box::new(
            Flex::row()
                .full_width()
                .height(WIDGET_ROW)
                .gap(8.0)
                .align(Align::Center)
                .child(Box::new(crate::icons::device_icon(s.device.kind, if live { p.text } else { p.muted }, 14.0)))
                .child(Box::new(Flex::row().grow(1.0).child(text(ellipsize(&s.device.name, 16), 12.0, if live { p.text } else { p.muted }))))
                .child(bar(44.0, 4.0, level, if charging { p.accent } else { color }, p.line))
                .child(Box::new(
                    Flex::row()
                        .width(36.0)
                        .justify(Justify::End)
                        .child(Box::new(Text::new(pct(level)).font_size(12.0).bold(true).color(color))),
                )),
        ));
    }

    Box::new(
        Flex::column().size(view.width, view.height).child(Box::new(
            CUIWindowDragArea::new()
                .size(view.width, view.height)
                .padding(WIDGET_PAD)
                .background(bg)
                .corner_radius(12.0)
                .child(Box::new(col)),
        )),
    )
}
