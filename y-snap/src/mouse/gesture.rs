//! Dragging, resizing and switching windows by mouse gesture.
//!
//! Everything here is driven purely by mouse buttons, the wheel, and the
//! modifier keys held with them. What the pointer happens to be hovering —
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
//! [`y_core::hotkey::absorb_modifier_tap`] breaks that pair, called right
//! here, directly, the moment a gesture activates or ends — exactly where
//! and how HulK's own `mouseEvent` calls its equivalent `key_press_code`.
//! See that function's doc comment for what actually makes it work.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetDesktopWindow, GetShellWindow, GetWindow,
    GetWindowLongW, PostMessageW, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow,
    SetWindowPos, WS_EX_TOPMOST, WS_VISIBLE,
};
use windows::core::BOOL;
use y_core::hotkey::absorb_modifier_tap;

use crate::config::{Button, Gesture as GestureConfig, Mouse};
use crate::target;

use super::geometry::window_rect;

/// A window resized down to nothing is not a feature.
const MIN_SIZE: i32 = 32;

/// Drag, resize and window-switch state.
pub struct Gesture {
    /// Where [`super::WM_ABSORB_MODIFIER`] is posted — see
    /// [`Gesture::on_button_down`].
    owner: HWND,
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
    pub fn new(owner: HWND, config: Mouse) -> Self {
        Self {
            owner,
            config,
            drag: None,
            resize: None,
        }
    }

