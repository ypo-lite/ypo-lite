//! Item tooltips for the native popup (`tooltip="..."` on a node).
//!
//! `TrackPopupMenu` owns the message loop and an owner-drawn item has no
//! window of its own, so there is no tooltip control to attach. Instead
//! `main` forwards `WM_MENUSELECT` here: highlighting a row with a tooltip
//! arms a one-shot timer on the owner window (the nested menu loop still
//! dispatches `WM_TIMER`), and when it fires a small [`Overlay`] appears just
//! under the row. Moving the highlight, or closing the menu, hides it again.
//!
//! The overlay is non-activating and click-through, so it can't steal the
//! menu's capture or swallow a click meant for the row beneath it. Its size
//! depends on its text, and a layered window's surface is fixed at creation,
//! so each showing creates a fresh overlay — cheap next to a half-second
//! hover delay.
//!
//! Simplification: one line, no wrapping. A long tooltip gets a wide box.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use y_core::menu::MenuDesign;
use y_core::monitor::Monitor;
use y_core::overlay::{Color, Font, Overlay, OverlaySpec, TextAlign};

use crate::menu::Tip;

/// Timer id on the owner window. Any value will do; the menu loop's own
/// timers are not delivered to us.
const TIMER_ID: usize = 0x7001;

/// How long a row must stay highlighted before its tooltip appears — about
/// what the system's own tooltips wait.
const DELAY_MS: u32 = 500;

const FONT_FAMILY: &str = "Segoe UI";
/// All at 100% scaling; converted per monitor when shown.
const FONT_HEIGHT: i32 = 13;
const PAD_X: i32 = 8;
const PAD_Y: i32 = 5;
const GAP_BELOW_ROW: i32 = 2;
const CORNER_RADIUS: i32 = 4;

/// Used when the popup has no `MenuDesign` (`theme="native"`): the classic
/// pale-yellow tooltip.
const NATIVE_BACK: Color = Color::rgb(0xFF, 0xFF, 0xE1);
const NATIVE_TEXT: Color = Color::BLACK;
const NATIVE_BORDER: Color = Color::rgb(0x76, 0x76, 0x76);

/// `Some` between `WM_MENUSELECT` and the timer firing.
struct Waiting {
    tip: Tip,
    design: Option<MenuDesign>,
}

#[derive(Default)]
pub struct Tooltip {
    waiting: Option<Waiting>,
    shown: Option<Overlay>,
}

impl Tooltip {
    /// The highlight moved: forget whatever was showing or pending, and, if
    /// the new row has a tooltip, start waiting to show it.
    pub fn highlight(&mut self, owner: HWND, tip: Option<Tip>, design: Option<MenuDesign>) {
        self.hide(owner);
        if let Some(tip) = tip {
            self.waiting = Some(Waiting { tip, design });
            // Re-arming an already-set timer restarts it, so a stale one
            // can't fire for the row before this one.
            unsafe { SetTimer(Some(owner), TIMER_ID, DELAY_MS, None) };
        }
    }

    /// Hides the tooltip and cancels any pending one.
    pub fn hide(&mut self, owner: HWND) {
        if self.waiting.take().is_some() {
            let _ = unsafe { KillTimer(Some(owner), TIMER_ID) };
        }
        // Dropping the overlay destroys its window.
        self.shown = None;
    }

    /// `WM_TIMER`: shows the pending tooltip if `timer` is ours. Returns
    /// whether it was, so the caller knows the message is handled.
    pub fn on_timer(&mut self, owner: HWND, timer: usize) -> bool {
        if timer != TIMER_ID {
            return false;
        }
        // One-shot.
        let _ = unsafe { KillTimer(Some(owner), TIMER_ID) };

        if let Some(waiting) = self.waiting.take() {
            self.shown = show(owner, &waiting.tip, waiting.design).ok();
        }
        true
    }
}

fn show(owner: HWND, tip: &Tip, design: Option<MenuDesign>) -> y_core::Result<Overlay> {
    let (back, text, border) = match design {
        Some(design) => (design.base.back, design.base.text, design.border.color),
        None => (NATIVE_BACK, NATIVE_TEXT, NATIVE_BORDER),
    };

    // Below the row, indented a little so it reads as hanging off the label
    // rather than the menu's edge.
    let anchor = POINT {
        x: tip.row.left,
        y: tip.row.bottom,
    };
    let monitor = Monitor::from_point(anchor);

    let font = Font::new(FONT_FAMILY, monitor.scaled(FONT_HEIGHT), false)?;
    let (text_width, text_height) = font.measure(&tip.text);
    let (pad_x, pad_y) = (monitor.scaled(PAD_X), monitor.scaled(PAD_Y));
    let size = (text_width + 2 * pad_x, text_height + 2 * pad_y);

    let mut overlay =
        Overlay::new(&OverlaySpec::new("YpoLite.YPop.Tooltip", size).with_owner(owner))?;
    let bounds = overlay.bounds();
    let canvas = overlay.canvas_mut();
    canvas.clear(back);
    canvas.stroke_rect(bounds, border);
    canvas.draw_text(
        &tip.text,
        RECT {
            left: pad_x,
            top: pad_y,
            right: size.0 - pad_x,
            bottom: size.1 - pad_y,
        },
        &font,
        text,
        TextAlign::Left,
    )?;
    canvas.round_corners(monitor.scaled(CORNER_RADIUS));

    let position = monitor.keep_inside(
        POINT {
            x: anchor.x + monitor.scaled(PAD_X * 2),
            y: anchor.y + monitor.scaled(GAP_BELOW_ROW),
        },
        size,
    );
    overlay.show_at(position)?;
    Ok(overlay)
}
