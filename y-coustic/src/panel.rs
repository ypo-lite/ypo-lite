//! The panel: every volume in one focusable window, for "let me set this up".
//!
//! # Shape
//!
//! The same as `y-snap`'s and `y-pop`'s popups. The window's own handler
//! ([`Forwarder`]) keeps no state worth the name and **posts private messages to
//! the owner window**, which owns the [`Panel`] and does the work; the panel
//! never touches audio itself. What it wants done comes back from
//! [`Panel::on_message`] as [`Request`]s, which the owner carries out through
//! [`crate::mixer`] before handing the panel the rows as they read afterwards
//! ([`Panel::update`]). Where a row is and what a point on it means is
//! [`layout`], pure and tested.
//!
//! The overlay is created per opening and dropped on close, so nothing stays
//! committed between uses; rows are read when it opens and again once a second
//! while it is open (the owner's timer), then dropped with it. Every message
//! carries the opening's **generation**: closing a focused window sends it a
//! `WM_KILLFOCUS` synchronously, which posts a close that arrives after the
//! call returns, by when a quick hotkey may have opened a new one.
//!
//! # Keyboard focus and dismissal
//!
//! Keys arrive as `WM_KEYDOWN`, and `WM_KILLFOCUS` means "clicked elsewhere,
//! close": no keyboard hook. A hotkey grants the process the right to take the
//! foreground; a click on a hook-claimed mouse binding may not. When
//! [`Overlay::activate`] does not win the focus the panel asks for a click: it
//! captures the mouse, a click on it takes the focus, a click outside closes it,
//! which is what `WM_KILLFOCUS` would have meant had it ever had the focus.
//!
//! The volume keys reach a focused window as `WM_APPCOMMAND` when y-coustic does
//! not hold them itself (`plain="windows"`); the handler turns those into the
//! same key message so they act on the selected row either way.
//!
//! # Height
//!
//! A layered window's surface is fixed at creation, so the panel is created for
//! `max-rows` and shown at the height in use ([`Overlay::show_at_size`]);
//! beyond `max-rows` the list scrolls with the selection.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY,
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_NUMPAD0, VK_PRIOR, VK_RIGHT, VK_SHIFT,
    VK_SPACE, VK_UP,
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, KillTimer, PostMessageW, SetForegroundWindow, SetTimer, WM_APPCOMMAND,
    WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_SETFOCUS, WM_SYSCHAR, WM_SYSKEYDOWN,
};
use windows::core::Result;

use y_core::ids::WM_APP_FIRST;
use y_core::monitor::Monitor;
use y_core::overlay::{Canvas, Color, Font, Overlay, OverlaySpec, TextAlign};
use y_core::theme;
use y_core::window::WindowHandler;

use y_coustic::config::{Anchor, Colors, HudConfig, MixerConfig, Position};
use y_coustic::model::{self, Action, Change, Kind, State};

use y_coustic::look::{AppRef, Look};
use y_coustic::paint::{self as hud, Palette};
use crate::mixer::{Row, RowId};

pub mod layout;

use layout::{Layout, Metrics, Part, ScrollHit};

const CLASS: &str = "YpoLite.YCoustic.Panel";

/// `wParam` of the `WM_TIMER` that has the owner refresh the rows while the
/// panel is open.
pub const TIMER_ID: usize = 0xC0_0002;
const REFRESH_MS: u32 = 1000;

/// Posted by the handler: `wParam` is the opening's generation.
pub const WM_PANEL_CLOSE: u32 = WM_APP_FIRST + 16;
/// `lParam`: the virtual key in the low word, the modifiers (`MOD_*` below) above.
pub const WM_PANEL_KEY: u32 = WM_APP_FIRST + 17;
/// `lParam`: the client position, see [`point_of`].
pub const WM_PANEL_DOWN: u32 = WM_APP_FIRST + 18;
pub const WM_PANEL_MOVE: u32 = WM_APP_FIRST + 19;
pub const WM_PANEL_UP: u32 = WM_APP_FIRST + 20;
pub const WM_PANEL_MIDDLE: u32 = WM_APP_FIRST + 21;
/// `lParam`: the client position in the low 32 bits, the wheel delta (signed)
/// from bit 32.
pub const WM_PANEL_WHEEL: u32 = WM_APP_FIRST + 22;
/// The panel got the keyboard focus (after waiting for a click to take it).
pub const WM_PANEL_FOCUSED: u32 = WM_APP_FIRST + 23;

const MOD_SHIFT: isize = 1;

