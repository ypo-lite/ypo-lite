//! The search window: a focusable overlay that the search hotkey opens
//! (`docs/search-plan.md`, phase 4; the design is `docs/overlay-modes.md` §2).
//!
//! # Shape
//!
//! Same as `y-snap`'s action overlay: the window's own handler
//! ([`Forwarder`]) holds no state worth the name and **posts private
//! messages to the owner window** (the tray's hidden window), which owns the
//! [`SearchWindow`] and does the work. The overlay is created per opening and
//! dropped on close, like the native popup, so nothing stays committed
//! between uses. The behaviour itself — what a key does to the text, the
//! results and the selection — is `y_pop::search::state`, and the text of a
//! row is `y_pop::search::display`: pure and tested. This file reads the
//! keyboard, the mouse and the clipboard, and paints.
//!
//! Every message the handler posts carries the opening's **generation** in
//! `wParam`. Closing a focused window sends it a `WM_KILLFOCUS` synchronously,
//! which posts a close request that only arrives after the call returns; by
//! then a quick hotkey may have opened a new window, and the stale request
//! would close that one. The generation tells them apart.
//!
//! # Keyboard focus
//!
//! A hotkey grants the process the right to take the foreground, so
//! [`Overlay::activate`] normally works. When it does not (a hotkey claimed
//! through the low-level hook fallback gives no such right), the window asks
//! for a click: it captures the mouse, a click on it takes the focus, and a
//! click outside closes it, which is what `WM_KILLFOCUS` would have meant had
//! it ever had the focus.
//!
//! # Size
//!
//! The list grows and shrinks with the results, but a layered window's
//! surface is fixed when it is created. So the overlay is created at the
//! tallest the window can be (`layout::max_height`) and shown at the height
//! in use ([`Overlay::refresh_size`]); the part of the surface below is never
//! shown.
//!
//! # Icons
//!
//! Loaded synchronously, for the rows on screen only, and kept as pixels for
//! the life of the window. Async loading is the native popup's roadmap step 7;
//! it is not needed until a slow icon is seen.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, PostMessageW, SetForegroundWindow, WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_SETFOCUS, WM_SYSKEYDOWN,
};

use y_core::{
    Result, clipboard,
    icon::IconCache,
    ids::WM_APP_FIRST,
    monitor::Monitor,
    overlay::{Canvas, Color, Font, Overlay, OverlaySpec, TextAlign},
    window::WindowHandler,
};

use y_pop::model::{ConfigTree, NodeId, ResultsLayout};
use y_pop::search::display;
use y_pop::search::index::Index;
use y_pop::search::layout::{self, Hit, Layout, Placed, Spec};
use y_pop::search::query;
use y_pop::search::state::{Event, Key, Mods, Outcome, Row, State};
use y_pop::skins;

/// Posted to the owner when the window should close (focus lost, a click
/// outside). `wParam` is the generation.
pub const WM_SEARCH_CLOSE: u32 = WM_APP_FIRST + 0x10;

/// Posted to the owner when the window got the keyboard focus after having
/// asked for a click. `wParam` is the generation.
pub const WM_SEARCH_FOCUSED: u32 = WM_APP_FIRST + 0x11;

/// A typed character. `lParam` is one UTF-16 unit (a character outside the
/// Basic Multilingual Plane arrives as two of these).
pub const WM_SEARCH_CHAR: u32 = WM_APP_FIRST + 0x12;

/// A key went down. `lParam` is the virtual-key code, with the modifiers in
/// the bits above it ([`MOD_CTRL`], [`MOD_SHIFT`], [`MOD_ALT`], shifted left
/// 16).
pub const WM_SEARCH_KEY: u32 = WM_APP_FIRST + 0x13;

/// The pointer moved. `lParam` is the window's client position, as the
/// original mouse message had it.
pub const WM_SEARCH_MOVE: u32 = WM_APP_FIRST + 0x14;

/// A click (the button went up). Same `lParam`.
pub const WM_SEARCH_CLICK: u32 = WM_APP_FIRST + 0x15;

/// The wheel turned. `lParam` is the signed delta, 120 per notch, positive
/// away from the user (up).
pub const WM_SEARCH_WHEEL: u32 = WM_APP_FIRST + 0x16;

const MOD_CTRL: isize = 1;
const MOD_SHIFT: isize = 2;
const MOD_ALT: isize = 4;

