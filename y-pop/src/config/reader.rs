//! [`Reader`] — walks one `<y-pop>` document, collecting warnings, interning
//! strings and building the [`crate::model::ConfigTree`] arena as it goes.
//! [`super`] (`crate::config`) owns the entry points (`load`/`load_from`/
//! `load_str`) and file-path resolution; this file owns the reader itself
//! and the tree-walking methods it drives, plus the `<skin>`/`theme=`
//! methods living instead in [`crate::theme`]'s own `impl Reader` block —
//! this type is `pub(crate)` precisely so a second file can extend it that
//! way.

use y_core::xml::Element;

use crate::model::{
    App, Browse, Builder, ConfigTree, Defaults, EventMask, HotkeyBinding, IconSource, MAX_DEPTH,
    NodeData, NodeFlags, NodeId, NodeKind, OpenFolder, PopupMode, PresenterStyle,
};
use crate::variables::{self, Lookup, Pool, Variable};

use std::fmt;

/// `pub(crate)`, with `warn`/`check`/`warnings` too: [`crate::theme`]'s own
/// `impl Reader` block (the `<skin>`/`theme=` parsing methods) needs to
/// reach them from outside this module.
pub(crate) struct Reader<'b> {
    pub(crate) warnings: Vec<String>,
    pool: Pool,
    /// The file's own `<var>`s, appended in document order as
    /// [`Reader::read_variables`] encounters them — see
    /// [`variables::expand`] for how this and `builtins` are checked
    /// together.
    custom: Vec<Variable>,
    builtins: &'b mut dyn Lookup,
    builder: Builder,
}

impl<'b> Reader<'b> {
    pub(super) fn new(builtins: &'b mut dyn Lookup) -> Self {
        Self {
            warnings: Vec::new(),
            pool: Pool::new(),
            custom: Vec::new(),
            builtins,
            builder: Builder::new(),
        }
    }

    /// Assembles the finished tree from `roots`/`defaults` (computed by
    /// [`super::load_str`] from this reader's own methods) plus whatever
    /// this reader accumulated along the way — the one place `builder`/
    /// `warnings` are consumed, so `super::load_str` never has to reach
    /// into a `Reader`'s private fields itself.
    pub(super) fn finish(self, roots: Vec<NodeId>, defaults: Defaults) -> ConfigTree {
        self.builder.finish(roots, defaults, self.warnings)
    }

    pub(crate) fn warn(&mut self, element: Element<'_>, message: impl fmt::Display) {
        self.warnings
            .push(format!("line {}: {message}", element.line()));
    }

