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

/// Coverage of a shape's fill at distance `d` (outlines: see `alpha`).
fn fill(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// Alpha (0..=1) of the glyph at pixel center (x, y) on the 28-unit grid,
/// outlines `stroke` units wide.
fn alpha(kind: DeviceKind, x: f32, y: f32, stroke: f32) -> f32 {
    let line = |d: f32| (stroke / 2.0 + 0.5 - d.abs()).clamp(0.0, 1.0);
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
            let a = (alpha(kind, x as f32 + 0.5, y as f32 + 0.5, STROKE) * 255.0).round() as u8;
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

/// Seven-segment masks for 0-9; bits a..g = 1..64 (a top, then clockwise, g middle).
const DIGITS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];

/// 32x32 straight RGBA tray icon: the device glyph on top in `color`, the
/// level below as seven-segment digits ("--" when unknown). Drawn big because
/// panels shrink it to 16-24 px.
pub fn tray_icon(kind: DeviceKind, level: Option<u8>, color: [u8; 4]) -> Vec<u8> {
    const S: usize = 32;
    const G: f32 = 17.0; // glyph box side, top-centered
    const K: f32 = N as f32 / G; // pixel -> glyph grid units
    // Digit centerline box, rounded-cap strokes.
    const W: f32 = 6.0;
    const H: f32 = 11.5;
    const TOP: f32 = 18.5;
    const PITCH: f32 = 10.0;
    const DIGIT_STROKE: f32 = 2.4;
    let digits = [0xEC, 0xEB, 0xE6, 0xFF];
    let masks: Vec<u8> = match level {
        Some(l) => l.min(100).to_string().bytes().map(|b| DIGITS[(b - b'0') as usize]).collect(),
        None => vec![0x40, 0x40],
    };
    let left = (S as f32 - (masks.len() - 1) as f32 * PITCH - W) / 2.0;
    let mut px = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let glyph = alpha(kind, (fx - (S as f32 - G) / 2.0) * K, fy * K, 2.2 * K);
            let mut digit = 0f32;
            for (i, m) in masks.iter().enumerate() {
                let (x0, y0) = (left + i as f32 * PITCH, TOP);
                let (x1, ym, y1) = (x0 + W, y0 + H / 2.0, y0 + H);
                let segs = [
                    ((x0, y0), (x1, y0)),
                    ((x1, y0), (x1, ym)),
                    ((x1, ym), (x1, y1)),
                    ((x0, y1), (x1, y1)),
                    ((x0, ym), (x0, y1)),
                    ((x0, y0), (x0, ym)),
                    ((x0, ym), (x1, ym)),
                ];
                for (bit, (a, b)) in segs.into_iter().enumerate() {
                    if m & (1 << bit) != 0 {
                        digit = digit.max((DIGIT_STROKE / 2.0 + 0.5 - segment(fx, fy, a, b)).clamp(0.0, 1.0));
                    }
                }
            }
            let (c, a) = if digit >= glyph { (digits, digit) } else { (color, glyph) };
            if a > 0.0 {
                px[(y * S + x) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], (a * 255.0).round() as u8]);
            }
        }
    }
    px
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

    #[test]
    fn tray_icon_draws_glyph_and_digits() {
        let ink = |px: &[u8], rows: std::ops::Range<usize>| rows.flat_map(|y| (0..32).map(move |x| (y, x))).filter(|(y, x)| px[(y * 32 + x) * 4 + 3] > 128).count();
        let full = tray_icon(DeviceKind::Mouse, Some(100), [255, 0, 0, 255]);
        let low = tray_icon(DeviceKind::Mouse, Some(7), [255, 0, 0, 255]);
        let unknown = tray_icon(DeviceKind::Mouse, None, [255, 0, 0, 255]);
        assert!(ink(&full, 0..17) > 20, "glyph on top");
        assert!(ink(&full, 18..32) > ink(&low, 18..32), "three digits ink more than one");
        assert!(ink(&low, 18..32) > ink(&unknown, 18..32), "7 inks more than --");
        assert!(ink(&unknown, 18..32) > 0);
        // Edges stay clear so panels don't clip anything.
        for i in 0..32 {
            assert_eq!(full[(i * 32) * 4 + 3], 0);
            assert_eq!(full[(i * 32 + 31) * 4 + 3], 0);
        }
    }
}
