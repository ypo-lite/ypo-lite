//! The low-level mouse hook: drag, resize, and hovering over a window's
//! minimize button.
//!
//! One `WH_MOUSE_LL` hook does all three, for the same reason
//! [`y_core::hotkey`]'s override path uses `WH_KEYBOARD_LL`: it runs on the
//! thread that installed it — this application's own message-loop thread —
//! so no DLL has to be injected into the target process to see clicks and
//! movement over windows that belong to it. HulK needed a DLL for exactly
//! this; a same-process low-level hook does not.
//!
//! Everything the hook procedure itself does has to stay cheap: Windows
//! silently removes a hook that takes longer than `LowLevelHooksTimeout`
//! (300 ms by default) to return. Anything that draws — showing or
//! repainting the minimize-button popup — is posted to the owner window
//! instead of done here, so it happens on the next pass through the
//! ordinary message loop rather than inside the hook. Actually moving or
//! resizing a window is posted the same way and for the same reason — see
//! [`gesture`]'s own doc comment for why a resize's `SetWindowPos` in
//! particular cannot safely happen inside the hook.
//!
//! # Two concerns, one hook
//!
//! Sharing the hook does not mean sharing the logic. What it does is split
//! two ways that have nothing to do with each other:
//!
//! * [`gesture`] — dragging and resizing windows. Driven by mouse buttons
//!   and modifier keys.
//! * [`hover`] — whether the pointer is over a window's minimize button,
//!   for [`crate::min_overlay`]'s popup. Driven by `WM_MOUSEMOVE` alone.
//!
//! This module is only the plumbing between them: installing and
//! uninstalling the hook, owning one of each behind [`HookState`], and
//! routing each message to whichever one cares about it.
//!
//! # The watchdog
//!
//! `LowLevelHooksTimeout` (see above) cuts both ways: Windows does not just
//! decline to wait past 300 ms, it silently *unhooks* a callback that
//! overruns it, with nothing telling the application that happened. Lost
//! that way, this hook stops seeing anything at all — clicks, moves, all of
//! it — until the process reinstalls it, which nothing here otherwise ever
//! has a reason to do. The symptom is exactly as confusing as it sounds: a
//! drag that never receives its button-up and so never ends, and normal
//! clicks nearby that quietly stop registering with this hook, both for no
//! visible reason. [`MouseHook::check_liveness`] is AltSnap's own recovery
//! for this (`REHOOK_TIMER`): polled every few seconds from an ordinary
//! `WM_TIMER`, comparing `GetCursorPos` against the last point
//! [`mouse_hook`]'s own `WM_MOUSEMOVE` arm recorded. Real mouse movement
//! reaches a live hook essentially immediately, so any disagreement between
//! the two means the hook is gone, and it's reinstalled.

use std::cell::{Cell, RefCell};

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    },
    UI::WindowsAndMessaging::{
        CallNextHookEx, GetCursorPos, HC_ACTION, HHOOK, KillTimer, MSLLHOOKSTRUCT, PostMessageW,
        SWP_NOACTIVATE, SWP_NOZORDER, SetTimer, SetWindowPos, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
        WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN,
        WM_XBUTTONUP,
    },
};
use windows::core::Result;
use y_core::ids::WM_APP_FIRST;
use y_core::monitor::Monitor;

use crate::action::Target;
use crate::config::{Button, Mouse, MouseShortcut, MouseTrigger};
use y_core::target;

mod geometry;
mod gesture;
mod hover;
mod shortcut;

pub use geometry::visual_window_rect;
pub use hover::{confirm_min_button, min_button_rect, min_popup_top, probe_min_button};

/// Posted to the owner window when the window whose minimize button the
/// pointer is over changes — never carries anything worth reading in
/// `wParam`; it means "hide whatever the popup is showing", both when the
/// button was just left and when a different one was just entered. Showing
/// the popup back up happens later, on [`MIN_HOVER_TIMER_ID`] — see
/// [`hover`]'s module doc comment for why the two are split. Handled by
/// `crate`'s own `on_min_hover`.
pub const WM_MIN_HOVER: u32 = WM_APP_FIRST;

