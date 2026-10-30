//! A picture of the search window, for a settings app that wants to *show* a
//! skin on it. Same layout, colours, fonts and frame as the window
//! ([`super::layout`], [`super::look`]), so a skin shows as it will there.
//!
//! **Simplification.** The sample is fixed and only what a skin changes is
//! drawn: the window's real painter needs an index, a state and the shell's
//! icons, so the rows are painted here in the same order instead. No icons
//! (their column is reserved and empty), and no window shadow.

use windows::Win32::Foundation::RECT;
use y_core::Result;
use y_core::menu::MenuDesign;
use y_core::menu::preview::Bitmap;
use y_core::overlay::{Canvas, TextAlign};

use super::display;
use super::layout::{self, Layout, RowKind, Spec};
use super::look::{self, Fonts, GLYPH_MORE, GLYPH_SEARCH, Palette, ROW_RADIUS, rect};

const TYPED: &str = "pa";
const HINTS: &str = "↑↓  Move      ↵  Open      Alt+Enter  Actions      Esc  Back";
const BUTTONS: [&str; 2] = ["Reveal", "Edit"];

/// How much of the window to draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sample {
    /// The input and two rows, for a theme's card.
    Card,
    /// A window of results: headings, a selected row, matched letters, the footer.
    Window,
}

/// One row of the sample.
#[derive(Clone, Copy)]
enum SampleRow {
    Heading(&'static str),
    Item {
        name: &'static str,
        /// Where [`TYPED`] is found in `name`.
        matched: usize,
        detail: &'static str,
        hotkey: Option<&'static str>,
    },
}

const fn item(
    name: &'static str,
    matched: usize,
    detail: &'static str,
    hotkey: Option<&'static str>,
) -> SampleRow {
    SampleRow::Item {
        name,
        matched,
        detail,
        hotkey,
    }
}

const CARD_ROWS: [SampleRow; 2] = [
    item("Paint", 0, "Graphics", None),
    item("Compare files", 3, "Tools", None),
];

const WINDOW_ROWS: [SampleRow; 5] = [
    SampleRow::Heading("TOP HIT"),
    item("Paint", 0, "Graphics › Paint", Some("Win+P")),
    SampleRow::Heading("APPS"),
    item("Paint 3D", 0, "Graphics › Paint 3D", None),
    item("Compare files", 3, "Tools › Compare files", None),
];

/// Draws the window painted with `design` (`None`: `theme="native"`) at
/// `scale`, the monitor's. `None` if Windows would not give a surface.
pub fn render(design: Option<MenuDesign>, scale: f32, sample: Sample) -> Option<Bitmap> {
    let (rows, footer): (&[SampleRow], bool) = match sample {
        Sample::Card => (&CARD_ROWS, false),
        Sample::Window => (&WINDOW_ROWS, true),
    };
    let kinds: Vec<RowKind> = rows
        .iter()
        .map(|row| match row {
            SampleRow::Heading(_) => RowKind::Header,
            SampleRow::Item { .. } => RowKind::Item,
        })
        .collect();
    let spec = Spec {
        width: layout::width_for(0, scale),
        scale,
        // Tall enough for every row: this list never scrolls.
        max_list_height: i32::MAX / 2,
        footer,
        band: false,
    };
    let fonts = Fonts::new(scale).ok()?;
    let selected = rows
        .iter()
        .position(|row| matches!(row, SampleRow::Item { .. }))?;
    let widths = BUTTONS.map(|label| fonts.small.measure(label).0);
    let layout = Layout::new(&spec, &kinds, 0).with_row_buttons(selected, &widths);

    let palette = Palette::of(design.as_ref());
    let mut canvas = Canvas::new(layout.size).ok()?;
    let painter = Painter {
        palette,
        fonts: &fonts,
        layout: &layout,
        scale,
        selected,
    };
    painter.paint(&mut canvas, rows).ok()?;

    let scaled = |logical: i32| (logical as f32 * scale).round() as i32;
    let (radius, border) = look::frame_of(design.as_ref());
    let window = RECT {
        left: 0,
        top: 0,
        right: layout.size.0,
        bottom: layout.size.1,
    };
    let (radius, border) = (scaled(radius), scaled(border));
    look::draw_frame(&mut canvas, window, radius, border, palette.border);
    Some(bitmap(&mut canvas))
}

struct Painter<'a> {
    palette: Palette,
    fonts: &'a Fonts,
    layout: &'a Layout,
    scale: f32,
    selected: usize,
}

