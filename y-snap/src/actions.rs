//! Carrying out an [`Action`].
//!
//! One place where a configured action becomes a Win32 call, so that adding
//! an action means adding a name in [`crate::action`] and an arm here — and
//! neither the hotkey path nor the overlay path has to learn about it.

use windows::Win32::Foundation::HWND;
use y_core::virtual_desktop::{self, Direction};

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

/// Runs `action` against `window`. `desktop` is only read by
/// [`Action::DesktopMove`] — the 1-based desktop number from the `desktop`
/// attribute, `0` (unset) for every other action.
///
/// [`Action::Quit`] and [`Action::Reload`] are not here: they act on y-snap
/// itself and need the application, not a window. Nor is [`Action::Overlay`]
/// — it needs the application's settings and owner window too, so `main.rs`
/// special-cases it the same way, before this is ever reached.
pub fn dispatch(action: Action, target: Option<HWND>, desktop: u32) -> Status {
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

        // Neither needs a window (see `Action::needs_window`), so `target`
        // is `None` when this fires from a hotkey — a plain switch, nothing
        // carried along. From an overlay tile it can still be `Some`, since
        // the overlay always resolves one before dispatching: "switch and
        // bring the window I opened this overlay on" is a reasonable thing
        // for a tile to mean even though a hotkey can't ask for it.
        (Action::DesktopNext, _) => {
            let _ = virtual_desktop::switch(Direction::Right, target);
        }
        (Action::DesktopPrev, _) => {
            let _ = virtual_desktop::switch(Direction::Left, target);
        }
        (Action::DesktopNew, _) => {
            let _ = virtual_desktop::create_desktop();
        }

        (Action::DesktopMove, Some(window)) => {
            // 1-based in the configuration file, 0-based on the COM side.
            let Some(index) = desktop.checked_sub(1) else {
                return Status::Done;
            };
            let _ = virtual_desktop::move_window_to_desktop(window, index);
        }
        (Action::PinToggle, Some(window)) => {
            let _ = virtual_desktop::toggle_window_pinned(window);
        }

        // Still to come: iconize/traynize, hiding windows behind a
        // notification icon.
        _ => return Status::Pending,
    }

    Status::Done
}

/// Whether `action` currently does something real when dispatched — every
/// arm [`dispatch`] does not fall through to [`Status::Pending`] for. The
/// action overlay renders a tile disabled, rather than pretending it works,
/// for anything this says no to.
pub fn is_implemented(action: Action) -> bool {
    matches!(
        action,
        Action::Minimize
            | Action::Maximize
            | Action::Close
            | Action::AlwaysOnTop
            | Action::Borderless
            | Action::Transparency
            | Action::Shade
            | Action::HideTaskbar
            | Action::CenterCursor
            | Action::DesktopNext
            | Action::DesktopPrev
            | Action::DesktopNew
            | Action::DesktopMove
            | Action::PinToggle
    )
}

/// The on/off state behind a toggle action's tile, for whichever of them
/// have one — `None` for an action with no such concept (`minimize`,
/// `close`, `center-cursor`, …), not just for one [`is_implemented`] refuses.
pub fn toggle_state(action: Action, window: HWND) -> Option<bool> {
    Some(match action {
        Action::Maximize => window::is_maximized(window),
        Action::AlwaysOnTop => window::is_always_on_top(window),
        Action::Borderless => window::is_borderless(window),
        Action::Transparency => window::is_faded(window),
        Action::Shade => window::is_shaded(window),
        Action::HideTaskbar => window::is_hidden_from_taskbar(window),
        Action::PinToggle => virtual_desktop::is_window_pinned(window).unwrap_or(false),
        _ => return None,
    })
}