// Design sizes, at 100% scaling.
const WIDTH: i32 = 380;
const PAD: i32 = 8;
const ROW: i32 = 40;
const SEPARATOR: i32 = 9;
const ICON: i32 = 24;
const GAP: i32 = 10;
const NAME_WIDTH: i32 = 120;
const PERCENT_WIDTH: i32 = 52;
const SCROLLBAR: i32 = 14;
const BAR_HEIGHT: i32 = 6;
const FONT_HEIGHT: i32 = 16;
/// Distance from the icon or the pointer.
const MARGIN: i32 = 8;

/// What the panel asks its owner to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Apply `change` to a row, then give the panel the rows back.
    Change { row: RowId, change: Change },
}

/// Where the panel should open, and what it was asked for.
pub struct OpenWith<'a> {
    pub config: &'a MixerConfig,
    /// The tray icon's place, when the shell says.
    pub tray: Option<RECT>,
    /// Where the HUD would be: for `anchor="hud"`.
    pub hud_position: Position,
    /// The HUD's look, which the mixer shares: colours, font, radius.
    pub look: &'a HudConfig,
    pub rows: Vec<Row>,
    /// The row to start on, if the list has it.
    pub select: Option<RowId>,
}

pub struct Panel {
    owner: HWND,
    open: Option<Open>,
    /// Bumped by every opening; see the module doc.
    generation: usize,
    /// When the panel last closed because it lost the focus: a click on the
    /// tray icon takes the focus first, then arrives as a click, and must not
    /// reopen what it just closed ([`Panel::just_lost_focus`]).
    focus_lost: Option<Instant>,
}

/// What a held left button is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Drag {
    /// A bar being set: the row, and the last percentage sent for it.
    Bar(RowId, u8),
    /// The scrollbar's thumb, taken this many pixels below its top.
    Thumb(i32),
}

/// How long after opening a loss of focus is read as the shell's, not the
/// user's: the tray's overflow flyout closes and hands the focus on right
/// after the click that opened the panel, and a panel closed by that would
/// flash and vanish.
const EARLY: Duration = Duration::from_millis(800);
/// How often it takes the focus back in that time before giving up.
const MAX_REGRABS: u8 = 3;

struct Open {
    overlay: Overlay,
    metrics: Metrics,
    max_rows: usize,
    rows: Vec<Row>,
    selected: RowId,
    scroll: usize,
    /// What the pressed mouse button holds, if anything.
    drag: Option<Drag>,
    /// Shared with the handler: true while waiting for a click to take the
    /// focus.
    awaiting: Rc<Cell<bool>>,
    /// When it opened, and how many times it took the focus back since: see
    /// [`Open::lost_focus_early`].
    opened_at: Instant,
    regrabs: u8,
    font: Font,
    colors: Colors,
    radius: i32,
    bar_height: i32,
    /// Names and icons, by executable (or by name, with none).
    looks: HashMap<String, Look>,
    /// Where the panel is, and whether it grows upward (a panel above a
    /// bottom taskbar keeps its bottom edge).
    position: POINT,
    grow_up: bool,
    monitor: Monitor,
    /// The size shown, shared with the handler to tell a click outside.
    shown: Rc<Cell<(i32, i32)>>,
    /// False while waiting for a click to take the focus.
    focused: bool,
}

impl Panel {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            open: None,
            generation: 0,
            focus_lost: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Whether the panel closed a moment ago for want of focus. A click on the
    /// tray icon does that before it is delivered as a click; reading it as
    /// "open" would reopen the panel it was meant to close.
    pub fn just_lost_focus(&self) -> bool {
        self.focus_lost
            .is_some_and(|at| at.elapsed() < Duration::from_millis(400))
    }

    /// The row the keys and the volume hotkeys act on now.
    pub fn selected(&self) -> Option<&RowId> {
        self.open.as_ref().map(|open| &open.selected)
    }