    /// Reports attributes nobody reads — almost always a typo, and silently
    /// ignoring one means a setting that visibly does nothing.
    pub(crate) fn check(&mut self, element: Element<'_>, known: &[&str]) {
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
    pub(super) fn read_variables(&mut self, root: Element<'_>) {
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
    pub(super) fn popup_mode(
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

    /// `depth="N"` — see [`crate::model::Browse::depth`]. Absent, or a value
    /// that doesn't parse as a non-negative integer, is `0` — the same "no
    /// folders shown" default a plain `browse="true"` had before this
    /// attribute existed; a bad value warns rather than failing the whole
    /// file, the same severity `check` itself uses for an unknown attribute.
    /// Parsed as `u32` first, not straight into `Browse::depth`'s own `u8`,
    /// so a value past [`MAX_DEPTH`] (or past `u8::MAX`) is a clamp-with-
    /// warning rather than a silent parse failure that reads as "0".
    fn depth(&mut self, element: Element<'_>) -> u8 {
        let depth = match element.attr("depth") {
            None => return 0,
            Some(raw) => match raw.trim().parse::<u32>() {
                Ok(depth) => depth,
                Err(_) => {
                    self.warn(
                        element,
                        format!("depth={raw:?} is not a non-negative integer"),
                    );
                    return 0;
                }
            },
        };
        if depth > u32::from(MAX_DEPTH) {
            self.warn(
                element,
                format!(
                    "depth={depth} is more than the {MAX_DEPTH} levels y-pop supports — clamped"
                ),
            );
            return MAX_DEPTH;
        }
        // Always fits: checked against MAX_DEPTH (< u8::MAX) above.
        u8::try_from(depth).unwrap_or(MAX_DEPTH)
    }

    /// `open-folder="all"|"max-depth"|"never"` — see [`OpenFolder`].
    /// Defaults to, and falls back on an unknown value to, [`OpenFolder::
    /// MaxDepth`], the least surprising of the three: no extra entries
    /// anywhere, but a folder that would otherwise be unreachable (having
    /// hit [`crate::model::Browse::depth`]) still shows up, openable.
    fn open_folder(&mut self, element: Element<'_>) -> OpenFolder {
        match element.attr("open-folder") {
            None | Some("max-depth") => OpenFolder::MaxDepth,
            Some("all") => OpenFolder::All,
            Some("never") => OpenFolder::Never,
            Some(other) => {
                self.warn(element, format!("unknown open-folder {other:?}"));
                OpenFolder::MaxDepth
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
        flags.set_auto_open(element.attr_bool("autoopen").unwrap_or(false));
        flags
    }

    /// Walks every child of `element` that is a menu node (`group`, `app`,
    /// `separator`, `about`, `quit`, `settings`, `break`), skipping `<hotkey>` (already
    /// consumed by [`Reader::hotkeys`]) and `<variables>`, and warning on
    /// anything else.
    pub(super) fn read_children(&mut self, element: Element<'_>) -> Vec<NodeId> {
        let mut children = Vec::new();
        for child in element.children() {
            match child.name() {
                "hotkey" | "variables" | "skin" => {} // consumed elsewhere
                "group" => children.push(self.read_group(child)),
                "app" => {
                    if let Some(id) = self.read_app(child) {
                        children.push(id);
                    }
                }
                "separator" => children.push(self.read_leaf(child, NodeKind::Separator, "")),
                "about" => children.push(self.read_leaf(child, NodeKind::About, "About")),
                "quit" => children.push(self.read_leaf(child, NodeKind::Quit, "Quit")),
                "settings" => children.push(self.read_leaf(child, NodeKind::Settings, "Settings…")),
                "break" => children.push(self.read_break(child)),
                other => self.warn(child, format!("unknown element <{other}>, skipping")),
            }
        }
        children
    }

    /// `<break/>` — a menu-column break at this exact point among its
    /// siblings, see [`NodeKind::Break`]. Takes no attributes at all: unlike
    /// `<separator/>` it has no name/icon/tooltip to show (there's nothing
    /// to draw), and unlike every other node kind it has nothing a hotkey
    /// could meaningfully target either.
    fn read_break(&mut self, element: Element<'_>) -> NodeId {
        self.check(element, &[]);
        self.builder.push(NodeData {
            kind: NodeKind::Break,
            name: self.pool.intern(String::new()),
            icon: None,
            tooltip: None,
            hotkeys: Vec::new(),
            flags: NodeFlags::default(),
            show_shortcut: None,
            popup: None,
            children: Vec::new(),
        })
    }

    fn read_group(&mut self, element: Element<'_>) -> NodeId {
        self.check(
            element,
            &[
                "name",
                "icon",
                "tooltip",
                "default",
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
                "depth",
                "open-folder",
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
                depth: self.depth(element),
                open_folder: self.open_folder(element),
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

    /// `separator`, `about`, `quit`, `settings` — same shared attributes, no kind-specific
    /// ones, no children. `default_name` fills in for a missing `name`
    /// attribute — `""` for `separator` (a bare rule legitimately has no
    /// text; `menu::append_children` reads that emptiness to choose a plain
    /// rule over a titled one), `"About"`/`"Quit"` for the other two, since
    /// nothing else about those nodes is self-explanatory the way a plain
    /// rule is. An explicitly written `name=""` is left alone rather than
    /// defaulted — a written-but-empty attribute is a choice, not an
    /// omission.
    fn read_leaf(&mut self, element: Element<'_>, kind: NodeKind, default_name: &str) -> NodeId {
        self.check(element, &["name", "icon", "tooltip", "default"]);

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
