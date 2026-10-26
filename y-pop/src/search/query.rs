//! Running a typed query over the [`Index`].
//!
//! A query is split into words; **every word must match** (`work rem`), each
//! against whichever field of the entry scores best, weighted by
//! [`Field::weight`]. The entry's score is the average of its words'. A query
//! that reads as a hotkey (`win+n`, [`KeyCombo::is_query`]) is also compared
//! with the entry's hotkeys as a set of keys, and the better of the two
//! counts.
//!
//! Ties keep configuration order: entries are visited in index order and the
//! sort is by score alone, descending, then by position. Commands are scored
//! at half, so they sit below an app that matches as well.

use crate::model::NodeId;

use super::hotkey::KeyCombo;
use super::index::{Entry, Field, Index, Kind};
use super::matcher::{Matcher, fold};

/// What was typed, folded and split once per keystroke.
#[derive(Debug, Default)]
pub struct Query {
    words: Vec<Vec<char>>,
    combo: Option<KeyCombo>,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let words = fold(text)
            .split_whitespace()
            .map(|word| word.chars().collect())
            .collect();

        let compact: String = text.split_whitespace().collect();
        let combo = KeyCombo::is_query(&compact)
            .then(|| KeyCombo::parse(&compact))
            .flatten();

        Self { words, combo }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// Which entries a query looks at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scope {
    #[default]
    All,
    /// What is under this group, at any depth (the group itself excluded):
    /// the box after `Tab` on a group.
    Within(NodeId),
}

/// One result: an entry of the index, how well it matched, and by what.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hit {
    /// Position in [`Index::entries`].
    pub entry: u32,
    pub score: u32,
    /// [`Field::Name`] when the name alone explained the match; otherwise the
    /// first field that had to help.
    pub matched: Field,
}

/// Owns the scratch space and the result buffer, both reused from one
/// keystroke to the next.
#[derive(Default)]
pub struct Searcher {
    matcher: Matcher,
    hits: Vec<Hit>,
    /// Added to an entry's score, by position in the index: what history
    /// makes of it. Empty means none. Set once per opening, so the order of
    /// results does not shift between keystrokes.
    bonuses: Vec<u32>,
}

