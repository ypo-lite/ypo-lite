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
use y_core::overlay::{
    Canvas, Color, Font, Overlay, OverlaySpec, TextAlign, alpha_of_percent,
};
use y_core::theme;

use y_coustic::config::{self, Style};
use y_coustic::model::{self, Action, Change, Kind, Level, State};

use crate::look::{AppRef, Look};
use crate::mixer::RowId;

mod forward;
mod layout;

use forward::Forwarder;
use layout::{
    FONT_HEIGHT, Geometry, Hit, PADDING, against, design_size, follow,
};
pub(crate) use layout::place;
pub use forward::{WM_TRAY_WHEEL, WheelHook, is_message, take_tray_wheel};

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

/// How the HUD looks, from the settings.
#[derive(Debug, Clone, PartialEq)]
struct Skin {
    style: Style,
    scale: u32,
    radius: i32,
    border: i32,
    opacity: u8,
    font: String,
    colors: config::Colors,
    interactive: bool,
}

impl Skin {
    fn of(settings: &config::HudConfig) -> Self {
        Self {
            style: settings.style,
            scale: settings.scale,
            radius: settings.radius as i32,
            border: settings.border as i32,
            opacity: alpha_of_percent(settings.opacity),
            font: settings.font.clone(),
            colors: settings.colors,
            interactive: settings.interactive,
        }
    }

    /// Whether a window built for `other` serves this skin: its size, font and
    /// whether it takes the pointer are fixed at creation, the rest is drawn.
    fn same_window(&self, other: &Skin) -> bool {
        self.style == other.style
            && self.scale == other.scale
            && self.font == other.font
            && self.interactive == other.interactive
    }