impl Painter<'_> {
    fn scaled(&self, logical: i32) -> i32 {
        (logical as f32 * self.scale).round() as i32
    }

    fn paint(&self, canvas: &mut Canvas, rows: &[SampleRow]) -> Result<()> {
        let palette = self.palette;
        let layout = self.layout;
        canvas.clear(palette.back);

        let parts = layout.input_parts();
        let padding = parts.icon.left;
        let field = rect(parts.field);
        canvas.draw_text(
            GLYPH_SEARCH,
            rect(parts.icon),
            &self.fonts.glyph,
            palette.dim,
            TextAlign::Centre,
        )?;
        canvas.draw_text(
            TYPED,
            field,
            &self.fonts.input,
            palette.text,
            TextAlign::Left,
        )?;
        let (typed_width, height) = self.fonts.input.measure(TYPED);
        canvas.draw_caret(
            field.left + typed_width,
            field.top + (field.bottom - field.top - height) / 2,
            height,
            self.scaled(1).max(2),
            palette.text,
        );

        if let Some(list) = layout.list {
            let rule = RECT {
                top: list.top - 1,
                bottom: list.top,
                ..rect(list)
            };
            canvas.fill_rect(rule, palette.line);
        }

        for placed in &layout.rows {
            match &rows[placed.index] {
                SampleRow::Heading(text) => canvas.draw_text_ellipsis(
                    text,
                    rect(placed.rect.inset(padding, 0)),
                    &self.fonts.heading,
                    palette.dim,
                    TextAlign::Left,
                )?,
                SampleRow::Item {
                    name,
                    matched,
                    detail,
                    hotkey,
                } => self.item(canvas, placed, name, *matched, detail, *hotkey)?,
            }
        }

        if let Some(footer) = layout.footer {
            let rule = RECT {
                bottom: footer.top + 1,
                ..rect(footer)
            };
            canvas.fill_rect(rule, palette.line);
            canvas.draw_text_ellipsis(
                HINTS,
                rect(footer.inset(padding, 0)),
                &self.fonts.small,
                palette.dim,
                TextAlign::Left,
            )?;
        }
        Ok(())
    }

    fn item(
        &self,
        canvas: &mut Canvas,
        placed: &layout::Placed,
        name: &str,
        matched: usize,
        detail: &str,
        hotkey: Option<&str>,
    ) -> Result<()> {
        let palette = self.palette;
        let selected = placed.index == self.selected;
        let row = placed.rect;
        let parts = self.layout.item_parts(row);
        let (title, emphasis, detail_color) = if selected {
            (
                palette.selected_title(),
                palette.selected_text,
                palette.selected_detail(),
            )
        } else {
            (palette.text, palette.matched, palette.dim)
        };
        let plate = look::plate(row, self.scale);
        if selected {
            canvas.fill_rect(plate, palette.selected_back);
        }

        let buttons = self.layout.buttons.as_ref().filter(|_| selected);
        let mut right = parts.title.right;
        if let Some(buttons) = buttons {
            right = right.min(buttons.left - self.scaled(12));
        }
        if let Some(hotkey) = hotkey {
            let (width, _) = self.fonts.small.measure(hotkey);
            canvas.draw_text(
                hotkey,
                RECT {
                    left: right - width,
                    right,
                    ..rect(row)
                },
                &self.fonts.small,
                detail_color,
                TextAlign::Right,
            )?;
            right -= width + self.scaled(12);
        }

        let title_box = RECT {
            right,
            ..rect(parts.title)
        };
        let offsets = self.fonts.title.caret_offsets(name);
        let positions: Vec<usize> = (matched..matched + TYPED.len()).collect();
        for run in display::runs(name, &positions) {
            canvas.draw_text(
                &name[run.units.clone()],
                RECT {
                    left: title_box.left + offsets[run.units.start],
                    ..title_box
                },
                &self.fonts.title,
                if run.matched { emphasis } else { title },
                TextAlign::Left,
            )?;
        }
        canvas.draw_text_ellipsis(
            detail,
            rect(parts.subtitle),
            &self.fonts.small,
            detail_color,
            TextAlign::Left,
        )?;

        if let Some(buttons) = buttons {
            let radius = self.scaled(4);
            for (cell, label) in buttons.items.iter().zip(BUTTONS) {
                let cell = rect(*cell);
                canvas.fill_rect(cell, palette.pill(false));
                canvas.draw_text(
                    label,
                    cell,
                    &self.fonts.small,
                    palette.selected_text,
                    TextAlign::Centre,
                )?;
                canvas.round_rect_corners(cell, radius, palette.selected_back);
            }
            let cell = rect(buttons.more);
            canvas.fill_rect(cell, palette.pill(false));
            canvas.draw_text(
                GLYPH_MORE,
                cell,
                &self.fonts.glyph,
                palette.selected_text,
                TextAlign::Centre,
            )?;
            canvas.round_rect_corners(cell, radius, palette.selected_back);
        }

        if selected {
            canvas.round_rect_corners(plate, self.scaled(ROW_RADIUS), palette.back);
        }
        Ok(())
    }
}

