//! What the HUD looks like, drawn: the colours, the shapes, the geometry of each
//! style and the one function that paints a reading into a canvas.
//!
//! Nothing here needs a window. The tray application draws into a layered window's
//! canvas, the mixer reuses the shapes and colours, and the settings app draws the
//! same pictures into a canvas of its own to show a preview ([`render_hud`]), so
//! what it shows is what y-coustic draws and not a second copy of the rules.
//!
//! Draw order is fill, then text, then the vector shapes, with the corners rounded
//! last: GDI text zeroes alpha wherever it touches, and the corner cut has to see
//! the finished image.

use windows::Win32::Foundation::RECT;
use windows::core::Result;

use y_core::overlay::{Canvas, Color, Font, TextAlign, alpha_of_percent};
use y_core::theme;

use crate::config::{self, Style};
use crate::look::Look;
use crate::model::{self, Kind, Level, State};

pub mod layout;

use layout::{Geometry, PADDING};

/// How the HUD looks, from the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    pub style: Style,
    pub scale: u32,
    pub radius: i32,
    pub border: i32,
    pub opacity: u8,
    pub font: String,
    pub colors: config::Colors,
    pub interactive: bool,
    /// Light (`Some(false)`) or dark (`Some(true)`), or the system's (`None`).
    pub dark: Option<bool>,
}

impl Skin {
    pub fn of(settings: &config::HudConfig) -> Self {
        Self {
            style: settings.style,
            scale: settings.scale,
            radius: settings.radius as i32,
            border: settings.border as i32,
            opacity: alpha_of_percent(settings.opacity),
            font: settings.font.clone(),
            colors: settings.colors,
            interactive: settings.interactive,
            dark: None,
        }
    }

    /// Whether a window built for `other` serves this skin: its size, font and
    /// whether it takes the pointer are fixed at creation, the rest is drawn.
    pub fn same_window(&self, other: &Skin) -> bool {
        self.style == other.style
            && self.scale == other.scale
            && self.font == other.font
            && self.interactive == other.interactive
    }

    /// A design length in real pixels: the size setting, then the DPI.
    pub fn px(&self, dpi: u32, length: i32) -> i32 {
        length * self.scale as i32 * dpi as i32 / (100 * 96)
    }
}

pub struct Palette {
    pub panel: Color,
    pub text: Color,
    pub track: Color,
    pub fill: Color,
    pub glyph: Color,
}

impl Palette {
    /// The theme's colours, then what `<colors>` says: for everything, then for
    /// this kind. The microphone's bar is red while it is live, the state worth
    /// noticing, so the general `accent` does not reach it: only its own does.
    pub fn current(kind: Kind, muted: bool, colors: &config::Colors) -> Self {
        Self::for_theme(theme::is_dark(), kind, muted, colors)
    }

    /// [`Palette::current`] for a theme chosen by the caller: a preview shows light
    /// and dark whatever Windows is set to.
    pub fn for_theme(dark: bool, kind: Kind, muted: bool, colors: &config::Colors) -> Self {
        let mut palette = Self::base(dark, muted);
        let own = match kind {
            Kind::Master => &colors.master,
            Kind::Mic => &colors.microphone,
            Kind::App => &colors.application,
        };
        if kind == Kind::Mic && !muted {
            palette.fill = if dark {
                Color::rgb(0xE5, 0x6B, 0x66)
            } else {
                Color::rgb(0xC4, 0x2B, 0x1C)
            };
        }
        for tint in [&colors.all, own] {
            let is_own = std::ptr::eq(tint, own);
            palette.panel = tint.panel.unwrap_or(palette.panel);
            if let Some(text) = tint.text {
                palette.text = text;
                palette.glyph = text;
            }
            palette.track = tint.track.unwrap_or(palette.track);
            let accent = if kind == Kind::Mic && !is_own {
                None
            } else {
                tint.accent
            };
            if muted {
                palette.fill = tint.muted.unwrap_or(palette.fill);
            } else {
                palette.fill = accent.unwrap_or(palette.fill);
            }
        }
        palette
    }