    /// Applies the pointer's new position to whichever gesture is in
    /// progress. Returns whether one was — `false` tells the hook there is
    /// nothing here for it, so it can fall through to hover tracking.
    pub fn on_move(&mut self, pt: POINT) -> bool {
        if let Some(drag) = &self.drag {
            let (dx, dy) = (pt.x - drag.origin.x, pt.y - drag.origin.y);
            let _ = unsafe {
                SetWindowPos(
                    drag.window,
                    None,
                    drag.start.left + dx,
                    drag.start.top + dy,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            true
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

            let _ = unsafe {
                SetWindowPos(
                    resize.window,
                    None,
                    left,
                    top,
                    right - left,
                    bottom - top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            true
        } else {
            false
        }
    }

    pub fn on_button_down(&mut self, button: Button, pt: POINT) -> bool {
        if let Some(gesture) = matched_start(self.config.drag, button)
            && self.drag.is_none()
            && self.resize.is_none()
            && let Some(window) = target::at(pt)
            && let Ok(start) = window_rect(window)
        {
            self.drag = Some(Move {
                window,
                origin: pt,
                start,
            });
            absorb_if_modified(gesture.modifiers);
            self.notify_absorb(gesture.modifiers);
            return true;
        }

        if let Some(gesture) = matched_start(self.config.resize, button)
            && self.drag.is_none()
            && self.resize.is_none()
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
            absorb_if_modified(gesture.modifiers);
            self.notify_absorb(gesture.modifiers);
            return true;
        }

        false
    }

    /// Posts [`super::WM_ABSORB_MODIFIER`] — see that constant's doc comment
    /// for why this exists alongside the direct [`absorb_if_modified`] call
    /// right above each of this method's two call sites, both on
    /// activation only: ending a gesture does not need a second attempt at
    /// something [`on_button_down`](Self::on_button_down) already tried.
    fn notify_absorb(&self, modifiers: HOT_KEY_MODIFIERS) {
        if modifiers.0 & (MOD_ALT.0 | MOD_WIN.0) != 0 {
            let _ = unsafe {
                PostMessageW(
                    Some(self.owner),
                    super::WM_ABSORB_MODIFIER,
                    WPARAM(modifiers.0 as usize),
                    LPARAM(0),
                )
            };
        }
    }

    /// Ends whichever gesture `button` drives, regardless of what modifiers
    /// are held now — a drag or resize in progress is ended by the button
    /// coming back up, full stop. Requiring the modifier still be down here
    /// would leave the gesture stuck the moment the user let go of Alt
    /// before the mouse button, which they are free to do in either order.
    ///
    /// Absorbing here too, like [`on_button_down`](Self::on_button_down),
    /// mirrors HulK's own `mouseEvent`, which calls `key_press_code` on
    /// every matching down *and* up: harmless if the modifier already came
    /// up first, and the case that matters if it has not yet.
    pub fn on_button_up(&mut self, button: Button) -> bool {
        if let Some(gesture) = matched_end(self.config.drag, button)
            && self.drag.take().is_some()
        {
            absorb_if_modified(gesture.modifiers);
            return true;
        }
        if let Some(gesture) = matched_end(self.config.resize, button)
            && self.resize.take().is_some()
        {
            absorb_if_modified(gesture.modifiers);
            return true;
        }
        false
    }

    pub fn on_wheel(&mut self, delta: i16) -> bool {
        let Some(gesture) = self.config.switch else {
            return false;
        };
        if gesture.button != Button::Wheel || gesture.modifiers != modifiers_held() {
            return false;
        }

        switch_window(delta > 0);
        true
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

/// Calls [`absorb_modifier_tap`] when `modifiers` used Alt or Win — the two
/// modifiers Windows treats specially on their own.
fn absorb_if_modified(modifiers: HOT_KEY_MODIFIERS) {
    if modifiers.0 & (MOD_ALT.0 | MOD_WIN.0) != 0 {
        absorb_modifier_tap(modifiers);
    }
}

/// The modifier keys held right now, in [`y_core::hotkey`]'s vocabulary.
fn modifiers_held() -> HOT_KEY_MODIFIERS {
    let held = |key: VIRTUAL_KEY| unsafe { GetAsyncKeyState(key.0 as i32) } as u16 & 0x8000 != 0;

    let mut modifiers = 0;
    if held(VK_LWIN) || held(VK_RWIN) {
        modifiers |= MOD_WIN.0;
    }
    if held(VK_CONTROL) {
        modifiers |= MOD_CONTROL.0;
    }
    if held(VK_MENU) {
        modifiers |= MOD_ALT.0;
    }
    if held(VK_SHIFT) {
        modifiers |= MOD_SHIFT.0;
    }
    HOT_KEY_MODIFIERS(modifiers)
}

/// Moves the foreground window to the next or previous one in z-order.
///
/// Simpler than HulK's own version, which pushed the current window to the
/// bottom of the stack before switching — an effect worth another look once
/// this has been used for a while, but not needed for a first cut.
fn switch_window(forward: bool) {
    let windows = top_level_windows();
    if windows.len() < 2 {
        return;
    }

    let current = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    let index = windows
        .iter()
        .position(|&window| window == current)
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % windows.len()
    } else {
        (index + windows.len() - 1) % windows.len()
    };

    let _ = unsafe { SetForegroundWindow(windows[next]) };
}

/// Visible, non-topmost, unowned top-level windows, in z-order — a rough
/// analogue of what Alt+Tab shows, and what the wheel gesture cycles through.
fn top_level_windows() -> Vec<HWND> {
    let mut windows: Vec<HWND> = Vec::new();
    let _ = unsafe { EnumWindows(Some(collect_window), LPARAM(&raw mut windows as isize)) };
    windows
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };

    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;

    if style & WS_VISIBLE.0 != 0 && ex_style & WS_EX_TOPMOST.0 == 0 {
        let shell = unsafe { GetShellWindow() };
        let desktop = unsafe { GetDesktopWindow() };

        // The root of hwnd's owner chain, stopping at the shell or the
        // desktop — mirrors HulK's own filter for "a real top-level window".
        let mut root = hwnd;
        loop {
            let owner = unsafe { GetWindow(root, GW_OWNER) }.unwrap_or_default();
            if owner.is_invalid() || owner == shell || owner == desktop {
                break;
            }
            root = owner;
        }

        if root == hwnd && hwnd != shell {
            windows.push(hwnd);
        }
    }

    BOOL(1)
}
