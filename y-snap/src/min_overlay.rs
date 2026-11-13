//! The popup shown when the pointer hovers a window's minimize button.
//!
//! [`crate::mouse`] detects the hover from inside a low-level hook and posts
//! [`crate::mouse::WM_MIN_HOVER`] to the owner window; everything from there
//! — creating the popup the first time, positioning it, drawing it, and
//! turning a click into an [`Action`] — happens here, on the ordinary
//! message loop rather than inside the hook (see the module docs of
//! [`crate::mouse`] for why that split exists).
//!
//! The tiles form a grid `<minbutton columns="…">` wide (one row when
//! unset). The tile under the pointer is painted in the palette's hover
//! colours, and a tile whose toggle is on for the target window (maximized,
//! pinned, always on top…) in the active ones, as in the action overlay. The
//! popup window posts [`WM_MIN_TILE_HOVER`] as the pointer moves
//! and the owner repaints through [`MinOverlay::set_hover`], the same shape
//! as the action overlay's.

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
        WindowsAndMessaging::{PostMessageW, WM_LBUTTONUP, WM_MOUSEMOVE},
    },
};
use y_core::{
    Result,
    ids::WM_APP_FIRST,
    memory,
    monitor::Monitor,
    overlay::{ColorFont, Overlay, OverlaySpec, TextAlign, release_color_text},
    window::WindowHandler,
};

use crate::{
    action::Action,
    actions,
    config::{Overlay as OverlayConfig, QuickAction},
    mouse,
};

/// Posted to the owner window when a tile is clicked. `wParam` is its index
/// into the [`MinOverlay`] that posted it.
pub const WM_MIN_ACTION: u32 = WM_APP_FIRST + 1;

/// Posted to the owner window when the pointer moves onto, off, or between
/// tiles. `wParam` is the tile index, `usize::MAX` for "off every tile".
pub const WM_MIN_TILE_HOVER: u32 = WM_APP_FIRST + 7;

/// Sent to a window when the pointer leaves its client area (`TME_LEAVE`).
const WM_MOUSELEAVE: u32 = 0x02A3;

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
    /// Tiles per row as configured; `0` means all on one row.
    columns: u32,
    overlay: Option<Overlay>,
    target: Option<HWND>,
    /// The tile under the pointer, painted in the palette's hover colours.
    hovered: Option<usize>,
}

impl MinOverlay {
    pub fn new(owner: HWND, items: Vec<QuickAction>, columns: u32) -> Self {
        Self {
            owner,
            items,
            columns,
            overlay: None,
            target: None,
            hovered: None,
        }
    }