/// The canvas's premultiplied `0xAARRGGBB` words as straight-alpha RGBA.
fn bitmap(canvas: &mut Canvas) -> Bitmap {
    let (width, height) = canvas.size();
    let straight = |channel: u32, alpha: u32| ((channel * 255 + alpha / 2) / alpha).min(255) as u8;
    let rgba = canvas
        .pixels_mut()
        .iter()
        .flat_map(|&pixel| {
            let alpha = pixel >> 24;
            if alpha == 0 {
                return [0; 4];
            }
            [
                straight((pixel >> 16) & 0xFF, alpha),
                straight((pixel >> 8) & 0xFF, alpha),
                straight(pixel & 0xFF, alpha),
                alpha as u8,
            ]
        })
        .collect();
    Bitmap {
        width: width as u32,
        height: height as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skins;
    use crate::theme::{BorderOverride, Theme, ThemeOverrides};
    use y_core::overlay::Color;

    /// The pixel at `(x, y)`, as `[r, g, b, a]`.
    fn pixel(image: &Bitmap, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * image.width + x) * 4) as usize;
        [
            image.rgba[at],
            image.rgba[at + 1],
            image.rgba[at + 2],
            image.rgba[at + 3],
        ]
    }

    fn window(design: Option<MenuDesign>) -> Bitmap {
        render(design, 1.0, Sample::Window).expect("drawn")
    }

    #[test]
    fn the_panel_is_the_skins_background() {
        let image = window(Some(skins::MIDNIGHT));
        let [r, g, b, a] = pixel(&image, image.width / 2, 30);
        let back = skins::MIDNIGHT.base.back;
        assert_eq!([r, g, b, a], [back.r, back.g, back.b, 255]);
    }

    #[test]
    fn a_window_is_taller_than_a_card() {
        let card = render(Some(skins::FLAT), 1.0, Sample::Card).expect("drawn");
        let window = window(Some(skins::FLAT));
        assert_eq!(card.width, window.width);
        assert!(window.height > card.height);
    }

    #[test]
    fn native_has_a_picture_too() {
        let image = window(None);
        assert!(image.width > 0 && image.height > 0);
    }

    #[test]
    fn it_is_sized_for_the_scale() {
        let small = render(Some(skins::FLAT), 1.0, Sample::Card).expect("drawn");
        let big = render(Some(skins::FLAT), 2.0, Sample::Card).expect("drawn");
        assert_eq!(big.width, small.width * 2);
    }

    #[test]
    fn a_rounded_window_has_see_through_corners_and_a_square_one_has_none() {
        assert_eq!(pixel(&window(Some(skins::DEFAULT)), 0, 0)[3], 0);
        assert_eq!(pixel(&window(Some(skins::DARK)), 0, 0)[3], 255);
    }

    #[test]
    fn the_border_override_shows() {
        let color = Color::rgb(0xFF, 0x00, 0x80);
        let overrides = ThemeOverrides {
            border: Some(BorderOverride {
                size: 3,
                round: 0,
                color,
            }),
            ..ThemeOverrides::default()
        };
        let design = skins::resolve_design(Theme::Dark, overrides);
        let image = window(design);
        let inside = pixel(&image, 2, image.height / 2);
        assert_eq!(inside, [color.r, color.g, color.b, 255]);
    }

    #[test]
    fn the_selected_row_uses_the_hover_colours() {
        let overrides = ThemeOverrides {
            hover_back: Some(Color::rgb(0xFF, 0x00, 0x80)),
            ..ThemeOverrides::default()
        };
        let image = window(skins::resolve_design(Theme::Flat, overrides));
        let found = image
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel == [0xFF, 0x00, 0x80, 0xFF]);
        assert!(found);
    }
}
