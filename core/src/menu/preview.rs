//! A themed menu drawn into memory instead of onto a popup, for a settings
//! app that wants to *show* a [`MenuDesign`] — the picture is made by the
//! very code that paints the real menu ([`handle_measure_item`] and
//! [`handle_draw_item`], fed the same structures Windows would send), so it
//! cannot drift from what y-pop shows.
//!
//! **Simplifications.** There is no popup window, so none of the system's
//! chrome: the menu gets a one-pixel frame in a colour between its text and
//! its background, and no shadow or Windows 11 corner rounding. Rows are
//! measured and drawn at the system DPI. Icons are not drawn (the icon column
//! is reserved, as in a real menu, and empty).

use std::ffi::c_void;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
    CreateFontIndirectW, CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject, FrameRect,
    GdiFlush, HGDIOBJ, SelectClipRgn, SelectObject,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODA_DRAWENTIRE, ODS_DISABLED, ODS_FLAGS, ODS_GRAYED,
    ODS_SELECTED, ODT_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SystemParametersInfoW,
};

use super::{ItemData, ItemKind, MenuDesign, handle_draw_item, handle_measure_item};
use crate::overlay::Color;

/// What a [`PreviewRow`] is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    Item,
    /// A rule; its `text` is the title, empty for a plain one.
    Separator,
}

/// One row of the menu to draw.
#[derive(Clone, Copy, Debug)]
pub struct PreviewRow<'a> {
    pub text: &'a str,
    pub shortcut: Option<&'a str>,
    pub kind: RowKind,
    pub submenu: bool,
    pub disabled: bool,
    /// Drawn as the row under the pointer.
    pub hovered: bool,
}

impl<'a> PreviewRow<'a> {
    pub fn item(text: &'a str) -> Self {
        Self {
            text,
            shortcut: None,
            kind: RowKind::Item,
            submenu: false,
            disabled: false,
            hovered: false,
        }
    }

    pub fn separator(title: &'a str) -> Self {
        Self {
            kind: RowKind::Separator,
            ..Self::item(title)
        }
    }

    pub fn shortcut(self, shortcut: &'a str) -> Self {
        Self {
            shortcut: Some(shortcut),
            ..self
        }
    }

    pub fn submenu(self) -> Self {
        Self {
            submenu: true,
            ..self
        }
    }

    pub fn disabled(self) -> Self {
        Self {
            disabled: true,
            ..self
        }
    }

    pub fn hovered(self) -> Self {
        Self {
            hovered: true,
            ..self
        }
    }
}

/// Straight-alpha RGBA pixels, top row first.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Bitmap {
    /// Fully transparent.
    pub fn blank(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    /// Copies `other` over this bitmap with its top-left corner at `(x, y)`;
    /// what falls outside is dropped. No blending: pixels are replaced.
    pub fn blit(&mut self, other: &Bitmap, x: i32, y: i32) {
        for row in 0..other.height as i32 {
            let target_y = y + row;
            if target_y < 0 || target_y >= self.height as i32 {
                continue;
            }
            let first = (-x).max(0);
            let last = (self.width as i32 - x).min(other.width as i32);
            if first >= last {
                continue;
            }
            let from = (row as usize * other.width as usize + first as usize) * 4;
            let to = (target_y as usize * self.width as usize + (x + first) as usize) * 4;
            let count = (last - first) as usize * 4;
            self.rgba[to..to + count].copy_from_slice(&other.rgba[from..from + count]);
        }
    }
}

/// A drawn menu and where its rows are.
pub struct RenderedMenu {
    pub image: Bitmap,
    /// The top edge of each row, in the image's pixels, in row order.
    pub row_tops: Vec<i32>,
}

