//! Turning a file the user dropped or picked into an app entry: the pure
//! half, so every rule is tested with fake resolver output. Reading the file
//! itself (the `.lnk`, the version resource) is `y_core::file_info`; the
//! caller does that, off the UI thread if the path may be slow, and hands the
//! answers over as a [`Resolved`].
//!
//! The rules are spec T5. Nothing is asked of the user: the entry is made
//! first, and [`Origin`] says how, so the inspector can offer the
//! alternatives afterwards (spec §3.3, plan 8.3).
//!
//! Paths are written the way the user would want to read them: through
//! the `tokenize` function the caller passes (`%programfiles%\…` rather than
//! `C:\Program Files\…`).

use std::path::Path;

use y_core::file_info::ShellLinkInfo;
use y_pop::settings::{AppFields, Item, ItemId, ItemKind};

/// What kind of thing was dropped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    /// `.exe` / `.com`.
    Program,
    /// A `.lnk` shortcut.
    Link,
    Folder,
    /// Anything else: a document, a script, a `.url`. Opened by the shell.
    Document,
    /// `shell:AppsFolder\<AppUserModelID>`: a packaged (Store) app.
    Store,
}

/// What reading the file found; every field is optional because any read can
/// fail, and a failed read must still give a usable entry.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Resolved {
    pub is_dir: bool,
    /// For a `.lnk`.
    pub link: Option<ShellLinkInfo>,
    /// For an executable: its `FileDescription`; for a Store app, its display name.
    pub description: Option<String>,
}

/// The choices the strip offers after a drop (spec §3.3). The default is how a
/// file is first turned into an entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Variant {
    /// A `.lnk`: use the shortcut itself rather than its target.
    pub keep_shortcut: bool,
    /// A folder: list it as a submenu rather than open it.
    pub browse: bool,
    /// Write paths with `%variables%` rather than absolute.
    pub variables: bool,
}

impl Default for Variant {
    fn default() -> Self {
        Self {
            keep_shortcut: false,
            browse: false,
            variables: true,
        }
    }
}

/// How an entry came about, kept while it stays selected (never saved).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Origin {
    pub kind: FileKind,
    /// The file as dropped, in full.
    pub path: String,
    /// Its name with the extension, for "Created from Code.lnk".
    pub file_name: String,
    /// The choices the entry was made with, and can be remade with.
    pub variant: Variant,
    /// Writing the path with `%variables%` would change something, so the
    /// choice is worth offering.
    pub tokenizable: bool,
}

const STORE_PREFIX: &str = r"shell:AppsFolder\";

/// Classifies a path before reading it, so the caller knows which reads to
/// do. A folder cannot be told from its text: pass `is_dir`.
pub fn kind_of(path: &str, is_dir: bool) -> FileKind {
    if path.len() >= STORE_PREFIX.len()
        && path[..STORE_PREFIX.len()].eq_ignore_ascii_case(STORE_PREFIX)
    {
        return FileKind::Store;
    }
    if is_dir {
        return FileKind::Folder;
    }
    match extension(path).as_str() {
        "exe" | "com" => FileKind::Program,
        "lnk" => FileKind::Link,
        _ => FileKind::Document,
    }
}

fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// The entry for `path`, and how it was made.
///
/// | dropped | entry |
/// |---|---|
/// | program | `exe` = it; name = its `FileDescription`, else the file name |
/// | `.lnk` | its target, arguments → `param`, working folder → `path`, icon location → `icon`; name = the shortcut's |
/// | `.lnk` with no file target | the `.lnk` itself, through the shell |
/// | folder | `exe` = it, opened (not listed); name = folder name |
/// | document, script, `.url` | `exe` = it, `shell="true"` |
/// | `shell:AppsFolder\…` | `app-id` |
pub fn item_from_file(
    id: ItemId,
    path: &str,
    resolved: &Resolved,
    tokenizer: &mut dyn FnMut(&str) -> String,
    variant: Variant,
) -> (Item, Origin) {
    let kind = kind_of(path, resolved.is_dir);
    let mut tokenizable = false;
    let mut tokenize = |text: &str| {
        let tokenized = tokenizer(text);
        tokenizable |= tokenized != text;
        if variant.variables {
            tokenized
        } else {
            text.to_owned()
        }
    };
    let mut item = Item::new(id, ItemKind::Group);
    let mut app = AppFields::default();

    match kind {
        FileKind::Store => {
            let app_id = &path[STORE_PREFIX.len()..];
            item.name = Some(
                resolved
                    .description
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| store_name(app_id)),
            );
            app.app_id = Some(app_id.to_owned());
            // Its own logo, not the generic icon (the user can clear it).
            item.icon = Some(y_pop::model::APP_LOGO.to_owned());
        }
        FileKind::Folder => {
            item.name = Some(file_name(path));
            app.exe = Some(tokenize(path));
            app.browse = variant.browse;
        }
        FileKind::Program => {
            item.name = Some(
                resolved
                    .description
                    .clone()
                    .filter(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| stem(path)),
            );
            app.exe = Some(tokenize(path));
        }
        FileKind::Document => {
            item.name = Some(stem(path));
            app.exe = Some(tokenize(path));
            app.shell = Some(true);
        }
        FileKind::Link => {
            item.name = Some(stem(path));
            match resolved
                .link
                .as_ref()
                .filter(|link| !link.target.is_empty() && !variant.keep_shortcut)
            {
                Some(link) => {
                    app.exe = Some(tokenize(&link.target));
                    app.param = non_empty(&link.arguments);
                    app.path = non_empty(&link.working_dir).map(|folder| tokenize(&folder));
                    item.icon = icon_location(link, &mut tokenize);
                    item.tooltip = non_empty(&link.description);
                }
                None => {
                    // Nothing a file path can name (a Store shortcut, a
                    // control-panel item): let the shell open the shortcut.
                    app.exe = Some(tokenize(path));
                    app.shell = Some(true);
                }
            }
        }
    }

    item.kind = ItemKind::App(Box::new(app));
    let origin = Origin {
        kind,
        path: path.to_owned(),
        file_name: file_name(path),
        variant,
        tokenizable,
    };
    (item, origin)
}

/// Remakes `current` (an entry made as `old`) the way `new` says, one field at a
/// time: a field the person edited since the drop (it no longer equals what `old`
/// gave) is left alone. Returns whether anything changed.
pub fn switch(current: &mut Item, old: &Item, new: &Item) -> bool {
    fn adopt<T: Clone + PartialEq>(slot: &mut T, was: &T, now: &T) -> bool {
        let take = slot == was && slot != now;
        if take {
            *slot = now.clone();
        }
        take
    }
    let mut changed = adopt(&mut current.icon, &old.icon, &new.icon);
    changed |= adopt(&mut current.tooltip, &old.tooltip, &new.tooltip);
    if let (ItemKind::App(slot), ItemKind::App(was), ItemKind::App(now)) =
        (&mut current.kind, &old.kind, &new.kind)
    {
        changed |= adopt(&mut slot.exe, &was.exe, &now.exe);
        changed |= adopt(&mut slot.param, &was.param, &now.param);
        changed |= adopt(&mut slot.path, &was.path, &now.path);
        changed |= adopt(&mut slot.shell, &was.shell, &now.shell);
        changed |= adopt(&mut slot.browse, &was.browse, &now.browse);
    }
    changed
}

/// Reads what [`item_from_file`] needs from the disk. Blocks on a slow path
/// (network, removable); a failed read leaves its field empty.
pub fn resolve(path: &str) -> Resolved {
    let is_dir = Path::new(path).is_dir();
    let mut resolved = Resolved {
        is_dir,
        ..Resolved::default()
    };
    match kind_of(path, is_dir) {
        FileKind::Link => resolved.link = y_core::file_info::read_shell_link(Path::new(path)),
        FileKind::Program => {
            resolved.description = y_core::file_info::file_description(Path::new(path))
        }
        // The display name, kept in `description` like a program's.
        FileKind::Store => {
            resolved.description =
                y_core::apps_folder::display_name_of(&path[STORE_PREFIX.len()..]);
        }
        _ => {}
    }
    resolved
}

fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// A shortcut's `IconLocation` as an `icon=` value: `file` or `file,index`.
/// Nothing when it names no file (the target's own icon then applies).
fn icon_location(link: &ShellLinkInfo, tokenize: &mut dyn FnMut(&str) -> String) -> Option<String> {
    let file = non_empty(&link.icon_file)?;
    let file = tokenize(&file);
    Some(if link.icon_index == 0 {
        file
    } else {
        format!("{file},{}", link.icon_index)
    })
}

