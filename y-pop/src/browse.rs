//! Listing a `browse="true"` folder.
//!
//! This module only reads a directory and decides what belongs in the
//! resulting menu — *when* it runs (only once the user actually opens that
//! particular submenu, not when the containing group's popup first appears)
//! is `menu`'s doing, via `WM_INITMENUPOPUP`; see that module's doc comment.
//! PortAL's own `HandleSubDirectory`/`listFiles`/`listSubDirectories`
//! (`menu.cpp`) follow the same split and the same per-submenu laziness —
//! confirmed by reading them, not assumed.

use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;

use y_pop::model::{Browse, OpenFolder};

/// One directory entry [`list`] found: a file to launch, or a subfolder —
/// either one to browse further ([`Entry::browsable`]) or one that has
/// already reached [`Browse::depth`] and is shown flattened instead. See
/// [`list`]'s own doc for how `level` decides which.
pub struct Entry {
    /// What the menu shows — extension hidden for [`HIDDEN_EXTENSIONS`],
    /// untouched for a folder.
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Only meaningful when `is_dir`: `true` means this folder gets a real,
    /// further-browsable submenu; `false` means it has reached [`Browse::
    /// depth`] and is a plain, already-flattened item instead — see
    /// `menu.rs`'s use of this field for what each one turns into. Always
    /// `false` for a file.
    pub browsable: bool,
}

/// Extensions hidden from a listed file's display name — the file *is*
/// what it is; the extension adds nothing but clutter, the same reasoning
/// Explorer's own "hide extensions for known file types" applies by
/// default. `.lnk` doubly so: a shortcut's own name essentially never
/// carries `.lnk` in ordinary use, and PortAL's own `listFiles` already
/// special-cased it (`removeLnkExt`) — though only `.lnk`, not `.exe`, so
/// hiding `.exe` here is a deliberate improvement over PortAL, not a
/// carried-over behaviour. Extend this list rather than special-casing
/// another extension elsewhere.
const HIDDEN_EXTENSIONS: &[&str] = &["exe", "lnk"];

/// `winnt.h`'s `FILE_ATTRIBUTE_HIDDEN` — not worth a `Win32_Storage_
/// FileSystem` feature (and the crate dependency policy that implies) for
/// one flag `std::fs::Metadata` already exposes the raw bits for via
/// `MetadataExt::file_attributes`.
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

