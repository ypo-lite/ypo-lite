//! What the search box is doing: the text typed, the results for it and which
//! one is selected. Pure — the window turns key presses and mouse movement
//! into [`Event`]s, and acts on the [`Outcome`] each one returns.
//!
//! Splitting it this way keeps every key of spec §2 a unit test, and leaves
//! the window with the Win32 half only: reading the keyboard, the clipboard,
//! painting.

use crate::model::{NodeId, ResultsLayout};

use super::index::{Field, Index, Kind};
use super::input::Input;
use super::layout::RowKind;
use super::query::{Hit, Query, Scope, Searcher};
use super::results::{self, Heading};

/// How many results `PgUp`/`PgDn` move over.
pub const PAGE: usize = 5;

/// Modifier keys held when a key went down.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A key the box reacts to. Typed characters are [`Event::Char`], not keys;
/// `Letter` exists for the Ctrl shortcuts only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    /// An upper-case letter, for `Ctrl+A/C/V/X`.
    Letter(char),
}

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
    /// Act on this result (Enter, or a click).
    Activate(Hit),
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
}

impl State {
    pub fn new(layout: ResultsLayout) -> Self {
        Self {
            input: Input::new(),
            query: Query::default(),
            scopes: Vec::new(),
            layout,
            searcher: Searcher::new(),
            rows: Vec::new(),
            selected: None,
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
        self.open_selected(index)
    }

    /// What Enter does: a group is entered (it becomes a chip and the box
    /// searches inside it); anything else is acted on.
    fn open_selected(&mut self, index: &Index) -> Outcome {
        let Some(hit) = self.selected_hit() else {
            return Outcome::Nothing;
        };
        if index.entries()[hit.entry as usize].kind == Kind::Group {
            self.enter_selected(index)
        } else {
            Outcome::Activate(hit)
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
        match event {
            Event::Char(c) => self.edited(index, |input| input.insert(c.encode_utf8(&mut [0; 4]))),
            Event::Paste(text) => self.edited(index, |input| input.insert(&text)),
            Event::Key(key, mods) => self.key(index, key, mods),
        }
    }

    fn key(&mut self, index: &Index, key: Key, mods: Mods) -> Outcome {
        let Mods { ctrl, shift, .. } = mods;
        match key {
            // The text first, then the groups entered, then the window.
            Key::Escape if !self.input.is_empty() => self.edited(index, Input::clear),
            Key::Escape if !self.scopes.is_empty() => self.leave_scope(index),
            Key::Escape => Outcome::Close,

            Key::Enter => self.open_selected(index),

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
            Key::Letter('C') if ctrl && self.input.has_selection() => {
                Outcome::Copy(self.input.selected_text().to_owned())
            }
            Key::Letter('X') if ctrl => {
                let cut = self.input.cut();
                match cut {
                    Some(text) => {
                        self.refresh(index);
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
            self.refresh(index);
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
    /// With nothing typed and no group entered there are no rows: what the
    /// empty box shows (Recent, the sections) is a later phase. Inside a
    /// group, an empty box lists what the group holds, in file order.
    fn refresh(&mut self, index: &Index) {
        self.query = Query::new(self.input.text());
        self.rows.clear();

        if self.query.is_empty() {
            if let Some(&group) = self.scopes.last() {
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
        State::new(ResultsLayout::Sections)
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
        let Outcome::Activate(hit) = press(&mut state, &index, Key::Enter) else {
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
        let Outcome::Activate(hit) = state.click_row(&index, first) else {
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
        assert_eq!(
            state.handle(&index, Event::Key(Key::Letter('C'), ctrl)),
            Outcome::Nothing,
            "nothing selected, nothing copied"
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
        let Outcome::Activate(hit) = press(&mut state, &index, Key::Enter) else {
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

    #[test]
    fn a_flat_layout_has_no_headers() {
        let (mut state, index) = (State::new(ResultsLayout::Flat), index());
        type_text(&mut state, &index, "not");
        assert!(state.rows().iter().all(|row| matches!(row, Row::Result(_))));
        assert_eq!(state.selected(), Some(0));
    }
}