    /// Whether a bar is being dragged: the owner holds its refresh then, which
    /// would move the bar under the pointer.
    pub fn is_dragging(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.drag.is_some())
    }

    /// Whether `msg` is one of the panel's.
    pub fn is_message(msg: u32) -> bool {
        (WM_PANEL_CLOSE..=WM_PANEL_FOCUSED).contains(&msg)
    }

    pub fn open(&mut self, with: OpenWith<'_>) -> Result<()> {
        if self.is_open() {
            return Ok(());
        }
        self.generation += 1;

        let anchor_point = match (with.config.anchor, with.tray) {
            (Anchor::Tray, Some(rect)) => POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            },
            _ => y_core::monitor::cursor_position().unwrap_or_default(),
        };
        let monitor = Monitor::from_point(anchor_point);
        let scaled = |length: i32| monitor.scaled(length);
        let metrics = Metrics {
            width: scaled(WIDTH),
            pad: scaled(PAD),
            row_height: scaled(ROW),
            separator: scaled(SEPARATOR),
            icon: scaled(ICON),
            gap: scaled(GAP),
            name_width: scaled(NAME_WIDTH),
            percent_width: scaled(PERCENT_WIDTH),
            scrollbar: scaled(SCROLLBAR),
        };
        let max_rows = with.config.max_rows as usize;

        // The list starts at the top, the master in view. The row asked for is
        // selected only when it is shown there: a selection out of sight would
        // have the volume keys act on something the user cannot see.
        let selected = with
            .select
            .filter(|id| {
                with.rows
                    .iter()
                    .position(|row| row.id == *id)
                    .is_some_and(|index| index < max_rows)
            })
            .unwrap_or(RowId::Master);
        let scroll = 0;
        let layout = Layout::new(metrics, with.rows.len(), max_rows, scroll);
        let size = layout.size();

        let (position, grow_up) = place(
            with.config.anchor,
            with.tray,
            anchor_point,
            with.hud_position,
            size,
            &monitor,
        );

        let shown = Rc::new(Cell::new(size));
        let awaiting = Rc::new(Cell::new(false));
        let handler = Forwarder {
            owner: self.owner,
            generation: self.generation,
            size: Rc::clone(&shown),
            awaiting: Rc::clone(&awaiting),
            swallow_release: false,
        };
        let spec = OverlaySpec::new(CLASS, Layout::max_size(metrics, max_rows))
            .interactive()
            .focusable()
            .with_owner(self.owner);
        let overlay = Overlay::with_handler(&spec, Box::new(handler))?;

        let mut open = Open {
            overlay,
            metrics,
            max_rows,
            rows: with.rows,
            selected,
            scroll,
            drag: None,
            awaiting: Rc::clone(&awaiting),
            opened_at: Instant::now(),
            regrabs: 0,
            font: Font::new(&with.look.font, scaled(FONT_HEIGHT), false)?,
            colors: with.look.colors,
            radius: scaled(with.look.radius as i32),
            bar_height: scaled(BAR_HEIGHT),
            looks: HashMap::new(),
            position,
            grow_up,
            monitor,
            shown,
            focused: true,
        };
        open.paint()?;
        open.overlay.show_at_size(position, size)?;
        open.take_focus();

        unsafe { SetTimer(Some(self.owner), TIMER_ID, REFRESH_MS, None) };
        self.open = Some(open);
        Ok(())
    }

    /// Closes the panel. Safe when already closed.
    pub fn close(&mut self) {
        let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
        // Dropping the overlay destroys its window and frees its pixels.
        self.open = None;
    }

    /// Gives the panel the rows as they read now: the selection stays on its
    /// row if the row is still there, else moves to the master.
    pub fn update(&mut self, rows: Vec<Row>) {
        if let Some(open) = self.open.as_mut() {
            open.rows = rows;
            if !open.rows.iter().any(|row| row.id == open.selected) {
                open.selected = RowId::Master;
            }
            let _ = open.repaint();
        }
    }

    /// The owner's message hook. What to do about the audio comes back.
    /// `step_pct` is the configured step, for the keys that move by one.
    pub fn on_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        step_pct: f32,
    ) -> Vec<Request> {
        // A message from a window that is already gone.
        if wparam.0 != self.generation {
            return Vec::new();
        }
        let Some(open) = self.open.as_mut() else {
            return Vec::new();
        };

        let mut requests = Vec::new();
        match msg {
            WM_PANEL_CLOSE => {
                if open.lost_focus_early() {
                    open.take_focus();
                } else {
                    self.focus_lost = Some(Instant::now());
                    self.close();
                }
            }
            WM_PANEL_FOCUSED => {
                open.focused = true;
                let _ = open.repaint();
            }
            WM_PANEL_KEY => {
                let vk = (lparam.0 & 0xFFFF) as u16;
                let shift = (lparam.0 >> 16) & MOD_SHIFT != 0;
                let closed = open.on_key(vk, shift, step_pct, &mut requests);
                if closed {
                    self.close();
                }
            }
            WM_PANEL_DOWN => open.on_down(point_of(lparam), &mut requests),
            WM_PANEL_MOVE => open.on_move(point_of(lparam), &mut requests),
            WM_PANEL_UP => open.drag = None,
            WM_PANEL_MIDDLE => open.on_middle(point_of(lparam), &mut requests),
            WM_PANEL_WHEEL => {
                let delta = (lparam.0 >> 32) as i16;
                open.on_wheel(point_of(lparam), delta, &mut requests);
            }
            _ => {}
        }
        requests
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
    }
}

