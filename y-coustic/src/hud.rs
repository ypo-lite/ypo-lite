//! The HUD: a small bar that says where the volume went.
//!
//! One non-activating, click-through layered window ([`Overlay`]), shown on the
//! monitor under the cursor and hidden by a timer that every change restarts,
//! so a burst of key presses keeps one window, not one per press.
//!
//! The window's surface is fixed at creation, and its size depends on the
//! monitor's DPI, so the overlay is rebuilt when the cursor's monitor has a
//! different DPI from the last one shown on: rare, and still never more than
//! one window.
//!
//! VulCan showed a balloon notification here; it is dropped. A balloon is the
//! shell's, slow to appear, queued behind its predecessors and gone from the
//! screen when Focus Assist is on: it cannot follow a key held down.
//!
//! Draw order is fill, then text, then the vector shapes, with the corners
//! rounded last: GDI text zeroes alpha wherever it touches, and the corner
//! cut has to see the finished image.

use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, KillTimer, SetTimer};
use windows::core::Result;

use y_core::monitor::Monitor;
use y_core::overlay::{Font, Overlay, OverlaySpec};

use y_coustic::config::{self, Style};
use y_coustic::model::{self, Action, Change, Kind, State};

use crate::mixer::RowId;
use y_coustic::look::{AppRef, Look};

mod forward;

use forward::Forwarder;
pub use forward::{WM_TRAY_WHEEL, WheelHook, is_message, take_tray_wheel};
use y_coustic::paint::layout::{FONT_HEIGHT, Geometry, Hit, against, design_size, follow, place};
use y_coustic::paint::{Skin, draw};

/// The `wParam` of the `WM_TIMER` that hides the HUD.
pub const TIMER_ID: usize = 0xC0_0001;

/// The `wParam` of the `WM_TIMER` that steps a fade; alive only during one.
pub const FADE_TIMER_ID: usize = 0xC0_0005;

/// Milliseconds between two steps of a fade: about the timer's resolution.
const FADE_STEP_MS: u32 = 16;

const CLASS: &str = "YpoLite.YCoustic.Hud";

/// What the HUD shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub kind: Kind,
    /// `None` when there is no such device (no microphone, no output).
    pub state: Option<State>,
    /// For [`Kind::App`]: which application.
    pub app: Option<AppRef>,
    /// Which line of the mixer this is, for a click or the wheel on the HUD to
    /// act on: the master, the microphone, or the application's.
    pub row: Option<RowId>,
}

impl Reading {
    pub fn new(kind: Kind, state: State) -> Self {
        Self {
            kind,
            state: Some(state),
            app: None,
            row: Self::row_of(kind),
        }
    }

    pub fn missing(kind: Kind) -> Self {
        Self {
            kind,
            state: None,
            app: None,
            row: Self::row_of(kind),
        }
    }

    /// An application's volume.
    pub fn application(row: RowId, app: AppRef, state: State) -> Self {
        Self {
            kind: Kind::App,
            state: Some(state),
            app: Some(app),
            row: Some(row),
        }
    }

    fn row_of(kind: Kind) -> Option<RowId> {
        match kind {
            Kind::Master => Some(RowId::Master),
            Kind::Mic => Some(RowId::Mic),
            Kind::App => None,
        }
    }
}

/// A fade under way: the opacity it started from and goes to, and since when.
#[derive(Debug, Clone, Copy)]
struct Fade {
    from: u8,
    to: u8,
    started: Instant,
}

/// The opacity `elapsed` milliseconds into a fade of `duration`: a straight line
/// from `from` to `to`.
fn fade_alpha(from: u8, to: u8, elapsed: u32, duration: u32) -> u8 {
    if duration == 0 || elapsed >= duration {
        return to;
    }
    let (from, to) = (i64::from(from), i64::from(to));
    (from + (to - from) * i64::from(elapsed) / i64::from(duration)) as u8
}

