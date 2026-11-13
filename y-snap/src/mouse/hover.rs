//! Tracking whether the pointer is over a window's minimize button, and the
//! two things that can happen there: the quick popup
//! ([`crate::min_overlay`]) after it sits still a while, or a right-click's
//! configured action right away.
//!
//! [`Hover::update`] runs on every `WM_MOUSEMOVE`, unrelated to whatever
//! [`super::gesture`] is doing with the same events — a drag or resize in
//! progress takes over the pointer entirely, so `super`'s hook dispatch only
//! reaches here when neither is. [`Hover::on_right_down`]/[`on_right_up`]
//! sit alongside `gesture`'s own button handling instead, since a plain
//! right-click has nothing to do with drag or resize.
//!
//! # Why the dwell delay is a `WM_TIMER`, not a `WM_MOUSEMOVE` count
//!
//! "Show after ~600ms–1s of *inactivity*" means the delay has to restart on
//! every move still inside the button, not just run once from the moment
//! the pointer arrives — otherwise the popup would appear while the pointer
//! is still travelling across the button on its way to maximize or close
//! next to it. But once the pointer actually stops, nothing calls
//! [`Hover::update`] again — there is no more `WM_MOUSEMOVE` to react to —
//! so only a real timer can notice "still here, and it's been long enough"
//! on its own. [`Hover::update`] only ever arms or resets
//! [`super::MIN_HOVER_TIMER_ID`] (via [`SetTimer`], cheap enough to call
//! from inside the hook on every qualifying move, same as this module
//! already calls [`geometry::visual_window_rect`] there); the popup itself
//! is shown from the ordinary message loop once that timer actually fires,
//! the same split [`super`]'s own module doc comment already draws for
//! drawing and for `SetWindowPos` — nothing that can wait belongs inside the
//! hook callback.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CAPTION_BUTTON_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClassNameW, GetWindowLongW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLOSE,
    HTHELP, HTHSCROLL, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTSYSMENU, HTTOP, HTTOPLEFT,
    HTTOPRIGHT, HTVSCROLL, KillTimer, PostMessageW, SM_CXSIZE, SM_CYCAPTION, SetTimer,
    WS_MINIMIZEBOX, WindowFromPoint,
};
use y_core::monitor::Monitor;

use y_core::target;

use super::geometry::{self, point_in, point_in_window};
use super::{MIN_HOVER_TIMER_ID, WM_MIN_HOVER, WM_MIN_RIGHTCLICK};

/// Which window is hovered, which window the popup itself currently
/// occupies (so the hover check does not lose the target the instant the
/// pointer reaches the popup), and the right-click-in-progress target, if
/// any.
pub struct Hover {
    owner: HWND,
    hovered: HWND,
    overlay: HWND,
    /// How long a stopped pointer has to sit still before the popup shows.
    dwell_ms: u32,
    /// Whether there is anything to dwell for at all — `<minbutton>` with no
    /// tiles arms nothing, same as [`crate::min_overlay::MinOverlay::show`]
    /// already no-ops on an empty list.
    show_popup: bool,
    /// Whether `<minbutton right-click-action="…">` (or its default) is on.
    right_click_enabled: bool,
    /// The button a right-click is currently pressed down over, if the
    /// down landed on one.
    right_click_armed: Option<HWND>,
}

impl Hover {
    pub fn new(owner: HWND, dwell_ms: u32, show_popup: bool, right_click_enabled: bool) -> Self {
        Self {
            owner,
            hovered: HWND::default(),
            overlay: HWND::default(),
            dwell_ms,
            show_popup,
            right_click_enabled,
            right_click_armed: None,
        }
    }

    /// The window currently hovered, or a null `HWND` if none is — read by
    /// [`super::take_min_hover_target`] when the dwell timer fires.
    pub fn hovered(&self) -> HWND {
        self.hovered
    }

    /// Set from the same thread the hook runs on, so there is nothing to
    /// synchronise. Called by [`crate::min_overlay`] when it shows or hides.
    pub fn set_overlay(&mut self, hwnd: HWND) {
        self.overlay = hwnd;
    }