/// A readable name from an AppUserModelID: `Microsoft.WindowsCalculator_8wekyb3d8bbwe!App`
/// → `WindowsCalculator`. Only the fallback: [`resolve`] asks the shell for the
/// real display name.
fn store_name(app_id: &str) -> String {
    let package = app_id.split(['_', '!']).next().unwrap_or(app_id);
    let name = package.rsplit('.').next().unwrap_or(package);
    if name.is_empty() {
        app_id.to_owned()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_pop::settings::Settings;

    fn tokenize(text: &str) -> String {
        match text.strip_prefix(r"C:\Program Files") {
            Some(rest) => format!(r"%programfiles%{rest}"),
            None => text.to_owned(),
        }
    }

    fn make(path: &str, resolved: &Resolved) -> (Item, Origin) {
        item_from_file(
            Settings::empty().new_id(),
            path,
            resolved,
            &mut tokenize,
            Variant::default(),
        )
    }

    fn app(item: &Item) -> &AppFields {
        match &item.kind {
            ItemKind::App(app) => app,
            other => panic!("not an app: {other:?}"),
        }
    }

    #[test]
    fn kinds_follow_the_path() {
        assert_eq!(kind_of(r"C:\a\b.EXE", false), FileKind::Program);
        assert_eq!(kind_of(r"C:\a\b.lnk", false), FileKind::Link);
        assert_eq!(kind_of(r"C:\a\b.txt", false), FileKind::Document);
        assert_eq!(kind_of(r"C:\a\b", false), FileKind::Document);
        assert_eq!(kind_of(r"C:\a\b.exe", true), FileKind::Folder);
        assert_eq!(kind_of(r"SHELL:appsfolder\X!App", false), FileKind::Store);
    }

    #[test]
    fn a_program_is_named_by_its_description_or_its_stem() {
        let described = Resolved {
            description: Some("Code editor".into()),
            ..Default::default()
        };
        let (item, origin) = make(r"C:\Program Files\Code\code.exe", &described);
        assert_eq!(item.name.as_deref(), Some("Code editor"));
        assert_eq!(
            app(&item).exe.as_deref(),
            Some(r"%programfiles%\Code\code.exe")
        );
        assert_eq!(app(&item).shell, None);
        assert_eq!(origin.kind, FileKind::Program);
        assert_eq!(origin.file_name, "code.exe");

        let (item, _) = make(r"D:\tools\thing.exe", &Resolved::default());
        assert_eq!(item.name.as_deref(), Some("thing"));
        let blank = Resolved {
            description: Some("  ".into()),
            ..Default::default()
        };
        assert_eq!(
            make(r"D:\tools\thing.exe", &blank).0.name.as_deref(),
            Some("thing")
        );
    }

    #[test]
    fn a_shortcut_brings_its_target_and_settings() {
        let resolved = Resolved {
            link: Some(ShellLinkInfo {
                target: r"C:\Program Files\Code\code.exe".into(),
                arguments: "--new-window".into(),
                working_dir: r"C:\Program Files\Code".into(),
                icon_file: r"C:\Program Files\Code\code.exe".into(),
                icon_index: 2,
                description: "Edit code".into(),
            }),
            ..Default::default()
        };
        let (item, origin) = make(r"C:\Users\me\Desktop\Code.lnk", &resolved);
        let app = app(&item);
        assert_eq!(item.name.as_deref(), Some("Code"));
        assert_eq!(app.exe.as_deref(), Some(r"%programfiles%\Code\code.exe"));
        assert_eq!(app.param.as_deref(), Some("--new-window"));
        assert_eq!(app.path.as_deref(), Some(r"%programfiles%\Code"));
        assert_eq!(
            item.icon.as_deref(),
            Some(r"%programfiles%\Code\code.exe,2")
        );
        assert_eq!(item.tooltip.as_deref(), Some("Edit code"));
        assert_eq!(origin.kind, FileKind::Link);
        assert_eq!(origin.file_name, "Code.lnk");
    }

    #[test]
    fn a_shortcut_leaves_out_what_it_does_not_say() {
        let resolved = Resolved {
            link: Some(ShellLinkInfo {
                target: r"D:\a.exe".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (item, _) = make(r"C:\x\a.lnk", &resolved);
        let app = app(&item);
        assert_eq!((app.param.as_deref(), app.path.as_deref()), (None, None));
        assert_eq!(item.icon, None);
        assert_eq!(item.tooltip, None);
    }

    #[test]
    fn a_shortcut_without_a_file_target_opens_through_the_shell() {
        let unreadable = make(r"C:\x\Store.lnk", &Resolved::default()).0;
        let no_target = make(
            r"C:\x\Store.lnk",
            &Resolved {
                link: Some(ShellLinkInfo::default()),
                ..Default::default()
            },
        )
        .0;
        for item in [unreadable, no_target] {
            assert_eq!(app(&item).exe.as_deref(), Some(r"C:\x\Store.lnk"));
            assert_eq!(app(&item).shell, Some(true));
            assert_eq!(item.name.as_deref(), Some("Store"));
        }
    }

    #[test]
    fn a_folder_is_opened_not_listed() {
        let resolved = Resolved {
            is_dir: true,
            ..Default::default()
        };
        let (item, origin) = make(r"C:\Program Files\Tools", &resolved);
        assert_eq!(item.name.as_deref(), Some("Tools"));
        assert_eq!(app(&item).exe.as_deref(), Some(r"%programfiles%\Tools"));
        assert!(!app(&item).browse);
        assert_eq!(origin.kind, FileKind::Folder);
    }

    #[test]
    fn a_document_goes_through_the_shell() {
        let (item, _) = make(r"C:\docs\Report.pdf", &Resolved::default());
        assert_eq!(item.name.as_deref(), Some("Report"));
        assert_eq!(app(&item).exe.as_deref(), Some(r"C:\docs\Report.pdf"));
        assert_eq!(app(&item).shell, Some(true));
    }

    #[test]
    fn a_store_app_takes_the_name_the_shell_gave() {
        let resolved = Resolved {
            description: Some("Calculator".into()),
            ..Resolved::default()
        };
        let (item, _) = make(
            r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            &resolved,
        );
        assert_eq!(item.name.as_deref(), Some("Calculator"));
    }

    #[test]
    fn a_store_app_becomes_an_app_id() {
        let (item, origin) = make(
            r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            &Resolved::default(),
        );
        assert_eq!(item.name.as_deref(), Some("WindowsCalculator"));
        assert_eq!(
            app(&item).app_id.as_deref(),
            Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App")
        );
        assert_eq!(app(&item).exe, None);
        assert_eq!(item.icon.as_deref(), Some("@app"), "its own logo");
        assert_eq!(origin.kind, FileKind::Store);
    }

    fn made(path: &str, resolved: &Resolved, variant: Variant) -> (Item, Origin) {
        item_from_file(
            Settings::empty().new_id(),
            path,
            resolved,
            &mut tokenize,
            variant,
        )
    }

    fn a_shortcut() -> Resolved {
        Resolved {
            link: Some(ShellLinkInfo {
                target: r"C:\Program Files\Code\code.exe".into(),
                arguments: "-n".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn the_variant_chooses_shortcut_or_target() {
        let resolved = a_shortcut();
        let (target, origin) = made(r"C:\x\Code.lnk", &resolved, Variant::default());
        assert_eq!(app(&target).param.as_deref(), Some("-n"));
        assert!(origin.tokenizable);
        let keep = Variant {
            keep_shortcut: true,
            ..Variant::default()
        };
        let (shortcut, _) = made(r"C:\x\Code.lnk", &resolved, keep);
        assert_eq!(app(&shortcut).exe.as_deref(), Some(r"C:\x\Code.lnk"));
        assert_eq!(app(&shortcut).shell, Some(true));
        assert_eq!(app(&shortcut).param, None);
    }

    #[test]
    fn the_variant_chooses_variables_and_browsing() {
        let folder = Resolved {
            is_dir: true,
            ..Default::default()
        };
        let absolute = Variant {
            variables: false,
            browse: true,
            ..Variant::default()
        };
        let (item, origin) = made(r"C:\Program Files\Tools", &folder, absolute);
        assert_eq!(app(&item).exe.as_deref(), Some(r"C:\Program Files\Tools"));
        assert!(app(&item).browse);
        assert!(origin.tokenizable, "variables are still on offer");
        let (plain, origin) = made(r"D:\Tools", &folder, Variant::default());
        assert!(!origin.tokenizable);
        assert_eq!(app(&plain).exe.as_deref(), Some(r"D:\Tools"));
    }

    #[test]
    fn switch_leaves_edited_fields_alone() {
        let resolved = a_shortcut();
        let path = r"C:\x\Code.lnk";
        let keep = Variant {
            keep_shortcut: true,
            ..Variant::default()
        };
        let (old, _) = made(path, &resolved, Variant::default());
        let (new, _) = made(path, &resolved, keep);

        let mut untouched = old.clone();
        assert!(switch(&mut untouched, &old, &new));
        assert_eq!(app(&untouched), app(&new));

        let mut edited = old.clone();
        if let ItemKind::App(app) = &mut edited.kind {
            app.param = Some("--mine".into());
        }
        assert!(switch(&mut edited, &old, &new));
        assert_eq!(app(&edited).param.as_deref(), Some("--mine"));
        assert_eq!(app(&edited).exe, app(&new).exe);
    }
}
