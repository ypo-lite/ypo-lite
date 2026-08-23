//! Tracking whether the pointer is over a window's minimize button.
//!
//! This is the whole reason the minimize-button popup ([`crate::min_overlay`])
//! knows when to show or hide itself: [`Hover::update`] runs on every
//! `WM_MOUSEMOVE`, unrelated to whatever [`super::gesture`] is doing with the
//! same events — a drag or resize in progress takes over the pointer
//! entirely, so `super`'s hook dispatch only reaches here when neither is.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetWindowLongW, GetWindowRect, PostMessageW, SM_CXSIZE, SM_CYCAPTION, WS_MINIMIZEBOX,
};
use y_core::monitor::Monitor;

use crate::target;

use super::WM_MIN_HOVER;
use super::geometry::{point_in, point_in_window};

/// Which window is hovered, and which window the popup itself currently
/// occupies — the latter so the hover check does not lose the target the
/// instant the pointer reaches the popup.
pub struct Hover {
    owner: HWND,
    hovered: HWND,
    overlay: HWND,
}

impl Hover {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            hovered: HWND::default(),
            overlay: HWND::default(),
        }
    }

    /// Set from the same thread the hook runs on, so there is nothing to
    /// synchronise. Called by [`crate::min_overlay`] when it shows or hides.
    pub fn set_overlay(&mut self, hwnd: HWND) {
        self.overlay = hwnd;
    }

    /// Posts [`WM_MIN_HOVER`] when the eligible window under the pointer
    /// changes, and only then — a hover that has not changed costs nothing.
    pub fn update(&mut self, pt: POINT) {
        if !self.overlay.is_invalid() && point_in_window(self.overlay, pt) {
            return; // the pointer reached the popup; the hover has not left
        }

        let hwnd = min_button_target(pt).unwrap_or_default();
        if hwnd != self.hovered {
            self.hovered = hwnd;
            let _ = unsafe {
                PostMessageW(
                    Some(self.owner),
                    WM_MIN_HOVER,
                    WPARAM(hwnd.0 as usize),
                    LPARAM(0),
                )
            };
        }
    }
}

/// The screen rectangle of `window`'s minimize button, estimated from its
/// caption metrics — the close and maximize buttons sit to its right, at one
/// button-width each, in that order on every version of Windows this targets.
///
/// This is an estimate, not a hit test: unlike `WM_NCHITTEST`, it never has
/// to call into another process's window procedure, which the low-level hook
/// context is not a safe place to do (see [`crate::mouse`]'s module docs). It
/// costs a few pixels of accuracy on windows with a non-standard caption; the
/// popup would rather be slightly misaligned than freeze the mouse.
pub fn min_button_rect(window: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(window, &mut rect) }.ok()?;

    let dpi = Monitor::from_window(window).dpi;
    let caption = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, dpi) };
    let button = unsafe { GetSystemMetricsForDpi(SM_CXSIZE, dpi) };

    // Right to left: Close, Maximize, Minimize.
    Some(RECT {
        left: rect.right - button * 2,
        right: rect.right - button,
        top: rect.top,
        bottom: rect.top + caption,
    })
}

/// The window whose minimize button `pt` is over, if any.
fn min_button_target(pt: POINT) -> Option<HWND> {
    let window = target::at(pt)?;

    let style = unsafe { GetWindowLongW(window, GWL_STYLE) } as u32;
    if style & WS_MINIMIZEBOX.0 == 0 {
        return None;
    }

    point_in(min_button_rect(window)?, pt).then_some(window)
}
