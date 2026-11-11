//! Where the parts of the HUD are, for each style, in the window's own pixels.
//!
//! Pure: the numbers are in one place, used by the drawing and by the pointer's
//! hit test, so the two cannot drift apart. Lengths are design pixels (at 100%
//! size on a 96 dpi screen) until `px` turns them into real ones.

use windows::Win32::Foundation::{POINT, RECT};

use y_core::monitor::Monitor;
use crate::config::{Position, Style};

// Design sizes, at 100% scaling.
pub const PADDING: i32 = 16;
pub const GLYPH: i32 = 28;
pub const GAP: i32 = 12;
pub const TEXT_WIDTH: i32 = 56;
pub const BAR_HEIGHT: i32 = 6;
pub const FONT_HEIGHT: i32 = 16;
/// Distance from the work area's edge.
pub const MARGIN: i32 = 64;
/// How far the HUD sits from the pointer it follows.
const CURSOR_OFFSET: i32 = 20;

/// The size of the window of `style` at 100%.
pub fn design_size(style: Style) -> (i32, i32) {
    match style {
        Style::Bar => (272, 56),
        Style::Compact => (176, 36),
        Style::Text => (136, 56),
        Style::VBar => (72, 208),
        Style::Ring => (120, 120),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub style: Style,
    pub width: i32,
    pub height: i32,
    /// Where the lower row starts and where its middle is: the whole height
    /// for a HUD of one row, the lower 60% for an application's in the `bar`
    /// style, which names itself above.
    pub row_top: i32,
    pub row_centre: i32,
    /// The icon or glyph, a square.
    pub glyph: RECT,
    /// What a click on the glyph reaches: the glyph and the gap around it.
    glyph_zone: RECT,
    /// The box of the bar (or, for a ring, the square around it); `None` for
    /// a style without one.
    pub track: Option<RECT>,
    /// The percentage's room; `None` for a style without it.
    pub text: Option<RECT>,
    /// The middle, the outer radius and the thickness of a ring (else zero).
    pub centre: POINT,
    pub outer: i32,
    pub thickness: i32,
}

/// What a point on the HUD is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    /// The icon or glyph: mutes.
    Glyph,
    /// The bar, with the level the point stands for, 0.0 to 1.0.
    Bar(f32),
    /// Anything else: the panel.
    Other,
}

fn square(left: i32, top: i32, side: i32) -> RECT {
    RECT {
        left,
        top,
        right: left + side,
        bottom: top + side,
    }
}