/// Posted to the owner window when a plain right-click lands on, and lifts
/// off, the same minimize button. `wParam` is the target `HWND`. Handled by
/// `crate`'s own `on_min_rightclick`.
pub const WM_MIN_RIGHTCLICK: u32 = WM_APP_FIRST + 3;

/// Posted to the owner window when a mouse shortcut fires. `wParam` is the
/// index into the list given to [`MouseHook::install`], `lParam` the target
/// `HWND` (null for an action that needs no window). The window was chosen in
/// the hook, at the press: resolving it later could pick another one if the
/// pointer had moved on. Handled by `crate`'s own `on_mouse_shortcut`.
pub const WM_MOUSE_SHORTCUT: u32 = WM_APP_FIRST + 8;

/// Posted to the owner window when a drag or resize in progress has a new
/// target rect to apply. The window and rect are read from
/// [`PENDING_APPLY`], not carried in the message, the same shape as
/// [`WM_MIN_HOVER`]'s neighbour [`crate::min_overlay::WM_MIN_ACTION`] — see
/// [`gesture`]'s doc comment for why `SetWindowPos` itself has to happen
/// here and not inside the hook. Handled in [`crate`]'s message loop.
pub const WM_APPLY_GESTURE: u32 = WM_APP_FIRST + 2;

thread_local! {
    /// Set by [`queue_apply`], consumed by [`apply_pending_gesture`]. Always
    /// holds the *latest* rect a gesture has asked for — a burst of
    /// `WM_MOUSEMOVE`s between two passes through the message loop overwrite
    /// each other here rather than queuing up [`WM_APPLY_GESTURE`] messages
    /// one per move.
    static PENDING_APPLY: Cell<Option<(HWND, RECT)>> = const { Cell::new(None) };
}

/// Records `(window, rect)` as the next thing to apply and, unless one is
/// already queued, posts [`WM_APPLY_GESTURE`] to `owner` to go get it.
fn queue_apply(owner: HWND, window: HWND, rect: RECT) {
    let already_queued = PENDING_APPLY.with(|cell| cell.replace(Some((window, rect))).is_some());
    if !already_queued {
        let _ = unsafe { PostMessageW(Some(owner), WM_APPLY_GESTURE, WPARAM(0), LPARAM(0)) };
    }
}

