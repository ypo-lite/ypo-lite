//! Layered overlay windows: what these applications show instead of a GUI.
//!
//! A snapping guide, a volume bar, a launcher popup — all of them are the same
//! thing: a borderless, always-on-top window that appears where the user is
//! looking, does not steal focus, and disappears again.
//!
//! The window is `WS_EX_LAYERED` and drawn with `UpdateLayeredWindow` from a
//! pixel buffer we own. That is the one path in Win32 that gives real
//! per-pixel alpha — soft shadows, rounded corners, translucency — with no
//! `WM_PAINT`, no flicker, and no compositing work of our own. The cost is
//! that we draw every pixel ourselves; [`Canvas`] is the brush.
//!
//! ```no_run
//! use y_core::monitor::Monitor;
//! use y_core::overlay::{Color, Font, Overlay, OverlaySpec, TextAlign};
//! # fn main() -> y_core::Result<()> {
//! let mut overlay = Overlay::new(&OverlaySpec::new("YpoLite.YCoustic.Volume", (240, 72)))?;
//!
//! let rect = overlay.bounds();
//! overlay.canvas_mut().clear(Color::rgba(24, 24, 28, 235));
//! let font = Font::new("Segoe UI", 20, false)?;
//! overlay.canvas_mut().draw_text("Volume 42%", rect, &font, Color::WHITE, TextAlign::Centre)?;
//! overlay.canvas_mut().round_corners(8);
//!
//! let monitor = Monitor::from_cursor()?;
//! overlay.show_at(monitor.centre(overlay.size()))?;
//! # Ok(())
//! # }
//! ```
//!
//! Nothing here is on a timer: an overlay that must fade out again is the
//! application's business (`SetTimer`, then [`Overlay::hide`]).

use std::ffi::c_void;

use windows::Win32::Foundation::{
    COLORREF, E_INVALIDARG, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE,
    FW_BOLD, FW_NORMAL, HBITMAP, HDC, HFONT, HGDIOBJ, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, ShowWindow, ULW_ALPHA,
    UpdateLayeredWindow, WINDOW_EX_STYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Error, HSTRING, PCWSTR, Result};

use crate::window::{self, WindowHandler, WindowSpec};

/// A colour, straight (not premultiplied) — the way a human writes one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// With `alpha` in place of this colour's own.
    pub const fn with_alpha(self, alpha: u8) -> Self {
        Self { a: alpha, ..self }
    }

    /// Reads `#RGB`, `#RRGGBB` or `#RRGGBBAA`, as written in a settings file.
    /// The `#` is optional; anything else is `None`, which a caller should
    /// report rather than silently paint black.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }

        let pair = |index: usize| u8::from_str_radix(&digits[index..index + 2], 16).ok();

        Some(match digits.len() {
            // #RGB is shorthand: each digit doubled, so #f0c == #ff00cc.
            3 => {
                let nibble = |index: usize| {
                    let value = u8::from_str_radix(&digits[index..index + 1], 16).ok()?;
                    Some(value << 4 | value)
                };
                Self::rgb(nibble(0)?, nibble(1)?, nibble(2)?)
            }
            6 => Self::rgb(pair(0)?, pair(2)?, pair(4)?),
            8 => Self::rgba(pair(0)?, pair(2)?, pair(4)?, pair(6)?),
            _ => return None,
        })
    }

    /// The BGRA word `UpdateLayeredWindow` expects: channels premultiplied by
    /// the alpha, because that is what `AC_SRC_ALPHA` blending assumes. A
    /// non-premultiplied buffer shows up as bright fringes around everything.
    fn premultiplied(self) -> u32 {
        let scale = |channel: u8| ((channel as u32 * self.a as u32) / 255) & 0xFF;
        (self.a as u32) << 24 | scale(self.r) << 16 | scale(self.g) << 8 | scale(self.b)
    }

    fn colorref(self) -> COLORREF {
        COLORREF((self.b as u32) << 16 | (self.g as u32) << 8 | self.r as u32)
    }
}

/// How [`Canvas::draw_text`] places a line in its rectangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Centre,
    Right,
}

