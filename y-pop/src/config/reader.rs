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
    ActionBinding, ActionId, ActionsConfig, App, Browse, Builder, ConfigTree, Defaults, EventMask,
    HotkeyBinding, IconSource, MAX_DEPTH, MenusBrowse, NodeData, NodeFlags, NodeId, NodeKind,
    OpenFolder, PluginSection, PopupMode, PresenterStyle, ResultsLayout, SearchConfig, Section,
};
use crate::search::keys::Chord;
use crate::variables::{self, Lookup, Pool, Variable};

use std::fmt;
use std::sync::Arc;

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
    /// Set while the children of `<search>` are read: a `<group>` there is
    /// a search-only group and takes no hotkey.
    in_search: bool,
}

impl<'b> Reader<'b> {
    pub(super) fn new(builtins: &'b mut dyn Lookup) -> Self {
        Self {
            warnings: Vec::new(),
            pool: Pool::new(),
            custom: Vec::new(),
            builtins,
            builder: Builder::new(),
            in_search: false,
        }
    }

    /// Assembles the finished tree from `roots`/`defaults`/`search`
    /// (computed by [`super::load_str`] from this reader's own methods) plus
    /// whatever this reader accumulated along the way — the one place
    /// `builder`/`warnings` are consumed, so `super::load_str` never has to
    /// reach into a `Reader`'s private fields itself.
    pub(super) fn finish(
        self,
        roots: Vec<NodeId>,
        defaults: Defaults,
        search: Option<SearchConfig>,
    ) -> ConfigTree {
        let mut tree = self.builder.finish(roots, defaults, self.warnings);
        tree.search = search;
        tree
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
                            "popup-style=\"runner\" is replaced by <search> — falling back to \"list\"",
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

    /// `<search>`, directly under the root only; a second one warns and is
    /// ignored. `None` when the file has none.
    ///
    /// No children means [`SearchConfig::default_sections`]. As soon as one
    /// is written the list is exactly what is written, except that
    /// `<menus/>` is always there: added right after `<recent>` (or first)
    /// when missing. Search-only groups and folders become nodes of the
    /// arena, outside `roots`.
    pub(super) fn read_search(&mut self, root: Element<'_>) -> Option<SearchConfig> {
        let element = root.child("search")?;
        for extra in root.children_named("search").skip(1) {
            self.warn(
                extra,
                "a second <search> is ignored — only the first one is read",
            );
        }
        self.check(
            element,
            &[
                "hotkey",
                "override",
                "collapsed",
                "results",
                "row-actions",
                "history",
            ],
        );

        let hotkey =
            element
                .attr("hotkey")
                .and_then(|text| match y_core::hotkey::Hotkey::parse(text) {
                    Some(hotkey) => Some(HotkeyBinding {
                        hotkey,
                        can_override: element.attr_bool("override").unwrap_or(false),
                    }),
                    None => {
                        self.warn(element, format!("{text:?} is not a key combination"));
                        None
                    }
                });
        let results = match element.attr("results") {
            None | Some("sections") => ResultsLayout::Sections,
            Some("flat") => ResultsLayout::Flat,
            Some(other) => {
                self.warn(element, format!("unknown results {other:?}"));
                ResultsLayout::Sections
            }
        };

        let mut sections = Vec::new();
        let (mut seen_recent, mut seen_menus, mut seen_start) = (false, false, false);
        let mut actions = None;
        self.in_search = true;
        for child in element.children() {
            let section = match child.name() {
                "recent" => {
                    if !std::mem::replace(&mut seen_recent, true) {
                        self.read_recent(child)
                    } else {
                        self.warn(child, "a second <recent> is ignored");
                        None
                    }
                }
                "menus" => {
                    if !std::mem::replace(&mut seen_menus, true) {
                        Some(self.read_menus(child))
                    } else {
                        self.warn(child, "a second <menus> is ignored");
                        None
                    }
                }
                "start-menu" => {
                    if !std::mem::replace(&mut seen_start, true) {
                        Some(self.read_start_menu(child))
                    } else {
                        self.warn(child, "a second <start-menu> is ignored");
                        None
                    }
                }
                "group" => Some(Section::Group(self.read_group(child))),
                "app" => self.read_search_folder(child),
                "plugin" => self.read_plugin(child),
                // Not a section: how results are acted on.
                "actions" => {
                    if actions.is_none() {
                        actions = Some(self.read_actions(child));
                    } else {
                        self.warn(child, "a second <actions> is ignored");
                    }
                    None
                }
                other => {
                    self.warn(child, format!("unknown element <{other}>, skipping"));
                    None
                }
            };
            sections.extend(section);
        }
        self.in_search = false;

        // `<actions>` alone leaves the sections as if nothing was written.
        if element.children().all(|child| child.name() == "actions") {
            sections = SearchConfig::default_sections();
        } else if !seen_menus {
            let at = sections
                .iter()
                .position(|section| matches!(section, Section::Recent { .. }))
                .map_or(0, |recent| recent + 1);
            sections.insert(
                at,
                Section::Menus {
                    browse: MenusBrowse::default(),
                    include: Vec::new(),
                },
            );
        }

        Some(SearchConfig {
            hotkey,
            collapsed: element.attr_bool("collapsed").unwrap_or(false),
            results,
            row_actions: element.attr_bool("row-actions").unwrap_or(true),
            history: element.attr_bool("history").unwrap_or(true),
            sections,
            actions: actions.unwrap_or_default(),
        })
    }

    /// `<actions>`: exactly the actions listed (in menu order), with `open`
    /// always among them. A shortcut that cannot work is a warning and the
    /// action keeps living without it.
    fn read_actions(&mut self, element: Element<'_>) -> ActionsConfig {
        self.check(element, &["menu-key", "quick-pick"]);
        let mut config = ActionsConfig {
            actions: Vec::new(),
            quick_pick: element.attr_bool("quick-pick").unwrap_or(true),
            ..ActionsConfig::default()
        };
        if let Some(text) = element.attr("menu-key") {
            match Chord::parse(text) {
                Ok(chord) => config.menu_key = chord,
                Err(error) => self.warn(element, format!("menu-key={text:?} {error}")),
            }
        }

        let mut taken = vec![config.menu_key];
        for child in element.children() {
            if child.name() != "action" {
                let tag = child.name();
                self.warn(child, format!("unknown element <{tag}>, skipping"));
                continue;
            }
            self.check(child, &["do", "key", "button"]);

            let Some(name) = child.attr("do") else {
                self.warn(child, "an <action> needs a do attribute");
                continue;
            };
            let Some(id) = ActionId::parse(name) else {
                self.warn(child, format!("unknown action {name:?}"));
                continue;
            };
            if config.actions.iter().any(|binding| binding.id == id) {
                self.warn(
                    child,
                    format!("{name:?} is listed twice, the first is kept"),
                );
                continue;
            }

            let key = child.attr("key").and_then(|text| match Chord::parse(text) {
                Ok(chord) if taken.contains(&chord) => {
                    self.warn(
                        child,
                        format!("key={text:?} is already another action's, ignored"),
                    );
                    None
                }
                Ok(chord) => {
                    taken.push(chord);
                    Some(chord)
                }
                Err(error) => {
                    self.warn(child, format!("key={text:?} {error}"));
                    None
                }
            });
            config.actions.push(ActionBinding {
                id,
                key,
                button: child.attr_bool("button").unwrap_or(false),
            });
        }

        // Open is always there: without it Enter would do nothing.
        if !config
            .actions
            .iter()
            .any(|binding| binding.id == ActionId::Open)
        {
            let enter = Chord::parse("Enter").expect("a valid shortcut");
            config.actions.insert(
                0,
                ActionBinding {
                    id: ActionId::Open,
                    key: (!taken.contains(&enter)).then_some(enter),
                    button: false,
                },
            );
        }
        config
    }

    fn read_recent(&mut self, element: Element<'_>) -> Option<Section> {
        self.check(element, &["count"]);
        let count = match element.attr("count") {
            None => Section::DEFAULT_RECENT,
            Some(raw) => raw.trim().parse().unwrap_or_else(|_| {
                self.warn(
                    element,
                    format!("count={raw:?} is not a non-negative integer"),
                );
                Section::DEFAULT_RECENT
            }),
        };
        Some(Section::Recent { count })
    }

    fn read_menus(&mut self, element: Element<'_>) -> Section {
        self.check(element, &["browse", "include"]);
        let browse = match element.attr("browse") {
            None | Some("columns") => MenusBrowse::Columns,
            Some("a-z") => MenusBrowse::AZ,
            Some(other) => {
                self.warn(element, format!("unknown browse {other:?}"));
                MenusBrowse::Columns
            }
        };
        Section::Menus {
            browse,
            include: self.name_list(element, "include"),
        }
    }

    fn read_start_menu(&mut self, element: Element<'_>) -> Section {
        self.check(element, &["exclude"]);
        Section::StartMenu {
            exclude: self.name_list(element, "exclude"),
        }
    }

    /// A `;`-separated list of names or patterns (not whitespace-separated:
    /// a tray menu may well be called "Admin tools"). Empty items dropped.
    fn name_list(&mut self, element: Element<'_>, attr: &str) -> Vec<Arc<str>> {
        let Some(raw) = element.attr(attr) else {
            return Vec::new();
        };
        raw.split(';')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| self.text(name))
            .collect()
    }