impl Geometry {
    /// `named` is an application's reading, which the `bar` style names above
    /// its bar.
    pub fn new(
        style: Style,
        width: i32,
        height: i32,
        named: bool,
        px: impl Fn(i32) -> i32,
    ) -> Self {
        let mut g = Self {
            style,
            width,
            height,
            row_top: 0,
            row_centre: height / 2,
            glyph: RECT::default(),
            glyph_zone: RECT::default(),
            track: None,
            text: None,
            centre: POINT::default(),
            outer: 0,
            thickness: 0,
        };
        match style {
            Style::Bar => {
                g.row_top = if named { height * 40 / 100 } else { 0 };
                g.row_centre = (g.row_top + height) / 2;
                let size = px(GLYPH);
                let left = px(PADDING);
                g.glyph = square(left, (height - size) / 2, size);
                let text = RECT {
                    left: width - px(PADDING) - px(TEXT_WIDTH),
                    top: g.row_top,
                    right: width - px(PADDING),
                    bottom: height,
                };
                let bar_left = g.glyph.right + px(GAP);
                g.track = Some(Self::bar(bar_left, text.left - px(GAP), g.row_centre, &px));
                g.text = Some(text);
            }
            Style::Compact => {
                let size = px(GLYPH * 3 / 4);
                let left = px(PADDING * 3 / 4);
                g.glyph = square(left, (height - size) / 2, size);
                let bar_left = g.glyph.right + px(GAP * 5 / 6);
                g.track = Some(Self::bar(
                    bar_left,
                    width - px(PADDING * 3 / 4),
                    g.row_centre,
                    &px,
                ));
            }
            Style::Text => {
                let size = px(GLYPH);
                g.glyph = square(px(PADDING), (height - size) / 2, size);
                g.text = Some(RECT {
                    left: g.glyph.right + px(GAP),
                    top: 0,
                    right: width - px(PADDING),
                    bottom: height,
                });
            }
            Style::VBar => {
                let size = px(GLYPH);
                g.glyph = square((width - size) / 2, px(PADDING), size);
                let text = RECT {
                    left: px(4),
                    top: height - px(PADDING) - px(20),
                    right: width - px(4),
                    bottom: height - px(PADDING),
                };
                let line = px(BAR_HEIGHT);
                g.track = Some(RECT {
                    left: (width - line) / 2,
                    top: g.glyph.bottom + px(GAP),
                    right: (width + line) / 2,
                    bottom: text.top - px(GAP),
                });
                g.text = Some(text);
            }
            Style::Ring => {
                g.centre = POINT {
                    x: width / 2,
                    y: height / 2,
                };
                g.thickness = px(8);
                g.outer = (width.min(height) / 2 - px(8)).max(1);
                g.track = Some(RECT {
                    left: g.centre.x - g.outer,
                    top: g.centre.y - g.outer,
                    right: g.centre.x + g.outer,
                    bottom: g.centre.y + g.outer,
                });
                let size = px(GLYPH);
                g.glyph = square(g.centre.x - size / 2, g.centre.y - size - px(2), size);
                g.text = Some(RECT {
                    left: g.centre.x - px(30),
                    top: g.centre.y + px(2),
                    right: g.centre.x + px(30),
                    bottom: g.centre.y + px(24),
                });
            }
        }
        g.glyph_zone = g.zone_of_glyph(&px);
        g
    }

    /// A horizontal bar's box, centred on `middle`.
    fn bar(left: i32, right: i32, middle: i32, px: &impl Fn(i32) -> i32) -> RECT {
        let height = px(BAR_HEIGHT);
        RECT {
            left,
            top: middle - height / 2,
            right: right.max(left + 1),
            bottom: middle - height / 2 + height,
        }
    }

    fn zone_of_glyph(&self, px: &impl Fn(i32) -> i32) -> RECT {
        let glyph = self.glyph;
        match self.style {
            // The gap to the bar is part of the target; the whole height of
            // the window is, as it is for the bar.
            Style::Bar | Style::Compact | Style::Text => RECT {
                left: glyph.left,
                top: 0,
                right: glyph.right + px(GAP) / 2,
                bottom: self.height,
            },
            Style::VBar | Style::Ring => {
                let around = px(GAP) / 2;
                RECT {
                    left: glyph.left - around,
                    top: glyph.top - around,
                    right: glyph.right + around,
                    bottom: glyph.bottom + around,
                }
            }
        }
    }

    pub fn vertical(&self) -> bool {
        self.style == Style::VBar
    }

    pub fn ring(&self) -> bool {
        self.style == Style::Ring
    }

    /// What is at (`x`, `y`) of the window. The bar counts for the whole height
    /// of its row: a 6-pixel line is not a target.
    pub fn hit(&self, x: i32, y: i32) -> Hit {
        let zone = self.glyph_zone;
        if (zone.left..zone.right).contains(&x) && (zone.top..zone.bottom).contains(&y) {
            return Hit::Glyph;
        }
        let Some(track) = self.track else {
            return Hit::Other;
        };
        let on_bar = if self.ring() {
            let (dx, dy) = ((x - self.centre.x) as f32, (y - self.centre.y) as f32);
            let distance = (dx * dx + dy * dy).sqrt();
            distance >= (self.outer - self.thickness * 2) as f32
                && distance <= (self.outer + self.thickness / 2) as f32
        } else if self.vertical() {
            (track.top..=track.bottom).contains(&y)
        } else {
            (track.left..=track.right).contains(&x) && y >= self.row_top
        };
        if on_bar {
            Hit::Bar(self.level_at(x, y))
        } else {
            Hit::Other
        }
    }

