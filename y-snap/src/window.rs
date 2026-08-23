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
    GWL_EXSTYLE, GWL_STYLE, GetPropW, GetWindowRect, HWND_NOTOPMOST, HWND_TOPMOST, LWA_ALPHA,
    PostMessageW, RemovePropW, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetCursorPos, SetLayeredWindowAttributes,
    SetPropW, SetWindowPos, ShowWindow, WM_CLOSE, WS_CAPTION, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_MAXIMIZE, WS_MINIMIZE, WS_THICKFRAME,
};
use windows::core::{PCWSTR, w};

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

#[allow(dead_code, reason = "read by the overlay, which is not written yet")]
pub fn is_minimized(window: HWND) -> bool {
    style(window, GWL_STYLE) & WS_MINIMIZE.0 != 0
}

pub fn is_always_on_top(window: HWND) -> bool {
    style(window, GWL_EXSTYLE) & WS_EX_TOPMOST.0 != 0
}

#[allow(dead_code, reason = "read by the overlay, which is not written yet")]
pub fn is_borderless(window: HWND) -> bool {
    has_prop(window, PROP_STYLE)
}

#[allow(dead_code, reason = "read by the overlay, which is not written yet")]
pub fn is_faded(window: HWND) -> bool {
    has_prop(window, PROP_EX_STYLE)
}

#[allow(dead_code, reason = "read by the overlay, which is not written yet")]
pub fn is_shaded(window: HWND) -> bool {
    has_prop(window, PROP_HEIGHT)
}

#[allow(dead_code, reason = "read by the overlay, which is not written yet")]
pub fn is_hidden_from_taskbar(window: HWND) -> bool {
    style(window, GWL_EXSTYLE) & WS_EX_TOOLWINDOW.0 != 0
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
