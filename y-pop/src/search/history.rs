//! The history is Windows's (spec §2.10): what the Start menu's "recent" and
//! "most used" lists are made of, read through [`y_core::recent`]. y-pop keeps
//! no list of its own; what it launches reaches this one through
//! [`crate::launch`], which tells Windows.
//!
//! It feeds the **Recent** list of the empty box, the full list behind it, and
//! a small ranking bonus for what was launched lately.
//!
//! # What is listed
//!
//! Windows's record of programs holds everything that ever ran: installers,
//! something in a temporary folder, a tool started once. So a launch is kept
//! only if it is a tray-menu or search-only item, which it then *is* (the row
//! is that item, with its name, hotkey and actions), or an app of
//! `shell:AppsFolder`, which is the Start menu's "All apps". Recent files are
//! kept as they are.
//!
//! [`listing`] is pure: the launches, files and apps are parameters, and time
//! too, so the tests need neither a registry nor a clock. Those read by
//! [`AppNames`] and at each opening are the only edges.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use y_core::icon::{Icon, IconCache};
use y_core::recent::{self, Launch, RecentFile};

use crate::model::{ConfigTree, NodeKind};

use super::index::{Entry, Facts, Index, Kind, Target};
use super::state::Recent;

/// How many items the list keeps; the oldest go first.
pub const CAP: usize = 200;

/// The most history can add to a score. Scores run to 1000 and each kind of
/// match has its own band, so this nudges inside a band and across a
/// neighbouring one, but never lifts a scattered match over a prefix.
pub const MAX_BONUS: u32 = 60;

/// How long the list of apps is trusted before the next opening reads it again,
/// behind the scenes.
const APPS_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// One thing Windows remembers that y-pop lists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// As the user reads it, in its own case: what the index folds.
    pub name: String,
    pub origin: Origin,
    /// Other records of the same thing: Windows keeps a program apart from a
    /// shortcut to it. Forgetting takes them too, or the next opening would
    /// list it again.
    pub older: Vec<Launch>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    App(Launch),
    File(RecentFile),
}

impl Item {
    /// What the shell opens for it: the shortcut of a file, the Start menu's
    /// address of an app.
    pub fn address(&self) -> String {
        match &self.origin {
            Origin::App(launch) => format!(r"shell:AppsFolder\{}", launch.id),
            Origin::File(file) => file.link.display().to_string(),
        }
    }

    /// The file it stands for, for showing it or copying its path: read from
    /// the shortcut, for a file. `None` for an app that is no file.
    pub fn file(&self) -> Option<String> {
        match &self.origin {
            Origin::App(launch) => launch.path(),
            Origin::File(file) => y_core::file_info::read_shell_link(&file.link)
                .map(|link| link.target)
                .filter(|target| !target.is_empty()),
        }
    }

    /// The second line of its row: where it is. Reads a shortcut, so it is for
    /// the rows on screen.
    pub fn detail(&self) -> String {
        match &self.origin {
            Origin::App(_) => self.file().unwrap_or_else(|| "App".to_owned()),
            Origin::File(_) => self
                .file()
                .as_deref()
                .and_then(|file| Path::new(file).parent())
                .map_or_else(|| "File".to_owned(), |folder| folder.display().to_string()),
        }
    }

    pub fn icon(&self, icons: &mut IconCache) -> Option<Icon> {
        match &self.origin {
            Origin::App(launch) => match launch.path() {
                Some(path) => icons.for_file(&path, None),
                None => icons.for_store_app(&launch.id),
            },
            Origin::File(file) => icons.for_file(&file.link.display().to_string(), None),
        }
    }

    /// Asks Windows to forget it. `false` if it could not.
    pub fn forget(&self) -> bool {
        let forgotten = match &self.origin {
            Origin::App(launch) => recent::forget_launch(launch),
            Origin::File(file) => recent::forget_file(file),
        };
        self.older.iter().fold(forgotten, |forgotten, launch| {
            recent::forget_launch(launch) && forgotten
        })
    }
}

