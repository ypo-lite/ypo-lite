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
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FW_BOLD, FW_NORMAL, GetDC,
    GetTextExtentExPointW, GetTextExtentPoint32W, HBITMAP, HDC, HFONT, HGDIOBJ, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WINDOW_EX_STYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Error, HSTRING, PCWSTR, Result};

use crate::window::{self, WindowHandler, WindowSpec};

// Colour text (DirectWrite, so emoji keep their colours) sits behind the
// `color-text` feature: it pulls Direct2D and DirectWrite into the process.
// Without the feature `ColorFont`/`draw_text_color` fall back to GDI, so
// callers need no `cfg` of their own.
#[cfg(feature = "color-text")]
mod color_text;
#[cfg(feature = "color-text")]
pub use color_text::{ColorFont, forget as forget_color_text, release as release_color_text};

/// Nothing to release without the `color-text` feature.
#[cfg(not(feature = "color-text"))]
pub fn release_color_text() -> bool {
    false
}

/// Nothing to forget without the `color-text` feature.
#[cfg(not(feature = "color-text"))]
pub fn forget_color_text() {}

/// GDI stand-in for the DirectWrite `ColorFont`: same API, monochrome emoji.
#[cfg(not(feature = "color-text"))]
pub struct ColorFont(Font);

#[cfg(not(feature = "color-text"))]
impl ColorFont {
    pub fn new(family: &str, height: i32, bold: bool) -> Result<Self> {
        Font::new(family, height, bold).map(Self)
    }
}

#[cfg(not(feature = "color-text"))]
impl Canvas {
    /// See [`Canvas::draw_text`]; this is it, the `color-text` feature being off.
    pub fn draw_text_color(
        &mut self,
        text: &str,
        rect: RECT,
        font: &ColorFont,
        color: Color,
        align: TextAlign,
    ) -> Result<()> {
        self.draw_text(text, rect, &font.0, color, align)
    }
}

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

    /// Exposed to `crate::menu` for owner-drawn menu theming — the only
    /// other place in `y-core` that talks to raw GDI colour values directly.
    pub(crate) fn colorref(self) -> COLORREF {
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
    /// Let the window take keyboard focus (drop `WS_EX_NOACTIVATE`) instead
    /// of staying behind whatever the user was working in. The right choice
    /// for a popup driven as much by the keyboard as the mouse — e.g. an
    /// action grid with a key bound to each tile — where routing `WM_KEYDOWN`
    /// straight to the window is simpler and cheaper than a scoped
    /// `WH_KEYBOARD_LL` hook, and losing focus (`WM_KILLFOCUS`) is exactly
    /// the signal that means "the user clicked outside, close this". See
    /// [`Overlay::activate`].
    pub focusable: bool,
    /// See [`crate::window::WindowSpec::owner`] — set this whenever more
    /// than one [`Overlay`] from the same app needs to stay reliably
    /// stacked and visible together (e.g. a target-window highlight shown
    /// alongside its own action popup): two independent, owner-less
    /// `WS_EX_TOPMOST` popups have no Z-order relationship for Windows to
    /// maintain between them, and in practice only the one whose
    /// `SetWindowPos` "wins" ends up visible — sharing an owner is what
    /// fixes that.
    pub owner: Option<HWND>,
}

impl<'a> OverlaySpec<'a> {
    pub const fn new(class: &'a str, size: (i32, i32)) -> Self {
        Self {
            class,
            size,
            click_through: true,
            focusable: false,
            owner: None,
        }
    }

    /// The overlay reacts to the mouse; its handler receives the messages.
    pub const fn interactive(mut self) -> Self {
        self.click_through = false;
        self
    }

    /// The overlay can take keyboard focus — see [`OverlaySpec::focusable`].
    pub const fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    pub const fn with_owner(mut self, owner: HWND) -> Self {
        self.owner = Some(owner);
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
        let mut ex_style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        if !spec.focusable {
            ex_style |= WS_EX_NOACTIVATE;
        }
        if spec.click_through {
            ex_style |= WS_EX_TRANSPARENT;
        }

        // WS_EX_TOOLWINDOW keeps it out of Alt+Tab and the taskbar;
        // WS_EX_NOACTIVATE keeps the window the user was working in focused
        // — dropped for a `focusable` overlay, which wants the opposite.
        let mut window = WindowSpec::new(spec.class, "")
            .with_style(WS_POPUP, ex_style)
            .with_bounds((0, 0), spec.size);
        if let Some(owner) = spec.owner {
            window = window.with_owner(owner);
        }

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
        self.show_at_size(position, self.canvas.size)
    }

