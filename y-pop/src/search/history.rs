//! What was launched from the search box, and when (spec §2.10).
//!
//! A short text file next to the configuration, `y-pop.history`: one line per
//! item, `key <TAB> count <TAB> last launch` (seconds since 1970), at most
//! [`CAP`] lines. It feeds two things: the **Recent** list of the empty box and
//! a small ranking bonus for what is launched often and lately.
//!
//! The key is the item's identity, never its `NodeId`, which changes with every
//! reload: `menu:Work/Outlook` (the names of the groups above it and its own),
//! `search:Admin/Regedit` for a search-only item. Renaming or moving an item
//! therefore starts its history over. A line whose item no longer exists is
//! skipped when listing and dropped at the next write.
//!
//! Everything here is pure except [`load`] and [`save`], the two edges. Time is
//! always a parameter, so the tests need no clock.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::model::ConfigTree;

use super::index::{Entry, Kind, Source};
use super::state::Recent;

/// How many lines the file keeps; the least recent go first.
pub const CAP: usize = 200;

/// The most a launch history can add to a score. Scores run to 1000 and each
/// kind of match has its own band, so this nudges inside a band and across a
/// neighbouring one, but never lifts a scattered match over a prefix.
pub const MAX_BONUS: u32 = 60;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub key: String,
    pub count: u32,
    /// Seconds since 1970.
    pub last: u64,
}

#[derive(Clone, Default, Debug)]
pub struct History {
    records: Vec<Record>,
    dirty: bool,
}

impl History {
    /// Reads a file's text. A line that is not `key`, a count and a time is
    /// ignored; the same key twice keeps the more recent line.
    pub fn parse(text: &str) -> Self {
        let mut history = Self::default();
        for line in text.lines() {
            let mut fields = line.split('\t');
            let (Some(key), Some(count), Some(last)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let (Ok(count), Ok(last)) = (count.trim().parse::<u32>(), last.trim().parse::<u64>())
            else {
                continue;
            };
            if key.is_empty() || count == 0 {
                continue;
            }
            match history.records.iter_mut().find(|record| record.key == key) {
                Some(record) if record.last >= last => {}
                Some(record) => {
                    record.count = count;
                    record.last = last;
                }
                None => history.records.push(Record {
                    key: key.to_owned(),
                    count,
                    last,
                }),
            }
        }
        history
    }

    /// The file's text: most recent first, at most [`CAP`] lines.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for record in self.newest_first().take(CAP) {
            text.push_str(&format!(
                "{}\t{}\t{}\n",
                record.key, record.count, record.last
            ));
        }
        text
    }

    /// Something was launched at `now`.
    pub fn record(&mut self, key: &str, now: u64) {
        self.dirty = true;
        match self.records.iter_mut().find(|record| record.key == key) {
            Some(record) => {
                record.count = record.count.saturating_add(1);
                record.last = now;
            }
            None => self.records.push(Record {
                key: key.to_owned(),
                count: 1,
                last: now,
            }),
        }
    }

    /// Takes one item out of the history. `false` if it was not there.
    pub fn forget(&mut self, key: &str) -> bool {
        let before = self.records.len();
        self.records.retain(|record| record.key != key);
        let removed = self.records.len() != before;
        self.dirty |= removed;
        removed
    }

    /// Takes everything out.
    pub fn clear(&mut self) {
        self.dirty |= !self.records.is_empty();
        self.records.clear();
    }

    pub fn get(&self, key: &str) -> Option<&Record> {
        self.records.iter().find(|record| record.key == key)
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Whether something changed since it was read, so the file is worth
    /// writing.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The records, the last launched first. Equal times keep the order they
    /// were read or added in.
    pub fn newest_first(&self) -> impl Iterator<Item = &Record> {
        let mut order: Vec<&Record> = self.records.iter().collect();
        order.sort_by_key(|record| std::cmp::Reverse(record.last));
        order.into_iter()
    }

    /// Drops the records `exists` refuses (items that are gone) and those past
    /// [`CAP`], the oldest first.
    pub fn prune(&mut self, exists: impl Fn(&str) -> bool) {
        let before = self.records.len();
        self.records.retain(|record| exists(&record.key));
        self.records
            .sort_by_key(|record| std::cmp::Reverse(record.last));
        self.records.truncate(CAP);
        self.dirty |= self.records.len() != before;
    }
}

/// What a record adds to a score at `now`: up to 40 for how often, up to 20 for
/// how lately (a day: all of it, a week: half).
pub fn bonus(record: &Record, now: u64) -> u32 {
    const DAY: u64 = 24 * 60 * 60;
    let often = record.count.min(20) * 2;
    let age = now.saturating_sub(record.last);
    let lately = if age < DAY {
        20
    } else if age < 7 * DAY {
        10
    } else {
        0
    };
    often + lately
}