    fn base(dark: bool, muted: bool) -> Self {
        if dark {
            Self {
                panel: Color::rgb(0x2B, 0x2B, 0x2B),
                text: Color::rgb(0xF2, 0xF2, 0xF2),
                track: Color::rgb(0x55, 0x55, 0x55),
                fill: if muted {
                    Color::rgb(0x8A, 0x8A, 0x8A)
                } else {
                    Color::rgb(0x60, 0xA5, 0xFA)
                },
                glyph: Color::rgb(0xF2, 0xF2, 0xF2),
            }
        } else {
            Self {
                panel: Color::rgb(0xF3, 0xF3, 0xF3),
                text: Color::rgb(0x1B, 0x1B, 0x1B),
                track: Color::rgb(0xC8, 0xC8, 0xC8),
                fill: if muted {
                    Color::rgb(0x9A, 0x9A, 0x9A)
                } else {
                    Color::rgb(0x00, 0x67, 0xC0)
                },
                glyph: Color::rgb(0x1B, 0x1B, 0x1B),
            }
        }
    }
}

/// Draws what the HUD shows into `canvas`: the volume of a `kind` (`None` when there
/// is no such device), with `look` for an application's name and icon, in `skin`
/// at `dpi`. The canvas is the size the style asks for at that scale; draw order
/// is fill, text, shapes, then the corners.
pub fn draw(
    canvas: &mut Canvas,
    font: &Font,
    skin: &Skin,
    dpi: u32,
    kind: Kind,
    state: Option<State>,
    look: Option<&Look>,
) -> Result<()> {
    let level = state.map_or(0.0, |state| state.level);
    let muted = state.is_none_or(|state| state.muted);
    let dark = skin.dark.unwrap_or_else(theme::is_dark);
    let palette = Palette::for_theme(dark, kind, muted, &skin.colors);
    let (width, height) = canvas.size();
    let px = |length: i32| skin.px(dpi, length);

    // In the `bar` style an application names itself on an upper row; the bar
    // and the percentage then share the lower one. Everything else is one row.
    let named = skin.style == Style::Bar && look.is_some() && state.is_some();
    let g = Geometry::new(skin.style, width, height, named, px);

    // Panel: opaque, so the text below has a uniform alpha to restore.
    canvas.clear(palette.panel);

    // The percentage.
    if let (Some(state), Some(rect)) = (state, g.text) {
        let text = if state.muted {
            "Muted".to_owned()
        } else {
            format!("{}%", model::to_percent(state.level))
        };
        let align = match skin.style {
            Style::Bar => TextAlign::Right,
            Style::Text => TextAlign::Left,
            _ => TextAlign::Centre,
        };
        canvas.draw_text(&text, rect, font, palette.text, align)?;
    }

    // Glyph, or the application's own icon.
    let glyph = g.glyph.right - g.glyph.left;
    let waves = waves(level, muted);
    let side = glyph as f32;
    match look.and_then(|look| look.pixels.as_deref().map(|pixels| (look.size, pixels))) {
        Some((size, pixels)) if size == glyph => {
            canvas.blit(g.glyph.left, g.glyph.top, size, size, pixels);
        }
        _ => paint(
            canvas,
            g.glyph.left,
            g.glyph.top,
            glyph,
            glyph,
            palette.glyph,
            |x, y| in_glyph(kind, x / side, y / side, waves, muted),
        ),
    }

    if let (true, Some(look), Some(track)) = (named, look, g.track) {
        let name_rect = RECT {
            left: track.left,
            top: px(4),
            right: width - px(PADDING),
            bottom: g.row_top,
        };
        canvas.draw_text_ellipsis(&look.name, name_rect, font, palette.text, TextAlign::Left)?;
    }
    if state.is_none() {
        // Nothing to set: say why, where the bar would be (a narrow style
        // gets what fits of it).
        let message = match skin.style {
            Style::Bar | Style::Compact | Style::Text => RECT {
                left: g.glyph.right + px(PADDING * 3 / 4),
                top: 0,
                right: width - px(PADDING),
                bottom: height,
            },
            Style::VBar | Style::Ring => g.text.unwrap_or(RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            }),
        };
        let align = match skin.style {
            Style::VBar | Style::Ring => TextAlign::Centre,
            _ => TextAlign::Left,
        };
        canvas.draw_text_ellipsis(missing_text(kind), message, font, palette.text, align)?;
        finish(canvas, skin, &px, palette.track);
        return Ok(());
    }

    if let Some(track) = g.track {
        let (w, h) = (track.right - track.left, track.bottom - track.top);
        if g.ring() {
            let (outer, thickness) = (g.outer as f32, g.thickness as f32);
            paint(
                canvas,
                track.left,
                track.top,
                w,
                h,
                palette.track,
                |x, y| in_ring(x - outer, y - outer, outer, thickness),
            );
            if level > 0.0 {
                paint(canvas, track.left, track.top, w, h, palette.fill, |x, y| {
                    in_arc(x - outer, y - outer, outer, thickness, level)
                });
            }
        } else if g.vertical() {
            paint(
                canvas,
                track.left,
                track.top,
                w,
                h,
                palette.track,
                |x, y| in_capsule(y, x, h as f32, w as f32),
            );
            let filled = fill_width(level, h, w);
            if filled > 0 {
                paint(
                    canvas,
                    track.left,
                    track.bottom - filled,
                    w,
                    filled,
                    palette.fill,
                    |x, y| in_capsule(y, x, filled as f32, w as f32),
                );
            }
        } else {
            paint(
                canvas,
                track.left,
                track.top,
                w,
                h,
                palette.track,
                |x, y| in_capsule(x, y, w as f32, h as f32),
            );
            let filled = fill_width(level, w, h);
            if filled > 0 {
                paint(
                    canvas,
                    track.left,
                    track.top,
                    filled,
                    h,
                    palette.fill,
                    |x, y| in_capsule(x, y, filled as f32, h as f32),
                );
            }
        }
    }

    finish(canvas, skin, &px, palette.track);
    Ok(())
}

