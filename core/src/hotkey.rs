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
//!
//! # The elevated-focus gap, and its recovery net
//!
//! The UIAccess restriction above has a sharper edge than "the hook doesn't
//! see elevated keystrokes": if the user presses a key, focus then switches
//! to an elevated window *while the key is still held*, and the key is
//! released only after that switch, this hook never sees the matching
//! key-up at all — not delayed, just gone. Anything that tracks state across
//! a key-down and a later key-up (this module's own `Override::pressed`;
//! `y-snap`'s `modifier_guard`, armed at gesture start and waiting for the
//! modifier's real key-up) is exposed to being left stuck, waiting
//! permanently for an event that already happened and that nothing will
//! ever deliver.
//!
//! [`watch_foreground`] is the recovery net PowerToys uses for exactly this
//! (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ...)`): cheap — one event per
//! foreground switch, not per keystroke — and independent of the keyboard
//! hook entirely, since it doesn't rely on a key event to fire. A caller
//! registers a callback that reconciles its own tracked state against
//! [`GetAsyncKeyState`] (real hardware state, regardless of whether a
//! message ever arrived) every time the foreground window changes, and
//! force-clears whatever the keyboard hook missed. This module registers
//! its own for `Override::pressed`; `y-snap`'s `modifier_guard` registers
//! its own for the armed modifier bits.

use std::cell::RefCell;
use std::fmt;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, RegisterHotKey, SendInput, UnregisterHotKey, VIRTUAL_KEY, VK_ADD, VK_APPS,
    VK_BACK, VK_BROWSER_BACK, VK_BROWSER_FAVORITES, VK_BROWSER_FORWARD, VK_BROWSER_HOME,
    VK_BROWSER_REFRESH, VK_BROWSER_SEARCH, VK_BROWSER_STOP, VK_CAPITAL, VK_CLEAR, VK_CONTROL,
    VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT,
    VK_LAUNCH_APP1, VK_LAUNCH_APP2, VK_LAUNCH_MAIL, VK_LAUNCH_MEDIA_SELECT, VK_LEFT, VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP, VK_MENU,
    VK_MULTIPLY, VK_NEXT, VK_NUMLOCK, VK_NUMPAD0, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5,
    VK_OEM_6, VK_OEM_7, VK_OEM_8, VK_OEM_102, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PAUSE,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT, VK_SNAPSHOT, VK_SPACE,
    VK_SUBTRACT, VK_TAB, VK_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, EVENT_SYSTEM_FOREGROUND, GetMessageExtraInfo, HC_ACTION, HHOOK,
    KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WINEVENT_OUTOFCONTEXT, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::Result;

/// A key combination: modifiers plus a virtual-key code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hotkey {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub key: VIRTUAL_KEY,
}

impl Hotkey {
    pub const fn new(modifiers: HOT_KEY_MODIFIERS, key: VIRTUAL_KEY) -> Self {
        Self { modifiers, key }
    }

    /// Parses `"Win+Shift+F12"`, as written in a settings file.
    ///
    /// Accepted modifiers: `Win`, `Ctrl` (`Control`), `Alt`, `Shift`, in any
    /// order and any case, separated by `+`. Accepted keys: `A`-`Z`, `0`-`9`,
    /// `F1`-`F24`, and the named keys of [`key_from_name`].
    ///
    /// `MOD_NOREPEAT` is added: a hotkey held down should fire once. Strip it
    /// if the application wants auto-repeat.
    ///
    /// Returns `None` on anything it does not understand, which a caller
    /// should report as a settings error rather than silently ignore.
    pub fn parse(text: &str) -> Option<Self> {
        let mut modifiers = MOD_NOREPEAT.0;
        let mut key = None;

        for part in text.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }

            match part.to_ascii_lowercase().as_str() {
                "win" | "windows" | "super" | "meta" => modifiers |= MOD_WIN.0,
                "ctrl" | "control" => modifiers |= MOD_CONTROL.0,
                "alt" => modifiers |= MOD_ALT.0,
                "shift" => modifiers |= MOD_SHIFT.0,
                // The last non-modifier wins nothing: a second one is a typo,
                // not a hotkey.
                _ if key.is_some() => return None,
                _ => key = Some(key_from_name(part)?),
            }
        }

        Some(Self::new(HOT_KEY_MODIFIERS(modifiers), key?))
    }

    fn has(&self, modifier: HOT_KEY_MODIFIERS) -> bool {
        self.modifiers.0 & modifier.0 != 0
    }
}