/// What the history makes of an opening.
#[derive(Debug, Default)]
pub struct Listing {
    /// What [`Target::History`] and [`Recent::item`] point into.
    pub items: Vec<Item>,
    /// Everything listed, newest first; the empty box shows the first few.
    pub recent: Vec<Recent>,
    /// What history adds to each entry's score, by position in the index.
    pub bonuses: Vec<u32>,
}

/// What a launch adds to a score at `now`: up to 40 for how often, up to 20 for
/// how lately (a day: all of it, a week: half). A launch with no count did
/// happen (see [`y_core::recent`]), and counts as one.
pub fn bonus(count: u32, last: u64, now: u64) -> u32 {
    const DAY: u64 = 24 * 60 * 60;
    let often = count.clamp(1, 20) * 2;
    let age = now.saturating_sub(last);
    let lately = if age < DAY {
        20
    } else if age < 7 * DAY {
        10
    } else {
        0
    };
    often + lately
}

/// The entries of the tree a launch can be, by what Windows would call them.
#[derive(Default)]
struct TreeApps {
    /// Lower case: full paths and app ids.
    exact: HashMap<String, u32>,
    /// Lower case: `notepad.exe`, an `exe` given with no folder.
    bare: HashMap<String, u32>,
}

impl TreeApps {
    fn of(tree: &ConfigTree, index: &Index) -> Self {
        let mut apps = Self::default();
        for (position, entry) in index.entries().iter().enumerate() {
            let Some(id) = entry.node() else { continue };
            let NodeKind::App(app) = &tree.node(id).kind else {
                continue;
            };
            let position = position as u32;
            if let Some(app_id) = &app.app_id {
                apps.exact.entry(app_id.to_lowercase()).or_insert(position);
            }
            if let Some(exe) = &app.exe {
                let exe = exe.to_lowercase();
                let names = if exe.contains(['\\', '/']) {
                    &mut apps.exact
                } else {
                    &mut apps.bare
                };
                names.entry(exe).or_insert(position);
            }
        }
        apps
    }

    fn find(&self, launch: &Launch) -> Option<u32> {
        let id = launch.id.to_lowercase();
        if let Some(&position) = self.exact.get(&id) {
            return Some(position);
        }
        let path = launch.path()?.to_lowercase();
        let file = path.rsplit('\\').next().unwrap_or(&path);
        self.exact
            .get(&path)
            .or_else(|| self.bare.get(file))
            .copied()
    }
}

/// Puts the history of an opening together: appends to `index` what it lists
/// and no tree item stands for, and returns the rest. `apps` is
/// [`AppNames::current`]. `listed`: whether the box lists the history
/// (`<recent>`). Without it only the bonus of the tree's items is left.
pub fn listing(
    tree: &ConfigTree,
    index: &mut Index,
    launches: Vec<Launch>,
    files: Vec<RecentFile>,
    apps: &HashMap<String, String>,
    listed: bool,
    now: u64,
) -> Listing {
    struct Found {
        item: Item,
        last: u64,
        entry: Option<u32>,
    }

    let tree_apps = TreeApps::of(tree, index);
    let mut bonuses = vec![0; index.len()];
    let mut found: Vec<Found> = Vec::new();

    // Newest first, so what is seen twice keeps its latest launch.
    for launch in launches {
        let (name, entry) = match tree_apps.find(&launch) {
            Some(position) => (String::new(), Some(position)),
            None => match apps.get(&launch.id.to_lowercase()) {
                Some(name) => (name.clone(), None),
                None => continue,
            },
        };
        let seen = found
            .iter_mut()
            .find(|seen| seen.entry == entry && (entry.is_some() || seen.item.name == name));
        if let Some(seen) = seen {
            seen.item.older.push(launch);
            continue;
        }
        if let Some(position) = entry {
            bonuses[position as usize] = bonus(launch.count, launch.last, now);
        }
        found.push(Found {
            last: launch.last,
            item: Item {
                name,
                origin: Origin::App(launch),
                older: Vec::new(),
            },
            entry,
        });
    }
    if !listed {
        return Listing {
            bonuses,
            ..Listing::default()
        };
    }
    found.extend(files.into_iter().map(|file| Found {
        last: file.last,
        item: Item {
            name: file.name.clone(),
            origin: Origin::File(file),
            older: Vec::new(),
        },
        entry: None,
    }));
    found.sort_by_key(|found| std::cmp::Reverse(found.last));
    found.truncate(CAP);

    let mut listing = Listing {
        bonuses,
        ..Listing::default()
    };
    for Found { item, last, entry } in found {
        let number = listing.items.len() as u32;
        let entry = entry.unwrap_or_else(|| {
            index.push(entry_of(&item, number));
            listing.bonuses.push(0);
            index.len() as u32 - 1
        });
        listing.recent.push(Recent {
            entry,
            item: number,
            last,
        });
        listing.items.push(item);
    }
    listing
}

