//! A `WH_KEYBOARD_LL` hook, installed only while at least one Alt/Win
//! -modified drag or resize is in progress, whose only job is to catch the
//! modifier's *real* key-up and fire
//! [`y_core::hotkey::absorb_modifier_tap`] synchronously, immediately before
//! letting that key-up continue down the chain.
//!
//! # Why a second hook, separate from [`super`]'s mouse hook
//!
//! The mouse hook only ever sees `WM_LBUTTONDOWN`/`WM_LBUTTONUP` and their
//! siblings — it has nothing to catch the modifier coming back up with,
//! because the user is free to let go of Alt or Win seconds after releasing
//! the mouse button, long after [`super::gesture::Gesture`] has forgotten
//! the gesture ever happened. A keyboard hook is the only thing that can see
//! a key-up at all, and it has to keep running for as long as that key-up
//! might still be pending — which is why this one is armed at gesture start
//! ([`arm`], called from
//! [`Gesture::on_button_down`](super::gesture::Gesture::on_button_down))
//! and only comes down once it has actually done its job, not when the
//! gesture itself ends.
//!
//! # Why the correction happens here and not at mouse-button time
//!
//! AltSnap and HulK both fire their equivalent of
//! [`absorb_modifier_tap`] from inside the keyboard hook that observes the
//! modifier's real key-up, not from mouse-button handling. y-snap tried
//! firing it at button-down/button-up instead, with several filler keys and
//! flag combinations, and none of it reliably stopped the Start menu or the
//! window's system menu from appearing. The timing turned out to be the
//! variable that mattered, not the filler key: injecting the correction
//! anywhere other than immediately before the real key-up is let through
//! leaves Windows free to see the two events in an order that still reads
//! as "modifier pressed and released alone".
//!
//! # Why this never swallows the real key-up
//!
//! The hook calls [`absorb_modifier_tap`] and then always calls
//! `CallNextHookEx` — it never returns a nonzero result. Swallowing the real
//! key-up here would leave every other application, and Windows itself,
//! thinking Alt or Win is still held down. That was the bug in the attempt
//! this module replaces: armed once and never safely disarmed, and the
//! thing it caught never actually let go.
//!
//! # The elevated-focus gap
//!
//! `WH_KEYBOARD_LL` can lose a key-up outright — not delayed, gone — when
//! focus switches to an elevated window while the key is still down; see
//! [`y_core::hotkey`]'s own doc comment. Without a way to notice that, this
//! module's hook (and the state that keeps it installed) would be stuck
//! armed forever after that happens, waiting for a key-up that already
//! occurred. [`arm`] registers [`reconcile`] with
//! [`y_core::hotkey::watch_foreground`] to cover exactly that: every
//! foreground switch, it checks the armed bits against
//! [`GetAsyncKeyState`] and force-clears (and, if nothing is left armed,
//! uninstalls) whatever the hook missed.

use std::cell::Cell;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_WIN, VIRTUAL_KEY, VK_LMENU, VK_LWIN, VK_MENU,
    VK_RMENU, VK_RWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_KEYUP, WM_SYSKEYUP,
};
use y_core::hotkey::{absorb_modifier_tap, unwatch_foreground, watch_foreground};

thread_local! {
    /// The Alt/Win bits (from [`MOD_ALT`] / [`MOD_WIN`]) at least one
    /// in-progress gesture is still waiting to see the real key-up for,
    /// together with the hook installed to watch for it — `None` when
    /// nothing is armed. A `Cell` rather than a `RefCell`: nothing here ever
    /// needs a borrow that outlives one statement.
    static ARMED: Cell<(u32, Option<HHOOK>)> = const { Cell::new((0, None)) };
}

/// Arms the guard for whichever of `modifiers`' Alt/Win bits are set,
/// installing the hook first if it is not already up. A no-op for a gesture
/// that uses neither modifier. Safe to call on every gesture activation,
/// including a second one that starts before the first's modifier has come
/// back up — see [`super::gesture::Gesture::on_button_down`], its only
/// caller.
pub fn arm(modifiers: HOT_KEY_MODIFIERS) {
    let bits = modifiers.0 & (MOD_ALT.0 | MOD_WIN.0);
    if bits == 0 {
        return;
    }

    ARMED.with(|cell| {
        let (armed, hook) = cell.get();
        let hook = hook.or_else(|| unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook),
                Some(instance.into()),
                0,
            )
            .ok()
        });
        cell.set((armed | bits, hook));
    });

    // See this module's doc comment on the elevated-focus gap. Idempotent,
    // like the hook install above — safe to call on every activation.
    watch_foreground(reconcile);
}

/// Clears `bits` out of whatever is currently armed. Uninstalls the hook,
/// and lets go of [`watch_foreground`], once nothing is left. The shared
/// tail of both [`keyboard_hook`] (which just did its job) and [`reconcile`]
/// (which just discovered the hook can no longer do it).
fn disarm(bits: u32) {
    ARMED.with(|cell| {
        let (armed, hook) = cell.get();
        let remaining = armed & !bits;
        if remaining == armed {
            return;
        }

        if remaining == 0 {
            if let Some(hook) = hook {
                let _ = unsafe { UnhookWindowsHookEx(hook) };
            }
            cell.set((0, None));
            unwatch_foreground(reconcile);
        } else {
            cell.set((remaining, hook));
        }
    });
}

/// Registered with [`y_core::hotkey::watch_foreground`] while armed — see
/// this module's doc comment on the elevated-focus gap.
fn reconcile() {
    let armed = ARMED.with(|cell| cell.get().0);
    if armed == 0 {
        return;
    }

    let held = |vk: VIRTUAL_KEY| {
        let state = unsafe { GetAsyncKeyState(vk.0 as i32) };
        state as u16 & 0x8000 != 0
    };

    let mut stale = 0;
    if armed & MOD_ALT.0 != 0 && !held(VK_MENU) {
        stale |= MOD_ALT.0;
    }
    if armed & MOD_WIN.0 != 0 && !(held(VK_LWIN) || held(VK_RWIN)) {
        stale |= MOD_WIN.0;
    }

    if stale != 0 {
        disarm(stale);
    }
}

/// Which modifier family `vk` belongs to, in [`arm`]'s vocabulary — `0` for
/// anything else. `KBDLLHOOKSTRUCT::vkCode` reports the side-specific code
/// (`VK_LMENU`/`VK_RMENU`, not the generic `VK_MENU`) for a real keystroke,
/// but the generic codes are matched too since nothing rules out seeing one
/// from another source.
fn family_of(vk: u16) -> u32 {
    if vk == VK_LMENU.0 || vk == VK_RMENU.0 || vk == VK_MENU.0 {
        MOD_ALT.0
    } else if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
        MOD_WIN.0
    } else {
        0
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let message = wparam.0 as u32;
        if message == WM_KEYUP || message == WM_SYSKEYUP {
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let family = family_of(info.vkCode as u16);

            if family != 0 && ARMED.with(|cell| cell.get().0) & family != 0 {
                // Fire the correction now, synchronously, before this real
                // key-up is allowed to continue below — see this module's
                // doc comment for why the timing is the whole point.
                absorb_modifier_tap();
                disarm(family);
            }
        }
    }

    // Never swallowed: the real key-up must always reach the rest of the
    // system, or every application ends up thinking the modifier is still
    // held.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
