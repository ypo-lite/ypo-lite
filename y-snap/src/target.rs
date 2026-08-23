//! Choosing the window an action applies to.
//!
//! Two ways to point at a window: the keyboard focus, or the mouse. Neither
//! is right on its own — a hotkey pressed while the pointer sits over the
//! desktop should still act on what the user is working in — so `Pointed`
//! falls back to `Focused`.
//!
//! The hard part is not finding a window but refusing the wrong ones. Under
//! the pointer there is always *something*: the taskbar, a tooltip, the task
//! switcher, the desktop itself. Minimizing the shell is not a feature.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetCursorPos, GetDesktopWindow,
    GetForegroundWindow, GetShellWindow, GetWindowLongW, IsWindowVisible, WS_EX_TOOLWINDOW,
    WindowFromPoint,
};

use crate::action::Target;

/// The window this action should act on, or `None` if there is nothing worth
/// acting on.
pub fn resolve(target: Target) -> Option<HWND> {
    match target {
        Target::Focused => focused(),
        Target::Pointed => pointed().or_else(focused),
    }
}

/// The foreground window, if it is a window a user would call theirs.
pub fn focused() -> Option<HWND> {
    accept(unsafe { GetForegroundWindow() })
}

/// The top-level window under the mouse pointer.
pub fn pointed() -> Option<HWND> {
    let mut position = Default::default();
    unsafe { GetCursorPos(&mut position) }.ok()?;
    at(position)
}

/// The top-level window under `point`, in screen coordinates — what
/// [`crate::mouse`] uses to resolve drag, resize and the minimize-button
/// hover from the position a low-level hook hands it, rather than from
/// [`GetCursorPos`], which would just be racing the hook for the same value.
pub fn at(point: POINT) -> Option<HWND> {
    // WindowFromPoint returns the deepest child — a button, a tab, a text
    // area. Every action here is about the window as a whole, so walk up.
    let window = unsafe { WindowFromPoint(point) };
    accept(unsafe { GetAncestor(window, GA_ROOT) })
}

/// Whether `window` is a window the user manages, as opposed to a piece of
/// the shell that merely happens to be under the pointer.
fn accept(window: HWND) -> Option<HWND> {
    if window.is_invalid() {
        return None;
    }

    // The desktop and the shell are not applications, whatever they look like.
    let shell = unsafe { GetShellWindow() };
    let desktop = unsafe { GetDesktopWindow() };
    if window == shell || window == desktop {
        return None;
    }

    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return None;
    }

    if is_banished(window) {
        return None;
    }

    Some(window)
}

/// Windows belonging to the shell and to transient UI, by class name.
///
/// There is no flag that means "this is not the user's window". A tooltip and
/// a text editor are both plain top-level windows; `WS_EX_TOOLWINDOW` catches
/// some but is also worn by legitimate palettes. So: a list, grown by
/// observation, of the classes that turn up under the pointer and must never
/// be acted on. Missing one costs a shell element behaving strangely, which
/// is why the list errs on the side of refusing.
fn is_banished(window: HWND) -> bool {
    const BANISHED: &[&str] = &[
        // The shell itself.
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "NotifyIconOverflowWindow",
        "TopLevelWindowForOverflowXamlIsland",
        // Task switching and the task view.
        "TaskSwitcherWnd",
        "TaskSwitcherOverlayWnd",
        "MultitaskingViewFrame",
        "TaskListThumbnailWnd",
        "XamlExplorerHostIslandWindow",
        // Transient UI that is nobody's window.
        "tooltips_class32",
        "#32768", // a menu
        "Xaml_WindowedPopupClass",
        "IPTip_Main_Window",
        // Immersive shell surfaces: the Start menu, search, notifications.
        "Windows.UI.Core.CoreWindow",
        "ImmersiveLauncher",
        "ImmersiveSplashScreenWindowClass",
    ];

    let mut buffer = [0u16; 256];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    if length <= 0 {
        return true; // no class name, no confidence
    }

    let class = String::from_utf16_lossy(&buffer[..length as usize]);

    // ApplicationFrameWindow hosts real store applications, which are fair
    // game; it is only banished while it is the empty frame the shell keeps
    // around, which has no taskbar presence.
    if class == "ApplicationFrameWindow" {
        let ex_style = unsafe { GetWindowLongW(window, GWL_EXSTYLE) } as u32;
        return ex_style & WS_EX_TOOLWINDOW.0 != 0;
    }

    BANISHED.contains(&class.as_str())
}