/// What [`Overlay::new`] needs to know.
#[derive(Clone, Copy)]
pub struct OverlaySpec<'a> {
    /// Window class, unique in the process.
    pub class: &'a str,
    /// Size in physical pixels. A layered window's surface is fixed at this
    /// size; a different size means a different [`Overlay`].
    pub size: (i32, i32),
    /// Let mouse input fall through to whatever is underneath. The right
    /// default for anything purely informative — a guide, a badge, a HUD.
    pub click_through: bool,
}

impl<'a> OverlaySpec<'a> {
    pub const fn new(class: &'a str, size: (i32, i32)) -> Self {
        Self {
            class,
            size,
            click_through: true,
        }
    }

    /// The overlay reacts to the mouse; its handler receives the messages.
    pub const fn interactive(mut self) -> Self {
        self.click_through = false;
        self
    }
}

/// A layered, topmost, non-activating window and the pixels behind it.
pub struct Overlay {
    hwnd: HWND,
    canvas: Canvas,
}

impl Overlay {
    /// Creates a hidden overlay. Draw into [`Overlay::canvas_mut`], then call
    /// [`Overlay::show_at`].
    pub fn new(spec: &OverlaySpec<'_>) -> Result<Self> {
        Self::with_handler(spec, Box::new(NoHandler))
    }

