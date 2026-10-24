//! The app picker's Start menu source: the shortcuts under both Start menu
//! `Programs` folders (the user's and the shared one), found and read on the
//! thread pool when the picker first opens, then listed, searched and ticked
//! on the UI thread.
//!
//! Adding goes through the same path as a drop (`MenuMsg::Dropped`), so an
//! entry becomes an item by the rules of `from_file` and gets its origin strip.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use y_core::known_folders::{KnownFolder, KnownFolders};

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
}

/// The picker's Start menu list: not asked for yet, being read, or read.
#[derive(Default)]
pub struct StartMenu {
    pub state: Load,
    pub search: String,
    /// Ticked entries, by shortcut path.
    pub checked: HashSet<String>,
    /// How many matches are listed: [`SHOWN`], plus [`SHOWN`] per "Show more".
    pub limit: usize,
}

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Load {
    #[default]
    NotStarted,
    Loading,
    Done,
}

impl StartMenu {
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
        let mut matching = all.iter().filter(|entry| {
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
