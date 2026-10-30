//! What the search box is doing: the text typed, the results for it and which
//! one is selected. Pure — the window turns key presses and mouse movement
//! into [`Event`]s, and acts on the [`Outcome`] each one returns.
//!
//! Splitting it this way keeps every key of spec §2 a unit test, and leaves
//! the window with the Win32 half only: reading the keyboard, the clipboard,
//! painting.

use crate::model::{ActionId, ActionsConfig, MenusBrowse, ResultsLayout};

use super::actions::{self, Available};
use super::columns::{Columns, PaneView, Preview};
use super::commands::{Command, Line, PREFIXES};
use super::files::Progress;
use super::hotkey::KeyCombo;
use super::index::{Field, Index, Kind, Source};
use super::input::Input;
use super::layout::RowKind;
use super::matcher::fold;
use super::query::{Hit, Query, Scope, Searcher};
use super::results::{self, Heading};
pub use super::sections::{Place, SectionRow};

/// How many results `PgUp`/`PgDn` move over.
pub const PAGE: usize = 5;

pub use super::keys::{Chord, Key, Mods};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// A typed character.
    Char(char),
    /// Text pasted (the window read the clipboard after [`Outcome::Paste`]).
    Paste(String),
    Key(Key, Mods),
}

/// What the window does next.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Nothing visible changed.
    Nothing,
    Redraw,
    /// Close the window without acting.
    Close,
    /// Do this action to this result (Enter, a click, a shortcut…).
    Activate(Hit, ActionId),
    /// Put this text on the clipboard.
    Copy(String),
    /// Read the clipboard and answer with [`Event::Paste`].
    Paste,
    /// Run this y-pop command (`>`).
    Command(Command),
    /// Enter on the answer of a key check (`#`): the window knows whether the key
    /// is free.
    CheckKey,
    /// A column opened on this browsed folder (an entry of the index): list it,
    /// and answer with [`State::column_listed`].
    ListFolder(u32),
}

/// One row of the list, in the order shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Header(Heading),
    /// The header of a letter of an A–Z list; `#` for what is not a letter.
    Letter(char),
    Result(Hit),
    /// A section of the empty box: Enter opens it as a chip.
    Section(SectionRow),
    /// A line of a prefixed list: a command, a prefix, the answer of a key check.
    Line(Line),
    /// The line of clickable prefixes under the sections.
    Prefixes,
}

impl Row {
    /// Whether the selection can rest on it: not a header, not the line of
    /// prefixes, not the note on how `/` is doing.
    pub fn selectable(&self) -> bool {
        !matches!(
            self,
            Self::Header(_) | Self::Letter(_) | Self::Prefixes | Self::Line(Line::Scan)
        )
    }
}

/// The action menu opened on a result: what it lists and which line is lit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActionMenu {
    /// The row it belongs to, and hangs from.
    pub row: usize,
    /// The actions that apply to that result, in the configured order.
    pub items: Vec<Available>,
    pub selected: usize,
}

pub struct State {
    input: Input,
    query: Query,
    /// The groups entered with Tab, outermost first: the chips in the box.
    /// A search looks only under the last.
    scopes: Vec<Place>,
    layout: ResultsLayout,
    searcher: Searcher,
    rows: Vec<Row>,
    /// Index into `rows`, always of a [`Row::Result`].
    selected: Option<usize>,
    /// Which actions exist, and the keys that reach them.
    actions: ActionsConfig,
    /// Open while the user chooses an action; the keys then belong to it.
    menu: Option<ActionMenu>,
    /// Whether the selected row shows buttons (`row-actions`).
    row_actions: bool,
    /// What Windows remembers, newest first. The empty box lists the first
    /// [`Self::recent_shown`]; [`Place::Recent`] lists them all.
    recent: Vec<Recent>,
    /// `<recent count>`.
    recent_shown: usize,
    /// The sections the empty box lists under Browse.
    sections: Vec<SectionRow>,
    /// The search hotkey, listed by `#`.
    search_hotkey: Option<KeyCombo>,
    /// What the typed combination is, when `#` asks whether it is free: the
    /// text after the prefix, for the window to probe.
    check: Option<String>,
    /// How the listing of `/` is going.
    scan: Progress,
    /// `collapsed="true"`: the list can be folded away, leaving the input row.
    collapsible: bool,
    /// Folded now: no rows, whatever the text. Only ever true when collapsible.
    folded: bool,
    /// `<menus browse>`: how [`Place::Menus`] opens.
    menus_browse: MenusBrowse,
    /// Open while inside [`Place::Menus`], with `browse="columns"`.
    columns: Option<Columns>,
    /// The letters of the A–Z list being shown, for its index.
    letters: Vec<char>,
}

/// One row of the history: what the empty box lists under **Recent**.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Recent {
    /// Position in [`Index::entries`].
    pub entry: u32,
    /// Position in [`super::history::Listing::items`].
    pub item: u32,
    /// Seconds since 1970.
    pub last: u64,
}

impl State {
    pub fn new(layout: ResultsLayout, actions: ActionsConfig) -> Self {
        Self {
            input: Input::new(),
            query: Query::default(),
            scopes: Vec::new(),
            layout,
            searcher: Searcher::new(),
            rows: Vec::new(),
            selected: None,
            actions,
            menu: None,
            row_actions: true,
            recent: Vec::new(),
            recent_shown: usize::MAX,
            sections: Vec::new(),
            search_hotkey: None,
            check: None,
            scan: Progress::default(),
            collapsible: false,
            folded: false,
            menus_browse: MenusBrowse::default(),
            columns: None,
            letters: Vec::new(),
        }
    }

    /// `<menus browse>`: how the tray menus section opens.
    pub fn with_menus_browse(mut self, browse: MenusBrowse) -> Self {
        self.menus_browse = browse;
        self
    }

    /// `<recent count>`: how many rows the empty box shows. A longer history
    /// is reached through a row that opens [`Place::Recent`].
    pub fn with_recent_shown(mut self, shown: usize) -> Self {
        self.recent_shown = shown;
        self
    }

    /// Sets what the history lists and what it adds to the scores, and shows
    /// the empty box. Called once, as the window opens.
    pub fn start(
        &mut self,
        index: &Index,
        recent: Vec<Recent>,
        sections: Vec<SectionRow>,
        bonuses: Vec<u32>,
    ) {
        self.recent = recent;
        self.sections = sections;
        self.searcher.set_bonuses(bonuses);
        self.refresh(index);
    }

    /// Whether `hit` is a row of the history.
    fn is_recent(&self, hit: Hit) -> bool {
        self.recent_row(hit).is_some()
    }

    /// The history's line for `hit`, if it is listed as such right now: on the
    /// empty box, or anywhere in [`Place::Recent`].
    pub fn recent_row(&self, hit: Hit) -> Option<Recent> {
        let on_empty_box =
            self.query.is_empty() && self.scopes.is_empty() && self.prefix().is_none();
        if !on_empty_box && self.scopes.last() != Some(&Place::Recent) {
            return None;
        }
        self.recent
            .iter()
            .find(|recent| recent.entry == hit.entry)
            .copied()
    }

    /// Takes the launch of `entry` off the Recent list (it was forgotten), the
    /// selection staying where it was.
    pub fn remove_recent(&mut self, index: &Index, entry: u32) {
        let at = self.selected;
        self.recent.retain(|recent| recent.entry != entry);
        self.refresh(index);
        if let Some(at) = at {
            let same_place =
                matches!(self.rows.get(at), Some(Row::Result(_) | Row::Section(_))).then_some(at);
            self.selected = same_place.or_else(|| {
                let last_recent = self
                    .rows
                    .iter()
                    .rposition(|row| matches!(row, Row::Result(_)));
                last_recent.or_else(|| self.first_selectable())
            });
        }
    }

    /// `collapsed="true"`: opens folded, as the input row alone (spec §2.4).
    /// `↓` or [`Self::unfold`] shows the list; `↑` on its first row, `Esc` with
    /// nothing typed, or erasing the text folds it again.
    pub fn with_collapsed(mut self, collapsed: bool) -> Self {
        self.collapsible = collapsed;
        self.folded = collapsed;
        self
    }

    /// Whether the list is folded away (the window is then one row high).
    pub fn folded(&self) -> bool {
        self.folded
    }

    /// Whether the window can fold at all, to draw the **⌄** that unfolds it.
    pub fn collapsible(&self) -> bool {
        self.collapsible
    }

    /// Shows the list again: Recent, or the group entered, first row selected.
    pub fn unfold(&mut self, index: &Index) -> Outcome {
        if !self.folded {
            return Outcome::Nothing;
        }
        self.folded = false;
        self.refresh(index);
        Outcome::Redraw
    }

    /// The first row of the unfolded empty box is selected (or there is none),
    /// so `↑` has nowhere to go but out of the list. Only when collapsible.
    fn at_top_of_empty_box(&self) -> bool {
        self.collapsible
            && !self.folded
            && self.query.is_empty()
            && self.scopes.is_empty()
            && self.prefix().is_none()
            && self.selected == self.first_selectable()
    }

    /// The first row a selection can rest on: not a header.
    fn first_selectable(&self) -> Option<usize> {
        self.rows.iter().position(Row::selectable)
    }

    fn fold(&mut self, index: &Index) -> Outcome {
        self.folded = true;
        self.refresh(index);
        Outcome::Redraw
    }

    /// After the text changed: typing unfolds, and erasing everything folds
    /// again — unless a group is entered, which is a place worth staying in.
    fn settle(&mut self, index: &Index) {
        if self.collapsible {
            self.folded = self.input.is_empty() && self.scopes.is_empty();
        }
        self.refresh(index);
    }

    /// `row-actions`: whether the selected row shows its action buttons.
    pub fn with_row_actions(mut self, row_actions: bool) -> Self {
        self.row_actions = row_actions;
        self
    }

    /// The actions shown as buttons on the selected row, in the configured
    /// order. Empty with `row-actions="false"`. The **⋯** that opens the menu
    /// is the window's to add, whenever [`Self::has_row_buttons`].
    pub fn row_buttons(&self, index: &Index) -> Vec<ActionId> {
        if !self.row_actions {
            return Vec::new();
        }
        self.available_actions(index)
            .into_iter()
            .filter(|action| action.button)
            .map(|action| action.id)
            .collect()
    }

    /// Whether the selected row has buttons at all: `row-actions` is on and
    /// some action applies to it.
    pub fn has_row_buttons(&self, index: &Index) -> bool {
        self.row_actions && !self.available_actions(index).is_empty()
    }

    /// A click on the `n`th of [`Self::row_buttons`]: does that action to the
    /// selected result.
    pub fn button_click(&mut self, index: &Index, n: usize) -> Outcome {
        match self.row_buttons(index).get(n) {
            Some(&id) => self.perform(index, id),
            None => Outcome::Nothing,
        }
    }

    pub fn input(&self) -> &Input {
        &self.input
    }

    /// The typed query, for emphasizing the matched letters of a name
    /// ([`Searcher::highlight`]).
    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The groups entered, outermost first — what the box shows as chips.
    pub fn scopes(&self) -> &[Place] {
        &self.scopes
    }

    fn scope(&self) -> Scope {
        self.scopes.last().map_or(Scope::All, |&place| match place {
            Place::Menus => Scope::Menus,
            Place::Group(group) => Scope::Within(group),
            // Everything, then narrowed to the history by `refresh`.
            Place::Recent => Scope::All,
        })
    }