/// Everything tied to one DPI and look: the window and the font drawn into it.
struct Slot {
    dpi: u32,
    skin: Skin,
    /// The monitor it was drawn for, to draw again in place.
    monitor: Monitor,
    overlay: Overlay,
    font: Font,
}

/// What the pointer on the HUD asks for. The owner carries it out (the HUD never
/// touches audio) and tells the HUD how it ended ([`Hud::redraw`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Apply `change` to a row of the mixer.
    Change { row: RowId, change: Change },
    /// A click on the HUD that is neither the glyph nor the bar: the panel.
    OpenPanel,
}

pub struct Hud {
    owner: HWND,
    slot: Option<Slot>,
    /// The application last shown, with its icon and name.
    look: Option<Look>,
    /// The timeout of the last showing, to restart the timer with.
    timeout: u32,
    /// Shown for the pointer resting on the tray icon: it stays, whatever the
    /// timeout, until the pointer leaves ([`Hud::release`]).
    holding: bool,
    /// What is on screen, so a click knows what it is a click on.
    current: Option<Reading>,
    visible: bool,
    /// The pointer is on the HUD: its timer is paused.
    hovering: bool,
    /// A bar is being dragged, and the last percentage sent for it.
    drag: Option<u8>,
    /// A click is down on neither glyph nor bar: it opens the panel on release.
    pending_panel: bool,
    /// The scoped wheel hook, while an interactive HUD is on screen.
    wheel: Option<WheelHook>,
    /// How long a fade takes, from the settings; `0` for none.
    fade_ms: u32,
    /// The fade under way, if any.
    fade: Option<Fade>,
}

