//! The Skin screen's pictures of a menu: the five theme cards and the live
//! preview. Each is drawn by `y_core::menu::preview`, which runs the real
//! owner-draw painter, so they match what y-pop shows (minus the system's
//! popup chrome — see that module).
//!
//! `windows-reactor`'s `Brush` has no gradients, so these can't be built
//! from controls; they are PNGs, rendered when the design they show changes
//! and kept in [`Previews`] otherwise (a view runs on every message, and
//! encoding a picture is not free).

use windows_reactor::EncodedImage;
use y_core::menu::preview::{Bitmap, PreviewRow, render_menu};
use y_core::menu::{Border, ColorPair, Fill, MenuDesign};
use y_core::overlay::Color;
use y_pop::theme::{Theme, ThemeOverrides};

use crate::skin;

/// A rendered picture and its size on screen, in DIPs (the picture is made
/// at the system DPI, so 1 pixel is `96 / dpi` DIPs).
#[derive(Clone)]
pub struct Picture {
    pub image: EncodedImage,
    pub width: f64,
    pub height: f64,
}

/// A picture and what it was drawn from, so it is drawn again only when
/// that changes. `dark` is Windows' light/dark setting, which `Auto` follows.
struct Cached<K> {
    key: K,
    picture: Option<Picture>,
}

#[derive(Default)]
pub struct Previews {
    cards: Vec<Cached<(Theme, bool)>>,
    main: Option<Cached<(Theme, ThemeOverrides, bool)>>,
}

impl Previews {
    /// The small menu on a theme's card.
    pub fn card(&mut self, theme: Theme) -> Option<Picture> {
        let key = (theme, y_core::theme::is_dark());
        if let Some(cached) = self.cards.iter().find(|cached| cached.key == key) {
            return cached.picture.clone();
        }
        let design = match theme {
            Theme::Native => native_look(),
            _ => skin::base_design(theme),
        };
        let picture = draw(design, &card_rows(), None);
        self.cards.push(Cached {
            key,
            picture: picture.clone(),
        });
        picture
    }

    /// The big menu of the preview: the theme with the overrides on top.
    /// `None` for Native with nothing overridden — Windows draws that one.
    pub fn main(&mut self, theme: Theme, overrides: ThemeOverrides) -> Option<Picture> {
        let key = (theme, overrides, y_core::theme::is_dark());
        if let Some(cached) = &self.main
            && cached.key == key
        {
            return cached.picture.clone();
        }
        let picture = skin::resolved(theme, overrides).and_then(|design| {
            draw(
                design,
                &sample_rows(),
                Some((SUBMENU_PARENT, &submenu_rows())),
            )
        });
        self.main = Some(Cached {
            key,
            picture: picture.clone(),
        });
        picture
    }
}

/// Which row of [`sample_rows`] opens the submenu.
const SUBMENU_PARENT: usize = 2;

/// A native menu is drawn by Windows, so there is no design to paint it
/// with: this approximates one (black on white, the default accent for the
/// hover) for the Native card, which says as much in its caption.
fn native_look() -> MenuDesign {
    MenuDesign {
        base: ColorPair {
            text: Color::rgb(0, 0, 0),
            back: Color::rgb(0xff, 0xff, 0xff),
        },
        selected: ColorPair {
            text: Color::rgb(0xff, 0xff, 0xff),
            back: Color::rgb(0x00, 0x78, 0xd7),
        },
        disabled_text: Color::rgb(0x6d, 0x6d, 0x6d),
        border: Border {
            size: 0,
            round: 0,
            color: Color::rgb(0, 0, 0),
        },
        fill: Fill::Solid,
        padding: 0,
        margin: 0,
        edge: None,
        icon_backdrop: None,
    }
}

fn card_rows() -> [PreviewRow<'static>; 4] {
    [
        PreviewRow::item("Notepad"),
        PreviewRow::item("Calculator").hovered(),
        PreviewRow::separator(""),
        PreviewRow::item("Quit"),
    ]
}

