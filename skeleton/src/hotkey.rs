//! System-wide hotkeys, with an optional low-level-hook fallback.
//!
//! Two ways to catch a key combination system wide:
//!
//! * `RegisterHotKey` — the polite one. The shell arbitrates, the combination
//!   is reserved for us, and `WM_HOTKEY` arrives in the normal message loop.
//!   It fails when someone else already owns the combination, and the shell
//!   itself owns most of the interesting `Win+key` ones.
//! * `WH_KEYBOARD_LL` — the rude one. We see every keystroke before the
//!   system does and can swallow the ones we want, stealing combinations the
//!   shell reserved. This is how an application overrides `Win+X`.
//!
//! [`HotkeyManager::register`] takes the system route and, when asked with
//! [`Fallback::LowLevelHook`], falls back to the hook. Either way the owner
//! window receives the same `WM_HOTKEY`, so the application code that reacts
//! to a hotkey never needs to know which route was taken.
//!
//! # Cost of the hook
//!
//! The hook procedure runs on the thread that installed it, inside the
//! message loop, for **every keystroke on the machine**. Windows removes
//! hooks that take longer than `LowLevelHooksTimeout` (300 ms by default), so
//! the procedure below allocates nothing, takes no lock that another thread
//! could hold, and bails out rather than blocking. Install it only when a
//! hotkey actually needs it.
//!
//! Note also that a process without UIAccess cannot hook keystrokes destined
//! for an elevated application, and the hook is silently inactive on the
//! secure desktop (UAC prompt, lock screen).

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    RegisterHotKey, SendInput, UnregisterHotKey, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};
use windows::core::Result;

/// A key combination: modifiers plus a virtual-key code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub key: VIRTUAL_KEY,
}

impl Hotkey {
    pub const fn new(modifiers: HOT_KEY_MODIFIERS, key: VIRTUAL_KEY) -> Self {
        Self { modifiers, key }
    }

    fn has(&self, modifier: HOT_KEY_MODIFIERS) -> bool {
        self.modifiers.0 & modifier.0 != 0
    }
}

/// What to do when `RegisterHotKey` refuses the combination.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// Report the failure. The right default: a combination owned by another
    /// application belongs to that application.
    None,
    /// Install a low-level keyboard hook and swallow the combination before
    /// the system sees it. Use it for combinations the shell reserved and the
    /// user explicitly asked to reclaim.
    LowLevelHook,
}

/// Which route a successful [`HotkeyManager::register`] ended up taking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Registration {
    /// Registered through `RegisterHotKey`.
    System,
    /// Captured by the low-level keyboard hook.
    Override,
}

/// Owns every hotkey registered for a window, by both routes.
///
/// Must be created, used and dropped on the thread that runs the message
/// loop: that is where `WM_HOTKEY` is delivered and where the hook procedure
/// is called.
pub struct HotkeyManager {
    owner: HWND,
    /// Ids registered through `RegisterHotKey`, in registration order.
    system: Vec<u32>,
    hook: Option<HHOOK>,
}