/// Handles `WM_APPLY_GESTURE` — applies whatever [`queue_apply`] last
/// recorded, if it hasn't already been superseded by a newer one.
pub fn apply_pending_gesture() {
    let Some((window, rect)) = PENDING_APPLY.with(|cell| cell.take()) else {
        return;
    };
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

/// Posted to the owner window when the pointer of a pinned drag (a maximized
/// window, with `allow-maximized` off) has reached another monitor. The
/// window and the point are read from [`PENDING_MONITOR`]. Handled in
/// [`crate`]'s message loop by [`apply_pending_monitor`].
pub const WM_GESTURE_MONITOR: u32 = WM_APP_FIRST + 9;

thread_local! {
    /// Set by [`queue_monitor`], consumed by [`apply_pending_monitor`]. The
    /// latest crossing wins, as with [`PENDING_APPLY`]. It carries the point
    /// and not a monitor handle: a monitor unplugged meanwhile cannot be
    /// dereferenced, and the point is resolved when it is used.
    static PENDING_MONITOR: Cell<Option<(HWND, POINT)>> = const { Cell::new(None) };
}

fn queue_monitor(owner: HWND, window: HWND, pt: POINT) {
    let already_queued = PENDING_MONITOR.with(|cell| cell.replace(Some((window, pt))).is_some());
    if !already_queued {
        let _ = unsafe { PostMessageW(Some(owner), WM_GESTURE_MONITOR, WPARAM(0), LPARAM(0)) };
    }
}

/// Handles `WM_GESTURE_MONITOR`: takes the window to the monitor the pointer
/// is on now. Does nothing when the window is gone or already there, which
/// absorbs a quick wobble across a boundary.
pub fn apply_pending_monitor() {
    let Some((window, pt)) = PENDING_MONITOR.with(|cell| cell.take()) else {
        return;
    };
    if !crate::window::exists(window) {
        return;
    }
    let target = Monitor::from_point(pt);
    if Monitor::from_window(window).handle == target.handle {
        return;
    }
    crate::window::move_to_monitor(window, &target);
}

/// `wParam` of the `WM_TIMER` that drives [`MouseHook::check_liveness`] —
/// see this module's doc comment. Handled in [`crate`]'s message loop.
pub const REHOOK_TIMER_ID: usize = 1;

/// `wParam` of the `WM_TIMER` that fires once the pointer has sat still
/// over a minimize button for its configured dwell delay — see [`hover`]'s
/// module doc comment. Armed and reset by [`hover::Hover::update`], read
/// (and stopped) by [`take_min_hover_target`]. Handled in [`crate`]'s
/// message loop.
pub const MIN_HOVER_TIMER_ID: usize = 2;

/// How often the watchdog polls. Frequent enough to notice a dropped hook
/// well inside the time it would take a user to get confused by it; rare
/// enough that `GetCursorPos` once per tick is nowhere near worth counting.
const REHOOK_INTERVAL_MS: u32 = 5_000;

thread_local! {
    /// The last point [`mouse_hook`]'s own `WM_MOUSEMOVE` arm recorded, kept
    /// outside [`HookState`] since the watchdog needs it whether or not a
    /// gesture or hover is using it — see [`MouseHook::check_liveness`].
    static LAST_POINT: Cell<Option<POINT>> = const { Cell::new(None) };
}

/// Owns the hook for as long as it lives. Uninstalled on drop.
pub struct MouseHook {
    owner: HWND,
    hook: Option<HHOOK>,
}

impl MouseHook {
    /// Installs the hook. `owner` is where [`WM_MIN_HOVER`] is posted, and
    /// where [`REHOOK_TIMER_ID`] and [`MIN_HOVER_TIMER_ID`] are armed.
    ///
    /// Installs even when every gesture in `mouse` is unset: hovering the
    /// minimize button works on its own, independent of drag/resize.
    pub fn install(
        owner: HWND,
        mouse: Mouse,
        shortcuts: &[MouseShortcut],
        min_button: &crate::config::MinButton,
    ) -> Result<Self> {
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookState {
                owner,
                gesture: gesture::Gesture::new(mouse),
                shortcuts: shortcut::Shortcuts::new(shortcuts),
                hover: hover::Hover::new(
                    owner,
                    min_button.hover_delay_ms,
                    !min_button.items.is_empty(),
                    min_button.right_click_action.is_some(),
                ),
            });
        });
        LAST_POINT.with(|cell| cell.set(None));

        let hook = install_hook()?;

        // Ignored beyond firing WM_TIMER at `owner`; failure here just means
        // no watchdog, not a broken hook, so it is not itself fatal.
        unsafe { SetTimer(Some(owner), REHOOK_TIMER_ID, REHOOK_INTERVAL_MS, None) };

        Ok(Self {
            owner,
            hook: Some(hook),
        })
    }

    /// Handles `WM_TIMER(REHOOK_TIMER_ID)` — see this module's doc comment.
    /// A no-op on every ordinary tick; only does anything on the tick where
    /// the hook has actually gone missing.
    pub fn check_liveness(&mut self) {
        let mut cursor = POINT::default();
        if unsafe { GetCursorPos(&mut cursor) }.is_err() {
            return;
        }
        if LAST_POINT.with(|cell| cell.get()) == Some(cursor) {
            return;
        }

        // The cursor moved since the hook's own record of it last changed —
        // a live hook would have caught up already. Reinstall from scratch;
        // HookState (gesture, hover) is untouched, it simply starts
        // receiving events again.
        if let Some(old) = self.hook.take() {
            let _ = unsafe { UnhookWindowsHookEx(old) };
        }
        if let Ok(hook) = install_hook() {
            self.hook = Some(hook);
        }

        // Whether or not the reinstall above succeeded, this is the truth
        // as of right now — recording it avoids re-triggering on every tick
        // until the next real move, which would otherwise happen since
        // nothing updates `LAST_POINT` while the cursor stays put.
        LAST_POINT.with(|cell| cell.set(Some(cursor)));
    }
}

