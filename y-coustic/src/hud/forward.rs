//! What lets the pointer use the HUD: the window's own handler, and a mouse
//! hook for the wheel that exists only while the HUD is on screen.
//!
//! # The handler
//!
//! The HUD window never takes the focus (`WS_EX_NOACTIVATE`), but it can still
//! receive the pointer once it is not click-through. [`Forwarder`] keeps no
//! state worth the name: it turns the mouse messages into private ones posted to
//! the owner window, which owns the [`super::Hud`] and decides what they mean
//! (`Hud::on_message`). It asks the system to tell it when the pointer leaves
//! (`TrackMouseEvent`), which is how the HUD knows to pause its timer while the
//! pointer is on it and start it again after.
//!
//! # The wheel
//!
//! A window that was never activated gets wheel messages only if the user's
//! "Scroll inactive windows" setting is on, so the HUD cannot rely on its own
//! handler for the wheel. [`WheelHook`] is a `WH_MOUSE_LL` hook installed while
//! the HUD is visible, which takes the wheel inside its rectangle and nothing
//! else: not outside the rectangle, and not with a modifier held (an `Alt+wheel`
//! binding over the HUD is still the binding's). The same rules as the bindings'
//! hook (`crate::mouse`): the hook only decides and posts, the delta is added up
//! in a `thread_local!` and the owner takes it, so a touchpad costs one audio
//! round per message-loop turn.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
    WM_MOUSEWHEEL,
};

use y_core::ids::WM_APP_FIRST;
use y_core::window::WindowHandler;

/// `WM_MOUSELEAVE`, which the `windows` crate does not bind.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Posted by the handler: the pointer came onto the HUD.
pub const WM_HUD_ENTER: u32 = WM_APP_FIRST + 32;
/// The pointer left the HUD.
pub const WM_HUD_LEAVE: u32 = WM_APP_FIRST + 33;
/// The left button went down on the HUD. `lParam`: the client position, see
/// [`point_of`].
pub const WM_HUD_DOWN: u32 = WM_APP_FIRST + 34;
/// The pointer moved with the left button held (captured).
pub const WM_HUD_MOVE: u32 = WM_APP_FIRST + 35;
/// The left button came up.
pub const WM_HUD_UP: u32 = WM_APP_FIRST + 36;
/// Posted by the wheel hook: delta waits to be taken ([`take_wheel`]).
pub const WM_HUD_WHEEL: u32 = WM_APP_FIRST + 37;

/// Posted by the tray icon's wheel hook ([`take_tray_wheel`]); not a HUD message.
pub const WM_TRAY_WHEEL: u32 = WM_APP_FIRST + 38;

/// Whether `msg` is one of the HUD's.
pub fn is_message(msg: u32) -> bool {
    (WM_HUD_ENTER..=WM_HUD_WHEEL).contains(&msg)
}

/// The client position packed in a message's `lParam` (two signed words).
pub fn point_of(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

/// The window procedure of the interactive HUD.
pub struct Forwarder {
    owner: HWND,
    /// Whether the system has been asked to say when the pointer leaves.
    tracking: bool,
}

impl Forwarder {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            tracking: false,
        }
    }

    fn post(&self, msg: u32, lparam: isize) {
        let _ = unsafe { PostMessageW(Some(self.owner), msg, WPARAM(0), LPARAM(lparam)) };
    }
}

impl WindowHandler for Forwarder {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        _wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            // A click must not take the focus from what the user is working in.
            WM_MOUSEACTIVATE => Some(LRESULT(3)), // MA_NOACTIVATE

            WM_MOUSEMOVE => {
                if !self.tracking {
                    let mut request = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    self.tracking = unsafe { TrackMouseEvent(&mut request) }.is_ok();
                    self.post(WM_HUD_ENTER, 0);
                }
                // Only a held button is a drag; a plain move is not worth a message.
                if unsafe { GetCapture() } == hwnd {
                    self.post(WM_HUD_MOVE, lparam.0);
                }
                Some(LRESULT(0))
            }
            // Tracking is one-shot: it is asked for again at the next move.
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.post(WM_HUD_LEAVE, 0);
                Some(LRESULT(0))
            }

            // The button is captured for the length of a press, so a drag that
            // leaves the HUD keeps reporting.
            WM_LBUTTONDOWN => {
                let _ = unsafe { SetCapture(hwnd) };
                self.post(WM_HUD_DOWN, lparam.0);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if unsafe { GetCapture() } == hwnd {
                    let _ = unsafe { ReleaseCapture() };
                }
                self.post(WM_HUD_UP, lparam.0);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

struct WheelState {
    owner: HWND,
    /// The message posted when delta starts to wait.
    message: u32,
    /// What takes the wheel, in screen coordinates: the HUD, or the tray icon.
    rect: RECT,
    /// The delta not yet taken by the owner.
    pending: i32,
}

/// Which of the two scoped hooks: they never share state, a HUD shown against
/// the tray icon keeps its own rectangle while the icon keeps another.
#[derive(Clone, Copy)]
enum Which {
    Hud,
    Tray,
}

thread_local! {
    static WHEEL: RefCell<Option<WheelState>> = const { RefCell::new(None) };
    static TRAY_WHEEL: RefCell<Option<WheelState>> = const { RefCell::new(None) };
}

fn cell(which: Which) -> &'static std::thread::LocalKey<RefCell<Option<WheelState>>> {
    match which {
        Which::Hud => &WHEEL,
        Which::Tray => &TRAY_WHEEL,
    }
}

