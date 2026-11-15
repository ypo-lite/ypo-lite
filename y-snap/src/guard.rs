//! When y-snap should stay out of the way.
//!
//! Two signals, both measured live with `examples/finder_probe.rs` against a
//! real game and a full-screen video (see `docs/design/mouse-finder-plan.md`):
//!
//! * **Confined pointer** (`GetClipCursor` smaller than the virtual screen).
//!   A game in play does it, and so would a remote desktop or a virtual
//!   machine. The application owns the pointer: nothing y-snap does with it
//!   (dragging a window, a shortcut, a hotkey, the finder) makes sense, so
//!   **all of y-snap is suspended** while it lasts — [`confined`], and
//!   [`confined_cached`] for the mouse hook.
//! * **A full-screen window on the pointer's monitor.** A game in its menus
//!   (pointer free again) or a full-screen video does not confine anything, so
//!   this is the only sign of them. It keeps the **automatic finder triggers**
//!   quiet. It is the pointer's monitor that counts: a video full-screen on
//!   one monitor does not stop the finder from helping with a pointer lost on
//!   another one.
//!
//! What was *not* kept: `SHQueryUserNotificationState` (it said "busy" for the
//! whole game, even in its menus and for seconds after leaving it, and
//! "app" for the Start menu, so it adds nothing the two signals above do not
//! tell better) and "the pointer is hidden" (it flips as a text field takes
//! focus, in the terminal as much as in a game).
//!
//! An explicit `find-mouse` (hotkey, shortcut, tray) ignores the full-screen
//! check and the exclude list: whoever asks wants the answer. It does not
//! ignore a confined pointer, since y-snap is suspended then.

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClassNameW, GetClipCursor, GetForegroundWindow, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowRect, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WS_CAPTION,
};
use y_core::monitor::Monitor;

use crate::config::normalize_process;

/// What the system says about the foreground at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signals {
    /// The pointer is confined to a smaller area than the whole desktop.
    pub confined: bool,
    /// The foreground window is full-screen on the pointer's monitor.
    pub fullscreen: bool,
    /// The foreground application, as [`normalize_process`] spells it.
    pub process: Option<String>,
}

/// Why an automatic trigger stays quiet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Confined,
    Excluded,
    Fullscreen,
}

/// Whether an automatic trigger must stay quiet. `fullscreen_check` is the
/// `guard` setting; the exclude list and a confined pointer always count.
pub fn blocks(signals: &Signals, exclude: &[String], fullscreen_check: bool) -> Option<Reason> {
    if signals.confined {
        return Some(Reason::Confined);
    }
    if let Some(process) = &signals.process
        && exclude.contains(process)
    {
        return Some(Reason::Excluded);
    }
    if fullscreen_check && signals.fullscreen {
        return Some(Reason::Fullscreen);
    }
    None
}

/// Reads the signals now. Not for a hook: it asks the system for the
/// foreground application, which is more than a hook may spend.
pub fn read(pointer: POINT) -> Signals {
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return Signals { confined: confined(), ..Signals::default() };
    }
    let process = y_core::process::executable_stem(y_core::process::window_process_id(window))
        .map(|name| normalize_process(&name));
    Signals {
        confined: confined(),
        fullscreen: is_fullscreen_on(window, pointer),
        process,
    }
}

/// Whether the pointer is confined to less than the virtual screen.
pub fn confined() -> bool {
    let mut clip = RECT::default();
    if unsafe { GetClipCursor(&mut clip) }.is_err() {
        return false;
    }
    let (x, y) = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN)) };
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
    is_confined(clip, RECT { left: x, top: y, right: x + w, bottom: y + h })
}

fn is_confined(clip: RECT, virtual_screen: RECT) -> bool {
    clip != virtual_screen
}

/// How long [`confined_cached`] trusts its last answer.
const CACHE_MS: u32 = 250;

thread_local! {
    static CACHE: Cell<Option<(u32, bool)>> = const { Cell::new(None) };
}