/// Ignored for a low-level hook per its own documentation, but every other
/// hook in this codebase passes it, and there is no cost to matching that.
fn install_hook() -> Result<HHOOK> {
    let instance = unsafe { GetModuleHandleW(None)? };
    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(instance.into()), 0) }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        let _ = unsafe { KillTimer(Some(self.owner), REHOOK_TIMER_ID) };
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

/// Handles `WM_TIMER(MIN_HOVER_TIMER_ID)`: the window whose minimize button
/// the dwell delay just elapsed for, and stops the timer so it does not fire
/// again on its own — [`hover::Hover::update`] is what re-arms it, on the
/// next move that still warrants one. A null `HWND` means the button was
/// left in the time it took this message to be handled.
pub fn take_min_hover_target() -> HWND {
    let mut target = HWND::default();
    with_state(|state| {
        target = state.hover.hovered();
        let _ = unsafe { KillTimer(Some(state.owner), MIN_HOVER_TIMER_ID) };
    });
    target
}

struct HookState {
    owner: HWND,
    gesture: gesture::Gesture,
    shortcuts: shortcut::Shortcuts,
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
    /// A mouse button went down. True when a shortcut claims the press.
    fn on_press(&mut self, trigger: MouseTrigger, pt: POINT) -> bool {
        if !self.shortcuts.uses(trigger) {
            return false;
        }
        let modifiers = modifiers_held();
        match self
            .shortcuts
            .press(trigger, modifiers, self.gesture.active())
        {
            shortcut::Press::Pass => false,
            shortcut::Press::Swallow => true,
            shortcut::Press::Fire(index) => self.fire(index, trigger, modifiers, Some(pt)),
        }
    }

    /// The wheel was tilted. True when a shortcut claims the event.
    fn on_tilt(&mut self, trigger: MouseTrigger, time: u32) -> bool {
        if !self.shortcuts.uses(trigger) {
            return false;
        }
        let modifiers = modifiers_held();
        match self
            .shortcuts
            .tilt(trigger, modifiers, time, self.gesture.active())
        {
            shortcut::Press::Pass => false,
            shortcut::Press::Swallow => true,
            shortcut::Press::Fire(index) => self.fire(index, trigger, modifiers, None),
        }
    }

    /// Resolves the window and posts [`WM_MOUSE_SHORTCUT`]: the part of a
    /// shortcut that cannot be pure. A slice scan and a `PostMessageW`, as the
    /// hook's time budget requires. `pt` is the press position, absent for a
    /// tilt, which has to use the cursor.
    fn fire(
        &mut self,
        index: usize,
        trigger: MouseTrigger,
        modifiers: HOT_KEY_MODIFIERS,
        pt: Option<POINT>,
    ) -> bool {
        let Some(binding) = self.shortcuts.binding(index).copied() else {
            return false;
        };

        // Armed before anything can pass the click on: a lone `Win` or `Alt`
        // release left behind would open Start or the system menu.
        y_core::modifier_guard::arm(modifiers);

        let mut window = HWND::default();
        if binding.action.needs_window() {
            let found = match (binding.target, pt) {
                (Target::Pointed, Some(pt)) => target::at(pt),
                (Target::Pointed, None) => target::pointed(),
                (Target::Focused, _) => target::focused(),
            };
            match found {
                Some(found) => window = found,
                None => {
                    // Nothing eligible: the event goes through untouched.
                    self.shortcuts.unclaim(trigger);
                    return false;
                }
            }
        }

        let _ = unsafe {
            PostMessageW(
                Some(self.owner),
                WM_MOUSE_SHORTCUT,
                WPARAM(index),
                LPARAM(window.0 as isize),
            )
        };
        true
    }

