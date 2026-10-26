//! What the search box is doing: the text typed, the results for it and which
//! one is selected. Pure — the window turns key presses and mouse movement
//! into [`Event`]s, and acts on the [`Outcome`] each one returns.
//!
//! Splitting it this way keeps every key of spec §2 a unit test, and leaves
//! the window with the Win32 half only: reading the keyboard, the clipboard,
//! painting.

use crate::model::{ActionId, ActionsConfig, NodeId, ResultsLayout};

use super::actions::{self, Available};
use super::index::{Field, Index, Kind};
use super::input::Input;
use super::layout::RowKind;
use super::query::{Hit, Query, Scope, Searcher};
use super::results::{self, Heading};

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
}

/// One row of the list, in the order shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Header(Heading),
    Result(Hit),
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
    scopes: Vec<NodeId>,
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
    /// The last launches, newest first, listed by the empty box.
    recent: Vec<Recent>,
    /// `collapsed="true"`: the list can be folded away, leaving the input row.
    collapsible: bool,
    /// Folded now: no rows, whatever the text. Only ever true when collapsible.
    folded: bool,
}

/// One launch of the history that still has an entry: what the empty box
/// lists under **Recent**.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Recent {
    /// Position in [`Index::entries`].
    pub entry: u32,
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
            collapsible: false,
            folded: false,
        }
    }

    /// Sets what the empty box lists under **Recent** and what history adds to
    /// the scores, and shows the empty box. Called once, as the window opens.
    pub fn start(&mut self, index: &Index, recent: Vec<Recent>, bonuses: Vec<u32>) {
        self.recent = recent;
        self.searcher.set_bonuses(bonuses);
        self.refresh(index);
    }

    /// Whether `hit` is a row of Recent: the empty box, outside any group.
    fn is_recent(&self, hit: Hit) -> bool {
        self.last_launch(hit).is_some()
    }

    /// When `hit` was launched, if it is listed under Recent right now.
    pub fn last_launch(&self, hit: Hit) -> Option<u64> {
        if !self.query.is_empty() || !self.scopes.is_empty() {
            return None;
        }
        self.recent
            .iter()
            .find(|recent| recent.entry == hit.entry)
            .map(|recent| recent.last)
    }

    /// Takes the launch of `entry` off the Recent list (it was forgotten), the
    /// selection staying where it was.
    pub fn remove_recent(&mut self, index: &Index, entry: u32) {
        let at = self.selected;
        self.recent.retain(|recent| recent.entry != entry);
        self.refresh(index);
        if let Some(at) = at {
            let same_place = matches!(self.rows.get(at), Some(Row::Result(_))).then_some(at);
            self.selected = same_place.or_else(|| {
                self.rows
                    .iter()
                    .rposition(|row| matches!(row, Row::Result(_)))
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
            && self.selected
                == self
                    .rows
                    .iter()
                    .position(|row| matches!(row, Row::Result(_)))
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
    pub fn scopes(&self) -> &[NodeId] {
        &self.scopes
    }

    fn scope(&self) -> Scope {
        self.scopes
            .last()
            .map_or(Scope::All, |&group| Scope::Within(group))
    }

    /// The rows as the layout wants to know them.
    pub fn row_kinds(&self) -> Vec<RowKind> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::Header(_) => RowKind::Header,
                Row::Result(_) => RowKind::Item,
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
            Row::Header(_) => None,
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
        match self.rows.get(row) {
            Some(Row::Result(_)) if self.selected != Some(row) => {
                self.selected = Some(row);
                true
            }
            _ => false,
        }
    }

    /// A click on `row`: selects it and, if it is a result, opens it the way
    /// Enter would.
    pub fn click_row(&mut self, index: &Index, row: usize) -> Outcome {
        if !matches!(self.rows.get(row), Some(Row::Result(_))) {
            return Outcome::Nothing;
        }
        self.selected = Some(row);
        self.perform(index, ActionId::Open)
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
        let Some(hit) = self.selected_hit() else {
            return Outcome::Nothing;
        };
        let entry = &index.entries()[hit.entry as usize];
        if entry.kind != Kind::Group {
            return Outcome::Nothing;
        }
        self.scopes.push(entry.id);
        self.input.clear();
        self.refresh(index);
        Outcome::Redraw
    }

    /// Back out of the innermost group.
    fn leave_scope(&mut self, index: &Index) -> Outcome {
        if self.scopes.pop().is_none() {
            return Outcome::Nothing;
        }
        self.refresh(index);
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
            .filter_map(|(row, kind)| matches!(kind, Row::Result(_)).then_some(row))
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
    /// With nothing typed and no group entered the rows are the **Recent**
    /// launches, if there are any (the sections are task 6.4). Inside a
    /// group, an empty box lists what the group holds, in file order.
    fn refresh(&mut self, index: &Index) {
        self.menu = None;
        self.query = Query::new(self.input.text());
        self.rows.clear();
        if self.folded {
            self.selected = None;
            return;
        }

        if self.query.is_empty() {
            if self.scopes.is_empty() {
                if !self.recent.is_empty() {
                    self.rows.push(Row::Header(Heading::Recent));
                    self.rows.extend(self.recent.iter().map(|recent| {
                        Row::Result(Hit {
                            entry: recent.entry,
                            score: 0,
                            matched: Field::Name,
                        })
                    }));
                }
            } else if let Some(&group) = self.scopes.last() {
                self.rows.extend(
                    index
                        .entries()
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| entry.ancestors.last() == Some(&group))
                        .map(|(position, _)| {
                            Row::Result(Hit {
                                entry: position as u32,
                                score: 0,
                                matched: Field::Name,
                            })
                        }),
                );
            }
        } else {
            let hits = self
                .searcher
                .search(index, &self.query, self.scope())
                .to_vec();
            for group in results::arrange(index, &hits, self.layout) {
                if group.heading != Heading::All {
                    self.rows.push(Row::Header(group.heading));
                }
                self.rows.extend(group.hits.into_iter().map(Row::Result));
            }
        }
        self.selected = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Result(_)));
    }
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
        assert!(state.rows().is_empty());
    }

    #[test]
    fn escape_clears_the_text_then_closes() {
        let (mut state, index) = (state(), index());
        type_text(&mut state, &index, "not");
        assert_eq!(press(&mut state, &index, Key::Escape), Outcome::Redraw);
        assert!(state.input().is_empty());
        assert!(state.rows().is_empty());
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
        assert!(state.rows().is_empty(), "and the results follow");

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
                Row::Header(_) => None,
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
        assert!(state.rows().is_empty());
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
                entry: position(&index, name),
                last: 1000 - n as u64,
            })
            .collect();
        state.start(&index, recent, Vec::new());
        (state, index)
    }

    fn row_names(state: &State, index: &Index) -> Vec<String> {
        state
            .rows()
            .iter()
            .map(|row| match row {
                Row::Header(heading) => format!("[{heading:?}]"),
                Row::Result(hit) => index.entries()[hit.entry as usize].name().to_owned(),
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
            state.last_launch(hit),
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
                Row::Result(hit) => state.last_launch(*hit),
                Row::Header(_) => None,
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
        assert!(state.rows().is_empty());
        assert_eq!(state.selected(), None);
    }

    fn collapsed(names: &[&str]) -> (State, Index) {
        let (state, index) = with_recent(names);
        let mut state = state.with_collapsed(true);
        let recent = names
            .iter()
            .enumerate()
            .map(|(n, name)| Recent {
                entry: position(&index, name),
                last: 1000 - n as u64,
            })
            .collect();
        state.start(&index, recent, Vec::new());
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
