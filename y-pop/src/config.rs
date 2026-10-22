//! Reading `y-pop.xml` into a [`ConfigTree`].
//!
//! Same severity split `y-snap`'s own config reader follows: a malformed
//! document (`Document::parse` itself failing) is fatal, reported with its
//! line number; anything recognized-but-wrong — an unknown element, a
//! hotkey that doesn't parse, an attribute nobody reads, a `<group>`
//! claiming an overlay presenter style that isn't implemented yet — is
//! collected as a warning and the rest of the file still loads. Losing every
//! menu entry because one of them has a typo would be the worse failure.
//!
//! Builtin variables (`%win%`, `%mydocs%`, …) are *not* computed here: this
//! module is pure Rust, no Win32. A caller passes in anything implementing
//! [`variables::Lookup`] — a plain `Vec<Variable>` in every test below, or
//! `y-pop`'s own lazy, `SHGetKnownFolderPath`-backed table in the real
//! application (see `docs/y-pop-plan.md`'s phased roadmap) — and this
//! module never has to know or care which.
//!
//! This file is the loading pipeline (`load`/`load_from`/`load_str`) and
//! where the file itself is found ([`path`]/`argument_path`) — the actual
//! walk over one document's elements is [`reader::Reader`]'s job, in its own
//! file, and [`crate::theme`]'s (the `<skin>`/`theme=` attributes, in an
//! `impl Reader` block of their own).

use std::fmt;
use std::path::PathBuf;

use y_core::xml::{self, Document};

use crate::model::{ConfigTree, Defaults, PopupMode};
use crate::variables::Lookup;

mod reader;
pub(crate) use reader::Reader;

/// The file y-pop looks for.
pub const FILE_NAME: &str = "y-pop.xml";

#[derive(Debug)]
pub enum LoadError {
    Missing(PathBuf),
    Unreadable(PathBuf, std::io::Error),
    Malformed(PathBuf, xml::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "No configuration file at {}", path.display()),
            Self::Unreadable(path, error) => {
                write!(f, "Cannot read {}: {error}", path.display())
            }
            Self::Malformed(path, error) => write!(f, "{}, {error}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

/// Reads the configuration from wherever [`path`] finds it.
pub fn load(builtins: &mut dyn Lookup) -> Result<ConfigTree, LoadError> {
    load_from(path(), builtins)
}

pub fn load_from(path: PathBuf, builtins: &mut dyn Lookup) -> Result<ConfigTree, LoadError> {
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoadError::Missing(path));
        }
        Err(error) => return Err(LoadError::Unreadable(path, error)),
    };

    load_str(&text, builtins).map_err(|error| LoadError::Malformed(path, error))
}

/// Parses already-read text — the entry point unit tests use, since it
/// touches neither the filesystem nor Win32 (given a plain `Vec<Variable>`
/// for `builtins`, which every test does).
pub fn load_str(text: &str, builtins: &mut dyn Lookup) -> Result<ConfigTree, xml::Error> {
    let document = Document::parse(text)?;
    let root = document.root();

    let mut reader = Reader::new(builtins);

    if root.name() != "y-pop" {
        reader.warn(root, format!("expected <y-pop>, found <{}>", root.name()));
    }
    reader.check(
        root,
        &[
            "shell-execute",
            "show-shortcuts",
            "popup",
            "popup-style",
            "theme",
        ],
    );

    reader.read_variables(root);

    let defaults = Defaults {
        shell_execute: root.attr_bool("shell-execute").unwrap_or(false),
        show_shortcuts: root.attr_bool("show-shortcuts").unwrap_or(true),
        popup: reader
            .popup_mode(root, "popup", "popup-style")
            .unwrap_or(PopupMode::Native),
        theme: reader.theme(root).unwrap_or_default(),
        theme_overrides: reader.theme_overrides(root),
    };

    let roots = reader.read_children(root);
    Ok(reader.finish(roots, defaults))
}

/// Where the configuration file lives, in the order the places are tried —
/// same order and reasoning as `y-snap`'s own `path()`: portable first, so
/// an executable carried around with its own `y-pop.xml` next to it is not
/// quietly steered to a file in the roaming profile instead.
pub fn path() -> PathBuf {
    if let Some(path) = argument_path() {
        return path;
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        let portable = directory.join(FILE_NAME);
        if portable.is_file() {
            return portable;
        }
    }

    match std::env::var_os("APPDATA") {
        Some(appdata) => PathBuf::from(appdata).join("Ypo-Lite").join(FILE_NAME),
        None => PathBuf::from(FILE_NAME),
    }
}