    /// An `<app>` directly in `<search>`: only a browsed folder makes
    /// sense there (spec §2.5.3).
    fn read_search_folder(&mut self, element: Element<'_>) -> Option<Section> {
        if !element.attr_bool("browse").unwrap_or(false) {
            self.warn(
                element,
                "an <app> directly in <search> must be a browsed folder (browse=\"true\") — put \
                 other apps in a <group>",
            );
            return None;
        }
        self.read_app(element).map(Section::Folder)
    }

    /// `<plugin>` parses and is kept, but nothing runs it yet.
    fn read_plugin(&mut self, element: Element<'_>) -> Option<Section> {
        self.check(element, &["name", "src", "prefix", "section", "allow"]);
        let (Some(name), Some(src)) = (
            self.opt_text(element, "name"),
            self.opt_text(element, "src"),
        ) else {
            self.warn(element, "a <plugin> needs both a name and a src attribute");
            return None;
        };
        self.warn(
            element,
            format!("<plugin> {name:?} is not implemented yet — kept, but not run"),
        );
        let prefix = element.attr("prefix").and_then(|raw| {
            let mut chars = raw.chars();
            match (chars.next(), chars.next()) {
                (Some(prefix), None) => Some(prefix),
                _ => {
                    self.warn(element, format!("prefix={raw:?} must be one character"));
                    None
                }
            }
        });
        Some(Section::Plugin(PluginSection { name, src, prefix }))
    }