/// The index entry of an item no tree node stands for.
fn entry_of(item: &Item, number: u32) -> Entry {
    let kind = match item.origin {
        Origin::App(_) => Kind::App,
        Origin::File(_) => Kind::File,
    };
    let mut entry = Entry::from_history(number, kind, &item.name);
    match &item.origin {
        Origin::App(launch) => {
            let path = launch.path();
            if let Some(path) = &path {
                entry = entry.with_exe(path);
            }
            entry.facts = Facts {
                elevated: false,
                file: path.is_some(),
                store: y_core::apps_folder::packaged_id(&launch.id).is_some(),
                recorded: Some(true),
                browsable: false,
            };
        }
        Origin::File(_) => {
            entry.facts.file = true;
            entry.facts.recorded = Some(true);
        }
    }
    debug_assert!(matches!(entry.target, Target::History(_)));
    entry
}

/// The list of `shell:AppsFolder`, read on a thread of its own: it takes a few
/// hundred milliseconds, too long for the search hotkey to wait, and hardly ever
/// changes. Lower-case id → display name.
#[derive(Default)]
pub struct AppNames {
    names: HashMap<String, String>,
    read_at: Option<Instant>,
    pending: Option<Receiver<HashMap<String, String>>>,
}

impl AppNames {
    /// Starts reading, if that is not already going on.
    pub fn refresh(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (sender, receiver) = channel();
        let started = std::thread::Builder::new()
            .name("y-pop apps".to_owned())
            .spawn(move || {
                let names = y_core::apps_folder::all_apps()
                    .into_iter()
                    .map(|(id, name)| (id.to_lowercase(), name))
                    .collect();
                let _ = sender.send(names);
            });
        if started.is_ok() {
            self.pending = Some(receiver);
        }
    }

    /// The apps as last read. Only an opening with nothing read yet waits for
    /// them; one that finds the list old starts reading it for the next.
    pub fn current(&mut self) -> &HashMap<String, String> {
        if self.read_at.is_none() {
            self.refresh();
        }
        if let Some(pending) = &self.pending {
            let answer = if self.read_at.is_none() {
                pending.recv().ok()
            } else {
                pending.try_recv().ok()
            };
            if let Some(names) = answer {
                self.names = names;
                self.read_at = Some(Instant::now());
                self.pending = None;
            }
        }
        if self
            .read_at
            .is_some_and(|read_at| read_at.elapsed() > APPS_LIFETIME)
        {
            self.refresh();
        }
        &self.names
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
    use std::path::PathBuf;

    use super::*;
    use crate::config;
    use crate::search::index::Source;
    use crate::variables::Variable;

    fn tree() -> ConfigTree {
        config::load_str(
            r#"<y-pop>
                <search><group name="Admin"><app name="Regedit" exe="C:\Windows\regedit.exe"/></group></search>
                <group name="Work">
                    <app name="Outlook" exe="C:\Office\OUTLOOK.EXE"/>
                    <app name="Pad" exe="notepad.exe"/>
                    <app name="Calc" app-id="Microsoft.Calc_8wekyb3d8bbwe!App"/>
                    <quit/>
                </group>
            </y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .unwrap()
    }

    fn launch(id: &str, last: u64) -> Launch {
        Launch::new(id, 0, last)
    }

    fn file(name: &str, last: u64) -> RecentFile {
        RecentFile {
            link: PathBuf::from(format!(r"C:\Recent\{name}.lnk")),
            name: name.to_owned(),
            last,
        }
    }

    fn apps(ids: &[(&str, &str)]) -> HashMap<String, String> {
        ids.iter()
            .map(|(id, name)| (id.to_lowercase(), (*name).to_owned()))
            .collect()
    }

    /// The names of the rows, newest first.
    fn names(listing: &Listing, index: &Index) -> Vec<String> {
        listing
            .recent
            .iter()
            .map(|recent| index.entries()[recent.entry as usize].name().to_owned())
            .collect()
    }

    #[test]
    fn apps_and_files_are_merged_newest_first() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let listing = listing(
            &tree,
            &mut index,
            vec![launch("Foo.Bar!App", 300), launch("Old.App!App", 100)],
            vec![file("report.docx", 200)],
            &apps(&[("Foo.Bar!App", "Foo Bar"), ("Old.App!App", "Old App")]),
            true,
            400,
        );
        assert_eq!(
            names(&listing, &index),
            ["foo bar", "report.docx", "old app"]
        );
        assert_eq!(listing.recent[1].last, 200);
        assert_eq!(
            listing.items[listing.recent[1].item as usize].name,
            "report.docx"
        );
    }

    #[test]
    fn a_launch_outside_the_start_menu_and_the_tree_is_left_out() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let before = index.len();
        let listing = listing(
            &tree,
            &mut index,
            vec![launch(r"C:\Temp\setup.exe", 300)],
            Vec::new(),
            &HashMap::new(),
            true,
            400,
        );
        assert!(listing.recent.is_empty());
        assert_eq!(index.len(), before);
    }