/// Renders the combination the way [`Hotkey::parse`] reads it back.
impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (modifier, name) in [
            (MOD_WIN, "Win"),
            (MOD_CONTROL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
        ] {
            if self.has(modifier) {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", key_name(self.key))
    }
}

/// Virtual-key code for a key name, as written in a settings file.
///
/// Letters, digits, `F1`-`F24`, `Numpad0`-`Numpad9`, the named keys below,
/// and `0x…` for a raw virtual-key code — the escape hatch for a key with no
/// name here, and the form [`key_name`] falls back to, so the two round trip.
///
/// The punctuation names (`Semicolon`, `Slash`, `Quote`, …) are aliases for
/// the `OEM…` codes at the position those characters occupy on a US layout.
/// The code is what gets registered, so on another layout the key is where
/// that layout puts it, not where the name says. `OEM7` says the same thing
/// without the implication.
pub fn key_from_name(name: &str) -> Option<VIRTUAL_KEY> {
    // `LAUNCH_APP2`, `Launch-App2` and `LaunchApp2` are the same key.
    let upper: String = name
        .to_ascii_uppercase()
        .chars()
        .filter(|c| !matches!(c, '_' | '-'))
        .collect();
    let bytes = upper.as_bytes();

    // Single letter or digit: the virtual-key code is the ASCII code itself.
    if bytes.len() == 1 && (bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit()) {
        return Some(VIRTUAL_KEY(bytes[0] as u16));
    }

    if let Some(digits) = upper.strip_prefix("0X")
        && let Ok(code) = u16::from_str_radix(digits, 16)
        && code != 0
    {
        return Some(VIRTUAL_KEY(code));
    }

    if let Some(number) = upper.strip_prefix('F')
        && let Ok(index) = number.parse::<u16>()
        && (1..=24).contains(&index)
    {
        return Some(VIRTUAL_KEY(0x6F + index)); // VK_F1 == 0x70
    }

    if let Some(digit) = upper.strip_prefix("NUMPAD")
        && let Ok(index) = digit.parse::<u16>()
        && index <= 9
    {
        return Some(VIRTUAL_KEY(VK_NUMPAD0.0 + index));
    }

    Some(match upper.as_str() {
        "ESC" | "ESCAPE" => VK_ESCAPE,
        "SPACE" => VK_SPACE,
        "TAB" => VK_TAB,
        "ENTER" | "RETURN" => VK_RETURN,
        "BACKSPACE" | "BACK" => VK_BACK,
        "INSERT" | "INS" => VK_INSERT,
        "DELETE" | "DEL" => VK_DELETE,
        "HOME" => VK_HOME,
        "END" => VK_END,
        "PAGEUP" | "PGUP" => VK_PRIOR,
        "PAGEDOWN" | "PGDN" => VK_NEXT,
        "LEFT" => VK_LEFT,
        "RIGHT" => VK_RIGHT,
        "UP" => VK_UP,
        "DOWN" => VK_DOWN,
        "PRINTSCREEN" | "PRTSC" | "SNAPSHOT" => VK_SNAPSHOT,
        "PAUSE" => VK_PAUSE,
        "CLEAR" => VK_CLEAR,
        "MENU" | "APPS" | "CONTEXTMENU" => VK_APPS,
        "CAPSLOCK" | "CAPITAL" => VK_CAPITAL,
        "SCROLLLOCK" | "SCROLL" => VK_SCROLL,

        // Numeric keypad. Distinct codes from the number row, which is the
        // point: a hotkey on the keypad does not fire on the row above it.
        "NUMLOCK" => VK_NUMLOCK,
        "NUMADD" | "NUMPLUS" => VK_ADD,
        "NUMSUB" | "NUMMINUS" => VK_SUBTRACT,
        "NUMMUL" | "NUMSTAR" => VK_MULTIPLY,
        "NUMDIV" | "NUMSLASH" => VK_DIVIDE,
        "NUMDECIMAL" | "NUMDOT" => VK_DECIMAL,

        // Punctuation, by code and by US-layout position — see above.
        "OEM1" | "SEMICOLON" => VK_OEM_1,
        "OEM2" | "SLASH" => VK_OEM_2,
        "OEM3" | "TILDE" | "BACKQUOTE" => VK_OEM_3,
        "OEM4" | "LBRACKET" => VK_OEM_4,
        "OEM5" | "BACKSLASH" => VK_OEM_5,
        "OEM6" | "RBRACKET" => VK_OEM_6,
        "OEM7" | "QUOTE" => VK_OEM_7,
        "OEM8" => VK_OEM_8,
        // The extra key of an ISO keyboard, next to the left Shift: `<` and
        // `>` on most European layouts.
        "OEM102" | "<" | "LESS" => VK_OEM_102,
        "PLUS" | "EQUALS" => VK_OEM_PLUS,
        "COMMA" => VK_OEM_COMMA,
        "MINUS" => VK_OEM_MINUS,
        "PERIOD" | "DOT" => VK_OEM_PERIOD,

        // Media and browser keys. Bound on most keyboards already, which is
        // exactly why a tool may want to take one over.
        "VOLUMEMUTE" => VK_VOLUME_MUTE,
        "VOLUMEDOWN" => VK_VOLUME_DOWN,
        "VOLUMEUP" => VK_VOLUME_UP,
        "MEDIANEXT" => VK_MEDIA_NEXT_TRACK,
        "MEDIAPREV" => VK_MEDIA_PREV_TRACK,
        "MEDIASTOP" => VK_MEDIA_STOP,
        "MEDIAPLAYPAUSE" | "MEDIAPLAY" => VK_MEDIA_PLAY_PAUSE,
        "BROWSERBACK" => VK_BROWSER_BACK,
        "BROWSERFORWARD" => VK_BROWSER_FORWARD,
        "BROWSERREFRESH" => VK_BROWSER_REFRESH,
        "BROWSERSTOP" => VK_BROWSER_STOP,
        "BROWSERSEARCH" => VK_BROWSER_SEARCH,
        "BROWSERFAVORITES" => VK_BROWSER_FAVORITES,
        "BROWSERHOME" => VK_BROWSER_HOME,
        "LAUNCHMAIL" => VK_LAUNCH_MAIL,
        "LAUNCHMEDIA" | "LAUNCHMEDIASELECT" => VK_LAUNCH_MEDIA_SELECT,
        "LAUNCHAPP1" => VK_LAUNCH_APP1,
        "LAUNCHAPP2" => VK_LAUNCH_APP2,

        _ => return None,
    })
}

/// The name [`key_from_name`] accepts for `key`, or its hexadecimal code —
/// which [`key_from_name`] also accepts, so every key survives the round trip.
pub fn key_name(key: VIRTUAL_KEY) -> String {
    let code = key.0;

    if (b'A' as u16..=b'Z' as u16).contains(&code) || (b'0' as u16..=b'9' as u16).contains(&code) {
        return (code as u8 as char).to_string();
    }
    if (0x70..=0x87).contains(&code) {
        return format!("F{}", code - 0x6F);
    }
    if (VK_NUMPAD0.0..=VK_NUMPAD0.0 + 9).contains(&code) {
        return format!("Numpad{}", code - VK_NUMPAD0.0);
    }

    // Canonical names only: the aliases above all resolve to one of these.
    for name in [
        "Esc",
        "Space",
        "Tab",
        "Enter",
        "Backspace",
        "Insert",
        "Delete",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Left",
        "Right",
        "Up",
        "Down",
        "PrintScreen",
        "Pause",
        "Clear",
        "Menu",
        "CapsLock",
        "ScrollLock",
        "NumLock",
        "NumAdd",
        "NumSub",
        "NumMul",
        "NumDiv",
        "NumDecimal",
        "OEM1",
        "OEM2",
        "OEM3",
        "OEM4",
        "OEM5",
        "OEM6",
        "OEM7",
        "OEM8",
        "<",
        "Plus",
        "Comma",
        "Minus",
        "Period",
        "VolumeMute",
        "VolumeDown",
        "VolumeUp",
        "MediaNext",
        "MediaPrev",
        "MediaStop",
        "MediaPlayPause",
        "BrowserBack",
        "BrowserForward",
        "BrowserRefresh",
        "BrowserStop",
        "BrowserSearch",
        "BrowserFavorites",
        "BrowserHome",
        "LaunchMail",
        "LaunchMedia",
        "LaunchApp1",
        "LaunchApp2",
    ] {
        if key_from_name(name) == Some(key) {
            return name.to_owned();
        }
    }

    format!("0x{code:02X}")
}

/// What to do when `RegisterHotKey` refuses the combination.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Fallback {
    /// Report the failure. The right default: a combination owned by another
    /// application belongs to that application.
    #[default]
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
        watch_foreground(reconcile_overrides);
        Ok(())
    }

    /// Keystrokes are cheap to let through but not free: drop the hook as
    /// soon as no override needs it.
    fn uninstall_hook_if_idle(&mut self) {
        if state(|state| state.is_empty())
            && let Some(hook) = self.hook.take()
        {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
            unwatch_foreground(reconcile_overrides);
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

/// Registered with [`watch_foreground`] for as long as
/// [`HotkeyManager::install_hook`] has the keyboard hook up. Clears any
/// override's `pressed` flag whose key is not actually held any more per
/// [`GetAsyncKeyState`] — see this module's own doc comment for the gap this
/// closes. Left uncorrected, a `pressed` flag stuck `true` would either
/// swallow the next legitimate press as bogus auto-repeat (for a
/// `MOD_NOREPEAT` hotkey) or swallow its key-up outright (for one that
/// isn't), depending on which the hotkey was registered with.
fn reconcile_overrides() {
    state(|state| {
        for entry in &mut state.overrides {
            if entry.pressed
                && unsafe { GetAsyncKeyState(entry.hotkey.key.0 as i32) } as u16 & 0x8000 == 0
            {
                entry.pressed = false;
            }
        }
    });
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
            absorb_modifier_tap();
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

/// Cancels the system gesture that fires when Alt or Win is pressed and
/// released with nothing else happening in between — the Start menu for Win,
/// the menu bar's keyboard focus for Alt.
///
/// Swallowing `Win+X` (or catching `Alt+X` before it reaches a window) leaves
/// the shell seeing a lone press-then-release of the modifier, which is
/// exactly that gesture. Injecting a keystroke the shell does not act on
/// breaks the pair. Right shift is a safe filler: no shell gesture is bound
/// to it, and it does not change the meaning of anything the user types next.
///
/// The same trick [`crate::hotkey`] uses for an overridden `Win+…` hotkey;
/// `y-snap`'s `modifier_guard` calls it too, for an Alt- or Win-modified drag
/// or resize, which has exactly the same problem the moment the modifier key
/// comes back up. In both cases the call has to land synchronously in the
/// keyboard-hook callback that observes the modifier's real key-up,
/// immediately before that key-up is allowed to continue down the chain —
/// not at some earlier or unrelated point (mouse-button time, in y-snap's
/// case). Timing, not the choice of filler key, is what makes this work.
///
/// This is HulK's own `key_press_code(VK_RSHIFT)`, reproduced call for call
/// rather than reasoned out from scratch: two separate `SendInput` calls, one
/// event each, both carrying `KEYEVENTF_EXTENDEDKEY`. All three details
/// looked like they should not matter and turned out to. `wScan` is `0`: it
/// is `wVk`, not the scan code, that identifies the key here, so nothing
/// reads it. `KEYEVENTF_EXTENDEDKEY` looks wrong for a Shift key — Right
/// Shift is not an `0xE0`-prefixed key on a real keyboard — but HulK sets it
/// for `VK_RSHIFT` regardless, and dropping it is what stopped this from
/// working during development. And two `SendInput` calls is not the same
/// thing as one call given both events: batching them (as a single call with
/// a two-element array, the documentation's own preferred form for a key
/// tap) was tried here first and did not cancel the tap either.
///
/// One filler, always: earlier revisions of this function varied the filler
/// by which modifier was being cancelled (a `Ctrl`-`[modifier]`-`Ctrl`
/// sandwich borrowed from AltSnap). That was two designs layered on top of
/// each other where either alone is known to work from a reference
/// implementation; this is the single one, HulK's.
pub fn absorb_modifier_tap() {
    let extra_info: usize = unsafe { GetMessageExtraInfo().0 as usize };

    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RSHIFT,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: extra_info, // OWN_INPUT_SIGNATURE,
            },
        },
    };

    // Two separate SendInput calls, not one batched call — see doc comment.
    unsafe {
        SendInput(
            &[key(KEYBD_EVENT_FLAGS(KEYEVENTF_EXTENDEDKEY.0))],
            size_of::<INPUT>() as i32,
        )
    };
    unsafe {
        SendInput(
            &[key(KEYBD_EVENT_FLAGS(
                KEYEVENTF_EXTENDEDKEY.0 | KEYEVENTF_KEYUP.0,
            ))],
            size_of::<INPUT>() as i32,
        )
    };
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
const OWN_INPUT_SIGNATURE: usize = 0x7970_6F5F_6C69_7465; // "ypo_lite"

/// The `WinEventHook` recovery net — see this module's doc comment.
struct ForegroundWatch {
    hook: Option<HWINEVENTHOOK>,
    /// Every caller currently relying on the watch, run in registration
    /// order on each foreground switch. A `Vec` of plain function pointers
    /// rather than boxed closures: every reconciler so far reaches its own
    /// state through its own thread local, so there is nothing to capture.
    reconcilers: Vec<fn()>,
}

thread_local! {
    static FOREGROUND: RefCell<ForegroundWatch> = const {
        RefCell::new(ForegroundWatch {
            hook: None,
            reconcilers: Vec::new(),
        })
    };
}

/// Registers `reconcile` to run every time the foreground window changes,
/// installing the `WinEventHook` watchdog on this thread if it is not
/// already up. Idempotent for a given function pointer — calling this twice
/// with the same `reconcile` does not run it twice per switch.
///
/// `reconcile` should compare whatever state its caller tracks across a
/// key-down and a later key-up against [`GetAsyncKeyState`] and force-clear
/// what disagrees — see this module's doc comment for why a foreground
/// switch is the right moment to do that, and [`unwatch_foreground`] for
/// letting go once that state has nothing left to reconcile.
pub fn watch_foreground(reconcile: fn()) {
    FOREGROUND.with(|cell| {
        let mut watch = cell.borrow_mut();
        if !watch
            .reconcilers
            .iter()
            .any(|&registered| std::ptr::fn_addr_eq(registered, reconcile))
        {
            watch.reconcilers.push(reconcile);
        }

        if watch.hook.is_none() {
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground_switch),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            if !hook.0.is_null() {
                watch.hook = Some(hook);
            }
        }
    });
}

