//! The `WH_MOUSE_LL` hook behind the `<mouse>` bindings: the glue around the
//! pure rules of [`y_coustic::input`].
//!
//! # What runs in the hook, and what does not
//!
//! Windows silently drops a low-level hook slower than 300 ms, so the hook
//! procedure only decides "swallow or not" and posts a private `WM_APP`
//! message; the COM calls and the HUD happen on the message loop. Reading the
//! modifiers (`GetAsyncKeyState`) is fine; resolving a window or touching a
//! device is not. The state lives in a `thread_local!`, not a `Mutex`: the
//! hook only ever runs on the thread that installed it.
//!
//! # Wheel deltas are coalesced
//!
//! A claimed notch adds its delta to a per-binding accumulator and posts
//! [`WM_MOUSE_WHEEL`] only when the accumulator was zero; the handler takes
//! and zeroes it ([`take_wheel`]). A precision touchpad sending hundreds of
//! small deltas a second costs one COM round per message-loop turn, and the
//! change stays proportional (`delta / 120` notches).
//!
//! # The Alt tap
//!
//! A claimed `Alt+wheel` or `Win+click` would otherwise leave a lone `Alt` or
//! `Win` release behind, which focuses the menu bar or opens Start. Every
//! claim arms [`y_core::modifier_guard`], which corrects it at the modifier's
//! key-up (`y-snap` has the same problem and the same cure).
//!
//! # Simplifications
//!
//! * No watchdog that reinstalls a hook Windows dropped, unlike `y-snap`'s:
//!   this hook does no work on a move, which is what makes a hook slow. Add
//!   one if a live check shows it being dropped.
//! * Injected events count like real ones (a remote-control tool's `Alt+wheel`
//!   works), except the wheel this program sends itself, tagged in
//!   `dwExtraInfo` (see [`reinject`]).
//!
//! # A wheel with nothing to change
//!
//! Over an application with no audio the wheel must still scroll, but the hook
//! has to decide before COM can say so. It swallows, the handler finds no
//! session, and [`reinject`] sends the same delta back: the application sees
//! what it would have, a moment late.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_MOUSE, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
    MOD_WIN, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use y_core::ids::WM_APP_FIRST;
use y_core::modifier_guard;

use y_coustic::input::{Input, Machine, MouseBinding, Verdict};

/// Posted to the owner when a wheel binding has delta waiting. `wParam` is
/// the binding's index in the list given to [`MouseHook::install`]; the delta
/// is read with [`take_wheel`].
pub const WM_MOUSE_WHEEL: u32 = WM_APP_FIRST + 2;

/// Posted to the owner when a button binding fires, on the release. `wParam`
/// is the binding's index.
pub const WM_MOUSE_ACTION: u32 = WM_APP_FIRST + 3;

struct HookState {
    owner: HWND,
    machine: Machine,
    /// Wheel delta not yet taken by the handler, per binding. Sized at
    /// install: the hook never allocates.
    pending: Vec<i32>,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// Owns the hook for as long as it lives. Uninstalled on drop.
pub struct MouseHook {
    hook: Option<HHOOK>,
}

impl MouseHook {
    /// Installs the hook for `bindings`, answering to `owner`. Call it only
    /// when there are bindings: with none there is nothing to hook.
    pub fn install(owner: HWND, bindings: &[MouseBinding]) -> windows::core::Result<Self> {
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookState {
                owner,
                machine: Machine::new(bindings),
                pending: vec![0; bindings.len()],
            });
        });

        let instance = unsafe { GetModuleHandleW(None) };
        let hook = instance.and_then(|instance| unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(instance.into()), 0)
        });
        match hook {
            Ok(hook) => Ok(Self { hook: Some(hook) }),
            Err(error) => {
                STATE.with(|cell| *cell.borrow_mut() = None);
                Err(error)
            }
        }
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

/// Marks a wheel event this program sent itself (`dwExtraInfo`), which the
/// hook lets through untouched: without it, the event [`reinject`] sends
/// while the modifiers are still held would be claimed again, forever.
pub(crate) const REINJECTED: usize = 0x5943_4F55; // "YCOU"

/// Gives a wheel back: sends `delta` as a wheel (or tilt) event, tagged so the
/// hook passes it. For a wheel that was swallowed before anyone could know the
/// application under the pointer had no audio to change: the page then scrolls
/// as it would have, a moment late. The delta is the raw one (no inversion),
/// and the modifiers still held reach the application as they were.
pub fn reinject(input: Input, delta: i32) {
    let flags = match input {
        Input::Wheel => MOUSEEVENTF_WHEEL,
        Input::WheelTilt => MOUSEEVENTF_HWHEEL,
        _ => return,
    };
    let event = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: delta as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: REINJECTED,
            },
        },
    };
    unsafe { SendInput(&[event], size_of::<INPUT>() as i32) };
}