    /// Posts [`WM_MIN_HOVER`] when the eligible window under the pointer
    /// changes, and (re)arms the dwell timer whenever the pointer is still
    /// moving inside the same one — see this module's doc comment for why
    /// showing the popup does not happen directly from here.
    pub fn update(&mut self, pt: POINT) {
        if !self.overlay.is_invalid() && point_in_window(self.overlay, pt) {
            return; // the pointer reached the popup; the hover has not left
        }

        // The application's own "Minimize" tooltip pops up just under the
        // button, in the very strip the pointer crosses to reach the popup.
        // It belongs to another process and cannot be suppressed from here
        // (unlike maximize, which has no tooltip), so it is looked through
        // instead: a tooltip under the pointer, inside the hovered window's
        // caption-button region, counts as still being on the button. Without
        // this, `target::at` refuses the tooltip, the hover is lost, and the
        // popup hides before the pointer gets to it.
        if !self.hovered.is_invalid()
            && candidate_rect(self.hovered).is_some_and(|rect| point_in(rect, pt))
            && tooltip_under(pt)
        {
            return;
        }

        let hwnd = min_button_target(pt).unwrap_or_default();
        if hwnd != self.hovered {
            self.hovered = hwnd;
            let _ = unsafe { PostMessageW(Some(self.owner), WM_MIN_HOVER, WPARAM(0), LPARAM(0)) };

            if hwnd.is_invalid() {
                let _ = unsafe { KillTimer(Some(self.owner), MIN_HOVER_TIMER_ID) };
            } else if self.show_popup {
                self.arm_dwell();
            }
        } else if !hwnd.is_invalid() && self.show_popup {
            self.arm_dwell();
        }
    }

    fn arm_dwell(&self) {
        let _ = unsafe { SetTimer(Some(self.owner), MIN_HOVER_TIMER_ID, self.dwell_ms, None) };
    }

    /// A right (or any other non-left) button just went down — claims
    /// it, swallowing the event, when it lands on a minimize button and
    /// `right-click-action` is configured on. `false` leaves it to
    /// [`super::gesture::Gesture`], the same as if this feature did not
    /// exist.
    pub fn on_right_down(&mut self, pt: POINT) -> bool {
        if !self.right_click_enabled {
            return false;
        }
        let Some(window) = min_button_target(pt) else {
            return false;
        };
        self.right_click_armed = Some(window);
        true
    }

    /// The matching button-up. Always swallows it once
    /// [`Hover::on_right_down`] claimed the down — releasing on the same
    /// button fires [`WM_MIN_RIGHTCLICK`]; releasing elsewhere just cancels,
    /// the same "click, not just press" shape
    /// [`crate::min_overlay::ClickHandler`] already uses for the popup's own
    /// tiles.
    pub fn on_right_up(&mut self, pt: POINT) -> bool {
        let Some(window) = self.right_click_armed.take() else {
            return false;
        };
        if min_button_target(pt) == Some(window) {
            let _ = unsafe {
                PostMessageW(
                    Some(self.owner),
                    WM_MIN_RIGHTCLICK,
                    WPARAM(window.0 as usize),
                    LPARAM(0),
                )
            };
        }
        true
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
///
/// Measured from [`geometry::visual_window_rect`], not a raw `GetWindowRect`
/// — the latter's few pixels of invisible DWM resize border are enough on
/// their own to shift this estimate off the real button entirely, on
/// ordinary windows, not just ones with a custom frame.
///
/// Used only to position the popup once something has already confirmed
/// the button is really there — a few pixels off is cosmetic. Hover
/// *detection* uses [`candidate_rect`] instead, deliberately looser than
/// this.
pub fn min_button_rect(window: HWND) -> Option<RECT> {
    let rect = geometry::visual_window_rect(window).ok()?;

    let dpi = Monitor::from_window(window).dpi;
    let caption = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, dpi) };
    let button = unsafe { GetSystemMetricsForDpi(SM_CXSIZE, dpi) };

    // Right to left: Close, Maximize, Minimize — so minimize is the *third*
    // button-width in, not the second (that slot belongs to maximize).
    Some(RECT {
        left: rect.right - button * 3,
        right: rect.right - button * 2,
        top: rect.top,
        bottom: rect.top + caption,
    })
}

