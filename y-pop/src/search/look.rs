//! What the search window looks like, shared by the window and by the picture
//! of it that `y-pop-config` shows ([`super::preview`]): the colours taken from
//! a skin, the fonts, and the bits of painting that decide how a skin shows.
//!
//! Of a [`MenuDesign`] the window reads the colours and the border, here. The
//! rest of a skin is about menu rows and has no counterpart in the window; see
//! [`crate::theme::ThemeOverrides::for_search`].

use windows::Win32::Foundation::RECT;
use y_core::Result;
use y_core::menu::MenuDesign;
use y_core::overlay::{Canvas, Color, Font};

use super::layout;

const FONT_FAMILY: &str = "Segoe UI";
/// Search glyph and the like: the icon font of Windows 10 and 11.
const GLYPH_FAMILY: &str = "Segoe MDL2 Assets";
pub const GLYPH_SEARCH: &str = "\u{E721}";
/// "More" (⋯): the button that opens the action menu.
pub const GLYPH_MORE: &str = "\u{E712}";
// Logical pixels, like the layout's.
const INPUT_FONT: i32 = 18;
const TITLE_FONT: i32 = 15;
const SMALL_FONT: i32 = 12;
const GLYPH_FONT: i32 = 16;
/// The window's corners and outline without a skin (`theme="native"`).
const CORNER_RADIUS: i32 = 8;
const BORDER: i32 = 1;
/// The selected row's inset from the window's sides, and its corner radius.
pub const ROW_MARGIN: i32 = 6;
pub const ROW_RADIUS: i32 = 6;
/// The selection highlight in the input: this much of the accent over the panel.
const ACCENT: Color = Color::rgb(0x00, 0x78, 0xD4);

/// The window's colours, from the skin in effect.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub back: Color,
    pub text: Color,
    /// Hints, second lines, headings: the skin's disabled text.
    pub dim: Color,
    pub border: Color,
    /// The rules between the input, the list and the footer, and the thumb.
    pub line: Color,
    /// Behind selected text in the input.
    pub selection: Color,
    /// The selected result row, and its text.
    pub selected_back: Color,
    pub selected_text: Color,
    /// The letters of a name that the query matched.
    pub matched: Color,
}

impl Palette {
    /// The skin's colours; without a skin (`theme="native"`), a flat panel
    /// that follows the system's light or dark setting.
    pub fn of(design: Option<&MenuDesign>) -> Self {
        let (back, text, border) = match design {
            Some(design) => (design.base.back, design.base.text, design.border.color),
            None if y_core::theme::is_dark() => (
                Color::rgb(0x20, 0x20, 0x20),
                Color::rgb(0xFF, 0xFF, 0xFF),
                Color::rgb(0x4A, 0x4A, 0x4A),
            ),
            None => (
                Color::rgb(0xF9, 0xF9, 0xF9),
                Color::rgb(0x00, 0x00, 0x00),
                Color::rgb(0xC8, 0xC8, 0xC8),
            ),
        };
        let dark = luma(back) < 128;
        let (selected_back, selected_text) = match design {
            Some(design) => (design.selected.back, design.selected.text),
            None => (blend(back, ACCENT, if dark { 0.45 } else { 0.22 }), text),
        };
        Self {
            back,
            text,
            dim: design.map_or(blend(text, back, 0.45), |design| design.disabled_text),
            border,
            line: blend(text, back, 0.85),
            selection: blend(back, ACCENT, 0.5),
            selected_back,
            selected_text,
            matched: if dark {
                Color::rgb(0x4C, 0xA3, 0xFF)
            } else {
                Color::rgb(0x00, 0x5A, 0xB4)
            },
        }
    }

    /// The name on the selected row: a little dimmer than its matched letters,
    /// which keep the full text colour.
    pub fn selected_title(&self) -> Color {
        blend(self.selected_text, self.selected_back, 0.22)
    }

    /// The second line and the hints of the selected row.
    pub fn selected_detail(&self) -> Color {
        blend(self.selected_text, self.selected_back, 0.35)
    }

    /// A button on the selected row.
    pub fn pill(&self, hovered: bool) -> Color {
        let amount = if hovered { 0.30 } else { 0.14 };
        blend(self.selected_back, self.selected_text, amount)
    }
}

