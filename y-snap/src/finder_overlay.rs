//! Drawing for the mouse finder: the shapes, and the screen capture some of
//! them read.
//!
//! Everything here is pure pixel work on a premultiplied `0xAARRGGBB` buffer
//! (the [`y_core::overlay::Canvas`]'s), apart from [`Capturer`], the one
//! piece of GDI. The shapes were prototyped in a probe program (since removed)
//! and tuned live; the numbers below come from there.

use std::ffi::c_void;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use y_core::monitor::Monitor;
use y_core::overlay::Color;

use crate::config::Shape;

pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// Blends `color` at `alpha` over one premultiplied pixel.
fn blend(pixels: &mut [u32], index: usize, color: Color, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0) * (color.a as f32 / 255.0);
    if a <= 0.0 {
        return;
    }
    let dst = pixels[index];
    let inv = 1.0 - a;
    let channel = |shift: u32, c: u8| {
        (((c as f32) * a + ((dst >> shift) & 0xFF) as f32 * inv).round() as u32).min(255)
    };
    let alpha_out = ((255.0 * a + ((dst >> 24) & 0xFF) as f32 * inv).round() as u32).min(255);
    pixels[index] = (alpha_out << 24)
        | (channel(16, color.r) << 16)
        | (channel(8, color.g) << 8)
        | channel(0, color.b);
}

/// The pixel buffer being drawn into, and its size.
struct Surface<'a> {
    p: &'a mut [u32],
    w: i32,
    h: i32,
}

impl Surface<'_> {
    /// A disc fading from full at the centre to nothing at `radius`.
    fn soft_disc(&mut self, (cx, cy): (f32, f32), radius: f32, color: Color, alpha: f32) {
        let (x0, x1) = (
            ((cx - radius).floor() as i32).max(0),
            ((cx + radius).ceil() as i32).min(self.w - 1),
        );
        let (y0, y1) = (
            ((cy - radius).floor() as i32).max(0),
            ((cy + radius).ceil() as i32).min(self.h - 1),
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let coverage = (1.0 - d / radius).clamp(0.0, 1.0);
                blend(
                    self.p,
                    (y * self.w + x) as usize,
                    color,
                    alpha * coverage * coverage.sqrt(),
                );
            }
        }
    }

    /// An antialiased ring.
    fn ring(
        &mut self,
        (cx, cy): (f32, f32),
        radius: f32,
        thickness: f32,
        color: Color,
        alpha: f32,
    ) {
        let reach = radius + thickness;
        let (x0, x1) = (
            ((cx - reach).floor() as i32).max(0),
            ((cx + reach).ceil() as i32).min(self.w - 1),
        );
        let (y0, y1) = (
            ((cy - reach).floor() as i32).max(0),
            ((cy + reach).ceil() as i32).min(self.h - 1),
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let coverage = (thickness / 2.0 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    blend(self.p, (y * self.w + x) as usize, color, alpha * coverage);
                }
            }
        }
    }

    fn fill_rect(&mut self, rect: RECT, color: Color, alpha: f32) {
        for y in rect.top.max(0)..rect.bottom.min(self.h) {
            for x in rect.left.max(0)..rect.right.min(self.w) {
                blend(self.p, (y * self.w + x) as usize, color, alpha);
            }
        }
    }

    /// Calls `f(index, distance)` for every pixel within `reach` of `centre`.
    fn around(&self, (cx, cy): (f32, f32), reach: f32, mut f: impl FnMut(usize, f32)) {
        for y in ((cy - reach).floor() as i32).max(0)..=((cy + reach).ceil() as i32).min(self.h - 1)
        {
            for x in
                ((cx - reach).floor() as i32).max(0)..=((cx + reach).ceil() as i32).min(self.w - 1)
            {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                f((y * self.w + x) as usize, d);
            }
        }
    }
}

/// What a frame is drawn from.
pub struct Frame<'a> {
    pub shape: Shape,
    /// Base radius in physical pixels, and the monitor's scale (1.0 at 96 dpi).
    pub radius: f32,
    pub scale: f32,
    pub color: Color,
    /// 0..1, how dark the spotlight makes the screen.
    pub dim: f32,
    pub monitors: &'a [Monitor],
    /// The monitor the surface is on.
    pub bounds: RECT,
    /// The surface holds only the map panel (the `monitor` shape on every
    /// monitor), not the whole monitor.
    pub docked: bool,
    pub pointer: POINT,
    /// The screen behind a small surface, same size as it (the shapes that
    /// read it only).
    pub backdrop: &'a [u32],
    /// Smoothed brightness of what is behind the pointer, 0 (black)..1.
    pub lum: f32,
    /// Progress through the animation, 0..1, and the seconds elapsed.
    pub t: f32,
    pub secs: f32,
    /// Seconds the animation lasts in all.
    pub total: f32,
}