/// Where the top left of a `size` panel goes, and whether it grows upward.
fn place(
    anchor: Anchor,
    tray: Option<RECT>,
    pointer: POINT,
    hud_position: Position,
    size: (i32, i32),
    monitor: &Monitor,
) -> (POINT, bool) {
    let margin = monitor.scaled(MARGIN);
    let area = monitor.work_area;
    let (position, grow_up) = match (anchor, tray) {
        // Above the icon when it sits in the lower half of the screen (a
        // bottom taskbar), below it otherwise; centred on it.
        (Anchor::Tray, Some(icon)) => {
            let x = (icon.left + icon.right) / 2 - size.0 / 2;
            let middle = area.top + (area.bottom - area.top) / 2;
            if icon.top >= middle {
                (POINT { x, y: icon.top - size.1 - margin }, true)
            } else {
                (POINT { x, y: icon.bottom + margin }, false)
            }
        }
        (Anchor::Hud, _) => (
            hud::layout::place(area, size, hud_position, monitor),
            hud_position.is_bottom(),
        ),
        // The pointer's place: the corner nearest the middle of the screen
        // stays on it, so the panel opens toward the content.
        _ => {
            let middle = area.top + (area.bottom - area.top) / 2;
            let below = pointer.y < middle;
            (
                POINT {
                    x: pointer.x - size.0 / 2,
                    y: if below { pointer.y + margin } else { pointer.y - size.1 - margin },
                },
                !below,
            )
        }
    };
    (monitor.keep_inside(position, size), grow_up)
}