    /// The rows as the layout wants to know them.
    pub fn row_kinds(&self) -> Vec<RowKind> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::Header(_) | Row::Letter(_) => RowKind::Header,
                Row::Result(_) | Row::Section(_) | Row::Line(_) => RowKind::Item,
                Row::Prefixes => RowKind::Prefixes,
            })
            .collect()
    }

    /// The selected row's index, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The selected result, if any.
    pub fn selected_hit(&self) -> Option<Hit> {
        match self.rows.get(self.selected?)? {
            Row::Result(hit) => Some(*hit),
            _ => None,
        }
    }

    /// The character positions of `hit`'s name that the typed words matched,
    /// for emphasizing them. For the rows on screen only: it allocates.
    pub fn highlight(&mut self, index: &Index, hit: Hit, positions: &mut Vec<usize>) {
        self.searcher
            .highlight(&index.entries()[hit.entry as usize], &self.query, positions);
    }

    /// Hover: selects the result at `row`. `false` if it is a header, out of
    /// range, or already selected.
    pub fn select_row(&mut self, row: usize) -> bool {
        // In the columns the selection decides what is shown to the right, so
        // it takes a click: the pointer passing over must not change that.
        if self.showing_columns() {
            return false;
        }
        match self.rows.get(row) {
            Some(selectable) if selectable.selectable() && self.selected != Some(row) => {
                self.selected = Some(row);
                true
            }
            _ => false,
        }
    }

    /// A click on `row`: selects it and, if it is a result, opens it the way
    /// Enter would.
    pub fn click_row(&mut self, index: &Index, row: usize) -> Outcome {
        if let Some(depth) = self.column_depth() {
            return self.column_click(index, depth, row);
        }
        match self.rows.get(row) {
            Some(Row::Result(_)) => {
                self.selected = Some(row);
                self.perform(index, ActionId::Open)
            }
            Some(Row::Section(section)) => {
                let place = section.place;
                self.selected = Some(row);
                self.enter(index, place)
            }
            Some(line @ Row::Line(_)) if line.selectable() => {
                self.selected = Some(row);
                self.activate_line(index)
            }
            _ => Outcome::Nothing,
        }
    }

    /// `row`'s place among the results, counting from 1 — the `n` of the
    /// `Alt+n` that picks it. `None` for a header or out of range.
    pub fn result_number(&self, row: usize) -> Option<usize> {
        matches!(self.rows.get(row), Some(Row::Result(_))).then(|| {
            1 + self.rows[..row]
                .iter()
                .filter(|row| matches!(row, Row::Result(_)))
                .count()
        })
    }

    /// Whether `Alt+1…9` pick results, and the rows say so.
    pub fn quick_pick(&self) -> bool {
        self.actions.quick_pick
    }

    /// The actions that apply to the selected result, in the configured
    /// order, each with its shortcut: the lines of the action menu.
    pub fn available_actions(&self, index: &Index) -> Vec<Available> {
        self.selected_hit()
            .map(|hit| {
                actions::available(
                    &self.actions,
                    &index.entries()[hit.entry as usize],
                    self.is_recent(hit),
                )
            })
            .unwrap_or_default()
    }

    /// What the action menu calls `id` on the selected result.
    pub fn action_label(&self, index: &Index, id: ActionId) -> &'static str {
        self.selected_hit().map_or(id.label(), |hit| {
            actions::label(id, &index.entries()[hit.entry as usize])
        })
    }

    /// What a row button calls `id` on the selected result.
    pub fn action_short_label(&self, index: &Index, id: ActionId) -> &'static str {
        self.selected_hit().map_or(id.short_label(), |hit| {
            actions::short_label(id, &index.entries()[hit.entry as usize])
        })
    }

    /// The action menu's key, to be recognised by the window.
    pub fn menu_key(&self) -> Chord {
        self.actions.menu_key
    }

    /// Does `action` to the selected result, if it applies to it. Open on a
    /// group enters it (it becomes a chip and the box searches inside it).
    fn perform(&mut self, index: &Index, action: ActionId) -> Outcome {
        let Some(hit) = self.selected_hit() else {
            return Outcome::Nothing;
        };
        let entry = &index.entries()[hit.entry as usize];
        if action == ActionId::Forget {
            return if self.is_recent(hit) {
                Outcome::Activate(hit, action)
            } else {
                Outcome::Nothing
            };
        }
        if !actions::applies(action, entry) {
            return Outcome::Nothing;
        }
        if action == ActionId::Open && entry.kind == Kind::Group {
            return self.enter_selected(index);
        }
        Outcome::Activate(hit, action)
    }

    /// `Alt+n`: opens the `n`th result (counting from 1), whichever group it
    /// is listed under.
    fn pick(&mut self, index: &Index, n: usize) -> Outcome {
        let row = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Result(_)))
            .nth(n.saturating_sub(1))
            .map(|(row, _)| row);
        match row {
            Some(row) => {
                self.selected = Some(row);
                self.perform(index, ActionId::Open)
            }
            None => Outcome::Nothing,
        }
    }

    /// The open action menu, if any.
    pub fn menu(&self) -> Option<&ActionMenu> {
        self.menu.as_ref()
    }

    /// Opens the action menu on the selected result. Nothing happens if there
    /// is no result, or no action applies to it.
    pub fn open_menu(&mut self, index: &Index) -> Outcome {
        let (Some(row), items) = (self.selected, self.available_actions(index)) else {
            return Outcome::Nothing;
        };
        if items.is_empty() {
            return Outcome::Nothing;
        }
        self.menu = Some(ActionMenu {
            row,
            items,
            selected: 0,
        });
        Outcome::Redraw
    }

    /// A right click on `row`: selects it and opens its menu.
    pub fn open_menu_at(&mut self, index: &Index, row: usize) -> Outcome {
        if !matches!(self.rows.get(row), Some(Row::Result(_))) {
            return Outcome::Nothing;
        }
        self.selected = Some(row);
        self.open_menu(index)
    }

    /// Closes the menu without doing anything. `false` if none was open.
    pub fn close_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    /// Hover: lights line `item` of the menu. `false` if nothing changed.
    pub fn menu_select(&mut self, item: usize) -> bool {
        match self.menu.as_mut() {
            Some(menu) if item < menu.items.len() && menu.selected != item => {
                menu.selected = item;
                true
            }
            _ => false,
        }
    }

    /// A click on line `item` of the menu: does that action.
    pub fn menu_click(&mut self, index: &Index, item: usize) -> Outcome {
        let id = self
            .menu
            .as_ref()
            .and_then(|menu| menu.items.get(item))
            .map(|a| a.id);
        match id {
            Some(id) => self.choose(index, id),
            None => Outcome::Nothing,
        }
    }

    /// What the keys do while the menu is open: move in it, choose from it,
    /// or close it. An action's own shortcut works too, if the action is in
    /// the menu.
    fn menu_keypress(&mut self, index: &Index, key: Key, mods: Mods) -> Outcome {
        let Some(menu) = self.menu.as_mut() else {
            return Outcome::Nothing;
        };
        let last = menu.items.len().saturating_sub(1);
        let chord = Chord::new(key, mods);

        match key {
            Key::Escape => {
                self.menu = None;
                Outcome::Redraw
            }
            Key::Up => {
                menu.selected = menu.selected.checked_sub(1).unwrap_or(last);
                Outcome::Redraw
            }
            Key::Down => {
                menu.selected = if menu.selected >= last {
                    0
                } else {
                    menu.selected + 1
                };
                Outcome::Redraw
            }
            Key::Home => {
                menu.selected = 0;
                Outcome::Redraw
            }
            Key::End => {
                menu.selected = last;
                Outcome::Redraw
            }
            // The lit line, whatever Enter does outside the menu.
            Key::Enter if mods == Mods::default() => {
                let id = menu.items[menu.selected].id;
                self.choose(index, id)
            }
            _ if key == Key::ContextMenu || chord == self.actions.menu_key => {
                self.menu = None;
                Outcome::Redraw
            }
            _ => {
                let bound = menu
                    .items
                    .iter()
                    .find(|item| item.key == Some(chord))
                    .map(|item| item.id);
                match bound {
                    Some(id) => self.choose(index, id),
                    None => Outcome::Nothing,
                }
            }
        }
    }

    /// Closes the menu and does `id` to the result it was opened on.
    fn choose(&mut self, index: &Index, id: ActionId) -> Outcome {
        self.menu = None;
        match self.perform(index, id) {
            Outcome::Nothing => Outcome::Redraw,
            outcome => outcome,
        }
    }

    /// Tab: enters the selected group, if it is one. The text typed to find
    /// it is cleared — it did its job.
    fn enter_selected(&mut self, index: &Index) -> Outcome {
        if self.showing_columns() {
            return self.column_in(index);
        }
        let Some(hit) = self.selected_hit() else {
            return Outcome::Nothing;
        };
        let entry = &index.entries()[hit.entry as usize];
        if entry.kind != Kind::Group {
            return Outcome::Nothing;
        }
        match entry.node() {
            Some(group) => self.enter(index, Place::Group(group)),
            None => Outcome::Nothing,
        }
    }

    /// Goes inside `place`: it becomes a chip, and the search looks only there.
    fn enter(&mut self, index: &Index, place: Place) -> Outcome {
        self.scopes.push(place);
        if place == Place::Menus && self.menus_browse == MenusBrowse::Columns {
            self.columns = Some(Columns::new(index));
        }
        self.input.clear();
        self.refresh(index);
        Outcome::Redraw
    }

    /// Whether the rows are the focused column of the tray menus.
    fn showing_columns(&self) -> bool {
        self.column_depth().is_some()
    }

    /// How many columns are left of the focused one, while the columns are
    /// what is shown: nothing typed, and no group entered from a search.
    pub fn column_depth(&self) -> Option<usize> {
        let columns = self.columns.as_ref().filter(|columns| columns.showing)?;
        Some(columns.depth())
    }

    /// The columns to draw, left to right: all of them, then the preview of the
    /// selected row. `None` unless the columns are shown.
    pub fn column_panes(&self, index: &Index) -> Option<Vec<PaneView>> {
        let columns = self.columns.as_ref().filter(|columns| columns.showing)?;
        let (focused, left) = columns.panes().split_last()?;
        let mut panes: Vec<PaneView> = left
            .iter()
            .map(|pane| PaneView {
                items: pane.items.clone(),
                selected: Some(pane.selected),
                ..PaneView::default()
            })
            .collect();
        panes.push(PaneView {
            items: focused.items.clone(),
            selected: self.selected,
            waiting: focused.waiting.is_some(),
            details: None,
        });
        let selected = self.selected_hit().map(|hit| hit.entry);
        panes.push(match Columns::preview(index, selected) {
            Preview::Items(items) => PaneView {
                items,
                ..PaneView::default()
            },
            Preview::Details(entry) => PaneView {
                details: Some(entry),
                ..PaneView::default()
            },
            Preview::Nothing => PaneView::default(),
        });
        Some(panes)
    }

    /// The entries the focused column was reached through: the breadcrumb.
    pub fn column_crumbs(&self) -> Vec<u32> {
        self.columns.as_ref().map_or_else(Vec::new, Columns::crumbs)
    }

    /// `→`: the column of the selected group or browsed folder opens, focused.
    fn column_in(&mut self, index: &Index) -> Outcome {
        let Some(hit) = self.selected_hit() else {
            return Outcome::Nothing;
        };
        let selected = self.selected.unwrap_or(0);
        let Some(columns) = self.columns.as_mut() else {
            return Outcome::Nothing;
        };
        columns.focused_mut().selected = selected;
        let Some(pane) = columns.open(index, hit.entry) else {
            return Outcome::Nothing;
        };
        let waiting = pane.waiting;
        self.show_column(index);
        waiting.map_or(Outcome::Redraw, Outcome::ListFolder)
    }

    /// `←`: back to the column before, its selection where it was.
    fn column_back(&mut self, index: &Index) -> Outcome {
        if !self.columns.as_mut().is_some_and(Columns::back) {
            return Outcome::Nothing;
        }
        self.show_column(index);
        Outcome::Redraw
    }

    /// Makes the last column the rows, at its own selection.
    fn show_column(&mut self, index: &Index) {
        self.selected = self
            .columns
            .as_ref()
            .map(|columns| columns.focused().selected);
        self.refresh(index);
    }

    /// A folder's listing is here: `items` are its files, as index entries. The
    /// column may be gone by now, and then nothing happens.
    pub fn column_listed(&mut self, index: &Index, folder: u32, items: Vec<u32>) {
        let Some(columns) = self.columns.as_mut() else {
            return;
        };
        if columns.listed(folder, items) && columns.showing {
            self.show_column(index);
        }
    }

    /// Row `row` of column `pane` becomes the selection, closing the columns
    /// right of it. `pane` one past the last is the preview, which opens first.
    pub fn column_select(&mut self, index: &Index, pane: usize, row: usize) -> Outcome {
        let Some(depth) = self.column_depth() else {
            return Outcome::Nothing;
        };
        if pane == depth + 1 && self.column_in(index) == Outcome::Nothing {
            return Outcome::Nothing;
        }
        let Some(columns) = self.columns.as_mut() else {
            return Outcome::Nothing;
        };
        let holds = |pane: &super::columns::Pane| row < pane.items.len();
        if !columns.panes().get(pane).is_some_and(holds) {
            return Outcome::Nothing;
        }
        columns.focus(pane);
        columns.focused_mut().selected = row;
        self.show_column(index);
        Outcome::Redraw
    }

    /// A click on a row of a column: selects it, and goes in if something opens
    /// from it. An app is opened from the details, whose buttons say so.
    pub fn column_click(&mut self, index: &Index, pane: usize, row: usize) -> Outcome {
        let selected = self.column_select(index, pane, row);
        match self.selected_hit() {
            Some(hit)
                if selected != Outcome::Nothing
                    && Columns::opens(&index.entries()[hit.entry as usize]) =>
            {
                match self.column_in(index) {
                    Outcome::Nothing => selected,
                    outcome => outcome,
                }
            }
            _ => selected,
        }
    }

    /// The keys that mean something else in the columns. `None`: the box's
    /// usual meaning, which is what moves the selection and runs the actions.
    fn column_key(&mut self, index: &Index, key: Key, mods: Mods) -> Option<Outcome> {
        let plain = mods == Mods::default();
        let back_key = (plain && matches!(key, Key::Left | Key::Backspace))
            || (key == Key::Tab && mods.shift && !mods.ctrl && !mods.alt);
        let opens = self
            .selected_hit()
            .is_some_and(|hit| Columns::opens(&index.entries()[hit.entry as usize]));
        match key {
            Key::Right | Key::Tab if plain => Some(self.column_in(index)),
            Key::Enter if plain && opens => Some(self.column_in(index)),
            // From the first column these leave the section, as anywhere.
            _ if back_key && self.column_depth().is_some_and(|depth| depth > 0) => {
                Some(self.column_back(index))
            }
            Key::Home if plain => Some(self.step(isize::MIN)),
            Key::End if plain => Some(self.step(isize::MAX)),
            _ => None,
        }
    }

    /// The actions the details of the selected row show as buttons.
    pub fn details_buttons(&self, index: &Index) -> Vec<ActionId> {
        self.available_actions(index)
            .into_iter()
            .map(|action| action.id)
            .filter(|id| matches!(id, ActionId::Open | ActionId::Reveal | ActionId::Edit))
            .collect()
    }

    /// A click on the `n`th of [`Self::details_buttons`].
    pub fn details_click(&mut self, index: &Index, n: usize) -> Outcome {
        match self.details_buttons(index).get(n) {
            Some(&id) => self.perform(index, id),
            None => Outcome::Nothing,
        }
    }

    /// The shortcut of `id`, for the footer.
    pub fn key_of(&self, id: ActionId) -> Option<Chord> {
        self.actions
            .actions
            .iter()
            .find(|binding| binding.id == id)
            .and_then(|binding| binding.key)
    }

    /// The letters of the A–Z list being shown; empty when none is.
    pub fn letters(&self) -> &[char] {
        &self.letters
    }

    /// A click on the `n`th of [`Self::letters`]: the first row under that letter
    /// is selected. Gives the row of its header, for the list to start there.
    pub fn letter_click(&mut self, n: usize) -> Option<usize> {
        let letter = *self.letters.get(n)?;
        let header = self
            .rows
            .iter()
            .position(|row| *row == Row::Letter(letter))?;
        self.selected = Some(header + 1);
        Some(header)
    }

    /// `browse="a-z"`: every app of the tray menus by name, under its letter.
    fn list_a_to_z(&mut self, index: &Index) {
        let mut apps: Vec<(char, &str, u32)> = index
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.source == Source::Menu && entry.kind == Kind::App && !entry.left_out
            })
            .map(|(position, entry)| (letter_of(entry.name()), entry.name(), position as u32))
            .collect();
        // Ties stay in file order: the position comes last.
        apps.sort_unstable();
        for (letter, _, entry) in apps {
            if self.letters.last() != Some(&letter) {
                self.letters.push(letter);
                self.rows.push(Row::Letter(letter));
            }
            self.rows.push(Row::Result(Hit {
                entry,
                score: 0,
                matched: Field::Name,
            }));
        }
    }

    /// Enter, `→` or `Tab` on a section row of the empty box.
    fn enter_section(&mut self, index: &Index) -> Outcome {
        match self.selected.and_then(|row| self.rows.get(row)) {
            Some(Row::Section(section)) => {
                let place = section.place;
                self.enter(index, place)
            }
            _ => Outcome::Nothing,
        }
    }

    /// Whether the selected row is a line of a prefixed list.
    fn on_line(&self) -> bool {
        matches!(
            self.selected.and_then(|row| self.rows.get(row)),
            Some(Row::Line(_))
        )
    }

    /// Enter or a click on the selected line.
    fn activate_line(&mut self, index: &Index) -> Outcome {
        match self.selected.and_then(|row| self.rows.get(row)) {
            Some(Row::Line(Line::Command(command))) => Outcome::Command(*command),
            Some(Row::Line(Line::Prefix(prefix))) => {
                let prefix = *prefix;
                self.type_prefix(index, prefix)
            }
            Some(Row::Line(Line::SearchHotkey)) => Outcome::Close,
            Some(Row::Line(Line::Check)) => Outcome::CheckKey,
            _ => Outcome::Nothing,
        }
    }

    /// Types a prefix into an empty box, the way a click on its chip does.
    pub fn type_prefix(&mut self, index: &Index, prefix: char) -> Outcome {
        self.input.set_text(&prefix.to_string());
        self.settle(index);
        Outcome::Redraw
    }

    /// A click on the `n`th chip of the line of prefixes.
    pub fn prefix_click(&mut self, index: &Index, n: usize) -> Outcome {
        match PREFIXES.get(n) {
            Some(info) => self.type_prefix(index, info.prefix),
            None => Outcome::Nothing,
        }
    }

    /// The search hotkey, to list under `#`.
    pub fn set_search_hotkey(&mut self, hotkey: Option<KeyCombo>) {
        self.search_hotkey = hotkey;
    }

    /// The combination `#` is being asked about, when nothing in the file uses
    /// it: the text the window should try to register.
    pub fn check(&self) -> Option<&str> {
        self.check.as_deref()
    }

    /// Whether `/` is in the box, so the folders should be listed.
    pub fn wants_files(&self) -> bool {
        self.prefix() == Some('/')
    }

    /// The listing of `/` got further: `index` has more entries, or `progress`
    /// has news. The new rows join without moving the selection, which stays on
    /// the same result (and an open action menu with it) wherever that lands.
    pub fn files_changed(&mut self, index: &Index, progress: Progress) {
        self.scan = progress;
        if !self.wants_files() {
            return;
        }
        let menu = self.menu.take();
        let kept = self.selected_hit();
        self.refresh(index);
        let row = kept.and_then(|kept| {
            self.rows
                .iter()
                .position(|row| matches!(row, Row::Result(hit) if hit.entry == kept.entry))
        });
        if let Some(row) = row {
            self.selected = Some(row);
            self.menu = menu.map(|menu| ActionMenu { row, ..menu });
        }
    }

    /// The prefix typed first, if it starts a list of its own. Only on the
    /// outermost box: inside a section or a group a `>` is text.
    fn prefix(&self) -> Option<char> {
        if !self.scopes.is_empty() {
            return None;
        }
        self.input
            .text()
            .chars()
            .next()
            .filter(|c| PREFIXES.iter().any(|info| info.prefix == *c))
    }

    /// Whether the selected row is a section of the empty box.
    fn on_section(&self) -> bool {
        matches!(
            self.selected.and_then(|row| self.rows.get(row)),
            Some(Row::Section(_))
        )
    }

    /// Back out of the innermost place. Back on the empty box, the section it
    /// was opened from is selected again.
    fn leave_scope(&mut self, index: &Index) -> Outcome {
        let Some(left) = self.scopes.pop() else {
            return Outcome::Nothing;
        };
        if left == Place::Menus {
            self.columns = None;
        }
        self.refresh(index);
        let row = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Section(section) if section.place == left));
        if row.is_some() {
            self.selected = row;
        }
        Outcome::Redraw
    }

    pub fn handle(&mut self, index: &Index, event: Event) -> Outcome {
        // An open action menu takes the keys; typing waits.
        if self.menu.is_some() {
            return match event {
                Event::Key(key, mods) => self.menu_keypress(index, key, mods),
                Event::Char(_) | Event::Paste(_) => Outcome::Nothing,
            };
        }
        match event {
            Event::Char(c) => self.edited(index, |input| input.insert(c.encode_utf8(&mut [0; 4]))),
            Event::Paste(text) => self.edited(index, |input| input.insert(&text)),
            Event::Key(key, mods) => self.key(index, key, mods),
        }
    }

    fn key(&mut self, index: &Index, key: Key, mods: Mods) -> Outcome {
        let Mods { ctrl, shift, .. } = mods;

        if self.showing_columns()
            && let Some(outcome) = self.column_key(index, key, mods)
        {
            return outcome;
        }

        // On a line of a prefixed list Enter does what it says.
        if self.on_line() && mods == Mods::default() && key == Key::Enter {
            return self.activate_line(index);
        }

        // On a section of the empty box Enter, `→` and Tab go inside it.
        if self.on_section()
            && mods == Mods::default()
            && matches!(key, Key::Enter | Key::Right | Key::Tab)
        {
            return self.enter_section(index);
        }

        // What the configuration binds to a shortcut comes first. (A shortcut
        // that would clash with editing or the arrows cannot be configured.)
        let chord = Chord::new(key, mods);
        if key == Key::ContextMenu || chord == self.actions.menu_key {
            return self.open_menu(index);
        }
        if let Some(binding) = self.actions.actions.iter().find(|b| b.key == Some(chord)) {
            let id = binding.id;
            return self.perform(index, id);
        }

        match key {
            // The text first, then the groups entered, then the window.
            Key::Escape if !self.input.is_empty() => self.edited(index, Input::clear),
            Key::Escape if !self.scopes.is_empty() => self.leave_scope(index),
            Key::Escape if self.collapsible && !self.folded => self.fold(index),
            Key::Escape => Outcome::Close,

            Key::Digit(n) if mods.alt && self.actions.quick_pick => self.pick(index, n as usize),
            // Enter, digits and function keys do only what they are bound to.
            Key::Enter | Key::Digit(_) | Key::Function(_) | Key::ContextMenu => Outcome::Nothing,

            Key::Down | Key::PageDown if self.folded => self.unfold(index),
            Key::Up if self.at_top_of_empty_box() => self.fold(index),
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::PageUp => self.step(-(PAGE as isize)),
            Key::PageDown => self.step(PAGE as isize),

            // `←` on an empty box comes back out of a section, like Backspace.
            Key::Left if self.input.is_empty() && !self.scopes.is_empty() => {
                self.leave_scope(index)
            }
            Key::Left => self.caret(|input| input.move_left(shift, ctrl)),
            Key::Right => self.caret(|input| input.move_right(shift, ctrl)),
            Key::Home => self.caret(|input| input.home(shift)),
            Key::End => self.caret(|input| input.end(shift)),

            // On an empty box, Backspace takes the last chip away.
            Key::Backspace if self.input.is_empty() => self.leave_scope(index),
            Key::Backspace if ctrl => self.edited(index, Input::backspace_word),
            Key::Backspace => self.edited(index, Input::backspace),
            Key::Delete if ctrl => self.edited(index, Input::delete_word),
            Key::Delete => self.edited(index, Input::delete),

            Key::Tab if shift => self.leave_scope(index),
            Key::Tab => self.enter_selected(index),

            Key::Letter('A') if ctrl => self.caret(Input::select_all),
            // Copying is text only; the path of a result is an action.
            Key::Letter('C') if ctrl && self.input.has_selection() => {
                Outcome::Copy(self.input.selected_text().to_owned())
            }
            Key::Letter('X') if ctrl => {
                let cut = self.input.cut();
                match cut {
                    Some(text) => {
                        self.settle(index);
                        // The window copies first, then redraws.
                        Outcome::Copy(text)
                    }
                    None => Outcome::Nothing,
                }
            }
            Key::Letter('V') if ctrl => Outcome::Paste,
            Key::Letter(_) => Outcome::Nothing,
        }
    }

    /// An edit of the text: searches again if it changed.
    fn edited(&mut self, index: &Index, edit: impl FnOnce(&mut Input) -> bool) -> Outcome {
        if edit(&mut self.input) {
            self.settle(index);
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    /// A caret move or a selection change: always worth a redraw.
    fn caret(&mut self, change: impl FnOnce(&mut Input)) -> Outcome {
        change(&mut self.input);
        Outcome::Redraw
    }

    /// Moves the selection `by` results, stopping at the first and the last
    /// (no wrapping), skipping headers.
    fn step(&mut self, by: isize) -> Outcome {
        let results: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(row, kind)| kind.selectable().then_some(row))
            .collect();
        let Some(&first) = results.first() else {
            return Outcome::Nothing;
        };
        let current = self
            .selected
            .and_then(|row| results.iter().position(|&candidate| candidate == row))
            .map_or(first, |position| {
                position.saturating_add_signed(by).min(results.len() - 1)
            });
        let moved = self.selected != Some(results[current]);
        self.selected = Some(results[current]);
        if moved {
            Outcome::Redraw
        } else {
            Outcome::Nothing
        }
    }

    /// Searches for the text as it is now and rebuilds the rows, the best
    /// result selected.
    ///
    /// With nothing typed and nothing entered the rows are the **Recent**
    /// launches, if there are any, then the **Browse** sections. Inside a place,
    /// an empty box lists what it holds, in file order.
    fn refresh(&mut self, index: &Index) {
        // The rows are about to go: a column keeps its selection for later.
        if let Some(columns) = self.columns.as_mut().filter(|columns| columns.showing) {
            columns.focused_mut().selected = self.selected.unwrap_or(0);
            columns.showing = false;
        }
        self.letters.clear();
        self.menu = None;
        self.query = Query::new(self.input.text());
        self.rows.clear();
        if self.folded {
            self.selected = None;
            return;
        }

        self.check = None;
        if let Some(prefix) = self.prefix() {
            let rest = self.input.text()[prefix.len_utf8()..].trim().to_owned();
            self.query = Query::new(&rest);
            self.list_prefixed(index, prefix, &rest);
            self.selected = self.first_selectable();
            return;
        }

        if self.query.is_empty() {
            let listed = |keep: &dyn Fn(&super::index::Entry) -> bool| -> Vec<Row> {
                index
                    .entries()
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| keep(entry))
                    .map(|(position, _)| {
                        Row::Result(Hit {
                            entry: position as u32,
                            score: 0,
                            matched: Field::Name,
                        })
                    })
                    .collect()
            };
            match self.scopes.last().copied() {
                // The empty box: the last launches, then the sections.
                None => {
                    let shown = self.recent.len().min(self.recent_shown);
                    if !self.recent.is_empty() {
                        self.rows.push(Row::Header(Heading::Recent));
                        self.rows
                            .extend(self.recent[..shown].iter().map(Self::recent_result));
                    }
                    if self.recent.len() > shown {
                        self.rows.push(Row::Section(SectionRow {
                            place: Place::Recent,
                            apps: self.recent.len(),
                        }));
                    }
                    if !self.sections.is_empty() {
                        self.rows.push(Row::Header(Heading::Browse));
                        self.rows
                            .extend(self.sections.iter().copied().map(Row::Section));
                    }
                    self.rows.push(Row::Prefixes);
                }
                Some(Place::Menus) if self.columns.is_some() => {
                    self.list_column();
                    return;
                }
                Some(Place::Menus) if self.menus_browse == MenusBrowse::AZ => {
                    self.list_a_to_z(index);
                }
                // The tray icons; a group is entered as a chip, like anywhere.
                Some(Place::Menus) => self.rows.extend(listed(&|entry| {
                    entry.source == Source::Menu && entry.ancestors.is_empty() && !entry.left_out
                })),
                Some(Place::Group(group)) => self
                    .rows
                    .extend(listed(&|entry| entry.ancestors.last() == Some(&group))),
                Some(Place::Recent) => self
                    .rows
                    .extend(self.recent.iter().map(Self::recent_result)),
            }
        } else {
            let mut hits = self
                .searcher
                .search(index, &self.query, self.scope())
                .to_vec();
            // All of it, in one list: nobody came here for the five best.
            let layout = if self.scopes.last() == Some(&Place::Recent) {
                let recent = &self.recent;
                hits.retain(|hit| recent.iter().any(|row| row.entry == hit.entry));
                ResultsLayout::Flat
            } else {
                self.layout
            };
            for group in results::arrange(index, &hits, layout) {
                if group.heading != Heading::All {
                    self.rows.push(Row::Header(group.heading));
                }
                self.rows.extend(group.hits.into_iter().map(Row::Result));
            }
        }
        self.selected = self.first_selectable();
    }

    /// The focused column as the rows, at its own selection.
    fn list_column(&mut self) {
        let Some(columns) = self.columns.as_mut() else {
            return;
        };
        columns.showing = true;
        let pane = columns.focused();
        self.rows.extend(pane.items.iter().map(|&entry| {
            Row::Result(Hit {
                entry,
                score: 0,
                matched: Field::Name,
            })
        }));
        let last = pane.items.len().checked_sub(1);
        self.selected = last.map(|last| pane.selected.min(last));
    }

    fn recent_result(recent: &Recent) -> Row {
        Row::Result(Hit {
            entry: recent.entry,
            score: 0,
            matched: Field::Name,
        })
    }

    /// The rows of a list a prefix asked for (spec §2.7): `rest` is what was
    /// typed after it.
    fn list_prefixed(&mut self, index: &Index, prefix: char, rest: &str) {
        match prefix {
            '>' => {
                let mut scored: Vec<(u32, Command)> = Vec::new();
                for command in Command::ALL {
                    let by_label = self
                        .searcher
                        .score_text(&self.query, &fold(command.label()));
                    // Words that are not in the label count for less.
                    let by_words = self
                        .searcher
                        .score_text(&self.query, &fold(command.keywords()))
                        .map(|score| score * 6 / 10);
                    if let Some(score) = by_label.max(by_words) {
                        scored.push((score, command));
                    }
                }
                scored.sort_by_key(|&(score, _)| std::cmp::Reverse(score));
                if !scored.is_empty() {
                    self.rows.push(Row::Header(Heading::Commands));
                    self.rows.extend(
                        scored
                            .into_iter()
                            .map(|(_, command)| Row::Line(Line::Command(command))),
                    );
                }
            }
            '#' => self.list_hotkeys(index, rest),
            '/' => {
                let hits = if rest.is_empty() {
                    index
                        .entries()
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| entry.source == Source::BrowsedFile)
                        .map(|(position, _)| Hit {
                            entry: position as u32,
                            score: 0,
                            matched: Field::Name,
                        })
                        .collect()
                } else {
                    self.searcher
                        .search(index, &self.query, Scope::Files)
                        .to_vec()
                };
                if !hits.is_empty() {
                    self.rows.push(Row::Header(Heading::Files));
                    self.rows.extend(hits.into_iter().map(Row::Result));
                }
                // Last: the rows above it do not move when it goes.
                if self.scan.worth_a_row() {
                    self.rows.push(Row::Line(Line::Scan));
                }
            }
            '?' => {
                let mut lines = Vec::new();
                for info in PREFIXES {
                    let text = fold(&format!(
                        "{} {} {}",
                        info.prefix, info.name, info.description
                    ));
                    if self.searcher.score_text(&self.query, &text).is_some() {
                        lines.push(Row::Line(Line::Prefix(info.prefix)));
                    }
                }
                if !lines.is_empty() {
                    self.rows.push(Row::Header(Heading::Prefixes));
                    self.rows.extend(lines);
                }
            }
            _ => {}
        }
    }

    /// `#`: every binding of the file (the search hotkey too), filtered by what
    /// follows; and, for a whole combination that nothing in the file uses, one
    /// more row that asks the system whether it is free (spec §2.14).
    fn list_hotkeys(&mut self, index: &Index, rest: &str) {
        let compact: String = rest.split_whitespace().collect();
        let typed = KeyCombo::is_query(&compact)
            .then(|| KeyCombo::parse(&compact))
            .flatten();
        let mut rows = Vec::new();

        if rest.is_empty() {
            if self.search_hotkey.is_some() {
                rows.push(Row::Line(Line::SearchHotkey));
            }
            rows.extend(
                index
                    .entries()
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| !entry.hotkeys().is_empty())
                    .map(|(position, _)| {
                        Row::Result(Hit {
                            entry: position as u32,
                            score: 0,
                            matched: Field::Hotkey,
                        })
                    }),
            );
        } else {
            let own = typed.as_ref().zip(self.search_hotkey.as_ref());
            if own.is_some_and(|(typed, own)| typed.match_score(own).is_some()) {
                rows.push(Row::Line(Line::SearchHotkey));
            }
            let hits = self
                .searcher
                .search(index, &self.query, Scope::Hotkeys)
                .to_vec();
            rows.extend(hits.into_iter().map(Row::Result));

            if let Some(combo) = typed
                && y_core::hotkey::Hotkey::parse(rest).is_some()
            {
                let used = self.search_hotkey.as_ref() == Some(&combo)
                    || index
                        .entries()
                        .iter()
                        .any(|entry| entry.hotkeys().contains(&combo));
                if !used {
                    self.check = Some(rest.to_owned());
                    rows.push(Row::Line(Line::Check));
                }
            }
        }

        if !rows.is_empty() {
            self.rows.push(Row::Header(Heading::Hotkeys));
            self.rows.extend(rows);
        }
    }
}