/// The last step: the border, then the corners, which see the finished image.
fn finish(canvas: &mut Canvas, skin: &Skin, px: &impl Fn(i32) -> i32, border_color: Color) {
    let radius = px(skin.radius);
    let border = if skin.border > 0 {
        px(skin.border).max(1)
    } else {
        0
    };
    if border == 0 {
        canvas.round_corners(radius);
        return;
    }
    let (width, height) = canvas.size();
    for inset in 0..border {
        canvas.stroke_rect(
            RECT {
                left: inset,
                top: inset,
                right: width - inset,
                bottom: height - inset,
            },
            border_color,
        );
    }
    canvas.round_corners_with_border(radius, border, border_color);
}

/// Is (`dx`, `dy`), relative to the middle of a ring of outer radius `outer`
/// and band `thickness`, on the band?
fn in_ring(dx: f32, dy: f32, outer: f32, thickness: f32) -> bool {
    let distance = (dx * dx + dy * dy).sqrt();
    distance <= outer && distance >= outer - thickness
}

/// The part of the ring from the top, clockwise, as far as `level` (0 to 1),
/// with a round end at both ends of the arc.
fn in_arc(dx: f32, dy: f32, outer: f32, thickness: f32, level: Level) -> bool {
    let level = level.clamp(0.0, 1.0);
    let cap = thickness / 2.0;
    let middle = outer - cap;
    let angle_end = level * std::f32::consts::TAU;
    let at = |angle: f32| (middle * angle.sin(), -middle * angle.cos());
    let in_cap = |(cx, cy): (f32, f32)| (dx - cx).powi(2) + (dy - cy).powi(2) <= cap * cap;
    if in_cap(at(0.0)) || in_cap(at(angle_end)) {
        return true;
    }
    if !in_ring(dx, dy, outer, thickness) {
        return false;
    }
    let mut angle = dx.atan2(-dy);
    if angle < 0.0 {
        angle += std::f32::consts::TAU;
    }
    angle <= angle_end
}