/// The window's corner radius and border width, in logical pixels.
pub fn frame_of(design: Option<&MenuDesign>) -> (i32, i32) {
    design.map_or((CORNER_RADIUS, BORDER), |design| {
        (design.border.round as i32, design.border.size as i32)
    })
}

/// The border and the corners of `window`, last of all: both work on the
/// finished image.
pub fn draw_frame(canvas: &mut Canvas, window: RECT, radius: i32, border: i32, color: Color) {
    // `stroke_rect` is one pixel wide; the corners take the whole width.
    for inset in 0..border {
        canvas.stroke_rect(
            RECT {
                left: window.left + inset,
                top: window.top + inset,
                right: window.right - inset,
                bottom: window.bottom - inset,
            },
            color,
        );
    }
    if border > 0 {
        canvas.round_rect_corners_with_border(window, radius, border, color);
    } else {
        canvas.round_rect_corners(window, radius, Color::TRANSPARENT);
    }
}

/// The fill of the selected row `row`.
pub fn plate(row: layout::Rect, scale: f32) -> RECT {
    let scaled = |logical: i32| (logical as f32 * scale).round() as i32;
    RECT {
        left: row.left + scaled(ROW_MARGIN),
        top: row.top + scaled(2),
        right: row.right - scaled(ROW_MARGIN),
        bottom: row.bottom - scaled(2),
    }
}

/// The window's fonts, at `scale`.
pub struct Fonts {
    pub input: Font,
    pub title: Font,
    pub small: Font,
    pub heading: Font,
    pub glyph: Font,
}

impl Fonts {
    pub fn new(scale: f32) -> Result<Self> {
        let scaled = |logical: i32| (logical as f32 * scale).round() as i32;
        Ok(Self {
            input: Font::new(FONT_FAMILY, scaled(INPUT_FONT), false)?,
            title: Font::new(FONT_FAMILY, scaled(TITLE_FONT), false)?,
            small: Font::new(FONT_FAMILY, scaled(SMALL_FONT), false)?,
            heading: Font::new(FONT_FAMILY, scaled(SMALL_FONT - 1), true)?,
            glyph: Font::new(GLYPH_FAMILY, scaled(GLYPH_FONT), false)?,
        })
    }
}

/// `a` moved `amount` of the way toward `b` (`0.0` is `a`, `1.0` is `b`).
pub fn blend(a: Color, b: Color, amount: f32) -> Color {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color::rgb(channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b))
}

fn luma(color: Color) -> u32 {
    (color.r as u32 * 299 + color.g as u32 * 587 + color.b as u32 * 114) / 1000
}

pub fn rect(rect: layout::Rect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skins;

    #[test]
    fn a_skin_gives_the_window_its_colours() {
        let palette = Palette::of(Some(&skins::SAND));
        assert_eq!(palette.back, skins::SAND.base.back);
        assert_eq!(palette.text, skins::SAND.base.text);
        assert_eq!(palette.dim, skins::SAND.disabled_text);
        assert_eq!(palette.border, skins::SAND.border.color);
        assert_eq!(palette.selected_back, skins::SAND.selected.back);
        assert_eq!(palette.selected_text, skins::SAND.selected.text);
    }

    #[test]
    fn the_frame_comes_from_the_border_of_the_skin() {
        assert_eq!(frame_of(Some(&skins::DEFAULT)), (10, 1));
        assert_eq!(frame_of(Some(&skins::DARK)), (0, 0));
        assert_eq!(frame_of(None), (CORNER_RADIUS, BORDER));
    }

    #[test]
    fn a_border_is_as_wide_as_asked_along_its_straight_edges() {
        let mut canvas = Canvas::new((40, 40)).expect("canvas");
        canvas.clear(Color::rgb(0, 0, 0));
        let window = RECT {
            left: 0,
            top: 0,
            right: 40,
            bottom: 40,
        };
        draw_frame(&mut canvas, window, 0, 3, Color::rgb(255, 0, 0));

        let pixels = canvas.pixels_mut();
        let middle = 20 * 40;
        assert_eq!(pixels[middle + 2], 0xFFFF_0000);
        assert_eq!(pixels[middle + 3], 0xFF00_0000);
    }
}