/// A wheel hook scoped to one rectangle: the HUD's, or the tray icon's.
/// Uninstalled on drop.
pub struct WheelHook {
    hook: Option<HHOOK>,
    which: Which,
}

impl WheelHook {
    /// Installs the hook for a HUD at `rect`.
    pub fn install(owner: HWND, rect: RECT) -> windows::core::Result<Self> {
        Self::install_for(Which::Hud, owner, rect, WM_HUD_WHEEL, hud_wheel_hook)
    }

    /// Installs the hook for the tray icon at `rect`: its wheel posts
    /// [`WM_TRAY_WHEEL`].
    pub fn install_tray(owner: HWND, rect: RECT) -> windows::core::Result<Self> {
        Self::install_for(Which::Tray, owner, rect, WM_TRAY_WHEEL, tray_wheel_hook)
    }

    fn install_for(
        which: Which,
        owner: HWND,
        rect: RECT,
        message: u32,
        procedure: unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT,
    ) -> windows::core::Result<Self> {
        cell(which).with(|cell| {
            *cell.borrow_mut() = Some(WheelState {
                owner,
                message,
                rect,
                pending: 0,
            });
        });
        let instance = unsafe { GetModuleHandleW(None) };
        let hook = instance.and_then(|instance| unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(procedure), Some(instance.into()), 0)
        });
        match hook {
            Ok(hook) => Ok(Self {
                hook: Some(hook),
                which,
            }),
            Err(error) => {
                cell(which).with(|cell| *cell.borrow_mut() = None);
                Err(error)
            }
        }
    }

    /// The target moved: the hook takes the wheel over its new place.
    pub fn set_rect(&self, rect: RECT) {
        cell(self.which).with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.rect = rect;
            }
        });
    }
}

impl Drop for WheelHook {
    fn drop(&mut self) {
        if let Some(hook) = self.hook.take() {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
        cell(self.which).with(|cell| *cell.borrow_mut() = None);
    }
}

/// The delta [`WM_HUD_WHEEL`] announced, taken: the accumulator starts again
/// from zero.
pub fn take_wheel() -> i32 {
    take(Which::Hud)
}

/// The delta [`WM_TRAY_WHEEL`] announced, taken.
pub fn take_tray_wheel() -> i32 {
    take(Which::Tray)
}

fn take(which: Which) -> i32 {
    cell(which).with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut state| state.as_mut().map(|state| std::mem::take(&mut state.pending)))
            .unwrap_or(0)
    })
}

unsafe extern "system" fn hud_wheel_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { wheel_hook(Which::Hud, code, wparam, lparam) }
}

unsafe extern "system" fn tray_wheel_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { wheel_hook(Which::Tray, code, wparam, lparam) }
}

unsafe fn wheel_hook(which: Which, code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let delta = (info.mouseData >> 16) as i16;
        // Our own re-injected wheel, and any wheel with a modifier held (a
        // binding's), are not ours.
        if delta != 0
            && info.dwExtraInfo != crate::mouse::REINJECTED
            && crate::mouse::modifiers_held() == 0
            && claim(which, info.pt, delta)
        {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Adds `delta` to what waits for the owner, when the pointer is in the
/// rectangle. True when it did (the event is then swallowed).
fn claim(which: Which, point: windows::Win32::Foundation::POINT, delta: i16) -> bool {
    cell(which).with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else {
            return false;
        };
        let Some(state) = state.as_mut() else {
            return false;
        };
        let inside = (state.rect.left..state.rect.right).contains(&point.x)
            && (state.rect.top..state.rect.bottom).contains(&point.y);
        if !inside {
            return false;
        }
        let was_idle = state.pending == 0;
        state.pending += i32::from(delta);
        if was_idle {
            let _ = unsafe { PostMessageW(Some(state.owner), state.message, WPARAM(0), LPARAM(0)) };
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_point_reads_back_with_its_signs() {
        let packed = (-5i16 as u16 as isize) | ((12i16 as u16 as isize) << 16);
        assert_eq!(point_of(LPARAM(packed)), (-5, 12));
    }

    #[test]
    fn only_the_huds_messages_are_its_own() {
        assert!(is_message(WM_HUD_ENTER));
        assert!(is_message(WM_HUD_WHEEL));
        assert!(!is_message(WM_APP_FIRST));
        assert!(!is_message(WM_HUD_WHEEL + 1));
    }
}