/// Draws `rows` as a menu painted with `design`. `None` if there are no rows
/// or Windows would not give a bitmap.
pub fn render_menu(design: MenuDesign, rows: &[PreviewRow<'_>]) -> Option<RenderedMenu> {
    if rows.is_empty() {
        return None;
    }

    let data: Vec<Box<ItemData>> = rows
        .iter()
        .map(|row| {
            Box::new(ItemData {
                text: row.text.encode_utf16().collect(),
                kind: match row.kind {
                    RowKind::Item => ItemKind::Normal,
                    RowKind::Separator => ItemKind::Separator,
                },
                design,
                bitmap: None,
                submenu: row.submenu,
                shortcut: row
                    .shortcut
                    .filter(|shortcut| !shortcut.is_empty())
                    .map(|shortcut| shortcut.encode_utf16().collect()),
            })
        })
        .collect();

    // What Windows would send: a menu is as wide as its widest row.
    let sizes: Vec<(i32, i32)> = data
        .iter()
        .map(|data| measure(data))
        .collect::<Option<_>>()?;
    let inner_width = sizes.iter().map(|size| size.0).max()?;
    let inner_height: i32 = sizes.iter().map(|size| size.1).sum();
    let (width, height) = (inner_width + 2, inner_height + 2);

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative: top-down, so row 0 is the top one.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let mut pixels: *mut c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0)
        else {
            let _ = DeleteDC(dc);
            return None;
        };
        if pixels.is_null() {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            return None;
        }
        let previous_bitmap = SelectObject(dc, bitmap.into());
        let font = menu_font();
        let previous_font = font.map(|font| SelectObject(dc, font.into()));

        // The popup's own background, which shows between and around rows.
        let whole = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let brush = CreateSolidBrush(design.base.back.colorref());
        windows::Win32::Graphics::Gdi::FillRect(dc, &whole, brush);
        let _ = DeleteObject(brush.into());

        let mut row_tops = Vec::with_capacity(rows.len());
        let mut top = 1;
        for ((row, data), (_, row_height)) in rows.iter().zip(&data).zip(&sizes) {
            row_tops.push(top);
            let mut state = 0;
            if row.disabled {
                state |= ODS_DISABLED.0 | ODS_GRAYED.0;
            }
            if row.hovered {
                state |= ODS_SELECTED.0;
            }
            let item = DRAWITEMSTRUCT {
                CtlType: ODT_MENU,
                itemAction: ODA_DRAWENTIRE,
                itemState: ODS_FLAGS(state),
                hDC: dc,
                rcItem: RECT {
                    left: 1,
                    top,
                    right: 1 + inner_width,
                    bottom: top + row_height,
                },
                itemData: &raw const **data as usize,
                ..Default::default()
            };
            handle_draw_item(LPARAM(&raw const item as isize));
            // A submenu row excludes its arrow column from the clip so the
            // system's own arrow is not drawn over it; a fresh row starts clean.
            SelectClipRgn(dc, None);
            top += row_height;
        }

        // The frame a popup would have.
        let frame_color = blend(design.base.text, design.base.back);
        let frame = CreateSolidBrush(frame_color.colorref());
        FrameRect(dc, &whole, frame);
        let _ = DeleteObject(frame.into());
        let _ = GdiFlush();

        // GDI leaves the alpha byte at 0 and the order as B, G, R.
        let bgra =
            std::slice::from_raw_parts(pixels as *const u8, width as usize * height as usize * 4);
        let rgba = bgra
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], 255])
            .collect();

        if let Some(previous) = previous_font {
            SelectObject(dc, previous);
        }
        if let Some(font) = font {
            let _ = DeleteObject(HGDIOBJ::from(font));
        }
        SelectObject(dc, previous_bitmap);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);

        Some(RenderedMenu {
            image: Bitmap {
                width: width as u32,
                height: height as u32,
                rgba,
            },
            row_tops,
        })
    }
}

/// The size `WM_MEASUREITEM` would settle on for one row.
fn measure(data: &ItemData) -> Option<(i32, i32)> {
    let mut item = MEASUREITEMSTRUCT {
        CtlType: ODT_MENU,
        itemData: data as *const ItemData as usize,
        ..Default::default()
    };
    handle_measure_item(LPARAM(&raw mut item as isize))
        .then_some((item.itemWidth as i32, item.itemHeight as i32))
}

/// The font the system draws menus in, if it will say.
fn menu_font() -> Option<windows::Win32::Graphics::Gdi::HFONT> {
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some((&raw mut metrics).cast()),
            Default::default(),
        )
        .ok()?;
        let font = CreateFontIndirectW(&metrics.lfMenuFont);
        (!font.is_invalid()).then_some(font)
    }
}

