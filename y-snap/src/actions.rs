//! Carrying out an [`Action`].
//!
//! One place where a configured action becomes a Win32 call, so that adding
//! an action means adding a name in [`crate::action`] and an arm here — and
//! neither the hotkey path nor the overlay path has to learn about it.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
use y_core::monitor::{self, Monitor};
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
/// [`Action::DesktopMove`] and [`Action::DesktopSend`] — the 1-based desktop number from the `desktop`
/// attribute, `0` (unset) for every other action.
///
/// [`Action::Quit`] and [`Action::Reload`] are not here: they act on y-snap
/// itself and need the application, not a window. Nor is [`Action::Overlay`]
/// — it needs the application's settings and owner window too, so `main.rs`
/// special-cases it the same way, before this is ever reached. The same goes
/// for iconize, traynize and the two uniconize actions: the windows they hide
/// are kept in `main.rs`'s [`crate::hidden::Hidden`].
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

        // `move` follows the window to its new desktop, `send` stays where
        // it is. Either way `desktop` is only read by the numbered pair.
        (Action::DesktopMove, Some(window)) => relocate(window, Place::Number(desktop), true),
        (Action::DesktopMoveNext, Some(window)) => relocate(window, Place::Next, true),
        (Action::DesktopMovePrev, Some(window)) => relocate(window, Place::Prev, true),
        (Action::DesktopSend, Some(window)) => relocate(window, Place::Number(desktop), false),
        (Action::DesktopSendNext, Some(window)) => relocate(window, Place::Next, false),
        (Action::DesktopSendPrev, Some(window)) => relocate(window, Place::Prev, false),
        (Action::PinToggle, Some(window)) => {
            let _ = virtual_desktop::toggle_window_pinned(window);
        }

        // `move` takes the pointer along, `send` leaves it.
        (Action::MonitorMoveNext, Some(window)) => to_monitor(window, true, true),
        (Action::MonitorMovePrev, Some(window)) => to_monitor(window, false, true),
        (Action::MonitorSendNext, Some(window)) => to_monitor(window, true, false),
        (Action::MonitorSendPrev, Some(window)) => to_monitor(window, false, false),

        // Handled by the application before it gets here, so that nothing
        // reaching this arm is silently ignored: the rest of what the
        // configuration can name is still to come.
        _ => return Status::Pending,
    }

    Status::Done
}

/// Where `desktop-move`/`desktop-send` put the window.
enum Place {
    /// The 1-based number from the `desktop` attribute; 0 (unset) does nothing.
    Number(u32),
    Next,
    Prev,
}

/// Puts `window` on another desktop, and when `follow` switches to that
/// desktop too (`desktop-move`); otherwise (`desktop-send`) the current
/// desktop stays as it is. Next and previous are relative to the current
/// desktop and wrap around at either end. Nothing happens when there is
/// nowhere to go, or the desktops cannot be read.
fn relocate(window: HWND, place: Place, follow: bool) {
    let index = match place {
        // 1-based in the configuration file, 0-based on the COM side.
        Place::Number(number) => number.checked_sub(1),
        Place::Next | Place::Prev => {
            let (Some(count), Some(current)) = (
                virtual_desktop::desktop_count(),
                virtual_desktop::current_desktop_index(),
            ) else {
                return;
            };
            wrapped_neighbour(current, count, matches!(place, Place::Next))
        }
    };
    let Some(index) = index else {
        return;
    };
    // Only follow a window that actually moved: a number past the last
    // desktop would otherwise switch away for nothing.
    if virtual_desktop::move_window_to_desktop(window, index).is_ok() && follow {
        let _ = virtual_desktop::switch_to(index, None);
    }
}