impl Searcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// What history adds to each entry's score, by position in the index.
    pub fn set_bonuses(&mut self, bonuses: Vec<u32>) {
        self.bonuses = bonuses;
    }

    /// The hits for `query`, best first. Empty for an empty query: what the
    /// box shows then (Recent, the sections) is not a search.
    pub fn search(&mut self, index: &Index, query: &Query, scope: Scope) -> &[Hit] {
        self.hits.clear();
        if query.is_empty() {
            return &self.hits;
        }
        for (position, entry) in index.entries().iter().enumerate() {
            if let Scope::Within(group) = scope
                && !entry.ancestors.contains(&group)
            {
                continue;
            }
            if let Some((score, matched)) = self.score(entry, query) {
                let score = score + self.bonuses.get(position).copied().unwrap_or(0);
                self.hits.push(Hit {
                    entry: position as u32,
                    score,
                    matched,
                });
            }
        }
        // Positions are unique, so an unstable sort is still a total order.
        self.hits
            .sort_unstable_by(|a, b| b.score.cmp(&a.score).then(a.entry.cmp(&b.entry)));
        &self.hits
    }

    fn score(&mut self, entry: &Entry, query: &Query) -> Option<(u32, Field)> {
        let by_text = self.score_words(entry, query);
        let by_key = query.combo.as_ref().and_then(|combo| {
            entry
                .hotkeys()
                .iter()
                .filter_map(|key| combo.match_score(key))
                .max()
        });

        let (score, matched) = match (by_text, by_key) {
            (Some(text), Some(key)) if key > text.0 => (key, Field::Hotkey),
            (Some(text), _) => text,
            (None, Some(key)) => (key, Field::Hotkey),
            (None, None) => return None,
        };
        let score = if entry.kind == Kind::Command {
            (score / 2).max(1)
        } else {
            score
        };
        Some((score, matched))
    }

    /// Every word of the query against the best field for it.
    fn score_words(&mut self, entry: &Entry, query: &Query) -> Option<(u32, Field)> {
        let mut total = 0;
        let mut matched = Field::Name;
        let mut helped = false;
        for word in &query.words {
            let mut best: Option<(u32, Field)> = None;
            for (field, text) in entry.fields() {
                let Some(score) = self.matcher.score(word, text) else {
                    continue;
                };
                let score = score * field.weight() / 1000;
                if best.is_none_or(|(top, _)| score > top) {
                    best = Some((score, field));
                }
            }
            let (score, field) = best?;
            total += score;
            if field != Field::Name && !helped {
                (matched, helped) = (field, true);
            }
        }
        Some((total / query.words.len() as u32, matched))
    }

    /// The character positions of `entry`'s name that the query's words
    /// matched, ascending, for emphasizing them. Meant for the rows on
    /// screen only — it allocates.
    pub fn highlight(&mut self, entry: &Entry, query: &Query, positions: &mut Vec<usize>) {
        positions.clear();
        let mut found = Vec::new();
        for word in &query.words {
            if self
                .matcher
                .positions(word, entry.name(), &mut found)
                .is_some()
            {
                positions.extend(&found);
            }
        }
        positions.sort_unstable();
        positions.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::model::ConfigTree;
    use crate::search::index::Source;
    use crate::variables::Variable;

    const MENUS: &str = r#"<y-pop>
        <search>
            <group name="Admin tools">
                <app name="Registry Editor" exe="C:\Windows\regedit.exe"/>
            </group>
        </search>
        <group name="Work">
            <app name="Outlook" exe="C:\Office\OUTLOOK.EXE" keywords="mail email"/>
            <app name="Open Utility Tool" exe="C:\x\tool.exe"/>
            <app name="Command line" exe="C:\Windows\System32\cmd.exe"/>
            <group name="Servers"><app name="Remote Desktop" exe="mstsc.exe"/></group>
            <app name="Task Manager" exe="taskmgr.exe"><hotkey key="Ctrl+Shift+Esc"/></app>
            <app name="Notes" exe="notes.exe"><hotkey key="Win+N"/></app>
            <app name="Café Réseau" exe="cr.exe"/>
            <app name="Editor" exe="a.exe"/>
            <app name="Editor" exe="b.exe"/>
            <app name="Settings Panel" exe="panel.exe"/>
            <separator/>
            <settings/>
        </group>
    </y-pop>"#;

    fn tree() -> ConfigTree {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(MENUS, &mut builtins).expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        tree
    }

    /// The hits' names, best first.
    fn find(index: &Index, text: &str, scope: Scope) -> Vec<String> {
        let mut searcher = Searcher::new();
        let hits = searcher.search(index, &Query::new(text), scope);
        hits.iter()
            .map(|hit| index.entries()[hit.entry as usize].name().to_owned())
            .collect()
    }

    fn top(index: &Index, text: &str) -> (String, Field) {
        let mut searcher = Searcher::new();
        let hits = searcher.search(index, &Query::new(text), Scope::All);
        let hit = hits
            .first()
            .unwrap_or_else(|| panic!("nothing for {text:?}"));
        (
            index.entries()[hit.entry as usize].name().to_owned(),
            hit.matched,
        )
    }

    #[test]
    fn a_name_prefix_beats_initials_and_scattered_letters() {
        let index = Index::from_tree(&tree());
        let names = find(&index, "out", Scope::All);
        assert_eq!(names[0], "outlook");
        assert_eq!(names[1], "open utility tool", "initials come next");
    }

    #[test]
    fn the_exe_file_name_finds_a_program_by_its_other_name() {
        let index = Index::from_tree(&tree());
        assert_eq!(top(&index, "cmd"), ("command line".to_owned(), Field::Exe));
    }

    #[test]
    fn keywords_find_an_entry_and_say_so() {
        let index = Index::from_tree(&tree());
        assert_eq!(top(&index, "mail"), ("outlook".to_owned(), Field::Keywords));
    }

    #[test]
    fn every_word_must_match_and_the_path_counts() {
        let index = Index::from_tree(&tree());
        assert_eq!(
            top(&index, "work rem"),
            ("remote desktop".to_owned(), Field::Path)
        );
        assert_eq!(find(&index, "rem work", Scope::All), ["remote desktop"]);
        assert!(
            find(&index, "work zzz", Scope::All).is_empty(),
            "a word with no match"
        );
    }

    #[test]
    fn a_typed_combination_finds_its_hotkey_in_any_order() {
        let index = Index::from_tree(&tree());
        assert_eq!(top(&index, "win+n"), ("notes".to_owned(), Field::Hotkey));
        assert_eq!(
            top(&index, "shift+ctrl+esc"),
            ("task manager".to_owned(), Field::Hotkey)
        );
        assert_eq!(top(&index, "Ctrl + Shift").0, "task manager");
        assert_eq!(find(&index, "alt+q", Scope::All), Vec::<String>::new());
    }

    #[test]
    fn accents_and_case_are_ignored_on_both_sides() {
        let index = Index::from_tree(&tree());
        assert_eq!(top(&index, "CAFE").0, "cafe reseau");
        assert_eq!(top(&index, "Réseau").0, "cafe reseau");
    }

    #[test]
    fn equal_matches_keep_configuration_order() {
        let index = Index::from_tree(&tree());
        let mut searcher = Searcher::new();
        let hits = searcher.search(&index, &Query::new("editor"), Scope::All);
        let editors: Vec<(&str, u32)> = hits
            .iter()
            .map(|hit| (index.entries()[hit.entry as usize].name(), hit.entry))
            .collect();
        // "Registry Editor" (search-only, last) is a word-start match and ranks
        // below the two exact ones; the two exact ones are in file order.
        assert_eq!(editors[0].0, "editor");
        assert_eq!(editors[1].0, "editor");
        assert!(editors[0].1 < editors[1].1);
    }

    #[test]
    fn history_breaks_a_tie_in_favour_of_what_was_launched() {
        let index = Index::from_tree(&tree());
        let mut searcher = Searcher::new();
        let first = |searcher: &mut Searcher| {
            let hits = searcher.search(&index, &Query::new("editor"), Scope::All);
            hits[0].entry
        };
        let before = first(&mut searcher);

        // The second "Editor" is the one launched.
        let editors: Vec<u32> = index
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name() == "editor")
            .map(|(position, _)| position as u32)
            .collect();
        let mut bonuses = vec![0; index.len()];
        bonuses[editors[1] as usize] = 30;
        searcher.set_bonuses(bonuses);
        assert_eq!(before, editors[0]);
        assert_eq!(first(&mut searcher), editors[1]);
    }

    #[test]
    fn a_command_ranks_below_an_app_that_matches_as_well() {
        let index = Index::from_tree(&tree());
        let names = find(&index, "settings", Scope::All);
        assert_eq!(names, ["settings panel", "settings…"]);
    }

    #[test]
    fn search_only_entries_are_found_and_labelled() {
        let index = Index::from_tree(&tree());
        let mut searcher = Searcher::new();
        let hits = searcher.search(&index, &Query::new("regedit"), Scope::All);
        assert_eq!(hits.len(), 1);
        let entry = &index.entries()[hits[0].entry as usize];
        assert_eq!(entry.source, Source::SearchOnly);
        assert_eq!(hits[0].matched, Field::Exe);
    }

    #[test]
    fn a_scope_limits_the_search_to_what_is_under_a_group() {
        let tree = tree();
        let index = Index::from_tree(&tree);
        let servers = index
            .entries()
            .iter()
            .find(|entry| entry.name() == "servers")
            .unwrap()
            .id;
        assert_eq!(
            find(&index, "e", Scope::Within(servers)),
            ["remote desktop"]
        );
        assert_eq!(top(&index, "servers").0, "servers");
    }

    #[test]
    fn an_empty_query_has_no_hits() {
        let index = Index::from_tree(&tree());
        assert!(find(&index, "", Scope::All).is_empty());
        assert!(find(&index, "   ", Scope::All).is_empty());
    }

    #[test]
    fn highlighting_covers_every_word_of_the_query() {
        let index = Index::from_tree(&tree());
        let entry = index
            .entries()
            .iter()
            .find(|entry| entry.name() == "remote desktop")
            .unwrap();
        let mut positions = Vec::new();
        Searcher::new().highlight(entry, &Query::new("work rem des"), &mut positions);
        assert_eq!(positions, [0, 1, 2, 7, 8, 9]);
    }

    #[test]
    fn five_thousand_entries_are_searched_in_a_blink() {
        let mut index = Index::default();
        for n in 0..5000u32 {
            index.push(
                Entry::new(
                    Source::Menu,
                    NodeId::from_u32(n),
                    Kind::App,
                    &format!("Application number {n} of the long tail"),
                )
                .with_keywords(&format!("keyword{n} other words"))
                .with_exe(&format!(r"C:\Program Files\Vendor {n}\tool{n}.exe"))
                .with_path(&["Work", "Tools", "Everything else"])
                .with_tooltip("A tooltip that says a few things about the application"),
            );
        }

        // A generous bound for a debug build: this catches an accidental
        // quadratic (a 25-million-step run), not a slow machine.
        let mut searcher = Searcher::new();
        let started = std::time::Instant::now();
        for text in ["app", "tail 49", "work tools", "zzzz", "a b c", "win+n"] {
            let hits = searcher.search(&index, &Query::new(text), Scope::All).len();
            assert!(hits <= 5000);
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }
}