/// What the HUD says when the device it was asked about does not exist.
fn missing_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Mic => "No microphone",
        Kind::Master => "No audio output",
        Kind::App => "No audio",
    }
}

/// Width of the filled part of the bar: nothing at silence, and never less
/// than the bar's height otherwise, so the rounded end stays a circle.
pub fn fill_width(level: Level, bar_width: i32, bar_height: i32) -> i32 {
    if level <= 0.0 {
        return 0;
    }
    ((bar_width as f32 * level.min(1.0)).round() as i32).clamp(bar_height, bar_width)
}

/// How many sound waves the speaker glyph shows.
pub fn waves(level: Level, muted: bool) -> u8 {
    if muted || level <= 0.0 {
        0
    } else if level < 0.5 {
        1
    } else {
        2
    }
}

/// The glyph of `kind`: a microphone for the microphone, a speaker otherwise.
pub fn in_glyph(kind: Kind, u: f32, v: f32, waves: u8, muted: bool) -> bool {
    match kind {
        Kind::Mic => in_microphone(u, v, muted),
        Kind::Master | Kind::App => in_speaker(u, v, waves, muted),
    }
}

/// Is the point in the unit square inside the microphone glyph? A capsule, the
/// cradle under it, a stem and a base; slashed through when muted.
fn in_microphone(u: f32, v: f32, muted: bool) -> bool {
    // Capsule: a vertical segment with a radius.
    if distance_to_segment((u, v), (0.5, 0.26), (0.5, 0.44)) <= 0.13 {
        return true;
    }
    // Cradle: the lower half of a ring around the capsule's lower centre.
    let (dx, dy) = (u - 0.5, v - 0.44);
    let radius = (dx * dx + dy * dy).sqrt();
    if dy >= 0.0 && (radius - 0.22).abs() <= 0.03 {
        return true;
    }
    // Stem and base.
    if (0.485..0.515).contains(&u) && (0.64..0.84).contains(&v) {
        return true;
    }
    if (0.34..0.66).contains(&u) && (0.82..0.87).contains(&v) {
        return true;
    }
    // The slash of the muted glyph.
    muted && distance_to_segment((u, v), (0.18, 0.12), (0.82, 0.92)) <= 0.045
}

/// Is the point (`u`, `v`) in the unit square inside the speaker glyph? The
/// cone and body at the left; waves as arcs on the right; when muted, a cross
/// where the waves would be.
fn in_speaker(u: f32, v: f32, waves: u8, muted: bool) -> bool {
    // Body.
    if (0.08..0.28).contains(&u) && (0.38..0.62).contains(&v) {
        return true;
    }
    // Cone, widening to the right.
    if (0.28..0.5).contains(&u) {
        let half = 0.12 + (u - 0.28) / 0.22 * 0.20;
        if (v - 0.5).abs() <= half {
            return true;
        }
    }
    // Waves: arcs of rings around the cone's mouth, within 50 degrees of the
    // axis.
    let (dx, dy) = (u - 0.5, v - 0.5);
    if dx > 0.0 {
        let radius = (dx * dx + dy * dy).sqrt();
        let within_angle = dy.abs() <= dx * 1.19; // tan(50 degrees)
        if within_angle {
            if waves >= 1 && (radius - 0.24).abs() <= 0.035 {
                return true;
            }
            if waves >= 2 && (radius - 0.38).abs() <= 0.035 {
                return true;
            }
        }
    }
    // The cross of the muted glyph.
    if muted {
        let thickness = 0.045;
        return distance_to_segment((u, v), (0.62, 0.36), (0.9, 0.64)) <= thickness
            || distance_to_segment((u, v), (0.62, 0.64), (0.9, 0.36)) <= thickness;
    }
    false
}

fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let (apx, apy) = (p.0 - a.0, p.1 - a.1);
    let t = ((apx * abx + apy * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * abx, a.1 + t * aby);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Is the pixel-space point (`x`, `y`) inside a capsule (a rounded bar) of
/// `width` by `height` at the origin?
pub fn in_capsule(x: f32, y: f32, width: f32, height: f32) -> bool {
    let radius = height / 2.0;
    let centre = (x.clamp(radius, (width - radius).max(radius)), radius);
    (x - centre.0).powi(2) + (y - centre.1).powi(2) <= radius * radius
}

/// Paints `color` over the opaque pixels of a `width` by `height` box at
/// (`left`, `top`) wherever `inside` holds, antialiased by 4x4 supersampling.
///
/// `inside` takes pixel offsets from the box's top left.
pub fn paint(
    canvas: &mut Canvas,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    color: Color,
    inside: impl Fn(f32, f32) -> bool,
) {
    const N: i32 = 4;
    let (surface_width, surface_height) = canvas.size();
    let pixels = canvas.pixels_mut();

    for y in top.max(0)..(top + height).min(surface_height) {
        for x in left.max(0)..(left + width).min(surface_width) {
            let mut hits = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let fx = (x - left) as f32 + (sx as f32 + 0.5) / N as f32;
                    let fy = (y - top) as f32 + (sy as f32 + 0.5) / N as f32;
                    hits += i32::from(inside(fx, fy));
                }
            }
            if hits == 0 {
                continue;
            }
            let index = (y * surface_width + x) as usize;
            pixels[index] = mix(pixels[index], color, hits as f32 / (N * N) as f32);
        }
    }
}

/// `color` over an opaque premultiplied `below`, with `coverage` in 0..=1.
pub fn mix(below: u32, color: Color, coverage: f32) -> u32 {
    let channel = |shift: u32, value: u8| -> u32 {
        let from = ((below >> shift) & 0xFF) as f32;
        (from + (f32::from(value) - from) * coverage).round() as u32
    };
    0xFF00_0000 | (channel(16, color.r) << 16) | (channel(8, color.g) << 8) | channel(0, color.b)
}

/// A HUD drawn into memory: its size in pixels, and the surface as
/// `0xAARRGGBB` premultiplied words, top row first.
pub struct Rendered {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
}

/// Draws the HUD the settings describe, for a reading of `kind` (a volume, or
/// `None` for a device that is not there), in the light or dark theme and at
/// `dpi`, into a canvas of its own: no window, no timer, nothing on the screen.
/// What a settings app shows as a preview.
pub fn render(
    settings: &config::HudConfig,
    kind: Kind,
    state: Option<State>,
    look: Option<&Look>,
    dark: bool,
    dpi: u32,
) -> Result<Rendered> {
    let mut skin = Skin::of(settings);
    skin.dark = Some(dark);
    let (width, height) = layout::design_size(settings.style);
    let size = (skin.px(dpi, width).max(1), skin.px(dpi, height).max(1));
    let mut canvas = Canvas::new(size)?;
    let font = Font::new(&skin.font, skin.px(dpi, layout::FONT_HEIGHT), true)?;
    draw(&mut canvas, &font, &skin, dpi, kind, state, look)?;
    Ok(Rendered {
        width: size.0,
        height: size.1,
        pixels: canvas.pixels_mut().to_vec(),
    })
}

