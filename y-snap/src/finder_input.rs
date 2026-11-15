//! The hooks behind the finder's automatic triggers.
//!
//! The recognition itself is in [`crate::finder_detect`]; this is the
//! plumbing that feeds it, and the only part that touches the system:
//!
//! * **Taps** need to see the modifier keys, so a `WH_KEYBOARD_LL` hook of
//!   their own is installed — only when some style declares a `<tap>`.
//! * **Shakes** are fed from the mouse hook that already runs for the
//!   gestures ([`crate::mouse`] calls [`on_move`] and [`on_button`]), so a
//!   shake costs no second hook; with no `<shake>` declared the call returns
//!   at once.
//!
//! Neither hook swallows anything: the event always goes on. A trigger is
//! posted to the owner window as [`WM_FINDER_TRIGGER`] and played from the
//! message loop, because a hook must not draw. Injected events (another
//! program sending keys or moving the pointer) are ignored, so a macro or a
//! remote-control tool cannot set the finder off.
//!
//! **Not covered yet:** unlike the mouse hook, the keyboard hook has no
//! watchdog. Windows drops a hook that is ever too slow, and then taps stop
//! working until the next reload. The hook does a few comparisons and a
//! post, so it should not happen.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MBUTTON,
    VK_RBUTTON, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_XBUTTON1, VK_XBUTTON2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use y_core::ids::WM_APP_FIRST;

use crate::config::{Finder, TapKey};
use crate::finder_detect::{Held, Shakes, Taps};

/// Posted to the owner window when an automatic trigger fires. `wParam` is the
/// index of the style to play. Handled by `crate`'s own `find_mouse`.
pub const WM_FINDER_TRIGGER: u32 = WM_APP_FIRST + 10;

/// `LLKHF_INJECTED` of a keyboard event, `LLMHF_INJECTED` of a mouse one.
const KEY_INJECTED: u32 = 0x10;

struct Input {
    owner: HWND,
    taps: Taps,
    shakes: Shakes,
}

thread_local! {
    static INPUT: RefCell<Option<Input>> = const { RefCell::new(None) };
    /// Whether any shake is declared, so the mouse hook can skip the lookup.
    static WATCHING_SHAKES: Cell<bool> = const { Cell::new(false) };
}

/// Owns what the triggers installed. Everything is undone on drop.
pub struct FinderInput {
    hook: Option<HHOOK>,
}

impl FinderInput {
    /// Sets up the triggers `finder`'s styles declare. `None` when it declares
    /// none, or when the keyboard hook that taps need cannot be installed.
    pub fn install(owner: HWND, finder: &Finder) -> Option<Self> {
        let mut taps = Vec::new();
        let mut shakes = Vec::new();
        for (index, style) in finder.styles.iter().enumerate() {
            let index = u16::try_from(index).ok()?;
            taps.extend(style.taps.iter().map(|tap| (index, *tap)));
            shakes.extend(style.shakes.iter().map(|shake| (index, *shake)));
        }
        if taps.is_empty() && shakes.is_empty() {
            return None;
        }

        let input = Input {
            owner,
            taps: Taps::new(taps),
            shakes: Shakes::new(shakes),
        };
        let wants_keys = !input.taps.is_empty();
        let wants_shakes = !input.shakes.is_empty();
        INPUT.with(|cell| *cell.borrow_mut() = Some(input));
        WATCHING_SHAKES.with(|cell| cell.set(wants_shakes));

        let hook = if wants_keys {
            let installed = unsafe {
                GetModuleHandleW(None).ok().and_then(|instance| {
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(instance.into()), 0).ok()
                })
            };
            if installed.is_none() && !wants_shakes {
                INPUT.with(|cell| *cell.borrow_mut() = None);
                return None;
            }
            installed
        } else {
            None
        };
        Some(Self { hook })
    }
}

impl Drop for FinderInput {
    fn drop(&mut self) {
        if let Some(hook) = self.hook.take() {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
        WATCHING_SHAKES.with(|cell| cell.set(false));
        INPUT.with(|cell| *cell.borrow_mut() = None);
    }
}

fn with_input(body: impl FnOnce(&mut Input)) {
    INPUT.with(|cell| {
        if let Ok(mut input) = cell.try_borrow_mut()
            && let Some(input) = input.as_mut()
        {
            body(input);
        }
    });
}

fn post(owner: HWND, style: u16) {
    let _ = unsafe { PostMessageW(Some(owner), WM_FINDER_TRIGGER, WPARAM(style as usize), LPARAM(0)) };
}

/// The pointer moved. Called by the mouse hook for every move.
pub fn on_move(x: i32, y: i32, time: u32, injected: bool) {
    if injected || !WATCHING_SHAKES.with(Cell::get) {
        return;
    }
    with_input(|input| {
        if let Some(style) = input.shakes.on_move(x, y, time, sense) {
            post(input.owner, style);
        }
    });
}

/// A mouse button went down: a click is not part of a tap.
pub fn on_button() {
    with_input(|input| input.taps.other());
}

/// The modifiers and buttons held right now. Read only when a path is judged.
fn sense() -> Held {
    let down = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| {
        (unsafe { GetAsyncKeyState(key.0 as i32) }) as u16 & 0x8000 != 0
    };
    Held {
        modifiers: crate::mouse::modifiers_held().0,
        button: [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2]
            .into_iter()
            .any(down),
    }
}

/// Which modifier a virtual key is, and whether it is the right-hand one.
fn modifier_of(vk: u32) -> Option<(TapKey, bool)> {
    match vk {
        v if v == VK_LCONTROL.0 as u32 => Some((TapKey::Ctrl, false)),
        v if v == VK_RCONTROL.0 as u32 => Some((TapKey::Ctrl, true)),
        v if v == VK_LSHIFT.0 as u32 => Some((TapKey::Shift, false)),
        v if v == VK_RSHIFT.0 as u32 => Some((TapKey::Shift, true)),
        v if v == VK_LMENU.0 as u32 => Some((TapKey::Alt, false)),
        v if v == VK_RMENU.0 as u32 => Some((TapKey::Alt, true)),
        v if v == VK_LWIN.0 as u32 => Some((TapKey::Win, false)),
        v if v == VK_RWIN.0 as u32 => Some((TapKey::Win, true)),
        _ => None,
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if info.flags.0 & KEY_INJECTED == 0 {
            let message = wparam.0 as u32;
            let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
            let up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
            if down || up {
                with_input(|input| match modifier_of(info.vkCode) {
                    Some((key, right)) => {
                        if let Some(style) = input.taps.key(key, right, down, info.time) {
                            post(input.owner, style);
                        }
                    }
                    // Any other key spoils a tap, going down; its release
                    // changes nothing.
                    None if down => input.taps.other(),
                    None => {}
                });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