/// A one-time, bounded check against the real window, run right before the
/// popup actually appears — confirming or rejecting the geometric guess
/// that got it this far. `min_button_rect`'s estimate assumes the OS's own
/// caption-button metrics; an application that paints its own,
/// differently-sized buttons inside a standard frame (Chrome, Opera, and
/// others) can leave that estimate pointed at the wrong button entirely —
/// this is what catches that, using [`geometry::hit_test`], which is only
/// safe to call from here: off the hook thread's hot path, at most once per
/// popup, not from inside the hook on every move.
///
/// Errs toward trusting the geometry: `true` on a confirmed hit *or* an
/// inconclusive one (`HTCLIENT`, `HTCAPTION`, a timeout, …) — the same
/// windows that already relied on the geometric estimate alone before this
/// existed keep doing so — and `false` only when the target window is
/// confident `pt` is somewhere else, such as `HTMAXBUTTON` for a
/// custom-styled maximize button that happens to sit where the OS metrics
/// alone would have guessed minimize.
pub fn confirm_min_button(window: HWND, pt: POINT) -> bool {
    match geometry::hit_test(window, pt) {
        Some(hit) if hit == HTMINBUTTON as i32 => true,
        Some(hit) if is_clearly_elsewhere(hit) => false,
        _ => true,
    }
}

/// Hit-test results that rule a point out of being the minimize button,
/// confidently enough to override the geometric guess — everything else
/// (`HTCLIENT`, `HTCAPTION`, `HTNOWHERE`, `HTTRANSPARENT`, …) is treated as
/// "the target has nothing more precise to say" rather than a rejection;
/// Explorer, for one, answers `HTCLIENT` for its whole tabbed title bar.
fn is_clearly_elsewhere(hit: i32) -> bool {
    const ELSEWHERE: &[u32] = &[
        HTCLOSE,
        HTMAXBUTTON,
        HTHELP,
        HTSYSMENU,
        HTHSCROLL,
        HTVSCROLL,
        HTBOTTOM,
        HTBOTTOMLEFT,
        HTBOTTOMRIGHT,
        HTLEFT,
        HTRIGHT,
        HTTOP,
        HTTOPLEFT,
        HTTOPRIGHT,
    ];
    hit >= 0 && ELSEWHERE.contains(&(hit as u32))
}

/// The minimize button's real rectangle, as the window itself reports it:
/// `pt` is known to be on the button (the pointer has dwelt there), so this
/// bisects outward from it with bounded `WM_NCHITTEST`s until the answer stops
/// being `HTMINBUTTON` — down, up, left and right. That is how Windows itself
/// learns where a caption button is (the snap-layouts flyout of maximize asks
/// the window the same question), and it works for windows that draw their
/// own title bar (Windows Terminal's tabs, Chrome) where no metric or DWM
/// query gets the button's size right. ~20 probes, run once per popup from the
/// message loop, never from the hook. `None` when `pt` is not confirmed on the
/// button (or the answer is implausible): the caller falls back to estimates.
pub fn probe_min_button(window: HWND, pt: POINT) -> Option<RECT> {
    let is_min =
        |x: i32, y: i32| geometry::hit_test(window, POINT { x, y }) == Some(HTMINBUTTON as i32);
    if !is_min(pt.x, pt.y) {
        return None;
    }

    let frame = geometry::visual_window_rect(window).ok()?;
    let dpi = Monitor::from_window(window).dpi;
    let reach = unsafe { GetSystemMetricsForDpi(SM_CXSIZE, dpi) } * 3;

    // The first coordinate past the button going from `from` towards `to`,
    // assuming the button is one contiguous run along that line.
    let edge = |from: i32, to: i32, on_line: &dyn Fn(i32) -> bool| -> Option<i32> {
        if on_line(to) {
            return None; // still on the button at the search limit: nonsense
        }
        let (mut inside, mut outside) = (from, to);
        while (outside - inside).abs() > 1 {
            let mid = inside + (outside - inside) / 2;
            if on_line(mid) {
                inside = mid;
            } else {
                outside = mid;
            }
        }
        Some(inside) // the last coordinate still on the button
    };

    let bottom = edge(pt.y, pt.y + reach, &|y| is_min(pt.x, y))?;
    let top = edge(pt.y, (pt.y - reach).max(frame.top), &|y| is_min(pt.x, y)).unwrap_or(frame.top);
    let left = edge(pt.x, pt.x - reach, &|x| is_min(x, pt.y))?;
    let right = edge(pt.x, pt.x + reach, &|x| is_min(x, pt.y))?;

    Some(RECT {
        left,
        top,
        right: right + 1,
        bottom: bottom + 1,
    })
}

