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
//!
//! # Files
//!
//! The files of `/` are listed by a thread ([`y_pop::search::files`]), started
//! by the first `/` of an opening. It only ever posts [`WM_SEARCH_FILES`], with
//! the generation like the rest, and what it found is taken here. A browsed
//! folder a column of the tray menus goes into gets a thread of its own the same
//! way, posting [`WM_SEARCH_LISTED`].
//!
//! # Columns
//!
//! The Miller columns ([`y_pop::search::columns`]) are drawn in place of the rows,
//! at a fixed height. The focused column *is* `State`'s rows, so the keys and the
//! action menu need nothing here. Simplification: the wheel scrolls the focused
//! column wherever the pointer is.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VIRTUAL_KEY, VK_APPS, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F10, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, PostMessageW, SetForegroundWindow, WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SETFOCUS,
    WM_SYSCHAR, WM_SYSKEYDOWN,
};

use y_core::{
    Result, clipboard,
    hotkey::{Fallback, Hotkey, HotkeyManager},
    icon::IconCache,
    ids::WM_APP_FIRST,
    monitor::Monitor,
    overlay::{Canvas, Color, Overlay, OverlaySpec, TextAlign},
    window::WindowHandler,
};

use y_pop::model::{ActionId, ActionsConfig, App, ConfigTree, NodeId, ResultsLayout, Section};
use y_pop::search::columns::{Columns, PaneView};
use y_pop::search::commands::{Command, Line, PREFIXES};
use y_pop::search::display::{self, LineContext};
use y_pop::search::files::Files;
use y_pop::search::history::{self, AppNames, Item};
use y_pop::search::hotkey::KeyCombo;
use y_pop::search::index::{Entry, Index, Target};
use y_pop::search::keycheck::{self, Availability};
use y_pop::search::layout::{self, ColumnsLayout, Hit, Layout, PaneSpec, Placed, Spec};
use y_pop::search::look::{
    self, Fonts, GLYPH_MORE, GLYPH_SEARCH, Palette, ROW_MARGIN, ROW_RADIUS, blend, rect,
};
use y_pop::search::matcher::plain;
use y_pop::search::query;
use y_pop::search::sections::{self, Place, SectionRow};
use y_pop::search::state::{Event, Key, Mods, Outcome, Row, State};
use y_pop::{ipc, skins};

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

/// A right click. `lParam` is the window's client position, as for a click.
pub const WM_SEARCH_CONTEXT: u32 = WM_APP_FIRST + 0x17;

/// The thread listing the files of `/` has found more
/// ([`y_pop::search::files`]).
pub const WM_SEARCH_FILES: u32 = WM_APP_FIRST + 0x18;

/// The folder a column went into has been listed
/// ([`y_pop::search::files::Files::list_column`]).
pub const WM_SEARCH_LISTED: u32 = WM_APP_FIRST + 0x19;

const MOD_CTRL: isize = 1;
const MOD_SHIFT: isize = 2;
const MOD_ALT: isize = 4;

const CLASS: &str = "YpoLite.YPop.Search";
/// Keys of a section row's icon in the pixel cache, which is otherwise keyed by
/// index entry: far above any entry number.
const SECTIONS_ICON: u32 = u32::MAX;
const SECTION_ICON_BASE: u32 = 1 << 31;
/// "Folder": the icon of a section row, when the system gives no icon.
const GLYPH_FOLDER: &str = "\u{E8B7}";
/// "ChevronRight" (›): the row opens something.
const GLYPH_ENTER: &str = "\u{E76C}";
/// "ChevronDown" (⌄): unfolds a collapsed box.
const GLYPH_CHEVRON: &str = "\u{E70D}";
/// "History": the row that opens everything Windows remembers.
const GLYPH_HISTORY: &str = "\u{E81C}";
/// The id of the hotkey registered for a moment to see whether a key is free.
const PROBE_ID: u32 = 0x7F01;
const ICON_SIZE: i32 = 32;
/// The window's top edge, as a share of the work area's height: the upper
/// third, a little above where the eye rests.
const TOP_SHARE: f32 = 0.18;
/// The list may take this share of the work area's height, input row included.
const MAX_HEIGHT_SHARE: f32 = 0.6;
const SCROLLBAR_WIDTH: i32 = 3;
/// How many rows one notch of the wheel scrolls.
const WHEEL_ROWS: i32 = 3;

const PLACEHOLDER: &str = "Search apps, menus, hotkeys…";
const PLACEHOLDER_UNFOCUSED: &str = "Click here, then type to search";

/// What the user picked in the window.
pub enum Pick {
    Node(NodeId),
    /// An item of Windows's history that no node stands for.
    History(Item),
    /// A file found by `/`, as the tray menu would run it.
    File(App),
}

/// What the owner keeps: nothing while closed, the open window otherwise.
pub struct SearchWindow {
    owner: HWND,
    open: Option<Open>,
    /// The Start menu's apps, kept between openings: reading them is slow.
    apps: AppNames,
    /// Bumped by every opening; see the module doc.
    generation: usize,
    /// What the user picked, closed over and waiting for the owner to act on
    /// it ([`SearchWindow::take_activation`]).
    activation: Option<(Pick, ActionId)>,
    /// The y-pop command the user ran with `>`, waiting for the owner
    /// ([`SearchWindow::take_command`]).
    command: Option<Command>,
}

struct Open {
    /// Who hears from the thread that lists the files of `/`, and as which
    /// opening.
    owner: HWND,
    generation: usize,
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
    /// What Windows remembers, as of this opening; [`Target::History`] and
    /// the Recent rows point into it. Empty with `history="false"`.
    items: Vec<Item>,
    /// The second lines of those items, as far as they were shown: one means
    /// reading a shortcut.
    details: HashMap<u32, String>,
    /// The files of `/`, listed the first time it is typed.
    files: Files,
    /// Whether Windows keeps recent files at all.
    tracks_files: bool,
    /// Whether y-pop starts with Windows, for the line in `>`; read once.
    startup: bool,
    /// The search hotkey as written, for the line in `#`.
    search_hotkey: Option<String>,
    /// What the system said of the combination `#` is checking.
    check: Option<(String, Availability)>,
    /// The row button under the pointer, if any: a [`Hit::Button`] or [`Hit::More`].
    hover: Option<Hit>,
    icons: IconCache,
    icon_size: i32,
    /// Icons already loaded, as pixels, by index entry.
    pixels: HashMap<u32, Option<Vec<u32>>>,
    /// The details pane's icon: a cache has one size, and that one is bigger
    /// than the rows'.
    big_icons: IconCache,
    /// Icons at another size than the rows': a column's, the details pane's.
    sized_pixels: HashMap<(u32, i32), Option<Vec<u32>>>,
    /// The panes the columns show, as of the last layout; empty out of them.
    panes: Vec<PaneView>,
    /// Which column has the keys, as of the last layout. Another one starts
    /// from its own scroll position, not this one's.
    column_depth: Option<usize>,
    /// The first half of a character outside the Basic Multilingual Plane,
    /// waiting for its second.
    high_surrogate: Option<u16>,
}