    /// A drag or resize in progress owns the pointer entirely; hover
    /// tracking only runs when neither is.
    fn on_move(&mut self, pt: POINT) {
        match self.gesture.on_move(pt) {
            gesture::Step::Apply(window, rect) => queue_apply(self.owner, window, rect),
            gesture::Step::Monitor(window, pt) => queue_monitor(self.owner, window, pt),
            // A pinned gesture owns the pointer too: no popup meanwhile.
            gesture::Step::Claimed => {}
            gesture::Step::Idle => self.hover.update(pt),
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let message = wparam.0 as u32;

        let consumed = match message {
            WM_MOUSEMOVE => {
                LAST_POINT.with(|cell| cell.set(Some(info.pt)));
                with_state(|state| state.on_move(info.pt));
                false
            }
            WM_LBUTTONDOWN => with_state_bool(|state| {
                state.on_press(MouseTrigger::Left, info.pt)
                    || state.gesture.on_button_down(Button::Left, info.pt)
            }),
            WM_LBUTTONUP => with_state_bool(|state| {
                state.shortcuts.release(MouseTrigger::Left)
                    || state.gesture.on_button_up(Button::Left)
            }),
            // A minimize-button right-click gets first refusal — it only
            // ever claims the event when it actually hits a button and the
            // feature is configured on, so an Alt+Right resize started over
            // one (an edge case, not the common shape of either gesture) is
            // never shadowed by it.
            WM_RBUTTONDOWN => with_state_bool(|state| {
                state.on_press(MouseTrigger::Right, info.pt)
                    || state.hover.on_right_down(info.pt)
                    || state.gesture.on_button_down(Button::Right, info.pt)
            }),
            WM_RBUTTONUP => with_state_bool(|state| {
                state.shortcuts.release(MouseTrigger::Right)
                    || state.hover.on_right_up(info.pt)
                    || state.gesture.on_button_up(Button::Right)
            }),
            WM_MBUTTONDOWN => with_state_bool(|state| {
                state.on_press(MouseTrigger::Middle, info.pt)
                    || state.gesture.on_button_down(Button::Middle, info.pt)
            }),
            WM_MBUTTONUP => with_state_bool(|state| {
                state.shortcuts.release(MouseTrigger::Middle)
                    || state.gesture.on_button_up(Button::Middle)
            }),
            // Which side button is in the high word of `mouseData`.
            WM_XBUTTONDOWN => match side_button(info.mouseData) {
                Some(trigger) => with_state_bool(|state| state.on_press(trigger, info.pt)),
                None => false,
            },
            WM_XBUTTONUP => match side_button(info.mouseData) {
                Some(trigger) => with_state_bool(|state| state.shortcuts.release(trigger)),
                None => false,
            },
            // The high word is the signed delta, positive for right. Plain
            // `WM_MOUSEWHEEL` is left alone, so ordinary scrolling costs
            // nothing new.
            WM_MOUSEHWHEEL => match (info.mouseData >> 16) as i16 {
                0 => false,
                delta => {
                    let trigger = if delta > 0 {
                        MouseTrigger::WheelRight
                    } else {
                        MouseTrigger::WheelLeft
                    };
                    with_state_bool(|state| state.on_tilt(trigger, info.time))
                }
            },
            _ => false,
        };

        if consumed {
            return LRESULT(1);
        }
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// `XBUTTON1` or `XBUTTON2`, from the high word of a `WM_XBUTTON*` event's
/// `mouseData`.
fn side_button(mouse_data: u32) -> Option<MouseTrigger> {
    match mouse_data >> 16 {
        1 => Some(MouseTrigger::X1),
        2 => Some(MouseTrigger::X2),
        _ => None,
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
