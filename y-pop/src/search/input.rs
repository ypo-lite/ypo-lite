//! The text box's state machine: the text, a caret and a selection, and the
//! editing keys. Pure — no Win32 — so every key is a unit test.
//!
//! Positions the outside world sees ([`Input::caret`], [`Input::selection`])
//! are in **UTF-16 units**, the unit GDI measures text in
//! (`y_core::overlay::Font::caret_offsets`), so a renderer can turn them into
//! pixels directly. Inside, the text is a `String` and positions are byte
//! offsets, which keeps every edit on a character boundary; a character
//! outside the Basic Multilingual Plane counts as two units outside and one
//! step for every key.
//!
//! Editing methods return whether the **text** changed (a search re-runs on
//! it); moving the caret returns nothing, since the caret is always redrawn.

use std::ops::Range;

/// The longest text the box takes, in characters. A pasted paragraph is cut
/// rather than laid out.
pub const MAX_CHARS: usize = 200;

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Input {
    text: String,
    /// Where the caret is, as a byte offset.
    caret: usize,
    /// The other end of the selection; equal to `caret` when there is none.
    anchor: usize,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The caret, in UTF-16 units from the start.
    pub fn caret(&self) -> usize {
        self.units(self.caret)
    }

    /// The selected range in UTF-16 units; empty (`start == end`) when
    /// nothing is selected.
    pub fn selection(&self) -> Range<usize> {
        let (start, end) = self.bounds();
        self.units(start)..self.units(end)
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self) -> &str {
        let (start, end) = self.bounds();
        &self.text[start..end]
    }

    // --- Replacing the text -------------------------------------------------

    /// Types or pastes `text` over the selection. Control characters are
    /// dropped, except that a line break or tab becomes a space.
    pub fn insert(&mut self, text: &str) -> bool {
        let cleaned: String = text
            .chars()
            .filter_map(|c| match c {
                '\r' | '\n' | '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect();
        if cleaned.is_empty() {
            return false;
        }

        let (start, end) = self.bounds();
        let room = MAX_CHARS
            .saturating_sub(self.text.chars().count() - self.text[start..end].chars().count());
        let kept: String = cleaned.chars().take(room).collect();
        if kept.is_empty() && start == end {
            return false;
        }

        self.text.replace_range(start..end, &kept);
        self.caret = start + kept.len();
        self.anchor = self.caret;
        true
    }

    /// Replaces everything with `text`, the caret at its end — what a prefix
    /// button does.
    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.insert(text);
    }

    pub fn clear(&mut self) -> bool {
        let changed = !self.text.is_empty();
        self.set_text("");
        changed
    }

    /// Backspace: the selection, else the character before the caret.
    pub fn backspace(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection();
        }
        let from = self.previous_char(self.caret);
        self.delete_range(from, self.caret)
    }

    /// Delete: the selection, else the character after the caret.
    pub fn delete(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection();
        }
        let to = self.next_char(self.caret);
        self.delete_range(self.caret, to)
    }

    /// Ctrl+Backspace: the selection, else back to the start of the word.
    pub fn backspace_word(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection();
        }
        let from = self.previous_word(self.caret);
        self.delete_range(from, self.caret)
    }

    /// Ctrl+Delete: the selection, else on to the start of the next word.
    pub fn delete_word(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection();
        }
        let to = self.next_word(self.caret);
        self.delete_range(self.caret, to)
    }

    /// Ctrl+X: the selected text, removed. `None` with nothing selected.
    pub fn cut(&mut self) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        let cut = self.selected_text().to_owned();
        self.delete_selection();
        Some(cut)
    }

    // --- Moving the caret ---------------------------------------------------

    /// Left (or Ctrl+Left by `word`). `extend` is Shift: grow the selection
    /// instead of dropping it. Without it, a selection collapses to its
    /// start, as in any text box.
    pub fn move_left(&mut self, extend: bool, word: bool) {
        if !extend && self.has_selection() {
            let (start, _) = self.bounds();
            self.place(start, false);
            return;
        }
        let to = if word {
            self.previous_word(self.caret)
        } else {
            self.previous_char(self.caret)
        };
        self.place(to, extend);
    }

    pub fn move_right(&mut self, extend: bool, word: bool) {
        if !extend && self.has_selection() {
            let (_, end) = self.bounds();
            self.place(end, false);
            return;
        }
        let to = if word {
            self.next_word(self.caret)
        } else {
            self.next_char(self.caret)
        };
        self.place(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.place(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.place(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    // --- Internals ----------------------------------------------------------

    fn bounds(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn units(&self, byte: usize) -> usize {
        self.text[..byte].encode_utf16().count()
    }

    fn place(&mut self, caret: usize, extend: bool) {
        self.caret = caret;
        if !extend {
            self.anchor = caret;
        }
    }

    fn delete_selection(&mut self) -> bool {
        let (start, end) = self.bounds();
        self.delete_range(start, end)
    }

    fn delete_range(&mut self, start: usize, end: usize) -> bool {
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    fn previous_char(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_char(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map_or(at, |c| at + c.len_utf8())
    }

    /// The start of the word before `at`: skip separators backwards, then
    /// the word's own letters.
    fn previous_word(&self, at: usize) -> usize {
        let before = &self.text[..at];
        let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric());
        trimmed.trim_end_matches(char::is_alphanumeric).len()
    }

    /// The start of the next word: past the rest of this one, then past the
    /// separators after it.
    fn next_word(&self, at: usize) -> usize {
        let after = &self.text[at..];
        let rest = after.trim_start_matches(char::is_alphanumeric);
        let rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
        self.text.len() - rest.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> Input {
        let mut input = Input::new();
        input.insert(text);
        input
    }

    #[test]
    fn typing_inserts_at_the_caret_and_moves_it() {
        let mut input = input("hello");
        input.move_left(false, false);
        input.move_left(false, false);
        assert!(input.insert("XY"));
        assert_eq!(input.text(), "helXYlo");
        assert_eq!(input.caret(), 5);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut input = input("hello world");
        input.select_all();
        input.insert("x");
        assert_eq!(input.text(), "x");
        assert!(!input.has_selection());
    }

    #[test]
    fn control_characters_are_dropped_and_breaks_become_spaces() {
        let mut input = Input::new();
        assert!(!input.insert("\u{1b}\u{7f}"), "nothing left to insert");
        input.insert("a\r\nb\tc\u{8}");
        assert_eq!(input.text(), "a  b c");
    }

    #[test]
    fn the_text_is_capped() {
        let mut input = Input::new();
        input.insert(&"x".repeat(MAX_CHARS + 50));
        assert_eq!(input.text().chars().count(), MAX_CHARS);
        assert!(!input.insert("more"), "full");
        input.select_all();
        assert!(input.insert("y"), "a selection makes room");
        assert_eq!(input.text(), "y");
    }

    #[test]
    fn backspace_and_delete_take_one_character_or_the_selection() {
        let mut input = input("abcd");
        input.move_left(false, false);
        assert!(input.backspace());
        assert_eq!(input.text(), "abd");
        assert!(input.delete());
        assert_eq!(input.text(), "ab");
        assert!(!input.delete(), "nothing after the caret");
        input.home(false);
        assert!(!input.backspace(), "nothing before the caret");

        input.select_all();
        assert!(input.backspace());
        assert!(input.is_empty());
    }

    #[test]
    fn ctrl_backspace_and_ctrl_delete_work_by_word() {
        let mut input = input("open  the door");
        assert!(input.backspace_word());
        assert_eq!(input.text(), "open  the ");
        assert!(input.backspace_word());
        assert_eq!(input.text(), "open  ");
        assert!(input.backspace_word());
        assert_eq!(input.text(), "");

        let mut input = self::input("open  the door");
        input.home(false);
        assert!(input.delete_word());
        assert_eq!(input.text(), "the door");
        assert!(input.delete_word());
        assert_eq!(input.text(), "door");
        assert!(input.delete_word());
        assert_eq!(input.text(), "");
        assert!(!input.delete_word());
    }

    #[test]
    fn words_stop_at_any_non_alphanumeric() {
        let mut input = input(r"C:\Tools\git-bash");
        input.move_left(false, true);
        assert_eq!(input.caret(), 13, "start of bash");
        input.move_left(false, true);
        assert_eq!(input.caret(), 9, "start of git");
        input.move_left(false, true);
        assert_eq!(input.caret(), 3, "start of Tools");
        input.move_right(false, true);
        assert_eq!(input.caret(), 9);
    }

    #[test]
    fn arrows_collapse_a_selection_to_its_edges() {
        let mut input = input("abcdef");
        input.move_left(false, false);
        input.move_left(true, false);
        input.move_left(true, false);
        assert_eq!(input.selection(), 3..5);

        let mut left = input.clone();
        left.move_left(false, false);
        assert_eq!((left.caret(), left.has_selection()), (3, false));

        input.move_right(false, false);
        assert_eq!((input.caret(), input.has_selection()), (5, false));
    }

    #[test]
    fn shift_extends_in_both_directions() {
        let mut input = input("abc");
        input.home(false);
        input.move_right(true, false);
        input.move_right(true, false);
        assert_eq!((input.selection(), input.selected_text()), (0..2, "ab"));
        input.move_left(true, false);
        assert_eq!(input.selection(), 0..1);
        input.end(true);
        assert_eq!(input.selection(), 0..3);
        input.home(true);
        assert!(!input.has_selection(), "shrunk back to nothing");
    }

    #[test]
    fn cut_returns_the_selection_and_removes_it() {
        let mut input = input("abcdef");
        assert_eq!(input.cut(), None);
        input.home(false);
        input.move_right(true, false);
        input.move_right(true, false);
        assert_eq!(input.cut().as_deref(), Some("ab"));
        assert_eq!(input.text(), "cdef");
        assert_eq!(input.caret(), 0);
    }

    #[test]
    fn set_text_replaces_everything_with_the_caret_at_the_end() {
        let mut input = input("old");
        input.select_all();
        input.set_text(">");
        assert_eq!(
            (input.text(), input.caret(), input.has_selection()),
            (">", 1, false)
        );
        assert!(input.clear());
        assert!(!input.clear());
    }

    #[test]
    fn positions_count_utf16_units_but_keys_step_by_character() {
        // 😀 is one character and two UTF-16 units; é is one of each.
        let mut input = input("a😀é");
        assert_eq!(input.caret(), 4);
        input.move_left(false, false);
        assert_eq!(input.caret(), 3, "over é");
        input.move_left(false, false);
        assert_eq!(input.caret(), 1, "over the whole emoji, not half of it");
        input.move_right(true, false);
        assert_eq!(input.selection(), 1..3);
        assert!(input.backspace());
        assert_eq!(input.text(), "aé");
    }

    #[test]
    fn an_empty_box_survives_every_key() {
        let mut input = Input::new();
        input.move_left(true, true);
        input.move_right(true, true);
        input.home(true);
        input.end(false);
        input.select_all();
        assert!(!input.backspace());
        assert!(!input.delete());
        assert!(!input.backspace_word());
        assert!(!input.delete_word());
        assert_eq!((input.caret(), input.selection()), (0, 0..0));
    }
}