/// Unregisters `reconcile`, uninstalling the watchdog once nothing is left
/// watching. A caller that never lets its armed/pressed state go fully idle
/// should still call this on the same lifecycle as it calls
/// [`watch_foreground`] — the hook is cheap, but not free, and there is no
/// reason to keep watching foreground switches on behalf of a hotkey the
/// user has since unregistered.
pub fn unwatch_foreground(reconcile: fn()) {
    FOREGROUND.with(|cell| {
        let mut watch = cell.borrow_mut();
        watch
            .reconcilers
            .retain(|&registered| !std::ptr::fn_addr_eq(registered, reconcile));
        if watch.reconcilers.is_empty()
            && let Some(hook) = watch.hook.take()
        {
            let _ = unsafe { UnhookWinEvent(hook) };
        }
    });
}

unsafe extern "system" fn foreground_switch(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // Snapshotted rather than run under the borrow: a reconciler that goes
    // fully idle calls `unwatch_foreground`, which needs its own mutable
    // borrow of the same thread local, and this procedure cannot know in
    // advance which of its callers will do that.
    let reconcilers = FOREGROUND.with(|cell| cell.borrow().reconcilers.clone());
    for reconcile in reconcilers {
        reconcile();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_combination() {
        let hotkey = Hotkey::parse("Win+Shift+F12").expect("valid");
        assert_eq!(hotkey.key, VIRTUAL_KEY(0x7B));
        assert!(hotkey.has(MOD_WIN) && hotkey.has(MOD_SHIFT) && hotkey.has(MOD_NOREPEAT));
        assert!(!hotkey.has(MOD_ALT) && !hotkey.has(MOD_CONTROL));
    }

    #[test]
    fn round_trips_through_display() {
        for text in [
            "Win+Shift+F12",
            "Ctrl+Alt+A",
            "Win+Space",
            "Ctrl+PageDown",
            "Win+<",
            "LaunchApp2",
        ] {
            let hotkey = Hotkey::parse(text).expect("valid");
            assert_eq!(hotkey.to_string(), text);
        }
    }

    #[test]
    fn is_case_and_space_insensitive() {
        assert_eq!(
            Hotkey::parse("ctrl + alt + del"),
            Hotkey::parse("Ctrl+Alt+Delete")
        );
    }

    #[test]
    fn understands_iso_and_launch_keys() {
        assert_eq!(Hotkey::parse("Win+<").map(|h| h.key), Some(VK_OEM_102));
        assert_eq!(Hotkey::parse("LAUNCH_APP2").map(|h| h.key), Some(VK_LAUNCH_APP2));
        assert_eq!(Hotkey::parse("launch-mail").map(|h| h.key), Some(VK_LAUNCH_MAIL));
    }

    #[test]
    fn rejects_nonsense() {
        for text in ["", "Win+", "Ctrl", "Ctrl+A+B", "Win+F25", "Ctrl+Nope"] {
            assert!(Hotkey::parse(text).is_none(), "{text} should not parse");
        }
    }
}
