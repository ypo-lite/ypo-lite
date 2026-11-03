//! What y-snap does to a window.
//!
//! Every toggle here has to remember what it changed, because Windows will
//! not: stripping a window's frame throws the old style away, and the
//! application that owns the window has no idea we did it. State could live
//! in a map inside y-snap, but then quitting and restarting would strand
//! every window we had touched — the frame gone, and nothing left that knows
//! how to put it back.
//!
//! So the state lives on the window itself, as a window property. Properties
//! are per-window, survive us being restarted, and are freed with the window,
//! which means the worst case of y-snap being killed is a window that stays
//! borderless until someone borderlesses it again — not a leak, and not a
//! window we can no longer fix.

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetPropW, GetWindowPlacement, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, HWND_NOTOPMOST, HWND_TOPMOST, IsWindow, IsWindowArranged, IsWindowVisible,
    LWA_ALPHA, PostMessageW, RemovePropW, SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    SW_SHOWMAXIMIZED, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SetCursorPos, SetForegroundWindow, SetLayeredWindowAttributes, SetPropW, SetWindowPlacement,
    SetWindowPos, ShowWindow, WINDOWPLACEMENT, WM_CLOSE, WS_CAPTION, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MAXIMIZE, WS_MINIMIZE, WS_THICKFRAME,
};
use windows::core::{PCWSTR, w};
use y_core::monitor::Monitor;

/// How transparent `transparency` makes a window. Enough to see through,
/// not so much that the window becomes unusable.
const FADED_ALPHA: u8 = 180;

/// The style bits `borderless` removes. `WS_SYSMENU` deliberately stays: the
/// window keeps its Alt+Space menu, which is often the only way left to move
/// a window with no title bar to grab.
const FRAME: u32 = WS_CAPTION.0 | WS_THICKFRAME.0;

// Window properties, one per toggle. The names are namespaced because they
// live on windows belonging to other applications.
const PROP_STYLE: PCWSTR = w!("y-snap.style");
const PROP_EX_STYLE: PCWSTR = w!("y-snap.exstyle");
const PROP_HEIGHT: PCWSTR = w!("y-snap.height");

/// Minimizes the window.
pub fn minimize(window: HWND) {
    let _ = unsafe { ShowWindow(window, SW_MINIMIZE) };
}

/// Maximizes the window, or restores it if it is already maximized.
pub fn maximize(window: HWND) {
    let show = if is_maximized(window) {
        SW_RESTORE
    } else {
        SW_MAXIMIZE
    };
    let _ = unsafe { ShowWindow(window, show) };
}

/// Asks the window to close.
///
/// `WM_CLOSE`, not `DestroyWindow`: the application gets to run its own close
/// handling — save prompts, confirmation dialogs — and to refuse. Killing the
/// window from outside would lose work.
pub fn close(window: HWND) {
    let _ = unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) };
}

/// Takes the window off the screen without closing it: minimized first, so
/// that [`bring_back`] returns it to the state it had (a maximized window
/// comes back maximized), then hidden.
///
/// Deliberately not `hide_taskbar`'s trick of changing the extended style: an
/// application with several windows would lose the taskbar buttons of the ones
/// still showing. `SW_HIDE` takes only this window.
pub fn hide_away(window: HWND) {
    let _ = unsafe { ShowWindow(window, SW_MINIMIZE) };
    let _ = unsafe { ShowWindow(window, SW_HIDE) };
}

/// Undoes [`hide_away`] and puts the window in front. y-snap is acting on a
/// click or a key it was just given, so Windows lets it take the foreground.
pub fn bring_back(window: HWND) {
    let _ = unsafe { ShowWindow(window, SW_SHOW) };
    let _ = unsafe { ShowWindow(window, SW_RESTORE) };
    let _ = unsafe { SetForegroundWindow(window) };
}

/// The window's title, as the owner last set it. Read from the system's own
/// copy: it does not wait on the other application, so a hung one cannot
/// block the caller.
pub fn title(window: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(window) };
    if length <= 0 {
        return String::new();
    }

    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