/// A colour a quarter of the way from `back` towards `text`: a frame that is
/// visible on any theme without shouting.
fn blend(text: Color, back: Color) -> Color {
    let mix = |text: u8, back: u8| ((u16::from(text) + u16::from(back) * 3) / 4) as u8;
    Color::rgb(
        mix(text.r, back.r),
        mix(text.g, back.g),
        mix(text.b, back.b),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{
        Border, ColorPair, EdgeRule, Fill, Gradient, GradientDirection, SeparatorSpan,
    };

    fn design() -> MenuDesign {
        MenuDesign {
            base: ColorPair {
                text: Color::rgb(0, 0, 0),
                back: Color::rgb(255, 255, 255),
            },
            selected: ColorPair {
                text: Color::rgb(255, 255, 255),
                back: Color::rgb(0, 0, 255),
            },
            disabled_text: Color::rgb(128, 128, 128),
            border: Border {
                size: 0,
                round: 0,
                color: Color::rgb(0, 0, 0),
            },
            fill: Fill::Solid,
            padding: 0,
            margin: 0,
            edge: None,
            separator: SeparatorSpan::AfterIcons,
            icon_backdrop: None,
        }
    }

    fn pixel(menu: &RenderedMenu, x: u32, y: u32) -> [u8; 4] {
        let at = (y * menu.image.width + x) as usize * 4;
        menu.image.rgba[at..at + 4].try_into().expect("four bytes")
    }

    #[test]
    fn no_rows_draw_nothing() {
        assert!(render_menu(design(), &[]).is_none());
    }

    #[test]
    fn the_image_is_as_tall_as_its_rows_plus_a_frame() {
        let menu = render_menu(
            design(),
            &[PreviewRow::item("One"), PreviewRow::item("Two")],
        )
        .expect("drawn");
        assert_eq!(menu.row_tops.len(), 2);
        assert_eq!(menu.row_tops[0], 1);
        assert!(menu.row_tops[1] > menu.row_tops[0]);
        assert!(menu.image.height as i32 > menu.row_tops[1]);
        assert_eq!(
            menu.image.rgba.len(),
            menu.image.width as usize * menu.image.height as usize * 4
        );
    }

    #[test]
    fn a_hovered_row_is_painted_in_the_selected_colour_and_the_others_are_not() {
        let menu = render_menu(
            design(),
            &[PreviewRow::item("One"), PreviewRow::item("Two").hovered()],
        )
        .expect("drawn");
        // Far right of the row: no text there, only the row's fill.
        let x = menu.image.width - 3;
        let idle = pixel(&menu, x, menu.row_tops[0] as u32 + 2);
        let hovered = pixel(&menu, x, menu.row_tops[1] as u32 + 2);
        assert_eq!(idle, [255, 255, 255, 255]);
        assert_eq!(hovered, [0, 0, 255, 255]);
    }

    /// A menu with a separator as its second row, and the leftmost x inside
    /// the frame where that row has anything but the white of the menu.
    fn separator_start(design: MenuDesign, title: &str) -> u32 {
        let menu = render_menu(
            design,
            &[
                PreviewRow::item("A name long enough for any title"),
                PreviewRow::separator(title),
                PreviewRow::item("Two"),
            ],
        )
        .expect("drawn");
        let rows = menu.row_tops[1] as u32..menu.row_tops[2] as u32;
        (2..menu.image.width - 2)
            .find(|&x| {
                rows.clone()
                    .any(|y| pixel(&menu, x, y) != [255, 255, 255, 255])
            })
            .expect("a line")
    }

    #[test]
    fn an_untitled_separator_starts_past_the_icon_column_and_a_titled_one_does_not() {
        let untitled = separator_start(design(), "");
        // The frame, then 8 + 16 + 8: the rule's place, and one more.
        assert_eq!(untitled, 1 + 32 + 1);
        assert!(separator_start(design(), "T") < untitled);
    }

    #[test]
    fn the_line_follows_the_column_of_rows_a_skin_pushes_right() {
        let mut pushed = design();
        pushed.padding = 3;
        pushed.margin = 2;
        pushed.border.size = 1;
        assert_eq!(separator_start(pushed, ""), separator_start(design(), "") + 6);
    }

    #[test]
    fn the_icon_columns_rule_and_backdrop_run_through_an_untitled_separator() {
        let mut ruled = design();
        ruled.edge = Some(EdgeRule::Always);
        assert_eq!(separator_start(ruled, ""), separator_start(design(), "") - 1);

        let mut backed = design();
        let red = Color::rgb(255, 0, 0);
        backed.icon_backdrop = Some(Gradient {
            direction: GradientDirection::Horizontal,
            start: red,
            end: red,
        });
        assert_eq!(separator_start(backed, ""), 2, "from the row's left edge");
        assert!(separator_start(backed, "T") > 2, "a title spans the row");
    }

    #[test]
    fn a_full_separator_crosses_the_icon_column_and_stops_it() {
        let mut full = design();
        full.separator = SeparatorSpan::Full;
        let before = separator_start(full, "");
        assert!(before < separator_start(design(), ""));

        full.edge = Some(EdgeRule::Always);
        assert_eq!(separator_start(full, ""), before, "no rule through it");
        let red = Color::rgb(255, 0, 0);
        full.icon_backdrop = Some(Gradient {
            direction: GradientDirection::Horizontal,
            start: red,
            end: red,
        });
        assert_eq!(separator_start(full, ""), before, "no backdrop either");
    }

    #[test]
    fn blit_replaces_pixels_and_clips_to_the_target() {
        let mut target = Bitmap::blank(4, 4);
        let source = Bitmap {
            width: 2,
            height: 2,
            rgba: vec![9; 16],
        };
        target.blit(&source, 3, 3);
        target.blit(&source, -1, -1);
        let at = |x: usize, y: usize| target.rgba[(y * 4 + x) * 4];
        assert_eq!(at(3, 3), 9);
        assert_eq!(at(0, 0), 9);
        assert_eq!(at(1, 1), 0);
        assert_eq!(at(2, 2), 0);
    }
}