    #[test]
    fn a_launch_of_a_tree_item_is_that_item() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let before = index.len();
        let listing = listing(
            &tree,
            &mut index,
            vec![
                launch(r"c:\office\outlook.exe", 500),
                launch(r"C:\Windows\System32\notepad.exe", 400),
                launch("Microsoft.Calc_8wekyb3d8bbwe!App", 300),
                launch(r"C:\Windows\regedit.exe", 200),
            ],
            Vec::new(),
            // In the Start menu as well: the tree's item still wins.
            &apps(&[(r"c:\office\outlook.exe", "Microsoft Outlook")]),
            true,
            600,
        );
        assert_eq!(
            names(&listing, &index),
            ["outlook", "pad", "calc", "regedit"]
        );
        assert_eq!(index.len(), before, "nothing new to search");
        let sources: Vec<Source> = listing
            .recent
            .iter()
            .map(|recent| index.entries()[recent.entry as usize].source)
            .collect();
        assert_eq!(sources[3], Source::SearchOnly);
    }

    #[test]
    fn the_same_app_twice_is_listed_once_at_its_latest() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let listing = listing(
            &tree,
            &mut index,
            vec![
                launch(r"C:\Office\OUTLOOK.EXE", 500),
                launch(r"C:\Office\OUTLOOK.EXE", 100),
                launch("Foo.Bar!App", 400),
                launch("Foo.Bar!App", 90),
            ],
            Vec::new(),
            &apps(&[("Foo.Bar!App", "Foo Bar")]),
            true,
            600,
        );
        assert_eq!(names(&listing, &index), ["outlook", "foo bar"]);
        assert_eq!(listing.recent[0].last, 500);
        assert!(
            listing.items.iter().all(|item| item.older.len() == 1),
            "the older records are kept, for forgetting"
        );
    }

    #[test]
    fn the_bonus_goes_to_the_tree_item_launched() {
        const DAY: u64 = 24 * 60 * 60;
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let now = 100 * DAY;
        let listing = listing(
            &tree,
            &mut index,
            vec![launch(r"C:\Office\OUTLOOK.EXE", now - 60)],
            Vec::new(),
            &HashMap::new(),
            true,
            now,
        );
        assert_eq!(listing.bonuses.len(), index.len());
        let bonus_of = |name: &str| {
            let position = index
                .entries()
                .iter()
                .position(|entry| entry.name() == name)
                .unwrap();
            listing.bonuses[position]
        };
        assert_eq!(bonus_of("outlook"), 2 + 20);
        assert_eq!(bonus_of("pad"), 0, "never launched");
    }

    #[test]
    fn without_recent_there_is_only_the_bonus() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let before = index.len();
        let listing = listing(
            &tree,
            &mut index,
            vec![
                launch(r"C:\Office\OUTLOOK.EXE", 300),
                launch("Foo.Bar!App", 200),
            ],
            vec![file("report.docx", 100)],
            &apps(&[("Foo.Bar!App", "Foo Bar")]),
            false,
            400,
        );
        assert!(listing.recent.is_empty() && listing.items.is_empty());
        assert_eq!(index.len(), before);
        assert!(listing.bonuses.iter().any(|&bonus| bonus > 0));
    }

    #[test]
    fn the_list_stops_at_the_cap_keeping_the_newest() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        let files: Vec<RecentFile> = (0..CAP as u64 + 10)
            .rev()
            .map(|n| file(&format!("file{n}"), n + 1))
            .collect();
        let listing = listing(
            &tree,
            &mut index,
            Vec::new(),
            files,
            &HashMap::new(),
            true,
            1000,
        );
        assert_eq!(listing.recent.len(), CAP);
        assert_eq!(listing.recent[0].last, CAP as u64 + 10);
        assert_eq!(
            index.len()
                - index
                    .entries()
                    .iter()
                    .filter(|e| e.node().is_some())
                    .count(),
            CAP
        );
        assert_eq!(listing.bonuses.len(), index.len());
    }

    #[test]
    fn an_app_or_a_file_says_what_the_shell_opens() {
        let app = Item {
            name: "Foo".to_owned(),
            origin: Origin::App(launch("Foo.Bar_x!App", 1)),
            older: Vec::new(),
        };
        assert_eq!(app.address(), r"shell:AppsFolder\Foo.Bar_x!App");
        assert_eq!(app.detail(), "App");
        let document = Item {
            name: "a.docx".to_owned(),
            origin: Origin::File(file("a.docx", 1)),
            older: Vec::new(),
        };
        assert_eq!(document.address(), r"C:\Recent\a.docx.lnk");
    }

    #[test]
    fn the_entries_say_what_applies_to_them() {
        let tree = tree();
        let mut index = Index::from_tree(&tree);
        listing(
            &tree,
            &mut index,
            vec![launch("Foo.Bar_x!App", 3), launch(r"C:\Tools\tool.exe", 2)],
            vec![file("report.docx", 1)],
            &apps(&[("Foo.Bar_x!App", "Foo"), (r"C:\Tools\tool.exe", "Tool")]),
            true,
            10,
        );
        let facts = |name: &str| {
            index
                .entries()
                .iter()
                .find(|entry| entry.name() == name)
                .map(|entry| (entry.kind, entry.facts.file, entry.facts.store))
                .unwrap()
        };
        assert_eq!(facts("foo"), (Kind::App, false, true));
        assert_eq!(facts("tool"), (Kind::App, true, false));
        assert_eq!(facts("report.docx"), (Kind::File, true, false));
    }

    #[test]
    fn a_file_or_an_app_of_the_history_can_be_opened_without_recording() {
        let app = entry_of(
            &Item {
                name: "Foo".to_owned(),
                origin: Origin::App(launch("Foo.Bar_x!App", 1)),
                older: Vec::new(),
            },
            0,
        );
        assert_eq!(app.facts.recorded, Some(true));
        let document = entry_of(
            &Item {
                name: "a.docx".to_owned(),
                origin: Origin::File(file("a.docx", 1)),
                older: Vec::new(),
            },
            1,
        );
        assert_eq!(document.facts.recorded, Some(true));
    }

    #[test]
    fn the_bonus_grows_with_use_and_fades_with_time_and_is_bounded() {
        const DAY: u64 = 24 * 60 * 60;
        let now = 100 * DAY;
        assert_eq!(bonus(1, now, now), 2 + 20);
        assert_eq!(bonus(0, now, now), 2 + 20, "no count still ran");
        assert_eq!(bonus(1, now - 3 * DAY, now), 2 + 10);
        assert_eq!(bonus(1, now - 30 * DAY, now), 2);
        assert!(bonus(5, now, now) > bonus(1, now, now));
        assert_eq!(bonus(u32::MAX, now, now), MAX_BONUS);
        assert_eq!(bonus(1, now + 5, now), 2 + 20, "a clock that went back");
    }
}