/// The identity `entry` is remembered under, or `None` for what is not
/// remembered: commands (About, Quit…) and what has no stable identity yet.
pub fn key_of(tree: &ConfigTree, entry: &Entry) -> Option<String> {
    let prefix = match entry.source {
        Source::Menu => "menu",
        Source::SearchOnly => "search",
        Source::StartMenu => return None,
    };
    if entry.kind == Kind::Command {
        return None;
    }
    let mut key = String::from(prefix);
    key.push(':');
    for &group in &entry.ancestors {
        key.push_str(&tree.node(group).name);
        key.push('/');
    }
    key.push_str(&tree.node(entry.id).name);
    Some(key)
}

/// The keys of every remembered-able entry, for [`History::prune`].
pub fn live_keys(tree: &ConfigTree, entries: &[Entry]) -> HashSet<String> {
    entries
        .iter()
        .filter_map(|entry| key_of(tree, entry))
        .collect()
}

/// What `history` makes of an opening: the **Recent** rows (at most `count`,
/// newest first, only for items that still exist) and the score bonus of each
/// entry, by position. Computed once per opening.
pub fn listing(
    tree: &ConfigTree,
    entries: &[Entry],
    history: &History,
    count: usize,
    now: u64,
) -> (Vec<Recent>, Vec<u32>) {
    let keys: Vec<Option<String>> = entries.iter().map(|entry| key_of(tree, entry)).collect();
    let bonuses = keys
        .iter()
        .map(|key| {
            key.as_deref()
                .and_then(|key| history.get(key))
                .map_or(0, |record| bonus(record, now))
        })
        .collect();
    let recent = history
        .newest_first()
        .filter_map(|record| {
            let entry = keys
                .iter()
                .position(|key| key.as_deref() == Some(record.key.as_str()))?;
            Some(Recent {
                entry: entry as u32,
                last: record.last,
            })
        })
        .take(count)
        .collect();
    (recent, bonuses)
}

/// `y-pop.history`, next to the configuration file.
pub fn path() -> PathBuf {
    crate::config::path().with_file_name("y-pop.history")
}

/// The file's history; an empty one if there is none or it cannot be read.
pub fn load() -> History {
    std::fs::read_to_string(path())
        .map(|text| History::parse(&text))
        .unwrap_or_default()
}