/// Whether the shape is drawn on a small surface that follows the pointer
/// (the others cover the pointer's whole monitor).
pub fn is_small(shape: Shape) -> bool {
    matches!(
        shape,
        Shape::Beacon | Shape::Halo | Shape::Negative | Shape::Contrast
    )
}

/// Whether the shape reads the pixels behind it.
pub fn reads_backdrop(shape: Shape) -> bool {
    matches!(shape, Shape::Negative | Shape::Contrast)
}

/// One frame of the shape into `pixels` (`w` x `h`, already cleared), with the
/// pointer at `centre` on that surface.
pub fn draw(pixels: &mut [u32], (w, h): (i32, i32), centre: (f32, f32), f: &Frame<'_>) {
    let mut s = Surface { p: pixels, w, h };
    let (r, t) = (f.radius, f.t);
    let fade = smoothstep(0.0, 0.1, t) * (1.0 - smoothstep(0.78, 1.0, t));
    // The persistent shapes ease in over 0.3 s and out over the last 0.3 s.
    let steady = smoothstep(0.0, 0.3, f.secs) * (1.0 - smoothstep(f.total - 0.3, f.total, f.secs));
    let (cx, cy) = centre;

    match f.shape {
        // Rings converging on the pointer over a soft halo.
        Shape::Beacon => {
            s.soft_disc(centre, 0.6 * r, f.color, 0.40 * fade);
            for i in 0..3 {
                let q = t * 1.5 - i as f32 * 0.2;
                if (0.0..=1.0).contains(&q) {
                    let radius = r * (1.0 - ease_out(q) * 0.85);
                    let alpha = smoothstep(0.0, 0.15, q) * (1.0 - 0.5 * q) * fade;
                    s.ring(centre, radius, 6.0 * f.scale, f.color, alpha);
                }
            }
        }
        // The screen dims; a clear circle shrinks onto the pointer.
        Shape::Spotlight => {
            let dim = f.dim * fade;
            s.p.fill(((dim * 255.0) as u32) << 24);
            let hole = r * (0.9 + 3.0 * (1.0 - ease_out(t / 0.35)));
            let soft = 0.3 * r;
            let reach = hole + soft;
            for y in ((cy - reach).floor() as i32).max(0)..=((cy + reach).ceil() as i32).min(h - 1)
            {
                for x in
                    ((cx - reach).floor() as i32).max(0)..=((cx + reach).ceil() as i32).min(w - 1)
                {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    let alpha = dim * smoothstep(hole, hole + soft, d);
                    s.p[(y * w + x) as usize] = ((alpha * 255.0) as u32) << 24;
                }
            }
        }
        // Lines across the whole monitor, with a gap and a ring at the pointer.
        Shape::Crosshair => {
            let grow = ease_out(t / 0.25);
            let extent = (grow * w.max(h) as f32) as i32;
            let gap = (0.6 * r) as i32;
            let thick = ((3.0 * f.scale) as i32).max(1);
            let alpha = 0.65 * fade;
            let (ix, iy) = (cx as i32, cy as i32);
            for y in (iy - thick / 2).max(0)..(iy - thick / 2 + thick).min(h) {
                for x in 0..w {
                    let dx = (x - ix).abs();
                    if dx >= gap && dx <= extent {
                        blend(s.p, (y * w + x) as usize, f.color, alpha);
                    }
                }
            }
            for x in (ix - thick / 2).max(0)..(ix - thick / 2 + thick).min(w) {
                for y in 0..h {
                    let dy = (y - iy).abs();
                    if dy >= gap && dy <= extent {
                        blend(s.p, (y * w + x) as usize, f.color, alpha);
                    }
                }
            }
            s.ring(centre, gap as f32, 3.0 * f.scale, f.color, 0.9 * fade);
        }
        // Which monitor: a pulsing border, and a map of all the monitors.
        Shape::Monitor => draw_monitor(&mut s, f, fade),
        // A persistent ring and glow that follow the pointer, with a thin
        // outline of the opposite tone so it reads on mixed backgrounds.
        Shape::Halo => {
            s.soft_disc(centre, 0.55 * r, f.color, 0.40 * steady);
            s.ring(centre, 0.5 * r, 4.0 * f.scale, f.color, 0.95 * steady);
            s.ring(
                centre,
                0.5 * r + 3.5 * f.scale,
                2.0 * f.scale,
                outline_for(f.lum),
                0.8 * steady,
            );
        }
        // A lens that shows the screen behind the pointer as a negative.
        Shape::Negative => {
            let radius = 0.5 * r;
            let soft = 0.12 * r;
            if f.backdrop.len() >= (w * h) as usize {
                let mut lens = Vec::new();
                s.around(centre, radius + 1.0, |index, d| {
                    let coverage = 1.0 - smoothstep(radius - soft, radius, d);
                    if coverage > 0.0 {
                        lens.push((index, coverage));
                    }
                });
                for (index, coverage) in lens {
                    let b = f.backdrop[index];
                    let inverted = Color::rgb(
                        255 - ((b >> 16) & 0xFF) as u8,
                        255 - ((b >> 8) & 0xFF) as u8,
                        255 - (b & 0xFF) as u8,
                    );
                    blend(s.p, index, inverted, coverage * steady);
                }
            }
            s.ring(
                centre,
                radius,
                2.0 * f.scale,
                outline_for(f.lum),
                0.7 * steady,
            );
        }
        // A ring painted black or white pixel by pixel, whichever contrasts
        // with the pixel behind it.
        Shape::Contrast => {
            let radius = 0.5 * r;
            let thickness = 8.0 * f.scale;
            if f.backdrop.len() >= (w * h) as usize {
                let mut band = Vec::new();
                s.around(centre, radius + thickness, |index, d| {
                    let coverage = (thickness / 2.0 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
                    if coverage > 0.0 {
                        band.push((index, coverage));
                    }
                });
                for (index, coverage) in band {
                    let colour = if luminance(f.backdrop[index]) > 0.5 {
                        Color::BLACK
                    } else {
                        Color::WHITE
                    };
                    blend(s.p, index, colour, 0.92 * coverage * steady);
                }
            }
        }
    }
}

/// How the map of the monitors is laid out: the desktop's bounding box, the
/// scale from desktop pixels to map pixels, the map's size and its padding.
#[derive(Clone, Copy)]
struct MapGeometry {
    union: RECT,
    k: f32,
    w: f32,
    h: f32,
    pad: f32,
}

fn map_geometry(monitors: &[Monitor], scale: f32) -> Option<MapGeometry> {
    let union = monitors.iter().fold(
        RECT {
            left: i32::MAX,
            top: i32::MAX,
            right: i32::MIN,
            bottom: i32::MIN,
        },
        |u, m| RECT {
            left: u.left.min(m.bounds.left),
            top: u.top.min(m.bounds.top),
            right: u.right.max(m.bounds.right),
            bottom: u.bottom.max(m.bounds.bottom),
        },
    );
    if union.right <= union.left || union.bottom <= union.top {
        return None;
    }
    let (uw, uh) = (
        (union.right - union.left) as f32,
        (union.bottom - union.top) as f32,
    );
    let k = ((380.0 * scale) / uw).min((200.0 * scale) / uh);
    Some(MapGeometry {
        union,
        k,
        w: uw * k,
        h: uh * k,
        pad: 16.0 * scale,
    })
}

/// The size of a surface that holds only the map panel (the `all-monitors`
/// layout), so that a monitor that is not the pointer's costs little.
pub fn map_panel_size(monitors: &[Monitor], scale: f32) -> (i32, i32) {
    match map_geometry(monitors, scale) {
        Some(g) => (
            (g.w + 2.0 * g.pad).ceil() as i32,
            (g.h + 2.0 * g.pad).ceil() as i32,
        ),
        None => (1, 1),
    }
}

/// Where the map panel's top left goes on a monitor: centred, a fifth of the
/// way down. Used to place the small surface so the map sits where the
/// full-monitor layout would have put it.
pub fn map_panel_origin(bounds: RECT, panel: (i32, i32), scale: f32) -> POINT {
    POINT {
        x: bounds.left + ((bounds.right - bounds.left) - panel.0) / 2,
        y: bounds.top + ((bounds.bottom - bounds.top) as f32 * 0.22 - 16.0 * scale) as i32,
    }
}

fn draw_monitor(s: &mut Surface<'_>, f: &Frame<'_>, fade: f32) {
    let (w, h) = (s.w, s.h);
    // Docked: the surface is the map panel itself, with no border around the
    // monitor.
    if !f.docked {
        draw_border(s, f, fade);
    }
    let Some(g) = map_geometry(f.monitors, f.scale) else {
        return;
    };
    let (mx, my) = if f.docked {
        (g.pad, g.pad)
    } else {
        (w as f32 / 2.0 - g.w / 2.0, h as f32 * 0.22)
    };
    draw_map(s, f, fade, &g, (mx, my));
}

fn draw_border(s: &mut Surface<'_>, f: &Frame<'_>, fade: f32) {
    let (w, h) = (s.w, s.h);
    let pulse = 0.65 + 0.35 * (f.secs * std::f32::consts::TAU * 2.0).cos();
    let b = (10.0 * f.scale) as i32;
    let alpha = fade * pulse;
    for rect in [
        RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: b,
        },
        RECT {
            left: 0,
            top: h - b,
            right: w,
            bottom: h,
        },
        RECT {
            left: 0,
            top: b,
            right: b,
            bottom: h - b,
        },
        RECT {
            left: w - b,
            top: b,
            right: w,
            bottom: h - b,
        },
    ] {
        s.fill_rect(rect, f.color, alpha);
    }
}