impl HotkeyManager {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            system: Vec::new(),
            hook: None,
        }
    }

    /// Registers `hotkey` under `id`, which the owner window receives as the
    /// `wParam` of `WM_HOTKEY`. Ids are the application's to choose and must
    /// be unique per window (`0..=0xBFFF`).
    ///
    /// Pass `MOD_NOREPEAT` in the modifiers to ignore auto-repeat; the
    /// fallback path honours it too.
    pub fn register(
        &mut self,
        id: u32,
        hotkey: Hotkey,
        fallback: Fallback,
    ) -> Result<Registration> {
        let attempt = unsafe {
            RegisterHotKey(
                Some(self.owner),
                id as i32,
                hotkey.modifiers,
                hotkey.key.0 as u32,
            )
        };

        match attempt {
            Ok(()) => {
                self.system.push(id);
                return Ok(Registration::System);
            }
            Err(error) if fallback == Fallback::None => return Err(error),
            Err(_) => {}
        }

        self.install_hook()?;
        state(|state| state.add(self.owner, id, hotkey));
        Ok(Registration::Override)
    }

    /// Releases the hotkey registered under `id`, whichever route it took.
    /// Unknown ids are ignored.
    #[allow(dead_code)] // part of the skeleton's public surface
    pub fn unregister(&mut self, id: u32) {
        if let Some(index) = self.system.iter().position(|&known| known == id) {
            let _ = unsafe { UnregisterHotKey(Some(self.owner), id as i32) };
            self.system.remove(index);
            return;
        }

        state(|state| state.remove(id));
        self.uninstall_hook_if_idle();
    }

    /// Releases every hotkey and the hook. Called on drop.
    pub fn unregister_all(&mut self) {
        for id in self.system.drain(..) {
            let _ = unsafe { UnregisterHotKey(Some(self.owner), id as i32) };
        }

        state(|state| state.clear());
        self.uninstall_hook_if_idle();
    }

    fn install_hook(&mut self) -> Result<()> {
        if self.hook.is_some() {
            return Ok(());
        }

        // For WH_KEYBOARD_LL the hook procedure lives in this process, so the
        // module handle is only used to keep it loaded.
        let instance = unsafe { GetModuleHandleW(None)? };
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook),
                Some(instance.into()),
                0,
            )?
        };

        self.hook = Some(hook);
        Ok(())
    }

    /// Keystrokes are cheap to let through but not free: drop the hook as
    /// soon as no override needs it.
    fn uninstall_hook_if_idle(&mut self) {
        if state(|state| state.is_empty())
            && let Some(hook) = self.hook.take()
        {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}

/// State reachable from the hook procedure, which is a bare C callback with
/// nowhere to carry a `self`.
///
/// A thread local rather than a global mutex: the procedure is only ever
/// called on the thread that installed the hook, which is also the only
/// thread a [`HotkeyManager`] lives on, so there is nothing to synchronise —
/// and no lock a keystroke could ever block on.
struct HookState {
    owner: HWND,
    overrides: Vec<Override>,
}

struct Override {
    id: u32,
    hotkey: Hotkey,
    /// Tracks auto-repeat, and marks the key-up we owe a swallow to.
    pressed: bool,
}

thread_local! {
    static HOOK_STATE: RefCell<HookState> = const {
        RefCell::new(HookState {
            owner: HWND(std::ptr::null_mut()),
            overrides: Vec::new(),
        })
    };
}

/// Runs `body` against the hook state, returning its default value if the
/// state is already borrowed — see [`keyboard_hook`].
fn state<T: Default>(body: impl FnOnce(&mut HookState) -> T) -> T {
    HOOK_STATE.with(|state| match state.try_borrow_mut() {
        Ok(mut state) => body(&mut state),
        Err(_) => T::default(),
    })
}

impl HookState {
    fn add(&mut self, owner: HWND, id: u32, hotkey: Hotkey) {
        self.owner = owner;
        self.remove(id);
        self.overrides.push(Override {
            id,
            hotkey,
            pressed: false,
        });
    }

    fn remove(&mut self, id: u32) {
        self.overrides.retain(|entry| entry.id != id);
    }

    fn clear(&mut self) {
        self.overrides.clear();
    }

    fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    /// Returns `Some(())` when the keystroke was consumed and must not reach
    /// the rest of the system.
    fn on_key(&mut self, message: u32, key: VIRTUAL_KEY) -> Option<()> {
        let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let up = message == WM_KEYUP || message == WM_SYSKEYUP;

        let index = self
            .overrides
            .iter()
            .position(|entry| entry.hotkey.key == key)?;

        if up {
            // Swallow the release of a key whose press we swallowed, so no
            // application sees half a keystroke. A release we did not cause
            // travels on.
            let entry = &mut self.overrides[index];
            if !entry.pressed {
                return None;
            }
            entry.pressed = false;
            return Some(());
        }

        if !down || !modifiers_pressed(self.overrides[index].hotkey.modifiers) {
            return None;
        }

        let entry = &mut self.overrides[index];
        if entry.pressed && entry.hotkey.has(MOD_NOREPEAT) {
            return Some(()); // auto-repeat: swallowed, not reported
        }
        entry.pressed = true;

        // Same message the system would have posted, so the application's
        // WM_HOTKEY handler cannot tell the two routes apart.
        let modifiers = entry.hotkey.modifiers.0 & !MOD_NOREPEAT.0;
        let _ = unsafe {
            PostMessageW(
                Some(self.owner),
                WM_HOTKEY,
                WPARAM(entry.id as usize),
                LPARAM(((key.0 as isize) << 16) | modifiers as isize),
            )
        };

        if entry.hotkey.has(MOD_WIN) {
            neutralise_win_key();
        }

        Some(())
    }
}

/// True when exactly the requested modifiers are held down.
///
/// The match is exact, like `RegisterHotKey`: with `Win+X` registered,
/// `Win+Ctrl+X` is somebody else's business.
fn modifiers_pressed(modifiers: HOT_KEY_MODIFIERS) -> bool {
    let held = |key: VIRTUAL_KEY| unsafe { GetAsyncKeyState(key.0 as i32) } as u16 & 0x8000 != 0;

    let win = held(VK_LWIN) || held(VK_RWIN);
    let expected = |modifier: HOT_KEY_MODIFIERS| modifiers.0 & modifier.0 != 0;

    win == expected(MOD_WIN)
        && held(VK_CONTROL) == expected(MOD_CONTROL)
        && held(VK_MENU) == expected(MOD_ALT)
        && held(VK_SHIFT) == expected(MOD_SHIFT)
}

/// Cancels the Start menu that would otherwise open when the Win key is
/// released.
///
/// Swallowing `Win+X` leaves the shell seeing a Win press followed by a Win
/// release with nothing in between, which is the gesture that opens the Start
/// menu. Injecting a keystroke it does not act on breaks the pair. Right
/// shift is a safe filler: no shell gesture is bound to it, and it does not
/// change the meaning of anything the user types next.
fn neutralise_win_key() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RSHIFT,
                wScan: 0x36,
                dwFlags: KEYEVENTF_EXTENDEDKEY | flags,
                time: 0,
                dwExtraInfo: OWN_INPUT_SIGNATURE,
            },
        },
    };

    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Anything other than HC_ACTION must be passed on untouched.
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };

        // Ignore what we inject ourselves (see `neutralise_win_key`), which
        // would otherwise come straight back through this procedure.
        //
        // Matching on our own signature rather than on `LLKHF_INJECTED`:
        // plenty of legitimate keystrokes are injected — on-screen keyboards,
        // remote desktop, macro tools, some laptop Fn keys — and a hotkey
        // that ignores all of them is a hotkey that mysteriously does not
        // work for those users.
        if info.dwExtraInfo != OWN_INPUT_SIGNATURE {
            let key = VIRTUAL_KEY(info.vkCode as u16);

            // `state` yields `None` if the state is already borrowed, which
            // would mean this procedure re-entered itself. It cannot happen
            // on one thread, and a dropped keystroke beats a panic across an
            // `extern "system"` boundary if it ever did.
            if state(|state| state.on_key(wparam.0 as u32, key)).is_some() {
                return LRESULT(1); // consumed
            }
        }
    }

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Stamped into `dwExtraInfo` of the keystrokes this module injects, so the
/// hook procedure can recognise its own work. Any value unlikely to collide
/// will do; it is only ever compared against itself.
const OWN_INPUT_SIGNATURE: usize = 0x7769_6E72_7573_746D; // "winrustm"