/// Whether the handle still names a window.
pub fn exists(window: HWND) -> bool {
    unsafe { IsWindow(Some(window)) }.as_bool()
}

/// Whether the window is shown on the desktop (minimized counts; hidden does
/// not).
pub fn is_visible(window: HWND) -> bool {
    unsafe { IsWindowVisible(window) }.as_bool()
}

/// Keeps the window above every other, or stops.
pub fn always_on_top(window: HWND) {
    let after = if is_always_on_top(window) {
        HWND_NOTOPMOST
    } else {
        HWND_TOPMOST
    };

    let _ = unsafe {
        SetWindowPos(
            window,
            Some(after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

/// Strips the title bar and frame, or puts back exactly what was there.
pub fn borderless(window: HWND) {
    match take_prop(window, PROP_STYLE) {
        Some(original) => set_style(window, GWL_STYLE, original as u32),
        None => {
            let style = style(window, GWL_STYLE);
            if style & FRAME == 0 {
                return; // nothing to strip: leave it alone rather than mark it
            }
            set_prop(window, PROP_STYLE, style as usize);
            set_style(window, GWL_STYLE, style & !FRAME);
        }
    }

    // The non-client area changed size, so the frame has to be recomputed;
    // without SWP_FRAMECHANGED the window keeps drawing the old one.
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    };
}

/// Fades the window, or makes it opaque again.
pub fn transparency(window: HWND) {
    if take_prop(window, PROP_EX_STYLE).is_some() {
        // We added WS_EX_LAYERED, so we take it away. Removing the bit is
        // what restores full opacity; setting alpha back to 255 would leave
        // the window layered, and a layered window renders differently.
        let ex_style = style(window, GWL_EXSTYLE);
        set_style(window, GWL_EXSTYLE, ex_style & !WS_EX_LAYERED.0);
        return;
    }

    let ex_style = style(window, GWL_EXSTYLE);
    if ex_style & WS_EX_LAYERED.0 != 0 {
        // Already layered by its own application, which may be using it for
        // a colour key or an animation. Not ours to interfere with.
        return;
    }

    set_prop(window, PROP_EX_STYLE, ex_style as usize);
    set_style(window, GWL_EXSTYLE, ex_style | WS_EX_LAYERED.0);
    let _ =
        unsafe { SetLayeredWindowAttributes(window, Default::default(), FADED_ALPHA, LWA_ALPHA) };
}

/// Rolls the window up into its title bar, or unrolls it.
///
/// There is no Win32 "shade": the window is resized to the height of its
/// non-client area and its real height remembered. The application is not
/// told to lay itself out differently, so anything it draws is simply out of
/// sight until the window comes back.
pub fn shade(window: HWND) {
    let Some(bounds) = bounds(window) else { return };
    let width = bounds.right - bounds.left;

    if let Some(height) = take_prop(window, PROP_HEIGHT) {
        resize(window, width, height as i32);
        return;
    }

    let height = bounds.bottom - bounds.top;
    let rolled = title_bar_height(window);
    if height <= rolled {
        return; // already that short: nothing to roll up
    }

    set_prop(window, PROP_HEIGHT, height as usize);
    resize(window, width, rolled);
}

/// Hides the window from the taskbar, or shows it again.
///
/// The taskbar reads `WS_EX_TOOLWINDOW` once, when the window is shown, so
/// the window has to be hidden and shown again for the change to register.
pub fn hide_taskbar(window: HWND) {
    let ex_style = style(window, GWL_EXSTYLE);
    let hidden = ex_style & WS_EX_TOOLWINDOW.0 != 0;

    let _ = unsafe { ShowWindow(window, windows::Win32::UI::WindowsAndMessaging::SW_HIDE) };
    set_style(
        window,
        GWL_EXSTYLE,
        if hidden {
            ex_style & !WS_EX_TOOLWINDOW.0
        } else {
            ex_style | WS_EX_TOOLWINDOW.0
        },
    );
    let _ = unsafe { ShowWindow(window, SW_RESTORE) };
}

/// Moves the pointer to the middle of the window — for finding it again on a
/// wide desktop, and for putting the pointer where a dialog's buttons are.
pub fn center_cursor(window: HWND) {
    let Some(bounds) = bounds(window) else { return };
    let centre = POINT {
        x: bounds.left + (bounds.right - bounds.left) / 2,
        y: bounds.top + (bounds.bottom - bounds.top) / 2,
    };
    let _ = unsafe { SetCursorPos(centre.x, centre.y) };
}

// State queries, for the overlay: which tiles should read as "on".
//
// Each mirrors the exact condition its toggle branches on, so the two can
// never disagree about what "on" means. The ones the toggles do not use
// themselves have no caller until the overlay exists.

pub fn is_maximized(window: HWND) -> bool {
    style(window, GWL_STYLE) & WS_MAXIMIZE.0 != 0
}

pub fn is_minimized(window: HWND) -> bool {
    style(window, GWL_STYLE) & WS_MINIMIZE.0 != 0
}

pub fn is_always_on_top(window: HWND) -> bool {
    style(window, GWL_EXSTYLE) & WS_EX_TOPMOST.0 != 0
}

pub fn is_borderless(window: HWND) -> bool {
    has_prop(window, PROP_STYLE)
}

pub fn is_faded(window: HWND) -> bool {
    has_prop(window, PROP_EX_STYLE)
}

pub fn is_shaded(window: HWND) -> bool {
    has_prop(window, PROP_HEIGHT)
}

pub fn is_hidden_from_taskbar(window: HWND) -> bool {
    style(window, GWL_EXSTYLE) & WS_EX_TOOLWINDOW.0 != 0
}

/// Sends the window to `target`, keeping its place relative to the work area.
///
/// A minimized window is left alone. A maximized one stays maximized and
/// un-maximizes on `target` (see [`move_maximized`]).
pub fn move_to_monitor(window: HWND, target: &Monitor) {
    if is_minimized(window) {
        return;
    }
    let source = Monitor::from_window(window);
    if source.handle == target.handle {
        return;
    }
    if is_maximized(window) {
        return move_maximized(window, &source, target);
    }
    let Some(current) = bounds(window) else {
        return;
    };

    let arranged = unsafe { IsWindowArranged(window) }.as_bool();
    let wanted = placed_rect(current, source.work_area, target.work_area, arranged);
    set_bounds(window, wanted);

    // The window may have settled on another size (a DPI change makes it
    // resize itself): one more pass keeps what it ended up with inside.
    if let Some(settled) = bounds(window) {
        let inside = placed_rect(settled, target.work_area, target.work_area, false);
        if !same(settled, inside) {
            set_bounds(window, inside);
        }
    }
}

/// Takes a maximized window to `target` without ever leaving the maximized
/// state, so there is no restore/maximize animation (candidate D of the
/// multi-monitor plan's Phase 0; B and C landed right but animated).
///
/// Two steps: `SetWindowPos` carries the window, frame overhang and all, to the
/// same place on the target's work area; then a placement call that asks for
/// maximized again records where the window restores to, moved onto the target
/// the way [`placed_rect`] moves any window. Without the second step it would
/// un-maximize back on the old monitor.
fn move_maximized(window: HWND, source: &Monitor, target: &Monitor) {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(window, &mut placement) }.is_err() {
        return;
    }
    let Some(rect) = bounds(window) else {
        return;
    };

    let (from, to) = (source.work_area, target.work_area);
    set_bounds(
        window,
        RECT {
            left: to.left + (rect.left - from.left),
            top: to.top + (rect.top - from.top),
            right: to.right + (rect.right - from.right),
            bottom: to.bottom + (rect.bottom - from.bottom),
        },
    );

    // `rcNormalPosition` is in workspace coordinates — relative to the
    // primary monitor's work area, so a taskbar on the top or left shifts the
    // origin — except for tool windows, which `hide-taskbar` makes: those use
    // screen coordinates.
    let (dx, dy) = if is_hidden_from_taskbar(window) {
        (0, 0)
    } else {
        let primary = Monitor::primary();
        (
            primary.work_area.left - primary.bounds.left,
            primary.work_area.top - primary.bounds.top,
        )
    };
    let normal = placement.rcNormalPosition;
    let screen = RECT {
        left: normal.left + dx,
        top: normal.top + dy,
        right: normal.right + dx,
        bottom: normal.bottom + dy,
    };
    let moved = placed_rect(screen, from, to, false);
    placement.rcNormalPosition = RECT {
        left: moved.left - dx,
        top: moved.top - dy,
        right: moved.right - dx,
        bottom: moved.bottom - dy,
    };
    placement.showCmd = SW_SHOWMAXIMIZED.0 as u32;
    let _ = unsafe { SetWindowPlacement(window, &placement) };
}

/// Where `rect`, placed in the work area `from`, belongs in the work area `to`.
///
/// The position scales with the work areas and the size stays, so a window
/// keeps its place on screen without growing or shrinking. An `arranged`
/// (snapped) window scales its size as well: a left half stays a left half.
/// Either way the result is clamped inside `to`, shrinking if it has to.
fn placed_rect(rect: RECT, from: RECT, to: RECT, arranged: bool) -> RECT {
    let scale = |value: i64, from_len: i64, to_len: i64| {
        if from_len <= 0 {
            value
        } else {
            value * to_len / from_len
        }
    };
    let (from_width, from_height) = (width(from) as i64, height(from) as i64);
    let (to_width, to_height) = (width(to) as i64, height(to) as i64);

    let (mut w, mut h) = (width(rect) as i64, height(rect) as i64);
    if arranged {
        w = scale(w, from_width, to_width);
        h = scale(h, from_height, to_height);
    }
    w = w.min(to_width);
    h = h.min(to_height);

    let left = to.left as i64 + scale((rect.left - from.left) as i64, from_width, to_width);
    let top = to.top as i64 + scale((rect.top - from.top) as i64, from_height, to_height);
    let left = left.min(to.right as i64 - w).max(to.left as i64) as i32;
    let top = top.min(to.bottom as i64 - h).max(to.top as i64) as i32;

    RECT {
        left,
        top,
        right: left + w as i32,
        bottom: top + h as i32,
    }
}

fn width(rect: RECT) -> i32 {
    rect.right - rect.left
}

fn height(rect: RECT) -> i32 {
    rect.bottom - rect.top
}

fn same(a: RECT, b: RECT) -> bool {
    (a.left, a.top, a.right, a.bottom) == (b.left, b.top, b.right, b.bottom)
}

fn set_bounds(window: HWND, rect: RECT) {
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            rect.left,
            rect.top,
            width(rect),
            height(rect),
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

fn bounds(window: HWND) -> Option<RECT> {
    let mut bounds = RECT::default();
    unsafe { GetWindowRect(window, &mut bounds) }.ok()?;
    Some(bounds)
}

fn resize(window: HWND, width: i32, height: i32) {
    let _ = unsafe {
        SetWindowPos(
            window,
            None,
            0,
            0,
            width,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

/// The height a rolled-up window keeps: its title bar, or a usable minimum
/// for a window that has no frame left to grab.
fn title_bar_height(window: HWND) -> i32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CYCAPTION, SM_CYSIZEFRAME};

    let caption = unsafe { GetSystemMetrics(SM_CYCAPTION) };
    let frame = unsafe { GetSystemMetrics(SM_CYSIZEFRAME) };
    let has_caption = style(window, GWL_STYLE) & WS_CAPTION.0 != 0;

    if has_caption {
        (caption + frame * 2).max(8)
    } else {
        (frame * 2).max(8)
    }
}

// `GetWindowLongPtrW` only exists on 64-bit; on 32-bit the pointer-sized call
// *is* `GetWindowLongW`. Both targets are plausible for a tool this small, so
// pick the right one here instead of pinning the crate to x64.

#[cfg(target_pointer_width = "64")]
fn style(
    window: HWND,
    index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
    unsafe { GetWindowLongPtrW(window, index) as u32 }
}

#[cfg(not(target_pointer_width = "64"))]
fn style(
    window: HWND,
    index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowLongW;
    unsafe { GetWindowLongW(window, index) as u32 }
}

#[cfg(target_pointer_width = "64")]
fn set_style(
    window: HWND,
    index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
    value: u32,
) {
    use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;
    unsafe { SetWindowLongPtrW(window, index, value as isize) };
}

#[cfg(not(target_pointer_width = "64"))]
fn set_style(
    window: HWND,
    index: windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX,
    value: u32,
) {
    use windows::Win32::UI::WindowsAndMessaging::SetWindowLongW;
    unsafe { SetWindowLongW(window, index, value as i32) };
}

/// Remembers a value on the window. Zero is never stored, because a property
/// that is absent and a property worth zero read the same.
fn set_prop(window: HWND, name: PCWSTR, value: usize) {
    let _ = unsafe { SetPropW(window, name, Some(HANDLE(value.max(1) as *mut _))) };
}

fn has_prop(window: HWND, name: PCWSTR) -> bool {
    !unsafe { GetPropW(window, name) }.is_invalid()
}

/// Reads a remembered value and forgets it, so a toggle reads "was it us who
/// changed this, and to what" in one step.
fn take_prop(window: HWND, name: PCWSTR) -> Option<usize> {
    let value = unsafe { GetPropW(window, name) };
    if value.is_invalid() {
        return None;
    }

    let _ = unsafe { RemovePropW(window, name) };
    Some(value.0 as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn parts(r: RECT) -> (i32, i32, i32, i32) {
        (r.left, r.top, r.right, r.bottom)
    }

    #[test]
    fn a_window_keeps_its_size_and_relative_position() {
        let from = rect(0, 0, 1000, 800);
        let to = rect(1000, 0, 3000, 1600);
        let placed = placed_rect(rect(100, 80, 500, 480), from, to, false);
        assert_eq!(parts(placed), (1200, 160, 1600, 560));
    }

    #[test]
    fn negative_coordinates_are_relative_to_the_work_area() {
        let from = rect(-1920, 0, 0, 1080);
        let to = rect(0, 0, 1920, 1080);
        let placed = placed_rect(rect(-1820, 100, -1020, 700), from, to, false);
        assert_eq!(parts(placed), (100, 100, 900, 700));
    }

    #[test]
    fn a_smaller_target_shrinks_and_clamps_the_window() {
        let from = rect(0, 0, 2000, 1200);
        let to = rect(2000, 0, 3000, 600);
        let placed = placed_rect(rect(1000, 600, 1900, 1150), from, to, false);
        assert_eq!(parts(placed), (2100, 50, 3000, 600));
        let placed = placed_rect(rect(100, 100, 1900, 1100), from, to, false);
        assert_eq!(parts(placed), (2000, 0, 3000, 600));
    }

    #[test]
    fn a_stacked_layout_scales_vertically() {
        let from = rect(0, 0, 1000, 1000);
        let to = rect(0, -500, 1000, 0);
        let placed = placed_rect(rect(0, 500, 400, 700), from, to, false);
        assert_eq!(parts(placed), (0, -250, 400, -50));
    }

    #[test]
    fn a_snapped_half_stays_a_half() {
        let from = rect(0, 0, 1000, 800);
        let to = rect(1000, 0, 3000, 1600);
        let left_half = placed_rect(rect(0, 0, 500, 800), from, to, true);
        assert_eq!(parts(left_half), (1000, 0, 2000, 1600));
        let right_half = placed_rect(rect(500, 0, 1000, 800), from, to, true);
        assert_eq!(parts(right_half), (2000, 0, 3000, 1600));
    }

    #[test]
    fn a_degenerate_source_does_not_divide_by_zero() {
        let placed = placed_rect(
            rect(0, 0, 100, 100),
            rect(0, 0, 0, 0),
            rect(0, 0, 500, 500),
            true,
        );
        assert_eq!(parts(placed), (0, 0, 100, 100));
    }
}