    /// The level a pointer at (`x`, `y`) stands for, within 0 to 1 whether or
    /// not it is still over the bar (a drag may leave it). A ring reads the
    /// angle from the top, clockwise.
    pub fn level_at(&self, x: i32, y: i32) -> f32 {
        let Some(track) = self.track else {
            return 0.0;
        };
        if self.ring() {
            let (dx, dy) = ((x - self.centre.x) as f32, (y - self.centre.y) as f32);
            let mut angle = dx.atan2(-dy);
            if angle < 0.0 {
                angle += std::f32::consts::TAU;
            }
            return (angle / std::f32::consts::TAU).clamp(0.0, 1.0);
        }
        if self.vertical() {
            let length = (track.bottom - track.top).max(1);
            return ((track.bottom - y) as f32 / length as f32).clamp(0.0, 1.0);
        }
        let length = (track.right - track.left).max(1);
        ((x - track.left) as f32 / length as f32).clamp(0.0, 1.0)
    }
}

/// Where the top left of a `size` HUD goes against `icon`: centred on it, above
/// when the icon is in the lower half of the monitor (a bottom taskbar), below
/// otherwise, a small gap between. Kept inside the work area.
pub fn against(icon: RECT, size: (i32, i32), monitor: &Monitor) -> POINT {
    let gap = monitor.scaled(GAP / 2);
    let area = monitor.work_area;
    let x = (icon.left + icon.right) / 2 - size.0 / 2;
    let middle = area.top + (area.bottom - area.top) / 2;
    let y = if icon.top >= middle {
        icon.top - size.1 - gap
    } else {
        icon.bottom + gap
    };
    monitor.keep_inside(POINT { x, y }, size)
}

/// Where the top left of a `size` HUD goes in `area`. `follow-cursor` is
/// [`follow`]'s business; here it stands for the bottom centre, which is also
/// where an `anchor="hud"` panel goes then.
pub fn place(area: RECT, size: (i32, i32), position: Position, monitor: &Monitor) -> POINT {
    let margin = monitor.scaled(MARGIN);
    let x = match position {
        Position::TopLeft | Position::CenterLeft | Position::BottomLeft => area.left + margin,
        Position::TopRight | Position::CenterRight | Position::BottomRight => {
            area.right - size.0 - margin
        }
        _ => area.left + ((area.right - area.left) - size.0) / 2,
    };
    let y = match position {
        Position::TopLeft | Position::TopCenter | Position::TopRight => area.top + margin,
        Position::CenterLeft | Position::Center | Position::CenterRight => {
            area.top + ((area.bottom - area.top) - size.1) / 2
        }
        _ => area.bottom - size.1 - margin,
    };
    POINT { x, y }
}