/// The client position packed in a message's `lParam` (two signed words).
fn point_of(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

impl Open {
    fn layout(&self) -> Layout {
        Layout::new(self.metrics, self.rows.len(), self.max_rows, self.scroll)
    }

    fn selected_index(&self) -> usize {
        self.rows
            .iter()
            .position(|row| row.id == self.selected)
            .unwrap_or(0)
    }

    fn change(&self, index: usize, change: Change, requests: &mut Vec<Request>) {
        if let Some(row) = self.rows.get(index) {
            requests.push(Request::Change {
                row: row.id.clone(),
                change,
            });
        }
    }

    fn select(&mut self, index: usize) {
        if let Some(row) = self.rows.get(index) {
            self.selected = row.id.clone();
        }
        self.scroll = self.layout().scroll_to_show(index);
        let _ = self.repaint();
    }

    /// A key. True when it closes the panel.
    fn on_key(&mut self, vk: u16, shift: bool, step_pct: f32, requests: &mut Vec<Request>) -> bool {
        let index = self.selected_index();
        let count = self.rows.len();
        let notch = |direction: f32| {
            // Shift: one percent, whatever the step is.
            let notches = if shift { 1.0 / step_pct.max(1.0) } else { 1.0 };
            Change::Scroll(direction * notches)
        };

        match VIRTUAL_KEY(vk) {
            VK_ESCAPE => return true,
            VK_UP => self.select(layout::move_selection(index, -1, count)),
            VK_DOWN => self.select(layout::move_selection(index, 1, count)),
            // A page, or the ends: the list follows the selection.
            VK_PRIOR => self.select(layout::move_selection(index, -(self.max_rows as isize), count)),
            VK_NEXT => self.select(layout::move_selection(index, self.max_rows as isize, count)),
            VK_HOME => self.select(0),
            VK_END => self.select(count.saturating_sub(1)),
            VK_LEFT => self.change(index, notch(-1.0), requests),
            VK_RIGHT => self.change(index, notch(1.0), requests),
            VK_VOLUME_UP => self.change(index, Change::Action(Action::VolumeUp), requests),
            VK_VOLUME_DOWN => self.change(index, Change::Action(Action::VolumeDown), requests),
            VK_VOLUME_MUTE | VK_SPACE => {
                self.change(index, Change::Action(Action::Mute), requests);
            }
            key if key.0 == u16::from(b'M') => {
                self.change(index, Change::Action(Action::Mute), requests);
            }
            // 0 to 9 set 0 to 90 percent, on the top row and on the numpad.
            key => {
                let digit = match key.0 {
                    d @ 0x30..=0x39 => Some(d - 0x30),
                    d if (VK_NUMPAD0.0..VK_NUMPAD0.0 + 10).contains(&d) => Some(d - VK_NUMPAD0.0),
                    _ => None,
                };
                if let Some(digit) = digit {
                    let percent = (digit * 10) as u8;
                    self.change(index, Change::Action(Action::SetVolume(percent)), requests);
                }
            }
        }
        false
    }

    fn on_down(&mut self, (x, y): (i32, i32), requests: &mut Vec<Request>) {
        let layout = self.layout();

        // The scrollbar first: its gutter lies beside the rows.
        if let Some(part) = layout.scrollbar_hit(x, y) {
            match part {
                ScrollHit::Thumb => {
                    let grab = layout.thumb().map_or(0, |(top, _)| y - top);
                    self.drag = Some(Drag::Thumb(grab));
                }
                ScrollHit::Above => self.scroll_to(layout.paged(-1)),
                ScrollHit::Below => self.scroll_to(layout.paged(1)),
            }
            return;
        }

        let Some(hit) = layout.hit(x, y) else {
            return;
        };
        self.select(hit.row);
        match hit.part {
            Part::Icon => self.change(hit.row, Change::Action(Action::Mute), requests),
            Part::Bar(level) => {
                let percent = model::to_percent(level);
                if let Some(row) = self.rows.get(hit.row) {
                    self.drag = Some(Drag::Bar(row.id.clone(), percent));
                }
                self.change(
                    hit.row,
                    Change::Action(Action::SetVolume(percent)),
                    requests,
                );
            }
            Part::Row => {}
        }
    }

    /// The pointer moved with the button down: a bar follows, even outside the
    /// bar, within 0 to 100; the scrollbar's thumb follows within the track.
    fn on_move(&mut self, (x, y): (i32, i32), requests: &mut Vec<Request>) {
        match self.drag.clone() {
            Some(Drag::Bar(id, last)) => {
                let percent = model::to_percent(self.layout().drag_level(x));
                if percent == last {
                    return;
                }
                self.drag = Some(Drag::Bar(id.clone(), percent));
                requests.push(Request::Change {
                    row: id,
                    change: Change::Action(Action::SetVolume(percent)),
                });
            }
            Some(Drag::Thumb(grab)) => {
                let scroll = self.layout().scroll_for_thumb(y, grab);
                self.scroll_to(scroll);
            }
            None => {}
        }
    }

    /// Scrolls the list to `scroll` and redraws, if that is a move.
    fn scroll_to(&mut self, scroll: usize) {
        let scroll = self.layout().paged_to(scroll);
        if scroll != self.scroll {
            self.scroll = scroll;
            // The window keeps its place: only what is drawn in it changes,
            // so this is not `repaint`, which would bring the selection back.
            let _ = self.paint();
            let _ = self.overlay.refresh_size(self.layout().size());
        }
    }

    fn on_middle(&mut self, (x, y): (i32, i32), requests: &mut Vec<Request>) {
        if let Some(hit) = self.layout().hit(x, y) {
            self.select(hit.row);
            self.change(hit.row, Change::Action(Action::Mute), requests);
        }
    }

    /// The wheel: over the scrollbar's gutter it scrolls the list, a row at a
    /// notch; over a row it steps that row.
    fn on_wheel(&mut self, (x, y): (i32, i32), delta: i16, requests: &mut Vec<Request>) {
        let layout = self.layout();
        if layout.scrollbar_hit(x, y).is_some() {
            // Up is toward the top; at least one row, however fine the delta.
            let rows = (i32::from(delta) / 120).unsigned_abs().max(1) as isize;
            let signed = if delta > 0 { -rows } else { rows };
            self.scroll_to(self.scroll.saturating_add_signed(signed));
        } else if let Some(hit) = layout.hit(x, y) {
            let notches = f32::from(delta) / 120.0;
            self.change(hit.row, Change::Scroll(notches), requests);
        }
    }

    /// Takes the keyboard focus. The system may refuse the foreground; then the
    /// panel asks for a click instead of sitting there deaf to the keyboard.
    fn take_focus(&mut self) {
        self.overlay.activate();
        if unsafe { GetForegroundWindow() } != self.overlay.hwnd() {
            self.focused = false;
            self.awaiting.set(true);
            let _ = unsafe { SetCapture(self.overlay.hwnd()) };
        }
    }

    /// Whether a loss of focus now is the shell's, not the user's, and the
    /// panel should take the focus back instead of closing. Counts the tries.
    fn lost_focus_early(&mut self) -> bool {
        if self.opened_at.elapsed() < EARLY && self.regrabs < MAX_REGRABS {
            self.regrabs += 1;
            true
        } else {
            false
        }
    }

    /// Draws the rows and pushes them to the screen, at the height they need
    /// now; the window grows or shrinks on the edge it is anchored by.
    fn repaint(&mut self) -> Result<()> {
        // Only keeps the position within what the list can now scroll: a
        // refresh must not bring the selection back into view, which would undo
        // a scroll made with the scrollbar. Moving the selection does that.
        self.scroll = self.layout().paged_to(self.scroll);
        self.paint()?;

        let size = self.layout().size();
        let old = self.shown.get();
        if size != old {
            if self.grow_up {
                self.position.y += old.1 - size.1;
            }
            self.position = self.monitor.keep_inside(self.position, size);
            self.shown.set(size);
            self.overlay.show_at_size(self.position, size)
        } else {
            self.overlay.refresh_size(size)
        }
    }

    fn paint(&mut self) -> Result<()> {
        let layout = self.layout();
        self.resolve_looks();

        let dark = theme::is_dark();
        let palette = Palette::current(Kind::Master, false, &self.colors);
        let panel = palette.panel;
        let highlight = if dark {
            Color::rgb(0x3A, 0x3A, 0x3A)
        } else {
            Color::rgb(0xE2, 0xE2, 0xE2)
        };
        let looks = &self.looks;
        let canvas = self.overlay.canvas_mut();
        canvas.clear(panel);

        let first = layout.scroll;
        let last = layout.scroll + layout.visible();

        // The selection first: the text over it needs an opaque ground.
        if let Some(rect) = self
            .rows
            .iter()
            .position(|row| row.id == self.selected)
            .and_then(|index| layout.row_rect(index))
        {
            let inset = RECT {
                left: self.metrics.pad / 2,
                right: rect.right - self.metrics.pad / 2,
                ..rect
            };
            canvas.fill_rect(inset, highlight);
        }

        // The line between the microphone and the applications.
        if first <= 1
            && last > 2
            && let Some(rect) = layout.row_rect(1)
        {
            let y = rect.bottom + self.metrics.separator / 2;
            let line = if dark {
                Color::rgb(0x55, 0x55, 0x55)
            } else {
                Color::rgb(0xC8, 0xC8, 0xC8)
            };
            canvas.fill_rect(
                RECT {
                    left: self.metrics.pad,
                    top: y,
                    right: self.metrics.width - self.metrics.pad,
                    bottom: y + 1,
                },
                line,
            );
        }

        for index in first..last {
            let Some(row) = self.rows.get(index) else {
                break;
            };
            draw_row(
                canvas,
                &RowStyle {
                    font: &self.font,
                    bar_height: self.bar_height,
                    colors: &self.colors,
                },
                &layout,
                index,
                row,
                row.app.as_ref().and_then(|app| looks.get(&look_key(app))),
            )?;
        }

        // The scrollbar, when there is more than the rows shown: the thumb says
        // how much, and where. Vertical capsules, as the bars are horizontal.
        if let (Some(track), Some((thumb_top, thumb_bottom))) = (layout.track_rect(), layout.thumb())
        {
            let (width, height) = (track.right - track.left, track.bottom - track.top);
            hud::paint(canvas, track.left, track.top, width, height, palette.track, |x, y| {
                hud::in_capsule(y, x, height as f32, width as f32)
            });
            let length = thumb_bottom - thumb_top;
            hud::paint(canvas, track.left, thumb_top, width, length, palette.fill, |x, y| {
                hud::in_capsule(y, x, length as f32, width as f32)
            });
        }

        canvas.round_corners(self.radius);
        Ok(())
    }

    /// Resolves the names and icons of the application rows that have none.
    fn resolve_looks(&mut self) {
        for row in &self.rows {
            if let Some(app) = row.app.as_ref() {
                self.looks
                    .entry(look_key(app))
                    .or_insert_with(|| Look::resolve(app, self.metrics.icon));
            }
        }
    }
}

fn look_key(app: &AppRef) -> String {
    app.exe
        .clone()
        .unwrap_or_else(|| format!("name:{}", app.name))
}

/// The text of the row's name.
fn row_name<'a>(row: &'a Row, look: Option<&'a Look>) -> &'a str {
    match row.id {
        RowId::Master => "Master",
        RowId::Mic => "Microphone",
        RowId::App(_) => look.map_or("Application", |look| look.name.as_str()),
    }
}

/// The colour of a muted row's text: the text's, a half of the way to the
/// panel's.
fn dimmed(text: Color, panel: Color) -> Color {
    let half = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
    Color::rgb(half(text.r, panel.r), half(text.g, panel.g), half(text.b, panel.b))
}

/// What every row is drawn with.
#[derive(Clone, Copy)]
struct RowStyle<'a> {
    font: &'a Font,
    bar_height: i32,
    colors: &'a Colors,
}