    /// A design length in real pixels: the size setting, then the DPI.
    fn px(&self, dpi: u32, length: i32) -> i32 {
        length * self.scale as i32 * dpi as i32 / (100 * 96)
    }
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
            &reading,
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
        let Some(slot) = self.slot.as_ref().filter(|_| self.fade_ms > 0 && self.visible) else {
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
            slot.overlay.set_opacity(fade_alpha(fade.from, fade.to, elapsed, self.fade_ms));
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
            reading,
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
        Some(Geometry::new(slot.skin.style, width, height, named, |length| {
            slot.skin.px(slot.dpi, length)
        }))
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

pub(crate) struct Palette {
    pub(crate) panel: Color,
    pub(crate) text: Color,
    pub(crate) track: Color,
    pub(crate) fill: Color,
    pub(crate) glyph: Color,
}

impl Palette {
    /// The theme's colours, then what `<colors>` says: for everything, then for
    /// this kind. The microphone's bar is red while it is live, the state worth
    /// noticing, so the general `accent` does not reach it: only its own does.
    pub(crate) fn current(kind: Kind, muted: bool, colors: &config::Colors) -> Self {
        let mut palette = Self::base(muted);
        let own = match kind {
            Kind::Master => &colors.master,
            Kind::Mic => &colors.microphone,
            Kind::App => &colors.application,
        };
        if kind == Kind::Mic && !muted {
            palette.fill = if theme::is_dark() {
                Color::rgb(0xE5, 0x6B, 0x66)
            } else {
                Color::rgb(0xC4, 0x2B, 0x1C)
            };
        }
        for tint in [&colors.all, own] {
            let is_own = std::ptr::eq(tint, own);
            palette.panel = tint.panel.unwrap_or(palette.panel);
            if let Some(text) = tint.text {
                palette.text = text;
                palette.glyph = text;
            }
            palette.track = tint.track.unwrap_or(palette.track);
            let accent = if kind == Kind::Mic && !is_own { None } else { tint.accent };
            if muted {
                palette.fill = tint.muted.unwrap_or(palette.fill);
            } else {
                palette.fill = accent.unwrap_or(palette.fill);
            }
        }
        palette
    }

    fn base(muted: bool) -> Self {
        if theme::is_dark() {
            Self {
                panel: Color::rgb(0x2B, 0x2B, 0x2B),
                text: Color::rgb(0xF2, 0xF2, 0xF2),
                track: Color::rgb(0x55, 0x55, 0x55),
                fill: if muted {
                    Color::rgb(0x8A, 0x8A, 0x8A)
                } else {
                    Color::rgb(0x60, 0xA5, 0xFA)
                },
                glyph: Color::rgb(0xF2, 0xF2, 0xF2),
            }
        } else {
            Self {
                panel: Color::rgb(0xF3, 0xF3, 0xF3),
                text: Color::rgb(0x1B, 0x1B, 0x1B),
                track: Color::rgb(0xC8, 0xC8, 0xC8),
                fill: if muted {
                    Color::rgb(0x9A, 0x9A, 0x9A)
                } else {
                    Color::rgb(0x00, 0x67, 0xC0)
                },
                glyph: Color::rgb(0x1B, 0x1B, 0x1B),
            }
        }
    }
}

fn draw(
    canvas: &mut Canvas,
    font: &Font,
    skin: &Skin,
    dpi: u32,
    reading: &Reading,
    look: Option<&Look>,
) -> Result<()> {
    let level = reading.state.map_or(0.0, |state| state.level);
    let muted = reading.state.is_none_or(|state| state.muted);
    let palette = Palette::current(reading.kind, muted, &skin.colors);
    let (width, height) = canvas.size();
    let px = |length: i32| skin.px(dpi, length);

    // In the `bar` style an application names itself on an upper row; the bar
    // and the percentage then share the lower one. Everything else is one row.
    let named = skin.style == Style::Bar && look.is_some() && reading.state.is_some();
    let g = Geometry::new(skin.style, width, height, named, px);

    // Panel: opaque, so the text below has a uniform alpha to restore.
    canvas.clear(palette.panel);

    // The percentage.
    if let (Some(state), Some(rect)) = (reading.state, g.text) {
        let text = if state.muted {
            "Muted".to_owned()
        } else {
            format!("{}%", model::to_percent(state.level))
        };
        let align = match skin.style {
            Style::Bar => TextAlign::Right,
            Style::Text => TextAlign::Left,
            _ => TextAlign::Centre,
        };
        canvas.draw_text(&text, rect, font, palette.text, align)?;
    }

    // Glyph, or the application's own icon.
    let glyph = g.glyph.right - g.glyph.left;
    let waves = waves(level, muted);
    let kind = reading.kind;
    let side = glyph as f32;
    match look.and_then(|look| look.pixels.as_deref().map(|pixels| (look.size, pixels))) {
        Some((size, pixels)) if size == glyph => {
            canvas.blit(g.glyph.left, g.glyph.top, size, size, pixels);
        }
        _ => paint(
            canvas,
            g.glyph.left,
            g.glyph.top,
            glyph,
            glyph,
            palette.glyph,
            |x, y| in_glyph(kind, x / side, y / side, waves, muted),
        ),
    }

    if let (true, Some(look), Some(track)) = (named, look, g.track) {
        let name_rect = RECT {
            left: track.left,
            top: px(4),
            right: width - px(PADDING),
            bottom: g.row_top,
        };
        canvas.draw_text_ellipsis(&look.name, name_rect, font, palette.text, TextAlign::Left)?;
    }
    if reading.state.is_none() {
        // Nothing to set: say why, where the bar would be (a narrow style
        // gets what fits of it).
        let message = match skin.style {
            Style::Bar | Style::Compact | Style::Text => RECT {
                left: g.glyph.right + px(PADDING * 3 / 4),
                top: 0,
                right: width - px(PADDING),
                bottom: height,
            },
            Style::VBar | Style::Ring => g.text.unwrap_or(RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            }),
        };
        let align = match skin.style {
            Style::VBar | Style::Ring => TextAlign::Centre,
            _ => TextAlign::Left,
        };
        canvas.draw_text_ellipsis(missing_text(reading.kind), message, font, palette.text, align)?;
        finish(canvas, skin, &px, palette.track);
        return Ok(());
    }

    if let Some(track) = g.track {
        let (w, h) = (track.right - track.left, track.bottom - track.top);
        if g.ring() {
            let (outer, thickness) = (g.outer as f32, g.thickness as f32);
            paint(canvas, track.left, track.top, w, h, palette.track, |x, y| {
                in_ring(x - outer, y - outer, outer, thickness)
            });
            if level > 0.0 {
                paint(canvas, track.left, track.top, w, h, palette.fill, |x, y| {
                    in_arc(x - outer, y - outer, outer, thickness, level)
                });
            }
        } else if g.vertical() {
            paint(canvas, track.left, track.top, w, h, palette.track, |x, y| {
                in_capsule(y, x, h as f32, w as f32)
            });
            let filled = fill_width(level, h, w);
            if filled > 0 {
                paint(
                    canvas,
                    track.left,
                    track.bottom - filled,
                    w,
                    filled,
                    palette.fill,
                    |x, y| in_capsule(y, x, filled as f32, w as f32),
                );
            }
        } else {
            paint(canvas, track.left, track.top, w, h, palette.track, |x, y| {
                in_capsule(x, y, w as f32, h as f32)
            });
            let filled = fill_width(level, w, h);
            if filled > 0 {
                paint(
                    canvas,
                    track.left,
                    track.top,
                    filled,
                    h,
                    palette.fill,
                    |x, y| in_capsule(x, y, filled as f32, h as f32),
                );
            }
        }
    }

    finish(canvas, skin, &px, palette.track);
    Ok(())
}

/// The last step: the border, then the corners, which see the finished image.
fn finish(canvas: &mut Canvas, skin: &Skin, px: &impl Fn(i32) -> i32, border_color: Color) {
    let radius = px(skin.radius);
    let border = if skin.border > 0 { px(skin.border).max(1) } else { 0 };
    if border == 0 {
        canvas.round_corners(radius);
        return;
    }
    let (width, height) = canvas.size();
    for inset in 0..border {
        canvas.stroke_rect(
            RECT {
                left: inset,
                top: inset,
                right: width - inset,
                bottom: height - inset,
            },
            border_color,
        );
    }
    canvas.round_corners_with_border(radius, border, border_color);
}

/// Is (`dx`, `dy`), relative to the middle of a ring of outer radius `outer`
/// and band `thickness`, on the band?
fn in_ring(dx: f32, dy: f32, outer: f32, thickness: f32) -> bool {
    let distance = (dx * dx + dy * dy).sqrt();
    distance <= outer && distance >= outer - thickness
}

/// The part of the ring from the top, clockwise, as far as `level` (0 to 1),
/// with a round end at both ends of the arc.
fn in_arc(dx: f32, dy: f32, outer: f32, thickness: f32, level: Level) -> bool {
    let level = level.clamp(0.0, 1.0);
    let cap = thickness / 2.0;
    let middle = outer - cap;
    let angle_end = level * std::f32::consts::TAU;
    let at = |angle: f32| (middle * angle.sin(), -middle * angle.cos());
    let in_cap = |(cx, cy): (f32, f32)| (dx - cx).powi(2) + (dy - cy).powi(2) <= cap * cap;
    if in_cap(at(0.0)) || in_cap(at(angle_end)) {
        return true;
    }
    if !in_ring(dx, dy, outer, thickness) {
        return false;
    }
    let mut angle = dx.atan2(-dy);
    if angle < 0.0 {
        angle += std::f32::consts::TAU;
    }
    angle <= angle_end
}

/// What the HUD says when the device it was asked about does not exist.
fn missing_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Mic => "No microphone",
        Kind::Master => "No audio output",
        Kind::App => "No audio",
    }
}