/// [`confined`] for a hook: one system call at most every quarter of a second,
/// whatever the number of events. `now` is the event's own time in
/// milliseconds.
pub fn confined_cached(now: u32) -> bool {
    CACHE.with(|cache| match cache.get() {
        Some((at, value)) if now.wrapping_sub(at) < CACHE_MS => value,
        _ => {
            let value = confined();
            cache.set(Some((now, value)));
            value
        }
    })
}

/// Whether `window` is full-screen on the monitor the pointer is on.
fn is_fullscreen_on(window: HWND, pointer: POINT) -> bool {
    let monitor = Monitor::from_window(window);
    if monitor.handle != Monitor::from_point(pointer).handle {
        return false;
    }
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(window, &mut rect) }.is_err() {
        return false;
    }
    let mut class = [0u16; 64];
    let length = unsafe { GetClassNameW(window, &mut class) }.max(0) as usize;
    let class = String::from_utf16_lossy(&class[..length]);
    let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as u32;
    covers_screen(rect, monitor.bounds, &class, style)
}

/// A window is full-screen when it covers its monitor, has no title bar (a
/// maximised window has one, and can cover a monitor whose taskbar hides) and
/// is not the desktop or the taskbar, which cover it too.
fn covers_screen(rect: RECT, bounds: RECT, class: &str, style: u32) -> bool {
    const SHELL: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
    let covers = rect.left <= bounds.left
        && rect.top <= bounds.top
        && rect.right >= bounds.right
        && rect.bottom >= bounds.bottom;
    covers && style & WS_CAPTION.0 != WS_CAPTION.0 && !SHELL.contains(&class)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE};

    const SCREEN: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1200 };
    const POPUP: u32 = WS_POPUP.0 | WS_VISIBLE.0;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_confined_pointer_blocks_whatever_the_settings() {
        let signals = Signals { confined: true, ..Signals::default() };
        assert_eq!(blocks(&signals, &[], false), Some(Reason::Confined));
    }

    #[test]
    fn an_excluded_application_always_blocks() {
        let signals = Signals { process: Some("game".into()), ..Signals::default() };
        assert_eq!(blocks(&signals, &names(&["game"]), false), Some(Reason::Excluded));
        assert_eq!(blocks(&signals, &names(&["other"]), true), None);
    }

    #[test]
    fn full_screen_blocks_only_while_the_guard_is_on() {
        let signals = Signals { fullscreen: true, ..Signals::default() };
        assert_eq!(blocks(&signals, &[], true), Some(Reason::Fullscreen));
        assert_eq!(blocks(&signals, &[], false), None);
        assert_eq!(blocks(&Signals::default(), &[], true), None);
    }

    #[test]
    fn the_pointer_is_confined_when_the_clip_is_smaller_than_the_desktop() {
        let game = RECT { left: 1, top: 1, right: 1919, bottom: 1199 };
        assert!(is_confined(game, SCREEN));
        assert!(!is_confined(SCREEN, SCREEN));
    }

    #[test]
    fn a_borderless_window_over_the_monitor_is_full_screen() {
        // What the game and the video showed in the probe.
        assert!(covers_screen(SCREEN, SCREEN, "UnrealWindow", POPUP));
        assert!(covers_screen(SCREEN, SCREEN, "Qt5QWindowIcon", POPUP));
    }

    #[test]
    fn a_maximised_window_with_a_title_bar_is_not() {
        let big = RECT { left: -8, top: -8, right: 1928, bottom: 1208 };
        assert!(!covers_screen(big, SCREEN, "Notepad", WS_OVERLAPPEDWINDOW.0));
    }

    #[test]
    fn the_desktop_and_the_taskbar_are_not() {
        for class in ["Progman", "WorkerW", "Shell_TrayWnd"] {
            assert!(!covers_screen(SCREEN, SCREEN, class, POPUP));
        }
    }

    #[test]
    fn a_window_that_does_not_reach_the_edges_is_not() {
        let small = RECT { left: 0, top: 0, right: 1900, bottom: 1200 };
        assert!(!covers_screen(small, SCREEN, "Qt5QWindowIcon", POPUP));
    }
}