fn draw_row(
    canvas: &mut Canvas,
    style: &RowStyle<'_>,
    layout: &Layout,
    index: usize,
    row: &Row,
    look: Option<&Look>,
) -> Result<()> {
    let (Some(row_rect), Some(icon), Some(name), Some(percent)) = (
        layout.row_rect(index),
        layout.icon_rect(index),
        layout.name_rect(index),
        layout.percent_rect(index),
    ) else {
        return Ok(());
    };

    let state = row.state;
    let muted = state.is_none_or(|state| state.muted);
    let level = state.map_or(0.0, |state: State| state.level);
    let RowStyle {
        font,
        bar_height,
        colors,
    } = *style;
    let palette = Palette::current(row.kind, muted, colors);
    let text_colour = if muted {
        dimmed(palette.text, palette.panel)
    } else {
        palette.text
    };

    // Name.
    canvas.draw_text_ellipsis(
        row_name(row, look),
        name,
        font,
        text_colour,
        TextAlign::Left,
    )?;

    // Icon: the application's own, else the glyph of its kind.
    let side = icon.right - icon.left;
    match look.and_then(|look| look.pixels.as_deref().map(|pixels| (look.size, pixels))) {
        Some((size, pixels)) if size == side => {
            canvas.blit(icon.left, icon.top, size, size, pixels);
        }
        _ => {
            let waves = hud::waves(level, muted);
            let kind = row.kind;
            let edge = side as f32;
            hud::paint(canvas, icon.left, icon.top, side, side, palette.glyph, |x, y| {
                hud::in_glyph(kind, x / edge, y / edge, waves, muted)
            });
        }
    }

    // The bar and the percentage, or what is missing.
    let (bar_left, bar_right) = layout.bar_span();
    let Some(state) = state else {
        let missing = RECT {
            left: bar_left,
            right: percent.right,
            ..row_rect
        };
        let text = match row.kind {
            Kind::Mic => "No microphone",
            Kind::Master => "No audio output",
            Kind::App => "No audio",
        };
        return canvas.draw_text(text, missing, font, text_colour, TextAlign::Left);
    };

    let bar_width = bar_right - bar_left;
    let top = row_rect.top + (row_rect.bottom - row_rect.top - bar_height) / 2;
    hud::paint(canvas, bar_left, top, bar_width, bar_height, palette.track, |x, y| {
        hud::in_capsule(x, y, bar_width as f32, bar_height as f32)
    });
    let filled = hud::fill_width(level, bar_width, bar_height);
    if filled > 0 {
        hud::paint(canvas, bar_left, top, filled, bar_height, palette.fill, |x, y| {
            hud::in_capsule(x, y, filled as f32, bar_height as f32)
        });
    }

    let text = if state.muted {
        "Muted".to_owned()
    } else {
        format!("{}%", model::to_percent(state.level))
    };
    canvas.draw_text(&text, percent, font, text_colour, TextAlign::Right)
}

