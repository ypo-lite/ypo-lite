//! Text through DirectWrite/Direct2D, for the `color-text` feature: the
//! colour-font (emoji) counterpart of [`Canvas::draw_text`]'s GDI.
//!
//! # Rendered once
//!
//! GDI cannot draw a colour font, so something else has to, but only once:
//! [`Canvas::draw_text_color`] renders a text the first time it is asked for
//! and keeps the pixels, and every later call is a [`Canvas::blit`] of them.
//! The cache costs next to nothing (a few KB per emoji) against what Direct2D
//! and DirectWrite hold while they are live (14 MB committed for 15 emoji,
//! measured, of which [`release`] gives back 8), so an application calls
//! [`release`] whenever it is done painting: nothing here holds a Direct2D or
//! DirectWrite object between two renders, and a font keeps only its
//! description. An overlay reopened with nothing new to draw never loads them
//! again, hover repaints included.
//!
//! The 6 MB left after [`release`] is what those libraries keep for as long
//! as they are loaded, which is out of our hands inside this process.

use std::cell::RefCell;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1DCRenderTarget, ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{HSTRING, PCWSTR, Result, w};

use super::{Canvas, Color, TextAlign};

impl Canvas {
    /// Draws one line of text with DirectWrite, colour fonts included — the
    /// counterpart of [`Canvas::draw_text`] for text that may hold emoji,
    /// which GDI can only draw as a monochrome outline.
    ///
    /// The result is blended in with real alpha, so unlike the GDI path there
    /// is nothing to repair afterwards: `color` may be translucent, the area
    /// under the text need not be uniformly opaque, and the call may happen
    /// in any order relative to the other drawing (corner rounding still goes
    /// last, as it erases pixels). Text is grayscale anti-aliased, since
    /// ClearType needs an opaque backdrop.
    ///
    /// See the module doc: only the first call for a given text, font, colour
    /// and rectangle size loads Direct2D.
    pub fn draw_text_color(
        &mut self,
        text: &str,
        rect: RECT,
        font: &ColorFont,
        color: Color,
        align: TextAlign,
    ) -> Result<()> {
        let size = (rect.right - rect.left, rect.bottom - rect.top);
        if size.0 <= 0 || size.1 <= 0 {
            return Ok(());
        }

        GLYPHS.with_borrow_mut(|glyphs| {
            let found = glyphs
                .iter()
                .position(|glyph| glyph.is(text, font, color, size, align));
            let index = match found {
                Some(index) => index,
                None => {
                    glyphs.push(Glyph::render(text, font, color, size, align)?);
                    glyphs.len() - 1
                }
            };

            let glyph = &glyphs[index];
            self.blit(
                rect.left + glyph.left,
                rect.top + glyph.top,
                glyph.width,
                glyph.height,
                &glyph.pixels,
            );
            Ok(())
        })
    }

    /// Draws straight through Direct2D, filling a surface `size` big. Only
    /// [`Glyph::render`] calls this, on its own short-lived canvas.
    fn render_text(
        &mut self,
        text: &str,
        size: (i32, i32),
        font: &ColorFont,
        color: Color,
        align: TextAlign,
    ) -> Result<()> {
        let factories = factories()?;
        let target = self.render_target(&factories)?;
        let format = font.format(&factories.dwrite)?;
        let text: Vec<u16> = text.encode_utf16().collect();

        let layout = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: size.0 as f32,
            bottom: size.1 as f32,
        };
        let brush_color = D2D1_COLOR_F {
            r: color.r as f32 / 255.0,
            g: color.g as f32 / 255.0,
            b: color.b as f32 / 255.0,
            a: color.a as f32 / 255.0,
        };

        unsafe {
            format.SetTextAlignment(match align {
                TextAlign::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Centre => DWRITE_TEXT_ALIGNMENT_CENTER,
                TextAlign::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            let brush = target.CreateSolidColorBrush(&brush_color, None)?;

            target.BeginDraw();
            target.DrawText(
                &text,
                &format,
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            target.EndDraw(None, None)
        }
    }

    /// A Direct2D render target bound to this canvas's DC.
    fn render_target(&self, factories: &Factories) -> Result<ID2D1DCRenderTarget> {
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            // Not DEFAULT: that picks the GPU when there is one, loading
            // Direct3D and the whole graphics driver (130+ MB of DLLs on an
            // Intel GPU) to draw a few small tiles. Measured over 16 tiles of
            // one emoji, that keeps 39 MB committed, 19 MB of it even once
            // the factories are gone. Software keeps 8 MB and still draws
            // colour fonts.
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            // 96 so that one DIP is one pixel: a `ColorFont`'s size is then
            // in physical pixels, like a `Font`'s.
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };

        unsafe {
            let target = factories.d2d.CreateDCRenderTarget(&properties)?;
            let bounds = RECT {
                left: 0,
                top: 0,
                right: self.size.0,
                bottom: self.size.1,
            };
            target.BindDC(self.dc, &bounds)?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Ok(target)
        }
    }
}

/// One rendered text, cropped to the pixels it actually covers: a tile's
/// rectangle is mostly empty, and this stays for as long as the process does.
///
/// `color` belongs to the key even though a colour emoji ignores it: a
/// monochrome glyph (all of `Segoe Fluent Icons`, some of `Segoe UI Emoji`)
/// is drawn in it, and a tile changes colour on hover. `size` and `align`
/// belong to it because they decide where the text lands.
struct Glyph {
    text: String,
    font: ColorFont,
    color: Color,
    size: (i32, i32),
    align: TextAlign,
    /// Offset of [`pixels`](Self::pixels) inside the rectangle.
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    pixels: Vec<u32>,
}

impl Glyph {
    fn is(
        &self,
        text: &str,
        font: &ColorFont,
        color: Color,
        size: (i32, i32),
        align: TextAlign,
    ) -> bool {
        self.text == text
            && self.font == *font
            && self.color == color
            && self.size == size
            && self.align == align
    }