/// Where a `size` HUD goes beside the pointer at `pointer`: below and to the
/// right of it, or on the other side where that would leave the screen.
pub fn follow(pointer: POINT, size: (i32, i32), monitor: &Monitor) -> POINT {
    let offset = monitor.scaled(CURSOR_OFFSET);
    let area = monitor.work_area;
    let x = if pointer.x + offset + size.0 <= area.right {
        pointer.x + offset
    } else {
        pointer.x - offset - size.0
    };
    let y = if pointer.y + offset + size.1 <= area.bottom {
        pointer.y + offset
    } else {
        pointer.y - offset - size.1
    };
    monitor.keep_inside(POINT { x, y }, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(style: Style, named: bool) -> Geometry {
        let (width, height) = design_size(style);
        Geometry::new(style, width, height, named, |length| length)
    }

    fn bar(g: &Geometry) -> RECT {
        g.track.expect("a bar")
    }

    #[test]
    fn the_glyph_the_bar_and_the_rest_are_told_apart() {
        let g = geometry(Style::Bar, false);
        let (width, height) = design_size(Style::Bar);
        // The glyph, with the gap to the bar as part of its target.
        assert_eq!(g.hit(PADDING + 2, height / 2), Hit::Glyph);
        assert_eq!(g.hit(PADDING + GLYPH + 2, height / 2), Hit::Glyph);
        // The bar, anywhere along its row, not only on its 6 pixels.
        let middle = (bar(&g).left + bar(&g).right) / 2;
        match g.hit(middle, 3) {
            Hit::Bar(level) => assert!((level - 0.5).abs() < 0.02, "{level}"),
            other => panic!("{other:?}"),
        }
        // The percentage, and the margin: the panel's.
        assert_eq!(g.hit(width - PADDING - 4, height / 2), Hit::Other);
        assert_eq!(g.hit(2, height / 2), Hit::Other);
    }

    #[test]
    fn the_level_follows_the_pointer_and_stops_at_the_ends() {
        let g = geometry(Style::Bar, false);
        let track = bar(&g);
        assert_eq!(g.level_at(track.left - 40, 0), 0.0);
        assert_eq!(g.level_at(track.right + 40, 0), 1.0);
        assert_eq!(g.level_at(track.left, 0), 0.0);
        assert_eq!(g.level_at(track.right, 0), 1.0);
    }

    #[test]
    fn an_application_names_itself_above_the_bar() {
        let one = geometry(Style::Bar, false);
        let two = geometry(Style::Bar, true);
        let height = design_size(Style::Bar).1;
        assert_eq!(one.row_top, 0);
        assert!(two.row_top > 0 && two.row_top < height);
        assert!(two.row_centre > one.row_centre, "the bar sits lower");
        // The name row is not the bar: a click on it is the panel's.
        let middle = (bar(&two).left + bar(&two).right) / 2;
        assert_eq!(two.hit(middle, two.row_top / 2), Hit::Other);
        assert!(matches!(two.hit(middle, two.row_centre), Hit::Bar(_)));
    }

    #[test]
    fn the_parts_scale_with_the_monitor() {
        let plain = geometry(Style::Bar, false);
        let (width, height) = design_size(Style::Bar);
        let double = Geometry::new(Style::Bar, width * 2, height * 2, false, |length| length * 2);
        assert_eq!(bar(&double).left, bar(&plain).left * 2);
        assert_eq!(bar(&double).right, bar(&plain).right * 2);
        assert_eq!(double.glyph.right - double.glyph.left, GLYPH * 2);
    }

    #[test]
    fn every_style_fits_its_parts_in_its_window() {
        for style in [
            Style::Bar,
            Style::VBar,
            Style::Ring,
            Style::Text,
            Style::Compact,
        ] {
            let g = geometry(style, false);
            let inside = |rect: RECT| {
                rect.left >= 0 && rect.top >= 0 && rect.right <= g.width && rect.bottom <= g.height
            };
            assert!(inside(g.glyph), "{style:?} glyph");
            if let Some(track) = g.track {
                assert!(inside(track), "{style:?} track {track:?}");
                assert!(track.right > track.left && track.bottom > track.top, "{style:?}");
            }
            if let Some(text) = g.text {
                assert!(inside(text), "{style:?} text {text:?}");
            }
        }
    }

    #[test]
    fn only_some_styles_have_a_bar_or_a_percentage() {
        assert!(geometry(Style::Text, false).track.is_none());
        assert!(geometry(Style::Compact, false).text.is_none());
        assert!(geometry(Style::VBar, false).track.is_some());
        assert!(geometry(Style::Ring, false).text.is_some());
        // Without a bar there is nothing to set: a click off the glyph is the panel's.
        let text = geometry(Style::Text, false);
        assert_eq!(text.hit(100, 28), Hit::Other);
        assert_eq!(text.hit(PADDING + 2, 28), Hit::Glyph);
    }

    #[test]
    fn a_vertical_bar_fills_upward() {
        let g = geometry(Style::VBar, false);
        let track = bar(&g);
        let x = (track.left + track.right) / 2;
        assert_eq!(g.level_at(x, track.bottom), 0.0);
        assert_eq!(g.level_at(x, track.top), 1.0);
        assert_eq!(g.level_at(x, track.top - 50), 1.0);
        let middle = (track.top + track.bottom) / 2;
        assert!(matches!(g.hit(x, middle), Hit::Bar(level) if (level - 0.5).abs() < 0.02));
        // The percentage below is not the bar.
        assert_eq!(g.hit(x, g.text.unwrap().top + 4), Hit::Other);
        assert_eq!(g.hit(g.glyph.left + 2, g.glyph.top + 2), Hit::Glyph);
    }

    #[test]
    fn a_ring_reads_the_angle_clockwise_from_the_top() {
        let g = geometry(Style::Ring, false);
        let (cx, cy, r) = (g.centre.x, g.centre.y, g.outer);
        let top = g.level_at(cx, cy - r);
        assert!(!(0.01..=0.99).contains(&top), "{top}");
        assert!((g.level_at(cx + r, cy) - 0.25).abs() < 0.01);
        assert!((g.level_at(cx, cy + r) - 0.5).abs() < 0.01);
        assert!((g.level_at(cx - r, cy) - 0.75).abs() < 0.01);
        // On the ring it is a level, in the middle it is the glyph, beyond it the panel.
        assert!(matches!(g.hit(cx + r - 2, cy), Hit::Bar(_)));
        assert_eq!(g.hit(g.glyph.left + 4, g.glyph.top + 4), Hit::Glyph);
        assert_eq!(g.hit(1, 1), Hit::Other);
    }

    fn monitor() -> Monitor {
        let area = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        };
        Monitor {
            bounds: area,
            work_area: area,
            dpi: 96,
        }
    }

    #[test]
    fn placement_follows_the_position_setting() {
        let monitor = monitor();
        let area = monitor.work_area;
        let size = (200, 50);
        let at = |position| {
            let point = place(area, size, position, &monitor);
            (point.x, point.y)
        };
        assert_eq!(at(Position::BottomCenter), (400, 800 - 50 - MARGIN));
        assert_eq!(at(Position::TopCenter), (400, MARGIN));
        assert_eq!(at(Position::Center), (400, 375));
        assert_eq!(at(Position::TopLeft), (MARGIN, MARGIN));
        assert_eq!(at(Position::TopRight), (1000 - 200 - MARGIN, MARGIN));
        assert_eq!(at(Position::CenterLeft), (MARGIN, 375));
        assert_eq!(at(Position::CenterRight), (1000 - 200 - MARGIN, 375));
        assert_eq!(at(Position::BottomLeft), (MARGIN, 800 - 50 - MARGIN));
        assert_eq!(at(Position::BottomRight), (1000 - 200 - MARGIN, 800 - 50 - MARGIN));
    }

    #[test]
    fn a_hud_that_follows_the_cursor_stays_beside_it_and_on_screen() {
        let monitor = monitor();
        let size = (200, 50);
        let beside = follow(POINT { x: 300, y: 300 }, size, &monitor);
        assert_eq!((beside.x, beside.y), (300 + CURSOR_OFFSET, 300 + CURSOR_OFFSET));
        // Near the right and bottom edges it goes to the other side.
        let flipped = follow(POINT { x: 950, y: 780 }, size, &monitor);
        assert_eq!(flipped.x, 950 - CURSOR_OFFSET - 200);
        assert_eq!(flipped.y, 780 - CURSOR_OFFSET - 50);
        // Never off the screen, even where neither side fits.
        let corner = follow(POINT { x: 5, y: 5 }, (990, 790), &monitor);
        assert!(corner.x >= 0 && corner.y >= 0);
    }
}