/// A fixed sample, not the user's own first menu, so that every override has
/// something to show on it: a hovered row, a submenu, a titled separator, a
/// disabled row, a shortcut hint. (Departure from the plan's wording, which
/// drew the first tray menu: a menu with no disabled row could not show
/// `disabled-text-color` at all.)
fn sample_rows() -> [PreviewRow<'static>; 8] {
    [
        PreviewRow::item("Notepad").shortcut("Win+N"),
        PreviewRow::item("Calculator"),
        PreviewRow::item("Paint").submenu().hovered(),
        PreviewRow::separator("Tools"),
        PreviewRow::item("Command prompt"),
        PreviewRow::item("Not available").disabled(),
        PreviewRow::separator(""),
        PreviewRow::item("Quit"),
    ]
}

fn submenu_rows() -> [PreviewRow<'static>; 3] {
    [
        PreviewRow::item("Sketch pad"),
        PreviewRow::item("Photo editor").hovered(),
        PreviewRow::item("Viewer"),
    ]
}

/// How far the submenu overlaps its parent, in pixels.
const CASCADE_OVERLAP: i32 = 4;

/// `rows` as a menu, with `submenu` (the index of the row it opens from, and
/// its rows) drawn beside it, as a PNG.
fn draw(
    design: MenuDesign,
    rows: &[PreviewRow<'_>],
    submenu: Option<(usize, &[PreviewRow<'_>])>,
) -> Option<Picture> {
    let menu = render_menu(design, rows)?;
    let mut canvas = menu.image.clone();

    if let Some((parent, sub_rows)) = submenu
        && let Some(sub) = render_menu(design, sub_rows)
    {
        let x = menu.image.width as i32 - CASCADE_OVERLAP;
        let y = menu.row_tops.get(parent).copied().unwrap_or(0);
        let mut wide = Bitmap::blank(
            (x + sub.image.width as i32) as u32,
            menu.image.height.max((y + sub.image.height as i32) as u32),
        );
        wide.blit(&menu.image, 0, 0);
        wide.blit(&sub.image, x, y);
        canvas = wide;
    }

    let png = y_core::png::encode_rgba(canvas.width, canvas.height, &canvas.rgba);
    if png.is_empty() {
        return None;
    }
    let scale = 96.0 / f64::from(dpi());
    Some(Picture {
        image: EncodedImage::new(png),
        width: f64::from(canvas.width) * scale,
        height: f64::from(canvas.height) * scale,
    })
}

fn dpi() -> u32 {
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
    if dpi == 0 { 96 } else { dpi }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_with_nothing_overridden_has_no_main_preview() {
        let mut previews = Previews::default();
        assert!(
            previews
                .main(Theme::Native, ThemeOverrides::default())
                .is_none()
        );
    }

    #[test]
    fn a_themed_preview_is_drawn_and_reused_until_its_design_changes() {
        let mut previews = Previews::default();
        let first = previews
            .main(Theme::Dark, ThemeOverrides::default())
            .expect("drawn");
        assert!(first.width > 0.0 && first.height > 0.0);
        assert!(previews.main.is_some());

        let overrides = ThemeOverrides {
            padding: Some(3),
            ..ThemeOverrides::default()
        };
        let taller = previews.main(Theme::Dark, overrides).expect("drawn");
        assert!(
            taller.height > first.height,
            "row gaps make the menu taller"
        );
    }

    #[test]
    fn every_theme_has_a_card() {
        let mut previews = Previews::default();
        for theme in [
            Theme::Native,
            Theme::Default,
            Theme::Flat,
            Theme::Dark,
            Theme::Auto,
        ] {
            assert!(previews.card(theme).is_some(), "{theme:?}");
        }
        assert_eq!(previews.cards.len(), 5);
        previews.card(Theme::Flat);
        assert_eq!(previews.cards.len(), 5, "drawn once");
    }
}