/// [`render`] as PNG bytes with straight alpha, which is what an image control
/// loads; `None` if anything fails.
pub fn render_png(
    settings: &config::HudConfig,
    kind: Kind,
    state: Option<State>,
    look: Option<&Look>,
    dark: bool,
    dpi: u32,
) -> Option<Vec<u8>> {
    let picture = render(settings, kind, state, look, dark, dpi).ok()?;
    let mut rgba = Vec::with_capacity(picture.pixels.len() * 4);
    for word in &picture.pixels {
        let alpha = word >> 24;
        let straight = |shift: u32| {
            (((word >> shift) & 0xFF) * 255)
                .checked_div(alpha)
                .map_or(0, |value| value.min(255) as u8)
        };
        rgba.extend_from_slice(&[straight(16), straight(8), straight(0), alpha as u8]);
    }
    let png = y_core::png::encode_rgba(picture.width as u32, picture.height as u32, &rgba);
    (!png.is_empty()).then_some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_empty_at_silence_and_full_at_one() {
        assert_eq!(fill_width(0.0, 100, 6), 0);
        assert_eq!(fill_width(1.0, 100, 6), 100);
        assert_eq!(fill_width(0.5, 100, 6), 50);
    }

    #[test]
    fn a_nearly_silent_bar_keeps_its_rounded_end() {
        assert_eq!(fill_width(0.01, 100, 6), 6);
    }

    #[test]
    fn waves_follow_the_level_and_vanish_when_muted() {
        assert_eq!(waves(0.0, false), 0);
        assert_eq!(waves(0.2, false), 1);
        assert_eq!(waves(0.5, false), 2);
        assert_eq!(waves(1.0, false), 2);
        assert_eq!(waves(1.0, true), 0);
    }

    #[test]
    fn the_glyph_has_a_body_and_a_cross_only_when_muted() {
        assert!(in_speaker(0.18, 0.5, 0, false));
        assert!(!in_speaker(0.76, 0.5, 0, false));
        assert!(in_speaker(0.76, 0.5, 0, true)); // the cross' centre
        assert!(!in_speaker(0.5, 0.05, 2, false));
    }

    #[test]
    fn the_microphone_glyph_is_slashed_only_when_muted() {
        assert!(in_microphone(0.5, 0.35, false)); // the capsule
        assert!(in_microphone(0.5, 0.84, false)); // the base
        assert!(!in_microphone(0.15, 0.15, false));
        assert!(in_microphone(0.5, 0.52, true)); // the middle of the slash
        assert!(!in_microphone(0.2, 0.9, true));
        assert!(!in_microphone(0.8, 0.2, true));
    }

    #[test]
    fn the_glyph_follows_the_kind() {
        assert!(in_glyph(Kind::Master, 0.18, 0.5, 0, false)); // speaker body
        assert!(!in_glyph(Kind::Mic, 0.18, 0.5, 0, false));
        assert!(in_glyph(Kind::Mic, 0.5, 0.35, 0, false));
    }

    #[test]
    fn each_missing_device_has_its_own_message() {
        assert_eq!(missing_text(Kind::Mic), "No microphone");
        assert_ne!(missing_text(Kind::Master), missing_text(Kind::Mic));
    }

    #[test]
    fn a_wave_only_shows_when_asked_for() {
        // On the inner ring, level with the cone's mouth.
        assert!(in_speaker(0.5 + 0.24, 0.5, 1, false));
        assert!(!in_speaker(0.5 + 0.24, 0.5, 0, false));
        assert!(in_speaker(0.5 + 0.38, 0.5, 2, false));
        assert!(!in_speaker(0.5 + 0.38, 0.5, 1, false));
    }

    #[test]
    fn a_capsule_is_round_at_both_ends() {
        assert!(in_capsule(50.0, 3.0, 100.0, 6.0));
        assert!(in_capsule(3.0, 3.0, 100.0, 6.0));
        assert!(!in_capsule(0.2, 0.2, 100.0, 6.0)); // outside the corner arc
        assert!(!in_capsule(99.8, 5.8, 100.0, 6.0));
    }

    #[test]
    fn mixing_blends_toward_the_colour() {
        let below = 0xFF00_0000;
        assert_eq!(mix(below, Color::rgb(200, 100, 50), 0.0), below);
        assert_eq!(mix(below, Color::rgb(200, 100, 50), 1.0), 0xFFC8_6432);
        assert_eq!(mix(below, Color::rgb(200, 100, 50), 0.5), 0xFF64_3219);
    }

    #[test]
    fn a_ring_is_a_band_and_its_arc_runs_clockwise_from_the_top() {
        // On the band at the right (a quarter turn), not in the middle or outside.
        assert!(in_ring(40.0, 0.0, 44.0, 8.0));
        assert!(!in_ring(0.0, 0.0, 44.0, 8.0));
        assert!(!in_ring(50.0, 0.0, 44.0, 8.0));
        // A quarter: the top and the right are filled, the bottom and the left are not.
        assert!(in_arc(0.0, -40.0, 44.0, 8.0, 0.25));
        assert!(in_arc(40.0, 0.0, 44.0, 8.0, 0.25));
        assert!(!in_arc(0.0, 40.0, 44.0, 8.0, 0.25));
        assert!(!in_arc(-40.0, 0.0, 44.0, 8.0, 0.25));
        // Everything at full, a rounded start even at a sliver.
        assert!(in_arc(-40.0, 0.0, 44.0, 8.0, 1.0));
        assert!(in_arc(0.0, -40.0, 44.0, 8.0, 0.001));
    }

    #[test]
    fn a_skin_scales_by_size_then_by_dpi_and_keeps_its_window_when_only_drawn_things_change() {
        let mut settings = config::HudConfig::default();
        let skin = Skin::of(&settings);
        assert_eq!(skin.px(96, 100), 100);
        assert_eq!(skin.px(192, 100), 200);
        settings.scale = 150;
        assert_eq!(Skin::of(&settings).px(96, 100), 150);

        let base = Skin::of(&config::HudConfig::default());
        let drawn = Skin::of(&config::HudConfig {
            radius: 0,
            border: 3,
            opacity: 50,
            ..config::HudConfig::default()
        });
        assert!(base.same_window(&drawn));
        for different in [
            config::HudConfig {
                style: Style::Ring,
                ..config::HudConfig::default()
            },
            config::HudConfig {
                scale: 200,
                ..config::HudConfig::default()
            },
            config::HudConfig {
                font: "Consolas".into(),
                ..config::HudConfig::default()
            },
            config::HudConfig {
                interactive: false,
                ..config::HudConfig::default()
            },
        ] {
            assert!(!base.same_window(&Skin::of(&different)));
        }
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;
    use config::{Colors, Tint};

    #[test]
    fn a_colour_reaches_its_kind_and_the_general_accent_spares_the_microphone() {
        let green = Color::rgb(0, 255, 0);
        let blue = Color::rgb(0, 0, 255);
        let colors = Colors {
            all: Tint {
                accent: Some(green),
                ..Tint::default()
            },
            application: Tint {
                accent: Some(blue),
                ..Tint::default()
            },
            ..Colors::default()
        };
        assert_eq!(Palette::current(Kind::Master, false, &colors).fill, green);
        assert_eq!(Palette::current(Kind::App, false, &colors).fill, blue);
        let mic = Palette::current(Kind::Mic, false, &colors).fill;
        assert_ne!(mic, green);
        assert_eq!(
            mic,
            Palette::current(Kind::Mic, false, &Colors::default()).fill
        );

        let own = Colors {
            microphone: Tint {
                accent: Some(blue),
                ..Tint::default()
            },
            ..colors
        };
        assert_eq!(Palette::current(Kind::Mic, false, &own).fill, blue);
        // Muted is its own colour, not the accent.
        assert_ne!(Palette::current(Kind::Master, true, &colors).fill, green);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::config::HudConfig;

    fn state(level: Level, muted: bool) -> Option<State> {
        Some(State { level, muted })
    }

    fn pixel(picture: &Rendered, x: i32, y: i32) -> u32 {
        picture.pixels[(y * picture.width + x) as usize]
    }

    fn opaque(color: Color) -> u32 {
        0xFF00_0000 | (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
    }

    #[test]
    fn each_style_is_drawn_at_its_own_size_scaled_by_size_and_dpi() {
        for style in [
            Style::Bar,
            Style::VBar,
            Style::Ring,
            Style::Text,
            Style::Compact,
        ] {
            let settings = HudConfig {
                style,
                ..HudConfig::default()
            };
            let (width, height) = layout::design_size(style);
            let picture =
                render(&settings, Kind::Master, state(0.5, false), None, true, 96).unwrap();
            assert_eq!(
                (picture.width, picture.height),
                (width, height),
                "{style:?}"
            );
            assert_eq!(picture.pixels.len(), (width * height) as usize);

            let scaled = HudConfig {
                scale: 150,
                ..settings
            };
            let big = render(&scaled, Kind::Master, state(0.5, false), None, true, 192).unwrap();
            assert_eq!(big.width, width * 3, "{style:?}");
        }
    }

    #[test]
    fn the_bar_fills_as_far_as_the_level_and_in_the_colour_of_its_kind() {
        let settings = HudConfig::default();
        let (width, height) = layout::design_size(settings.style);
        let geometry = Geometry::new(settings.style, width, height, false, |length| length);
        let track = geometry.track.expect("the bar style has a track");
        let probe = (
            track.left + (track.right - track.left) * 4 / 5,
            (track.top + track.bottom) / 2,
        );

        let palette = Palette::for_theme(true, Kind::Master, false, &settings.colors);
        let high = render(&settings, Kind::Master, state(0.95, false), None, true, 96).unwrap();
        let low = render(&settings, Kind::Master, state(0.10, false), None, true, 96).unwrap();
        assert_eq!(pixel(&high, probe.0, probe.1), opaque(palette.fill));
        assert_eq!(pixel(&low, probe.0, probe.1), opaque(palette.track));

        // A live microphone is red, not the master's blue.
        let mic = render(&settings, Kind::Mic, state(0.95, false), None, true, 96).unwrap();
        let red = Palette::for_theme(true, Kind::Mic, false, &settings.colors);
        assert_eq!(pixel(&mic, probe.0, probe.1), opaque(red.fill));
        assert_ne!(red.fill, palette.fill);
    }

    #[test]
    fn the_theme_and_the_colours_come_from_the_caller_not_from_windows() {
        let settings = HudConfig::default();
        let dark = render(&settings, Kind::Master, state(0.5, false), None, true, 96).unwrap();
        let light = render(&settings, Kind::Master, state(0.5, false), None, false, 96).unwrap();
        // Left of the glyph, mid height: nothing is drawn there, only the panel.
        let at = (4, dark.height / 2);
        assert_eq!(
            pixel(&dark, at.0, at.1),
            opaque(Palette::for_theme(true, Kind::Master, false, &settings.colors).panel)
        );
        assert_eq!(
            pixel(&light, at.0, at.1),
            opaque(Palette::for_theme(false, Kind::Master, false, &settings.colors).panel)
        );

        let mut tinted = HudConfig::default();
        tinted.colors.all.panel = Some(Color::rgb(0x12, 0x34, 0x56));
        let custom = render(&tinted, Kind::Master, state(0.5, false), None, true, 96).unwrap();
        assert_eq!(
            pixel(&custom, at.0, at.1),
            opaque(Color::rgb(0x12, 0x34, 0x56))
        );
    }

    #[test]
    fn the_corners_are_cut_by_the_radius() {
        let round = HudConfig {
            radius: 12,
            ..HudConfig::default()
        };
        let square = HudConfig {
            radius: 0,
            ..HudConfig::default()
        };
        let rounded = render(&round, Kind::Master, state(0.5, false), None, true, 96).unwrap();
        let sharp = render(&square, Kind::Master, state(0.5, false), None, true, 96).unwrap();
        assert_eq!(pixel(&rounded, 0, 0) >> 24, 0, "the corner is transparent");
        assert_eq!(pixel(&sharp, 0, 0) >> 24, 0xFF, "no radius, no cut");
    }

    #[test]
    fn a_picture_comes_out_as_a_png() {
        let png = render_png(
            &HudConfig::default(),
            Kind::Master,
            state(0.4, false),
            None,
            true,
            96,
        )
        .expect("a PNG");
        assert_eq!(&png[1..4], b"PNG");
        // No device: a message instead of a bar, still a picture.
        assert!(render_png(&HudConfig::default(), Kind::Mic, None, None, false, 96).is_some());
    }
}