    /// Shows the popup under `window`'s minimize button. Does nothing if the
    /// configuration has no `<minbutton>` actions — the common case, and one
    /// [`crate::mouse`] cannot tell on its own without knowing the config.
    ///
    /// `measured` is the button's real rectangle when the window could be
    /// asked for it ([`mouse::probe_min_button`]); otherwise the popup falls
    /// back to the geometric estimate.
    pub fn show(
        &mut self,
        window: HWND,
        config: &OverlayConfig,
        measured: Option<RECT>,
    ) -> Result<()> {
        if self.items.is_empty() {
            return Ok(());
        }

        let Some(button) = measured.or_else(|| mouse::min_button_rect(window)) else {
            self.hide();
            return Ok(());
        };
        let top = measured
            .map(|rect| rect.bottom)
            .or_else(|| mouse::min_popup_top(window))
            .unwrap_or(button.bottom);

        if self.overlay.is_none() {
            let handler = ClickHandler {
                owner: self.owner,
                tiles: self.items.len(),
                columns: self.columns(),
                hovered: None,
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
        self.hovered = None;
        self.paint(config)?;

        // Centred under the button, then pulled back onto the screen: on a
        // maximized window the button sits against the right edge, and a
        // popup wider than the button would hang off it. The popup still
        // covers the button's horizontal span afterwards, so the pointer
        // can still travel straight down onto it.
        let size = self.size();
        let position = Monitor::from_window(window).keep_inside(
            POINT {
                x: button.left + (button.right - button.left) / 2 - size.0 / 2,
                y: top + GAP,
            },
            size,
        );
        self.overlay
            .as_ref()
            .expect("created above")
            .show_at(position)?;
        Ok(())
    }

    /// Whether the popup is already showing for `window`.
    pub fn is_showing_for(&self, window: HWND) -> bool {
        self.target == Some(window)
    }

    /// Hides the popup. Safe to call when it is already hidden.
    pub fn hide(&mut self) {
        self.target = None;
        self.hovered = None;
        if let Some(overlay) = &self.overlay {
            overlay.hide();
        }
        mouse::set_overlay_hwnd(HWND::default());

        // This popup hides far too often to trim every time, and only rarely
        // has anything new to give back: trim on the hide after a first
        // render, which is the one that left Direct2D's pages behind.
        if release_color_text() {
            let _ = memory::trim_working_set();
        }
    }

    /// The pointer moved onto, off, or between tiles — `None` for "off every
    /// tile". Called on [`WM_MIN_TILE_HOVER`]; a no-op when nothing changed
    /// or when the popup is not showing (a stale message).
    pub fn set_hover(&mut self, index: Option<usize>, config: &OverlayConfig) -> Result<()> {
        if self.hovered == index || self.target.is_none() {
            return Ok(());
        }
        self.hovered = index;
        self.paint(config)?;
        if let Some(overlay) = &self.overlay {
            overlay.refresh()?;
        }
        Ok(())
    }

    /// The action and target for tile `index`, if the popup is currently
    /// showing one — `None` for a stale click that arrived after the popup
    /// was already hidden by something else.
    pub fn action_at(&self, index: usize) -> Option<(Action, HWND)> {
        let item = self.items.get(index)?;
        Some((item.action, self.target?))
    }

    /// Tiles per row: the configured count capped at the number of tiles, or
    /// all of them when unset.
    fn columns(&self) -> usize {
        let tiles = self.items.len().max(1);
        match self.columns as usize {
            0 => tiles,
            columns => columns.min(tiles),
        }
    }

    fn size(&self) -> (i32, i32) {
        let columns = self.columns();
        let rows = self.items.len().max(1).div_ceil(columns);
        (TILE.0 * columns as i32, TILE.1 * rows as i32)
    }

    fn paint(&mut self, config: &OverlayConfig) -> Result<()> {
        let columns = self.columns();
        let target = self.target;
        let Some(overlay) = &mut self.overlay else {
            return Ok(());
        };
        let bounds = overlay.bounds();
        let palette = &config.palette;

        let canvas = overlay.canvas_mut();
        canvas.clear(palette.back.with_alpha(config.alpha));

        let font = ColorFont::new("Segoe UI Emoji", 18, false)?;
        for (index, item) in self.items.iter().enumerate() {
            let (column, row) = ((index % columns) as i32, (index / columns) as i32);
            let rect = RECT {
                left: bounds.left + column * TILE.0,
                top: bounds.top + row * TILE.1,
                right: bounds.left + (column + 1) * TILE.0,
                bottom: bounds.top + (row + 1) * TILE.1,
            };
            // Same precedence as the action overlay's tiles (`tile_colors`):
            // hover wins over the toggle's on-state. The state is read at
            // each paint, so it is current whenever the popup is shown.
            let active = target
                .is_some_and(|window| actions::toggle_state(item.action, window).unwrap_or(false));
            let text = if self.hovered == Some(index) {
                canvas.fill_rect(rect, palette.hover_back);
                palette.hover_text
            } else if active {
                canvas.fill_rect(rect, palette.active_back);
                palette.active_text
            } else {
                palette.text
            };
            let glyph = item
                .icon
                .as_deref()
                .or(item.label.as_deref())
                .unwrap_or("?");
            canvas.draw_text_color(glyph, rect, &font, text, TextAlign::Centre)?;
        }
        canvas.round_corners(config.corner_radius as i32);
        Ok(())
    }
}

/// The tile under the client-area point `(x, y)` of a grid `columns` wide
/// holding `tiles` tiles — `None` outside the grid, or on the empty cells
/// that end an incomplete last row.
fn tile_at(x: i32, y: i32, columns: usize, tiles: usize) -> Option<usize> {
    if x < 0 || y < 0 {
        return None;
    }
    let (column, row) = ((x / TILE.0) as usize, (y / TILE.1) as usize);
    let index = row * columns + column;
    (column < columns && index < tiles).then_some(index)
}

/// The popup's own window handler: turns a click into a message the owner
/// window can act on with the rest of [`crate`]'s state, which this handler
/// — living inside the popup window, not the application — does not have.
/// Likewise for the pointer moving between tiles, so the owner can repaint.
struct ClickHandler {
    owner: HWND,
    tiles: usize,
    columns: usize,
    /// Only to decide when a [`WM_MIN_TILE_HOVER`] is worth posting; the
    /// copy in [`MinOverlay`] is the one that drives painting.
    hovered: Option<usize>,
}

impl ClickHandler {
    fn post_hover(&self) {
        let index = self.hovered.unwrap_or(usize::MAX);
        let _ = unsafe {
            PostMessageW(
                Some(self.owner),
                WM_MIN_TILE_HOVER,
                WPARAM(index),
                LPARAM(0),
            )
        };
    }
}

impl WindowHandler for ClickHandler {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        _wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let x = (lparam.0 & 0xFFFF) as i16 as i32;
        let y = (lparam.0 >> 16 & 0xFFFF) as i16 as i32;

        match msg {
            WM_LBUTTONUP => {
                if let Some(index) = tile_at(x, y, self.columns, self.tiles) {
                    let _ = unsafe {
                        PostMessageW(Some(self.owner), WM_MIN_ACTION, WPARAM(index), LPARAM(0))
                    };
                }
                Some(LRESULT(0))
            }

            WM_MOUSEMOVE => {
                let hit = tile_at(x, y, self.columns, self.tiles);
                if hit != self.hovered {
                    self.hovered = hit;
                    self.post_hover();
                }
                // One-shot, so re-armed on every move.
                let mut event = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut event) };
                Some(LRESULT(0))
            }

            WM_MOUSELEAVE => {
                if self.hovered.take().is_some() {
                    self.post_hover();
                }
                Some(LRESULT(0))
            }

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TILE, tile_at};

    #[test]
    fn finds_the_tile_in_a_grid() {
        // Five tiles, two per row: (0 1) (2 3) (4 _).
        let at = |column: i32, row: i32| tile_at(column * TILE.0 + 1, row * TILE.1 + 1, 2, 5);
        assert_eq!(at(0, 0), Some(0));
        assert_eq!(at(1, 0), Some(1));
        assert_eq!(at(0, 2), Some(4));
        // The empty cell after the last tile, and past the right edge.
        assert_eq!(at(1, 2), None);
        assert_eq!(at(2, 0), None);
        assert_eq!(tile_at(-1, 0, 2, 5), None);
    }
}