    /// The tray menus: the children of the root. `<search>` is read apart
    /// ([`Reader::read_search`]), so it is skipped here — anywhere else it
    /// is an unknown element.
    pub(super) fn read_roots(&mut self, root: Element<'_>) -> Vec<NodeId> {
        self.read_nodes(root, true)
    }

    /// Walks every child of `element` that is a menu node (`group`, `app`,
    /// `separator`, `about`, `quit`, `settings`, `break`), skipping `<hotkey>` (already
    /// consumed by [`Reader::hotkeys`]) and `<variables>`, and warning on
    /// anything else.
    pub(super) fn read_children(&mut self, element: Element<'_>) -> Vec<NodeId> {
        self.read_nodes(element, false)
    }

    fn read_nodes(&mut self, element: Element<'_>, is_root: bool) -> Vec<NodeId> {
        let mut children = Vec::new();
        for child in element.children() {
            match child.name() {
                "hotkey" | "variables" | "skin" => {} // consumed elsewhere
                "search" if is_root => {}             // read_search

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
            keywords: None,
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
                "keywords",
                "default",
                "autoopen",
                "show-shortcuts",
                "popup",
                "popup-style",
            ],
        );

        let mut hotkeys = self.hotkeys(element);
        if self.in_search && !hotkeys.is_empty() {
            self.warn(
                element,
                "a <hotkey> in a group of <search> is ignored — it would make the group a tray \
                 menu in all but name",
            );
            hotkeys.clear();
        }
        let flags = self.common_flags(element);
        let popup = self.popup_mode(element, "popup", "popup-style");
        let name = self.opt_text(element, "name").unwrap_or_else(|| {
            self.warn(element, "<group> has no name attribute");
            self.pool.intern(String::new())
        });
        let icon = element.attr("icon").map(|raw| self.icon(raw));
        let tooltip = self.opt_text(element, "tooltip");
        let keywords = self.opt_text(element, "keywords");
        let show_shortcut = element.attr_bool("show-shortcuts");

        let id = self.builder.push(NodeData {
            kind: NodeKind::Group,
            name,
            icon,
            tooltip,
            keywords,
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
                "keywords",
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
                "index",
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
        let keywords = self.opt_text(element, "keywords");
        let show_shortcut = element.attr_bool("show-shortcuts");

        let browse = element
            .attr_bool("browse")
            .unwrap_or(false)
            .then(|| Browse {
                filter: self.opt_text(element, "filter"),
                depth: self.depth(element),
                open_folder: self.open_folder(element),
                index: element.attr_bool("index").unwrap_or(false),
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
            shell: element.attr_bool("shell"),
            elevate: element.attr_bool("elevate").unwrap_or(false),
            events,
            browse,
        };

        Some(self.builder.push(NodeData {
            kind: NodeKind::App(app),
            name,
            icon,
            tooltip,
            keywords,
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
        self.check(element, &["name", "icon", "tooltip", "keywords", "default"]);

        let hotkeys = self.hotkeys(element);
        let flags = self.common_flags(element);
        let name = self
            .opt_text(element, "name")
            .unwrap_or_else(|| self.pool.intern(default_name.to_owned()));
        let icon = element.attr("icon").map(|raw| self.icon(raw));
        let tooltip = self.opt_text(element, "tooltip");
        let keywords = self.opt_text(element, "keywords");

        self.builder.push(NodeData {
            kind,
            name,
            icon,
            tooltip,
            keywords,
            hotkeys,
            flags,
            show_shortcut: None,
            popup: None,
            children: Vec::new(),
        })
    }
}