    fn render(
        text: &str,
        font: &ColorFont,
        color: Color,
        size: (i32, i32),
        align: TextAlign,
    ) -> Result<Self> {
        let mut canvas = Canvas::new(size)?;
        canvas.render_text(text, size, font, color, align)?;

        let (left, top, width, height, pixels) = crop(canvas.pixels_mut(), size);
        Ok(Self {
            text: text.to_owned(),
            font: font.clone(),
            color,
            size,
            align,
            left,
            top,
            width,
            height,
            pixels,
        })
    }
}

/// The smallest box around `pixels`' non-empty words, as `(left, top, width,
/// height, its pixels)`. All zero for nothing drawn, such as a blank text.
fn crop(pixels: &[u32], size: (i32, i32)) -> (i32, i32, i32, i32, Vec<u32>) {
    let (width, height) = size;
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);

    for y in 0..height {
        for x in 0..width {
            if pixels[(y * width + x) as usize] != 0 {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }

    if left >= right {
        return (0, 0, 0, 0, Vec::new());
    }

    let mut kept = Vec::with_capacity(((right - left) * (bottom - top)) as usize);
    for y in top..bottom {
        let row = (y * width) as usize;
        kept.extend_from_slice(&pixels[row + left as usize..row + right as usize]);
    }
    (left, top, right - left, bottom - top, kept)
}

/// The Direct2D and DirectWrite factories of this thread, made once.
#[derive(Clone)]
struct Factories {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
}

thread_local! {
    static FACTORIES: RefCell<Option<Factories>> = const { RefCell::new(None) };
    /// Looked through in order: a configuration asks for a few dozen at most,
    /// and a hit should not have to allocate a key.
    static GLYPHS: RefCell<Vec<Glyph>> = const { RefCell::new(Vec::new()) };
}

/// The thread's factories, created on first call. The Direct2D factory is
/// single-threaded, hence per thread — overlays live on the UI thread anyway.
fn factories() -> Result<Factories> {
    FACTORIES.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(factories) = &*slot {
            return Ok(factories.clone());
        }

        let factories = unsafe {
            Factories {
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            }
        };
        *slot = Some(factories.clone());
        Ok(factories)
    })
}

/// Drops this thread's factories, keeping every rendered text, and says
/// whether there were any: `false` means no paint has needed them since the
/// last call, so there is nothing new to trim from the working set.
///
/// Safe at any time, and cheap enough to call on every close. Also needed
/// once at shutdown: the process otherwise exits with Direct2D/DirectWrite
/// still live, and their teardown during process exit is slow (seconds,
/// observed) once anything has been drawn.
pub fn release() -> bool {
    FACTORIES.with_borrow_mut(|slot| slot.take().is_some())
}

/// Forgets every rendered text, for a configuration reload: what the old
/// configuration asked for may never be drawn again.
pub fn forget() {
    GLYPHS.with_borrow_mut(|glyphs| *glyphs = Vec::new());
}

/// What a DirectWrite text format is made from, for
/// [`Canvas::draw_text_color`]. The format itself only exists during a render.
#[derive(Clone, PartialEq)]
pub struct ColorFont {
    family: String,
    height: i32,
    bold: bool,
}

impl ColorFont {
    /// A font by family name and height in pixels, same meaning as in
    /// [`super::Font::new`]. Missing glyphs fall back to other installed
    /// fonts (emoji to Segoe UI Emoji) as in any DirectWrite text.
    pub fn new(family: &str, height: i32, bold: bool) -> Result<Self> {
        Ok(Self {
            family: family.to_owned(),
            height,
            bold,
        })
    }