/// Sends `window` to the next or previous monitor, and when `follow` takes the
/// pointer along to the same relative spot of that monitor's work area
/// (`monitor-move`); otherwise (`monitor-send`) the pointer stays put, so with
/// `target="pointed"` it is now over whatever lies beneath the old place.
/// The monitors are counted left to right, wrapping at the ends. With one
/// monitor, or a minimized window, nothing happens.
fn to_monitor(window: HWND, next: bool, follow: bool) {
    if window::is_minimized(window) {
        return;
    }
    // Read fresh each time: monitors come and go.
    let monitors = Monitor::all();
    let here = Monitor::from_window(window).handle;
    let Some(index) = monitors.iter().position(|m| m.handle == here) else {
        return;
    };
    let Some(destination) =
        wrapped_neighbour(index as u32, monitors.len() as u32, next).map(|i| i as usize)
    else {
        return;
    };

    let pointer = follow.then(|| monitor::cursor_position().ok()).flatten();
    window::move_to_monitor(window, &monitors[destination]);

    if let Some(pointer) = pointer {
        let spot = scaled_point(
            pointer,
            monitors[index].work_area,
            monitors[destination].work_area,
        );
        let _ = unsafe { SetCursorPos(spot.x, spot.y) };
    }
}

/// Where `point`, placed in the work area `from`, falls in the work area `to`:
/// the same fraction of the way across and down, kept inside `to`.
fn scaled_point(point: POINT, from: RECT, to: RECT) -> POINT {
    let scale = |value: i32, from_start: i32, from_end: i32, to_start: i32, to_end: i32| {
        let from_len = (from_end - from_start) as i64;
        let to_len = (to_end - to_start) as i64;
        let offset = if from_len <= 0 {
            to_len / 2
        } else {
            (value - from_start) as i64 * to_len / from_len
        };
        to_start + offset.clamp(0, (to_len - 1).max(0)) as i32
    };
    POINT {
        x: scale(point.x, from.left, from.right, to.left, to.right),
        y: scale(point.y, from.top, from.bottom, to.top, to.bottom),
    }
}

/// The 0-based desktop after or before `current` out of `count`, circling
/// round at the ends. `None` when there is nowhere else to go.
fn wrapped_neighbour(current: u32, count: u32, next: bool) -> Option<u32> {
    if count < 2 || current >= count {
        return None;
    }
    Some(if next {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    })
}

/// Whether `action` currently does something real when dispatched — every
/// arm [`dispatch`] does not fall through to [`Status::Pending`] for, and the
/// ones the application handles itself (see [`dispatch`]). The
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
            | Action::DesktopMoveNext
            | Action::DesktopMovePrev
            | Action::DesktopSend
            | Action::DesktopSendNext
            | Action::DesktopSendPrev
            | Action::PinToggle
            | Action::MonitorMoveNext
            | Action::MonitorMovePrev
            | Action::MonitorSendNext
            | Action::MonitorSendPrev
            | Action::Iconize
            | Action::Traynize
            | Action::Uniconize
            | Action::UniconizeAll
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

#[cfg(test)]
mod tests {
    use super::{scaled_point, wrapped_neighbour};
    use windows::Win32::Foundation::{POINT, RECT};

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn the_pointer_keeps_its_fraction_of_the_work_area() {
        let from = rect(0, 0, 1000, 800);
        let to = rect(1000, 0, 3000, 1600);
        let spot = scaled_point(POINT { x: 250, y: 400 }, from, to);
        assert_eq!((spot.x, spot.y), (1500, 800));
    }

    #[test]
    fn the_pointer_stays_inside_the_target() {
        let from = rect(0, 0, 1000, 800);
        let to = rect(-500, 0, 0, 400);
        // Off the source monitor altogether, and at its far corner.
        let outside = scaled_point(POINT { x: 5000, y: -50 }, from, to);
        assert_eq!((outside.x, outside.y), (-1, 0));
        let corner = scaled_point(POINT { x: 1000, y: 800 }, from, to);
        assert_eq!((corner.x, corner.y), (-1, 399));
    }

    #[test]
    fn moving_next_and_prev_circles_round() {
        assert_eq!(wrapped_neighbour(0, 3, true), Some(1));
        assert_eq!(wrapped_neighbour(2, 3, true), Some(0));
        assert_eq!(wrapped_neighbour(2, 3, false), Some(1));
        assert_eq!(wrapped_neighbour(0, 3, false), Some(2));
    }

    #[test]
    fn nowhere_to_go_with_one_desktop_or_a_stale_index() {
        assert_eq!(wrapped_neighbour(0, 1, true), None);
        assert_eq!(wrapped_neighbour(0, 1, false), None);
        assert_eq!(wrapped_neighbour(0, 0, true), None);
        assert_eq!(wrapped_neighbour(5, 3, true), None);
    }
}