    /// [`Overlay::show_at`], showing only the top left `size` of the surface
    /// (clamped to it). A layered window's surface is fixed at creation, so
    /// a window whose height changes — a list that grows as results come in
    /// — is created at the largest size it will need and shown at the size
    /// in use; [`Overlay::refresh_size`] changes it later.
    pub fn show_at_size(&self, position: POINT, size: (i32, i32)) -> Result<()> {
        self.update(Some(position), size)?;

        // ShowWindow's BOOL return is not a success/failure code — unlike
        // almost every other BOOL-returning Win32 call here, it reports
        // whether the window was *previously* visible (0 = was hidden, the
        // ordinary case for a popup nobody has shown yet). Treating that
        // like an error via `.ok()?` was a real bug: it aborted `show_at`
        // right here — before the topmost re-assertion below ever ran —
        // on what was actually a normal, successful first show. Windows had
        // already made the window visible by this point regardless; only
        // this function's own bookkeeping thought it had failed. Silently
        // showing a second, unrelated overlay (e.g. `y-snap`'s target-window
        // focus border next to its action popup) after the first looked
        // exactly like this: only the first one's `show_at` ever got called
        // (the false "error" made the caller bail before showing the
        // second), so the second simply never appeared.
        let _ = unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };

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
        self.update(None, self.canvas.size)
    }

    /// [`Overlay::refresh`] at a different size, the top left of the surface:
    /// the window grows or shrinks in place, its top left staying where it is.
    pub fn refresh_size(&self, size: (i32, i32)) -> Result<()> {
        self.update(None, size)
    }

    pub fn hide(&self) {
        let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    /// Gives this overlay the keyboard focus — only meaningful for one
    /// created with [`OverlaySpec::focusable`]; call it right after
    /// [`Overlay::show_at`], which shows the window without activating it.
    /// `SetForegroundWindow` first, matching HulK's `ShowActionOverlay`: it
    /// is what actually moves the *foreground* window, which `SetFocus`
    /// alone does not reliably do across processes.
    pub fn activate(&self) {
        let _ = unsafe { SetForegroundWindow(self.hwnd) };
        let _ = unsafe { SetFocus(Some(self.hwnd)) };
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

    fn update(&self, position: Option<POINT>, size: (i32, i32)) -> Result<()> {
        let size = SIZE {
            cx: size.0.clamp(1, self.canvas.size.0),
            cy: size.1.clamp(1, self.canvas.size.1),
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

        for_each_corner_pixel(width, height, radius, |x, y, coverage| {
            self.blend_pixel(x, y, coverage, 0);
        });
    }

    /// Rounds one rectangle's corners in place, the same antialiased corner
    /// test [`Canvas::round_corners`] uses on the whole surface — but
    /// blending what falls outside the radius toward `background` instead of
    /// toward transparent. A tile sits on the popup's own opaque-ish
    /// background, not directly on the desktop, so its corners should read
    /// as "the panel showing through" rather than a punched-out hole in the
    /// layered window; pass the same colour [`Canvas::clear`] painted the
    /// panel with.
    ///
    /// Call this last, after any GDI text/icon drawing inside `rect` — same
    /// requirement as `round_corners`, and for the same reason: GDI can
    /// still touch pixels right up to `rect`'s edges, including the corner
    /// region a small radius carves out.
    pub fn round_rect_corners(&mut self, rect: RECT, radius: i32, background: Color) {
        let (width, height) = self.size;
        let radius = radius
            .min((rect.right - rect.left) / 2)
            .min((rect.bottom - rect.top) / 2);
        if radius <= 0 {
            return;
        }
        let target = background.premultiplied();

        for_each_rect_corner_pixel(rect, width, height, radius, |x, y, coverage| {
            self.blend_pixel(x, y, coverage, target);
        });
    }

    /// [`Canvas::round_corners`], plus a `border_width`-thick opaque outline
    /// traced along the rounded boundary in `border_color`.
    ///
    /// Not the same as drawing a plain [`Canvas::stroke_rect`] and rounding
    /// it afterwards — a straight rectangular stroke has no relationship to
    /// the corner cut `round_corners` makes, so near each corner the
    /// straight line just gets partially erased by an arc it was never drawn
    /// to trace, leaving a straight-edged gap instead of a border that
    /// curves with the panel. This tests the border against the *same*
    /// rounded boundary it is being cut from, the fix HulK's
    /// `PointInRoundedRect`/`FillRoundedRectWithBorder` uses for the
    /// identical bug. The straight (non-corner) edges of the border are
    /// unaffected by rounding at any radius, so draw those the ordinary way
    /// first — `stroke_rect(bounds, color)` before calling this — and let
    /// this method redo just the four corner boxes it touches, on top of
    /// that.
    ///
    /// Call it last, same requirement as `round_corners`.
    pub fn round_corners_with_border(
        &mut self,
        radius: i32,
        border_width: i32,
        border_color: Color,
    ) {
        let (width, height) = self.size;
        let whole = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        self.round_rect_corners_with_border(whole, radius, border_width, border_color);
    }

    /// [`Canvas::round_corners_with_border`] for the part of the surface in
    /// `rect` — the visible part of an overlay shown smaller than its canvas
    /// ([`Overlay::show_at_size`]). Corners fade to transparent, as for the
    /// whole surface; `rect` must lie inside it.
    pub fn round_rect_corners_with_border(
        &mut self,
        rect: RECT,
        radius: i32,
        border_width: i32,
        border_color: Color,
    ) {
        let radius = radius
            .min((rect.right - rect.left) / 2)
            .min((rect.bottom - rect.top) / 2);
        if radius <= 0 {
            return;
        }
        let inner_radius = (radius - border_width.max(0)).max(0);
        let border = border_color.premultiplied();

        for corner_y in 0..radius {
            for corner_x in 0..radius {
                // One distance, from the *outer* arc's own centre of
                // curvature — the inner arc is concentric with it (a border
                // band is a ring of uniform width, not two independently
                // inset corners), so it is measured against that same
                // distance too, just a smaller radius. Calling
                // `corner_coverage(corner_x, corner_y, inner_radius)` here
                // instead — computing a *second*, differently-centred
                // distance from `inner_radius` — was the bug this replaced:
                // it silently shifted the inner arc's centre inward along
                // with its radius, so it was never actually concentric with
                // the outer one.
                let distance = corner_distance(corner_x, corner_y, radius);
                let outer = coverage_at(distance, radius);

                // The border/interior cutover is a hard edge, not
                // antialiased — matching HulK's own boolean
                // `PointInRoundedRect`, and simpler: both sides are fully
                // opaque, so a one-pixel stair-step there is not visible the
                // way an unantialiased *outer* silhouette against the
                // desktop would be.
                let in_border_band = coverage_at(distance, inner_radius) < 1.0;

                if !in_border_band && outer >= 1.0 {
                    // Deep interior, outside the border band entirely —
                    // already correct from earlier drawing. Everything past
                    // this point either needs the border colour written in
                    // (`in_border_band`) or needs fading toward transparent
                    // at the true outer edge (`outer < 1.0`) — a pixel can be
                    // `in_border_band` and still have `outer == 1.0` (most of
                    // a several-pixel-wide band does, away from that outer
                    // edge itself), which is exactly what the earlier
                    // `outer >= 1.0` early-out here got wrong: it skipped
                    // nearly the whole curved border, leaving the corner box
                    // untouched and reading as a sharp square corner.
                    continue;
                }

                for (x, y) in [
                    (rect.left + corner_x, rect.top + corner_y),
                    (rect.right - 1 - corner_x, rect.top + corner_y),
                    (rect.left + corner_x, rect.bottom - 1 - corner_y),
                    (rect.right - 1 - corner_x, rect.bottom - 1 - corner_y),
                ] {
                    if in_border_band {
                        // Overwrite outright — this pixel is meant to be
                        // solid border colour regardless of whatever the
                        // earlier straight `stroke_rect` or the panel's own
                        // background left here.
                        self.set_pixel(x, y, border);
                    }
                    if outer < 1.0 {
                        // The true outer silhouette: fade toward transparent
                        // using whatever was just written above as `from`.
                        self.blend_pixel(x, y, outer, 0);
                    }
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
        self.draw_text_with(text, rect, font, color, align, DRAW_TEXT_FORMAT(0))
    }

    /// [`Canvas::draw_text`] for text that may not fit `rect`: what does not
    /// is replaced by `…`. `&` is drawn as a letter, not as an underline
    /// prefix, since this is for names the user wrote (`Q&A`).
    pub fn draw_text_ellipsis(
        &mut self,
        text: &str,
        rect: RECT,
        font: &Font,
        color: Color,
        align: TextAlign,
    ) -> Result<()> {
        self.draw_text_with(
            text,
            rect,
            font,
            color,
            align,
            DT_END_ELLIPSIS | DT_NOPREFIX,
        )
    }

    fn draw_text_with(
        &mut self,
        text: &str,
        rect: RECT,
        font: &Font,
        color: Color,
        align: TextAlign,
        extra: DRAW_TEXT_FORMAT,
    ) -> Result<()> {
        // Nothing to draw, and `DrawTextW` must not be asked to: an empty `Vec`'s
        // pointer is dangling, and it reads through it.
        if text.is_empty() {
            return Ok(());
        }
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
                alignment | DT_SINGLELINE | DT_VCENTER | extra,
            );

            SelectObject(self.dc, previous);

            if drawn == 0 {
                return Err(Error::from_thread());
            }
        }

        self.restore_alpha(rect, color.a);
        Ok(())
    }

    /// A text caret: a bar `thickness` pixels wide and `height` tall, its
    /// left edge at `x`. Draw it after the text (it needs no GDI, so it
    /// leaves alpha alone). A selection highlight is a [`Canvas::fill_rect`]
    /// drawn *before* the text, between the two offsets
    /// [`Font::caret_offsets`] gives for its ends.
    pub fn draw_caret(&mut self, x: i32, top: i32, height: i32, thickness: i32, color: Color) {
        self.fill_rect(
            RECT {
                left: x,
                top,
                right: x + thickness,
                bottom: top + height,
            },
            color,
        );
    }

    /// Composites `pixels` — `width`×`height` words, `0xAARRGGBB`
    /// premultiplied, top row first, as [`crate::icon::Icon::to_pixels`]
    /// makes — over the surface with its top left at `(x, y)`, clipped to
    /// the surface. Unlike the fills this one *blends*, so an icon's soft
    /// edge lands on whatever is underneath.
    ///
    /// Pure pixel work, no GDI: it can come before or after text and
    /// [`Canvas::restore_alpha`] without being disturbed by either. Like all
    /// drawing, it comes before the corner rounding.
    pub fn blit(&mut self, x: i32, y: i32, width: i32, height: i32, pixels: &[u32]) {
        if width <= 0 || height <= 0 || pixels.len() < (width * height) as usize {
            return;
        }
        let (surface_width, surface_height) = self.size;
        let left = x.max(0);
        let right = (x + width).min(surface_width);
        let top = y.max(0);
        let bottom = (y + height).min(surface_height);

        for row in top..bottom {
            let source = ((row - y) * width) as usize;
            let target = (row * surface_width) as usize;
            for column in left..right {
                let source = pixels[source + (column - x) as usize];
                let target = &mut self.pixels_mut()[target + column as usize];
                *target = over(source, *target);
            }
        }
    }

    /// Draws `icon` at `size`×`size` with its top left at `(x, y)`. Renders
    /// it every call (`Icon::to_pixels`); a caller that draws the same icon
    /// repeatedly keeps the pixels and uses [`Canvas::blit`]. Nothing is
    /// drawn if the icon cannot be rendered.
    pub fn draw_icon(&mut self, icon: &crate::icon::Icon, x: i32, y: i32, size: i32) {
        if let Some(pixels) = icon.to_pixels(size) {
            self.blit(x, y, size, size, &pixels);
        }
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

    /// Blends a pixel toward `target` (an already-premultiplied word) by
    /// `coverage` — `1.0` leaves the pixel as it is, `0.0` replaces it with
    /// `target` outright. [`Canvas::round_corners`] blends toward
    /// transparent (`target = 0`); [`Canvas::round_rect_corners`] toward a
    /// caller-supplied background colour.
    fn blend_pixel(&mut self, x: i32, y: i32, coverage: f32, target: u32) {
        let width = self.size.0;
        let index = (y * width + x) as usize;
        let pixel = self.pixels_mut()[index];

        let channel = |shift: u32| {
            let from = ((pixel >> shift) & 0xFF) as f32;
            let to = ((target >> shift) & 0xFF) as f32;
            ((to + (from - to) * coverage).round() as u32).min(255) << shift
        };

        self.pixels_mut()[index] = channel(24) | channel(16) | channel(8) | channel(0);
    }

    /// Overwrites a pixel outright with an already-premultiplied word — the
    /// unconditional counterpart to [`Canvas::blend_pixel`], for a border
    /// pixel that must be exactly `border_color` regardless of whatever was
    /// drawn underneath it.
    fn set_pixel(&mut self, x: i32, y: i32, value: u32) {
        let width = self.size.0;
        let index = (y * width + x) as usize;
        self.pixels_mut()[index] = value;
    }
}

/// `source` laid over `destination`, both premultiplied: the source plus
/// what the destination keeps of itself, `destination * (1 - source alpha)`.
fn over(source: u32, destination: u32) -> u32 {
    match source >> 24 {
        255 => source,
        0 => destination,
        alpha => {
            let keep = 255 - alpha;
            let channel = |shift: u32| {
                let from_source = (source >> shift) & 0xFF;
                let from_destination = (destination >> shift) & 0xFF;
                (from_source + (from_destination * keep + 127) / 255).min(255) << shift
            };
            channel(24) | channel(16) | channel(8) | channel(0)
        }
    }
}

/// Distance from a pixel at `(corner_x, corner_y)` — its offset from the
/// true corner of a `radius`-rounded rectangle — to that corner's own
/// centre of curvature (inset `radius` in from each straight edge),
/// sampled at the pixel's centre. The shared building block: two arcs
/// concentric on the *same* centre (an outer boundary and, inside it, an
/// inner one a border band is measured against) both call this with that
/// one fixed `radius`, then compare the one `distance` it returns to
/// whichever of their own radii they each care about — see
/// [`Canvas::round_corners_with_border`] for why computing it twice, once
/// per radius, is the bug this shape avoids: it silently shifts the centre
/// along with the radius instead of keeping both arcs concentric.
fn corner_distance(corner_x: i32, corner_y: i32, radius: i32) -> f32 {
    let dx = (radius - corner_x) as f32 - 0.5;
    let dy = (radius - corner_y) as f32 - 0.5;
    (dx * dx + dy * dy).sqrt()
}

/// `1.0` at/inside `radius` of a corner's centre of curvature, `0.0` a
/// pixel of width past it, blending in between — the antialiasing every
/// corner-rounding method here shares.
fn coverage_at(distance: f32, radius: i32) -> f32 {
    (radius as f32 - distance + 0.5).clamp(0.0, 1.0)
}

/// [`coverage_at`] against the corner's own centre of curvature — the
/// common case, used by every caller that only ever tests one radius per
/// corner (unlike [`Canvas::round_corners_with_border`], which tests two
/// concentric ones and so calls [`corner_distance`]/[`coverage_at`]
/// separately instead).
fn corner_coverage(corner_x: i32, corner_y: i32, radius: i32) -> f32 {
    coverage_at(corner_distance(corner_x, corner_y, radius), radius)
}

/// Walks the pixels in each of a `width`×`height` surface's four
/// `radius`×`radius` corner boxes, calling `paint(x, y, coverage)` for every
/// one not fully inside the rounded shape (`coverage == 1.0`, needing no
/// change, is skipped) — the shared iteration [`Canvas::round_corners`] and
/// [`Canvas::round_rect_corners`] both drive with a different `paint`.
fn for_each_corner_pixel(
    width: i32,
    height: i32,
    radius: i32,
    mut paint: impl FnMut(i32, i32, f32),
) {
    for corner_y in 0..radius {
        for corner_x in 0..radius {
            let coverage = corner_coverage(corner_x, corner_y, radius);
            if coverage >= 1.0 {
                continue;
            }
            for (x, y) in [
                (corner_x, corner_y),
                (width - 1 - corner_x, corner_y),
                (corner_x, height - 1 - corner_y),
                (width - 1 - corner_x, height - 1 - corner_y),
            ] {
                paint(x, y, coverage);
            }
        }
    }
}

/// [`for_each_corner_pixel`], scoped to one `rect` inside a `width`×
/// `height` surface instead of the whole thing, clipped to the surface's
/// own bounds — a tile can sit flush against the popup's own edge.
fn for_each_rect_corner_pixel(
    rect: RECT,
    width: i32,
    height: i32,
    radius: i32,
    mut paint: impl FnMut(i32, i32, f32),
) {
    for corner_y in 0..radius {
        for corner_x in 0..radius {
            let coverage = corner_coverage(corner_x, corner_y, radius);
            if coverage >= 1.0 {
                continue;
            }
            for (x, y) in [
                (rect.left + corner_x, rect.top + corner_y),
                (rect.right - 1 - corner_x, rect.top + corner_y),
                (rect.left + corner_x, rect.bottom - 1 - corner_y),
                (rect.right - 1 - corner_x, rect.bottom - 1 - corner_y),
            ] {
                if x >= 0 && x < width && y >= 0 && y < height {
                    paint(x, y, coverage);
                }
            }
        }
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

    /// The width and height `text` takes on one line in this font, in pixels
    /// — what an overlay sized to its content needs before it can be created.
    pub fn measure(&self, text: &str) -> (i32, i32) {
        let text: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            let dc = GetDC(None);
            let previous = SelectObject(dc, self.handle.into());
            let _ = GetTextExtentPoint32W(dc, &text, &mut size);
            SelectObject(dc, previous);
            ReleaseDC(None, dc);
        }
        (size.cx, size.cy)
    }

    /// Where each caret position of `text` falls, in pixels from the left of
    /// the text: entry `n` is the width of the first `n` UTF-16 units, so
    /// there are one more entries than units, the first `0` and the last the
    /// whole width. What a caret or a selection highlight needs, from the
    /// positions `y-pop`'s text input keeps in the same unit. The two halves
    /// of a surrogate pair share an offset (a caret never sits between them).
    pub fn caret_offsets(&self, text: &str) -> Vec<i32> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut offsets = vec![0; units.len() + 1];
        if units.is_empty() {
            return offsets;
        }

        let mut size = SIZE::default();
        unsafe {
            let dc = GetDC(None);
            let previous = SelectObject(dc, self.handle.into());
            let _ = GetTextExtentExPointW(
                dc,
                PCWSTR(units.as_ptr()),
                units.len() as i32,
                i32::MAX,
                None,
                Some(offsets[1..].as_mut_ptr()),
                &mut size,
            );
            SelectObject(dc, previous);
            ReleaseDC(None, dc);
        }

        // Never go backwards, whatever the font does with combining marks.
        for at in 1..offsets.len() {
            offsets[at] = offsets[at].max(offsets[at - 1]);
        }
        offsets
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
    use super::{Canvas, Color, Font, TextAlign, over};
    use windows::Win32::Foundation::RECT;

    #[test]
    fn over_keeps_what_the_source_leaves() {
        let opaque = 0xFF10_2030;
        let half_white = Color::WHITE.with_alpha(128).premultiplied(); // 0x80808080
        assert_eq!(over(opaque, 0xFFAA_BBCC), opaque, "opaque replaces");
        assert_eq!(over(0, 0xFFAA_BBCC), 0xFFAA_BBCC, "transparent leaves");
        assert_eq!(over(0, 0), 0);

        // Half-transparent white on opaque black: mid grey, still opaque.
        let on_black = over(half_white, 0xFF00_0000);
        assert_eq!(on_black >> 24, 255);
        assert!(((on_black >> 8) & 0xFF).abs_diff(128) <= 1, "{on_black:#x}");

        // On a transparent surface: the source as it was.
        assert_eq!(over(half_white, 0), half_white);

        // Never more than full.
        assert!(over(0xFFFF_FFFF, 0xFFFF_FFFF) == 0xFFFF_FFFF);
        assert_eq!(over(0x80FF_FFFF, 0xFFFF_FFFF), 0xFFFF_FFFF);
    }

    #[test]
    fn blit_blends_inside_and_clips_outside_the_surface() {
        let mut canvas = Canvas::new((6, 4)).unwrap();
        canvas.clear(Color::rgb(0, 0, 0));

        // A 3x2 block: one opaque red pixel, the rest transparent.
        let red = Color::rgb(255, 0, 0).premultiplied();
        let block = [red, 0, 0, 0, 0, 0];
        canvas.blit(1, 1, 3, 2, &block);
        canvas.blit(5, 3, 3, 2, &[red; 6]); // hangs off the corner
        canvas.blit(-2, -1, 3, 2, &[red; 6]); // hangs off the other

        let pixels = canvas.pixels_mut().to_vec();
        let at = |x: usize, y: usize| pixels[y * 6 + x];
        assert_eq!(at(1, 1), red);
        assert_eq!(
            at(2, 1),
            Color::rgb(0, 0, 0).premultiplied(),
            "transparent leaves it"
        );
        assert_eq!(at(5, 3), red, "clipped, not dropped");
        assert_eq!(at(0, 0), red, "clipped on the other side");
        assert_eq!(at(1, 0), Color::rgb(0, 0, 0).premultiplied());

        // Nonsense sizes draw nothing and do not panic.
        canvas.blit(0, 0, 0, 5, &[]);
        canvas.blit(0, 0, 4, 4, &[red; 3]);
    }

    #[test]
    fn a_caret_is_a_bar() {
        let mut canvas = Canvas::new((10, 10)).unwrap();
        canvas.clear(Color::TRANSPARENT);
        canvas.draw_caret(4, 2, 5, 2, Color::WHITE);
        let pixels = canvas.pixels_mut().to_vec();
        let lit: Vec<(usize, usize)> = (0..100)
            .filter(|&i| pixels[i] != 0)
            .map(|i| (i % 10, i / 10))
            .collect();
        assert_eq!(lit.len(), 10);
        assert!(
            lit.iter()
                .all(|&(x, y)| (4..6).contains(&x) && (2..7).contains(&y))
        );
    }

    #[test]
    fn caret_offsets_has_one_more_entry_than_units_and_grows() {
        let font = Font::new("Segoe UI", 16, false).unwrap();
        assert_eq!(font.caret_offsets(""), [0]);

        let text = "Open 😀 é";
        let units = text.encode_utf16().count();
        let offsets = font.caret_offsets(text);
        assert_eq!(offsets.len(), units + 1);
        assert_eq!(offsets[0], 0);
        assert!(
            offsets.windows(2).all(|pair| pair[0] <= pair[1]),
            "{offsets:?}"
        );
        assert!(offsets[units] > offsets[1], "wider as it goes");

        let measured = font.measure(text).0;
        assert!(
            (offsets[units] - measured).abs() <= 2,
            "{} vs {measured}",
            offsets[units]
        );
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis_and_stays_inside_its_rectangle() {
        let font = Font::new("Segoe UI", 16, false).unwrap();
        let (width, height) = (60, 24);
        let mut canvas = Canvas::new((200, height)).unwrap();
        canvas.clear(Color::rgb(0, 0, 0));
        let rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        canvas
            .draw_text_ellipsis(
                "A very long name that cannot possibly fit",
                rect,
                &font,
                Color::WHITE,
                TextAlign::Left,
            )
            .unwrap();

        let pixels = canvas.pixels_mut().to_vec();
        let lit_right_of_rect = (0..height as usize)
            .flat_map(|y| (width as usize..200).map(move |x| (x, y)))
            .any(|(x, y)| pixels[y * 200 + x] != Color::rgb(0, 0, 0).premultiplied());
        assert!(!lit_right_of_rect, "nothing is drawn past the rectangle");
        let lit_inside = (0..width as usize).any(|x| {
            pixels[12 * 200 + x] != Color::rgb(0, 0, 0).premultiplied()
                || pixels[8 * 200 + x] != Color::rgb(0, 0, 0).premultiplied()
        });
        assert!(lit_inside, "and something is drawn inside it");
    }

    #[test]
    fn empty_text_draws_nothing_in_any_format() {
        let mut canvas = Canvas::new((80, 24)).unwrap();
        let font = Font::new("Segoe UI", 12, false).unwrap();
        let rect = RECT {
            left: 0,
            top: 0,
            right: 80,
            bottom: 24,
        };
        canvas
            .draw_text("", rect, &font, Color::WHITE, TextAlign::Left)
            .unwrap();
        canvas
            .draw_text_ellipsis("", rect, &font, Color::WHITE, TextAlign::Left)
            .unwrap();
    }

    #[test]
    fn premultiplies() {
        assert_eq!(Color::WHITE.premultiplied(), 0xFFFF_FFFF);
        assert_eq!(Color::TRANSPARENT.premultiplied(), 0);
        // Half-transparent white: every channel halved with it.
        assert_eq!(Color::WHITE.with_alpha(128).premultiplied(), 0x8080_8080);
    }

    #[test]
    fn a_rounded_border_can_be_traced_on_part_of_the_surface() {
        let interior = Color::rgb(0, 0, 255);
        let border = Color::rgb(255, 0, 0);
        let mut canvas = Canvas::new((40, 40)).unwrap();
        canvas.clear(interior);
        // The top left 30x20 of the surface is the visible window.
        let visible = RECT {
            left: 0,
            top: 0,
            right: 30,
            bottom: 20,
        };
        canvas.round_rect_corners_with_border(visible, 8, 2, border);

        let pixel =
            |canvas: &mut Canvas, x: i32, y: i32| canvas.pixels_mut()[(y * 40 + x) as usize];
        // The visible window's own corners are cut, all four…
        for (x, y) in [(0, 0), (29, 0), (0, 19), (29, 19)] {
            assert_eq!(pixel(&mut canvas, x, y), 0, "{x},{y}");
        }
        // …and its border band is traced along them…
        assert_eq!(pixel(&mut canvas, 5, 1), border.premultiplied());
        assert_eq!(pixel(&mut canvas, 24, 18), border.premultiplied());
        // …while the surface beyond it is not touched.
        assert_eq!(pixel(&mut canvas, 35, 35), interior.premultiplied());
        assert_eq!(pixel(&mut canvas, 39, 0), interior.premultiplied());
        assert_eq!(pixel(&mut canvas, 15, 39), interior.premultiplied());
    }

    #[test]
    fn a_rounded_border_traces_the_whole_curve_not_just_its_outer_edge() {
        let interior = Color::rgb(0, 0, 255);
        let border = Color::rgb(255, 0, 0);
        let mut canvas = Canvas::new((40, 40)).unwrap();
        canvas.clear(interior);
        canvas.round_corners_with_border(8, 2, border);

        let mut pixel = |x: i32, y: i32| canvas.pixels_mut()[(y * 40 + x) as usize];

        // Deep interior, nowhere near a corner box: left exactly as
        // `clear` painted it.
        assert_eq!(pixel(20, 20), interior.premultiplied());

        // The true corner pixel: outside the rounded shape entirely, so
        // erased to transparent — same as plain `round_corners`.
        assert_eq!(pixel(0, 0), 0);

        // (5, 1): inside the border band (radius 8, border_width 2, so
        // between 6px and 8px from the corner's own centre of curvature)
        // but past `coverage_at`'s antialiasing fringe, i.e. `outer`
        // saturates to 1.0 there — exactly the pixel a border-band test
        // that (a) exits early whenever `outer >= 1.0`, or (b) measures the
        // inner boundary from a *different*, radius-shifted centre instead
        // of the same one the outer boundary uses, both get wrong. Solid
        // border colour, not the interior blue underneath it.
        assert_eq!(pixel(5, 1), border.premultiplied());
    }
}