/// A generous candidate region covering the whole cluster of caption
/// buttons (minimize, maximize, close) — not [`min_button_rect`]'s own
/// narrow guess at minimize's exact slot.
///
/// Windows 10 and 11's own theme draws these noticeably larger than the
/// classic `SM_CXSIZE`/`SM_CYCAPTION` metrics describe — this is the same
/// theme change the reference implementation works around with
/// `WM_NCHITTEST`/`TITLEBARINFOEX`. Trying to out-guess the exact size
/// geometrically left this only overlapping the real button's top-right
/// corner: close enough to *sometimes* land inside it, never enough to
/// cover it. This gives up on precision instead — it only has to contain
/// the real button somewhere inside it, generously, since
/// [`confirm_min_button`] is what actually decides which button, if any,
/// was hit, not this.
fn candidate_rect(window: HWND) -> Option<RECT> {
    let rect = geometry::visual_window_rect(window).ok()?;

    let dpi = Monitor::from_window(window).dpi;
    let caption = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, dpi) };
    let button = unsafe { GetSystemMetricsForDpi(SM_CXSIZE, dpi) };

    Some(RECT {
        left: rect.right - button * 4,
        right: rect.right,
        top: rect.top,
        // Never shorter than the popup's anchor, so the region always touches
        // the popup however tall the title bar is.
        bottom: (rect.top + caption * CANDIDATE_HEIGHT_FACTOR)
            .max(min_popup_top(window).unwrap_or_default()),
    })
}

/// Height of [`candidate_rect`], in multiples of `SM_CYCAPTION`. The real
/// caption buttons are taller than that metric says (Win11 draws ~31 px for a
/// 23 px metric, and Chrome's tall title bar more still), so the popup is
/// anchored at this region's bottom edge, not the estimated button's: below
/// the whole button, where it cannot cover its lower part. The region still
/// touches the popup, so the pointer travelling down never leaves the hover.
const CANDIDATE_HEIGHT_FACTOR: i32 = 2;

/// Screen y just under the caption-button area of `window` — where the
/// minimize popup's top edge goes. See [`CANDIDATE_HEIGHT_FACTOR`].
pub fn min_popup_top(window: HWND) -> Option<i32> {
    let rect = geometry::visual_window_rect(window).ok()?;
    let dpi = Monitor::from_window(window).dpi;
    let caption = unsafe { GetSystemMetricsForDpi(SM_CYCAPTION, dpi) };
    // Preferred: ask DWM where the caption buttons really end. The title bar
    // height differs between restored and maximized windows (Notepad++, for
    // one), and no system metric tells which; DWM measures the actual
    // buttons. Same safety as `visual_window_rect`: dwm.exe, not the target.
    // Fallback: 1.5 captions, just under Win11's ~1.35-caption button.
    Some(caption_buttons_bottom(window, caption).unwrap_or(rect.top + caption * 3 / 2))
}

/// Screen y of the bottom edge of `window`'s caption buttons, from
/// `DWMWA_CAPTION_BUTTON_BOUNDS` (relative to the `GetWindowRect` origin).
/// `None` when DWM has no answer or an implausible one (zero, or taller than
/// five captions — a custom frame reporting something unrelated).
fn caption_buttons_bottom(window: HWND, caption: i32) -> Option<i32> {
    let mut bounds = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CAPTION_BUTTON_BOUNDS,
            &mut bounds as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    if bounds.bottom <= 0 || bounds.bottom > caption * 5 {
        return None;
    }
    Some(geometry::window_rect(window).ok()?.top + bounds.bottom)
}

/// Whether the window under `pt` is a tooltip (`tooltips_class32`, the class
/// of the classic and the themed caption-button tooltips alike). Two cheap
/// calls that never leave this thread's reach into another window procedure —
/// safe in the hook, like the rest of [`Hover::update`].
fn tooltip_under(pt: POINT) -> bool {
    let window = unsafe { WindowFromPoint(pt) };
    if window.is_invalid() {
        return false;
    }
    let mut buffer = [0u16; 32];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    if length <= 0 {
        return false;
    }
    // XAML applications (Windows Terminal, the Settings app...) draw their
    // tooltips in a windowed popup of their own, not a `tooltips_class32`.
    matches!(
        String::from_utf16_lossy(&buffer[..length as usize]).as_str(),
        "tooltips_class32" | "Xaml_WindowedPopupClass"
    )
}

/// The window whose minimize button `pt` is over, if any.
fn min_button_target(pt: POINT) -> Option<HWND> {
    let window = target::at(pt)?;

    let style = unsafe { GetWindowLongW(window, GWL_STYLE) } as u32;
    if style & WS_MINIMIZEBOX.0 == 0 {
        return None;
    }

    point_in(candidate_rect(window)?, pt).then_some(window)
}
