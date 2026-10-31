//! Dragging and resizing windows by mouse gesture.
//!
//! Everything here is driven purely by mouse buttons and the modifier keys
//! held with them. What the pointer happens to be hovering —
//! the minimize button popup — is [`super::hover`]'s business, not this
//! module's; the two are kept apart so each can be read on its own.
//!
//! # Swallowing the modifier
//!
//! A drag or resize bound to Alt or Win has a problem neither HulK nor
//! y-snap's own hotkeys have to deal with: the modifier is read with
//! [`GetAsyncKeyState`] rather than intercepted, so nothing stops the key
//! from also reaching the shell. If the user drags with Alt and lets go of
//! Alt after the mouse button, Windows sees a lone Alt (or Win) press
//! followed by a release with nothing in between — the exact gesture that
//! opens the window's system menu or the Start menu.
//!
//! [`super::modifier_guard`] breaks that pair. Activation here only arms
//! it — see [`Gesture::on_button_down`] — the correction itself fires later,
//! from that module's own keyboard hook, synchronously at the modifier's
//! real key-up, which is the part that actually matters; see that module's
//! doc comment.
//!
//! # Why `on_move` does not call `SetWindowPos` itself
//!
//! It used to, for both a drag and a resize, directly from inside the
//! low-level hook. That is fine for a move — [`SetWindowPos`] is called with
//! `SWP_NOSIZE`, so nothing about the window's size changes and `WM_SIZE`
//! is not sent. A resize is a different story: [`SetWindowPos`] there *does*
//! change the size, which dispatches `WM_SIZE` to the target window, and if
//! that window belongs to another thread — normal, since it is essentially
//! always a different process — delivering that message blocks the calling
//! thread until the target's own message loop gets around to handling it.
//! A window whose resize handler does anything non-trivial (a browser, an
//! IDE, most Office apps) can easily take longer than the hook's own
//! `LowLevelHooksTimeout` budget, and Windows drops the hook exactly as if
//! it had hung: the resize then never receives its button-up and looks
//! stuck, with no pattern to when it happens beyond "which window, how busy
//! its thread was right then" — both invisible from here.
//!
//! [`on_move`](Gesture::on_move) now only computes the target rect; applying
//! it is [`super`]'s job, deferred to the ordinary message loop the same way
//! it already defers drawing the minimize-button popup, so the only thing
//! that can block is this thread's own message pump, which has no such
//! timeout.

use windows::Win32::Foundation::{HWND, POINT, RECT};

use crate::config::{Button, Gesture as GestureConfig, Mouse};
use crate::target;

use super::geometry::window_rect;
use super::modifier_guard;
use super::modifiers_held;

/// A window resized down to nothing is not a feature.
const MIN_SIZE: i32 = 32;

/// Drag and resize state.
pub struct Gesture {
    config: Mouse,
    drag: Option<Move>,
    resize: Option<Resize>,
}

struct Move {
    window: HWND,
    origin: POINT,
    start: RECT,
}

struct Resize {
    window: HWND,
    origin: POINT,
    start: RECT,
    /// Which edge each axis grows from: the corner nearest the pointer at
    /// the moment the resize started stays fixed, the opposite one moves.
    invert_x: bool,
    invert_y: bool,
}

impl Gesture {
    pub fn new(config: Mouse) -> Self {
        Self {
            config,
            drag: None,
            resize: None,
        }
    }

    /// Whether a drag or resize is in progress — what a mouse shortcut
    /// pressed meanwhile is swallowed for.
    pub fn active(&self) -> bool {
        self.drag.is_some() || self.resize.is_some()
    }