/// Width of the filled part of the bar: nothing at silence, and never less
/// than the bar's height otherwise, so the rounded end stays a circle.
pub(crate) fn fill_width(level: Level, bar_width: i32, bar_height: i32) -> i32 {
    if level <= 0.0 {
        return 0;
    }
    ((bar_width as f32 * level.min(1.0)).round() as i32).clamp(bar_height, bar_width)
}

/// How many sound waves the speaker glyph shows.
pub(crate) fn waves(level: Level, muted: bool) -> u8 {
    if muted || level <= 0.0 {
        0
    } else if level < 0.5 {
        1
    } else {
        2
    }
}

/// The glyph of `kind`: a microphone for the microphone, a speaker otherwise.
pub(crate) fn in_glyph(kind: Kind, u: f32, v: f32, waves: u8, muted: bool) -> bool {
    match kind {
        Kind::Mic => in_microphone(u, v, muted),
        Kind::Master | Kind::App => in_speaker(u, v, waves, muted),
    }
}

/// Is the point in the unit square inside the microphone glyph? A capsule, the
/// cradle under it, a stem and a base; slashed through when muted.
fn in_microphone(u: f32, v: f32, muted: bool) -> bool {
    // Capsule: a vertical segment with a radius.
    if distance_to_segment((u, v), (0.5, 0.26), (0.5, 0.44)) <= 0.13 {
        return true;
    }
    // Cradle: the lower half of a ring around the capsule's lower centre.
    let (dx, dy) = (u - 0.5, v - 0.44);
    let radius = (dx * dx + dy * dy).sqrt();
    if dy >= 0.0 && (radius - 0.22).abs() <= 0.03 {
        return true;
    }
    // Stem and base.
    if (0.485..0.515).contains(&u) && (0.64..0.84).contains(&v) {
        return true;
    }
    if (0.34..0.66).contains(&u) && (0.82..0.87).contains(&v) {
        return true;
    }
    // The slash of the muted glyph.
    muted && distance_to_segment((u, v), (0.18, 0.12), (0.82, 0.92)) <= 0.045
}

