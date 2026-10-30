//! The `/` prefix (spec §2.7): the files inside browsed folders, the tray menus'
//! and the search-only ones.
//!
//! Nothing is listed until the first `/` of an opening, so opening search never
//! waits on a disk. Then a thread walks the folders with the same
//! [`crate::browse::list`] the tray menus use, so `filter`, `depth` and
//! `open-folder` mean the same thing here, and sends what it finds as it goes.
//! The window takes it with [`Files::take`], which appends to the [`Index`]:
//! existing entries keep their numbers, so the rows on screen stay valid.
//!
//! The walk is breadth-first over all the folders at once, so a deep folder does
//! not use up [`CAP`] before its neighbours are reached, and local folders go
//! before network ones, so a slow share holds nothing else up.
//!
//! Dropping [`Files`] (the window closes) ends the thread at its next send. A
//! thread stuck on an unresponsive share cannot be interrupted: it ends when the
//! system gives up, which is also why such a folder takes `index="true"`.
//! Only `\\…` paths count as network folders: a mapped drive is not told apart.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use y_core::icon::{Icon, IconCache};

use crate::browse;
use crate::model::{App, Browse, ConfigTree, EventMask, NodeKind};

use super::index::{Entry, Facts, Index, Kind};
use super::matcher::plain;

/// How many files and folders one opening lists at most. Search stays quick
/// and the memory stays small; past this the row of the list says so.
pub const CAP: usize = 5000;

/// A browsed folder to list.
#[derive(Clone, Debug)]
struct Folder {
    /// The item's name in the menu: searched, so `/scripts dep` narrows.
    name: String,
    dir: String,
    browse: Browse,
    /// Inherited by its files, as in a tray menu.
    shell: Option<bool>,
    elevate: bool,
    /// See [`Facts::recorded`].
    recorded: Option<bool>,
    network: bool,
}

/// One file or sub-folder found.
#[derive(Clone, Debug)]
pub struct File {
    /// As a tray menu shows it (`.exe` and `.lnk` hidden), in its own case.
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Position in [`Files::folders`].
    folder: u32,
}

/// How the listing is going, for the row at the end of the list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Progress {
    /// `/` has been typed in this opening.
    pub started: bool,
    /// The thread is still at work.
    pub listing: bool,
    /// How many browsed folders are being listed.
    pub folders: usize,
    /// Network folders left out, for want of `index="true"`.
    pub skipped: usize,
    /// How many were found so far.
    pub found: usize,
    /// [`CAP`] was reached: there may be more.
    pub capped: bool,
}

impl Progress {
    /// Whether there is anything to say about it.
    pub fn worth_a_row(&self) -> bool {
        self.started && (self.listing || self.capped || self.skipped > 0 || self.folders == 0)
    }
}

enum Found {
    Files(Vec<File>),
    Done { capped: bool },
}

/// The files of one opening.
#[derive(Default)]
pub struct Files {
    folders: Arc<[Folder]>,
    files: Vec<File>,
    progress: Progress,
    receiver: Option<Receiver<Found>>,
    /// True while a wake-up is on its way: see [`Files::start`].
    woken: Arc<AtomicBool>,
}