/// The delta [`WM_MOUSE_WHEEL`] announced for binding `index`, taken (the
/// accumulator restarts from zero). Zero when the state is gone, which a
/// reload between the post and now can cause.
pub fn take_wheel(index: usize) -> i32 {
    with_state(|state| state.pending.get_mut(index).map(std::mem::take))
        .flatten()
        .unwrap_or(0)
}

/// The binding at `index` of the installed list, for the handler of either
/// message: read here and not from the settings so a reload in between cannot
/// pair a delta with another binding.
pub fn binding(index: usize) -> Option<MouseBinding> {
    with_state(|state| state.machine.binding(index).copied()).flatten()
}

/// Runs `body` against the state, `None` if it is absent or already borrowed
/// (only a reentrant call into the hook, on its own thread, can do that).
fn with_state<T>(body: impl FnOnce(&mut HookState) -> T) -> Option<T> {
    STATE.with(|cell| {
        let mut state = cell.try_borrow_mut().ok()?;
        state.as_mut().map(body)
    })
}

/// Same, for a decision to swallow: "let it through" when unavailable.
fn with_state_bool(body: impl FnOnce(&mut HookState) -> bool) -> bool {
    with_state(body).unwrap_or(false)
}

impl HookState {
    fn post(&self, message: u32, index: usize) {
        let _ = unsafe { PostMessageW(Some(self.owner), message, WPARAM(index), LPARAM(0)) };
    }

    /// The wheel turned or tilted by `delta`. True when a binding claims it.
    fn on_wheel(&mut self, input: Input, delta: i16) -> bool {
        // Cheapest refusals first: an event nobody listens to costs a scan.
        if delta == 0 || !self.machine.uses(input) {
            return false;
        }
        let modifiers = modifiers_held();
        let Verdict::Fire(index) = self.machine.wheel(input, modifiers) else {
            return false;
        };

        // Armed before the event can be passed on or the key let go.
        modifier_guard::arm(HOT_KEY_MODIFIERS(modifiers));

        let Some(pending) = self.pending.get_mut(index) else {
            return true;
        };
        let was_idle = *pending == 0;
        *pending += i32::from(delta);
        if was_idle {
            self.post(WM_MOUSE_WHEEL, index);
        }
        true
    }

    /// A button went down. True when a binding claims the press.
    fn on_down(&mut self, input: Input) -> bool {
        if !self.machine.uses(input) {
            return false;
        }
        let modifiers = modifiers_held();
        if self.machine.button_down(input, modifiers) == Verdict::Swallow {
            modifier_guard::arm(HOT_KEY_MODIFIERS(modifiers));
            true
        } else {
            false
        }
    }

    /// A button came up. True when it answers a claimed press, which fires.
    fn on_up(&mut self, input: Input) -> bool {
        match self.machine.button_up(input) {
            Verdict::Fire(index) => {
                self.post(WM_MOUSE_ACTION, index);
                true
            }
            _ => false,
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        // The signed delta of a wheel, the button of an X event: both sit in
        // the high word of `mouseData`.
        let high_word = (info.mouseData >> 16) as i16;

        let consumed = match wparam.0 as u32 {
            // Our own re-injected wheel passes.
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL if info.dwExtraInfo == REINJECTED => false,
            WM_MOUSEWHEEL => with_state_bool(|state| state.on_wheel(Input::Wheel, high_word)),
            WM_MOUSEHWHEEL => with_state_bool(|state| state.on_wheel(Input::WheelTilt, high_word)),
            WM_LBUTTONDOWN => with_state_bool(|state| state.on_down(Input::Left)),
            WM_LBUTTONUP => with_state_bool(|state| state.on_up(Input::Left)),
            WM_RBUTTONDOWN => with_state_bool(|state| state.on_down(Input::Right)),
            WM_RBUTTONUP => with_state_bool(|state| state.on_up(Input::Right)),
            WM_MBUTTONDOWN => with_state_bool(|state| state.on_down(Input::Middle)),
            WM_MBUTTONUP => with_state_bool(|state| state.on_up(Input::Middle)),
            WM_XBUTTONDOWN => match side_button(high_word) {
                Some(input) => with_state_bool(|state| state.on_down(input)),
                None => false,
            },
            WM_XBUTTONUP => match side_button(high_word) {
                Some(input) => with_state_bool(|state| state.on_up(input)),
                None => false,
            },
            // Moves and everything else: untouched, at no cost.
            _ => false,
        };
        if consumed {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// `XBUTTON1` or `XBUTTON2`, from the high word of an X event's `mouseData`.
fn side_button(which: i16) -> Option<Input> {
    match which {
        1 => Some(Input::X1),
        2 => Some(Input::X2),
        _ => None,
    }
}

/// The modifier keys held right now, as `MOD_*` flags.
pub(crate) fn modifiers_held() -> u32 {
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
    modifiers
}