/// Writes the history, if it changed. A failure is logged and otherwise
/// ignored: losing a launch count is not worth an error box.
pub fn save(history: &History) {
    if !history.is_dirty() {
        return;
    }
    let path = path();
    if let Err(error) = std::fs::write(&path, history.to_text()) {
        y_core::log::line(format_args!("{}: {error}", path.display()));
    }
}

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::search::index::Index;
    use crate::variables::Variable;

    #[test]
    fn a_launch_counts_and_remembers_when() {
        let mut history = History::default();
        history.record("menu:Work/Outlook", 100);
        history.record("menu:Work/Outlook", 250);
        history.record("menu:Notes", 200);
        let outlook = history.get("menu:Work/Outlook").unwrap();
        assert_eq!((outlook.count, outlook.last), (2, 250));
        assert!(history.is_dirty());
        assert_eq!(
            history
                .newest_first()
                .map(|r| r.key.as_str())
                .collect::<Vec<_>>(),
            ["menu:Work/Outlook", "menu:Notes"]
        );
    }

    #[test]
    fn the_text_round_trips_and_a_bad_line_is_skipped() {
        let mut history = History::default();
        history.record("menu:A", 10);
        history.record("menu:B", 20);
        history.record("menu:B", 30);
        let again = History::parse(&history.to_text());
        assert_eq!(again.get("menu:B").unwrap().count, 2);
        assert_eq!(again.get("menu:A").unwrap().last, 10);
        assert!(!again.is_dirty(), "just read, nothing to write back");

        let messy = "menu:Ok\t3\t99\nno tabs here\nmenu:Bad\tx\t1\nmenu:Zero\t0\t5\n\t1\t1\nmenu:Ok\t1\t50\n";
        let parsed = History::parse(messy);
        assert_eq!(parsed.newest_first().count(), 1);
        assert_eq!(
            parsed.get("menu:Ok").unwrap().count,
            3,
            "the later line wins"
        );
    }

    #[test]
    fn forgetting_and_clearing() {
        let mut history = History::parse("menu:A\t1\t1\nmenu:B\t1\t2\n");
        assert!(!history.forget("menu:Z"));
        assert!(!history.is_dirty());
        assert!(history.forget("menu:A"));
        assert!(history.is_dirty());
        assert!(history.get("menu:A").is_none());
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn pruning_drops_what_is_gone_and_what_is_past_the_cap() {
        let mut text = String::new();
        for n in 0..CAP + 20 {
            text.push_str(&format!("menu:Item{n}\t1\t{n}\n"));
        }
        let mut history = History::parse(&text);
        history.prune(|key| key != "menu:Item150");
        assert!(history.get("menu:Item150").is_none(), "gone");
        assert_eq!(history.newest_first().count(), CAP);
        assert!(history.get("menu:Item0").is_none(), "the oldest went first");
        assert!(history.get("menu:Item219").is_some());
        assert_eq!(history.to_text().lines().count(), CAP);
    }

    #[test]
    fn the_bonus_grows_with_use_and_fades_with_time_and_is_bounded() {
        const DAY: u64 = 24 * 60 * 60;
        let record = |count, last| Record {
            key: String::new(),
            count,
            last,
        };
        let now = 100 * DAY;
        assert_eq!(bonus(&record(1, now), now), 2 + 20);
        assert_eq!(bonus(&record(1, now - 3 * DAY), now), 2 + 10);
        assert_eq!(bonus(&record(1, now - 30 * DAY), now), 2);
        assert!(bonus(&record(5, now), now) > bonus(&record(1, now), now));
        assert_eq!(bonus(&record(u32::MAX, now), now), MAX_BONUS);
        assert_eq!(
            bonus(&record(1, now + 5), now),
            2 + 20,
            "a clock that went back"
        );
    }

    fn index() -> (ConfigTree, Index) {
        let tree = config::load_str(
            r#"<y-pop>
                <search><group name="Admin"><app name="Regedit" exe="r.exe"/></group></search>
                <group name="Work">
                    <app name="Outlook" exe="o.exe"/>
                    <group name="Servers"><app name="Putty" exe="p.exe"/></group>
                    <quit/>
                </group>
            </y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .unwrap();
        let index = Index::from_tree(&tree);
        (tree, index)
    }

    fn key(tree: &ConfigTree, index: &Index, name: &str) -> Option<String> {
        let entry = index
            .entries()
            .iter()
            .find(|entry| entry.name() == name)
            .unwrap();
        key_of(tree, entry)
    }

    #[test]
    fn an_item_is_known_by_the_names_above_it() {
        let (tree, index) = index();
        assert_eq!(
            key(&tree, &index, "outlook").as_deref(),
            Some("menu:Work/Outlook")
        );
        assert_eq!(
            key(&tree, &index, "putty").as_deref(),
            Some("menu:Work/Servers/Putty")
        );
        assert_eq!(key(&tree, &index, "work").as_deref(), Some("menu:Work"));
        assert_eq!(
            key(&tree, &index, "regedit").as_deref(),
            Some("search:Admin/Regedit"),
            "search-only items are kept apart from the menus'"
        );
        assert_eq!(
            key(&tree, &index, "quit"),
            None,
            "commands are not remembered"
        );
    }

    #[test]
    fn an_opening_lists_the_recent_items_that_exist_and_a_bonus_for_each() {
        let (tree, index) = index();
        let history = History::parse(
            "menu:Work/Outlook\t2\t300\nmenu:Gone\t9\t400\nsearch:Admin/Regedit\t1\t200\nmenu:Work/Servers/Putty\t1\t100\n",
        );
        let (recent, bonuses) = listing(&tree, index.entries(), &history, 2, 300);

        let names: Vec<_> = recent
            .iter()
            .map(|r| index.entries()[r.entry as usize].name())
            .collect();
        assert_eq!(
            names,
            ["outlook", "regedit"],
            "newest first, the vanished one skipped, two of them"
        );
        assert_eq!(recent[0].last, 300);

        assert_eq!(bonuses.len(), index.len());
        let bonus_of = |name: &str| {
            let position = index
                .entries()
                .iter()
                .position(|e| e.name() == name)
                .unwrap();
            bonuses[position]
        };
        assert_eq!(bonus_of("outlook"), 2 * 2 + 20);
        assert_eq!(bonus_of("work"), 0, "never launched");
    }
    #[test]
    fn live_keys_are_what_prune_keeps() {
        let (tree, index) = index();
        let live = live_keys(&tree, index.entries());
        let mut history = History::parse("menu:Work/Outlook\t1\t5\nmenu:Work/Renamed\t1\t6\n");
        history.prune(|key| live.contains(key));
        assert!(history.get("menu:Work/Outlook").is_some());
        assert!(history.get("menu:Work/Renamed").is_none());
    }
}