/// Lists `dir`, applying `browse`'s filter, depth and open-folder settings.
/// `level` is `dir`'s own distance from the browsed app's root (the root
/// itself, and every folder [`crate::menu`] calls this for, is one level
/// below its parent — `0` for the very first call). An unreadable directory
/// (removed, permission denied, a network path that timed out) yields an
/// empty list rather than an error — the submenu opens showing nothing
/// rather than not opening at all, the same tolerance [`crate::launch`]
/// extends a clipboard it can't read.
///
/// Folders sort before files, both alphabetically — PortAL's own ordering
/// (`HandleSubDirectory` lists subdirectories, then files). Every directory
/// entry this returns is browsable further (see [`Entry::browsable`]) unless
/// it sits exactly at `browse.depth` levels below the root — one past that
/// and it isn't returned at all, since nothing calls `list` on a folder that
/// wasn't itself returned as browsable.
pub fn list(dir: &str, browse: &Browse, level: u8) -> Vec<Entry> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries: Vec<Entry> = read_dir
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if is_hidden(&metadata) {
                return None;
            }

            let is_dir = metadata.is_dir();
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let browsable;
            if is_dir {
                if file_name == "." || file_name == ".." {
                    return None;
                }
                // `depth == 0` is the "no folders shown at all" special case
                // documented on `Browse::depth` — kept separate from the
                // general rule below rather than falling out of it, so a
                // plain `browse="true"` behaves exactly as it always has.
                if browse.depth == 0 {
                    return None;
                }
                browsable = level < browse.depth;
                if !browsable && browse.open_folder == OpenFolder::Never {
                    return None;
                }
            } else {
                if !matches(&file_name, browse.filter.as_deref()) {
                    return None;
                }
                browsable = false;
            }

            Some(Entry {
                name: display_name(&file_name, is_dir),
                path: entry.path().display().to_string(),
                is_dir,
                browsable,
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

fn is_hidden(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

/// `filter` is a comma/semicolon-separated list of extensions (`"sln"` or
/// `"*.sln"` — a leading `*`/`.` is stripped so either spelling works),
/// matched case-insensitively — the "provide more extensions" improvement
/// over PortAL's own filter, which is one `FindFirstFileW` wildcard and
/// therefore, mechanically, one pattern. `None`, an empty list, or a bare
/// `"*"`/`"*.*"` entry matches everything, PortAL's own default when no
/// `param` filter is given. A pattern with an embedded wildcard
/// (`"notes-*.txt"`) is *not* supported — this is an extension list, not a
/// glob engine; such a pattern simply never matches, which is a known,
/// narrower scope than PortAL's single-pattern-but-arbitrary-glob filter,
/// traded for supporting more than one pattern at all.
fn matches(file_name: &str, filter: Option<&str>) -> bool {
    let Some(filter) = filter else { return true };
    let extension = file_name.rsplit_once('.').map(|(_, ext)| ext);

    filter.split([',', ';']).map(str::trim).any(|pattern| {
        let pattern = pattern.trim_start_matches('*').trim_start_matches('.');
        pattern.is_empty()
            || pattern == "*"
            || extension.is_some_and(|ext| ext.eq_ignore_ascii_case(pattern))
    })
}

fn display_name(file_name: &str, is_dir: bool) -> String {
    if is_dir {
        return file_name.to_owned();
    }
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if HIDDEN_EXTENSIONS
                .iter()
                .any(|hidden| ext.eq_ignore_ascii_case(hidden)) =>
        {
            stem.to_owned()
        }
        _ => file_name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory under the system temp folder, removed on drop
    /// — real files on real disk, since `list` itself (folder sorting,
    /// hidden-file skipping, depth/open-folder) is the one piece the pure
    /// `matches`/`display_name` tests below don't touch at all.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("y-pop-browse-test-{name}"));
            let _ = std::fs::remove_dir_all(&path); // a previous run's leftovers, if any
            std::fs::create_dir(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &str {
            self.0.to_str().expect("temp dir path is valid UTF-8")
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

    fn browse(depth: u8, open_folder: OpenFolder) -> Browse {
        Browse {
            filter: None,
            depth,
            open_folder,
        }
    }

    #[test]
    fn lists_folders_before_files_both_alphabetical() {
        let dir = TempDir::new("ordering");
        dir.touch("b.txt");
        dir.touch("a.txt");
        dir.mkdir("Zeta");
        dir.mkdir("Alpha");

        let names: Vec<_> = list(dir.path(), &browse(1, OpenFolder::MaxDepth), 0)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["Alpha", "Zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn subfolders_are_skipped_when_depth_is_zero() {
        let dir = TempDir::new("depth-zero");
        dir.touch("file.txt");
        dir.mkdir("sub");

        let names: Vec<_> = list(dir.path(), &browse(0, OpenFolder::MaxDepth), 0)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["file.txt"]);
    }

    #[test]
    fn a_folder_within_depth_is_browsable() {
        let dir = TempDir::new("within-depth");
        dir.mkdir("sub");

        // level 0 -> the listed folder is level 1, at or under depth 2.
        let entries = list(dir.path(), &browse(2, OpenFolder::MaxDepth), 0);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_dir);
        assert!(
            entries[0].browsable,
            "level 1 <= depth 2, should still get a submenu"
        );
    }

    #[test]
    fn a_folder_exactly_at_max_depth_is_flattened_not_browsable() {
        let dir = TempDir::new("at-max-depth");
        dir.mkdir("sub");

        // level 0 -> the listed folder is level 1, one past depth 0.
        let entries = list(dir.path(), &browse(0, OpenFolder::MaxDepth), 0);
        // depth == 0 is the "no folders at all" special case, so this alone
        // doesn't show it — bump depth to 1 and list one level deeper to
        // actually land a folder exactly on the max-depth boundary.
        assert!(entries.is_empty());

        let entries = list(dir.path(), &browse(1, OpenFolder::MaxDepth), 1);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_dir);
        assert!(
            !entries[0].browsable,
            "level 2 > depth 1, should be flattened"
        );
    }

    #[test]
    fn open_folder_never_hides_a_folder_past_max_depth() {
        let dir = TempDir::new("open-folder-never");
        dir.mkdir("sub");

        let entries = list(dir.path(), &browse(1, OpenFolder::Never), 1);
        assert!(entries.is_empty());
    }

    #[test]
    fn open_folder_all_still_only_flattens_past_max_depth() {
        let dir = TempDir::new("open-folder-all");
        dir.mkdir("sub");

        // `open-folder="all"` adds entries inside real submenus; it doesn't
        // extend how deep those submenus go.
        let entries = list(dir.path(), &browse(1, OpenFolder::All), 1);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].browsable);
    }

    #[test]
    fn an_unreadable_directory_yields_an_empty_list_not_an_error() {
        assert!(
            list(
                r"Z:\this\does\not\exist",
                &browse(0, OpenFolder::MaxDepth),
                0
            )
            .is_empty()
        );
    }

    #[test]
    fn a_bare_extension_and_a_star_dot_extension_are_the_same_filter() {
        assert!(matches("readme.sln", Some("sln")));
        assert!(matches("readme.sln", Some("*.sln")));
        assert!(!matches("readme.txt", Some("sln")));
    }

    #[test]
    fn a_filter_accepts_more_than_one_extension() {
        let filter = Some("sln,csproj; vcxproj");
        assert!(matches("app.sln", filter));
        assert!(matches("app.csproj", filter));
        assert!(matches("app.vcxproj", filter)); // leading space after ';' trimmed
        assert!(!matches("app.txt", filter));
    }

    #[test]
    fn no_filter_or_a_star_matches_everything() {
        for filter in [None, Some("*"), Some("*.*")] {
            assert!(matches("anything.bin", filter));
            assert!(matches("no_extension", filter));
        }
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(matches("Setup.EXE", Some("exe")));
    }

    #[test]
    fn exe_and_lnk_lose_their_extension_but_nothing_else_does() {
        assert_eq!(display_name("Notepad.exe", false), "Notepad");
        assert_eq!(display_name("Shortcut.lnk", false), "Shortcut");
        assert_eq!(display_name("readme.txt", false), "readme.txt");
        assert_eq!(display_name("no_extension", false), "no_extension");
        assert_eq!(display_name("Projects.exe", true), "Projects.exe"); // a dir, never stripped
    }
}
