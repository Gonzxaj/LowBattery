//! Device-kind glyphs for the widget, drawn in code (creamui's built-in
//! symbols have no mouse / headset / controller). Monochrome alpha masks:
//! the Icon widget tints them with the row's color.

use battery_core::DeviceKind;
use creamui::core::RgbaImage;
use creamui::{Icon, IconImage, IconSource};

/// Raster side; drawn at half size so HiDPI stays sharp.
const N: usize = 28;
const STROKE: f32 = 2.0;

/// Signed distance to a rounded rectangle centered at (cx, cy).
fn round_rect(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let (qx, qy) = ((x - cx).abs() - hw + r, (y - cy).abs() - hh + r);
    qx.max(qy).min(0.0) + (qx.max(0.0).hypot(qy.max(0.0))) - r
}

fn segment(x: f32, y: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    let (px, py, dx, dy) = (x - ax, y - ay, bx - ax, by - ay);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - dx * t).hypot(py - dy * t)
}

/// Coverage of the shape's outline at distance `d`, and of its fill.
fn line(d: f32) -> f32 {
    (STROKE / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0)
}
fn fill(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Alpha (0..=1) of the glyph at pixel center (x, y) on the 28-unit grid.
fn alpha(kind: DeviceKind, x: f32, y: f32) -> f32 {
    match kind {
        DeviceKind::Mouse => {
            let body = line(round_rect(x, y, 14.0, 14.0, 7.0, 10.5, 7.0));
            let wheel = line(segment(x, y, (14.0, 7.5), (14.0, 11.0)));
            body.max(wheel)
        }
        DeviceKind::Keyboard => {
            let case = line(round_rect(x, y, 14.0, 14.0, 12.0, 8.0, 2.5));
            let keys = [10.5f32, 14.5]
                .iter()
                .flat_map(|&ky| [7.5f32, 11.5, 15.5, 19.5].map(|kx| fill(round_rect(x, y, kx, ky, 1.2, 1.2, 0.4))))
                .fold(0.0, f32::max);
            let space = fill(round_rect(x, y, 14.0, 18.5, 5.0, 1.0, 0.5));
            case.max(keys).max(space)
        }
        DeviceKind::Headset => {
            // Upper half of a ring for the band, two filled cups.
            let band = if y <= 16.0 { line((x - 14.0).hypot(y - 16.0) - 9.5) } else { 0.0 };
            let cups = fill(round_rect(x, y, 5.5, 18.5, 2.5, 4.5, 1.5)).max(fill(round_rect(x, y, 22.5, 18.5, 2.5, 4.5, 1.5)));
            band.max(cups)
        }
        DeviceKind::Controller => {
            let body = line(round_rect(x, y, 14.0, 14.5, 12.0, 7.0, 6.0));
            let dpad = line(segment(x, y, (6.5, 14.5), (10.5, 14.5))).max(line(segment(x, y, (8.5, 12.5), (8.5, 16.5))));
            let buttons = fill((x - 19.0).hypot(y - 12.5) - 1.5).max(fill((x - 21.5).hypot(y - 16.0) - 1.5));
            body.max(dpad).max(buttons)
        }
        DeviceKind::Other => {
            // A plain rounded tag with a dot: "some device".
            line(round_rect(x, y, 14.0, 14.0, 9.0, 9.0, 3.0)).max(fill((x - 14.0).hypot(y - 14.0) - 2.5))
        }
    }
}

/// Premultiplied white RGBA mask, N x N.
fn pixels(kind: DeviceKind) -> Vec<u8> {
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let a = (alpha(kind, x as f32 + 0.5, y as f32 + 0.5) * 255.0).round() as u8;
            px.extend_from_slice(&[a, a, a, a]);
        }
    }
    px
}

thread_local! {
    // One image per kind for the whole run: a fresh RgbaImage gets a fresh id
    // and would be re-uploaded on every rebuild.
    static CACHE: std::cell::RefCell<Vec<(DeviceKind, RgbaImage)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn device_icon(kind: DeviceKind, color: creamui::Color, size: f32) -> Icon {
    let image = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, img)) = c.iter().find(|(k, _)| *k == kind) {
            return img.clone();
        }
        let img = RgbaImage::new(N as u32, N as u32, pixels(kind)).expect("N*N*4 bytes");
        c.push((kind, img.clone()));
        img
    });
    Icon::new(IconSource::Image(IconImage { image, monochrome: true }), color).size(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph draws something, stays inside the grid with a margin, and they differ.
    #[test]
    fn glyphs_are_distinct_and_inside() {
        let kinds = [DeviceKind::Mouse, DeviceKind::Keyboard, DeviceKind::Headset, DeviceKind::Controller, DeviceKind::Other];
        let all: Vec<Vec<u8>> = kinds.iter().map(|&k| pixels(k)).collect();
        for (k, px) in kinds.iter().zip(&all) {
            assert!(px.iter().step_by(4).filter(|&&a| a > 128).count() > 20, "{k:?} is empty");
            for i in 0..N {
                for edge in [(i, 0), (0, i)] {
                    assert_eq!(px[(edge.1 * N + edge.0) * 4 + 3], 0, "{k:?} touches the edge");
                }
            }
        }
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
    }
}
