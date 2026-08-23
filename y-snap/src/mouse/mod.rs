//! The low-level mouse hook: drag, resize, window switching, and hovering
//! over a window's minimize button.
//!
//! One `WH_MOUSE_LL` hook does all four, for the same reason
//! [`y_core::hotkey`]'s override path uses `WH_KEYBOARD_LL`: it runs on the
//! thread that installed it — this application's own message-loop thread —
//! so no DLL has to be injected into the target process to see clicks and
//! movement over windows that belong to it. HulK needed a DLL for exactly
//! this; a same-process low-level hook does not.
//!
//! Everything the hook procedure itself does has to stay cheap: Windows
//! silently removes a hook that takes longer than `LowLevelHooksTimeout`
//! (300 ms by default) to return. Moving and resizing call `SetWindowPos`
//! directly — fast, and what HulK itself did in its hook. Anything that
//! draws — showing or repainting the minimize-button popup — is posted to
//! the owner window instead of done here, so it happens on the next pass
//! through the ordinary message loop rather than inside the hook.
//!
//! # Two concerns, one hook
//!
//! Sharing the hook does not mean sharing the logic. What it does is split
//! two ways that have nothing to do with each other:
//!
//! * [`gesture`] — dragging, resizing, and switching windows. Driven by
//!   mouse buttons, the wheel, and modifier keys.
//! * [`hover`] — whether the pointer is over a window's minimize button,
//!   for [`crate::min_overlay`]'s popup. Driven by `WM_MOUSEMOVE` alone.
//!
//! This module is only the plumbing between them: installing and
//! uninstalling the hook, owning one of each behind [`HookState`], and
//! routing each message to whichever one cares about it.

use std::cell::RefCell;

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CallNextHookEx, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
        WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
    },
};
use windows::core::Result;
use y_core::ids::WM_APP_FIRST;

use crate::config::{Button, Mouse};

mod geometry;
mod gesture;
mod hover;

pub use hover::min_button_rect;

/// Posted to the owner window when the window whose minimize button the
/// pointer is over changes. `wParam` is the target `HWND`, or a null `HWND`
/// when nothing is hovered any more. Handled in [`crate::min_overlay`].
pub const WM_MIN_HOVER: u32 = WM_APP_FIRST;

/// Posted to the owner window when [`gesture`] activates a drag or resize
/// using Alt or Win — in *addition* to the direct, in-hook
/// [`y_core::hotkey::absorb_modifier_tap`] call [`gesture`] already makes.
/// This one runs its side of the comparison from the ordinary message loop
/// instead, on the theory that calling `SendInput` from inside the
/// low-level hook is itself part of what makes the direct call unreliable.
///
/// Temporary, for that side-by-side comparison. Delete once it has an
/// answer, along with its handler in [`crate`]'s message loop.
pub const WM_ABSORB_MODIFIER: u32 = WM_APP_FIRST + 2;

/// Owns the hook for as long as it lives. Uninstalled on drop.
pub struct MouseHook {
    hook: Option<HHOOK>,
}

impl MouseHook {
    /// Installs the hook. `owner` is where [`WM_MIN_HOVER`] is posted.
    ///
    /// Installs even when every gesture in `mouse` is unset: hovering the
    /// minimize button works on its own, independent of drag/resize/switch.
    pub fn install(owner: HWND, mouse: Mouse) -> Result<Self> {
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookState {
                gesture: gesture::Gesture::new(owner, mouse),
                hover: hover::Hover::new(owner),
            });
        });

        // Ignored for a low-level hook per its own documentation, but every
        // other hook in this codebase passes it, and there is no cost to
        // matching that.
        let instance = unsafe { GetModuleHandleW(None)? };
        let hook =
            unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(instance.into()), 0)? };

        Ok(Self { hook: Some(hook) })
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        if let Some(hook) = self.hook.take() {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
        STATE.with(|cell| *cell.borrow_mut() = None);
    }
}

/// The window currently shown by [`crate::min_overlay::MinOverlay`]. See
/// [`hover::Hover::set_overlay`].
pub fn set_overlay_hwnd(hwnd: HWND) {
    with_state(|state| state.hover.set_overlay(hwnd));
}

struct HookState {
    gesture: gesture::Gesture,
    hover: hover::Hover,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// Runs `body` against the hook state, doing nothing if the state is absent
/// or already borrowed — see [`super::hotkey`]'s identical rationale: this
/// can only happen from a reentrant call into the hook, on the one thread it
/// ever runs on.
fn with_state(body: impl FnOnce(&mut HookState)) {
    STATE.with(|cell| {
        if let Ok(mut state) = cell.try_borrow_mut()
            && let Some(state) = state.as_mut()
        {
            body(state);
        }
    });
}

/// Same, but for a check that decides whether to swallow the event —
/// defaults to "let it through" when the state is unavailable.
fn with_state_bool(body: impl FnOnce(&mut HookState) -> bool) -> bool {
    STATE.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut state| state.as_mut().map(body))
            .unwrap_or(false)
    })
}

impl HookState {
    /// A drag or resize in progress owns the pointer entirely; hover
    /// tracking only runs when neither is.
    fn on_move(&mut self, pt: POINT) {
        if !self.gesture.on_move(pt) {
            self.hover.update(pt);
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let message = wparam.0 as u32;

        let consumed = match message {
            WM_MOUSEMOVE => {
                with_state(|state| state.on_move(info.pt));
                false
            }
            WM_LBUTTONDOWN => {
                with_state_bool(|state| state.gesture.on_button_down(Button::Left, info.pt))
            }
            WM_LBUTTONUP => with_state_bool(|state| state.gesture.on_button_up(Button::Left)),
            WM_RBUTTONDOWN => {
                with_state_bool(|state| state.gesture.on_button_down(Button::Right, info.pt))
            }
            WM_RBUTTONUP => with_state_bool(|state| state.gesture.on_button_up(Button::Right)),
            WM_MBUTTONDOWN => {
                with_state_bool(|state| state.gesture.on_button_down(Button::Middle, info.pt))
            }
            WM_MBUTTONUP => with_state_bool(|state| state.gesture.on_button_up(Button::Middle)),
            WM_MOUSEWHEEL => {
                // MSLLHOOKSTRUCT packs the signed wheel delta into the high
                // word of mouseData (a normal WM_MOUSEWHEEL carries it in
                // wParam instead, but this is the low-level hook's struct).
                let delta = (info.mouseData >> 16) as i16;
                with_state_bool(|state| state.gesture.on_wheel(delta))
            }
            _ => false,
        };

        if consumed {
            return LRESULT(1);
        }
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