    fn format(&self, dwrite: &IDWriteFactory) -> Result<IDWriteTextFormat> {
        let family = HSTRING::from(&self.family);
        let format = unsafe {
            dwrite.CreateTextFormat(
                PCWSTR(family.as_ptr()),
                None::<&IDWriteFontCollection>,
                if self.bold {
                    DWRITE_FONT_WEIGHT_BOLD
                } else {
                    DWRITE_FONT_WEIGHT_NORMAL
                },
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                self.height as f32,
                w!(""),
            )?
        };

        unsafe {
            // One line, centred vertically in its rectangle like the GDI path.
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }

        Ok(format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::rgb(255, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 255);
    const SIZE: (i32, i32) = (60, 40);
    const WHOLE: RECT = RECT {
        left: 0,
        top: 0,
        right: SIZE.0,
        bottom: SIZE.1,
    };

    fn font() -> ColorFont {
        ColorFont::new("Segoe UI", 20, false).unwrap()
    }

    fn drawn(text: &str, color: Color) -> Vec<u32> {
        let mut canvas = Canvas::new(SIZE).unwrap();
        canvas
            .draw_text_color(text, WHOLE, &font(), color, TextAlign::Centre)
            .unwrap();
        canvas.pixels_mut().to_vec()
    }

    fn start_empty() {
        forget();
        release();
    }

    #[test]
    fn a_repeated_draw_never_touches_direct2d() {
        start_empty();

        let first = drawn("Ab", RED);
        assert!(release(), "the first draw has to render");
        assert!(first.iter().any(|pixel| *pixel != 0), "nothing was drawn");

        assert_eq!(drawn("Ab", RED), first);
        assert!(!release(), "the second draw should come from the cache");
    }

    #[test]
    fn a_different_colour_is_rendered_again() {
        start_empty();

        let red = drawn("Ab", RED);
        release();
        let blue = drawn("Ab", BLUE);

        assert!(release(), "a monochrome glyph takes the brush colour");
        assert_ne!(red, blue);
    }

    #[test]
    fn a_kept_text_lands_where_a_direct_render_does() {
        start_empty();

        let mut direct = Canvas::new(SIZE).unwrap();
        direct
            .render_text("Ab", SIZE, &font(), RED, TextAlign::Centre)
            .unwrap();

        assert_eq!(drawn("Ab", RED), direct.pixels_mut());
    }

    #[test]
    fn the_rectangle_position_only_moves_the_text() {
        start_empty();

        let mut moved = Canvas::new((SIZE.0 + 10, SIZE.1 + 5)).unwrap();
        let rect = RECT {
            left: 10,
            top: 5,
            right: SIZE.0 + 10,
            bottom: SIZE.1 + 5,
        };
        moved
            .draw_text_color("Ab", rect, &font(), RED, TextAlign::Centre)
            .unwrap();

        let expected = drawn("Ab", RED);
        let pixels = moved.pixels_mut();
        for y in 0..SIZE.1 {
            for x in 0..SIZE.0 {
                let shifted = ((y + 5) * (SIZE.0 + 10) + x + 10) as usize;
                assert_eq!(pixels[shifted], expected[(y * SIZE.0 + x) as usize]);
            }
        }
    }

    #[test]
    fn only_the_covered_pixels_are_kept() {
        start_empty();

        drawn("Ab", RED);

        GLYPHS.with_borrow(|glyphs| {
            let glyph = &glyphs[0];
            assert!(glyph.width > 0 && glyph.width < SIZE.0);
            assert!(glyph.height > 0 && glyph.height < SIZE.1);
            assert_eq!(glyph.pixels.len(), (glyph.width * glyph.height) as usize);
        });
    }

    #[test]
    fn crop_finds_the_box_around_what_was_drawn() {
        let mut pixels = vec![0; 20];
        pixels[6] = 1; // (1, 1) of a 5×4 surface
        pixels[13] = 2; // (3, 2)

        let (left, top, width, height, kept) = crop(&pixels, (5, 4));

        assert_eq!((left, top, width, height), (1, 1, 3, 2));
        assert_eq!(kept, [1, 0, 0, 0, 0, 2]);
    }

    #[test]
    fn crop_of_nothing_is_empty() {
        assert_eq!(crop(&[0; 20], (5, 4)), (0, 0, 0, 0, Vec::new()));
    }

    #[test]
    fn a_blank_text_or_an_empty_rectangle_draws_nothing() {
        start_empty();

        assert!(drawn(" ", RED).iter().all(|pixel| *pixel == 0));

        let mut canvas = Canvas::new(SIZE).unwrap();
        let empty = RECT {
            left: 5,
            top: 5,
            right: 5,
            bottom: 20,
        };
        canvas
            .draw_text_color("Ab", empty, &font(), RED, TextAlign::Left)
            .unwrap();
        assert!(canvas.pixels_mut().iter().all(|pixel| *pixel == 0));
    }

    #[test]
    fn forgetting_renders_again() {
        start_empty();

        drawn("Ab", RED);
        release();
        forget();
        drawn("Ab", RED);

        assert!(release());
    }

    #[test]
    fn an_emoji_keeps_its_colours() {
        start_empty();

        let coloured = drawn("\u{1F600}", Color::WHITE)
            .iter()
            .filter(|pixel| {
                let (r, g, b) = (**pixel >> 16 & 255, **pixel >> 8 & 255, **pixel & 255);
                r != g || g != b
            })
            .count();

        assert!(coloured > 0, "drawn in white, so any colour is the font's");
    }
}
