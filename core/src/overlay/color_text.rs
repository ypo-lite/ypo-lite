//! Text through DirectWrite/Direct2D, for the `color-text` feature: the
//! colour-font (emoji) counterpart of [`Canvas::draw_text`]'s GDI.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
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
    /// Direct2D writes real alpha, so unlike the GDI path there is nothing to
    /// repair afterwards: `color` may be translucent, the area under the text
    /// need not be uniformly opaque, and the call may happen in any order
    /// relative to the other drawing (corner rounding still goes last, as it
    /// erases pixels). Text is grayscale anti-aliased, since ClearType needs
    /// an opaque backdrop.
    pub fn draw_text_color(
        &mut self,
        text: &str,
        rect: RECT,
        font: &ColorFont,
        color: Color,
        align: TextAlign,
    ) -> Result<()> {
        let target = self.render_target()?;
        let text: Vec<u16> = text.encode_utf16().collect();

        let layout = D2D_RECT_F {
            left: rect.left as f32,
            top: rect.top as f32,
            right: rect.right as f32,
            bottom: rect.bottom as f32,
        };
        let brush_color = D2D1_COLOR_F {
            r: color.r as f32 / 255.0,
            g: color.g as f32 / 255.0,
            b: color.b as f32 / 255.0,
            a: color.a as f32 / 255.0,
        };

        unsafe {
            font.format.SetTextAlignment(match align {
                TextAlign::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Centre => DWRITE_TEXT_ALIGNMENT_CENTER,
                TextAlign::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
            let brush = target.CreateSolidColorBrush(&brush_color, None)?;

            target.BeginDraw();
            target.DrawText(
                &text,
                &font.format,
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            target.EndDraw(None, None)
        }
    }

    /// The Direct2D render target bound to this canvas's DC, made on first use.
    fn render_target(&mut self) -> Result<ID2D1DCRenderTarget> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }

        let factories = factories()?;
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
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
            self.target = Some(target.clone());
            Ok(target)
        }
    }
}

/// The Direct2D and DirectWrite factories of this thread, made once.
#[derive(Clone)]
struct Factories {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
}

thread_local! {
    static FACTORIES: std::cell::RefCell<Option<Factories>> = const { std::cell::RefCell::new(None) };
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

/// Drops this thread's cached factories. Call at shutdown once every
/// [`ColorFont`] and [`Canvas`] is gone: the process otherwise exits with
/// Direct2D/DirectWrite still live, and their teardown during process exit
/// is slow (seconds, observed) once anything has been drawn.
pub fn release() {
    FACTORIES.with(|slot| *slot.borrow_mut() = None);
}

/// A DirectWrite text format, for [`Canvas::draw_text_color`].
pub struct ColorFont {
    format: IDWriteTextFormat,
}

impl ColorFont {
    /// A font by family name and height in pixels, same meaning as in
    /// [`super::Font::new`]. Missing glyphs fall back to other installed
    /// fonts (emoji to Segoe UI Emoji) as in any DirectWrite text.
    pub fn new(family: &str, height: i32, bold: bool) -> Result<Self> {
        let family = HSTRING::from(family);
        let format = unsafe {
            factories()?.dwrite.CreateTextFormat(
                PCWSTR(family.as_ptr()),
                None::<&IDWriteFontCollection>,
                if bold {
                    DWRITE_FONT_WEIGHT_BOLD
                } else {
                    DWRITE_FONT_WEIGHT_NORMAL
                },
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                height as f32,
                w!(""),
            )?
        };

        unsafe {
            // One line, centred vertically in its rectangle like the GDI path.
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        }

        Ok(Self { format })
    }
}
