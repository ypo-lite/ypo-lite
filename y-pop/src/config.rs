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

use std::fmt;
use std::path::PathBuf;

use y_core::xml::{self, Document, Element};

use crate::model::{
    App, Browse, Builder, ConfigTree, Defaults, EventMask, HotkeyBinding, IconSource, NodeData,
    NodeFlags, NodeId, NodeKind, PopupMode, PresenterStyle,
};
use crate::variables::{self, Lookup, Pool, Variable};

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

    let mut reader = Reader {
        warnings: Vec::new(),
        pool: Pool::new(),
        custom: Vec::new(),
        builtins,
        builder: Builder::new(),
    };

    if root.name() != "y-pop" {
        reader.warn(root, format!("expected <y-pop>, found <{}>", root.name()));
    }
    reader.check(
        root,
        &["shell-execute", "show-shortcuts", "popup", "popup-style"],
    );

    reader.read_variables(root);

    let defaults = Defaults {
        shell_execute: root.attr_bool("shell-execute").unwrap_or(false),
        show_shortcuts: root.attr_bool("show-shortcuts").unwrap_or(true),
        popup: reader
            .popup_mode(root, "popup", "popup-style")
            .unwrap_or(PopupMode::Native),
    };

    let roots = reader.read_children(root);
    let Reader {
        warnings, builder, ..
    } = reader;

    Ok(builder.finish(roots, defaults, warnings))
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

/// Walks the document, collecting warnings, interning strings and building
/// the arena as it goes.
struct Reader<'b> {
    warnings: Vec<String>,
    pool: Pool,
    /// The file's own `<var>`s, appended in document order as
    /// [`Reader::read_variables`] encounters them — see
    /// [`variables::expand`] for how this and `builtins` are checked
    /// together.
    custom: Vec<Variable>,
    builtins: &'b mut dyn Lookup,
    builder: Builder,
}

