//! The popup shown when the pointer hovers a window's minimize button.
//!
//! [`crate::mouse`] detects the hover from inside a low-level hook and posts
//! [`crate::mouse::WM_MIN_HOVER`] to the owner window; everything from there
//! — creating the popup the first time, positioning it, drawing it, and
//! turning a click into an [`Action`] — happens here, on the ordinary
//! message loop rather than inside the hook (see the module docs of
//! [`crate::mouse`] for why that split exists).

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, WM_LBUTTONUP},
};
use y_core::{
    Result,
    ids::WM_APP_FIRST,
    overlay::{Font, Overlay, OverlaySpec, TextAlign},
    window::WindowHandler,
};

use crate::{
    action::Action,
    config::{Overlay as OverlayConfig, QuickAction},
    mouse,
};

/// Posted to the owner window when a tile is clicked. `wParam` is its index
/// into the [`MinOverlay`] that posted it.
pub const WM_MIN_ACTION: u32 = WM_APP_FIRST + 1;

const CLASS: &str = "YpoLite.YSnap.MinButton";
const TILE: (i32, i32) = (72, 40);
/// How far below the minimize button the popup sits. Flush against it, not
/// merely close: a gap is a pixel the pointer can wander off into on its way
/// down, which would count as leaving the button and hide the popup before
/// it is reached.
const GAP: i32 = 0;

/// Owns the popup window (created lazily, on first use) and which window it
/// currently applies to.
pub struct MinOverlay {
    owner: HWND,
    items: Vec<QuickAction>,
    overlay: Option<Overlay>,
    target: Option<HWND>,
}

impl MinOverlay {
    pub fn new(owner: HWND, items: Vec<QuickAction>) -> Self {
        Self {
            owner,
            items,
            overlay: None,
            target: None,
        }
    }

    /// Shows the popup under `window`'s minimize button. Does nothing if the
    /// configuration has no `<minbutton>` actions — the common case, and one
    /// [`crate::mouse`] cannot tell on its own without knowing the config.
    pub fn show(&mut self, window: HWND, config: &OverlayConfig) -> Result<()> {
        if self.items.is_empty() {
            return Ok(());
        }

        let Some(button) = mouse::min_button_rect(window) else {
            self.hide();
            return Ok(());
        };

        if self.overlay.is_none() {
            let handler = ClickHandler {
                owner: self.owner,
                tiles: self.items.len(),
            };
            let spec = OverlaySpec::new(CLASS, self.size()).interactive();
            let overlay = Overlay::with_handler(&spec, Box::new(handler))?;
            self.overlay = Some(overlay);
        }
        // Every show, not only the first: `hide` unconditionally clears this
        // back to null (see its own doc comment), since it also has to
        // handle the case where the popup was never created at all. Leaving
        // it null past the first hide would mean `hover::Hover::update`'s
        // "the pointer reached the popup" guard never matches again for the
        // rest of this `MinOverlay`'s life — the popup would hide itself
        // the instant the pointer left the button on its way down to it,
        // on every hover after the first.
        mouse::set_overlay_hwnd(self.overlay.as_ref().expect("created above").hwnd());

        self.target = Some(window);
        self.paint(config)?;

        let size = self.size();
        let position = POINT {
            x: button.left + (button.right - button.left) / 2 - size.0 / 2,
            y: button.bottom + GAP,
        };
        self.overlay
            .as_ref()
            .expect("created above")
            .show_at(position)?;
        Ok(())
    }

    /// Hides the popup. Safe to call when it is already hidden.
    pub fn hide(&mut self) {
        self.target = None;
        if let Some(overlay) = &self.overlay {
            overlay.hide();
        }
        mouse::set_overlay_hwnd(HWND::default());
    }

    /// The action and target for tile `index`, if the popup is currently
    /// showing one — `None` for a stale click that arrived after the popup
    /// was already hidden by something else.
    pub fn action_at(&self, index: usize) -> Option<(Action, HWND)> {
        let item = self.items.get(index)?;
        Some((item.action, self.target?))
    }

    fn size(&self) -> (i32, i32) {
        (TILE.0 * self.items.len().max(1) as i32, TILE.1)
    }

    fn paint(&mut self, config: &OverlayConfig) -> Result<()> {
        let Some(overlay) = &mut self.overlay else {
            return Ok(());
        };
        let bounds = overlay.bounds();
        let palette = &config.palette;

        let canvas = overlay.canvas_mut();
        canvas.clear(palette.back.with_alpha(config.alpha));

        let font = Font::new("Segoe UI Emoji", 18, false)?;
        for (index, item) in self.items.iter().enumerate() {
            let rect = RECT {
                left: bounds.left + index as i32 * TILE.0,
                top: bounds.top,
                right: bounds.left + (index as i32 + 1) * TILE.0,
                bottom: bounds.bottom,
            };
            let glyph = item
                .icon
                .as_deref()
                .or(item.label.as_deref())
                .unwrap_or("?");
            canvas.draw_text(glyph, rect, &font, palette.text, TextAlign::Centre)?;
        }
        canvas.round_corners(config.corner_radius as i32);
        Ok(())
    }
}

/// The popup's own window handler: turns a click into a message the owner
/// window can act on with the rest of [`crate`]'s state, which this handler
/// — living inside the popup window, not the application — does not have.
struct ClickHandler {
    owner: HWND,
    tiles: usize,
}

impl WindowHandler for ClickHandler {
    fn handle_message(
        &mut self,
        _hwnd: HWND,
        msg: u32,
        _wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if msg == WM_LBUTTONUP && self.tiles > 0 {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let index = (x / TILE.0).clamp(0, self.tiles as i32 - 1) as usize;
            let _ =
                unsafe { PostMessageW(Some(self.owner), WM_MIN_ACTION, WPARAM(index), LPARAM(0)) };
            return Some(LRESULT(0));
        }
        None
    }
}