    /// Same, with a handler for the mouse and keyboard messages an
    /// interactive overlay wants (see [`OverlaySpec::interactive`]).
    pub fn with_handler(spec: &OverlaySpec<'_>, handler: Box<dyn WindowHandler>) -> Result<Self> {
        let mut ex_style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if spec.click_through {
            ex_style |= WS_EX_TRANSPARENT;
        }

        // WS_EX_TOOLWINDOW keeps it out of Alt+Tab and the taskbar;
        // WS_EX_NOACTIVATE keeps the window the user was working in focused.
        let window = WindowSpec::new(spec.class, "")
            .with_style(WS_POPUP, ex_style)
            .with_bounds((0, 0), spec.size);

        let hwnd = window::create(&window, handler)?;
        let canvas = match Canvas::new(spec.size) {
            Ok(canvas) => canvas,
            Err(error) => {
                let _ = unsafe { DestroyWindow(hwnd) };
                return Err(error);
            }
        };

        Ok(Self { hwnd, canvas })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn size(&self) -> (i32, i32) {
        self.canvas.size
    }

    /// The whole surface, as a rectangle to draw into.
    pub fn bounds(&self) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: self.canvas.size.0,
            bottom: self.canvas.size.1,
        }
    }

    pub fn canvas_mut(&mut self) -> &mut Canvas {
        &mut self.canvas
    }

    /// Pushes the current pixels to the screen at `position` and shows the
    /// window without activating it.
    pub fn show_at(&self, position: POINT) -> Result<()> {
        self.update(Some(position))?;
        unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE).ok()? };

        // Topmost is not sticky: a full-screen application taking over resets
        // the Z order, so re-assert it every time the overlay comes up.
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
    }

    /// Pushes the current pixels to an already visible overlay, leaving it
    /// where it is — the redraw path of a HUD that changes while shown.
    pub fn refresh(&self) -> Result<()> {
        self.update(None)
    }

    pub fn hide(&self) {
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// Turns mouse transparency on or off after creation, for an overlay that
    /// is only interactive some of the time.
    pub fn set_click_through(&self, click_through: bool) {
        let current = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) } as u32);
        let updated = if click_through {
            current | WS_EX_TRANSPARENT
        } else {
            WINDOW_EX_STYLE(current.0 & !WS_EX_TRANSPARENT.0)
        };
        unsafe { SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, updated.0 as isize) };
    }

    fn update(&self, position: Option<POINT>) -> Result<()> {
        let size = SIZE {
            cx: self.canvas.size.0,
            cy: self.canvas.size.1,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255, // per-pixel alpha, not a global fade
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                position.as_ref().map(|p| p as *const POINT),
                Some(&size),
                Some(self.canvas.dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// The pixel buffer behind an [`Overlay`]: a 32-bit top-down DIB selected
/// into a memory DC.
///
/// Pixels are `0xAARRGGBB` **premultiplied**, which is what
/// `UpdateLayeredWindow` blends with. The fills below do the multiplication;
/// code writing [`Canvas::pixels_mut`] directly must do it itself.
pub struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u32,
    size: (i32, i32),
}

impl Canvas {
    /// Allocates a surface of `size` physical pixels, fully transparent.
    pub fn new(size: (i32, i32)) -> Result<Self> {
        if size.0 <= 0 || size.1 <= 0 {
            return Err(Error::new(E_INVALIDARG, "an overlay needs a positive size"));
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.0,
                // Negative height: a top-down bitmap, so row 0 is the top one
                // and the buffer is indexed the way anyone would expect.
                biHeight: -size.1,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return Err(Error::from_thread());
        }

        let mut pixels: *mut c_void = std::ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0) };

        let bitmap = match bitmap {
            Ok(bitmap) if !pixels.is_null() => bitmap,
            Ok(_) => {
                let _ = unsafe { DeleteDC(dc) };
                return Err(Error::new(
                    E_INVALIDARG,
                    "CreateDIBSection returned no bits",
                ));
            }
            Err(error) => {
                let _ = unsafe { DeleteDC(dc) };
                return Err(error);
            }
        };

        let previous = unsafe { SelectObject(dc, bitmap.into()) };

        Ok(Self {
            dc,
            bitmap,
            previous,
            pixels: pixels as *mut u32,
            size,
        })
    }

    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The raw surface, one `0xAARRGGBB` premultiplied word per pixel, top
    /// row first, `width` words per row.
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        let count = (self.size.0 * self.size.1) as usize;
        unsafe { std::slice::from_raw_parts_mut(self.pixels, count) }
    }

    /// Paints the whole surface. `Color::TRANSPARENT` erases it.
    pub fn clear(&mut self, color: Color) {
        let value = color.premultiplied();
        self.pixels_mut().fill(value);
    }

    /// Fills a rectangle, clipped to the surface. Replaces what was there —
    /// these are not blended: an overlay is drawn back to front in one pass.
    pub fn fill_rect(&mut self, rect: RECT, color: Color) {
        let value = color.premultiplied();
        let (width, height) = self.size;
        let left = rect.left.clamp(0, width);
        let right = rect.right.clamp(0, width);
        let top = rect.top.clamp(0, height);
        let bottom = rect.bottom.clamp(0, height);

        for y in top..bottom {
            let row = (y * width) as usize;
            self.pixels_mut()[row + left as usize..row + right as usize].fill(value);
        }
    }

    /// A one-pixel-per-side outline just inside `rect`.
    pub fn stroke_rect(&mut self, rect: RECT, color: Color) {
        let RECT {
            left,
            top,
            right,
            bottom,
        } = rect;
        self.fill_rect(
            RECT {
                left,
                top,
                right,
                bottom: top + 1,
            },
            color,
        );
        self.fill_rect(
            RECT {
                left,
                top: bottom - 1,
                right,
                bottom,
            },
            color,
        );
        self.fill_rect(
            RECT {
                left,
                top,
                right: left + 1,
                bottom,
            },
            color,
        );
        self.fill_rect(
            RECT {
                left: right - 1,
                top,
                right,
                bottom,
            },
            color,
        );
    }

    /// Rounds the four corners of the whole surface by erasing what falls
    /// outside the radius, with an antialiased edge.
    ///
    /// Call it last: it works on the finished image, which is what lets the
    /// same routine round a filled panel, an image, or text-covered chrome.
    pub fn round_corners(&mut self, radius: i32) {
        let (width, height) = self.size;
        let radius = radius.min(width / 2).min(height / 2);
        if radius <= 0 {
            return;
        }

        for corner_y in 0..radius {
            for corner_x in 0..radius {
                // Distance from the corner's centre of curvature, sampled at
                // the pixel centre; coverage is the fractional part over one
                // pixel of width, which is antialiasing enough at this size.
                let dx = (radius - corner_x) as f32 - 0.5;
                let dy = (radius - corner_y) as f32 - 0.5;
                let distance = (dx * dx + dy * dy).sqrt();
                let coverage = (radius as f32 - distance + 0.5).clamp(0.0, 1.0);
                if coverage >= 1.0 {
                    continue;
                }

                for (x, y) in [
                    (corner_x, corner_y),
                    (width - 1 - corner_x, corner_y),
                    (corner_x, height - 1 - corner_y),
                    (width - 1 - corner_x, height - 1 - corner_y),
                ] {
                    self.scale_pixel(x, y, coverage);
                }
            }
        }
    }

    /// Draws one line of text with GDI.
    ///
    /// GDI knows nothing about alpha and writes zero into that channel
    /// wherever it touches, which on a layered window means holes. The fix is
    /// the reason this method takes a rectangle: after drawing, the alpha of
    /// the whole rectangle is restored to `color`'s. So the area under the
    /// text must be uniformly opaque — fill the panel first, draw the text,
    /// round the corners last, which is the order the module example uses.
    pub fn draw_text(
        &mut self,
        text: &str,
        rect: RECT,
        font: &Font,
        color: Color,
        align: TextAlign,
    ) -> Result<()> {
        let mut rect = rect;
        let text: Vec<u16> = text.encode_utf16().collect();

        let alignment = match align {
            TextAlign::Left => DT_LEFT,
            TextAlign::Centre => DT_CENTER,
            TextAlign::Right => DT_RIGHT,
        };

        unsafe {
            let previous = SelectObject(self.dc, font.handle.into());
            SetBkMode(self.dc, TRANSPARENT);
            SetTextColor(self.dc, color.colorref());

            let mut buffer = text;
            let drawn = DrawTextW(
                self.dc,
                &mut buffer,
                &mut rect,
                alignment | DT_SINGLELINE | DT_VCENTER,
            );

            SelectObject(self.dc, previous);

            if drawn == 0 && !buffer.is_empty() {
                return Err(Error::from_thread());
            }
        }

        self.restore_alpha(rect, color.a);
        Ok(())
    }

    /// Sets the alpha channel of every pixel in `rect`, repairing what GDI
    /// zeroed. Exposed for code that draws with GDI directly.
    pub fn restore_alpha(&mut self, rect: RECT, alpha: u8) {
        let (width, height) = self.size;
        let left = rect.left.clamp(0, width);
        let right = rect.right.clamp(0, width);
        let top = rect.top.clamp(0, height);
        let bottom = rect.bottom.clamp(0, height);
        let alpha = (alpha as u32) << 24;

        let pixels = self.pixels_mut();
        for y in top..bottom {
            let row = (y * width) as usize;
            for x in left..right {
                let pixel = &mut pixels[row + x as usize];
                *pixel = (*pixel & 0x00FF_FFFF) | alpha;
            }
        }
    }

    /// Multiplies a pixel — all four premultiplied channels — by `factor`.
    fn scale_pixel(&mut self, x: i32, y: i32, factor: f32) {
        let width = self.size.0;
        let index = (y * width + x) as usize;
        let pixel = self.pixels_mut()[index];

        let channel = |shift: u32| {
            let value = ((pixel >> shift) & 0xFF) as f32 * factor;
            (value as u32).min(255) << shift
        };

        self.pixels_mut()[index] = channel(24) | channel(16) | channel(8) | channel(0);
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// RAII wrapper around a GDI font.
pub struct Font {
    handle: HFONT,
}

impl Font {
    /// A font by family name and height in pixels — physical pixels, so scale
    /// the value with [`crate::monitor::Monitor::scaled`] on a high-DPI
    /// screen.
    pub fn new(family: &str, height: i32, bold: bool) -> Result<Self> {
        let family = HSTRING::from(family);
        let handle = unsafe {
            CreateFontW(
                -height, // negative: character height rather than cell height
                0,
                0,
                0,
                if bold {
                    FW_BOLD.0 as i32
                } else {
                    FW_NORMAL.0 as i32
                },
                0,
                0,
                0,
                DEFAULT_CHARSET,
                Default::default(),
                Default::default(),
                Default::default(),
                FF_DONTCARE.0 as u32,
                PCWSTR(family.as_ptr()),
            )
        };

        if handle.is_invalid() {
            return Err(Error::from_thread());
        }

        Ok(Self { handle })
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.handle.into()) };
    }
}

/// The handler of an overlay that only ever shows pixels.
struct NoHandler;

impl WindowHandler for NoHandler {
    fn handle_message(
        &mut self,
        _hwnd: HWND,
        _msg: u32,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Option<LRESULT> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn premultiplies() {
        assert_eq!(Color::WHITE.premultiplied(), 0xFFFF_FFFF);
        assert_eq!(Color::TRANSPARENT.premultiplied(), 0);
        // Half-transparent white: every channel halved with it.
        assert_eq!(Color::WHITE.with_alpha(128).premultiplied(), 0x8080_8080);
    }
}