/// Is the point (`u`, `v`) in the unit square inside the speaker glyph? The
/// cone and body at the left; waves as arcs on the right; when muted, a cross
/// where the waves would be.
fn in_speaker(u: f32, v: f32, waves: u8, muted: bool) -> bool {
    // Body.
    if (0.08..0.28).contains(&u) && (0.38..0.62).contains(&v) {
        return true;
    }
    // Cone, widening to the right.
    if (0.28..0.5).contains(&u) {
        let half = 0.12 + (u - 0.28) / 0.22 * 0.20;
        if (v - 0.5).abs() <= half {
            return true;
        }
    }
    // Waves: arcs of rings around the cone's mouth, within 50 degrees of the
    // axis.
    let (dx, dy) = (u - 0.5, v - 0.5);
    if dx > 0.0 {
        let radius = (dx * dx + dy * dy).sqrt();
        let within_angle = dy.abs() <= dx * 1.19; // tan(50 degrees)
        if within_angle {
            if waves >= 1 && (radius - 0.24).abs() <= 0.035 {
                return true;
            }
            if waves >= 2 && (radius - 0.38).abs() <= 0.035 {
                return true;
            }
        }
    }
    // The cross of the muted glyph.
    if muted {
        let thickness = 0.045;
        return distance_to_segment((u, v), (0.62, 0.36), (0.9, 0.64)) <= thickness
            || distance_to_segment((u, v), (0.62, 0.64), (0.9, 0.36)) <= thickness;
    }
    false
}

fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let (apx, apy) = (p.0 - a.0, p.1 - a.1);
    let t = ((apx * abx + apy * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    let (cx, cy) = (a.0 + t * abx, a.1 + t * aby);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Is the pixel-space point (`x`, `y`) inside a capsule (a rounded bar) of
/// `width` by `height` at the origin?
pub(crate) fn in_capsule(x: f32, y: f32, width: f32, height: f32) -> bool {
    let radius = height / 2.0;
    let centre = (x.clamp(radius, (width - radius).max(radius)), radius);
    (x - centre.0).powi(2) + (y - centre.1).powi(2) <= radius * radius
}

/// Paints `color` over the opaque pixels of a `width` by `height` box at
/// (`left`, `top`) wherever `inside` holds, antialiased by 4x4 supersampling.
///
/// `inside` takes pixel offsets from the box's top left.
pub(crate) fn paint(
    canvas: &mut Canvas,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    color: Color,
    inside: impl Fn(f32, f32) -> bool,
) {
    const N: i32 = 4;
    let (surface_width, surface_height) = canvas.size();
    let pixels = canvas.pixels_mut();

    for y in top.max(0)..(top + height).min(surface_height) {
        for x in left.max(0)..(left + width).min(surface_width) {
            let mut hits = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let fx = (x - left) as f32 + (sx as f32 + 0.5) / N as f32;
                    let fy = (y - top) as f32 + (sy as f32 + 0.5) / N as f32;
                    hits += i32::from(inside(fx, fy));
                }
            }
            if hits == 0 {
                continue;
            }
            let index = (y * surface_width + x) as usize;
            pixels[index] = mix(pixels[index], color, hits as f32 / (N * N) as f32);
        }
    }
}