impl SearchWindow {
    /// `history`: the file wants the history, so its list of apps is read now,
    /// for the first opening not to wait for it.
    pub fn new(owner: HWND, history: bool) -> Self {
        let mut apps = AppNames::default();
        if history {
            apps.refresh();
        }
        Self {
            owner,
            open: None,
            apps,
            generation: 0,
            activation: None,
            command: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The node the user chose, and what to do with it, once, after the
    /// window has closed. The owner opens it like any other pick from a menu
    /// (an app is launched, `<quit/>` quits, `<settings/>` opens the
    /// settings), or runs it elevated, or shows it in Explorer. Closing first
    /// matters for launching: the window gives the foreground back to what
    /// had it, which the new program can then take.
    pub fn take_activation(&mut self) -> Option<(Pick, ActionId)> {
        self.activation.take()
    }

    /// The command chosen with `>`, once, after the window has closed.
    pub fn take_command(&mut self) -> Option<Command> {
        self.command.take()
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
            // Always, here: the surface is made for the tallest the window gets,
            // and `relayout` drops the footer while the list is folded.
            footer: true,
            band: false,
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

        let design = skins::resolve_search_design(tree);
        let scaled = |logical: i32| monitor.scaled(logical);
        let icon_size = scaled(ICON_SIZE);
        // What Windows remembers: it orders the results and fills Recent.
        let mut index = Index::from_tree(tree);
        let sections = sections::rows(tree, &index);
        let history_on = search.is_some_and(|search| search.history);
        let recent_count = search.filter(|_| history_on).and_then(|search| {
            search.sections.iter().find_map(|section| match section {
                Section::Recent { count } => Some(*count as usize),
                _ => None,
            })
        });
        let listing = if history_on {
            let listed = recent_count.is_some();
            let files = if listed {
                y_core::recent::files()
            } else {
                Vec::new()
            };
            history::listing(
                tree,
                &mut index,
                y_core::recent::launches(),
                files,
                self.apps.current(),
                listed,
                history::now(),
            )
        } else {
            history::Listing::default()
        };
        let mut state = State::new(
            search.map_or(ResultsLayout::Sections, |search| search.results),
            search.map_or_else(ActionsConfig::default, |search| search.actions.clone()),
        )
        .with_row_actions(search.is_none_or(|search| search.row_actions))
        .with_collapsed(search.is_some_and(|search| search.collapsed))
        .with_menus_browse(search.map_or_else(Default::default, |search| search.menus_browse()))
        .with_recent_shown(recent_count.unwrap_or(0));
        state.start(&index, listing.recent, sections, listing.bonuses);
        let files = Files::of(tree, &index);

        let (radius, border) = look::frame_of(design.as_ref());
        let mut open = Open {
            owner: self.owner,
            generation: self.generation,
            palette: Palette::of(design.as_ref()),
            radius: scaled(radius),
            border: scaled(border),
            caret_width: scaled(1).max(2),
            fonts: Fonts::new(monitor.scale())?,
            overlay,
            layout,
            spec,
            scroll: 0,
            visible,
            focused: true,
            index,
            state,
            items: listing.items,
            details: HashMap::new(),
            files,
            tracks_files: y_core::recent::tracks_files(),
            startup: y_core::startup::is_enabled(ipc::STARTUP_NAME),
            search_hotkey: tree
                .search_hotkey()
                .map(|(_, binding)| binding.hotkey.to_string()),
            check: None,
            hover: None,
            icons: IconCache::with_size(icon_size),
            icon_size,
            pixels: HashMap::new(),
            big_icons: IconCache::with_size(scaled(layout::DETAILS_ICON)),
            sized_pixels: HashMap::new(),
            panes: Vec::new(),
            column_depth: None,
            high_surrogate: None,
        };

        // The upper part of the monitor, centred. Placed for the tallest the
        // window gets, so that it can grow without leaving the screen.
        open.relayout(false);
        let size = open.layout.size;
        let position = monitor.keep_inside(
            POINT {
                x: area.left + (area_width - size.0) / 2,
                y: area.top + (area_height as f32 * TOP_SHARE) as i32,
            },
            (size.0, open.tallest()),
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
        if !(WM_SEARCH_CLOSE..=WM_SEARCH_LISTED).contains(&msg) {
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
                let code = lparam.0 as u16;
                // Shift+F10 is the keyboard's "context menu", like the Menu key.
                let key = if code == VK_F10.0 && mods.shift {
                    Some(Key::ContextMenu)
                } else {
                    key_of(code)
                };
                if let Some(key) = key {
                    self.event(tree, Event::Key(key, mods));
                }
            }
            WM_SEARCH_MOVE => {
                if let Some(open) = self.open.as_mut() {
                    let hit = open.hit(lparam);
                    let changed = if open.state.menu().is_some() {
                        // With the menu open, the pointer lights its lines
                        // and leaves the list and the buttons alone.
                        matches!(hit, Some(Hit::Menu(item)) if open.state.menu_select(item))
                    } else {
                        let button = hit.filter(|hit| {
                            matches!(
                                hit,
                                Hit::Button(_)
                                    | Hit::More
                                    | Hit::Prefix(_)
                                    | Hit::Letter(_)
                                    | Hit::Details(_)
                            )
                        });
                        let lit = std::mem::replace(&mut open.hover, button) != button;
                        matches!(hit, Some(Hit::Row(row)) if open.state.select_row(row)) || lit
                    };
                    if changed {
                        // Hover must not scroll the list under the pointer.
                        self.redraw(tree, false);
                    }
                }
            }
            WM_SEARCH_CLICK => {
                let outcome = self.open.as_mut().and_then(|open| {
                    let hit = open.hit(lparam);
                    if open.state.menu().is_some() {
                        return match hit {
                            Some(Hit::Menu(item)) => Some(open.state.menu_click(&open.index, item)),
                            // Between its lines: nothing; anywhere else, away
                            // with it, and the click is spent doing that.
                            Some(Hit::MenuPanel) => None,
                            _ => {
                                open.state.close_menu();
                                Some(Outcome::Redraw)
                            }
                        };
                    }
                    match hit {
                        Some(Hit::Row(row)) => Some(open.state.click_row(&open.index, row)),
                        Some(Hit::Pane(pane, row)) => {
                            Some(open.state.column_click(&open.index, pane, row))
                        }
                        Some(Hit::Details(n)) => Some(open.state.details_click(&open.index, n)),
                        Some(Hit::Letter(n)) => open.state.letter_click(n).map(|header| {
                            let kinds = open.state.row_kinds();
                            open.scroll = layout::scroll_to_top(&open.spec, &kinds, header);
                            Outcome::Redraw
                        }),
                        Some(Hit::Button(n)) => Some(open.state.button_click(&open.index, n)),
                        Some(Hit::Chevron) => Some(open.state.unfold(&open.index)),
                        Some(Hit::Prefix(n)) => Some(open.state.prefix_click(&open.index, n)),
                        Some(Hit::More) => Some(open.state.open_menu(&open.index)),
                        _ => None,
                    }
                });
                if let Some(outcome) = outcome {
                    self.act(tree, outcome, false);
                }
            }
            WM_SEARCH_CONTEXT => {
                let outcome = self.open.as_mut().map(|open| match open.hit(lparam) {
                    Some(Hit::Row(row)) => open.state.open_menu_at(&open.index, row),
                    Some(Hit::Pane(pane, row)) => {
                        open.state.column_select(&open.index, pane, row);
                        open.state.open_menu(&open.index)
                    }
                    // A right click on anything else puts an open menu away.
                    _ if open.state.close_menu() => Outcome::Redraw,
                    _ => Outcome::Nothing,
                });
                if let Some(outcome) = outcome {
                    self.act(tree, outcome, true);
                }
            }
            WM_SEARCH_WHEEL => {
                if let Some(open) = self.open.as_mut() {
                    // The menu hangs from a row; scrolling would leave it
                    // hanging from nothing.
                    let closed = open.state.close_menu();
                    let notches = lparam.0 as i32 / 120;
                    let last = open.state.rows().len().saturating_sub(1) as i32;
                    let scroll = (open.scroll as i32 - notches * WHEEL_ROWS).clamp(0, last);
                    let scrolled = scroll as usize != open.scroll;
                    open.scroll = scroll as usize;
                    if closed || scrolled {
                        self.redraw(tree, false);
                    }
                }
            }
            WM_SEARCH_FILES if self.open.as_mut().is_some_and(Open::take_files) => {
                self.redraw(tree, true);
            }
            WM_SEARCH_LISTED if self.open.as_mut().is_some_and(Open::take_column) => {
                self.redraw(tree, true);
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
        open.hover = None;
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
            Outcome::Activate(hit, ActionId::Forget) => {
                if let Some(open) = self.open.as_mut()
                    && let Some(row) = open.state.recent_row(hit)
                {
                    // What Windows forgets, y-pop no longer lists: the two
                    // cannot disagree, as there is no second list.
                    if open.items[row.item as usize].forget() {
                        open.state.remove_recent(&open.index, hit.entry);
                    } else {
                        y_core::log::line("search: Windows did not forget that item");
                    }
                }
                self.redraw(tree, true);
            }
            Outcome::Activate(hit, action) => {
                if let Some(open) = self.open.as_mut() {
                    let entry = &open.index.entries()[hit.entry as usize];
                    let item = match entry.target {
                        Target::History(number) => Some(&open.items[number as usize]),
                        Target::Node(_) | Target::File(_) => None,
                    };
                    match action {
                        // Done here: the clipboard needs only a window of
                        // ours, and the tree says what the path is.
                        ActionId::CopyPath => {
                            let path = match (entry.target, item) {
                                (Target::File(number), _) => {
                                    Some(open.files.get(number).path.clone())
                                }
                                (_, Some(item)) => item.file().or_else(|| Some(item.address())),
                                (_, None) => display::target(tree, entry),
                            };
                            if let Some(path) = path {
                                clipboard::set_text(self.owner, &path);
                            }
                        }
                        _ => {
                            let pick = match (entry.target, item) {
                                (Target::Node(node), _) => Some(Pick::Node(node)),
                                (Target::File(number), _) => {
                                    Some(Pick::File(open.files.app(number)))
                                }
                                (_, Some(item)) => Some(Pick::History(item.clone())),
                                (_, None) => None,
                            };
                            self.activation = pick.map(|pick| (pick, action));
                        }
                    }
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
            Outcome::Command(command) => {
                self.command = Some(command);
                self.close();
            }
            Outcome::CheckKey => {
                let free = self
                    .open
                    .as_ref()
                    .and_then(|open| open.check.as_ref())
                    .is_some_and(|(_, availability)| *availability == Availability::Free);
                // Free: open the settings, to put the key somewhere.
                if free {
                    self.command = Some(Command::OpenSettings);
                    self.close();
                }
            }
            Outcome::ListFolder(entry) => {
                if let Some(open) = self.open.as_mut() {
                    open.list_column(tree, entry);
                }
                self.redraw(tree, follow);
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

    /// The tallest the window gets: the footer shown, the list full.
    fn tallest(&self) -> i32 {
        layout::max_height(&Spec {
            footer: true,
            ..self.spec
        })
    }

    /// The first `/` of this opening: sets the thread listing the folders off.
    /// It posts to the owner, as nothing else may touch the window from there.
    fn start_files(&mut self) {
        // A handle is a number to the thread, which only ever posts to it.
        let owner = self.owner.0 as isize;
        let generation = self.generation;
        self.files.start(move || {
            let owner = HWND(owner as *mut _);
            let _ = unsafe {
                PostMessageW(Some(owner), WM_SEARCH_FILES, WPARAM(generation), LPARAM(0))
            };
        });
        self.state.files_changed(&self.index, self.files.progress());
    }

    /// What the thread has found since last time joins the list. `true` if the
    /// list changed.
    fn take_files(&mut self) -> bool {
        let changed = self.files.take(&mut self.index);
        if changed {
            self.state.files_changed(&self.index, self.files.progress());
        }
        changed
    }

    /// A column opened on the browsed folder `entry`: lists it on a thread,
    /// which only ever posts to the owner.
    fn list_column(&mut self, tree: &ConfigTree, entry: u32) {
        let owner = self.owner.0 as isize;
        let generation = self.generation;
        let started = self.files.list_column(tree, &self.index, entry, move || {
            let owner = HWND(owner as *mut _);
            let _ = unsafe {
                PostMessageW(Some(owner), WM_SEARCH_LISTED, WPARAM(generation), LPARAM(0))
            };
        });
        if !started {
            self.state.column_listed(&self.index, entry, Vec::new());
        }
    }

    /// What the thread of [`Self::list_column`] found joins its column. `true`
    /// if there was anything.
    fn take_column(&mut self) -> bool {
        match self.files.take_column(&mut self.index) {
            Some((folder, items)) => {
                self.state.column_listed(&self.index, folder, items);
                true
            }
            None => false,
        }
    }

    /// The Miller columns, for the state as it is.
    fn columns_layout(&mut self, panes: Vec<PaneView>, depth: usize, follow: bool) -> Layout {
        if follow && let Some(selected) = self.state.selected() {
            self.scroll = layout::scroll_column(&self.spec, self.scroll, selected);
        }
        let specs: Vec<PaneSpec> = panes
            .iter()
            .map(|pane| PaneSpec {
                rows: pane.items.len(),
                selected: pane.selected,
            })
            .collect();
        let layout = Layout::columns(&self.spec, &specs, depth, self.scroll);
        self.scroll = layout.first;
        let details = panes.last().is_some_and(|pane| pane.details.is_some());
        self.panes = panes;
        if !details {
            return layout;
        }
        let widths: Vec<i32> = self
            .state
            .details_buttons(&self.index)
            .iter()
            .map(|&id| {
                let label = self.state.action_short_label(&self.index, id);
                self.fonts.small.measure(label).0
            })
            .collect();
        layout.with_details_buttons(&widths)
    }

    /// Lays the rows out again for the state as it is. `follow` scrolls the
    /// list to keep the selected row in view (after typing or a key, not
    /// after the wheel or hover).
    fn relayout(&mut self, follow: bool) {
        if self.state.wants_files() && !self.files.progress().started {
            self.start_files();
        }
        self.spec.footer = !self.state.folded();
        self.spec.band = !self.state.letters().is_empty();

        // Each column has its own scroll position, which is the one its
        // selection gives: the focus moving starts from there.
        let depth = self.state.column_depth();
        let moved = std::mem::replace(&mut self.column_depth, depth) != depth;
        if moved {
            self.scroll = 0;
        }
        let follow = follow || moved;
        let panes = self.state.column_panes(&self.index);
        if let (Some(panes), Some(depth)) = (panes, depth) {
            self.layout = self.columns_layout(panes, depth, follow);
            self.attach_menu();
            return;
        }
        self.panes.clear();

        let kinds = self.state.row_kinds();
        if follow && let Some(selected) = self.state.selected() {
            self.scroll = layout::scroll_to_show(&self.spec, &kinds, self.scroll, selected);
        }
        let mut layout = Layout::new(&self.spec, &kinds, self.scroll);
        self.scroll = layout.first;
        if self.state.folded() && self.state.collapsible() {
            layout = layout.with_chevron();
        }
        layout = layout.with_letters(self.state.letters().len());
        // The selected row's buttons, each as wide as its label.
        if let Some(selected) = self.state.selected()
            && self.state.has_row_buttons(&self.index)
        {
            let widths: Vec<i32> = self
                .state
                .row_buttons(&self.index)
                .iter()
                .map(|&id| {
                    let label = self.state.action_short_label(&self.index, id);
                    self.fonts.small.measure(label).0
                })
                .collect();
            layout = layout.with_row_buttons(selected, &widths);
        }
        // The line of prefixes, each chip as wide as its label.
        let chips: Vec<i32> = PREFIXES
            .iter()
            .map(|info| {
                self.fonts
                    .small
                    .measure(&chip_label(info.prefix, info.name))
                    .0
            })
            .collect();
        self.layout = layout.with_prefix_chips(&chips);
        self.attach_menu();
        self.probe_check();
    }

    /// An open action menu hangs from its row, and the window grows to hold it.
    /// Last in a layout: this is also where the window learns its size.
    fn attach_menu(&mut self) {
        let anchor = self.state.menu().and_then(|menu| {
            let anchor = self.layout.rect_of(menu.row)?;
            Some((anchor, menu.items.len()))
        });
        if let Some((anchor, count)) = anchor {
            let tallest = self.tallest();
            self.layout = std::mem::replace(&mut self.layout, Layout::new(&self.spec, &[], 0))
                .with_menu(anchor, count, tallest);
        }
        self.visible.set(self.layout.size);
    }

    /// Asks the system whether the combination `#` is checking is free, once
    /// for each: it is tried on this window and let go at once.
    fn probe_check(&mut self) {
        let Some(typed) = self.state.check().map(str::to_owned) else {
            self.check = None;
            return;
        };
        if self.check.as_ref().is_some_and(|(done, _)| *done == typed) {
            return;
        }
        let (Some(hotkey), Some(combo)) = (Hotkey::parse(&typed), KeyCombo::parse(&typed)) else {
            return;
        };
        let mut probe = HotkeyManager::new(self.overlay.hwnd());
        let registered = probe.register(PROBE_ID, hotkey, Fallback::None).is_ok();
        probe.unregister(PROBE_ID);
        self.check = Some((typed, keycheck::availability(&combo, registered)));
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
            big_icons,
            sized_pixels,
            panes,
            items,
            details,
            files,
            tracks_files,
            radius,
            border,
            caret_width,
            focused,
            spec,
            hover,
            startup,
            search_hotkey,
            check,
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
            .map(|&place| {
                let name = place.name(tree);
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
                (true, Some(&place)) => format!("Search in {}…", place.name(tree)),
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

        if let Some(chevron) = layout.chevron {
            canvas.draw_text(
                GLYPH_CHEVRON,
                rect(chevron),
                &fonts.glyph,
                dim,
                TextAlign::Centre,
            )?;
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
        let check_text = state.check().map(str::to_owned);
        let context = LineContext {
            startup: *startup,
            search_hotkey: search_hotkey.as_deref(),
            check: check_text.as_deref().map(|typed| {
                let answer = check
                    .as_ref()
                    .filter(|(done, _)| done == typed)
                    .map(|(_, availability)| *availability);
                (typed, answer)
            }),
            scan: files.progress(),
        };
        let rows = state.rows().to_vec();
        let mut painter = ItemPainter {
            palette: &palette,
            fonts,
            layout,
            tree,
            index,
            icons,
            pixels,
            big_icons,
            sized_pixels,
            items,
            details,
            files,
            tracks_files: *tracks_files,
            icon_size: *icon_size,
            scale: spec.scale,
            hover: *hover,
            menu_open: state.menu().is_some(),
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
                Row::Letter(letter) => {
                    canvas.draw_text_ellipsis(
                        &letter.to_string(),
                        rect(placed.rect.inset(padding, 0)),
                        &fonts.heading,
                        dim,
                        TextAlign::Left,
                    )?;
                }
                Row::Result(_) if layout.columns.is_some() => {}
                Row::Result(hit) => {
                    let is_selected = state.selected() == Some(placed.index);
                    let number = state
                        .quick_pick()
                        .then(|| state.result_number(placed.index))
                        .flatten();
                    painter.item(canvas, state, placed, hit, is_selected, number)?;
                }
                Row::Section(section) => {
                    let is_selected = state.selected() == Some(placed.index);
                    painter.section(canvas, placed, &section, is_selected)?;
                }
                Row::Line(line_kind) => {
                    let is_selected = state.selected() == Some(placed.index);
                    painter.line(canvas, placed, line_kind, &context, is_selected)?;
                }
                Row::Prefixes => painter.prefixes(canvas)?,
            }
        }
        painter.letters(canvas, state)?;
        if let Some(columns) = &layout.columns {
            painter.columns(canvas, state, columns, panes)?;
        } else if let Some(list) = layout.list
            && (layout.more_above || layout.more_below)
        {
            scrollbar(
                canvas,
                list,
                layout.first..layout.end,
                rows.len(),
                line,
                spec.scale,
            );
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
            let hints = if state.menu().is_some() {
                "↑↓  Choose        ↵  Do it        Esc  Close the menu".to_owned()
            } else if state.column_depth().is_some() {
                let mut hints = "↑↓  Move      ←→  Columns      ↵  Open".to_owned();
                for id in [ActionId::Reveal, ActionId::Edit] {
                    if let Some(key) = state.key_of(id) {
                        hints.push_str(&format!("      {key}  {}", id.short_label()));
                    }
                }
                hints.push_str("      Esc  Back");
                hints
            } else {
                format!(
                    "↑↓  Move      ↵  Open      {}  Actions      Tab  Into group      Esc  Back",
                    state.menu_key()
                )
            };
            canvas.draw_text_ellipsis(
                &hints,
                rect(footer.inset(padding, 0)),
                &fonts.small,
                dim,
                TextAlign::Left,
            )?;
        }

        // --- The action menu, over the rest ----------------------------
        if let (Some(menu_layout), Some(menu)) = (&layout.menu, state.menu()) {
            let panel = blend(back, text, 0.06);
            canvas.fill_rect(rect(menu_layout.rect), panel);
            canvas.stroke_rect(rect(menu_layout.rect), line);

            let pad = scaled(10);
            for (n, (item_rect, item)) in menu_layout.items.iter().zip(&menu.items).enumerate() {
                let lit = n == menu.selected;
                let (name_color, hint_color, plate) = if lit {
                    (
                        palette.selected_text,
                        blend(palette.selected_text, palette.selected_back, 0.35),
                        Some(palette.selected_back),
                    )
                } else {
                    (text, dim, None)
                };
                let cell = rect(*item_rect);
                if let Some(plate) = plate {
                    canvas.fill_rect(cell, plate);
                }

                // The shortcut, right aligned; the name ends before it.
                let mut name_right = cell.right - pad;
                if let Some(chord) = item.key {
                    let hint = chord.to_string();
                    let (width, _) = fonts.small.measure(&hint);
                    canvas.draw_text(
                        &hint,
                        RECT {
                            left: name_right - width,
                            right: name_right,
                            ..cell
                        },
                        &fonts.small,
                        hint_color,
                        TextAlign::Right,
                    )?;
                    name_right -= width + pad;
                }
                canvas.draw_text_ellipsis(
                    state.action_label(index, item.id),
                    RECT {
                        left: cell.left + pad,
                        right: name_right,
                        ..cell
                    },
                    &fonts.title,
                    name_color,
                    TextAlign::Left,
                )?;
                if lit {
                    canvas.round_rect_corners(cell, scaled(4), panel);
                }
            }
        }

        // --- Border and corners, last ----------------------------------
        look::draw_frame(canvas, window, *radius, *border, palette.border);
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
    big_icons: &'a mut IconCache,
    sized_pixels: &'a mut HashMap<(u32, i32), Option<Vec<u32>>>,
    items: &'a [Item],
    details: &'a mut HashMap<u32, String>,
    files: &'a Files,
    tracks_files: bool,
    icon_size: i32,
    scale: f32,
    /// The row button under the pointer.
    hover: Option<Hit>,
    /// While the action menu is open the buttons are shown but do not light.
    menu_open: bool,
}

impl ItemPainter<'_> {
    fn item(
        &mut self,
        canvas: &mut Canvas,
        state: &mut State,
        placed: &Placed,
        hit: query::Hit,
        selected: bool,
        number: Option<usize>,
    ) -> Result<()> {
        let palette = self.palette;
        let entry = &self.index.entries()[hit.entry as usize];
        let node = entry.node().map(|id| self.tree.node(id));
        let item = match entry.target {
            Target::History(number) => Some((number, &self.items[number as usize])),
            Target::Node(_) | Target::File(_) => None,
        };
        let file = match entry.target {
            Target::File(number) => Some(number),
            Target::Node(_) | Target::History(_) => None,
        };
        let row = placed.rect;
        let mut parts = self.layout.item_parts(row);
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;

        // The buttons of the selected row take the room at its right end; the
        // texts end before them.
        let buttons = self
            .layout
            .buttons
            .as_ref()
            .filter(|buttons| selected && buttons.row == placed.index);
        if let Some(buttons) = buttons {
            let limit = buttons.left - scaled(12);
            parts.title.right = parts.title.right.min(limit);
            parts.subtitle.right = parts.subtitle.right.min(limit);
        }

        // On the selected row the name is a little dimmer than its matched
        // letters, which keep the full text colour.
        let (title, matched, detail, background) = if selected {
            (
                palette.selected_title(),
                palette.selected_text,
                palette.selected_detail(),
                Some(palette.selected_back),
            )
        } else {
            (palette.text, palette.matched, palette.dim, None)
        };
        let plate = look::plate(row, self.scale);
        if let Some(background) = background {
            canvas.fill_rect(plate, background);
        }

        // The icon, loaded the first time its row is shown.
        let size = self.icon_size;
        let (icons, tree, items, files) = (&mut *self.icons, self.tree, self.items, self.files);
        let pixels = self.pixels.entry(hit.entry).or_insert_with(|| {
            load_icon(icons, entry, tree, items, files).and_then(|icon| icon.to_pixels(size))
        });
        if let Some(pixels) = pixels {
            canvas.blit(parts.icon.left, parts.icon.top, size, size, pixels);
        }

        // The hotkey, right aligned; the texts end before it.
        let mut right = parts.title.right;
        // The hotkey that opens it or, failing one, the key that picks it.
        let trailing = node
            .and_then(|node| crate::menu::shortcut_text(&self.tree.defaults, node))
            .or_else(|| number.filter(|&n| n <= 9).map(|n| format!("Alt+{n}")));
        if let Some(shortcut) = trailing {
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
        let label = entry_name(entry, self.tree, self.items, self.files);
        let label = label.as_str();
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

        // The second line; under Recent, led by when it was launched.
        let mut subtitle = match item {
            Some((number, item)) => self
                .details
                .entry(number)
                .or_insert_with(|| item.detail())
                .clone(),
            None => match file {
                Some(number) => files.detail(number),
                None => display::subtitle(self.tree, entry, hit.matched),
            },
        };
        if let Some(row) = state.recent_row(hit) {
            let since = history::now().saturating_sub(row.last);
            subtitle = format!("{} · {subtitle}", display::ago(since));
        }
        canvas.draw_text_ellipsis(
            &subtitle,
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

        // The buttons: a pill for each action, then the ⋯ that opens the menu.
        if let Some(buttons) = buttons {
            let (back, text) = (palette.selected_back, palette.selected_text);
            let lit = |button: Hit| !self.menu_open && self.hover == Some(button);
            let pill = |hovered: bool| palette.pill(hovered);
            let radius = scaled(4);
            let ids = state.row_buttons(self.index);
            for (n, (cell, id)) in buttons.items.iter().zip(&ids).enumerate() {
                let cell = rect(*cell);
                canvas.fill_rect(cell, pill(lit(Hit::Button(n))));
                canvas.draw_text(
                    state.action_short_label(self.index, *id),
                    cell,
                    &self.fonts.small,
                    text,
                    TextAlign::Centre,
                )?;
                canvas.round_rect_corners(cell, radius, back);
            }
            let cell = rect(buttons.more);
            canvas.fill_rect(cell, pill(lit(Hit::More)));
            canvas.draw_text(GLYPH_MORE, cell, &self.fonts.glyph, text, TextAlign::Centre)?;
            canvas.round_rect_corners(cell, radius, back);
        }

        // After the text: the plate's corners, toward the panel behind it.
        if background.is_some() {
            canvas.round_rect_corners(plate, scaled(ROW_RADIUS), palette.back);
        }
        Ok(())
    }
}

impl ItemPainter<'_> {
    /// A section of the empty box: its icon (a folder, unless the group has one
    /// of its own), its name, how many apps it holds, and a › — Enter, → or a
    /// click goes inside.
    fn section(
        &mut self,
        canvas: &mut Canvas,
        placed: &Placed,
        section: &SectionRow,
        selected: bool,
    ) -> Result<()> {
        let palette = self.palette;
        let row = placed.rect;
        let parts = self.layout.item_parts(row);
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;

        let (title, detail, background) = if selected {
            (
                palette.selected_title(),
                palette.selected_detail(),
                Some(palette.selected_back),
            )
        } else {
            (palette.text, palette.dim, None)
        };
        let plate = look::plate(row, self.scale);
        if let Some(background) = background {
            canvas.fill_rect(plate, background);
        }

        // The same icon a group has in the tray menu, loaded once per opening;
        // the folder glyph only if the system gives none.
        let (key, node) = match section.place {
            Place::Menus | Place::Recent => (SECTIONS_ICON, None),
            Place::Group(group) => (SECTION_ICON_BASE | group.as_u32(), Some(group)),
        };
        let size = self.icon_size;
        let (icons, tree) = (&mut *self.icons, self.tree);
        let is_recent = section.place == Place::Recent;
        let pixels = if is_recent {
            &None
        } else {
            &*self.pixels.entry(key).or_insert_with(|| {
                let icon = match node {
                    Some(group) => crate::menu::resolve_icon(icons, tree.node(group)),
                    None => icons.for_directory(),
                };
                icon.and_then(|icon| icon.to_pixels(size))
            })
        };
        match pixels {
            Some(pixels) => canvas.blit(parts.icon.left, parts.icon.top, size, size, pixels),
            None => canvas.draw_text(
                if is_recent {
                    GLYPH_HISTORY
                } else {
                    GLYPH_FOLDER
                },
                rect(parts.icon),
                &self.fonts.glyph,
                detail,
                TextAlign::Centre,
            )?,
        }
        // The › at the right end; the texts stop before it.
        let (arrow_width, _) = self.fonts.glyph.measure(GLYPH_ENTER);
        let right = parts.title.right;
        canvas.draw_text(
            GLYPH_ENTER,
            RECT {
                left: right - arrow_width,
                top: row.top,
                right,
                bottom: row.bottom,
            },
            &self.fonts.glyph,
            detail,
            TextAlign::Right,
        )?;
        let text_right = right - arrow_width - scaled(12);
        canvas.draw_text_ellipsis(
            &section.place.name(self.tree),
            RECT {
                right: text_right,
                ..rect(parts.title)
            },
            &self.fonts.title,
            title,
            TextAlign::Left,
        )?;
        let noun = match (is_recent, section.apps) {
            (true, _) => "items",
            (false, 1) => "app",
            (false, _) => "apps",
        };
        let mut line = format!("{} {noun}", section.apps);
        if is_recent && !self.tracks_files {
            line.push_str(" · Windows is not keeping recent files");
        }
        canvas.draw_text_ellipsis(
            &line,
            RECT {
                right: text_right,
                ..rect(parts.subtitle)
            },
            &self.fonts.small,
            detail,
            TextAlign::Left,
        )?;

        if background.is_some() {
            canvas.round_rect_corners(plate, scaled(ROW_RADIUS), palette.back);
        }
        Ok(())
    }
}

/// The icon of `entry`, whatever it stands for, at the size of `icons`.
fn load_icon(
    icons: &mut IconCache,
    entry: &Entry,
    tree: &ConfigTree,
    items: &[Item],
    files: &Files,
) -> Option<y_core::icon::Icon> {
    match entry.target {
        Target::Node(id) => crate::menu::resolve_icon(icons, tree.node(id)),
        Target::History(number) => items[number as usize].icon(icons),
        Target::File(number) => files.icon(number, icons),
    }
}

/// The name of `entry` as written, whatever it stands for.
fn entry_name(entry: &Entry, tree: &ConfigTree, items: &[Item], files: &Files) -> String {
    match entry.target {
        Target::Node(id) => plain(&tree.node(id).name),
        Target::History(number) => items[number as usize].name.clone(),
        Target::File(number) => files.get(number).name.clone(),
    }
}

impl ItemPainter<'_> {
    /// The letters of an A–Z list, across their band: a click jumps to one.
    fn letters(&self, canvas: &mut Canvas, state: &State) -> Result<()> {
        let palette = self.palette;
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;
        for (n, (cell, letter)) in self.layout.letters.iter().zip(state.letters()).enumerate() {
            let lit = !self.menu_open && self.hover == Some(Hit::Letter(n));
            let cell = rect(*cell);
            canvas.fill_rect(
                cell,
                blend(palette.back, palette.text, if lit { 0.16 } else { 0.07 }),
            );
            canvas.draw_text(
                &letter.to_string(),
                cell,
                &self.fonts.small,
                if lit { palette.text } else { palette.dim },
                TextAlign::Centre,
            )?;
            canvas.round_rect_corners(cell, scaled(4), palette.back);
        }
        Ok(())
    }

    /// The Miller columns: the breadcrumb, then each pane on screen, split by a
    /// thin line.
    fn columns(
        &mut self,
        canvas: &mut Canvas,
        state: &State,
        columns: &ColumnsLayout,
        panes: &[PaneView],
    ) -> Result<()> {
        let palette = self.palette;
        let scale = self.scale;
        let scaled = |logical: i32| (logical as f32 * scale).round() as i32;

        let mut crumbs = vec![Place::Menus.name(self.tree)];
        crumbs.extend(state.column_crumbs().iter().map(|&entry| {
            let entry = &self.index.entries()[entry as usize];
            entry_name(entry, self.tree, self.items, self.files)
        }));
        canvas.draw_text_ellipsis(
            &crumbs.join(" › "),
            rect(columns.crumb.inset(scaled(16), 0)),
            &self.fonts.small,
            palette.dim,
            TextAlign::Left,
        )?;
        canvas.fill_rect(
            RECT {
                top: columns.crumb.bottom - 1,
                ..rect(columns.crumb)
            },
            palette.line,
        );

        for pane in &columns.panes {
            if pane.rect.left > 0 {
                canvas.fill_rect(
                    RECT {
                        right: pane.rect.left + 1,
                        ..rect(pane.rect)
                    },
                    palette.line,
                );
            }
            let Some(view) = panes.get(pane.pane) else {
                continue;
            };
            if let Some(entry) = view.details {
                self.details(canvas, state, columns, pane.rect, entry)?;
                continue;
            }
            for (n, row) in pane.rows.iter().enumerate() {
                let number = pane.first + n;
                let lit = (view.selected == Some(number)).then_some(pane.pane == columns.focus);
                self.column_row(canvas, *row, view.items[number], lit)?;
            }
            if pane.rows.is_empty() && (pane.pane <= columns.focus || state.selected().is_some()) {
                let note = if view.waiting { "Listing…" } else { "Empty" };
                canvas.draw_text_ellipsis(
                    note,
                    rect(pane.rect.inset(scaled(14), scaled(12))),
                    &self.fonts.small,
                    palette.dim,
                    TextAlign::Left,
                )?;
            }
            if pane.total > pane.rows.len() {
                let shown = pane.first..pane.first + pane.rows.len();
                scrollbar(
                    canvas,
                    pane.rect,
                    shown,
                    pane.total,
                    palette.line,
                    self.scale,
                );
            }
        }
        Ok(())
    }

    /// A row of a column. `lit`: it is the selection of its column, `true` when
    /// that column has the keys.
    fn column_row(
        &mut self,
        canvas: &mut Canvas,
        row: layout::Rect,
        position: u32,
        lit: Option<bool>,
    ) -> Result<()> {
        let palette = self.palette;
        let entry = &self.index.entries()[position as usize];
        let parts = self.layout.compact_parts(row);
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;
        let (title, detail, back) = match lit {
            Some(true) => {
                let (back, text) = (palette.selected_back, palette.selected_text);
                (text, blend(text, back, 0.35), Some(back))
            }
            Some(false) => (
                palette.text,
                palette.dim,
                Some(blend(palette.back, palette.text, 0.1)),
            ),
            None => (palette.text, palette.dim, None),
        };
        let plate = RECT {
            left: row.left + scaled(ROW_MARGIN),
            top: row.top + 1,
            right: row.right - scaled(ROW_MARGIN),
            bottom: row.bottom - 1,
        };
        if let Some(back) = back {
            canvas.fill_rect(plate, back);
        }

        let size = parts.icon.width();
        let (icons, tree, items, files) = (&mut *self.icons, self.tree, self.items, self.files);
        let pixels = self
            .sized_pixels
            .entry((position, size))
            .or_insert_with(|| {
                load_icon(icons, entry, tree, items, files).and_then(|icon| icon.to_pixels(size))
            });
        if let Some(pixels) = pixels {
            canvas.blit(parts.icon.left, parts.icon.top, size, size, pixels);
        }

        // A › where something opens from it; otherwise the hotkey, if any.
        let mut right = parts.title.right;
        if Columns::opens(entry) {
            let (width, _) = self.fonts.glyph.measure(GLYPH_ENTER);
            canvas.draw_text(
                GLYPH_ENTER,
                RECT {
                    left: right - width,
                    right,
                    ..rect(parts.trailing)
                },
                &self.fonts.glyph,
                detail,
                TextAlign::Right,
            )?;
            right -= width + scaled(6);
        } else if let Some(shortcut) = entry
            .node()
            .and_then(|id| crate::menu::shortcut_text(&self.tree.defaults, self.tree.node(id)))
        {
            let (width, _) = self.fonts.small.measure(&shortcut);
            // Rather the name than the hotkey, in so narrow a column.
            if width * 2 < parts.title.width() {
                canvas.draw_text(
                    &shortcut,
                    RECT {
                        left: right - width,
                        right,
                        ..rect(parts.trailing)
                    },
                    &self.fonts.small,
                    detail,
                    TextAlign::Right,
                )?;
                right -= width + scaled(6);
            }
        }
        canvas.draw_text_ellipsis(
            &entry_name(entry, self.tree, self.items, self.files),
            RECT {
                right,
                ..rect(parts.title)
            },
            &self.fonts.title,
            title,
            TextAlign::Left,
        )?;
        if back.is_some() {
            canvas.round_rect_corners(plate, scaled(ROW_RADIUS), palette.back);
        }
        Ok(())
    }

    /// The details pane: a big icon, the name, what is known of it, and the
    /// buttons along its bottom.
    fn details(
        &mut self,
        canvas: &mut Canvas,
        state: &State,
        columns: &ColumnsLayout,
        pane: layout::Rect,
        position: u32,
    ) -> Result<()> {
        let palette = self.palette;
        let entry = &self.index.entries()[position as usize];
        let parts = self.layout.details_parts(pane);
        let size = parts.icon.width();
        let (icons, tree, items, files) = (&mut *self.big_icons, self.tree, self.items, self.files);
        let pixels = self
            .sized_pixels
            .entry((position, size))
            .or_insert_with(|| {
                load_icon(icons, entry, tree, items, files).and_then(|icon| icon.to_pixels(size))
            });
        if let Some(pixels) = pixels {
            canvas.blit(parts.icon.left, parts.icon.top, size, size, pixels);
        }
        canvas.draw_text_ellipsis(
            &entry_name(entry, self.tree, self.items, self.files),
            rect(parts.title),
            &self.fonts.title,
            palette.text,
            TextAlign::Left,
        )?;

        let mut lines = display::details(self.tree, entry);
        if let Target::File(number) = entry.target {
            lines.push(self.files.detail(number));
            if entry.facts.browsable {
                lines.push(display::FOLDER_HINT.to_owned());
            }
        }
        let mut cell = parts.first_line;
        for line in &lines {
            if cell.bottom > parts.bottom {
                break;
            }
            canvas.draw_text_ellipsis(
                line,
                rect(cell),
                &self.fonts.small,
                palette.dim,
                TextAlign::Left,
            )?;
            cell.top += parts.line;
            cell.bottom += parts.line;
        }

        let ids = state.details_buttons(self.index);
        let radius = (4.0 * self.scale).round() as i32;
        for (n, (button, id)) in columns.buttons.iter().zip(&ids).enumerate() {
            let lit = !self.menu_open && self.hover == Some(Hit::Details(n));
            let cell = rect(*button);
            canvas.fill_rect(
                cell,
                blend(palette.back, palette.text, if lit { 0.2 } else { 0.1 }),
            );
            canvas.draw_text(
                state.action_short_label(self.index, *id),
                cell,
                &self.fonts.small,
                palette.text,
                TextAlign::Centre,
            )?;
            canvas.round_rect_corners(cell, radius, palette.back);
        }
        Ok(())
    }
}

/// The text of a chip of the line of prefixes: `> commands`.
fn chip_label(prefix: char, name: &str) -> String {
    format!("{prefix}  {name}")
}

impl ItemPainter<'_> {
    /// A row of a prefixed list: a glyph, the title, the second line and, at
    /// the right end, a state or a hotkey.
    fn line(
        &self,
        canvas: &mut Canvas,
        placed: &Placed,
        line: Line,
        context: &LineContext,
        selected: bool,
    ) -> Result<()> {
        let palette = self.palette;
        let row = placed.rect;
        let parts = self.layout.item_parts(row);
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;
        let words = display::line_text(line, context);
        let glyph = match line {
            Line::Command(_) => "\u{E756}",
            Line::Prefix(_) => "\u{E8D5}",
            Line::SearchHotkey => "\u{E765}",
            // "Sync" while it runs, then "Info".
            Line::Scan if context.scan.listing => "\u{E895}",
            Line::Scan => "\u{E946}",
            Line::Check => match context.check {
                Some((_, Some(Availability::Free))) => "\u{E73E}",
                Some((_, Some(_))) => "\u{E711}",
                _ => "\u{E946}",
            },
        };

        let (title, detail, background) = if selected {
            (
                palette.selected_title(),
                palette.selected_detail(),
                Some(palette.selected_back),
            )
        } else {
            (palette.text, palette.dim, None)
        };
        let plate = look::plate(row, self.scale);
        if let Some(background) = background {
            canvas.fill_rect(plate, background);
        }

        canvas.draw_text(
            glyph,
            rect(parts.icon),
            &self.fonts.glyph,
            detail,
            TextAlign::Centre,
        )?;
        let mut right = parts.title.right;
        if let Some(trailing) = &words.trailing {
            let (width, _) = self.fonts.small.measure(trailing);
            canvas.draw_text(
                trailing,
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
        canvas.draw_text_ellipsis(
            &words.title,
            RECT {
                right,
                ..rect(parts.title)
            },
            &self.fonts.title,
            title,
            TextAlign::Left,
        )?;
        canvas.draw_text_ellipsis(
            &words.subtitle,
            RECT {
                right: parts.subtitle.right,
                ..rect(parts.subtitle)
            },
            &self.fonts.small,
            detail,
            TextAlign::Left,
        )?;
        if background.is_some() {
            canvas.round_rect_corners(plate, scaled(ROW_RADIUS), palette.back);
        }
        Ok(())
    }

    /// The line of prefixes under the sections: a chip for each, clickable.
    fn prefixes(&self, canvas: &mut Canvas) -> Result<()> {
        let palette = self.palette;
        let scaled = |logical: i32| (logical as f32 * self.scale).round() as i32;
        for (n, (chip, info)) in self.layout.prefix_chips.iter().zip(PREFIXES).enumerate() {
            let lit = !self.menu_open && self.hover == Some(Hit::Prefix(n));
            let cell = rect(*chip);
            canvas.fill_rect(
                cell,
                blend(palette.back, palette.text, if lit { 0.16 } else { 0.07 }),
            );
            canvas.draw_text(
                &chip_label(info.prefix, info.name),
                cell,
                &self.fonts.small,
                if lit { palette.text } else { palette.dim },
                TextAlign::Centre,
            )?;
            canvas.round_rect_corners(cell, scaled(4), palette.back);
        }
        Ok(())
    }
}

/// A thin thumb on the right edge of `list`: where the rows `visible` sit among
/// all of them.
fn scrollbar(
    canvas: &mut Canvas,
    list: layout::Rect,
    visible: Range<usize>,
    total: usize,
    color: Color,
    scale: f32,
) {
    if total == 0 {
        return;
    }
    let width = ((SCROLLBAR_WIDTH as f32 * scale).round() as i32).max(2);
    let track = list.height();
    let top = list.top + (track as usize * visible.start / total) as i32;
    let height = ((track as usize * visible.len() / total) as i32).max(width * 4);
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
        c if c == VK_APPS.0 => Key::ContextMenu,
        c if c == VK_BACK.0 => Key::Backspace,
        c if c == VK_DELETE.0 => Key::Delete,
        // The number row: `Alt+1…9` picks a result.
        digit @ 0x30..=0x39 => Key::Digit((digit - 0x30) as u8),
        // F1…F12.
        function @ 0x70..=0x7B => Key::Function((function - 0x6F) as u8),
        // Letters have the code of their upper-case ASCII.
        letter @ 0x41..=0x5A => Key::Letter(letter as u8 as char),
        _ => return None,
    })
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
            // Alt+letter has no menu to open here; swallowing the character it
            // makes spares a beep.
            WM_SYSCHAR => Some(LRESULT(0)),
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
            WM_RBUTTONUP => {
                self.post(WM_SEARCH_CONTEXT, lparam.0);
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
