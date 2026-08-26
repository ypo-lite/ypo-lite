//! Small rectangle/point helpers shared by [`super::gesture`] and
//! [`super::hover`] — neither concern owns these, both need them.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::UI::HiDpi::{GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_NCHITTEST,
};
use windows::core::Result;

pub fn window_rect(window: HWND) -> Result<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }?;
    Ok(rect)
}

/// `window`'s actual visual bounds — what a user would call its edge —
/// as opposed to [`window_rect`]'s raw `GetWindowRect`. Since Windows 10,
/// DWM gives ordinary top-level windows several pixels of invisible resize
/// border on the left, right and bottom that `GetWindowRect` counts as part
/// of the window; anything computed from it — like
/// [`super::hover::min_button_rect`]'s button estimate — ends up shifted by
/// that margin and can miss the real button by more than its own width.
/// `DWMWA_EXTENDED_FRAME_BOUNDS` is the border DWM actually paints and hit-
/// tests against, which is the one worth measuring from.
///
/// Falls back to [`window_rect`] if the DWM query fails — composition off,
/// or a window with no non-client frame to speak of. Safe to call from
/// inside the mouse hook: unlike `SendMessage` to the target window's own
/// message queue (what [`super`]'s module doc comment already rules out),
/// this asks DWM directly, and `dwm.exe` — unlike an arbitrary target
/// application — is never legitimately busy long enough to risk the hook's
/// timeout.
pub fn visual_window_rect(window: HWND) -> Result<RECT> {
    let mut rect = RECT::default();
    let ok = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    }
    .is_ok();

    if ok { Ok(rect) } else { window_rect(window) }
}

/// How long [`hit_test`] waits for the target window's own thread to answer
/// before giving up on it. Short enough that a genuinely hung window costs
/// this thread's message loop only a moment, not a hang of its own.
const HIT_TEST_TIMEOUT_MS: u32 = 150;

/// A bounded `WM_NCHITTEST` against `window`, run in *its* DPI-awareness
/// context rather than this process's own — otherwise `pt` (physical screen
/// coordinates) is interpreted at the wrong scale for a window whose DPI
/// awareness differs from y-snap's. `None` means the target did not answer
/// in time (treated as "hung", not as any particular answer) or the call
/// failed outright; a real hit-test result, including negative ones like
/// `HTTRANSPARENT`, comes back as-is.
///
/// Only ever call this off the hook thread's hot path — see
/// [`super::hover::confirm_min_button`], its one caller. `SendMessage`,
/// bounded or not, into another process's window procedure is exactly what
/// [`super::MouseHook`]'s own module doc comment rules out doing from
/// inside the hook callback itself; [`SendMessageTimeoutW`] only bounds how
/// long a hung target can hold up *this* call, not the hook's own 300ms
/// budget, which nothing at all is allowed to spend on a `SendMessage`.
pub fn hit_test(window: HWND, pt: POINT) -> Option<i32> {
    let target_context = unsafe { GetWindowDpiAwarenessContext(window) };
    let previous_context = unsafe { SetThreadDpiAwarenessContext(target_context) };

    let lparam = LPARAM((((pt.y as u32) << 16) | (pt.x as u32 & 0xFFFF)) as i32 as isize);
    let mut result = 0usize;
    let answered = unsafe {
        SendMessageTimeoutW(
            window,
            WM_NCHITTEST,
            WPARAM(0),
            lparam,
            SMTO_ABORTIFHUNG,
            HIT_TEST_TIMEOUT_MS,
            Some(&mut result),
        )
    };

    unsafe { SetThreadDpiAwarenessContext(previous_context) };

    (answered.0 != 0).then_some(result as isize as i32)
}

pub fn point_in_window(window: HWND, pt: POINT) -> bool {
    window_rect(window).is_ok_and(|rect| point_in(rect, pt))
}

pub fn point_in(rect: RECT, pt: POINT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_is_inside_its_own_rectangle_but_not_past_its_edges() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 20,
            bottom: 20,
        };
        assert!(point_in(rect, POINT { x: 10, y: 10 })); // top-left is inside
        assert!(!point_in(rect, POINT { x: 20, y: 15 })); // right edge is not
        assert!(!point_in(rect, POINT { x: 15, y: 20 })); // bottom edge is not
        assert!(!point_in(rect, POINT { x: 9, y: 15 }));
    }
}