/// The panel's window procedure: turns the few messages that matter into
/// posts to the owner. See the module doc.
struct Forwarder {
    owner: HWND,
    generation: usize,
    /// The size shown now, to tell a click outside (while the mouse is
    /// captured) from one on the panel.
    size: Rc<Cell<(i32, i32)>>,
    /// True while the panel waits for a click to take the focus.
    awaiting: Rc<Cell<bool>>,
    /// The click that took the focus must not also act on a row.
    swallow_release: bool,
}

impl Forwarder {
    fn post(&self, msg: u32, lparam: isize) {
        let _ = unsafe {
            PostMessageW(
                Some(self.owner),
                msg,
                WPARAM(self.generation),
                LPARAM(lparam),
            )
        };
    }

    fn shift_held() -> isize {
        if unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0 {
            MOD_SHIFT
        } else {
            0
        }
    }
}

impl WindowHandler for Forwarder {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_KEYDOWN => {
                self.post(
                    WM_PANEL_KEY,
                    (wparam.0 as isize & 0xFFFF) | (Self::shift_held() << 16),
                );
                Some(LRESULT(0))
            }
            // Alt+letter has no menu to open here; swallowing the character it
            // makes spares a beep.
            WM_SYSCHAR => Some(LRESULT(0)),
            // Alt held: ours to read, not to swallow (Alt+F4 keeps its meaning).
            WM_SYSKEYDOWN => {
                self.post(
                    WM_PANEL_KEY,
                    (wparam.0 as isize & 0xFFFF) | (Self::shift_held() << 16),
                );
                None
            }
            // The volume keys, when Windows turned them into commands.
            WM_APPCOMMAND => {
                let command = ((lparam.0 >> 16) & 0x0FFF) as u32;
                let key = match command {
                    8 => VK_VOLUME_MUTE,
                    9 => VK_VOLUME_DOWN,
                    10 => VK_VOLUME_UP,
                    _ => return None,
                };
                self.post(WM_PANEL_KEY, key.0 as isize);
                Some(LRESULT(1))
            }