impl Files {
    /// What `/` would list: every browsed folder of `index`, in configuration
    /// order, each once. Lists nothing yet.
    pub fn of(tree: &ConfigTree, index: &Index) -> Self {
        let history = tree.search.as_ref().is_some_and(|search| search.history);
        let mut seen = HashSet::new();
        let mut skipped = 0;
        let mut folders = Vec::new();

        for entry in index.entries() {
            if entry.kind != Kind::Folder {
                continue;
            }
            let Some(id) = entry.node() else { continue };
            let node = tree.node(id);
            let NodeKind::App(app) = &node.kind else {
                continue;
            };
            let (Some(dir), Some(browse)) = (&app.exe, &app.browse) else {
                continue;
            };
            if !seen.insert(dir.to_lowercase()) {
                continue;
            }
            let network = is_network(dir);
            if network && !browse.index {
                skipped += 1;
                continue;
            }
            folders.push(Folder {
                name: plain(&node.name),
                dir: dir.to_string(),
                browse: browse.clone(),
                shell: app.shell,
                elevate: app.elevate,
                recorded: history.then_some(app.shell != Some(false)),
                network,
            });
        }

        Self {
            progress: Progress {
                folders: folders.len(),
                skipped,
                ..Progress::default()
            },
            folders: folders.into(),
            ..Self::default()
        }
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// Starts listing, the first time it is called. `wake` is called from the
    /// thread when there is something to [`take`](Self::take): at most once
    /// between two takes, however fast files turn up, so a walk through
    /// thousands of small folders does not flood the message queue.
    pub fn start(&mut self, wake: impl Fn() + Send + 'static) {
        if self.progress.started {
            return;
        }
        self.progress.started = true;
        if self.folders.is_empty() {
            return;
        }
        let (sender, receiver) = channel();
        let folders = Arc::clone(&self.folders);
        let woken = Arc::clone(&self.woken);
        let started = std::thread::Builder::new()
            .name("y-pop files".to_owned())
            .spawn(move || {
                let send = |found: Found| {
                    let sent = sender.send(found).is_ok();
                    if sent && !woken.swap(true, Ordering::AcqRel) {
                        wake();
                    }
                    sent
                };
                walk(&folders, send);
            });
        match started {
            Ok(_) => {
                self.receiver = Some(receiver);
                self.progress.listing = true;
            }
            Err(error) => y_core::log::line(format_args!("search: cannot list files: {error}")),
        }
    }

    /// Appends to `index` what the thread found since the last call. `true` if
    /// anything changed.
    pub fn take(&mut self, index: &mut Index) -> bool {
        self.woken.store(false, Ordering::Release);
        let Some(receiver) = &self.receiver else {
            return false;
        };
        let mut changed = false;
        let mut done = None;
        loop {
            match receiver.try_recv() {
                Ok(Found::Files(found)) => {
                    changed = true;
                    for file in found {
                        let number = self.files.len() as u32;
                        index.push(entry_of(&file, &self.folders[file.folder as usize], number));
                        self.files.push(file);
                    }
                }
                Ok(Found::Done { capped }) => done = Some(capped),
                Err(TryRecvError::Empty) => break,
                // The thread is gone without a word: it panicked.
                Err(TryRecvError::Disconnected) => {
                    done = done.or(Some(false));
                    break;
                }
            }
        }
        self.progress.found = self.files.len();
        if let Some(capped) = done {
            self.progress.listing = false;
            self.progress.capped = capped;
            self.receiver = None;
            changed = true;
        }
        changed
    }

    /// The file behind [`super::index::Target::File`].
    pub fn get(&self, number: u32) -> &File {
        &self.files[number as usize]
    }

    /// What opens file `number`: what the tray menu would run for it.
    pub fn app(&self, number: u32) -> App {
        let file = self.get(number);
        if file.is_dir {
            return App::open_folder(&file.path);
        }
        let folder = &self.folders[file.folder as usize];
        App {
            exe: Some(Arc::from(file.path.as_str())),
            app_id: None,
            param: None,
            path: None,
            shell: folder.shell,
            elevate: folder.elevate,
            events: EventMask::default(),
            browse: None,
        }
    }

    /// The second line of a row: the folder it is in.
    pub fn detail(&self, number: u32) -> String {
        let path = Path::new(&self.get(number).path);
        path.parent()
            .map_or_else(String::new, |folder| folder.display().to_string())
    }

    /// The icon of its row. None for a network folder: reading one blocks, and
    /// the window paints on the thread that reads the keyboard.
    pub fn icon(&self, number: u32, icons: &mut IconCache) -> Option<Icon> {
        let file = self.get(number);
        if file.is_dir {
            icons.for_directory()
        } else if self.folders[file.folder as usize].network {
            None
        } else {
            icons.for_file(&file.path, None)
        }
    }
}

/// Whether `dir` is on a network share. The `\\?\` and `\\.\` forms of a local
/// path look like one and are not, unless `\\?\UNC\` says so.
fn is_network(dir: &str) -> bool {
    let long_unc = dir
        .get(..8)
        .is_some_and(|start| start.eq_ignore_ascii_case(r"\\?\UNC\"));
    let unc = dir.starts_with(r"\\") || dir.starts_with("//");
    long_unc || (unc && !dir.starts_with(r"\\?\") && !dir.starts_with(r"\\.\"))
}

/// The index entry of a file.
fn entry_of(file: &File, folder: &Folder, number: u32) -> Entry {
    let kind = if file.is_dir {
        Kind::Folder
    } else {
        Kind::File
    };
    let mut entry = Entry::from_file(number, kind, &file.name)
        .with_exe(&file.path)
        .with_path(&[&folder.name]);
    entry.facts = Facts {
        elevated: false,
        file: true,
        store: false,
        recorded: folder.recorded.filter(|_| !file.is_dir),
    };
    entry
}

/// Lists `folders`, local ones first, handing what it finds to `send` a folder
/// at a time, so a slow folder does not hold back what is already found. Stops
/// when `send` says nobody is listening any more.
fn walk(folders: &[Folder], send: impl Fn(Found) -> bool) {
    let mut seen = HashSet::new();
    let mut found = 0;
    for network in [false, true] {
        let mut queue: VecDeque<(u32, String, u8)> = folders
            .iter()
            .enumerate()
            .filter(|(_, folder)| folder.network == network)
            .map(|(number, folder)| (number as u32, folder.dir.clone(), 0))
            .collect();

        while let Some((number, dir, level)) = queue.pop_front() {
            // Two browsed folders can overlap.
            if !seen.insert(dir.to_lowercase()) {
                continue;
            }
            let browse = &folders[number as usize].browse;
            let mut batch = Vec::new();
            for entry in browse::list(&dir, browse, level) {
                if found == CAP {
                    send(Found::Files(batch));
                    send(Found::Done { capped: true });
                    return;
                }
                found += 1;
                if entry.is_dir && entry.browsable {
                    queue.push_back((number, entry.path.clone(), level + 1));
                }
                batch.push(File {
                    name: entry.name,
                    path: entry.path,
                    is_dir: entry.is_dir,
                    folder: number,
                });
            }
            if !batch.is_empty() && !send(Found::Files(batch)) {
                return;
            }
        }
    }
    send(Found::Done { capped: false });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::config;
    use crate::search::index::{Source, Target};
    use crate::search::query::{Query, Scope, Searcher};
    use crate::variables::Variable;

    /// A throwaway directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("y-pop-files-test-{name}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }

        fn touch(&self, name: &str) {
            std::fs::write(self.0.join(name), b"").expect("create file");
        }

        fn mkdir(&self, name: &str) {
            std::fs::create_dir(self.0.join(name)).expect("create subdir");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(xml: &str) -> ConfigTree {
        let tree = config::load_str(xml, &mut Vec::<Variable>::new()).expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        tree
    }

    fn browsed(dir: &str, attributes: &str) -> String {
        format!(r#"<app name="Scripts" exe="{dir}" browse="true" {attributes}/>"#)
    }

    /// Everything listed, the thread having finished.
    fn listed(files: &mut Files, index: &mut Index) {
        files.start(|| {});
        let deadline = Instant::now() + Duration::from_secs(20);
        while files.progress().listing {
            assert!(Instant::now() < deadline, "the thread did not finish");
            files.take(index);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn found_names(index: &Index) -> Vec<&str> {
        let mut names: Vec<&str> = index
            .entries()
            .iter()
            .filter(|entry| entry.source == Source::BrowsedFile)
            .map(Entry::name)
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn folders_are_told_from_network_shares() {
        assert!(is_network(r"\\host\share"));
        assert!(is_network("//host/share"));
        assert!(is_network(r"\\?\UNC\host\share"));
        assert!(is_network(r"\\?\unc\host\share"));
        assert!(!is_network(r"C:\Scripts"));
        assert!(!is_network(r"\\?\C:\Scripts"));
        assert!(!is_network(r"\\.\C:\Scripts"));
        assert!(!is_network("Z:"));
    }

    #[test]
    fn a_network_folder_is_skipped_unless_it_asks_to_be_indexed() {
        let tree = tree(
            r#"<y-pop>
                <app name="Share" exe="\\host\one" browse="true"/>
                <app name="Other" exe="\\host\two" browse="true" index="true"/>
                <app name="Local" exe="C:\Scripts" browse="true"/>
            </y-pop>"#,
        );
        let index = Index::from_tree(&tree);
        let files = Files::of(&tree, &index);
        assert_eq!(files.progress().skipped, 1);
        assert_eq!(files.progress().folders, 2);
        let dirs: Vec<&str> = files.folders.iter().map(|f| f.dir.as_str()).collect();
        assert_eq!(dirs, [r"\\host\two", r"C:\Scripts"]);
    }

    #[test]
    fn a_folder_browsed_twice_and_a_search_only_one_are_all_covered() {
        let tree = tree(
            r#"<y-pop>
                <search><app name="Notes" exe="C:\Notes" browse="true"/></search>
                <group name="Work">
                    <app name="Scripts" exe="C:\Scripts" browse="true"/>
                    <app name="Again" exe="c:\scripts" browse="true"/>
                    <app name="Mail" exe="mail.exe"/>
                </group>
            </y-pop>"#,
        );
        let index = Index::from_tree(&tree);
        let files = Files::of(&tree, &index);
        let dirs: Vec<&str> = files.folders.iter().map(|f| f.dir.as_str()).collect();
        assert_eq!(dirs, [r"C:\Scripts", r"C:\Notes"]);
    }

    #[test]
    fn the_listing_follows_the_folders_rules_and_lands_in_the_index() {
        let dir = TempDir::new("rules");
        dir.touch("deploy.ps1");
        dir.touch("readme.txt");
        dir.touch("Tool.exe");
        dir.mkdir("sub");
        std::fs::write(dir.0.join("sub").join("inner.ps1"), b"").unwrap();
        std::fs::create_dir(dir.0.join("sub").join("deeper")).unwrap();
        std::fs::write(dir.0.join("sub").join("deeper").join("out.ps1"), b"").unwrap();

        let tree = tree(&format!(
            "<y-pop>{}</y-pop>",
            browsed(&dir.path(), r#"depth="1" filter="ps1,exe""#)
        ));
        let mut index = Index::from_tree(&tree);
        let before = index.len();
        let mut files = Files::of(&tree, &index);
        assert_eq!(index.len(), before, "nothing is listed in advance");

        listed(&mut files, &mut index);
        assert_eq!(
            found_names(&index),
            ["deeper", "deploy.ps1", "inner.ps1", "sub", "tool"],
            "filtered, `.exe` hidden; a folder past `depth` is found, not entered"
        );
        assert_eq!(files.progress().found, index.len() - before);
        assert!(!files.progress().capped);
    }

    #[test]
    fn a_file_is_opened_like_the_tray_menu_opens_it_and_a_folder_in_explorer() {
        let dir = TempDir::new("open");
        dir.touch("deploy.ps1");
        dir.mkdir("sub");
        let tree = tree(&format!(
            "<y-pop>{}</y-pop>",
            browsed(&dir.path(), r#"depth="1" elevate="true" shell="false""#)
        ));
        let mut index = Index::from_tree(&tree);
        let mut files = Files::of(&tree, &index);
        listed(&mut files, &mut index);

        let number = |name: &str| {
            let entry = index.entries().iter().find(|e| e.name() == name).unwrap();
            match entry.target {
                Target::File(number) => number,
                other => panic!("{other:?}"),
            }
        };
        let file = files.app(number("deploy.ps1"));
        assert_eq!(file.shell, Some(false), "inherited from the folder");
        assert!(file.elevate);
        assert_eq!(
            file.exe.as_deref(),
            Some(dir.0.join("deploy.ps1").to_str().unwrap())
        );
        assert_eq!(files.detail(number("deploy.ps1")), dir.path());

        let folder = files.app(number("sub"));
        assert_eq!(folder.shell, Some(true), "only the shell opens a folder");
        assert!(!folder.elevate);
    }

    #[test]
    fn the_listing_stops_at_the_cap_and_says_so() {
        let dir = TempDir::new("cap");
        let folders = [Folder {
            name: "Many".to_owned(),
            dir: dir.path(),
            browse: Browse {
                filter: None,
                depth: 0,
                open_folder: Default::default(),
                index: false,
            },
            shell: None,
            elevate: false,
            recorded: None,
            network: false,
        }];
        for n in 0..CAP + 3 {
            dir.touch(&format!("{n}.txt"));
        }

        let found = std::cell::RefCell::new((0, None));
        walk(&folders, |message| {
            let mut found = found.borrow_mut();
            match message {
                Found::Files(files) => found.0 += files.len(),
                Found::Done { capped } => found.1 = Some(capped),
            }
            true
        });
        assert_eq!(*found.borrow(), (CAP, Some(true)));
    }

    #[test]
    fn nobody_listening_stops_the_walk() {
        let (a, b) = (TempDir::new("stop-a"), TempDir::new("stop-b"));
        a.touch("one.txt");
        b.touch("two.txt");
        let folder = |dir: &TempDir| Folder {
            name: String::new(),
            dir: dir.path(),
            browse: Browse {
                filter: None,
                depth: 0,
                open_folder: Default::default(),
                index: false,
            },
            shell: None,
            elevate: false,
            recorded: None,
            network: false,
        };
        let sends = std::cell::Cell::new(0);
        walk(&[folder(&a), folder(&b)], |_| {
            sends.set(sends.get() + 1);
            false
        });
        assert_eq!(sends.get(), 1);
    }

    #[test]
    fn files_are_searched_only_through_their_own_scope() {
        let dir = TempDir::new("scope");
        dir.touch("deploy.ps1");
        let tree = tree(&format!("<y-pop>{}</y-pop>", browsed(&dir.path(), "")));
        let mut index = Index::from_tree(&tree);
        let mut files = Files::of(&tree, &index);
        listed(&mut files, &mut index);

        let mut searcher = Searcher::new();
        let query = Query::new("deploy");
        assert!(searcher.search(&index, &query, Scope::All).is_empty());
        assert!(searcher.search(&index, &query, Scope::Menus).is_empty());
        assert_eq!(searcher.search(&index, &query, Scope::Files).len(), 1);
        // The folder's name narrows too.
        let query = Query::new("scripts dep");
        assert_eq!(searcher.search(&index, &query, Scope::Files).len(), 1);
    }

    #[test]
    fn a_name_keeps_its_ampersand_so_the_letters_line_up() {
        let dir = TempDir::new("ampersand");
        dir.touch("R&D notes.txt");
        let tree = tree(&format!("<y-pop>{}</y-pop>", browsed(&dir.path(), "")));
        let mut index = Index::from_tree(&tree);
        let mut files = Files::of(&tree, &index);
        listed(&mut files, &mut index);
        assert_eq!(found_names(&index), ["r&d notes.txt"]);
    }

    #[test]
    fn what_is_worth_a_row() {
        let idle = Progress::default();
        assert!(!idle.worth_a_row(), "/ was not typed");
        let done = Progress {
            started: true,
            folders: 2,
            found: 10,
            ..idle
        };
        assert!(!done.worth_a_row(), "nothing to report");
        for progress in [
            Progress {
                listing: true,
                ..done
            },
            Progress {
                capped: true,
                ..done
            },
            Progress { skipped: 1, ..done },
            Progress { folders: 0, ..done },
        ] {
            assert!(progress.worth_a_row(), "{progress:?}");
        }
    }
}
