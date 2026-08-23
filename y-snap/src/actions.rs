//! Carrying out an [`Action`].
//!
//! One place where a configured action becomes a Win32 call, so that adding
//! an action means adding a name in [`crate::action`] and an arm here — and
//! neither the hotkey path nor the overlay path has to learn about it.

use windows::Win32::Foundation::HWND;

use crate::action::Action;
use crate::window;

/// What came of dispatching an action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Done,
    /// The action needed a window and there was none worth acting on.
    NoTarget,
    /// Configured, understood, and not written yet.
    Pending,
}

/// Runs `action` against `window`.
///
/// [`Action::Quit`] and [`Action::Reload`] are not here: they act on y-snap
/// itself and need the application, not a window.
pub fn dispatch(action: Action, target: Option<HWND>) -> Status {
    if action.needs_window() && target.is_none() {
        return Status::NoTarget;
    }

    match (action, target) {
        (Action::Minimize, Some(window)) => window::minimize(window),
        (Action::Maximize, Some(window)) => window::maximize(window),
        (Action::Close, Some(window)) => window::close(window),
        (Action::AlwaysOnTop, Some(window)) => window::always_on_top(window),
        (Action::Borderless, Some(window)) => window::borderless(window),
        (Action::Transparency, Some(window)) => window::transparency(window),
        (Action::Shade, Some(window)) => window::shade(window),
        (Action::HideTaskbar, Some(window)) => window::hide_taskbar(window),
        (Action::CenterCursor, Some(window)) => window::center_cursor(window),

        // Still to come, each its own piece of work:
        //   overlay          — the action popup, on y_core::overlay
        //   iconize/traynize — hiding windows behind a notification icon
        //   desktop-*        — the virtual desktop COM interfaces
        _ => return Status::Pending,
    }

    Status::Done
}