    /// Computes the target rect for whichever gesture is in progress, for
    /// the caller to actually apply — see this module's doc comment for why
    /// that has to happen somewhere other than here. `None` means there is
    /// nothing here for it, so it can fall through to hover tracking.
    pub fn on_move(&mut self, pt: POINT) -> Option<(HWND, RECT)> {
        if let Some(drag) = &self.drag {
            let (dx, dy) = (pt.x - drag.origin.x, pt.y - drag.origin.y);
            let (left, top) = (drag.start.left + dx, drag.start.top + dy);
            let (width, height) = (
                drag.start.right - drag.start.left,
                drag.start.bottom - drag.start.top,
            );
            Some((
                drag.window,
                RECT {
                    left,
                    top,
                    right: left + width,
                    bottom: top + height,
                },
            ))
        } else if let Some(resize) = &self.resize {
            let (dx, dy) = (pt.x - resize.origin.x, pt.y - resize.origin.y);

            let (mut left, mut right) = (resize.start.left, resize.start.right);
            if resize.invert_x {
                right += dx;
            } else {
                left += dx;
            }
            let (mut top, mut bottom) = (resize.start.top, resize.start.bottom);
            if resize.invert_y {
                bottom += dy;
            } else {
                top += dy;
            }

            if right - left < MIN_SIZE {
                if resize.invert_x {
                    right = left + MIN_SIZE;
                } else {
                    left = right - MIN_SIZE;
                }
            }
            if bottom - top < MIN_SIZE {
                if resize.invert_y {
                    bottom = top + MIN_SIZE;
                } else {
                    top = bottom - MIN_SIZE;
                }
            }

            Some((
                resize.window,
                RECT {
                    left,
                    top,
                    right,
                    bottom,
                },
            ))
        } else {
            None
        }
    }

    pub fn on_button_down(&mut self, button: Button, pt: POINT) -> bool {
        let drag_match = matched_start(self.config.drag, button);
        let resize_match = matched_start(self.config.resize, button);

        // A gesture already in progress owns the pointer until it ends.
        // Falling through here instead — the shape this had before — sends
        // the second button straight to whatever's under the cursor: with
        // the stock config (Alt+Left drag, Alt+Right resize) a stray or
        // intentional Alt+Right while already Alt-dragging left-handed
        // reached the dragged application as an ordinary right-click and
        // opened its context menu mid-drag. Swallowing it here is a plain
        // "not now", not a gesture of its own.
        if (drag_match.is_some() || resize_match.is_some())
            && (self.drag.is_some() || self.resize.is_some())
        {
            return true;
        }

        if let Some(gesture) = drag_match
            && let Some(window) = target::at(pt)
            && let Ok(start) = window_rect(window)
        {
            self.drag = Some(Move {
                window,
                origin: pt,
                start,
            });
            modifier_guard::arm(gesture.modifiers);
            return true;
        }

        if let Some(gesture) = resize_match
            && let Some(window) = target::at(pt)
            && let Ok(start) = window_rect(window)
        {
            let invert_x = (pt.x - start.left) > (start.right - start.left) / 2;
            let invert_y = (pt.y - start.top) > (start.bottom - start.top) / 2;
            self.resize = Some(Resize {
                window,
                origin: pt,
                start,
                invert_x,
                invert_y,
            });
            modifier_guard::arm(gesture.modifiers);
            return true;
        }

        false
    }

    /// Ends whichever gesture `button` drives, regardless of what modifiers
    /// are held now — a drag or resize in progress is ended by the button
    /// coming back up, full stop. Requiring the modifier still be down here
    /// would leave the gesture stuck the moment the user let go of Alt
    /// before the mouse button, which they are free to do in either order.
    ///
    /// Nothing here re-arms or otherwise touches [`modifier_guard`]: it was
    /// armed for this gesture's modifiers back in
    /// [`on_button_down`](Self::on_button_down) and stays armed on its own,
    /// independent of the mouse button, until the modifier's real key-up
    /// actually happens — which this end of the gesture cannot assume has
    /// occurred yet.
    pub fn on_button_up(&mut self, button: Button) -> bool {
        if matched_end(self.config.drag, button).is_some() && self.drag.take().is_some() {
            return true;
        }
        if matched_end(self.config.resize, button).is_some() && self.resize.take().is_some() {
            return true;
        }
        false
    }
}

/// `gesture` if it is bound to `button` and its modifiers are exactly the
/// ones held right now — what starts a gesture.
fn matched_start(gesture: Option<GestureConfig>, button: Button) -> Option<GestureConfig> {
    gesture.filter(|gesture| gesture.button == button && gesture.modifiers == modifiers_held())
}

/// `gesture` if it is bound to `button`, regardless of what modifiers are
/// held now — what ends a gesture. See [`Gesture::on_button_up`] for why the
/// modifiers do not matter here the way they do for [`matched_start`].
fn matched_end(gesture: Option<GestureConfig>, button: Button) -> Option<GestureConfig> {
    gesture.filter(|gesture| gesture.button == button)
}