impl Reader<'_> {
    fn warn(&mut self, element: Element<'_>, message: impl fmt::Display) {
        self.warnings
            .push(format!("line {}: {message}", element.line()));
    }

    /// Reports attributes nobody reads — almost always a typo, and silently
    /// ignoring one means a setting that visibly does nothing.
    fn check(&mut self, element: Element<'_>, known: &[&str]) {
        let unknown: Vec<_> = element
            .attributes()
            .map(|(name, _)| name)
            .filter(|name| !known.contains(name))
            .collect();
        for name in unknown {
            let tag = element.name();
            self.warn(element, format!("<{tag}> has no {name} attribute"));
        }
    }

    /// Expands `%tokens%` against `custom` then `builtins` and interns the
    /// result — the point where the layered memory plan actually happens:
    /// every string field a node ends up with passes through here.
    fn text(&mut self, raw: &str) -> std::sync::Arc<str> {
        let expanded = variables::expand(raw, &self.custom, self.builtins);
        self.pool.intern(expanded)
    }

    fn opt_text(&mut self, element: Element<'_>, attr: &str) -> Option<std::sync::Arc<str>> {
        element.attr(attr).map(|raw| self.text(raw))
    }

    /// `<variables><var name="" value=""/>...</variables>`, directly under
    /// the root only — appended to `self.custom` in document order, each
    /// one's value expanded against everything defined before it (builtins,
    /// then earlier `<var>`s), *not* against itself or anything later:
    /// simpler than recursive expansion, and the schema has no need for a
    /// variable to reference itself or one defined after it.
    fn read_variables(&mut self, root: Element<'_>) {
        let Some(section) = root.child("variables") else {
            return;
        };
        for extra in root.children_named("variables").skip(1) {
            self.warn(
                extra,
                "a second <variables> is ignored — only the first one is read",
            );
        }

        for var in section.children() {
            if var.name() != "var" {
                let tag = var.name();
                self.warn(var, format!("<{tag}> is not a <var>"));
                continue;
            }
            self.check(var, &["name", "value"]);

            let (Some(name), Some(value)) = (var.attr("name"), var.attr("value")) else {
                self.warn(var, "a <var> needs both a name and a value attribute");
                continue;
            };
            let value = variables::expand(value, &self.custom, self.builtins);
            self.custom.push(Variable {
                name: name.to_owned(),
                value,
            });
        }
    }

    /// `popup`/`popup-style` (or their group-level override names, passed
    /// in) as a [`PopupMode`]. `None` if neither attribute is present, so
    /// the caller can tell "not set" from "set to native" and fall back to
    /// inheriting rather than resetting to a default.
    fn popup_mode(
        &mut self,
        element: Element<'_>,
        popup_attr: &str,
        style_attr: &str,
    ) -> Option<PopupMode> {
        let mode = element.attr(popup_attr)?;
        match mode {
            "native" => Some(PopupMode::Native),
            "overlay" => {
                let style = match element.attr(style_attr) {
                    None | Some("list") => PresenterStyle::List,
                    Some("runner") => {
                        self.warn(
                            element,
                            "popup-style=\"runner\" is not implemented yet — falling back to \"list\"",
                        );
                        PresenterStyle::List
                    }
                    Some("radial") => {
                        self.warn(
                            element,
                            "popup-style=\"radial\" is not implemented yet — falling back to \"list\"",
                        );
                        PresenterStyle::List
                    }
                    Some(other) => {
                        self.warn(element, format!("unknown popup-style {other:?}"));
                        PresenterStyle::List
                    }
                };
                Some(PopupMode::Overlay(style))
            }
            other => {
                self.warn(element, format!("unknown popup mode {other:?}"));
                None
            }
        }
    }

    /// Parses `"path"` or `"path,index"` — PortAL's `ico="...,6"` shorthand
    /// for a multi-icon `.exe`/`.dll`/`.ico`.
    fn icon(&mut self, raw: &str) -> IconSource {
        let expanded = variables::expand(raw, &self.custom, self.builtins);
        if let Some((path, index)) = expanded.rsplit_once(',')
            && let Ok(index) = index.trim().parse::<i32>()
        {
            return IconSource {
                path: self.pool.intern(path.to_owned()),
                index: Some(index),
            };
        }
        IconSource {
            path: self.pool.intern(expanded),
            index: None,
        }
    }

    /// `<hotkey key="Win+D"/>`, one or more as direct children of the node
    /// element itself — repeated child elements, not PortAL's numbered
    /// `key`/`key1`/`key2`… attributes, matching `y-snap.xml`'s own
    /// convention for repeated data. No `<hotkeys>` wrapper: the common case
    /// is exactly one binding, and a wrapper that exists only to hold one
    /// child is a level of nesting the config file gains nothing from — see
    /// [`Reader::read_children`], which already skips `<hotkey>` the same
    /// way it does `<variables>`.
    fn hotkeys(&mut self, element: Element<'_>) -> Vec<HotkeyBinding> {
        let mut hotkeys = Vec::new();
        for hotkey_element in element.children_named("hotkey") {
            self.check(hotkey_element, &["key", "override"]);

            let Some(text) = hotkey_element.attr("key") else {
                self.warn(hotkey_element, "a <hotkey> needs a key attribute");
                continue;
            };
            match y_core::hotkey::Hotkey::parse(text) {
                Some(hotkey) => hotkeys.push(HotkeyBinding {
                    hotkey,
                    can_override: hotkey_element.attr_bool("override").unwrap_or(false),
                }),
                None => self.warn(hotkey_element, format!("{text:?} is not a key combination")),
            }
        }
        hotkeys
    }

    /// The attributes and children every node kind shares.
    fn common_flags(&mut self, element: Element<'_>) -> NodeFlags {
        let mut flags = NodeFlags::default();
        flags.set_default(element.attr_bool("default").unwrap_or(false));
        flags.set_break_after(element.attr_bool("break").unwrap_or(false));
        flags.set_auto_open(element.attr_bool("autoopen").unwrap_or(false));
        flags
    }

    /// Walks every child of `element` that is a menu node (`group`, `app`,
    /// `separator`, `about`, `quit`), skipping `<hotkey>` (already consumed
    /// by [`Reader::hotkeys`]) and `<variables>`, and warning on anything
    /// else.
    fn read_children(&mut self, element: Element<'_>) -> Vec<NodeId> {
        let mut children = Vec::new();
        for child in element.children() {
            match child.name() {
                "hotkey" | "variables" => {} // consumed elsewhere
                "group" => children.push(self.read_group(child)),
                "app" => {
                    if let Some(id) = self.read_app(child) {
                        children.push(id);
                    }
                }
                "separator" => children.push(self.read_leaf(child, NodeKind::Separator, "")),
                "about" => children.push(self.read_leaf(child, NodeKind::About, "About")),
                "quit" => children.push(self.read_leaf(child, NodeKind::Quit, "Quit")),
                other => self.warn(child, format!("unknown element <{other}>, skipping")),
            }
        }
        children
    }

    fn read_group(&mut self, element: Element<'_>) -> NodeId {
        self.check(
            element,
            &[
                "name",
                "icon",
                "tooltip",
                "default",
                "break",
                "autoopen",
                "show-shortcuts",
                "popup",
                "popup-style",
            ],
        );

        let hotkeys = self.hotkeys(element);
        let flags = self.common_flags(element);
        let popup = self.popup_mode(element, "popup", "popup-style");
        let name = self.opt_text(element, "name").unwrap_or_else(|| {
            self.warn(element, "<group> has no name attribute");
            self.pool.intern(String::new())
        });
        let icon = element.attr("icon").map(|raw| self.icon(raw));
        let tooltip = self.opt_text(element, "tooltip");
        let show_shortcut = element.attr_bool("show-shortcuts");

        let id = self.builder.push(NodeData {
            kind: NodeKind::Group,
            name,
            icon,
            tooltip,
            hotkeys,
            flags,
            show_shortcut,
            popup,
            children: Vec::new(),
        });

        let children = self.read_children(element);
        self.builder.set_children(id, children);
        id
    }

    fn read_app(&mut self, element: Element<'_>) -> Option<NodeId> {
        self.check(
            element,
            &[
                "name",
                "icon",
                "tooltip",
                "default",
                "break",
                "show-shortcuts",
                "exe",
                "app-id",
                "param",
                "path",
                "shell",
                "elevate",
                "events",
                "browse",
                "filter",
                "recurse",
            ],
        );

        let exe = self.opt_text(element, "exe");
        let app_id = self.opt_text(element, "app-id");
        if exe.is_none() && app_id.is_none() {
            self.warn(element, "<app> needs either an exe or an app-id attribute");
            return None;
        }
        if exe.is_some() && app_id.is_some() {
            self.warn(
                element,
                "<app> has both exe and app-id — app-id (Store/RT app) wins",
            );
        }

        let hotkeys = self.hotkeys(element);
        let flags = self.common_flags(element);
        let name = self.opt_text(element, "name").unwrap_or_else(|| {
            self.warn(element, "<app> has no name attribute");
            self.pool.intern(String::new())
        });
        let icon = element.attr("icon").map(|raw| self.icon(raw));
        let tooltip = self.opt_text(element, "tooltip");
        let show_shortcut = element.attr_bool("show-shortcuts");

        let browse = element
            .attr_bool("browse")
            .unwrap_or(false)
            .then(|| Browse {
                filter: self.opt_text(element, "filter"),
                recurse: element.attr_bool("recurse").unwrap_or(false),
            });

        let mut events = EventMask::default();
        if let Some(text) = element.attr("events") {
            for event in text.split(',').map(str::trim) {
                match event {
                    "start" => events.insert(EventMask::START),
                    "exit" => events.insert(EventMask::EXIT),
                    "" => {}
                    other => self.warn(element, format!("unknown event {other:?}")),
                }
            }
        }

        let app = App {
            exe: if app_id.is_some() { None } else { exe },
            app_id,
            param: self.opt_text(element, "param"),
            path: self.opt_text(element, "path"),
            shell: element.attr_bool("shell").unwrap_or(false),
            elevate: element.attr_bool("elevate").unwrap_or(false),
            events,
            browse,
        };

        Some(self.builder.push(NodeData {
            kind: NodeKind::App(app),
            name,
            icon,
            tooltip,
            hotkeys,
            flags,
            show_shortcut,
            popup: None, // a leaf never opens its own submenu
            children: Vec::new(),
        }))
    }

    /// `separator`, `about`, `quit` — same shared attributes, no kind-specific
    /// ones, no children. `default_name` fills in for a missing `name`
    /// attribute — `""` for `separator` (a bare rule legitimately has no
    /// text; `menu::append_children` reads that emptiness to choose a plain
    /// rule over a titled one), `"About"`/`"Quit"` for the other two, since
    /// nothing else about those nodes is self-explanatory the way a plain
    /// rule is. An explicitly written `name=""` is left alone rather than
    /// defaulted — a written-but-empty attribute is a choice, not an
    /// omission.
    fn read_leaf(&mut self, element: Element<'_>, kind: NodeKind, default_name: &str) -> NodeId {
        self.check(element, &["name", "icon", "tooltip", "default", "break"]);

        let hotkeys = self.hotkeys(element);
        let flags = self.common_flags(element);
        let name = self
            .opt_text(element, "name")
            .unwrap_or_else(|| self.pool.intern(default_name.to_owned()));
        let icon = element.attr("icon").map(|raw| self.icon(raw));
        let tooltip = self.opt_text(element, "tooltip");

        self.builder.push(NodeData {
            kind,
            name,
            icon,
            tooltip,
            hotkeys,
            flags,
            show_shortcut: None,
            popup: None,
            children: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!browse.recurse);
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
    fn an_explicit_empty_name_is_not_defaulted() {
        let tree = load_str(r#"<y-pop><quit name=""/></y-pop>"#, &mut builtins()).expect("valid");
        assert_eq!(tree.node(tree.roots[0]).name.as_ref(), "");
    }
}
