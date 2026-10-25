//! The app picker's Start menu source: the shortcuts under both Start menu
//! `Programs` folders (the user's and the shared one), plus the packaged
//! (Store) apps of `shell:AppsFolder`, found and read on the thread pool when
//! the dialog first opens, then listed, searched and ticked on the UI thread.
//! The dialog shows one of the two at a time ([`Source`]), and can also pick a
//! single Store app for an item being edited.
//!
//! Adding goes through the same path as a drop (`MenuMsg::Dropped`), so an
//! entry becomes an item by the rules of `from_file` and gets its origin strip.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use y_core::known_folders::{KnownFolder, KnownFolders};
use y_pop::settings::ItemId;

/// How deep under `Programs` a shortcut is looked for. Vendors nest one or
/// two folders; deeper is nearly always documentation and uninstallers.
const DEPTH: usize = 4;
/// Entries shown at once; the rest is reached by searching.
pub const SHOWN: usize = 60;

/// One shortcut.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StartEntry {
    /// The shortcut's name, without `.lnk`.
    pub name: String,
    /// The `.lnk` file.
    pub path: String,
    /// What it points at; empty when it points at no file (a Store app).
    pub target: String,
    /// A packaged app from `shell:AppsFolder` rather than a shortcut: `path`
    /// is then `shell:AppsFolder\<app id>`, which a drop turns into an `app-id`.
    pub store: bool,
}

impl StartEntry {
    /// What its row's icon is loaded from: the shortcut or program itself, or
    /// a Store app's logo.
    pub fn icon_spec(&self) -> crate::app_icons::IconSpec {
        use crate::app_icons::IconSpec;
        match self
            .path
            .strip_prefix(&format!(r"{}\", y_core::apps_folder::FOLDER))
        {
            Some(app_id) if self.store => IconSpec::Store(app_id.to_owned()),
            _ => IconSpec::File {
                path: self.path.clone(),
                index: None,
            },
        }
    }
}

/// The picker's Start menu list: not asked for yet, being read, or read.
#[derive(Default)]
pub struct StartMenu {
    pub state: Load,
    /// The running programs, read again each time that list opens.
    pub running: Load,
    pub search: String,
    /// Ticked entries, by shortcut path.
    pub checked: HashSet<String>,
    /// How many matches are listed: [`SHOWN`], plus [`SHOWN`] per "Show more".
    pub limit: usize,
    /// Which list the dialog shows.
    pub source: Source,
    /// The Store app item being given an `app-id`: the dialog then picks one
    /// entry instead of ticking several to add.
    pub target: Option<ItemId>,
}

/// The lists the dialog can show, kept apart: a shortcut, a packaged app and
/// a running program are different things to add.
#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Source {
    #[default]
    Shortcuts,
    Store,
    /// The programs with a window open now.
    Running,
}

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Load {
    #[default]
    NotStarted,
    Loading,
    Done,
}

impl StartMenu {
    /// Starts the dialog on `source`, empty. With a `target` it picks one app
    /// for that item.
    pub fn open(&mut self, source: Source, target: Option<ItemId>) {
        self.source = source;
        self.target = target;
        if source == Source::Running {
            self.running = Load::NotStarted;
        }
        self.checked.clear();
        self.set_search(String::new());
    }

    /// The shown list has been read.
    pub fn ready(&self) -> bool {
        match self.source {
            Source::Running => self.running == Load::Done,
            Source::Shortcuts | Source::Store => self.state == Load::Done,
        }
    }

    /// Ticks or unticks `path`; in the pick-one dialog ticking replaces.
    pub fn toggle(&mut self, path: String) {
        if self.checked.remove(&path) {
            return;
        }
        if self.target.is_some() {
            self.checked.clear();
        }
        self.checked.insert(path);
    }

    /// Lists [`SHOWN`] more matches.
    pub fn show_more(&mut self) {
        self.limit = self.limit.max(SHOWN) + SHOWN;
    }

    /// A new search starts again from the first page.
    pub fn set_search(&mut self, text: String) {
        self.search = text;
        self.limit = SHOWN;
    }

    /// The entries to list: those matching the search (every word, anywhere
    /// in the name), up to the limit (at least [`SHOWN`]), plus how many more
    /// matched.
    pub fn listed<'a>(&self, all: &'a [StartEntry]) -> (Vec<&'a StartEntry>, usize) {
        let words: Vec<String> = self
            .search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let store = self.source == Source::Store;
        let mut matching = all.iter().filter(|entry| {
            if entry.store != store {
                return false;
            }
            let name = entry.name.to_lowercase();
            words.iter().all(|word| name.contains(word.as_str()))
        });
        let shown: Vec<&StartEntry> = matching.by_ref().take(self.limit.max(SHOWN)).collect();
        let more = matching.count();
        (shown, more)
    }