            // Focus lost: a click outside, Alt+Tab, another window in front.
            WM_KILLFOCUS => {
                self.post(WM_PANEL_CLOSE, 0);
                Some(LRESULT(0))
            }
            WM_SETFOCUS => {
                self.awaiting.set(false);
                self.post(WM_PANEL_FOCUSED, 0);
                None
            }

            // While waiting for a click to take the focus, the mouse is captured:
            // a click on the panel takes it, one outside closes.
            WM_LBUTTONDOWN if self.awaiting.get() && unsafe { GetCapture() } == hwnd => {
                let (x, y) = (
                    (lparam.0 & 0xFFFF) as i16 as i32,
                    ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                );
                let (width, height) = self.size.get();
                let _ = unsafe { ReleaseCapture() };
                self.awaiting.set(false);
                if (0..width).contains(&x) && (0..height).contains(&y) {
                    self.swallow_release = true;
                    let _ = unsafe { SetForegroundWindow(hwnd) };
                    let _ = unsafe { SetFocus(Some(hwnd)) };
                } else {
                    self.post(WM_PANEL_CLOSE, 0);
                }
                Some(LRESULT(0))
            }

            // The button is captured for the length of a press, so a drag that
            // leaves the panel keeps reporting.
            WM_LBUTTONDOWN => {
                let _ = unsafe { SetCapture(hwnd) };
                self.post(WM_PANEL_DOWN, lparam.0);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                if unsafe { GetCapture() } == hwnd {
                    self.post(WM_PANEL_MOVE, lparam.0);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if unsafe { GetCapture() } == hwnd {
                    let _ = unsafe { ReleaseCapture() };
                }
                if std::mem::take(&mut self.swallow_release) {
                    return Some(LRESULT(0));
                }
                self.post(WM_PANEL_UP, lparam.0);
                Some(LRESULT(0))
            }
            WM_MBUTTONUP => {
                self.post(WM_PANEL_MIDDLE, lparam.0);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                // The position of a wheel message is the screen's.
                let mut at = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let _ = unsafe { ScreenToClient(hwnd, &mut at) };
                let delta = (wparam.0 >> 16) as i16;
                let packed = (at.x as i16 as u16 as isize)
                    | ((at.y as i16 as u16 as isize) << 16)
                    | (isize::from(delta) << 32);
                self.post(WM_PANEL_WHEEL, packed);
                Some(LRESULT(0))
            }

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_point_reads_back_with_its_signs() {
        let packed = (-5i16 as u16 as isize) | ((12i16 as u16 as isize) << 16);
        assert_eq!(point_of(LPARAM(packed)), (-5, 12));
    }

    #[test]
    fn the_wheel_delta_rides_above_the_point() {
        let packed = (7i16 as u16 as isize) | ((9i16 as u16 as isize) << 16) | (-120isize << 32);
        assert_eq!(point_of(LPARAM(packed)), (7, 9));
        assert_eq!((packed >> 32) as i16, -120);
    }

    #[test]
    fn dimming_goes_halfway_to_the_panel() {
        let text = Color::rgb(200, 100, 0);
        let panel = Color::rgb(0, 100, 200);
        assert_eq!(dimmed(text, panel), Color::rgb(100, 100, 100));
    }

    #[test]
    fn only_the_panels_messages_are_its_own() {
        assert!(Panel::is_message(WM_PANEL_KEY));
        assert!(Panel::is_message(WM_PANEL_FOCUSED));
        assert!(!Panel::is_message(WM_APP_FIRST));
        assert!(!Panel::is_message(WM_APP_FIRST + 24));
    }
}