const CLASS: &str = "YpoLite.YPop.Search";
const FONT_FAMILY: &str = "Segoe UI";
/// Search glyph and the like: the icon font of Windows 10 and 11.
const GLYPH_FAMILY: &str = "Segoe MDL2 Assets";
const GLYPH_SEARCH: &str = "\u{E721}";
// Logical pixels, like the layout's.
const INPUT_FONT: i32 = 18;
const TITLE_FONT: i32 = 15;
const SMALL_FONT: i32 = 12;
const GLYPH_FONT: i32 = 16;
const ICON_SIZE: i32 = 32;
/// The window's top edge, as a share of the work area's height: the upper
/// third, a little above where the eye rests.
const TOP_SHARE: f32 = 0.18;
/// The list may take this share of the work area's height, input row included.
const MAX_HEIGHT_SHARE: f32 = 0.6;
const CORNER_RADIUS: i32 = 8;
/// The selected row's inset from the window's sides, and its corner radius.
const ROW_MARGIN: i32 = 6;
const ROW_RADIUS: i32 = 6;
const SCROLLBAR_WIDTH: i32 = 3;
/// How many rows one notch of the wheel scrolls.
const WHEEL_ROWS: i32 = 3;
/// The selection highlight in the input: this much of the accent over the panel.
const ACCENT: Color = Color::rgb(0x00, 0x78, 0xD4);

const PLACEHOLDER: &str = "Search apps, menus, hotkeys…";
const PLACEHOLDER_UNFOCUSED: &str = "Click here, then type to search";
const FOOTER: &str = "↑↓  Move      ↵  Open      Tab  Into group      Esc  Back";

/// What the owner keeps: nothing while closed, the open window otherwise.
pub struct SearchWindow {
    owner: HWND,
    open: Option<Open>,
    /// Bumped by every opening; see the module doc.
    generation: usize,
    /// The node the user picked, closed over and waiting for the owner to
    /// act on it ([`SearchWindow::take_activation`]).
    activation: Option<NodeId>,
}

struct Open {
    overlay: Overlay,
    layout: Layout,
    spec: Spec,
    /// The first row of the list that is shown.
    scroll: usize,
    /// The size of the part of the surface in use, shared with the handler.
    visible: Rc<Cell<(i32, i32)>>,
    palette: Palette,
    fonts: Fonts,
    /// The corner radius, border width and caret width in physical pixels.
    radius: i32,
    border: i32,
    caret_width: i32,
    /// False while waiting for a click to take the focus.
    focused: bool,
    /// What can be searched. Built for each opening from the tree as it is
    /// now, and dropped with the window.
    index: Index,
    state: State,
    icons: IconCache,
    icon_size: i32,
    /// Icons already loaded, as pixels, by index entry.
    pixels: HashMap<u32, Option<Vec<u32>>>,
    /// The first half of a character outside the Basic Multilingual Plane,
    /// waiting for its second.
    high_surrogate: Option<u16>,
}

struct Fonts {
    input: Font,
    title: Font,
    small: Font,
    heading: Font,
    glyph: Font,
}