/// The panel with every monitor in it, the pointer's highlighted, and the
/// pointer as a dot; `(mx, my)` is where the map itself starts.
fn draw_map(s: &mut Surface<'_>, f: &Frame<'_>, fade: f32, g: &MapGeometry, (mx, my): (f32, f32)) {
    let MapGeometry {
        union,
        k,
        w: map_w,
        h: map_h,
        pad,
    } = *g;
    let panel = RECT {
        left: (mx - pad) as i32,
        top: (my - pad) as i32,
        right: (mx + map_w + pad) as i32,
        bottom: (my + map_h + pad) as i32,
    };
    s.fill_rect(panel, Color::rgb(20, 20, 20), 0.8 * fade);
    for m in f.monitors {
        let rect = RECT {
            left: (mx + (m.bounds.left - union.left) as f32 * k) as i32 + 2,
            top: (my + (m.bounds.top - union.top) as f32 * k) as i32 + 2,
            right: (mx + (m.bounds.right - union.left) as f32 * k) as i32 - 2,
            bottom: (my + (m.bounds.bottom - union.top) as f32 * k) as i32 - 2,
        };
        let current = m.bounds.left == f.bounds.left && m.bounds.top == f.bounds.top;
        let (colour, strength) = if current {
            (f.color, 0.55)
        } else {
            (Color::WHITE, 0.14)
        };
        s.fill_rect(rect, colour, strength * fade);
        for edge in [
            RECT {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.top + 2,
            },
            RECT {
                left: rect.left,
                top: rect.bottom - 2,
                right: rect.right,
                bottom: rect.bottom,
            },
            RECT {
                left: rect.left,
                top: rect.top,
                right: rect.left + 2,
                bottom: rect.bottom,
            },
            RECT {
                left: rect.right - 2,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
        ] {
            s.fill_rect(edge, Color::WHITE, 0.85 * fade);
        }
    }
    let dot = (
        mx + (f.pointer.x - union.left) as f32 * k,
        my + (f.pointer.y - union.top) as f32 * k,
    );
    s.ring(dot, 7.0 * f.scale, 3.0 * f.scale, Color::WHITE, fade);
    s.soft_disc(dot, 6.0 * f.scale, Color::rgb(255, 40, 40), fade);
}

pub fn luminance(pixel: u32) -> f32 {
    let (r, g, b) = ((pixel >> 16) & 0xFF, (pixel >> 8) & 0xFF, pixel & 0xFF);
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

/// Average luminance of a pixel buffer (sampled, not exhaustive).
pub fn average_luminance(pixels: &[u32]) -> f32 {
    if pixels.is_empty() {
        return 0.5;
    }
    let step = (pixels.len() / 2048).max(1);
    let (mut sum, mut count) = (0.0, 0u32);
    for pixel in pixels.iter().step_by(step) {
        sum += luminance(*pixel);
        count += 1;
    }
    sum / count as f32
}

/// The configured colour, darkened on a light backdrop and lightened on a
/// dark one, so it never melts into what is behind it. `lum` is 0 (black)..1.
pub fn auto_color(base: Color, lum: f32) -> Color {
    let light = smoothstep(0.35, 0.65, lum);
    let mix = |c: u8| {
        let for_dark = c as f32 + (255.0 - c as f32) * 0.35;
        let for_light = c as f32 * 0.3;
        (for_dark + (for_light - for_dark) * light).round() as u8
    };
    Color {
        r: mix(base.r),
        g: mix(base.g),
        b: mix(base.b),
        a: base.a,
    }
}

/// Black or white, whichever separates from the ring drawn for `lum`.
fn outline_for(lum: f32) -> Color {
    if lum > 0.5 {
        Color::WHITE
    } else {
        Color::BLACK
    }
}

/// A reusable copy of a screen region. Layered windows (ours) are not part of
/// a plain `BitBlt` from the screen, and the overlays also opt out of capture
/// (`WDA_EXCLUDEFROMCAPTURE`), so what comes back is what is *behind* the
/// effect.
pub struct Capturer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Capturer {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = unsafe { CreateCompatibleDC(None) };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) };
        let Ok(bitmap) = bitmap else {
            let _ = unsafe { DeleteDC(dc) };
            return None;
        };
        if bits.is_null() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            let _ = unsafe { DeleteDC(dc) };
            return None;
        }
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        Some(Self {
            dc,
            bitmap,
            previous,
            bits: bits as *mut u32,
            w,
            h,
        })
    }

    /// The `w`x`h` region of the screen whose top left is `(x, y)`, top row first.
    pub fn grab(&mut self, x: i32, y: i32) -> &[u32] {
        unsafe {
            let screen = GetDC(None);
            let _ = BitBlt(self.dc, 0, 0, self.w, self.h, Some(screen), x, y, SRCCOPY);
            ReleaseDC(None, screen);
            std::slice::from_raw_parts(self.bits, (self.w * self.h) as usize)
        }
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_darkens_on_light_and_lightens_on_dark() {
        let base = Color::rgb(0xFF, 0xB0, 0x00);
        let on_light = auto_color(base, 1.0);
        let on_dark = auto_color(base, 0.0);
        assert!(on_light.r < base.r && on_light.g < base.g);
        assert!(on_dark.b > base.b);
        assert_eq!(auto_color(base, 0.5).a, base.a);
    }

    #[test]
    fn luminance_runs_from_black_to_white() {
        assert_eq!(luminance(0x000000), 0.0);
        assert!((luminance(0xFFFFFF) - 1.0).abs() < 1e-3);
        assert_eq!(average_luminance(&[]), 0.5);
    }

    #[test]
    fn a_beacon_frame_draws_something_and_stays_inside() {
        let (w, h) = (200, 200);
        let mut pixels = vec![0u32; (w * h) as usize];
        let frame = Frame {
            shape: Shape::Beacon,
            radius: 60.0,
            scale: 1.0,
            color: Color::rgb(255, 176, 0),
            dim: 0.6,
            monitors: &[],
            bounds: RECT::default(),
            docked: false,
            pointer: POINT::default(),
            backdrop: &[],
            lum: 0.5,
            t: 0.3,
            secs: 0.3,
            total: 1.0,
        };
        draw(&mut pixels, (w, h), (100.0, 100.0), &frame);
        assert!(pixels.iter().any(|p| p >> 24 != 0));
        // The corner is out of reach of the effect.
        assert_eq!(pixels[0], 0);
    }

    #[test]
    fn a_docked_map_fits_its_panel_and_draws_no_border() {
        let panel = map_panel_size(&[], 1.0);
        assert_eq!(panel, (1, 1), "no monitor, no map");

        let bounds = RECT {
            left: 100,
            top: 50,
            right: 1100,
            bottom: 650,
        };
        let origin = map_panel_origin(bounds, (400, 200), 1.0);
        assert_eq!(origin.x, 100 + (1000 - 400) / 2);
        assert_eq!(origin.y, 50 + (600.0f32 * 0.22 - 16.0) as i32);
    }

    #[test]
    fn shapes_that_cover_the_monitor_are_not_small() {
        assert!(is_small(Shape::Halo));
        assert!(!is_small(Shape::Spotlight));
        assert!(!is_small(Shape::Monitor));
        assert!(reads_backdrop(Shape::Negative));
        assert!(!reads_backdrop(Shape::Halo));
    }
}
