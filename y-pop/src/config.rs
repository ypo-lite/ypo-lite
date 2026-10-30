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

    let roots = reader.read_roots(root);
    let search = reader.read_search(root);
    Ok(reader.finish(roots, defaults, search))
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
    use crate::model::{MenusBrowse, NodeKind, OpenFolder, PresenterStyle, ResultsLayout, Section};
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
    fn the_app_logo_value_is_read_as_the_reserved_icon() {
        let tree = load_str(
            r#"<y-pop><app name="Calc" app-id="Pub.Calc_x!App" icon="@app"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        let node = tree.node(tree.roots[0]);
        assert!(node.icon.as_ref().is_some_and(|icon| icon.is_app_logo()));
        assert!(tree.warnings.is_empty());
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

    const SPEC_EXAMPLE: &str = r#"<y-pop>
        <search hotkey="Alt+Space" results="sections" row-actions="true">
            <recent count="5"/>
            <menus browse="columns"/>
            <start-menu/>
            <group name="Admin tools">
                <app name="Registry Editor" exe="%win%\regedit.exe"/>
                <app name="Services" exe="services.msc"/>
            </group>
            <app name="Scripts" exe="%win%\Scripts" browse="true"/>
            <plugin name="Calculator" src="plugins\calc.wasm" prefix="="/>
        </search>
        <group name="Work"><app name="Notepad" exe="notepad.exe"/></group>
    </y-pop>"#;

    #[test]
    fn no_search_element_means_no_search() {
        let tree = load_str(r#"<y-pop><quit/></y-pop>"#, &mut builtins()).expect("valid");
        assert!(tree.search.is_none());
        assert!(tree.search_hotkey().is_none());
    }

    #[test]
    fn the_spec_example_loads_with_only_the_plugin_warning() {
        let tree = load_str(SPEC_EXAMPLE, &mut builtins()).expect("valid");
        assert_eq!(tree.warnings.len(), 1, "{:?}", tree.warnings);
        assert!(tree.warnings[0].contains("not implemented yet"));

        let search = tree.search.as_ref().expect("a <search>");
        assert!(search.hotkey.is_some());
        assert!(!search.collapsed);
        assert_eq!(search.results, ResultsLayout::Sections);
        assert!(search.row_actions);
        assert!(search.history);
        assert!(matches!(
            search.sections.as_slice(),
            [
                Section::Recent { count: 5 },
                Section::Menus { .. },
                Section::StartMenu { .. },
                Section::Group(_),
                Section::Folder(_),
                Section::Plugin(_),
            ]
        ));
        // Search-only nodes are not tray icons.
        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.node(tree.roots[0]).name.as_ref(), "Work");
    }

    #[test]
    fn search_only_nodes_are_in_the_arena_and_reached_by_all_nodes() {
        let tree = load_str(SPEC_EXAMPLE, &mut builtins()).expect("valid");
        let names: Vec<&str> = tree
            .all_nodes()
            .map(|id| tree.node(id).name.as_ref())
            .collect();
        for expected in [
            "Admin tools",
            "Registry Editor",
            "Services",
            "Scripts",
            "Notepad",
        ] {
            assert!(names.contains(&expected), "{expected} in {names:?}");
        }

        let search = tree.search.as_ref().unwrap();
        let Some(Section::Group(group)) = search
            .sections
            .iter()
            .find(|section| matches!(section, Section::Group(_)))
        else {
            panic!("a group section")
        };
        assert_eq!(tree.children(*group).count(), 2);
        let Some(Section::Folder(folder)) = search
            .sections
            .iter()
            .find(|section| matches!(section, Section::Folder(_)))
        else {
            panic!("a folder section")
        };
        let NodeKind::App(app) = &tree.node(*folder).kind else {
            panic!("an app")
        };
        assert!(app.browse.is_some());
    }

    #[test]
    fn an_empty_search_gets_the_default_sections() {
        let tree = load_str(
            r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let search = tree.search.unwrap();
        assert!(matches!(
            search.sections.as_slice(),
            [
                Section::Recent { count: 5 },
                Section::Menus {
                    browse: MenusBrowse::Columns,
                    ..
                },
                Section::StartMenu { .. }
            ]
        ));
    }

    #[test]
    fn written_sections_are_exactly_the_list_except_that_menus_is_always_there() {
        let tree = load_str(
            r#"<y-pop><search><start-menu/><recent/></search></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert!(matches!(
            tree.search.unwrap().sections.as_slice(),
            [
                Section::StartMenu { .. },
                Section::Recent { count: 5 },
                Section::Menus { .. }
            ]
        ));

        // Without <recent>, <menus> goes first.
        let tree = load_str(
            r#"<y-pop><search><start-menu/></search></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(matches!(
            tree.search.unwrap().sections.as_slice(),
            [Section::Menus { .. }, Section::StartMenu { .. }]
        ));
    }

    #[test]
    fn section_attributes_are_read() {
        let tree = load_str(
            r#"<y-pop><search collapsed="true" results="flat" row-actions="false" history="false">
                <recent count="9"/>
                <menus browse="a-z" include="y-pop; Admin tools"/>
                <start-menu exclude="*uninstall*;Help"/>
            </search>
            <group name="y-pop"><quit/></group>
            <group name="Admin tools"><quit/></group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);

        let search = tree.search.unwrap();
        assert!(search.collapsed);
        assert_eq!(search.results, ResultsLayout::Flat);
        assert!(!search.row_actions);
        assert!(!search.history);
        let [
            Section::Recent { count: 9 },
            Section::Menus { browse, include },
            Section::StartMenu { exclude },
        ] = search.sections.as_slice()
        else {
            panic!("{:?}", search.sections)
        };
        assert_eq!(*browse, MenusBrowse::AZ);
        assert_eq!(
            include.iter().map(|name| name.as_ref()).collect::<Vec<_>>(),
            ["y-pop", "Admin tools"]
        );
        assert_eq!(
            exclude.iter().map(|name| name.as_ref()).collect::<Vec<_>>(),
            ["*uninstall*", "Help"]
        );
    }

    #[test]
    fn include_is_checked_against_the_tray_menus_wherever_search_stands_in_the_file() {
        let tree = load_str(
            r#"<y-pop>
                <search><menus include="&amp;wörk;Nope"/></search>
                <group name="W&amp;ork"><quit/></group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(tree.warnings.len(), 1, "{:?}", tree.warnings);
        assert!(tree.warnings[0].contains("\"Nope\""), "{:?}", tree.warnings);

        let search = tree.search.unwrap();
        assert!(search.includes_menu("Work"), "case, accents and & aside");
        assert!(!search.includes_menu("Play"));
    }

    #[test]
    fn browse_reads_each_view() {
        for (text, browse) in [
            ("columns", MenusBrowse::Columns),
            ("a-z", MenusBrowse::AZ),
            ("list", MenusBrowse::List),
        ] {
            let xml = format!(r#"<y-pop><search><menus browse="{text}"/></search></y-pop>"#);
            let tree = load_str(&xml, &mut builtins()).expect("valid");
            assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
            assert_eq!(tree.search.unwrap().menus_browse(), browse);
        }
    }

    #[test]
    fn no_include_means_every_tray_menu() {
        let tree = load_str(r#"<y-pop><search/></y-pop>"#, &mut builtins()).expect("valid");
        assert!(tree.search.unwrap().includes_menu("Anything"));
    }

    #[test]
    fn the_search_hotkey_gets_the_id_after_every_node_hotkey() {
        let tree = load_str(
            r#"<y-pop>
                <search hotkey="Alt+Space" override="true"/>
                <group name="A"><hotkey key="Win+D"/><quit/></group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);

        let node_ids: Vec<u32> = tree.hotkey_bindings().map(|(id, _)| id).collect();
        let (id, binding) = tree.search_hotkey().expect("a search hotkey");
        assert!(binding.can_override);
        assert!(!node_ids.contains(&id));
        assert_eq!(id, y_core::ids::IDH_FIRST + node_ids.len() as u32);
        assert_eq!(tree.hotkey_owner(id), None);
    }

    #[test]
    fn search_warnings() {
        let warnings = |xml: &str| {
            load_str(xml, &mut builtins())
                .expect("valid")
                .warnings
                .join("\n")
        };

        assert!(
            warnings(r#"<y-pop><search/><search/></y-pop>"#).contains("a second <search>"),
            "second <search>"
        );
        assert!(
            warnings(r#"<y-pop><search hotkey="Nonsense+"/></y-pop>"#)
                .contains("is not a key combination")
        );
        assert!(warnings(r#"<y-pop><search results="grid"/></y-pop>"#).contains("unknown results"));
        assert!(warnings(r#"<y-pop><search typo="1"/></y-pop>"#).contains("no typo attribute"));
        assert!(
            warnings(r#"<y-pop><search><widget/></search></y-pop>"#)
                .contains("unknown element <widget>")
        );
        assert!(
            warnings(r#"<y-pop><search><recent/><recent/></search></y-pop>"#)
                .contains("a second <recent>")
        );
        assert!(
            warnings(r#"<y-pop><search><menus browse="tree"/></search></y-pop>"#)
                .contains("unknown browse")
        );
        assert!(
            warnings(r#"<y-pop><search><recent count="many"/></search></y-pop>"#)
                .contains("not a non-negative integer")
        );
        assert!(
            warnings(r#"<y-pop><search><app name="X" exe="x.exe"/></search></y-pop>"#)
                .contains("must be a browsed folder")
        );
        assert!(
            warnings(r#"<y-pop><search><plugin name="P"/></search></y-pop>"#)
                .contains("needs both a name and a src")
        );
        assert!(
            warnings(r#"<y-pop><group name="G"><search/></group></y-pop>"#)
                .contains("unknown element <search>")
        );
    }

    #[test]
    fn a_hotkey_in_a_search_only_group_is_ignored_with_a_warning() {
        let tree = load_str(
            r#"<y-pop><search>
                <group name="Tools"><hotkey key="Win+T"/><quit/></group>
            </search></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(tree.warnings.len(), 1, "{:?}", tree.warnings);
        assert!(tree.warnings[0].contains("ignored"));
        assert_eq!(tree.hotkey_bindings().count(), 0);
    }

    #[test]
    fn keywords_are_read_off_any_node_and_interned() {
        let tree = load_str(
            r#"<y-pop>
                <app name="Mail" exe="a.exe" keywords="email outlook"/>
                <group name="G" keywords="email outlook"><quit keywords="exit close"/></group>
            </y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);

        let mail = tree.node(tree.roots[0]);
        let group = tree.node(tree.roots[1]);
        assert_eq!(mail.keywords.as_deref(), Some("email outlook"));
        assert!(std::sync::Arc::ptr_eq(
            mail.keywords.as_ref().unwrap(),
            group.keywords.as_ref().unwrap()
        ));
        let quit = tree.node(group.children[0]);
        assert_eq!(quit.keywords.as_deref(), Some("exit close"));
    }

    #[test]
    fn the_runner_style_warning_points_at_search() {
        let tree = load_str(
            r#"<y-pop popup="overlay" popup-style="runner"><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(
            tree.warnings
                .iter()
                .any(|warning| warning.contains("replaced by <search>"))
        );
    }

    fn actions_of(xml: &str) -> (crate::model::ActionsConfig, Vec<String>) {
        let tree = load_str(xml, &mut builtins()).expect("valid");
        let actions = tree.search.expect("a <search>").actions;
        (actions, tree.warnings)
    }

    fn bindings(
        actions: &crate::model::ActionsConfig,
    ) -> Vec<(&'static str, Option<String>, bool)> {
        actions
            .actions
            .iter()
            .map(|a| (a.id.name(), a.key.map(|key| key.to_string()), a.button))
            .collect()
    }

    #[test]
    fn without_actions_the_defaults_apply() {
        let (actions, warnings) = actions_of(r#"<y-pop><search/></y-pop>"#);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(actions, crate::model::ActionsConfig::default());
        assert_eq!(actions.menu_key.to_string(), "Alt+Enter");
        assert!(actions.quick_pick);
        assert_eq!(
            bindings(&actions)
                .iter()
                .filter_map(|(id, key, _)| key.as_ref().map(|key| (*id, key.as_str())))
                .collect::<Vec<_>>(),
            [
                ("open", "Enter"),
                ("open-toggle-history", "Ctrl+Shift+Enter"),
                ("reveal", "Ctrl+R"),
                ("edit", "Ctrl+E"),
                ("forget", "Ctrl+Del")
            ],
            "the rest are in the menu only"
        );
    }

    #[test]
    fn actions_list_exactly_what_exists_and_in_order() {
        let (actions, warnings) = actions_of(
            r#"<y-pop><search>
                <actions menu-key="Ctrl+K" quick-pick="false">
                    <action do="edit" key="Ctrl+E" button="true"/>
                    <action do="open" key="Enter"/>
                    <action do="copy-path"/>
                </actions>
            </search></y-pop>"#,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(actions.menu_key.to_string(), "Ctrl+K");
        assert!(!actions.quick_pick);
        assert_eq!(
            bindings(&actions),
            [
                ("edit", Some("Ctrl+E".into()), true),
                ("open", Some("Enter".into()), false),
                ("copy-path", None, false),
            ]
        );
        assert_eq!(
            actions
                .key_of(crate::model::ActionId::Edit)
                .map(|k| k.to_string()),
            Some("Ctrl+E".into())
        );
        assert_eq!(
            actions.key_of(crate::model::ActionId::Reveal),
            None,
            "not listed"
        );
    }

    #[test]
    fn open_is_always_there_and_empty_actions_keep_only_it() {
        let (actions, warnings) = actions_of(
            r#"<y-pop><search><actions><action do="edit" key="Ctrl+E"/></actions></search></y-pop>"#,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            bindings(&actions),
            [
                ("open", Some("Enter".into()), false),
                ("edit", Some("Ctrl+E".into()), false)
            ]
        );

        let (empty, _) = actions_of(r#"<y-pop><search><actions/></search></y-pop>"#);
        assert_eq!(bindings(&empty), [("open", Some("Enter".into()), false)]);

        // If Enter is another action's, Open simply has no key.
        let (taken, _) = actions_of(
            r#"<y-pop><search><actions><action do="edit" key="Enter"/></actions></search></y-pop>"#,
        );
        assert_eq!(
            bindings(&taken),
            [("open", None, false), ("edit", Some("Enter".into()), false)]
        );
    }

    #[test]
    fn actions_alone_leave_the_default_sections() {
        let tree = load_str(
            r#"<y-pop><search><actions/></search></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(
            tree.search.unwrap().sections.len(),
            crate::model::SearchConfig::default_sections().len(),
            "Recent, Menus and Start menu, as for a bare <search/>"
        );
    }

    #[test]
    fn action_warnings() {
        let warnings = |xml: &str| actions_of(xml).1.join("\n");

        assert!(
            warnings(r#"<y-pop><search><actions><action do="fly"/></actions></search></y-pop>"#)
                .contains("unknown action \"fly\"")
        );
        assert!(
            warnings(
                r#"<y-pop><search><actions><action key="Ctrl+E"/></actions></search></y-pop>"#
            )
            .contains("needs a do attribute")
        );
        assert!(
            warnings(r#"<y-pop><search><actions><widget/></actions></search></y-pop>"#)
                .contains("unknown element <widget>")
        );
        assert!(
            warnings(r#"<y-pop><search><actions typo="1"/></search></y-pop>"#)
                .contains("no typo attribute")
        );
        assert!(
            warnings(r#"<y-pop><search><actions menu-key="Esc"/></search></y-pop>"#)
                .contains("menu-key=\"Esc\"")
        );
        assert!(
            warnings(
                r#"<y-pop><search><actions><action do="edit"/><action do="edit"/></actions></search></y-pop>"#
            )
            .contains("listed twice")
        );
        assert!(
            warnings(r#"<y-pop><search><actions/><actions/></search></y-pop>"#)
                .contains("a second <actions>")
        );
    }

    #[test]
    fn a_shortcut_that_cannot_work_is_dropped_but_the_action_stays() {
        let (actions, warnings) = actions_of(
            r#"<y-pop><search><actions>
                <action do="edit" key="E"/>
                <action do="copy-path" key="Ctrl+C"/>
                <action do="reveal" key="Alt+3"/>
                <action do="forget" key="Ctrl+Nonsense"/>
            </actions></search></y-pop>"#,
        );
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("or it would be typing")));
        assert!(warnings.iter().any(|w| w.contains("it edits the text")));
        assert!(warnings.iter().any(|w| w.contains("Alt+1…9")));
        let ids: Vec<_> = bindings(&actions).iter().map(|b| b.0).collect();
        assert_eq!(ids, ["open", "edit", "copy-path", "reveal", "forget"]);
        assert!(actions.actions.iter().skip(1).all(|a| a.key.is_none()));
    }

    #[test]
    fn two_actions_on_one_shortcut_the_second_loses_it() {
        let (actions, warnings) = actions_of(
            r#"<y-pop><search><actions menu-key="Ctrl+K">
                <action do="edit" key="Ctrl+E"/>
                <action do="reveal" key="Ctrl+E"/>
                <action do="forget" key="Ctrl+K"/>
            </actions></search></y-pop>"#,
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        let key = |id| actions.key_of(id).map(|k| k.to_string());
        assert_eq!(key(crate::model::ActionId::Edit), Some("Ctrl+E".into()));
        assert_eq!(key(crate::model::ActionId::Reveal), None);
        assert_eq!(key(crate::model::ActionId::Forget), None, "the menu key's");
    }

    #[test]
    fn every_action_has_a_name_that_parses_back() {
        for id in crate::model::ActionId::ALL {
            assert_eq!(crate::model::ActionId::parse(id.name()), Some(id));
            assert!(!id.label().is_empty());
        }
        assert_eq!(crate::model::ActionId::parse("nope"), None);
    }

    fn extras(xml: &str) -> crate::model::TrayExtras {
        let tree = load_str(xml, &mut builtins()).expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        tree.tray_extras(tree.roots[0])
    }

    #[test]
    fn a_right_click_adds_search_about_and_quit_to_a_plain_menu() {
        let all = extras(
            r#"<y-pop><search/><group name="Work"><app name="A" exe="a.exe"/></group></y-pop>"#,
        );
        assert_eq!(
            (all.search, all.about, all.quit),
            (true, true, true),
            "{all:?}"
        );
    }

    #[test]
    fn search_is_offered_only_when_the_file_has_one() {
        let none =
            extras(r#"<y-pop><group name="Work"><app name="A" exe="a.exe"/></group></y-pop>"#);
        assert!(!none.search);
        assert!(none.about && none.quit);
    }

    #[test]
    fn a_menu_that_holds_about_or_quit_does_not_get_another() {
        let quit =
            extras(r#"<y-pop><group name="W"><app name="A" exe="a.exe"/><quit/></group></y-pop>"#);
        assert_eq!((quit.about, quit.quit), (true, false));

        let about = extras(r#"<y-pop><group name="W"><about/></group></y-pop>"#);
        assert_eq!((about.about, about.quit), (false, true));

        // Only the top level counts: one in a submenu is not what a right
        // click would show, so the entry is still added.
        let nested = extras(
            r#"<y-pop><group name="W"><group name="y-pop"><about/><quit/></group></group></y-pop>"#,
        );
        assert_eq!((nested.about, nested.quit), (true, true));
    }

    #[test]
    fn only_the_menu_of_that_icon_counts() {
        let tree = load_str(
            r#"<y-pop><group name="One"><quit/></group><group name="Two"><app name="A" exe="a.exe"/></group></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert!(!tree.tray_extras(tree.roots[0]).quit);
        assert!(
            tree.tray_extras(tree.roots[1]).quit,
            "Two has none of its own"
        );
    }

    #[test]
    fn an_icon_that_is_an_app_or_a_quit_button_still_gets_the_rest() {
        let app = extras(r#"<y-pop><app name="A" exe="a.exe"/></y-pop>"#);
        assert!(app.about && app.quit, "an app has no menu to hold them");

        let quit = extras(r#"<y-pop><quit/></y-pop>"#);
        assert!(quit.about);
        assert!(!quit.quit, "it is one");
    }

    #[test]
    fn an_explicit_empty_name_is_not_defaulted() {
        let tree = load_str(r#"<y-pop><quit name=""/></y-pop>"#, &mut builtins()).expect("valid");
        assert_eq!(tree.node(tree.roots[0]).name.as_ref(), "");
    }
}