impl SearchWindow {
    pub fn new(owner: HWND) -> Self {
        Self {
            owner,
            open: None,
            generation: 0,
            activation: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The node the user chose with Enter or a click, once, after the window
    /// has closed. The owner runs it like any other pick from a menu: an app
    /// is launched, `<quit/>` quits, `<settings/>` opens the settings.
    /// Closing first matters for launching: the window gives the foreground
    /// back to what had it, which the new program can then take.
    pub fn take_activation(&mut self) -> Option<NodeId> {
        self.activation.take()
    }

    /// The hotkey: opens the window, or closes it if it is already open.
    pub fn toggle(&mut self, tree: &ConfigTree) -> Result<()> {
        if self.is_open() {
            self.close();
            Ok(())
        } else {
            self.open(tree)
        }
    }

    /// Opens the window on the monitor the cursor is on, themed like the
    /// tray menus. A no-op while already open.
    pub fn open(&mut self, tree: &ConfigTree) -> Result<()> {
        if self.is_open() {
            return Ok(());
        }
        self.generation += 1;

        let monitor = Monitor::from_cursor().unwrap_or_else(|_| Monitor::primary());
        let area = monitor.work_area;
        let (area_width, area_height) = (area.right - area.left, area.bottom - area.top);

        let search = tree.search.as_ref();
        let spec = Spec {
            width: layout::width_for(area_width, monitor.scale()),
            scale: monitor.scale(),
            max_list_height: (area_height as f32 * MAX_HEIGHT_SHARE) as i32,
            footer: !search.is_some_and(|search| search.collapsed),
        };
        let layout = Layout::new(&spec, &[], 0);
        let visible = Rc::new(Cell::new(layout.size));

        let handler = Forwarder {
            owner: self.owner,
            generation: self.generation,
            size: Rc::clone(&visible),
            swallow_release: false,
        };
        let overlay_spec = OverlaySpec::new(CLASS, (spec.width, layout::max_height(&spec)))
            .interactive()
            .focusable()
            .with_owner(self.owner);
        let overlay = Overlay::with_handler(&overlay_spec, Box::new(handler))?;

        let design = skins::resolve_design(tree.defaults.theme, tree.defaults.theme_overrides);
        let scaled = |logical: i32| monitor.scaled(logical);
        let icon_size = scaled(ICON_SIZE);
        let mut open = Open {
            palette: Palette::of(design.as_ref()),
            radius: scaled(
                design
                    .as_ref()
                    .map_or(CORNER_RADIUS, |design| design.border.round as i32),
            ),
            border: scaled(
                design
                    .as_ref()
                    .map_or(1, |design| design.border.size as i32),
            ),
            caret_width: scaled(1).max(2),
            fonts: Fonts {
                input: Font::new(FONT_FAMILY, scaled(INPUT_FONT), false)?,
                title: Font::new(FONT_FAMILY, scaled(TITLE_FONT), false)?,
                small: Font::new(FONT_FAMILY, scaled(SMALL_FONT), false)?,
                heading: Font::new(FONT_FAMILY, scaled(SMALL_FONT - 1), true)?,
                glyph: Font::new(GLYPH_FAMILY, scaled(GLYPH_FONT), false)?,
            },
            overlay,
            layout,
            spec,
            scroll: 0,
            visible,
            focused: true,
            index: Index::from_tree(tree),
            state: State::new(search.map_or(ResultsLayout::Sections, |search| search.results)),
            icons: IconCache::with_size(icon_size),
            icon_size,
            pixels: HashMap::new(),
            high_surrogate: None,
        };

        // The upper part of the monitor, centred. Placed for the tallest the
        // window gets, so that it can grow without leaving the screen.
        let size = open.layout.size;
        let position = monitor.keep_inside(
            POINT {
                x: area.left + (area_width - size.0) / 2,
                y: area.top + (area_height as f32 * TOP_SHARE) as i32,
            },
            (size.0, layout::max_height(&open.spec)),
        );

        open.paint(tree)?;
        open.overlay.show_at_size(position, size)?;
        open.overlay.activate();

        // If the system refused the foreground, ask for a click instead of
        // sitting there deaf to the keyboard.
        if unsafe { GetForegroundWindow() } != open.overlay.hwnd() {
            open.focused = false;
            open.paint(tree)?;
            open.overlay.refresh_size(size)?;
            let _ = unsafe { SetCapture(open.overlay.hwnd()) };
        }

        self.open = Some(open);
        Ok(())
    }

    /// Closes the window without acting. Safe when already closed.
    pub fn close(&mut self) {
        // Dropping the overlay destroys its window and frees its pixels.
        self.open = None;
    }

    /// The owner's message hook: `true` if `msg` was one of ours. `tree` is
    /// the tree the window was opened on (a reload closes it first).
    pub fn on_message(
        &mut self,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        tree: &ConfigTree,
    ) -> bool {
        if !(WM_SEARCH_CLOSE..=WM_SEARCH_WHEEL).contains(&msg) {
            return false;
        }
        // A message from a window that is already gone.
        if wparam.0 != self.generation {
            return true;
        }

        match msg {
            WM_SEARCH_CLOSE => self.close(),
            WM_SEARCH_FOCUSED => {
                if let Some(open) = self.open.as_mut()
                    && !open.focused
                {
                    open.focused = true;
                    self.redraw(tree, false);
                }
            }
            WM_SEARCH_CHAR => {
                let event = self
                    .open
                    .as_mut()
                    .and_then(|open| open.character(lparam.0 as u16));
                if let Some(event) = event {
                    self.event(tree, event);
                }
            }
            WM_SEARCH_KEY => {
                let bits = lparam.0 >> 16;
                let mods = Mods {
                    ctrl: bits & MOD_CTRL != 0,
                    shift: bits & MOD_SHIFT != 0,
                    alt: bits & MOD_ALT != 0,
                };
                if let Some(key) = key_of(lparam.0 as u16) {
                    self.event(tree, Event::Key(key, mods));
                }
            }
            WM_SEARCH_MOVE => {
                if let Some(open) = self.open.as_mut()
                    && let Some(Hit::Row(row)) = open.hit(lparam)
                    && open.state.select_row(row)
                {
                    // Hover must not scroll the list under the pointer.
                    self.redraw(tree, false);
                }
            }
            WM_SEARCH_CLICK => {
                let outcome = self.open.as_mut().and_then(|open| match open.hit(lparam) {
                    Some(Hit::Row(row)) => Some(open.state.click_row(&open.index, row)),
                    _ => None,
                });
                if let Some(outcome) = outcome {
                    self.act(tree, outcome, false);
                }
            }
            WM_SEARCH_WHEEL => {
                if let Some(open) = self.open.as_mut() {
                    let notches = lparam.0 as i32 / 120;
                    let last = open.state.rows().len().saturating_sub(1) as i32;
                    let scroll = (open.scroll as i32 - notches * WHEEL_ROWS).clamp(0, last);
                    if scroll as usize != open.scroll {
                        open.scroll = scroll as usize;
                        self.redraw(tree, false);
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn event(&mut self, tree: &ConfigTree, event: Event) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let before = open.state.input().text().to_owned();
        let outcome = open.state.handle(&open.index, event);
        if open.state.input().text() != before {
            // A new search starts at the top.
            open.scroll = 0;
        }
        self.act(tree, outcome, true);
    }

    /// Does what `outcome` asks. `follow`: keep the selected row in view.
    fn act(&mut self, tree: &ConfigTree, outcome: Outcome, follow: bool) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw(tree, follow),
            Outcome::Close => self.close(),
            Outcome::Activate(hit) => {
                if let Some(open) = self.open.as_ref() {
                    self.activation = Some(open.index.entries()[hit.entry as usize].id);
                }
                self.close();
            }
            Outcome::Copy(text) => {
                clipboard::set_text(self.owner, &text);
                self.redraw(tree, follow);
            }
            Outcome::Paste => {
                if let Some(text) = clipboard::text() {
                    self.event(tree, Event::Paste(text));
                }
            }
        }
    }

    fn redraw(&mut self, tree: &ConfigTree, follow: bool) {
        if let Some(open) = self.open.as_mut() {
            open.relayout(follow);
            if open.paint(tree).is_ok() {
                let _ = open.overlay.refresh_size(open.layout.size);
            }
        }
    }
}

impl Open {
    /// A typed UTF-16 unit as a character, joining a surrogate pair's two
    /// halves. `None` for the first half, and for a stray second one.
    fn character(&mut self, unit: u16) -> Option<Event> {
        let high = self.high_surrogate.take();
        let decoded = match (high, unit) {
            (_, 0xD800..=0xDBFF) => {
                self.high_surrogate = Some(unit);
                return None;
            }
            (Some(high), 0xDC00..=0xDFFF) => char::decode_utf16([high, unit]).next()?.ok()?,
            (None, 0xDC00..=0xDFFF) => return None,
            (_, unit) => char::from_u32(unit as u32)?,
        };
        Some(Event::Char(decoded))
    }

    /// What is at the client position `lparam` of a mouse message.
    fn hit(&self, lparam: LPARAM) -> Option<Hit> {
        let x = (lparam.0 & 0xFFFF) as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
        self.layout.hit_test(x, y)
    }

    /// Lays the rows out again for the state as it is. `follow` scrolls the
    /// list to keep the selected row in view (after typing or a key, not
    /// after the wheel or hover).
    fn relayout(&mut self, follow: bool) {
        let kinds = self.state.row_kinds();
        if follow && let Some(selected) = self.state.selected() {
            self.scroll = layout::scroll_to_show(&self.spec, &kinds, self.scroll, selected);
        }
        self.layout = Layout::new(&self.spec, &kinds, self.scroll);
        self.scroll = self.layout.first;
        self.visible.set(self.layout.size);
    }

    /// Draws the whole window, in the order the canvas needs: fills, text and
    /// icons, corners last.
    fn paint(&mut self, tree: &ConfigTree) -> Result<()> {
        let Self {
            overlay,
            layout,
            palette,
            fonts,
            state,
            index,
            icons,
            icon_size,
            pixels,
            radius,
            border,
            caret_width,
            focused,
            spec,
            ..
        } = self;
        let palette = *palette;
        let Palette {
            back,
            text,
            dim,
            line,
            selection,
            ..
        } = palette;
        let padding = layout.input_parts().icon.left;
        let window = RECT {
            left: 0,
            top: 0,
            right: layout.size.0,
            bottom: layout.size.1,
        };
        let canvas = overlay.canvas_mut();
        canvas.clear(back);

        // --- The input row ---------------------------------------------
        let parts = layout.input_parts();
        let field = parts.field;
        let typed = state.input().text();
        let (_, text_height) = fonts.input.measure("Ag");
        let text_top = field.top + (field.height() - text_height) / 2;
        let scaled = |logical: i32| (logical as f32 * spec.scale).round() as i32;

        // A chip for each group entered; the text field starts after them.
        let (_, chip_text_height) = fonts.title.measure("Ag");
        let (chip_pad_x, chip_pad_y, chip_gap) = (scaled(8), scaled(3), scaled(6));
        let chips: Vec<(String, i32)> = state
            .scopes()
            .iter()
            .map(|&group| {
                let name = tree.node(group).name.to_string();
                let width = fonts.title.measure(&name).0 + 2 * chip_pad_x;
                (name, width)
            })
            .collect();
        let field_left = field.left + chips.iter().map(|(_, w)| w + chip_gap).sum::<i32>();

        // Scrolled left just enough to keep the caret in view.
        let offsets = fonts.input.caret_offsets(typed);
        let caret_x = offsets[state.input().caret().min(offsets.len() - 1)];
        let room = field.right - field_left;
        let origin = field_left - (caret_x + *caret_width - room).max(0);

        let selected = state.input().selection();
        if !selected.is_empty() {
            canvas.fill_rect(
                RECT {
                    left: origin + offsets[selected.start],
                    top: text_top,
                    right: origin + offsets[selected.end],
                    bottom: text_top + text_height,
                },
                selection,
            );
        }
        if typed.is_empty() {
            let placeholder = match (*focused, state.scopes().last()) {
                (false, _) => PLACEHOLDER_UNFOCUSED.to_owned(),
                (true, Some(&group)) => format!("Search in {}…", tree.node(group).name),
                (true, None) => PLACEHOLDER.to_owned(),
            };
            canvas.draw_text_ellipsis(
                &placeholder,
                RECT {
                    left: field_left,
                    ..rect(field)
                },
                &fonts.input,
                if *focused { dim } else { text },
                TextAlign::Left,
            )?;
        } else {
            canvas.draw_text(
                typed,
                RECT {
                    left: origin,
                    top: field.top,
                    right: field.right,
                    bottom: field.bottom,
                },
                &fonts.input,
                text,
                TextAlign::Left,
            )?;
        }

        // Text scrolled out of the field ran under the glyph and the chips:
        // cover that, then draw them.
        canvas.fill_rect(
            RECT {
                left: 0,
                top: layout.input.top,
                right: field_left,
                bottom: layout.input.bottom - 1,
            },
            back,
        );
        canvas.draw_text(
            GLYPH_SEARCH,
            rect(parts.icon),
            &fonts.glyph,
            dim,
            TextAlign::Centre,
        )?;
        let chip_back = blend(text, back, 0.9);
        let middle = (field.top + field.bottom) / 2;
        let mut chip_left = field.left;
        for (name, width) in &chips {
            let chip = RECT {
                left: chip_left,
                top: middle - chip_text_height / 2 - chip_pad_y,
                right: chip_left + width,
                bottom: middle + chip_text_height / 2 + chip_pad_y,
            };
            canvas.fill_rect(chip, chip_back);
            canvas.draw_text(name, chip, &fonts.title, text, TextAlign::Centre)?;
            canvas.round_rect_corners(chip, scaled(5), back);
            chip_left += width + chip_gap;
        }

        if *focused {
            canvas.draw_caret(origin + caret_x, text_top, text_height, *caret_width, text);
        }

        // --- The list --------------------------------------------------
        if let Some(list) = layout.list {
            canvas.fill_rect(
                RECT {
                    left: 0,
                    top: list.top - 1,
                    right: list.right,
                    bottom: list.top,
                },
                line,
            );
        }
        let rows = state.rows().to_vec();
        let mut painter = ItemPainter {
            palette: &palette,
            fonts,
            layout,
            tree,
            index,
            icons,
            pixels,
            icon_size: *icon_size,
            scale: spec.scale,
        };
        for placed in &layout.rows {
            match rows[placed.index] {
                Row::Header(heading) => {
                    canvas.draw_text_ellipsis(
                        display::heading(heading),
                        RECT {
                            left: placed.rect.left + padding,
                            top: placed.rect.top,
                            right: placed.rect.right - padding,
                            bottom: placed.rect.bottom,
                        },
                        &fonts.heading,
                        dim,
                        TextAlign::Left,
                    )?;
                }
                Row::Result(hit) => {
                    let is_selected = state.selected() == Some(placed.index);
                    painter.item(canvas, state, placed, hit, is_selected)?;
                }
            }
        }
        if let Some(list) = layout.list
            && (layout.more_above || layout.more_below)
        {
            scrollbar(canvas, layout, list, rows.len(), line, spec.scale);
        }

        // --- The footer ------------------------------------------------
        if let Some(footer) = layout.footer {
            canvas.fill_rect(
                RECT {
                    top: footer.top,
                    bottom: footer.top + 1,
                    ..rect(footer)
                },
                line,
            );
            canvas.draw_text_ellipsis(
                FOOTER,
                rect(footer.inset(padding, 0)),
                &fonts.small,
                dim,
                TextAlign::Left,
            )?;
        }

        // --- Border and corners, last ----------------------------------
        let border_color = palette.border;
        if *radius > 0 && *border > 0 {
            canvas.stroke_rect(window, border_color);
            canvas.round_rect_corners_with_border(window, *radius, *border, border_color);
        } else if *radius > 0 {
            canvas.round_rect_corners(window, *radius, Color::TRANSPARENT);
        } else if *border > 0 {
            canvas.stroke_rect(window, border_color);
        }
        Ok(())
    }
}

/// What painting a result row needs, borrowed from the window.
struct ItemPainter<'a> {
    palette: &'a Palette,
    fonts: &'a Fonts,
    layout: &'a Layout,
    tree: &'a ConfigTree,
    index: &'a Index,
    icons: &'a mut IconCache,
    pixels: &'a mut HashMap<u32, Option<Vec<u32>>>,
    icon_size: i32,
    scale: f32,
}

impl ItemPainter<'_> {
    fn item(
        &mut self,
        canvas: &mut Canvas,
        state: &mut State,
        placed: &Placed,
        hit: query::Hit,
        selected: bool,
    ) -> Result<()> {
        let palette = self.palette;
        let entry = &self.index.entries()[hit.entry as usize];
        let node = self.tree.node(entry.id);
        let row = placed.rect;
        let parts = self.layout.item_parts(row);
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;

        // On the selected row the name is a little dimmer than its matched
        // letters, which keep the full text colour.
        let (title, matched, detail, background) = if selected {
            let (back, text) = (palette.selected_back, palette.selected_text);
            (
                blend(text, back, 0.22),
                text,
                blend(text, back, 0.35),
                Some(back),
            )
        } else {
            (palette.text, palette.matched, palette.dim, None)
        };
        let margin = scaled(ROW_MARGIN);
        let plate = RECT {
            left: row.left + margin,
            top: row.top + scaled(2),
            right: row.right - margin,
            bottom: row.bottom - scaled(2),
        };
        if let Some(background) = background {
            canvas.fill_rect(plate, background);
        }

        // The icon, loaded the first time its row is shown.
        let size = self.icon_size;
        let icons = &mut *self.icons;
        let pixels = self.pixels.entry(hit.entry).or_insert_with(|| {
            crate::menu::resolve_icon(icons, node).and_then(|icon| icon.to_pixels(size))
        });
        if let Some(pixels) = pixels {
            canvas.blit(parts.icon.left, parts.icon.top, size, size, pixels);
        }

        // The hotkey, right aligned; the texts end before it.
        let mut right = parts.title.right;
        if let Some(shortcut) = crate::menu::shortcut_text(&self.tree.defaults, node) {
            let (width, _) = self.fonts.small.measure(&shortcut);
            canvas.draw_text(
                &shortcut,
                RECT {
                    left: right - width,
                    top: row.top,
                    right,
                    bottom: row.bottom,
                },
                &self.fonts.small,
                detail,
                TextAlign::Right,
            )?;
            right -= width + scaled(12);
        }

        // The name, the letters the query matched emphasized. A name too
        // long for its box is cut with an ellipsis instead, unemphasized.
        let label = node.name.as_ref();
        let title_box = RECT {
            left: parts.title.left,
            top: parts.title.top,
            right,
            bottom: parts.title.bottom,
        };
        let offsets = self.fonts.title.caret_offsets(label);
        let fits = offsets.last().copied().unwrap_or(0) <= title_box.right - title_box.left;
        if fits {
            let mut positions = Vec::new();
            state.highlight(self.index, hit, &mut positions);
            for run in display::runs(label, &positions) {
                canvas.draw_text(
                    &unit_slice(label, &run.units),
                    RECT {
                        left: title_box.left + offsets[run.units.start],
                        ..title_box
                    },
                    &self.fonts.title,
                    if run.matched { matched } else { title },
                    TextAlign::Left,
                )?;
            }
        } else {
            canvas.draw_text_ellipsis(
                label,
                title_box,
                &self.fonts.title,
                title,
                TextAlign::Left,
            )?;
        }

        // The second line.
        canvas.draw_text_ellipsis(
            &display::subtitle(self.tree, entry, hit.matched),
            RECT {
                left: parts.subtitle.left,
                top: parts.subtitle.top,
                right: parts.title.right,
                bottom: parts.subtitle.bottom,
            },
            &self.fonts.small,
            detail,
            TextAlign::Left,
        )?;

        // After the text: the plate's corners, toward the panel behind it.
        if background.is_some() {
            canvas.round_rect_corners(plate, scaled(ROW_RADIUS), palette.back);
        }
        Ok(())
    }
}

/// A thin thumb on the list's right edge: where the visible rows sit among
/// all of them.
fn scrollbar(
    canvas: &mut Canvas,
    layout: &Layout,
    list: layout::Rect,
    total: usize,
    color: Color,
    scale: f32,
) {
    if total == 0 {
        return;
    }
    let width = ((SCROLLBAR_WIDTH as f32 * scale).round() as i32).max(2);
    let track = list.height();
    let top = list.top + (track as usize * layout.first / total) as i32;
    let height = ((track as usize * (layout.end - layout.first) / total) as i32).max(width * 4);
    canvas.fill_rect(
        RECT {
            left: list.right - width - 2,
            top,
            right: list.right - 2,
            bottom: (top + height).min(list.bottom),
        },
        color,
    );
}

fn rect(rect: layout::Rect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

/// The part of `text` covering the UTF-16 units `units`.
fn unit_slice(text: &str, units: &Range<usize>) -> String {
    let mut unit = 0;
    let mut slice = String::new();
    for c in text.chars() {
        if unit >= units.start && unit < units.end {
            slice.push(c);
        }
        unit += c.len_utf16();
    }
    slice
}

/// The keys the box reacts to, by virtual-key code.
fn key_of(code: u16) -> Option<Key> {
    Some(match code {
        c if c == VK_UP.0 => Key::Up,
        c if c == VK_DOWN.0 => Key::Down,
        c if c == VK_PRIOR.0 => Key::PageUp,
        c if c == VK_NEXT.0 => Key::PageDown,
        c if c == VK_LEFT.0 => Key::Left,
        c if c == VK_RIGHT.0 => Key::Right,
        c if c == VK_HOME.0 => Key::Home,
        c if c == VK_END.0 => Key::End,
        c if c == VK_RETURN.0 => Key::Enter,
        c if c == VK_ESCAPE.0 => Key::Escape,
        c if c == VK_TAB.0 => Key::Tab,
        c if c == VK_BACK.0 => Key::Backspace,
        c if c == VK_DELETE.0 => Key::Delete,
        // Letters have the code of their upper-case ASCII.
        letter @ 0x41..=0x5A => Key::Letter(letter as u8 as char),
        _ => return None,
    })
}

/// The window's colours, from the tray menus' skin.
#[derive(Clone, Copy, Debug)]
struct Palette {
    back: Color,
    text: Color,
    /// Hints, second lines: the text colour part of the way to the background.
    dim: Color,
    border: Color,
    /// The rules between the input, the list and the footer, and the thumb.
    line: Color,
    /// Behind selected text in the input.
    selection: Color,
    /// The selected result row, and its text.
    selected_back: Color,
    selected_text: Color,
    /// The letters of a name that the query matched.
    matched: Color,
}

impl Palette {
    /// The skin's colours; without a skin (`theme="native"`), a flat panel
    /// that follows the system's light or dark setting.
    fn of(design: Option<&y_core::menu::MenuDesign>) -> Self {
        let (back, text, border) = match design {
            Some(design) => (design.base.back, design.base.text, design.border.color),
            None if y_core::theme::is_dark() => (
                Color::rgb(0x20, 0x20, 0x20),
                Color::rgb(0xFF, 0xFF, 0xFF),
                Color::rgb(0x4A, 0x4A, 0x4A),
            ),
            None => (
                Color::rgb(0xF9, 0xF9, 0xF9),
                Color::rgb(0x00, 0x00, 0x00),
                Color::rgb(0xC8, 0xC8, 0xC8),
            ),
        };
        let dark = luma(back) < 128;
        let (selected_back, selected_text) = match design {
            Some(design) => (design.selected.back, design.selected.text),
            None => (blend(back, ACCENT, if dark { 0.45 } else { 0.22 }), text),
        };
        Self {
            back,
            text,
            dim: blend(text, back, 0.45),
            border,
            line: blend(text, back, 0.85),
            selection: blend(back, ACCENT, 0.5),
            selected_back,
            selected_text,
            matched: if dark {
                Color::rgb(0x4C, 0xA3, 0xFF)
            } else {
                Color::rgb(0x00, 0x5A, 0xB4)
            },
        }
    }
}

/// `a` moved `amount` of the way toward `b` (`0.0` is `a`, `1.0` is `b`).
fn blend(a: Color, b: Color, amount: f32) -> Color {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color::rgb(channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b))
}

fn luma(color: Color) -> u32 {
    (color.r as u32 * 299 + color.g as u32 * 587 + color.b as u32 * 114) / 1000
}

/// The overlay's window procedure: turns the few messages that matter into
/// posts to the owner. See the module doc.
struct Forwarder {
    owner: HWND,
    generation: usize,
    /// The size of the window as shown now, to tell a click outside (while
    /// the mouse is captured) from one on it.
    size: Rc<Cell<(i32, i32)>>,
    /// The click that took the focus must not also be a click on a row.
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

    /// The modifier keys held now, as the bits [`WM_SEARCH_KEY`] carries.
    fn modifiers() -> isize {
        let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0 as i32) } < 0;
        (if down(VK_CONTROL) { MOD_CTRL } else { 0 })
            | (if down(VK_SHIFT) { MOD_SHIFT } else { 0 })
            | (if down(VK_MENU) { MOD_ALT } else { 0 })
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
                    WM_SEARCH_KEY,
                    wparam.0 as isize & 0xFFFF | Self::modifiers() << 16,
                );
                Some(LRESULT(0))
            }
            // Alt held: also ours to read, but not to swallow — Alt+F4 and
            // the like still have their system meaning.
            WM_SYSKEYDOWN => {
                self.post(
                    WM_SEARCH_KEY,
                    wparam.0 as isize & 0xFFFF | Self::modifiers() << 16,
                );
                None
            }
            // Control characters (Backspace, Enter, Esc, Ctrl+letter) are
            // keys, which arrive above; only what is printable is typed.
            WM_CHAR => {
                if wparam.0 >= 0x20 {
                    self.post(WM_SEARCH_CHAR, wparam.0 as isize);
                }
                Some(LRESULT(0))
            }

            // Focus lost: a click outside, Alt+Tab, another window coming to
            // the front. Closes the window, as Escape on an empty box does.
            WM_KILLFOCUS => {
                self.post(WM_SEARCH_CLOSE, 0);
                Some(LRESULT(0))
            }
            WM_SETFOCUS => {
                self.post(WM_SEARCH_FOCUSED, 0);
                None
            }

            // The mouse is captured only while the window is waiting for a
            // click to take the focus (see the module doc).
            WM_LBUTTONDOWN if unsafe { GetCapture() } == hwnd => {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let (width, height) = self.size.get();
                let _ = unsafe { ReleaseCapture() };
                if (0..width).contains(&x) && (0..height).contains(&y) {
                    self.swallow_release = true;
                    let _ = unsafe { SetForegroundWindow(hwnd) };
                    let _ = unsafe { SetFocus(Some(hwnd)) };
                } else {
                    self.post(WM_SEARCH_CLOSE, 0);
                }
                Some(LRESULT(0))
            }

            WM_MOUSEMOVE => {
                self.post(WM_SEARCH_MOVE, lparam.0);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if std::mem::take(&mut self.swallow_release) {
                    return Some(LRESULT(0));
                }
                self.post(WM_SEARCH_CLICK, lparam.0);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = (wparam.0 >> 16) as i16 as isize;
                self.post(WM_SEARCH_WHEEL, delta);
                Some(LRESULT(0))
            }

            _ => None,
        }
    }
}