/// The header a name goes under in an A–Z list. Names are folded already.
fn letter_of(name: &str) -> char {
    name.chars()
        .next()
        .filter(|c| c.is_alphabetic())
        .map_or('#', |c| c.to_uppercase().next().unwrap_or(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::variables::Variable;

    fn index() -> Index {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(
            r#"<y-pop><group name="Work">
                <app name="Outlook" exe="outlook.exe"/>
                <app name="Open Utility Tool" exe="tool.exe"/>
                <app name="Notes" exe="notes.exe"/>
                <app name="Notepad" exe="notepad.exe"/>
                <app name="Note Taker" exe="taker.exe"/>
                <app name="Notable" exe="notable.exe"/>
                <app name="Notion" exe="notion.exe"/>
                <app name="Nothing" exe="nothing.exe"/>
                <app name="Elevated" exe="elevated.exe" elevate="true"/>
                <app name="Plain Program" exe="plain.exe"/>
                <group name="Servers">
                    <app name="Remote Desktop" exe="mstsc.exe"/>
                    <app name="Putty" exe="putty.exe"/>
                </group>
            </group></y-pop>"#,
            &mut builtins,
        )
        .unwrap();
        Index::from_tree(&tree)
    }

    fn state() -> State {
        State::new(ResultsLayout::Sections, ActionsConfig::default())
    }

    fn type_text(state: &mut State, index: &Index, text: &str) {
        for c in text.chars() {
            state.handle(index, Event::Char(c));
        }
    }

    fn press(state: &mut State, index: &Index, key: Key) -> Outcome {
        state.handle(index, Event::Key(key, Mods::default()))
    }

    fn with(ctrl: bool, shift: bool) -> Mods {
        Mods {
            ctrl,
            shift,
            alt: false,
        }
    }

    fn name_of(index: &Index, hit: Hit) -> &str {
        index.entries()[hit.entry as usize].name()
    }

    #[test]
    fn typing_searches_and_selects_the_best_result() {
        let (mut state, index) = (state(), index());
        assert!(state.rows().is_empty(), "nothing typed, nothing listed");

        assert_eq!(state.handle(&index, Event::Char('o')), Outcome::Redraw);
        type_text(&mut state, &index, "ut");
        assert_eq!(state.input().text(), "out");

        let hit = state.selected_hit().expect("a result is selected");
        assert_eq!(name_of(&index, hit), "outlook");
        assert_eq!(state.rows()[0], Row::Header(Heading::TopHit));
        assert_eq!(state.selected(), Some(1), "the row after the header");
        assert_eq!(state.row_kinds()[..2], [RowKind::Header, RowKind::Item]);
    }

    #[test]
    fn control_characters_are_not_typed() {
        let (mut state, index) = (state(), index());
        for c in ['\u{1}', '\r', '\u{8}', '\u{1b}', '\t'] {
            let outcome = state.handle(&index, Event::Char(c));
            // Tab and CR become spaces in a paste, but a typed one is a key.
            assert!(
                matches!(outcome, Outcome::Nothing | Outcome::Redraw),
                "{c:?}"
            );
        }
        assert!(!state.input().text().chars().any(char::is_control));
    }

    #[test]
    fn up_and_down_walk_the_results_skipping_headers_and_stopping_at_the_ends() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");
        // Top hit + Apps group: 2 headers, 7 results.
        let results = state.rows().len() - 2;
        assert!(results >= 6);

        let top = state.selected().unwrap();
        assert_eq!(
            press(&mut state, &index, Key::Up),
            Outcome::Nothing,
            "already first"
        );
        assert_eq!(state.selected(), Some(top));

        assert_eq!(press(&mut state, &index, Key::Down), Outcome::Redraw);
        let second = state.selected().unwrap();
        assert_eq!(
            state.rows()[second - 1],
            Row::Header(Heading::Apps),
            "stepped over the second header"
        );
        for _ in 0..20 {
            press(&mut state, &index, Key::Down);
        }
        assert_eq!(
            state.selected(),
            Some(state.rows().len() - 1),
            "stops at the last"
        );
        assert_eq!(press(&mut state, &index, Key::Down), Outcome::Nothing);

        press(&mut state, &index, Key::PageUp);
        let after_page = state.selected().unwrap();
        assert!(after_page < state.rows().len() - 1);
        for _ in 0..5 {
            press(&mut state, &index, Key::PageUp);
        }
        assert_eq!(state.selected(), Some(top), "back at the first result");
    }

    #[test]
    fn page_keys_move_by_a_page_of_results() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");
        let first = state.selected().unwrap();
        press(&mut state, &index, Key::PageDown);
        let moved = state.selected().unwrap();
        let results_between = state.rows()[first..moved]
            .iter()
            .filter(|row| matches!(row, Row::Result(_)))
            .count();
        assert_eq!(results_between, PAGE);
    }

    #[test]
    fn enter_activates_the_selected_result() {
        let (mut state, index) = (state(), index());
        assert_eq!(
            press(&mut state, &index, Key::Enter),
            Outcome::Nothing,
            "nothing selected"
        );

        type_text(&mut state, &index, "notepad");
        let Outcome::Activate(hit, ActionId::Open) = press(&mut state, &index, Key::Enter) else {
            panic!("expected an activation")
        };
        assert_eq!(name_of(&index, hit), "notepad");
        assert_eq!(state.selected_hit(), Some(hit), "and it stays selected");
    }

    #[test]
    fn hover_selects_results_only_and_click_activates() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");

        assert!(!state.select_row(0), "a header");
        assert!(!state.select_row(999), "out of range");
        let row = state
            .rows()
            .iter()
            .rposition(|row| matches!(row, Row::Result(_)))
            .unwrap();
        assert!(state.select_row(row));
        assert!(!state.select_row(row), "already selected");
        assert_eq!(state.selected(), Some(row));

        let first = state
            .rows()
            .iter()
            .position(|r| matches!(r, Row::Result(_)))
            .unwrap();
        let Outcome::Activate(hit, ActionId::Open) = state.click_row(&index, first) else {
            panic!("a click on a result activates it")
        };
        assert_eq!(state.selected(), Some(first));
        assert_eq!(Some(hit), state.selected_hit());
        assert_eq!(state.click_row(&index, 0), Outcome::Nothing, "a header");
    }

    #[test]
    fn editing_keys_search_again() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "outq");
        assert!(state.rows().is_empty(), "no match for outq");

        assert_eq!(press(&mut state, &index, Key::Backspace), Outcome::Redraw);
        assert_eq!(state.input().text(), "out");
        assert!(!state.rows().is_empty(), "and out matches again");

        press(&mut state, &index, Key::Home);
        press(&mut state, &index, Key::Delete);
        assert_eq!(state.input().text(), "ut");

        state.handle(&index, Event::Key(Key::Backspace, with(true, false)));
        assert_eq!(
            state.input().text(),
            "ut",
            "Ctrl+Backspace at the start does nothing"
        );
        press(&mut state, &index, Key::End);
        state.handle(&index, Event::Key(Key::Backspace, with(true, false)));
        assert_eq!(state.input().text(), "");
        assert!(only_prefixes(&state));
    }

    #[test]
    fn escape_clears_the_text_then_closes() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.input().is_empty());
        assert!(only_prefixes(&state));
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Close);
    }

    #[test]
    fn the_caret_keys_edit_the_selection_not_the_results() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "note");
        let selected = state.selected();

        state.handle(&index, Event::Key(Key::Left, with(false, true)));
        state.handle(&index, Event::Key(Key::Left, with(false, true)));
        assert_eq!(state.input().selection(), 2..4);
        assert_eq!(state.selected(), selected, "the results did not move");

        state.handle(&index, Event::Key(Key::Letter('A'), with(true, false)));
        assert_eq!(state.input().selection(), 0..4);
    }

    #[test]
    fn copy_cut_and_paste_go_through_the_window() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "notepad");

        let ctrl = with(true, false);
        // Nothing selected in the box: Ctrl+C is not a result's action.
        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('C'), ctrl)),
            Outcome::Nothing
        );

        state.handle(&index, Event::Key(Key::Letter('A'), ctrl));
        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('C'), ctrl)),
            Outcome::Copy("notepad".into())
        );
        assert_eq!(state.input().text(), "notepad", "copy leaves it");

        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('X'), ctrl)),
            Outcome::Copy("notepad".into())
        );
        assert!(state.input().is_empty());
        assert!(only_prefixes(&state), "and the results follow");

        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('V'), ctrl)),
            Outcome::Paste
        );
        assert_eq!(
            state.handle(&index, Event::Paste("out\r\nlook".into())),
            Outcome::Redraw
        );
        assert_eq!(state.input().text(), "out  look");

        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('Q'), ctrl)),
            Outcome::Nothing
        );
    }

    fn names(state: &State, index: &Index) -> Vec<String> {
        state
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Result(hit) => Some(name_of(index, *hit).to_owned()),
                _ => None,
            })
            .collect()
    }

    fn shift() -> Mods {
        with(false, true)
    }

    #[test]
    fn enter_on_a_group_goes_inside_it_instead_of_acting() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        assert_eq!(name_of(&index, state.selected_hit().unwrap()), "servers");

        assert_eq!(press(&mut state, &index, Key::Enter), Outcome::Redraw);
        assert_eq!(state.scopes().len(), 1);
        assert!(state.input().is_empty(), "the text that found it is gone");
        assert_eq!(names(&state, &index), ["remote desktop", "putty"]);
        assert!(
            state.rows().iter().all(|row| matches!(row, Row::Result(_))),
            "a plain list, in file order"
        );
        assert_eq!(state.selected(), Some(0));

        // And now Enter acts on what is inside.
        let Outcome::Activate(hit, ActionId::Open) = press(&mut state, &index, Key::Enter) else {
            panic!("an app is activated")
        };
        assert_eq!(name_of(&index, hit), "remote desktop");
    }

    #[test]
    fn tab_enters_a_group_but_does_nothing_on_an_app() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "notepad");
        assert_eq!(press(&mut state, &index, Key::Tab), Outcome::Nothing);
        assert!(state.scopes().is_empty());

        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        type_text(&mut state, &index, "servers");
        assert_eq!(press(&mut state, &index, Key::Tab), Outcome::Redraw);
        assert_eq!(state.scopes().len(), 1);
    }

    #[test]
    fn inside_a_group_a_search_looks_only_there() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        press(&mut state, &index, Key::Tab);

        type_text(&mut state, &index, "put");
        assert_eq!(names(&state, &index), ["putty"]);
        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        type_text(&mut state, &index, "notepad");
        assert!(state.rows().is_empty(), "notepad is outside the group");
    }

    #[test]
    fn backspace_on_an_empty_box_and_shift_tab_leave_the_group() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        press(&mut state, &index, Key::Tab);

        type_text(&mut state, &index, "pu");
        assert_eq!(press(&mut state, &index, Key::Backspace), Outcome::Redraw);
        assert_eq!(
            state.input().text(),
            "p",
            "Backspace edits while there is text"
        );
        assert_eq!(state.scopes().len(), 1);
        press(&mut state, &index, Key::Backspace);
        assert_eq!(press(&mut state, &index, Key::Backspace), Outcome::Redraw);
        assert!(
            state.scopes().is_empty(),
            "and leaves once the box is empty"
        );
        assert!(only_prefixes(&state));
        assert_eq!(
            press(&mut state, &index, Key::Backspace),
            Outcome::Nothing,
            "nothing left to take away"
        );

        type_text(&mut state, &index, "servers");
        press(&mut state, &index, Key::Tab);
        assert_eq!(
            state.handle(&index, Event::Key(Key::Tab, shift())),
            Outcome::Redraw
        );
        assert!(state.scopes().is_empty());
        assert_eq!(
            state.handle(&index, Event::Key(Key::Tab, shift())),
            Outcome::Nothing
        );
    }

    #[test]
    fn escape_clears_the_text_then_leaves_the_group_then_closes() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        press(&mut state, &index, Key::Tab);
        type_text(&mut state, &index, "pu");

        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.input().is_empty());
        assert_eq!(state.scopes().len(), 1, "still inside");
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.scopes().is_empty());
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Close);
    }

    #[test]
    fn groups_nest_and_each_level_lists_its_own_children() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "work");
        press(&mut state, &index, Key::Tab);
        let inside_work = names(&state, &index);
        assert!(inside_work.contains(&"servers".to_owned()));
        assert!(inside_work.contains(&"outlook".to_owned()));
        assert!(
            !inside_work.contains(&"putty".to_owned()),
            "that is one level down"
        );

        // Select the Servers group in the list and go in.
        let row = state
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Result(hit) if name_of(&index, *hit) == "servers"))
            .unwrap();
        assert!(state.select_row(row));
        press(&mut state, &index, Key::Tab);
        assert_eq!(state.scopes().len(), 2);
        assert_eq!(names(&state, &index), ["remote desktop", "putty"]);

        press(&mut state, &index, Key::Backspace);
        assert_eq!(state.scopes().len(), 1);
        assert_eq!(names(&state, &index), inside_work, "back to the outer list");
    }

    #[test]
    fn a_click_on_a_group_goes_inside_it() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        let row = state.selected().unwrap();
        assert_eq!(state.click_row(&index, row), Outcome::Redraw);
        assert_eq!(state.scopes().len(), 1);
    }

    fn alt() -> Mods {
        Mods {
            alt: true,
            ..Mods::default()
        }
    }

    /// A state whose `<actions>` is the one written in `actions_xml`.
    fn state_with(actions_xml: &str) -> State {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(
            &format!("<y-pop><search>{actions_xml}</search></y-pop>"),
            &mut builtins,
        )
        .unwrap();
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        State::new(ResultsLayout::Sections, tree.search.unwrap().actions)
    }

    fn shortcut(text: &str) -> Event {
        let chord = Chord::parse(text).unwrap();
        Event::Key(chord.key, chord.mods)
    }

    /// The action an outcome asks for, if it asks for one.
    fn done(outcome: Outcome) -> Option<ActionId> {
        match outcome {
            Outcome::Activate(_, action) => Some(action),
            _ => None,
        }
    }

    #[test]
    fn by_default_only_enter_and_the_two_named_shortcuts_act() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");

        assert_eq!(
            done(state.handle(&index, shortcut("Enter"))),
            Some(ActionId::Open)
        );
        assert_eq!(
            done(state.handle(&index, shortcut("Ctrl+E"))),
            Some(ActionId::Edit)
        );
        assert_eq!(
            done(state.handle(&index, shortcut("Ctrl+R"))),
            Some(ActionId::Reveal)
        );

        // The shortcuts of the first cut are not bound unless asked for.
        for text in ["Ctrl+Enter", "Shift+Enter"] {
            assert_eq!(
                state.handle(&index, shortcut(text)),
                Outcome::Nothing,
                "{text}"
            );
        }
        // And Ctrl+C is the text's: with none selected it copies nothing.
        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('C'), with(true, false))),
            Outcome::Nothing
        );
    }

    #[test]
    fn configured_shortcuts_reach_their_actions() {
        let mut state = state_with(
            r#"<actions>
                <action do="open" key="Enter"/>
                <action do="run-as-admin" key="Ctrl+Shift+Enter"/>
                <action do="show-in-folder" key="Shift+Enter"/>
                <action do="copy-path" key="Ctrl+Alt+C"/>
            </actions>"#,
        );
        let index = index();
        type_text(&mut state, &index, "plain program");

        for (text, action) in [
            ("Enter", ActionId::Open),
            ("Ctrl+Shift+Enter", ActionId::RunAsAdmin),
            ("Shift+Enter", ActionId::ShowInFolder),
            ("Ctrl+Alt+C", ActionId::CopyPath),
        ] {
            assert_eq!(
                done(state.handle(&index, shortcut(text))),
                Some(action),
                "{text}"
            );
        }
        // What the configuration does not list does not exist.
        assert_eq!(state.handle(&index, shortcut("Ctrl+E")), Outcome::Nothing);
    }

    #[test]
    fn open_can_be_moved_off_enter() {
        let mut state = state_with(r#"<actions><action do="open" key="F2"/></actions>"#);
        let index = index();
        type_text(&mut state, &index, "plain program");
        assert_eq!(state.handle(&index, shortcut("Enter")), Outcome::Nothing);
        assert_eq!(
            done(state.handle(&index, shortcut("F2"))),
            Some(ActionId::Open)
        );
    }

    #[test]
    fn a_shortcut_does_nothing_where_its_action_does_not_apply() {
        let mut state = state_with(
            r#"<actions>
                <action do="run-as-admin" key="Ctrl+Shift+Enter"/>
                <action do="copy-path" key="Ctrl+Alt+C"/>
            </actions>"#,
        );
        let index = index();

        type_text(&mut state, &index, "elevated"); // already elevate="true"
        assert_eq!(
            state.handle(&index, shortcut("Ctrl+Shift+Enter")),
            Outcome::Nothing
        );
        assert_eq!(
            done(state.handle(&index, shortcut("Ctrl+Alt+C"))),
            Some(ActionId::CopyPath)
        );

        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        type_text(&mut state, &index, "servers"); // a group
        assert_eq!(
            state.handle(&index, shortcut("Ctrl+Shift+Enter")),
            Outcome::Nothing
        );
        assert_eq!(
            state.handle(&index, shortcut("Ctrl+Alt+C")),
            Outcome::Nothing
        );
        assert!(
            state.scopes().is_empty(),
            "and it did not go inside the group"
        );
    }

    #[test]
    fn the_menu_lists_what_applies_to_the_selection_with_its_shortcuts() {
        let (mut state, index) = (state(), index());
        let listed = |state: &State| -> Vec<(&'static str, Option<String>)> {
            state
                .available_actions(&index)
                .iter()
                .map(|a| (a.id.name(), a.key.map(|key| key.to_string())))
                .collect()
        };

        assert!(listed(&state).is_empty(), "nothing selected");

        type_text(&mut state, &index, "plain program");
        assert_eq!(
            listed(&state),
            [
                ("open", Some("Enter".into())),
                ("run-as-admin", None),
                ("show-in-folder", None),
                ("copy-path", None),
                ("reveal", Some("Ctrl+R".into())),
                ("edit", Some("Ctrl+E".into())),
            ]
        );

        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        type_text(&mut state, &index, "elevated");
        assert!(!listed(&state).iter().any(|(id, _)| *id == "run-as-admin"));

        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        type_text(&mut state, &index, "servers");
        assert_eq!(
            listed(&state).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            ["open", "reveal", "edit"]
        );
    }

    fn menu_ids(state: &State) -> Vec<&'static str> {
        state
            .menu()
            .expect("the menu is open")
            .items
            .iter()
            .map(|a| a.id.name())
            .collect()
    }

    #[test]
    fn the_menu_key_opens_the_menu_on_the_selected_result() {
        let (mut state, index) = (state(), index());
        assert_eq!(
            state.handle(&index, shortcut("Alt+Enter")),
            Outcome::Nothing,
            "no result, no menu"
        );

        type_text(&mut state, &index, "plain program");
        let row = state.selected().unwrap();
        assert_eq!(state.handle(&index, shortcut("Alt+Enter")), Outcome::Redraw);
        let menu = state.menu().unwrap();
        assert_eq!((menu.row, menu.selected), (row, 0));
        assert_eq!(
            menu_ids(&state),
            [
                "open",
                "run-as-admin",
                "show-in-folder",
                "copy-path",
                "reveal",
                "edit"
            ]
        );
    }

    #[test]
    fn the_context_menu_key_and_a_right_click_open_it_too() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");

        assert_eq!(
            state.handle(&index, Event::Key(Key::ContextMenu, Mods::default())),
            Outcome::Redraw
        );
        assert!(state.menu().is_some());
        state.close_menu();

        // A right click selects the row it is on, then opens the menu there.
        let other = state
            .rows()
            .iter()
            .rposition(|row| matches!(row, Row::Result(_)))
            .unwrap();
        assert_ne!(state.selected(), Some(other));
        assert_eq!(state.open_menu_at(&index, other), Outcome::Redraw);
        assert_eq!(state.selected(), Some(other));
        assert_eq!(state.menu().unwrap().row, other);
        assert_eq!(state.open_menu_at(&index, 0), Outcome::Nothing, "a header");
    }

    #[test]
    fn inside_the_menu_the_arrows_wrap_and_enter_chooses_the_lit_line() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");
        state.handle(&index, shortcut("Alt+Enter"));

        let lit = |state: &State| state.menu().unwrap().selected;
        state.handle(&index, Event::Key(Key::Down, Mods::default()));
        state.handle(&index, Event::Key(Key::Down, Mods::default()));
        assert_eq!(lit(&state), 2);
        state.handle(&index, Event::Key(Key::End, Mods::default()));
        assert_eq!(lit(&state), 5);
        state.handle(&index, Event::Key(Key::Down, Mods::default()));
        assert_eq!(lit(&state), 0, "wraps to the top");
        state.handle(&index, Event::Key(Key::Up, Mods::default()));
        assert_eq!(lit(&state), 5, "and to the bottom");

        state.handle(&index, Event::Key(Key::Home, Mods::default()));
        state.handle(&index, Event::Key(Key::Down, Mods::default()));
        // run-as-admin is the second line.
        assert_eq!(
            done(state.handle(&index, Event::Key(Key::Enter, Mods::default()))),
            Some(ActionId::RunAsAdmin)
        );
        assert!(state.menu().is_none(), "choosing closes it");
    }

    #[test]
    fn a_click_on_a_line_chooses_it() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");
        state.handle(&index, shortcut("Alt+Enter"));

        assert!(state.menu_select(3));
        assert!(!state.menu_select(3), "already lit");
        assert!(!state.menu_select(99), "out of range");
        assert_eq!(done(state.menu_click(&index, 3)), Some(ActionId::CopyPath));
        assert!(state.menu().is_none());
        assert_eq!(state.menu_click(&index, 0), Outcome::Nothing, "none open");
    }

    #[test]
    fn escape_and_the_menu_key_close_it_without_acting() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");

        state.handle(&index, shortcut("Alt+Enter"));
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.menu().is_none());
        assert_eq!(
            state.input().text(),
            "plain program",
            "the text is untouched"
        );

        state.handle(&index, shortcut("Alt+Enter"));
        assert_eq!(state.handle(&index, shortcut("Alt+Enter")), Outcome::Redraw);
        assert!(state.menu().is_none(), "the key that opened it closes it");

        state.handle(&index, shortcut("Alt+Enter"));
        state.handle(&index, Event::Key(Key::ContextMenu, Mods::default()));
        assert!(state.menu().is_none());
    }

    #[test]
    fn an_actions_own_shortcut_works_inside_the_menu() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");
        state.handle(&index, shortcut("Alt+Enter"));

        assert_eq!(
            done(state.handle(&index, shortcut("Ctrl+E"))),
            Some(ActionId::Edit)
        );
        assert!(state.menu().is_none());

        // A shortcut of an action that is not in this menu does nothing, and
        // leaves it open.
        type_text(&mut state, &index, "");
        state.handle(&index, shortcut("Alt+Enter"));
        assert_eq!(state.handle(&index, shortcut("Ctrl+Del")), Outcome::Nothing);
        assert!(state.menu().is_some());
    }

    #[test]
    fn typing_waits_while_the_menu_is_open() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");
        state.handle(&index, shortcut("Alt+Enter"));

        assert_eq!(state.handle(&index, Event::Char('x')), Outcome::Nothing);
        assert_eq!(
            state.handle(&index, Event::Paste("more".into())),
            Outcome::Nothing
        );
        assert_eq!(state.input().text(), "plain program");
        assert_eq!(
            state.handle(&index, Event::Key(Key::Backspace, Mods::default())),
            Outcome::Nothing
        );
    }

    #[test]
    fn choosing_open_on_a_group_goes_inside_it() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        state.handle(&index, shortcut("Alt+Enter"));
        assert_eq!(menu_ids(&state), ["open", "reveal", "edit"]);

        assert_eq!(
            state.handle(&index, Event::Key(Key::Enter, Mods::default())),
            Outcome::Redraw
        );
        assert_eq!(state.scopes().len(), 1);
        assert!(state.menu().is_none());
    }

    #[test]
    fn the_menu_key_is_the_configured_one() {
        assert_eq!(state().menu_key().to_string(), "Alt+Enter");
        let state = state_with(r#"<actions menu-key="Ctrl+K"/>"#);
        assert_eq!(state.menu_key().to_string(), "Ctrl+K");
    }

    #[test]
    fn quick_pick_can_be_switched_off() {
        let mut state = state_with(r#"<actions quick-pick="false"/>"#);
        let index = index();
        type_text(&mut state, &index, "not");
        assert!(!state.quick_pick());
        assert_eq!(
            state.handle(&index, Event::Key(Key::Digit(1), alt())),
            Outcome::Nothing
        );
    }

    #[test]
    fn alt_and_a_digit_pick_that_result_whichever_group_it_is_under() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");
        let results = names(&state, &index);
        assert!(results.len() >= 3);

        let Outcome::Activate(hit, ActionId::Open) =
            state.handle(&index, Event::Key(Key::Digit(3), alt()))
        else {
            panic!("Alt+3 opens the third result")
        };
        assert_eq!(name_of(&index, hit), results[2]);

        // Past the last one, and without Alt, nothing happens.
        assert_eq!(
            state.handle(&index, Event::Key(Key::Digit(9), alt())),
            Outcome::Nothing
        );
        assert_eq!(
            state.handle(&index, Event::Key(Key::Digit(1), Mods::default())),
            Outcome::Nothing
        );
    }

    #[test]
    fn alt_and_a_digit_on_a_group_goes_inside_it() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "servers");
        assert_eq!(
            state.handle(&index, Event::Key(Key::Digit(1), alt())),
            Outcome::Redraw
        );
        assert_eq!(state.scopes().len(), 1);
    }

    #[test]
    fn results_are_numbered_from_one_skipping_headers() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");
        assert_eq!(state.result_number(0), None, "the Top hit header");
        assert_eq!(state.result_number(1), Some(1));
        let numbers: Vec<_> = (0..state.rows().len())
            .filter_map(|row| state.result_number(row))
            .collect();
        assert_eq!(numbers, (1..=numbers.len()).collect::<Vec<_>>());
        assert_eq!(state.result_number(999), None);
    }

    #[test]
    fn a_flat_layout_has_no_headers() {
        let (mut state, index) = (
            State::new(ResultsLayout::Flat, ActionsConfig::default()),
            index(),
        );
        type_text(&mut state, &index, "not");
        assert!(state.rows().iter().all(|row| matches!(row, Row::Result(_))));
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn the_selected_row_shows_the_buttons_of_the_actions_that_ask_and_apply() {
        let (mut state, index) = (state(), index());
        assert!(!state.has_row_buttons(&index), "no result, no buttons");
        assert!(state.row_buttons(&index).is_empty());

        type_text(&mut state, &index, "plain program");
        assert!(state.has_row_buttons(&index));
        // By default reveal and edit ask for a button; forget never applies
        // outside the history, and a program has no "add to a menu".
        let names: Vec<_> = state
            .row_buttons(&index)
            .iter()
            .map(|id| id.name())
            .collect();
        assert_eq!(names, ["reveal", "edit"]);

        let mut state = state_with(r#"<actions><action do="copy-path" button="true"/></actions>"#);
        type_text(&mut state, &index, "plain program");
        let names: Vec<_> = state
            .row_buttons(&index)
            .iter()
            .map(|id| id.name())
            .collect();
        assert_eq!(names, ["copy-path"], "only what is configured as a button");
    }

    fn position(index: &Index, name: &str) -> u32 {
        index
            .entries()
            .iter()
            .position(|entry| entry.name() == name)
            .unwrap() as u32
    }

    /// A state that opened with these launches in its history, newest first.
    fn with_recent(names: &[&str]) -> (State, Index) {
        let (mut state, index) = (state(), index());
        let recent = names
            .iter()
            .enumerate()
            .map(|(n, name)| Recent {
                item: n as u32,
                entry: position(&index, name),
                last: 1000 - n as u64,
            })
            .collect();
        state.start(&index, recent, Vec::new(), Vec::new());
        (state, index)
    }

    /// The box shows nothing but the line of prefixes: the empty box with no
    /// history and no sections.
    fn only_prefixes(state: &State) -> bool {
        state.rows().iter().all(|row| matches!(row, Row::Prefixes))
    }

    fn row_names(state: &State, index: &Index) -> Vec<String> {
        state
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Header(heading) => Some(format!("[{heading:?}]")),
                Row::Letter(letter) => Some(format!("[{letter}]")),
                Row::Result(hit) => Some(index.entries()[hit.entry as usize].name().to_owned()),
                Row::Section(section) => Some(format!("<{:?}>", section.place)),
                Row::Line(line) => Some(format!("{line:?}")),
                // Always last on the empty box: left out so the tests read.
                Row::Prefixes => None,
            })
            .collect()
    }

    #[test]
    fn the_empty_box_lists_the_recent_launches_with_the_last_one_selected_first() {
        let (state, index) = with_recent(&["notepad", "outlook"]);
        assert_eq!(
            row_names(&state, &index),
            ["[Recent]", "notepad", "outlook"]
        );
        assert_eq!(state.selected(), Some(1), "the newest, not the header");
    }

    #[test]
    fn without_history_the_empty_box_has_no_rows() {
        let (state, index) = with_recent(&[]);
        assert!(row_names(&state, &index).is_empty());
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn typing_replaces_recent_and_erasing_brings_it_back() {
        let (mut state, index) = with_recent(&["notepad"]);
        type_text(&mut state, &index, "out");
        assert!(!row_names(&state, &index).contains(&"[Recent]".to_owned()));
        let hit = state.selected_hit().unwrap();
        assert_eq!(
            state.recent_row(hit),
            None,
            "a search result is not a recent row"
        );

        for _ in 0..3 {
            press(&mut state, &index, Key::Backspace);
        }
        assert_eq!(row_names(&state, &index), ["[Recent]", "notepad"]);
    }

    #[test]
    fn a_recent_row_knows_when_it_was_launched() {
        let (state, _) = with_recent(&["notepad", "outlook"]);
        let hits: Vec<_> = state
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Result(hit) => state.recent_row(*hit).map(|row| row.last),
                _ => None,
            })
            .collect();
        assert_eq!(hits, [1000, 999]);
    }

    #[test]
    fn enter_on_the_first_recent_row_opens_it() {
        let (mut state, index) = with_recent(&["notepad"]);
        let hit = state.selected_hit().unwrap();
        assert_eq!(
            state.handle(&index, shortcut("Enter")),
            Outcome::Activate(hit, ActionId::Open)
        );
    }

    #[test]
    fn forget_applies_to_a_recent_row_and_to_nothing_else() {
        let (mut state, index) = with_recent(&["notepad"]);
        let hit = state.selected_hit().unwrap();
        assert_eq!(
            state.handle(&index, shortcut("Ctrl+Del")),
            Outcome::Activate(hit, ActionId::Forget)
        );
        // It is in the menu of that row…
        state.handle(&index, shortcut("Alt+Enter"));
        assert!(menu_ids(&state).contains(&"forget"));
        state.handle(&index, shortcut("Alt+Enter"));
        // …and of no search result.
        type_text(&mut state, &index, "notepad");
        assert_eq!(state.handle(&index, shortcut("Ctrl+Del")), Outcome::Nothing);
        state.handle(&index, shortcut("Alt+Enter"));
        assert!(!menu_ids(&state).contains(&"forget"));
    }

    /// `with_recent`, the empty box showing the first `shown`.
    fn with_recent_shown(names: &[&str], shown: usize) -> (State, Index) {
        let index = index();
        let mut state = state().with_recent_shown(shown);
        let recent = names
            .iter()
            .enumerate()
            .map(|(n, name)| Recent {
                item: n as u32,
                entry: position(&index, name),
                last: 1000 - n as u64,
            })
            .collect();
        state.start(&index, recent, Vec::new(), Vec::new());
        (state, index)
    }

    #[test]
    fn a_history_longer_than_count_ends_in_a_row_that_opens_it_all() {
        let (state, index) = with_recent_shown(&["notepad", "outlook", "notes"], 2);
        assert_eq!(
            row_names(&state, &index),
            ["[Recent]", "notepad", "outlook", "<Recent>"]
        );
        let (state, index) = with_recent_shown(&["notepad", "outlook"], 2);
        assert_eq!(
            row_names(&state, &index),
            ["[Recent]", "notepad", "outlook"],
            "nothing more to see"
        );
    }

    #[test]
    fn the_whole_history_opens_as_a_chip_and_comes_back() {
        let (mut state, index) = with_recent_shown(&["notepad", "outlook", "notes"], 1);
        press(&mut state, &index, Key::Down);
        assert_eq!(state.selected(), Some(2), "the row after the first");
        press(&mut state, &index, Key::Enter);
        assert_eq!(state.scopes(), [Place::Recent]);
        assert_eq!(
            row_names(&state, &index),
            ["notepad", "outlook", "notes"],
            "everything, newest first"
        );
        assert_eq!(state.selected(), Some(0));

        press(&mut state, &index, Key::Backspace);
        assert_eq!(state.scopes(), []);
        assert_eq!(state.selected(), Some(2), "back on the row it opened from");
    }

    #[test]
    fn typing_inside_the_history_searches_only_it_and_forget_still_applies() {
        let (mut state, index) = with_recent_shown(&["notepad", "outlook"], 1);
        press(&mut state, &index, Key::Down);
        press(&mut state, &index, Key::Enter);

        type_text(&mut state, &index, "no");
        assert_eq!(
            row_names(&state, &index),
            ["notepad"],
            "not the `notes` that is no launch, and no headings"
        );
        let hit = state.selected_hit().unwrap();
        assert_eq!(state.recent_row(hit).map(|row| row.item), Some(0));
        assert_eq!(
            state.handle(&index, shortcut("Ctrl+Del")),
            Outcome::Activate(hit, ActionId::Forget)
        );
    }

    #[test]
    fn forgetting_shortens_the_count_and_the_more_row_goes_when_all_fits() {
        let (mut state, index) = with_recent_shown(&["notepad", "outlook", "notes"], 2);
        let count = |state: &State| {
            state.rows().iter().find_map(|row| match row {
                Row::Section(section) => Some(section.apps),
                _ => None,
            })
        };
        assert_eq!(count(&state), Some(3));
        state.remove_recent(&index, position(&index, "notes"));
        assert_eq!(count(&state), None);
    }

    #[test]
    fn forgetting_a_launch_keeps_the_selection_in_place() {
        let (mut state, index) = with_recent(&["notepad", "outlook", "notes"]);
        press(&mut state, &index, Key::Down);
        assert_eq!(state.selected(), Some(2), "outlook");

        state.remove_recent(&index, position(&index, "outlook"));
        assert_eq!(row_names(&state, &index), ["[Recent]", "notepad", "notes"]);
        assert_eq!(
            state.selected(),
            Some(2),
            "now notes, the one that took its place"
        );

        state.remove_recent(&index, position(&index, "notes"));
        assert_eq!(state.selected(), Some(1), "the last one: the one before");

        state.remove_recent(&index, position(&index, "notepad"));
        assert!(only_prefixes(&state));
        assert_eq!(state.selected(), None);
    }

    fn collapsed(names: &[&str]) -> (State, Index) {
        let (state, index) = with_recent(names);
        let mut state = state.with_collapsed(true);
        let recent = names
            .iter()
            .enumerate()
            .map(|(n, name)| Recent {
                item: n as u32,
                entry: position(&index, name),
                last: 1000 - n as u64,
            })
            .collect();
        state.start(&index, recent, Vec::new(), Vec::new());
        (state, index)
    }

    #[test]
    fn a_collapsed_box_opens_folded_and_does_nothing_on_enter() {
        let (mut state, index) = collapsed(&["notepad"]);
        assert!(state.folded() && state.collapsible());
        assert!(state.rows().is_empty());
        assert_eq!(state.selected(), None);
        assert_eq!(state.handle(&index, shortcut("Enter")), Outcome::Nothing);
        assert_eq!(
            state.handle(&index, shortcut("Alt+Enter")),
            Outcome::Nothing
        );
    }

    #[test]
    fn down_unfolds_to_recent_with_the_first_row_selected() {
        let (mut state, index) = collapsed(&["notepad", "outlook"]);
        assert_eq!(press(&mut state, &index, Key::Down), Outcome::Redraw);
        assert!(!state.folded());
        assert_eq!(
            row_names(&state, &index),
            ["[Recent]", "notepad", "outlook"]
        );
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn up_on_the_first_row_folds_but_elsewhere_it_just_moves() {
        let (mut state, index) = collapsed(&["notepad", "outlook"]);
        press(&mut state, &index, Key::Down);
        press(&mut state, &index, Key::Down);
        assert_eq!(state.selected(), Some(2));
        press(&mut state, &index, Key::Up);
        assert!(!state.folded());
        assert_eq!(state.selected(), Some(1));
        assert_eq!(press(&mut state, &index, Key::Up), Outcome::Redraw);
        assert!(state.folded());
        assert!(state.rows().is_empty());
    }

    #[test]
    fn an_uncollapsed_box_never_folds() {
        let (mut state, index) = with_recent(&["notepad"]);
        assert!(!state.folded() && !state.collapsible());
        assert_eq!(press(&mut state, &index, Key::Up), Outcome::Nothing);
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Close);
        assert_eq!(state.unfold(&index), Outcome::Nothing);
    }

    #[test]
    fn typing_unfolds_to_the_results_and_erasing_folds_again() {
        let (mut state, index) = collapsed(&["notepad"]);
        type_text(&mut state, &index, "out");
        assert!(!state.folded());
        assert!(!state.rows().is_empty(), "the results of out");

        for _ in 0..3 {
            press(&mut state, &index, Key::Backspace);
        }
        assert!(state.folded(), "no text, folded");
        assert!(state.rows().is_empty());
    }

    #[test]
    fn escape_clears_then_folds_then_closes() {
        let (mut state, index) = collapsed(&["notepad"]);
        press(&mut state, &index, Key::Down);
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.folded(), "first the list folds");
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Close);

        // With text: the text goes (and the list with it), then the window.
        type_text(&mut state, &index, "out");
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.folded() && state.input().is_empty());
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Close);
    }

    #[test]
    fn leaving_a_group_does_not_fold_the_list() {
        let (mut state, index) = collapsed(&[]);
        type_text(&mut state, &index, "servers");
        press(&mut state, &index, Key::Tab);
        assert_eq!(state.scopes().len(), 1);
        assert!(!state.folded());
        press(&mut state, &index, Key::Backspace);
        assert!(state.scopes().is_empty());
        assert!(!state.folded(), "you asked to see it");
        // Escape from there folds it, then closes.
        press(&mut state, &index, Key::Escape);
        assert!(state.folded());
    }

    #[test]
    fn a_click_on_the_chevron_unfolds() {
        let (mut state, index) = collapsed(&["notepad"]);
        assert_eq!(state.unfold(&index), Outcome::Redraw);
        assert_eq!(row_names(&state, &index), ["[Recent]", "notepad"]);
        assert_eq!(state.unfold(&index), Outcome::Nothing, "already open");
    }

    const SECTIONS: &str = r#"<y-pop>
        <search>
            <recent count="3"/>
            <group name="Admin"><app name="Regedit" exe="r.exe"/><app name="Services" exe="s.exe"/></group>
        </search>
        <group name="Work">
            <app name="Outlook" exe="o.exe"/>
            <group name="Servers"><app name="Putty" exe="p.exe"/></group>
        </group>
        <app name="Alone" exe="a.exe"/>
    </y-pop>"#;

    /// A state on the sections of `SECTIONS`, with `recent` launched before.
    fn with_sections(recent: &[&str]) -> (State, Index) {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(SECTIONS, &mut builtins).unwrap();
        let index = Index::from_tree(&tree);
        let recent = recent
            .iter()
            .enumerate()
            .map(|(n, name)| Recent {
                item: n as u32,
                entry: position(&index, name),
                last: 1000 - n as u64,
            })
            .collect();
        let mut state = state();
        state.start(
            &index,
            recent,
            super::super::sections::rows(&tree, &index),
            Vec::new(),
        );
        (state, index)
    }

    #[test]
    fn the_empty_box_lists_recent_then_the_sections() {
        let (state, index) = with_sections(&["outlook"]);
        assert_eq!(
            row_names(&state, &index),
            [
                "[Recent]",
                "outlook",
                "[Browse]",
                "<Menus>",
                &format!(
                    "<{:?}>",
                    Place::Group(
                        index.entries()[position(&index, "admin") as usize]
                            .node()
                            .unwrap()
                    )
                ),
            ]
        );
        assert_eq!(state.selected(), Some(1), "the last launch, not a section");
    }

    #[test]
    fn with_no_history_the_first_section_is_selected() {
        let (state, _) = with_sections(&[]);
        assert_eq!(state.selected(), Some(1));
        assert!(matches!(state.rows()[1], Row::Section(_)));
    }

    #[test]
    fn the_arrows_walk_the_sections_too() {
        let (mut state, index) = with_sections(&["outlook"]);
        press(&mut state, &index, Key::Down);
        assert_eq!(state.selected(), Some(3), "the header is skipped");
        press(&mut state, &index, Key::Down);
        assert_eq!(state.selected(), Some(4));
        press(&mut state, &index, Key::Down);
        assert_eq!(state.selected(), Some(4), "the end");
        assert!(state.selected_hit().is_none(), "a section is not a result");
    }

    #[test]
    fn enter_right_and_tab_open_a_section_as_a_chip() {
        for key in [Key::Enter, Key::Right, Key::Tab] {
            let (mut state, index) = with_sections(&[]);
            assert_eq!(press(&mut state, &index, key), Outcome::Redraw, "{key:?}");
            assert_eq!(state.scopes(), [Place::Menus], "{key:?}");
            assert!(state.input().is_empty());
        }
    }

    #[test]
    fn the_tray_menus_section_lists_the_tray_icons_and_searches_only_the_menus() {
        let (mut state, index) = with_sections(&[]);
        press(&mut state, &index, Key::Enter);
        assert_eq!(row_names(&state, &index), ["work", "alone"]);

        // Searching inside finds the menus' items, not the search-only ones.
        type_text(&mut state, &index, "e");
        let found = row_names(&state, &index);
        assert!(found.contains(&"outlook".to_owned()), "{found:?}");
        assert!(!found.contains(&"regedit".to_owned()), "{found:?}");
    }

    #[test]
    fn a_search_only_group_section_lists_what_is_in_it() {
        let (mut state, index) = with_sections(&[]);
        press(&mut state, &index, Key::Down);
        press(&mut state, &index, Key::Enter);
        assert_eq!(row_names(&state, &index), ["regedit", "services"]);
    }

    #[test]
    fn left_and_backspace_come_back_to_the_section_they_left() {
        for key in [Key::Left, Key::Backspace] {
            let (mut state, index) = with_sections(&[]);
            press(&mut state, &index, Key::Down);
            press(&mut state, &index, Key::Enter);
            assert_eq!(state.scopes().len(), 1);

            press(&mut state, &index, key);
            assert!(state.scopes().is_empty(), "{key:?}");
            assert_eq!(state.selected(), Some(2), "{key:?}: the Admin row, again");
        }
    }

    #[test]
    fn a_section_has_no_actions_and_no_buttons() {
        let (mut state, index) = with_sections(&[]);
        assert!(!state.has_row_buttons(&index));
        assert!(state.available_actions(&index).is_empty());
        assert_eq!(
            state.handle(&index, shortcut("Alt+Enter")),
            Outcome::Nothing
        );
        assert_eq!(state.handle(&index, shortcut("Ctrl+E")), Outcome::Nothing);
    }

    #[test]
    fn a_click_on_a_section_goes_inside_it() {
        let (mut state, index) = with_sections(&[]);
        assert!(state.select_row(2), "hover lights a section");
        assert_eq!(state.click_row(&index, 1), Outcome::Redraw);
        assert_eq!(state.scopes(), [Place::Menus]);
    }

    #[test]
    fn typing_replaces_the_sections_and_erasing_brings_them_back() {
        let (mut state, index) = with_sections(&[]);
        type_text(&mut state, &index, "out");
        assert!(
            state
                .rows()
                .iter()
                .all(|row| !matches!(row, Row::Section(_)))
        );
        for _ in 0..3 {
            press(&mut state, &index, Key::Backspace);
        }
        assert!(matches!(state.rows()[1], Row::Section(_)));
    }

    const WALKED: &str = r#"<y-pop>
        <search hotkey="Alt+Space"/>
        <group name="Work">
            <app name="Mail" exe="m.exe"/>
            <group name="Servers">
                <app name="Putty" exe="p.exe"/>
                <group name="Deep"><app name="Bottom" exe="b.exe"/></group>
            </group>
            <group name="Hollow"/>
            <app name="Scripts" exe="C:\Scripts" browse="true"/>
            <app name="Zed" exe="z.exe"/>
        </group>
        <app name="Alone" exe="a.exe"/>
        <group name="Play"><app name="Chess" exe="c.exe"/><app name="1942" exe="n.exe"/></group>
    </y-pop>"#;

    /// A state inside the tray menus section, on `WALKED`.
    fn walking(browse: MenusBrowse) -> (State, Index) {
        let tree = config::load_str(WALKED, &mut Vec::<Variable>::new()).unwrap();
        let index = Index::from_tree(&tree);
        let mut state = state().with_menus_browse(browse);
        let sections = super::super::sections::rows(&tree, &index);
        state.start(&index, Vec::new(), sections, Vec::new());
        press(&mut state, &index, Key::Enter);
        assert_eq!(state.scopes(), [Place::Menus]);
        (state, index)
    }

    fn selected_name(state: &State, index: &Index) -> Option<String> {
        let hit = state.selected_hit()?;
        Some(name_of(index, hit).to_owned())
    }

    fn panes_of(state: &State, index: &Index) -> Vec<Vec<String>> {
        state
            .column_panes(index)
            .unwrap()
            .iter()
            .map(|pane| {
                pane.items
                    .iter()
                    .map(|&item| index.entries()[item as usize].name().to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_columns_open_on_the_tray_menus_with_the_first_one_previewed() {
        let (state, index) = walking(MenusBrowse::Columns);
        assert_eq!(state.column_depth(), Some(0));
        assert_eq!(selected_name(&state, &index).unwrap(), "work");
        assert_eq!(
            panes_of(&state, &index),
            [
                vec!["work", "alone", "play"],
                vec!["mail", "servers", "hollow", "scripts", "zed"],
            ]
        );
    }

    #[test]
    fn moving_the_selection_changes_the_preview() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Down);
        let panes = state.column_panes(&index).unwrap();
        assert_eq!(selected_name(&state, &index).unwrap(), "alone");
        assert_eq!(
            panes.last().unwrap().details,
            state.selected_hit().map(|hit| hit.entry),
            "an app previews its details"
        );
    }

    #[test]
    fn right_tab_and_enter_go_in_and_left_comes_back_to_the_same_row() {
        for key in [Key::Right, Key::Tab, Key::Enter] {
            let (mut state, index) = walking(MenusBrowse::Columns);
            press(&mut state, &index, key);
            assert_eq!(state.column_depth(), Some(1), "{key:?}");
            assert_eq!(selected_name(&state, &index).unwrap(), "mail");

            press(&mut state, &index, Key::Down);
            press(&mut state, &index, Key::Right);
            assert_eq!(state.column_depth(), Some(2));
            assert_eq!(selected_name(&state, &index).unwrap(), "putty");

            press(&mut state, &index, Key::Left);
            assert_eq!(selected_name(&state, &index).unwrap(), "servers");
            press(&mut state, &index, Key::Backspace);
            assert_eq!(selected_name(&state, &index).unwrap(), "work");
            assert_eq!(state.scopes(), [Place::Menus], "{key:?}: still inside");
        }
    }

    #[test]
    fn left_from_the_first_column_leaves_the_section() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Left);
        assert!(state.scopes().is_empty());
        assert_eq!(state.column_depth(), None);
        assert!(matches!(
            state.rows()[state.selected().unwrap()],
            Row::Section(_)
        ));
    }

    #[test]
    fn esc_leaves_the_section_from_any_column_and_coming_back_starts_afresh() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::Escape);
        assert!(state.scopes().is_empty());

        press(&mut state, &index, Key::Enter);
        assert_eq!(state.column_depth(), Some(0));
        assert_eq!(selected_name(&state, &index).unwrap(), "work");
    }

    #[test]
    fn nothing_opens_from_an_app_or_an_empty_group_and_right_stays_put() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Down);
        assert_eq!(press(&mut state, &index, Key::Right), Outcome::Nothing);
        assert_eq!(state.column_depth(), Some(0));

        press(&mut state, &index, Key::Up);
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::Down);
        press(&mut state, &index, Key::Down);
        assert_eq!(selected_name(&state, &index).unwrap(), "hollow");
        assert_eq!(press(&mut state, &index, Key::Enter), Outcome::Nothing);
        assert_eq!(state.column_depth(), Some(1));
    }

    #[test]
    fn enter_on_an_app_opens_it_and_its_shortcuts_work() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        assert!(matches!(
            press(&mut state, &index, Key::Enter),
            Outcome::Activate(hit, ActionId::Open) if name_of(&index, hit) == "mail"
        ));
        assert!(matches!(
            state.handle(&index, shortcut("Ctrl+R")),
            Outcome::Activate(hit, ActionId::Reveal) if name_of(&index, hit) == "mail"
        ));
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_column() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::End);
        assert_eq!(selected_name(&state, &index).unwrap(), "play");
        press(&mut state, &index, Key::Home);
        assert_eq!(selected_name(&state, &index).unwrap(), "work");
    }

    #[test]
    fn typing_searches_and_erasing_finds_the_columns_as_they_were() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::Down);
        type_text(&mut state, &index, "put");
        assert_eq!(state.column_depth(), None, "the results replace them");
        assert_eq!(selected_name(&state, &index).unwrap(), "putty");

        press(&mut state, &index, Key::Down);
        for _ in 0..3 {
            press(&mut state, &index, Key::Backspace);
        }
        assert_eq!(state.column_depth(), Some(1));
        assert_eq!(selected_name(&state, &index).unwrap(), "servers");
    }

    #[test]
    fn a_group_entered_from_a_search_keeps_the_columns_underneath() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Down);
        type_text(&mut state, &index, "serv");
        press(&mut state, &index, Key::Tab);
        assert_eq!(state.scopes().len(), 2);
        assert_eq!(state.column_depth(), None);

        press(&mut state, &index, Key::Backspace);
        assert_eq!(state.scopes(), [Place::Menus]);
        assert_eq!(state.column_depth(), Some(0));
        assert_eq!(
            selected_name(&state, &index).unwrap(),
            "alone",
            "the group's rows did not touch the column's selection"
        );
    }

    #[test]
    fn a_browsed_folder_is_not_read_until_its_column_opens_and_then_waits() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::End);
        press(&mut state, &index, Key::Up);
        assert_eq!(selected_name(&state, &index).unwrap(), "scripts");
        let folder = state.selected_hit().unwrap().entry;
        assert_eq!(
            state.column_panes(&index).unwrap().last().unwrap().details,
            Some(folder),
            "selected, it shows its details"
        );

        assert_eq!(
            press(&mut state, &index, Key::Enter),
            Outcome::ListFolder(folder)
        );
        assert_eq!(state.column_depth(), Some(2));
        assert!(state.column_panes(&index).unwrap()[2].waiting);
        assert!(state.selected_hit().is_none());

        // Two of the index's entries stand in for the files.
        let files: Vec<u32> = vec![0, 1];
        state.column_listed(&index, folder, files.clone());
        assert_eq!(state.selected_hit().unwrap().entry, files[0]);
        assert!(!state.column_panes(&index).unwrap()[2].waiting);
    }

    #[test]
    fn a_listing_nobody_waits_for_any_more_changes_nothing() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::End);
        press(&mut state, &index, Key::Up);
        let folder = state.selected_hit().unwrap().entry;
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::Left);

        state.column_listed(&index, folder, vec![0]);
        assert_eq!(state.column_depth(), Some(1));
        assert_eq!(selected_name(&state, &index).unwrap(), "scripts");
    }

    #[test]
    fn a_click_goes_into_a_group_and_only_selects_an_app() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        // A row of the preview: the group it shows opens first.
        assert_eq!(state.column_click(&index, 1, 1), Outcome::Redraw);
        assert_eq!(state.column_depth(), Some(2), "work, then servers");
        assert_eq!(selected_name(&state, &index).unwrap(), "putty");

        // A row of a column to the left closes what is right of it.
        assert_eq!(state.column_click(&index, 1, 0), Outcome::Redraw);
        assert_eq!(state.column_depth(), Some(1));
        assert_eq!(selected_name(&state, &index).unwrap(), "mail");

        // The focused column's row, as the window reports it.
        assert_eq!(state.click_row(&index, 4), Outcome::Redraw);
        assert_eq!(selected_name(&state, &index).unwrap(), "zed");
        assert_eq!(state.column_depth(), Some(1), "an app stays where it is");
    }

    #[test]
    fn hovering_a_column_does_not_move_the_selection() {
        let (mut state, _) = walking(MenusBrowse::Columns);
        assert!(!state.select_row(1));
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn the_details_buttons_are_open_reveal_and_edit_when_they_apply() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        assert_eq!(
            state.details_buttons(&index),
            [ActionId::Open, ActionId::Reveal, ActionId::Edit]
        );
        assert!(matches!(
            state.details_click(&index, 1),
            Outcome::Activate(_, ActionId::Reveal)
        ));
        assert_eq!(state.details_click(&index, 9), Outcome::Nothing);
    }

    #[test]
    fn the_crumbs_name_the_way_in() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        press(&mut state, &index, Key::Down);
        press(&mut state, &index, Key::Right);
        let names: Vec<&str> = state
            .column_crumbs()
            .iter()
            .map(|&entry| index.entries()[entry as usize].name())
            .collect();
        assert_eq!(names, ["work", "servers"]);
    }

    #[test]
    fn the_action_menu_works_on_the_focused_column() {
        let (mut state, index) = walking(MenusBrowse::Columns);
        press(&mut state, &index, Key::Right);
        assert_eq!(state.open_menu(&index), Outcome::Redraw);
        assert_eq!(state.menu().unwrap().row, 0);
        let up = press(&mut state, &index, Key::Enter);
        assert!(matches!(up, Outcome::Activate(_, ActionId::Open)));
    }

    #[test]
    fn a_z_lists_every_app_of_every_menu_under_its_letter() {
        let (state, index) = walking(MenusBrowse::AZ);
        assert_eq!(state.column_depth(), None);
        assert_eq!(
            row_names(&state, &index),
            [
                "[#]", "1942", "[A]", "alone", "[B]", "bottom", "[C]", "chess", "[M]", "mail",
                "[P]", "putty", "[Z]", "zed",
            ],
            "apps only: no group, no browsed folder"
        );
        assert_eq!(state.letters(), ['#', 'A', 'B', 'C', 'M', 'P', 'Z']);
        assert_eq!(state.selected(), Some(1), "the headers are skipped");
    }

    #[test]
    fn a_click_on_a_letter_selects_its_first_row_and_gives_its_header() {
        let (mut state, index) = walking(MenusBrowse::AZ);
        assert_eq!(state.letter_click(4), Some(8));
        assert_eq!(selected_name(&state, &index).unwrap(), "mail");
        assert_eq!(state.letter_click(99), None);
    }

    #[test]
    fn typing_in_a_z_searches_and_the_letters_go_away_until_it_is_erased() {
        let (mut state, index) = walking(MenusBrowse::AZ);
        type_text(&mut state, &index, "m");
        assert!(state.letters().is_empty());
        press(&mut state, &index, Key::Backspace);
        assert!(!state.letters().is_empty());
    }

    #[test]
    fn include_limits_the_section_and_the_plain_search_but_not_the_hotkeys() {
        let tree = config::load_str(
            r#"<y-pop>
                <search><menus browse="a-z" include="Work"/></search>
                <group name="Work"><app name="Mail" exe="m.exe"/></group>
                <group name="Play"><app name="Mailer" exe="g.exe"><hotkey key="Win+G"/></app></group>
            </y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .unwrap();
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let index = Index::from_tree(&tree);
        let mut state = state().with_menus_browse(tree.search.as_ref().unwrap().menus_browse());
        let sections = super::super::sections::rows(&tree, &index);
        assert_eq!(sections[0].apps, 1);
        state.start(&index, Vec::new(), sections, Vec::new());

        type_text(&mut state, &index, "mail");
        assert_eq!(row_names(&state, &index), ["[TopHit]", "mail"]);

        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        type_text(&mut state, &index, "#");
        assert!(row_names(&state, &index).contains(&"mailer".to_owned()));

        state.handle(&index, Event::Key(Key::Escape, Mods::default()));
        press(&mut state, &index, Key::Enter);
        assert_eq!(row_names(&state, &index), ["[M]", "mail"]);
    }

    #[test]
    fn the_list_shows_the_tray_icons_and_tab_goes_into_a_group_as_a_chip() {
        let (mut state, index) = walking(MenusBrowse::List);
        assert_eq!(state.column_depth(), None);
        assert!(state.letters().is_empty());
        assert_eq!(row_names(&state, &index), ["work", "alone", "play"]);

        press(&mut state, &index, Key::Tab);
        assert_eq!(state.scopes().len(), 2, "a chip for the group");
        assert_eq!(
            row_names(&state, &index),
            ["mail", "servers", "hollow", "scripts", "zed"]
        );
        press(&mut state, &index, Key::Backspace);
        assert_eq!(state.scopes(), [Place::Menus]);
        assert_eq!(row_names(&state, &index), ["work", "alone", "play"]);
    }

    #[test]
    fn the_list_searches_like_the_other_views() {
        let (mut state, index) = walking(MenusBrowse::List);
        type_text(&mut state, &index, "putty");
        assert_eq!(selected_name(&state, &index).unwrap(), "putty");
    }

    const KEYED: &str = r#"<y-pop>
        <search hotkey="Alt+Space"/>
        <group name="Work">
            <app name="Task Manager" exe="t.exe"><hotkey key="Ctrl+Shift+Esc"/></app>
            <app name="Notes" exe="n.exe"><hotkey key="Win+N"/></app>
            <app name="Plain" exe="p.exe"/>
        </group>
    </y-pop>"#;

    /// A state on `KEYED`, which knows the search hotkey.
    fn keyed() -> (State, Index) {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(KEYED, &mut builtins).unwrap();
        let index = Index::from_tree(&tree);
        let mut state = state();
        state.set_search_hotkey(KeyCombo::parse("alt+space"));
        state.start(&index, Vec::new(), Vec::new(), Vec::new());
        (state, index)
    }

    fn lines(state: &State) -> Vec<Line> {
        state
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Line(line) => Some(*line),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_empty_box_ends_with_the_line_of_prefixes_which_cannot_be_selected() {
        let (mut state, index) = with_sections(&["outlook"]);
        assert_eq!(state.rows().last(), Some(&Row::Prefixes));
        assert_eq!(
            state.row_kinds().last(),
            Some(&super::super::layout::RowKind::Prefixes)
        );
        for _ in 0..6 {
            press(&mut state, &index, Key::Down);
        }
        assert_ne!(state.selected(), Some(state.rows().len() - 1));
        assert!(!state.select_row(state.rows().len() - 1));
    }

    #[test]
    fn a_click_on_a_prefix_types_it() {
        let (mut state, index) = with_sections(&[]);
        assert_eq!(state.prefix_click(&index, 1), Outcome::Redraw);
        assert_eq!(state.input().text(), "#");
        assert_eq!(state.prefix_click(&index, 9), Outcome::Nothing);
    }

    #[test]
    fn greater_than_lists_the_commands_and_enter_runs_one() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, ">");
        assert_eq!(
            lines(&state),
            Command::ALL.map(Line::Command),
            "all of them, in order"
        );
        assert_eq!(state.rows()[0], Row::Header(Heading::Commands));
        assert_eq!(
            press(&mut state, &index, Key::Enter),
            Outcome::Command(Command::Reload)
        );
    }

    #[test]
    fn a_command_is_found_by_its_name_or_by_other_words() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, ">quit");
        assert_eq!(lines(&state)[0], Line::Command(Command::Quit));
        press(&mut state, &index, Key::Escape);

        type_text(&mut state, &index, ">restart");
        assert_eq!(lines(&state), [Line::Command(Command::Reload)]);
        press(&mut state, &index, Key::Escape);

        type_text(&mut state, &index, ">zzzz");
        assert!(state.rows().is_empty());
        assert_eq!(press(&mut state, &index, Key::Enter), Outcome::Nothing);
    }

    #[test]
    fn hash_lists_every_binding_with_the_search_hotkey_first() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, "#");
        assert_eq!(row_names(&state, &index)[0], "[Hotkeys]");
        assert_eq!(lines(&state), [Line::SearchHotkey]);
        let names = row_names(&state, &index);
        assert!(names.contains(&"task manager".to_owned()), "{names:?}");
        assert!(names.contains(&"notes".to_owned()));
        assert!(
            !names.contains(&"plain".to_owned()),
            "no hotkey, not listed"
        );

        // Enter on a binding launches what it opens.
        press(&mut state, &index, Key::Down);
        let hit = state.selected_hit().unwrap();
        assert_eq!(
            press(&mut state, &index, Key::Enter),
            Outcome::Activate(hit, ActionId::Open)
        );
    }

    #[test]
    fn hash_and_part_of_a_combination_filters_the_bindings() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, "#shift+ctrl");
        let names = row_names(&state, &index);
        assert!(names.contains(&"task manager".to_owned()), "{names:?}");
        assert!(!names.contains(&"notes".to_owned()));
        assert_eq!(state.check(), None, "not a whole combination");
    }

    #[test]
    fn a_whole_combination_nothing_uses_asks_whether_it_is_free() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, "#ctrl+alt+x");
        assert_eq!(lines(&state), [Line::Check]);
        assert_eq!(state.check(), Some("ctrl+alt+x"));
        assert_eq!(press(&mut state, &index, Key::Enter), Outcome::CheckKey);
    }

    #[test]
    fn a_combination_the_file_uses_is_found_not_checked() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, "#win+n");
        assert_eq!(state.check(), None);
        assert!(lines(&state).is_empty());
        assert!(row_names(&state, &index).contains(&"notes".to_owned()));

        press(&mut state, &index, Key::Escape);
        type_text(&mut state, &index, "#alt+space");
        assert_eq!(
            lines(&state),
            [Line::SearchHotkey],
            "it is the search's own"
        );
        assert_eq!(state.check(), None);
    }

    #[test]
    fn question_mark_lists_the_prefixes_and_enter_types_one() {
        let (mut state, index) = keyed();
        type_text(&mut state, &index, "?");
        assert_eq!(
            lines(&state),
            PREFIXES.map(|info| Line::Prefix(info.prefix))
        );
        press(&mut state, &index, Key::Down);
        assert_eq!(press(&mut state, &index, Key::Enter), Outcome::Redraw);
        assert_eq!(state.input().text(), "#", "the second one");
        assert_eq!(lines(&state), [Line::SearchHotkey]);
    }

    /// `index` with `names` appended as files of `/`.
    fn with_files(index: &mut Index, names: &[&str]) {
        let first = index
            .entries()
            .iter()
            .filter(|entry| entry.source == Source::BrowsedFile)
            .count() as u32;
        for (n, name) in names.iter().enumerate() {
            index.push(super::super::index::Entry::from_file(
                first + n as u32,
                Kind::File,
                name,
            ));
        }
    }

    fn listing() -> Progress {
        Progress {
            started: true,
            listing: true,
            folders: 1,
            ..Progress::default()
        }
    }

    #[test]
    fn slash_asks_for_the_files_lists_them_and_says_how_it_is_going() {
        let (mut state, mut index) = (state(), index());
        assert!(!state.wants_files());
        type_text(&mut state, &index, "/");
        assert!(state.wants_files());
        assert!(state.rows().is_empty(), "nothing yet");

        state.files_changed(&index, listing());
        assert_eq!(lines(&state), [Line::Scan]);
        assert_eq!(state.selected(), None, "a note is not a selection");
        assert!(!state.select_row(0));
        assert_eq!(state.click_row(&index, 0), Outcome::Nothing);
        assert_eq!(press(&mut state, &index, Key::Enter), Outcome::Nothing);

        with_files(&mut index, &["deploy.ps1", "notes.txt"]);
        state.files_changed(&index, listing());
        assert_eq!(state.rows()[0], Row::Header(Heading::Files));
        assert_eq!(names(&state, &index), ["deploy.ps1", "notes.txt"]);
        assert_eq!(state.rows().last(), Some(&Row::Line(Line::Scan)));
        assert_eq!(state.selected(), Some(1));

        let done = Progress {
            listing: false,
            ..listing()
        };
        state.files_changed(&index, done);
        assert!(lines(&state).is_empty(), "nothing left to say");
    }

    #[test]
    fn slash_searches_files_and_only_files() {
        let (mut state, mut index) = (state(), index());
        with_files(&mut index, &["notes.txt", "deploy.ps1"]);
        type_text(&mut state, &index, "/note");
        assert_eq!(names(&state, &index), ["notes.txt"], "not the app Notes");

        press(&mut state, &index, Key::Escape);
        type_text(&mut state, &index, "note");
        assert!(
            !names(&state, &index).contains(&"notes.txt".to_owned()),
            "and a plain search does not see the files"
        );
    }

    #[test]
    fn files_join_without_moving_the_selection() {
        let (mut state, mut index) = (state(), index());
        with_files(&mut index, &["report a.txt", "report b.txt"]);
        type_text(&mut state, &index, "/report");
        press(&mut state, &index, Key::Down);
        let kept = state.selected_hit().unwrap();
        assert_eq!(name_of(&index, kept), "report b.txt");
        state.handle(&index, shortcut("Alt+Enter"));
        assert!(state.menu().is_some());

        // A better match arrives, above it.
        with_files(&mut index, &["report"]);
        state.files_changed(&index, listing());
        assert_eq!(names(&state, &index)[0], "report");
        let hit = state.selected_hit().unwrap();
        assert_eq!(hit.entry, kept.entry, "still on the same file");
        let menu = state.menu().expect("the menu stays open");
        assert_eq!(Some(menu.row), state.selected(), "and on its row");
    }

    #[test]
    fn files_arriving_leave_any_other_list_alone() {
        let (mut state, mut index) = (state(), index());
        type_text(&mut state, &index, "notepad");
        let rows = state.rows().to_vec();
        with_files(&mut index, &["notepad.txt"]);
        state.files_changed(&index, listing());
        assert_eq!(state.rows(), rows);
    }

    #[test]
    fn a_file_has_the_actions_of_a_file() {
        let (mut state, mut index) = (state(), index());
        with_files(&mut index, &["deploy.ps1"]);
        type_text(&mut state, &index, "/dep");
        let ids: Vec<ActionId> = state
            .available_actions(&index)
            .iter()
            .map(|action| action.id)
            .collect();
        assert_eq!(ids, [ActionId::Open], "no file behind this bare entry");

        let entry = &index.entries()[state.selected_hit().unwrap().entry as usize];
        assert!(!actions::applies(ActionId::Reveal, entry), "in no menu");
        assert!(!actions::applies(ActionId::Edit, entry), "in no file");
        let hit = state.selected_hit().unwrap();
        assert_eq!(
            press(&mut state, &index, Key::Enter),
            Outcome::Activate(hit, ActionId::Open)
        );
    }

    #[test]
    fn a_prefix_is_only_a_prefix_on_the_outermost_box() {
        let (mut state, index) = with_sections(&[]);
        press(&mut state, &index, Key::Enter);
        assert_eq!(state.scopes().len(), 1);
        type_text(&mut state, &index, ">");
        assert!(lines(&state).is_empty(), "a > inside a section is text");
    }

    #[test]
    fn erasing_the_prefix_brings_the_empty_box_back() {
        let (mut state, index) = with_sections(&["outlook"]);
        type_text(&mut state, &index, ">");
        assert!(!lines(&state).is_empty());
        press(&mut state, &index, Key::Backspace);
        assert_eq!(state.rows()[0], Row::Header(Heading::Recent));
        // Escape clears it too.
        type_text(&mut state, &index, "?");
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.input().is_empty());
    }
    #[test]
    fn row_actions_false_takes_every_button_away() {
        let (mut state, index) = (state().with_row_actions(false), index());
        type_text(&mut state, &index, "plain program");
        assert!(state.row_buttons(&index).is_empty());
        assert!(!state.has_row_buttons(&index), "the ⋯ goes too");
        // The menu is still there for the keyboard and a right click.
        assert_eq!(state.handle(&index, shortcut("Alt+Enter")), Outcome::Redraw);
    }

    #[test]
    fn clicking_a_button_does_its_action_to_the_selected_result() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "plain program");
        let hit = state.selected_hit().unwrap();
        assert_eq!(
            state.button_click(&index, 1),
            Outcome::Activate(hit, ActionId::Edit)
        );
        assert_eq!(
            state.button_click(&index, 2),
            Outcome::Nothing,
            "no such button"
        );
    }
}