fn argument_path() -> Option<PathBuf> {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument.eq_ignore_ascii_case("-config") || argument.eq_ignore_ascii_case("--config") {
            return Some(PathBuf::from(arguments.next()?));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeKind, OpenFolder, PresenterStyle};
    use crate::variables::Variable;

    fn builtins() -> Vec<Variable> {
        vec![Variable {
            name: "win".into(),
            value: r"C:\Windows".into(),
        }]
    }

    #[test]
    fn the_default_configuration_is_valid() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rsc/y-pop.xml"),
        )
        .expect("the default y-pop.xml ships with the crate");
        let tree = load_str(&text, &mut builtins()).expect("valid XML");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert!(!tree.roots.is_empty());
    }

    /// The default config's own "Recent documents" is the exact node a
    /// report of "browse shows no submenu" pointed at — confirms it really
    /// does parse into a `browse`-carrying `App`, on the real shipped file,
    /// not just a minimal synthetic one.
    #[test]
    fn the_default_configuration_s_browse_app_actually_has_browse_set() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rsc/y-pop.xml"),
        )
        .expect("the default y-pop.xml ships with the crate");
        let tree = load_str(&text, &mut builtins()).expect("valid XML");

        let found = tree.all_nodes().find_map(|id| {
            let node = tree.node(id);
            match &node.kind {
                NodeKind::App(app) if node.name.as_ref() == "Recent documents" => Some(app),
                _ => None,
            }
        });
        let app = found.expect("the default config has a \"Recent documents\" app node");
        let browse = app
            .browse
            .as_ref()
            .expect("browse=\"true\" should have parsed");
        assert_eq!(browse.filter.as_deref(), Some("*.*"));
        assert_eq!(browse.depth, 0);
    }

    #[test]
    fn parses_a_minimal_group() {
        let tree = load_str(
            r#"<y-pop>
                <group name="Dev">
                    <hotkey key="Win+D"/>
                    <app name="Notepad" exe="%win%\notepad.exe"/>
                    <separator/>
                    <quit/>
                </group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert_eq!(tree.roots.len(), 1);

        let group = tree.node(tree.roots[0]);
        assert!(matches!(group.kind, NodeKind::Group));
        assert_eq!(group.name.as_ref(), "Dev");
        assert_eq!(group.hotkeys.len(), 1);
        assert_eq!(group.children.len(), 3);

        let app = tree.node(group.children[0]);
        let NodeKind::App(app_data) = &app.kind else {
            panic!("expected an App node")
        };
        assert_eq!(app_data.exe.as_deref(), Some(r"C:\Windows\notepad.exe"));

        let bindings: Vec<_> = tree.hotkey_bindings().collect();
        assert_eq!(bindings.len(), 1);
        assert!(
            !bindings[0].1.can_override,
            "off unless explicitly requested"
        );
        assert_eq!(tree.hotkey_owner(bindings[0].0), Some(tree.roots[0]));
    }

    #[test]
    fn a_break_element_parses_as_its_own_node_with_no_attributes_allowed() {
        let tree = load_str(
            r#"<y-pop>
                <group name="Dev">
                    <app name="A" exe="a.exe"/>
                    <break/>
                    <app name="B" exe="b.exe"/>
                </group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);

        let group = tree.node(tree.roots[0]);
        assert_eq!(group.children.len(), 3);
        assert!(matches!(tree.node(group.children[1]).kind, NodeKind::Break));
    }

    #[test]
    fn a_break_with_an_attribute_warns() {
        let tree = load_str(
            r#"<y-pop><group name="Dev"><break foo="1"/></group></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(tree.warnings.len(), 1, "{:?}", tree.warnings);
    }

    #[test]
    fn override_is_off_by_default_and_on_when_asked() {
        let tree = load_str(
            r#"<y-pop>
                <group name="Dev">
                    <hotkey key="Win+D"/>
                    <hotkey key="Win+E" override="true"/>
                    <quit/>
                </group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let mut bindings: Vec<_> = tree.hotkey_bindings().map(|(_, binding)| binding).collect();
        bindings.sort_by_key(|binding| binding.can_override);

        assert_eq!(bindings.len(), 2);
        assert!(!bindings[0].can_override);
        assert!(bindings[1].can_override);
    }

    #[test]
    fn a_node_can_have_more_than_one_bare_hotkey_child() {
        let tree = load_str(
            r#"<y-pop><group name="Dev"><hotkey key="Win+D"/><hotkey key="Win+E"/><quit/></group></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert_eq!(tree.node(tree.roots[0]).hotkeys.len(), 2);
    }

    #[test]
    fn an_unrecognised_element_is_a_warning_not_a_failure() {
        let tree = load_str(
            r#"<y-pop><widget name="nope"/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid XML — the mistake is in the schema, not the syntax");
        assert_eq!(tree.roots.len(), 1); // <quit/> still loaded
        assert!(
            tree.warnings
                .iter()
                .any(|warning| warning.contains("unknown element <widget>"))
        );
    }

    #[test]
    fn an_app_needs_exe_or_app_id() {
        let tree =
            load_str(r#"<y-pop><app name="Broken"/></y-pop>"#, &mut builtins()).expect("valid XML");
        assert!(tree.roots.is_empty());
        assert!(
            tree.warnings
                .iter()
                .any(|warning| warning.contains("exe or an app-id"))
        );
    }

    #[test]
    fn identical_strings_are_interned_once() {
        let tree = load_str(
            r#"<y-pop>
                <app name="A" exe="%win%\tool.exe" icon="%win%\tool.exe"/>
                <app name="B" exe="%win%\tool.exe"/>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        let NodeKind::App(a) = &tree.node(tree.roots[0]).kind else {
            unreachable!()
        };
        let NodeKind::App(b) = &tree.node(tree.roots[1]).kind else {
            unreachable!()
        };
        assert!(std::sync::Arc::ptr_eq(
            a.exe.as_ref().unwrap(),
            b.exe.as_ref().unwrap()
        ));
        assert!(std::sync::Arc::ptr_eq(
            a.exe.as_ref().unwrap(),
            &tree.node(tree.roots[0]).icon.as_ref().unwrap().path
        ));
    }

    #[test]
    fn browse_children_are_not_expanded_at_parse_time() {
        let tree = load_str(
            r#"<y-pop><app name="Projects" exe="%win%\projects" browse="true" filter="*.sln"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        let node = tree.node(tree.roots[0]);
        assert!(node.children.is_empty());
        let NodeKind::App(app) = &node.kind else {
            unreachable!()
        };
        let browse = app.browse.as_ref().expect("browse requested");
        assert_eq!(browse.filter.as_deref(), Some("*.sln"));
    }

    #[test]
    fn depth_and_open_folder_are_read_off_a_browse_app() {
        let tree = load_str(
            r#"<y-pop><app name="Projects" exe="%win%\projects" browse="true" depth="3" open-folder="all"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);

        let NodeKind::App(app) = &tree.node(tree.roots[0]).kind else {
            unreachable!()
        };
        let browse = app.browse.as_ref().expect("browse requested");
        assert_eq!(browse.depth, 3);
        assert_eq!(browse.open_folder, OpenFolder::All);
    }

    /// A `depth` past [`crate::model::MAX_DEPTH`] is clamped, with a
    /// warning, rather than either failing the whole file or silently
    /// wrapping — nobody sanely nests this deep, but a typo (an extra digit)
    /// shouldn't produce a `u8` wraparound either.
    #[test]
    fn a_depth_past_the_hard_limit_is_clamped_with_a_warning() {
        let tree = load_str(
            r#"<y-pop><app name="Projects" exe="%win%\projects" browse="true" depth="9999"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(tree.warnings.len(), 1, "{:?}", tree.warnings);

        let NodeKind::App(app) = &tree.node(tree.roots[0]).kind else {
            unreachable!()
        };
        let browse = app.browse.as_ref().expect("browse requested");
        assert_eq!(browse.depth, crate::model::MAX_DEPTH);
    }

    #[test]
    fn an_unknown_open_folder_value_warns_and_falls_back_to_max_depth() {
        let tree = load_str(
            r#"<y-pop><app name="Projects" exe="%win%\projects" browse="true" open-folder="sometimes"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(tree.warnings.len(), 1, "{:?}", tree.warnings);

        let NodeKind::App(app) = &tree.node(tree.roots[0]).kind else {
            unreachable!()
        };
        let browse = app.browse.as_ref().expect("browse requested");
        assert_eq!(browse.open_folder, OpenFolder::MaxDepth);
    }

    #[test]
    fn clipboard_tokens_survive_unexpanded() {
        let tree = load_str(
            r#"<y-pop><app name="Paste" exe="%win%\notepad.exe" param="%clipboard%"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        let NodeKind::App(app) = &tree.node(tree.roots[0]).kind else {
            unreachable!()
        };
        assert_eq!(app.param.as_deref(), Some("%clipboard%"));
    }

    #[test]
    fn an_unimplemented_presenter_style_falls_back_with_a_warning() {
        let tree = load_str(
            r#"<y-pop popup="overlay" popup-style="radial"><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert_eq!(
            tree.defaults.popup,
            PopupMode::Overlay(PresenterStyle::List)
        );
        assert!(
            tree.warnings
                .iter()
                .any(|warning| warning.contains("not implemented yet"))
        );
    }

    // `theme=`/`<skin>` parsing has its own test suite alongside the code
    // that implements it — see `crate::theme`'s own `mod tests`.

    #[test]
    fn about_and_quit_default_to_a_readable_name() {
        let tree = load_str(
            r#"<y-pop><separator/><about/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        let names: Vec<&str> = tree
            .roots
            .iter()
            .map(|&id| tree.node(id).name.as_ref())
            .collect();
        assert_eq!(names, ["", "About", "Quit"]);
    }

    #[test]
    fn settings_is_a_leaf_that_defaults_to_a_readable_name() {
        let tree = load_str(
            r#"<y-pop><settings/><settings name="Prefs"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);

        let names: Vec<&str> = tree
            .roots
            .iter()
            .map(|&id| tree.node(id).name.as_ref())
            .collect();
        assert_eq!(names, ["Settings…", "Prefs"]);
        assert!(matches!(tree.node(tree.roots[0]).kind, NodeKind::Settings));
    }

    #[test]
    fn an_explicit_empty_name_is_not_defaulted() {
        let tree = load_str(r#"<y-pop><quit name=""/></y-pop>"#, &mut builtins()).expect("valid");
        assert_eq!(tree.node(tree.roots[0]).name.as_ref(), "");
    }
}