impl Hud {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            slot: None,
            look: None,
            timeout: 0,
            holding: false,
            current: None,
            visible: false,
            hovering: false,
            drag: None,
            pending_panel: false,
            wheel: None,
            fade_ms: 0,
            fade: None,
        }
    }

    /// Draws `reading`, shows the HUD where `overlay` says and restarts the
    /// timer. Failures are not reported: a HUD that cannot appear must not
    /// get between a key press and the volume it already changed.
    pub fn show(&mut self, reading: Reading, overlay: &config::HudConfig) {
        self.holding = false;
        let _ = self.try_show(reading, overlay, None);
    }

    /// [`show`](Self::show), placed against `anchor` (the tray icon's rectangle)
    /// instead of where the settings say, so the pointer is a short move away
    /// from it; on the monitor of the anchor.
    pub fn show_against(&mut self, reading: Reading, overlay: &config::HudConfig, anchor: RECT) {
        // The pointer is on the icon: the HUD stays while it is.
        self.holding = true;
        let _ = self.try_show(reading, overlay, Some(anchor));
    }

    fn try_show(
        &mut self,
        reading: Reading,
        settings: &config::HudConfig,
        anchor: Option<RECT>,
    ) -> Result<()> {
        let monitor = match anchor {
            Some(rect) => Monitor::from_point(POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            }),
            None => Monitor::from_cursor().unwrap_or_else(|_| Monitor::primary()),
        };

        let skin = Skin::of(settings);
        if self
            .slot
            .as_ref()
            .is_none_or(|slot| slot.dpi != monitor.dpi || !slot.skin.same_window(&skin))
        {
            // The old window goes before the new one exists; with it any hold
            // the pointer had on it.
            self.slot = None;
            self.hovering = false;
            let (width, height) = design_size(skin.style);
            let size = (
                skin.px(monitor.dpi, width).max(1),
                skin.px(monitor.dpi, height).max(1),
            );
            let spec = OverlaySpec::new(CLASS, size);
            let overlay = if skin.interactive {
                Overlay::with_handler(&spec.interactive(), Box::new(Forwarder::new(self.owner)))?
            } else {
                Overlay::new(&spec)?
            };
            let font = Font::new(&skin.font, skin.px(monitor.dpi, FONT_HEIGHT), true)?;
            self.slot = Some(Slot {
                dpi: monitor.dpi,
                skin: skin.clone(),
                monitor,
                overlay,
                font,
            });
        }
        let Some(slot) = self.slot.as_mut() else {
            return Ok(());
        };
        slot.monitor = monitor;
        // What is only drawn (corners, border, opacity) follows the settings
        // without a new window.
        slot.skin = skin;
        slot.overlay.set_opacity(slot.skin.opacity);

        // The icon is drawn the size of the glyph it replaces.
        let size = Geometry::new(
            slot.skin.style,
            slot.overlay.size().0,
            slot.overlay.size().1,
            false,
            |length| slot.skin.px(monitor.dpi, length),
        )
        .glyph;
        let size = size.right - size.left;
        match reading.app.as_ref() {
            Some(app) => {
                if self
                    .look
                    .as_ref()
                    .is_none_or(|look| look.size != size || look.app != *app)
                {
                    self.look = Some(Look::resolve(app, size));
                }
            }
            None => self.look = None,
        }

        draw(
            slot.overlay.canvas_mut(),
            &slot.font,
            &slot.skin,
            monitor.dpi,
            reading.kind,
            reading.state,
            self.look.as_ref(),
        )?;
        self.current = Some(reading);

        let position = match anchor {
            Some(rect) => against(rect, slot.overlay.size(), &monitor),
            None if settings.position == config::Position::FollowCursor => {
                let mut pointer = POINT::default();
                let _ = unsafe { GetCursorPos(&mut pointer) };
                follow(pointer, slot.overlay.size(), &monitor)
            }
            None => place(
                monitor.work_area,
                slot.overlay.size(),
                settings.position,
                &monitor,
            ),
        };
        // A fade in starts from nothing, or from where a fade out had got to; a
        // fade in under way is left to finish.
        self.fade_ms = settings.fade;
        let target = slot.skin.opacity;
        let fading_in = self.fade.is_some_and(|fade| fade.to != 0);
        let from = if settings.fade == 0 {
            None
        } else if fading_in {
            Some(None)
        } else if !self.visible {
            Some(Some(0))
        } else if self.fade.is_some() {
            Some(Some(slot.overlay.opacity()))
        } else {
            None
        };
        match from {
            Some(None) => {}
            Some(Some(from)) => slot.overlay.set_opacity(from),
            None => slot.overlay.set_opacity(target),
        }
        slot.overlay.show_at(position)?;
        match from {
            Some(None) => {}
            Some(Some(from)) => {
                self.fade = Some(Fade {
                    from,
                    to: target,
                    started: Instant::now(),
                });
                unsafe { SetTimer(Some(self.owner), FADE_TIMER_ID, FADE_STEP_MS, None) };
            }
            None => {
                self.fade = None;
                let _ = unsafe { KillTimer(Some(self.owner), FADE_TIMER_ID) };
            }
        }
        self.visible = true;

        // The wheel is taken inside the rectangle of the HUD, wherever it is now.
        if settings.interactive {
            let (width, height) = slot.overlay.size();
            let rect = RECT {
                left: position.x,
                top: position.y,
                right: position.x + width,
                bottom: position.y + height,
            };
            match self.wheel.as_ref() {
                Some(hook) => hook.set_rect(rect),
                None => self.wheel = WheelHook::install(self.owner, rect).ok(),
            }
        }

        // Restarting a timer by SetTimer on the same id replaces it. With the
        // pointer on the HUD it stays paused: the pointer is using it.
        self.timeout = settings.timeout;
        if !self.hovering {
            unsafe { SetTimer(Some(self.owner), TIMER_ID, settings.timeout, None) };
        }
        Ok(())
    }

    /// The timer fired: hide, and let go of the window memory. A HUD held
    /// for the pointer on the tray icon stays, and its timer starts over.
    pub fn on_timer(&mut self) {
        if self.holding {
            unsafe { SetTimer(Some(self.owner), TIMER_ID, self.timeout.max(100), None) };
            return;
        }
        self.begin_hide();
    }

    /// Hides, fading out first when the settings ask for a fade.
    fn begin_hide(&mut self) {
        let Some(slot) = self
            .slot
            .as_ref()
            .filter(|_| self.fade_ms > 0 && self.visible)
        else {
            self.hide();
            return;
        };
        if self.fade.is_some_and(|fade| fade.to == 0) {
            return;
        }
        let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
        self.fade = Some(Fade {
            from: slot.overlay.opacity(),
            to: 0,
            started: Instant::now(),
        });
        unsafe { SetTimer(Some(self.owner), FADE_TIMER_ID, FADE_STEP_MS, None) };
    }

    /// A step of the fade: the opacity for how far it has got, and the end of a
    /// fade out is the end of the HUD.
    pub fn on_fade_timer(&mut self) {
        let Some(fade) = self.fade else {
            let _ = unsafe { KillTimer(Some(self.owner), FADE_TIMER_ID) };
            return;
        };
        let elapsed = fade.started.elapsed().as_millis() as u32;
        if let Some(slot) = self.slot.as_ref() {
            slot.overlay
                .set_opacity(fade_alpha(fade.from, fade.to, elapsed, self.fade_ms));
            let _ = slot.overlay.refresh();
        }
        if elapsed >= self.fade_ms {
            let _ = unsafe { KillTimer(Some(self.owner), FADE_TIMER_ID) };
            self.fade = None;
            if fade.to == 0 {
                self.hide();
            }
        }
    }

    /// The pointer is on a HUD that was going away: it stays, at full opacity.
    fn cancel_fade_out(&mut self) {
        if self.fade.is_some_and(|fade| fade.to == 0) {
            self.fade = None;
            let _ = unsafe { KillTimer(Some(self.owner), FADE_TIMER_ID) };
            if let Some(slot) = self.slot.as_ref() {
                slot.overlay.set_opacity(slot.skin.opacity);
                let _ = slot.overlay.refresh();
            }
        }
    }

    /// The pointer left the tray icon: a held HUD goes after the usual timeout
    /// from now, as after any change.
    pub fn release(&mut self) {
        if std::mem::take(&mut self.holding) && !self.hovering {
            unsafe { SetTimer(Some(self.owner), TIMER_ID, self.timeout.max(100), None) };
        }
    }

    /// Hides it now, held or not, and lets go of the wheel.
    pub fn hide(&mut self) {
        self.holding = false;
        self.hovering = false;
        self.drag = None;
        self.pending_panel = false;
        self.visible = false;
        self.wheel = None;
        self.fade = None;
        let _ = unsafe { KillTimer(Some(self.owner), FADE_TIMER_ID) };
        let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
        if let Some(slot) = self.slot.as_ref() {
            slot.overlay.hide();
        }
    }

    /// Draws the state the owner got from a request, in place: the HUD stays
    /// where it is, under the pointer that is using it, and its timer is left
    /// as it is.
    pub fn redraw(&mut self, state: State) {
        let (Some(reading), Some(slot)) = (self.current.as_mut(), self.slot.as_mut()) else {
            return;
        };
        if reading.state.is_none() {
            return;
        }
        reading.state = Some(state);
        let _ = draw(
            slot.overlay.canvas_mut(),
            &slot.font,
            &slot.skin,
            slot.dpi,
            reading.kind,
            reading.state,
            self.look.as_ref(),
        );
        let _ = slot.overlay.refresh();
    }

    /// The geometry of what is on screen now, for a hit test.
    fn geometry(&self) -> Option<Geometry> {
        let slot = self.slot.as_ref()?;
        let reading = self.current.as_ref()?;
        let (width, height) = slot.overlay.size();
        let named = slot.skin.style == Style::Bar && self.look.is_some() && reading.state.is_some();
        Some(Geometry::new(
            slot.skin.style,
            width,
            height,
            named,
            |length| slot.skin.px(slot.dpi, length),
        ))
    }

    /// One of the messages of the HUD ([`is_message`]). What the pointer asks
    /// for comes back for the owner to carry out.
    pub fn on_message(&mut self, msg: u32, lparam: LPARAM) -> Option<Request> {
        use forward::{
            WM_HUD_DOWN, WM_HUD_ENTER, WM_HUD_LEAVE, WM_HUD_MOVE, WM_HUD_UP, WM_HUD_WHEEL,
        };

        match msg {
            // The pointer is using the HUD: it stays until the pointer is gone.
            WM_HUD_ENTER => {
                self.hovering = true;
                self.cancel_fade_out();
                let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
                None
            }
            WM_HUD_LEAVE => {
                self.hovering = false;
                if self.visible && !self.holding && self.drag.is_none() {
                    unsafe { SetTimer(Some(self.owner), TIMER_ID, self.timeout.max(100), None) };
                }
                None
            }
            WM_HUD_DOWN => self.on_down(forward::point_of(lparam)),
            WM_HUD_MOVE => self.on_drag(forward::point_of(lparam)),
            WM_HUD_UP => {
                self.drag = None;
                if std::mem::take(&mut self.pending_panel) {
                    return Some(Request::OpenPanel);
                }
                None
            }
            WM_HUD_WHEEL => {
                let delta = forward::take_wheel();
                let row = self.current.as_ref()?.row.clone()?;
                (delta != 0).then(|| Request::Change {
                    row,
                    change: Change::Scroll(delta as f32 / 120.0),
                })
            }
            _ => None,
        }
    }

    fn on_down(&mut self, (x, y): (i32, i32)) -> Option<Request> {
        let geometry = self.geometry()?;
        let reading = self.current.as_ref()?;
        // A reading that is not there has nothing to set or mute: a click on it
        // is the panel one.
        let Some((row, _)) = reading.row.clone().zip(reading.state) else {
            self.pending_panel = true;
            return None;
        };
        Some(match geometry.hit(x, y) {
            Hit::Glyph => Request::Change {
                row,
                change: Change::Action(Action::Mute),
            },
            Hit::Bar(level) => {
                let percent = model::to_percent(level);
                self.drag = Some(percent);
                Request::Change {
                    row,
                    change: Change::Action(Action::SetVolume(percent)),
                }
            }
            Hit::Other => {
                self.pending_panel = true;
                return None;
            }
        })
    }

    /// The pointer moved with the button down on the bar: the level follows,
    /// within 0 to 100 whether or not the pointer is still over the bar.
    fn on_drag(&mut self, (x, y): (i32, i32)) -> Option<Request> {
        let last = self.drag?;
        let percent = model::to_percent(self.geometry()?.level_at(x, y));
        if percent == last {
            return None;
        }
        self.drag = Some(percent);
        let row = self.current.as_ref()?.row.clone()?;
        Some(Request::Change {
            row,
            change: Change::Action(Action::SetVolume(percent)),
        })
    }
}

impl Drop for Hud {
    fn drop(&mut self) {
        let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
    }
}

#[cfg(test)]
mod fade_tests {
    use super::fade_alpha;

    #[test]
    fn a_fade_is_a_straight_line_between_its_ends() {
        assert_eq!(fade_alpha(0, 200, 0, 100), 0);
        assert_eq!(fade_alpha(0, 200, 50, 100), 100);
        assert_eq!(fade_alpha(0, 200, 100, 100), 200);
        assert_eq!(fade_alpha(200, 0, 25, 100), 150);
        // Past the end, or no duration at all: where it was going.
        assert_eq!(fade_alpha(0, 200, 500, 100), 200);
        assert_eq!(fade_alpha(255, 0, 10, 0), 0);
        // Resumed from the middle of another fade.
        assert_eq!(fade_alpha(80, 255, 100, 100), 255);
    }
}