/// `color` over an opaque premultiplied `below`, with `coverage` in 0..=1.
pub(crate) fn mix(below: u32, color: Color, coverage: f32) -> u32 {
    let channel = |shift: u32, value: u8| -> u32 {
        let from = ((below >> shift) & 0xFF) as f32;
        (from + (f32::from(value) - from) * coverage).round() as u32
    };
    0xFF00_0000 | (channel(16, color.r) << 16) | (channel(8, color.g) << 8) | channel(0, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_empty_at_silence_and_full_at_one() {
        assert_eq!(fill_width(0.0, 100, 6), 0);
        assert_eq!(fill_width(1.0, 100, 6), 100);
        assert_eq!(fill_width(0.5, 100, 6), 50);
    }

    #[test]
    fn a_nearly_silent_bar_keeps_its_rounded_end() {
        assert_eq!(fill_width(0.01, 100, 6), 6);
    }

    #[test]
    fn waves_follow_the_level_and_vanish_when_muted() {
        assert_eq!(waves(0.0, false), 0);
        assert_eq!(waves(0.2, false), 1);
        assert_eq!(waves(0.5, false), 2);
        assert_eq!(waves(1.0, false), 2);
        assert_eq!(waves(1.0, true), 0);
    }

    #[test]
    fn the_glyph_has_a_body_and_a_cross_only_when_muted() {
        assert!(in_speaker(0.18, 0.5, 0, false));
        assert!(!in_speaker(0.76, 0.5, 0, false));
        assert!(in_speaker(0.76, 0.5, 0, true)); // the cross' centre
        assert!(!in_speaker(0.5, 0.05, 2, false));
    }

    #[test]
    fn the_microphone_glyph_is_slashed_only_when_muted() {
        assert!(in_microphone(0.5, 0.35, false)); // the capsule
        assert!(in_microphone(0.5, 0.84, false)); // the base
        assert!(!in_microphone(0.15, 0.15, false));
        assert!(in_microphone(0.5, 0.52, true)); // the middle of the slash
        assert!(!in_microphone(0.2, 0.9, true));
        assert!(!in_microphone(0.8, 0.2, true));
    }

    #[test]
    fn the_glyph_follows_the_kind() {
        assert!(in_glyph(Kind::Master, 0.18, 0.5, 0, false)); // speaker body
        assert!(!in_glyph(Kind::Mic, 0.18, 0.5, 0, false));
        assert!(in_glyph(Kind::Mic, 0.5, 0.35, 0, false));
    }

    #[test]
    fn each_missing_device_has_its_own_message() {
        assert_eq!(missing_text(Kind::Mic), "No microphone");
        assert_ne!(missing_text(Kind::Master), missing_text(Kind::Mic));
    }

    #[test]
    fn a_wave_only_shows_when_asked_for() {
        // On the inner ring, level with the cone's mouth.
        assert!(in_speaker(0.5 + 0.24, 0.5, 1, false));
        assert!(!in_speaker(0.5 + 0.24, 0.5, 0, false));
        assert!(in_speaker(0.5 + 0.38, 0.5, 2, false));
        assert!(!in_speaker(0.5 + 0.38, 0.5, 1, false));
    }

    #[test]
    fn a_capsule_is_round_at_both_ends() {
        assert!(in_capsule(50.0, 3.0, 100.0, 6.0));
        assert!(in_capsule(3.0, 3.0, 100.0, 6.0));
        assert!(!in_capsule(0.2, 0.2, 100.0, 6.0)); // outside the corner arc
        assert!(!in_capsule(99.8, 5.8, 100.0, 6.0));
    }

    #[test]
    fn mixing_blends_toward_the_colour() {
        let below = 0xFF00_0000;
        assert_eq!(mix(below, Color::rgb(200, 100, 50), 0.0), below);
        assert_eq!(mix(below, Color::rgb(200, 100, 50), 1.0), 0xFFC8_6432);
        assert_eq!(mix(below, Color::rgb(200, 100, 50), 0.5), 0xFF64_3219);
    }

    #[test]
    fn a_ring_is_a_band_and_its_arc_runs_clockwise_from_the_top() {
        // On the band at the right (a quarter turn), not in the middle or outside.
        assert!(in_ring(40.0, 0.0, 44.0, 8.0));
        assert!(!in_ring(0.0, 0.0, 44.0, 8.0));
        assert!(!in_ring(50.0, 0.0, 44.0, 8.0));
        // A quarter: the top and the right are filled, the bottom and the left are not.
        assert!(in_arc(0.0, -40.0, 44.0, 8.0, 0.25));
        assert!(in_arc(40.0, 0.0, 44.0, 8.0, 0.25));
        assert!(!in_arc(0.0, 40.0, 44.0, 8.0, 0.25));
        assert!(!in_arc(-40.0, 0.0, 44.0, 8.0, 0.25));
        // Everything at full, a rounded start even at a sliver.
        assert!(in_arc(-40.0, 0.0, 44.0, 8.0, 1.0));
        assert!(in_arc(0.0, -40.0, 44.0, 8.0, 0.001));
    }

    #[test]
    fn a_skin_scales_by_size_then_by_dpi_and_keeps_its_window_when_only_drawn_things_change() {
        let mut settings = config::HudConfig::default();
        let skin = Skin::of(&settings);
        assert_eq!(skin.px(96, 100), 100);
        assert_eq!(skin.px(192, 100), 200);
        settings.scale = 150;
        assert_eq!(Skin::of(&settings).px(96, 100), 150);

        let base = Skin::of(&config::HudConfig::default());
        let drawn = Skin::of(&config::HudConfig {
            radius: 0,
            border: 3,
            opacity: 50,
            ..config::HudConfig::default()
        });
        assert!(base.same_window(&drawn));
        for different in [
            config::HudConfig {
                style: Style::Ring,
                ..config::HudConfig::default()
            },
            config::HudConfig {
                scale: 200,
                ..config::HudConfig::default()
            },
            config::HudConfig {
                font: "Consolas".into(),
                ..config::HudConfig::default()
            },
            config::HudConfig {
                interactive: false,
                ..config::HudConfig::default()
            },
        ] {
            assert!(!base.same_window(&Skin::of(&different)));
        }
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;
    use config::{Colors, Tint};

    #[test]
    fn a_colour_reaches_its_kind_and_the_general_accent_spares_the_microphone() {
        let green = Color::rgb(0, 255, 0);
        let blue = Color::rgb(0, 0, 255);
        let colors = Colors {
            all: Tint {
                accent: Some(green),
                ..Tint::default()
            },
            application: Tint {
                accent: Some(blue),
                ..Tint::default()
            },
            ..Colors::default()
        };
        assert_eq!(Palette::current(Kind::Master, false, &colors).fill, green);
        assert_eq!(Palette::current(Kind::App, false, &colors).fill, blue);
        let mic = Palette::current(Kind::Mic, false, &colors).fill;
        assert_ne!(mic, green);
        assert_eq!(mic, Palette::current(Kind::Mic, false, &Colors::default()).fill);

        let own = Colors {
            microphone: Tint {
                accent: Some(blue),
                ..Tint::default()
            },
            ..colors
        };
        assert_eq!(Palette::current(Kind::Mic, false, &own).fill, blue);
        // Muted is its own colour, not the accent.
        assert_ne!(Palette::current(Kind::Master, true, &colors).fill, green);
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
