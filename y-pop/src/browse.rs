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

use y_pop::model::Browse;

/// One directory entry [`list`] found: a file to launch, or (only when
/// [`Browse::recurse`] asked for it) a subfolder to browse further.
pub struct Entry {
    /// What the menu shows — extension hidden for [`HIDDEN_EXTENSIONS`],
    /// untouched for a folder.
    pub name: String,
    pub path: String,
    pub is_dir: bool,
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

/// Lists `dir`, applying `browse`'s filter and recurse setting. An
/// unreadable directory (removed, permission denied, a network path that
/// timed out) yields an empty list rather than an error — the submenu opens
/// showing nothing rather than not opening at all, the same tolerance
/// [`crate::launch`] extends a clipboard it can't read.
///
/// Folders sort before files, both alphabetically — PortAL's own ordering
/// (`HandleSubDirectory` lists subdirectories, then files).
pub fn list(dir: &str, browse: &Browse) -> Vec<Entry> {
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
            if is_dir {
                if !browse.recurse || file_name == "." || file_name == ".." {
                    return None;
                }
            } else if !matches(&file_name, browse.filter.as_deref()) {
                return None;
            }

            Some(Entry {
                name: display_name(&file_name, is_dir),
                path: entry.path().display().to_string(),
                is_dir,
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
    /// hidden-file skipping, `recurse`) is the one piece the pure
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

    #[test]
    fn lists_folders_before_files_both_alphabetical() {
        let dir = TempDir::new("ordering");
        dir.touch("b.txt");
        dir.touch("a.txt");
        dir.mkdir("Zeta");
        dir.mkdir("Alpha");

        let browse = Browse {
            filter: None,
            recurse: true,
        };
        let names: Vec<_> = list(dir.path(), &browse)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["Alpha", "Zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn subfolders_are_skipped_unless_recurse_is_set() {
        let dir = TempDir::new("recurse");
        dir.touch("file.txt");
        dir.mkdir("sub");

        let no_recurse = Browse {
            filter: None,
            recurse: false,
        };
        let names: Vec<_> = list(dir.path(), &no_recurse)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, ["file.txt"]);
    }

    #[test]
    fn an_unreadable_directory_yields_an_empty_list_not_an_error() {
        let browse = Browse {
            filter: None,
            recurse: false,
        };
        assert!(list(r"Z:\this\does\not\exist", &browse).is_empty());
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