    /// The ticked entries' paths, in list order.
    pub fn checked_in_order(&self, all: &[StartEntry]) -> Vec<String> {
        all.iter()
            .filter(|entry| self.checked.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect()
    }
}

/// The two folders to scan.
pub fn folders() -> Vec<PathBuf> {
    let mut known = KnownFolders::new();
    [
        KnownFolder::StartMenuPrograms,
        KnownFolder::CommonStartMenuPrograms,
    ]
    .into_iter()
    .filter_map(|folder| known.get(folder).map(PathBuf::from))
    .collect()
}

/// Finds and reads every shortcut under `folders`. Meant for a background
/// thread: it touches the disk and COM. Sorted by name; a name present in
/// both folders is listed once (the user's wins).
pub fn scan(folders: &[PathBuf]) -> Vec<StartEntry> {
    let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let mut entries = find(folders);
    for entry in &mut entries {
        if let Some(link) = y_core::file_info::read_shell_link(Path::new(&entry.path)) {
            entry.target = link.target;
        }
    }
    if entered {
        unsafe { CoUninitialize() };
    }
    entries.extend(
        y_core::apps_folder::store_apps()
            .into_iter()
            .map(|app| StartEntry {
                name: app.name,
                path: format!(r"{}\{}", y_core::apps_folder::FOLDER, app.app_id),
                target: String::new(),
                store: true,
            }),
    );
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

/// The programs with a window open now, named by their executable's
/// description (else its file name), sorted. Meant for a background thread:
/// it reads version resources off the disk. `path` is the executable, which a
/// drop turns into an item as it does any program.
pub fn scan_running() -> Vec<StartEntry> {
    let mut entries: Vec<StartEntry> = y_core::open_windows::running_programs()
        .into_iter()
        .map(|program| {
            let name = y_core::file_info::file_description(Path::new(&program.exe))
                .or_else(|| {
                    Path::new(&program.exe)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .unwrap_or(program.title);
            StartEntry {
                name,
                path: program.exe,
                target: String::new(),
                store: false,
            }
        })
        .collect();
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

/// The shortcut files, without reading them: sorted, de-duplicated by name,
/// uninstallers left out.
fn find(folders: &[PathBuf]) -> Vec<StartEntry> {
    let mut entries = Vec::new();
    for folder in folders {
        walk(folder, 0, &mut entries);
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries.dedup_by_key(|entry| entry.name.to_lowercase());
    entries
}

fn walk(folder: &Path, depth: usize, into: &mut Vec<StartEntry>) {
    let Ok(read) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < DEPTH {
                walk(&path, depth + 1, into);
            }
            continue;
        }
        let is_link = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
        let Some(name) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        if is_link && !name.to_lowercase().contains("uninstall") {
            into.push(StartEntry {
                name,
                path: path.to_string_lossy().into_owned(),
                target: String::new(),
                store: false,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> StartEntry {
        StartEntry {
            name: name.into(),
            path: format!(r"C:\sm\{name}.lnk"),
            target: String::new(),
            store: false,
        }
    }

    #[test]
    fn finding_walks_folders_sorts_and_skips_uninstallers() {
        let root = std::env::temp_dir().join(format!("ypo-startmenu-{}", std::process::id()));
        let user = root.join("user");
        let shared = root.join("shared");
        std::fs::create_dir_all(user.join("Vendor")).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        for file in [
            user.join("Zed.lnk"),
            user.join("Vendor").join("Alpha.lnk"),
            user.join("Vendor").join("Uninstall Alpha.lnk"),
            user.join("Vendor").join("Readme.txt"),
            shared.join("zed.LNK"),
            shared.join("Beta.lnk"),
        ] {
            std::fs::write(file, b"").unwrap();
        }

        let found = find(&[user, shared]);
        let names: Vec<&str> = found.iter().map(|entry| entry.name.as_str()).collect();
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(names, ["Alpha", "Beta", "Zed"]);
    }

    #[test]
    fn searching_needs_every_word_and_counts_the_rest() {
        let all: Vec<StartEntry> = ["Visual Studio Code", "Visual Studio", "Notepad"]
            .iter()
            .map(|name| entry(name))
            .collect();
        let mut menu = StartMenu::default();
        let (shown, more) = menu.listed(&all);
        assert_eq!((shown.len(), more), (3, 0));

        menu.search = "studio  VISUAL".into();
        let (shown, _) = menu.listed(&all);
        assert_eq!(shown.len(), 2);
        menu.search = "code".into();
        assert_eq!(menu.listed(&all).0[0].name, "Visual Studio Code");
        menu.search = "zzz".into();
        assert!(menu.listed(&all).0.is_empty());
    }

    #[test]
    fn the_list_is_capped_and_says_how_many_more() {
        let all: Vec<StartEntry> = (0..SHOWN + 7).map(|n| entry(&format!("App {n}"))).collect();
        let (shown, more) = StartMenu::default().listed(&all);
        assert_eq!((shown.len(), more), (SHOWN, 7));
    }

    #[test]
    fn show_more_lists_another_page_and_a_new_search_starts_over() {
        let all: Vec<StartEntry> = (0..SHOWN * 2 + 7)
            .map(|n| entry(&format!("App {n}")))
            .collect();
        let mut menu = StartMenu::default();
        menu.show_more();
        assert_eq!(menu.listed(&all), (all.iter().take(SHOWN * 2).collect(), 7));
        menu.show_more();
        assert_eq!(menu.listed(&all).1, 0);
        menu.set_search("app".into());
        assert_eq!(menu.listed(&all).0.len(), SHOWN);
    }

    #[test]
    fn ticked_entries_come_back_in_list_order() {
        let all = [entry("A"), entry("B"), entry("C")];
        let mut menu = StartMenu::default();
        menu.checked.insert(all[2].path.clone());
        menu.checked.insert(all[0].path.clone());
        assert_eq!(
            menu.checked_in_order(&all),
            [all[0].path.clone(), all[2].path.clone()]
        );
    }
}
