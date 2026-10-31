//! Reading `y-pop.xml` into [`Settings`].
//!
//! Two passes over the same text. The first is the real reader
//! ([`crate::config::load_str`]), used only for its warnings and for its
//! verdict on malformed XML — so the editor never disagrees with y-pop about
//! what is wrong. The second walks the document with comments
//! (`y_core::xml`'s `children_with_comments`) and copies it into the owned,
//! raw model.
//!
//! Every attribute is *taken* from a list as it is understood; whatever is
//! left (unknown names, and known ones whose value didn't parse) becomes the
//! element's `extra_attrs`, which is how a save keeps what the editor has no
//! field for.

use std::path::PathBuf;

use y_core::overlay::Color;
use y_core::xml::{self, Comment as XmlComment, Document, Element, Node, Writer};

use super::{
    ActionEntry, ActionItem, AppFields, Comment, Entry, Globals, HotkeyEntry, Item, ItemKind,
    MenusMode, MonitorMode, PopupStyle, Renderer, ResultsMode, SearchEntry, SearchSection,
    SearchSettings, SectionKind, Settings, Skin, Var, VarEntry,
};
use crate::config::{self, LoadError};
use crate::model::OpenFolder;
use crate::theme::{
    BorderOverride, EdgeRule, GradientDirection, GradientOverride, SeparatorSpan, Theme,
    ThemeOverrides,
};
use crate::variables::Lookup;

impl Settings {
    /// Reads the file at `path`. `builtins` only serves the warnings (the
    /// reader expands variables to check them); nothing it returns is
    /// expanded.
    pub fn load_from(path: PathBuf, builtins: &mut dyn Lookup) -> Result<Self, LoadError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(LoadError::Missing(path));
            }
            Err(error) => return Err(LoadError::Unreadable(path, error)),
        };
        Self::load_str(&text, builtins).map_err(|error| LoadError::Malformed(path, error))
    }

    /// Reads already-read text. Fails only on malformed XML, with a line
    /// number; anything else is a warning on the result.
    pub fn load_str(text: &str, builtins: &mut dyn Lookup) -> Result<Self, xml::Error> {
        let tree = config::load_str(text, builtins)?;
        let document = Document::parse(text)?;

        let mut loader = Loader {
            next_id: 1,
            warnings: tree.warnings,
        };
        let mut settings = loader.settings(&document);
        settings.next_id = loader.next_id;
        settings.warnings = loader.warnings;
        Ok(settings)
    }
}

struct Loader {
    next_id: u32,
    warnings: Vec<String>,
}

/// What a container's children turned into.
#[derive(Default)]
struct Body {
    hotkeys: Vec<HotkeyEntry>,
    inner: Vec<Comment>,
    entries: Vec<Entry>,
    intro: Vec<Comment>,
    variables: Option<Vec<VarEntry>>,
    skin: Option<Skin>,
    search: Option<SearchSettings>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Root,
    Group,
    /// An element with no menu children: only hotkeys and comments.
    Leaf,
}

impl Loader {
    fn settings(&mut self, document: &Document) -> Settings {
        let root = document.root();
        let mut settings = Settings::empty();

        settings.prolog = comments(document.prolog_comments());
        settings.epilog = comments(document.epilog_comments());

        let mut attrs = Attrs::of(root);
        settings.globals = Globals {
            shell_execute: attrs.bool("shell-execute"),
            show_shortcuts: attrs.bool("show-shortcuts"),
            popup: attrs.parse("popup", Renderer::parse),
            popup_style: attrs.parse("popup-style", PopupStyle::parse),
            theme: attrs.parse("theme", parse_theme),
            extra_attrs: Vec::new(),
        };
        settings.globals.extra_attrs = attrs.rest();

        let body = self.body(root, Container::Root);
        settings.intro = body.intro;
        settings.variables = body.variables.unwrap_or_default();
        settings.skin = body.skin.unwrap_or_default();
        settings.search = body.search;
        settings.roots = body.entries;
        // A `<hotkey>` directly under `<y-pop>` is read by nobody: it came
        // back as a disabled item among `body.inner`, which belongs with
        // the roots.
        settings
            .roots
            .extend(body.inner.into_iter().map(Entry::Comment));
        settings
    }

    fn id(&mut self) -> super::ItemId {
        let id = super::ItemId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Everything inside `element`: hotkeys, menu items, and the comments
    /// between them, placed by the rules on [`Comment`].
    fn body(&mut self, element: Element<'_>, container: Container) -> Body {
        let mut body = Body::default();
        let mut pending: Vec<Comment> = Vec::new();
        let mut seen_element = false;
        let mut seen_variables = false;
        let mut seen_skin = false;
        let mut seen_search = false;

        for node in element.children_with_comments() {
            let child = match node {
                Node::Comment(comment) => {
                    pending.push(comment_of(comment));
                    continue;
                }
                Node::Element(child) => child,
            };

            let first = !seen_element;
            seen_element = true;
            let name = child.name();

            // Not menu items: they take every comment above them with them
            // to wherever comments go when nothing owns them.
            match name {
                "hotkey" => {
                    body.inner.append(&mut pending);
                    match self.hotkey(child) {
                        Some(hotkey) => body.hotkeys.push(hotkey),
                        None => {
                            let kept = self.disabled(child, "a <hotkey> needs a key");
                            body.inner.push(kept);
                        }
                    }
                    continue;
                }
                "variables" if container == Container::Root && !seen_variables => {
                    seen_variables = true;
                    self.free(&mut body, &mut pending, container, first);
                    body.variables = Some(self.variables(child));
                    continue;
                }
                "skin" if container == Container::Root && !seen_skin => {
                    seen_skin = true;
                    self.free(&mut body, &mut pending, container, first);
                    body.skin = Some(self.skin(child));
                    continue;
                }
                "search" if container == Container::Root && !seen_search => {
                    seen_search = true;
                    let (lead, blank_before) = attached(&mut pending, child);
                    self.free(&mut body, &mut pending, container, first);
                    body.search = Some(self.search(child, lead, blank_before));
                    continue;
                }
                _ => {}
            }

            // A menu item, or something to keep as a disabled item.
            let item = if container == Container::Leaf {
                None
            } else {
                self.item(child)
            };
            let Some(mut item) = item else {
                let reason = if name == "app" {
                    "an <app> needs an exe or an app-id".to_owned()
                } else if container == Container::Leaf {
                    format!(
                        "<{}> has no menu items of its own, so y-pop ignores <{name}>",
                        element.name()
                    )
                } else {
                    format!("y-pop does not load <{name}> here")
                };
                self.free(&mut body, &mut pending, container, first);
                let kept = self.disabled(child, &reason);
                match container {
                    Container::Leaf => body.inner.push(kept),
                    _ => body.entries.push(Entry::Comment(kept)),
                }
                continue;
            };

            (item.lead, item.blank_before) = attached(&mut pending, child);

            self.free(&mut body, &mut pending, container, first);
            body.entries.push(Entry::Item(item));
        }

        // Comments after the last child.
        let rest = std::mem::take(&mut pending);
        match container {
            Container::Leaf => body.inner.extend(rest),
            Container::Root if !seen_element => body.intro.extend(rest),
            _ => body.entries.extend(rest.into_iter().map(Entry::Comment)),
        }
        body
    }

    /// Moves comments no item claimed into their place: the file's intro if
    /// nothing but comments came before, else free entries of the list.
    fn free(
        &mut self,
        body: &mut Body,
        pending: &mut Vec<Comment>,
        container: Container,
        first: bool,
    ) {
        let comments = std::mem::take(pending);
        match container {
            Container::Root if first => body.intro.extend(comments),
            Container::Leaf => body.inner.extend(comments),
            _ => body
                .entries
                .extend(comments.into_iter().map(Entry::Comment)),
        }
    }

    /// `<search>` and everything in it. Group and folder children are
    /// ordinary [`Item`]s; the other sections keep only what they say.
    fn search(
        &mut self,
        element: Element<'_>,
        lead: Vec<Comment>,
        blank_before: bool,
    ) -> SearchSettings {
        let mut attrs = Attrs::of(element);
        let mut search = SearchSettings {
            lead,
            blank_before,
            hotkey: attrs.take("hotkey"),
            can_override: attrs.bool("override").unwrap_or(false),
            collapsed: attrs.bool("collapsed"),
            monitor: attrs.parse("monitor", MonitorMode::parse),
            results: attrs.parse("results", ResultsMode::parse),
            row_actions: attrs.bool("row-actions"),
            history: attrs.bool("history"),
            theme: attrs.parse("theme", parse_theme),
            ..SearchSettings::default()
        };
        search.extra_attrs = attrs.rest();

        let mut pending: Vec<Comment> = Vec::new();
        for node in element.children_with_comments() {
            let child = match node {
                Node::Comment(comment) => {
                    pending.push(comment_of(comment));
                    continue;
                }
                Node::Element(child) => child,
            };
            if child.name() == "skin" && search.skin.is_empty() {
                search.skin = self.skin(child);
                continue;
            }
            let (lead, blank_before) = attached(&mut pending, child);
            search
                .children
                .extend(pending.drain(..).map(SearchEntry::Comment));

            let entry = match child.name() {
                "group" | "app" => self.item(child).map(|mut item| {
                    (item.lead, item.blank_before) = (lead, blank_before);
                    SearchEntry::Item(item)
                }),
                _ => self.section(child).map(|mut section| {
                    (section.lead, section.blank_before) = (lead, blank_before);
                    SearchEntry::Section(section)
                }),
            };
            let entry = entry.unwrap_or_else(|| {
                let reason = format!("y-pop does not load <{}> here", child.name());
                SearchEntry::Comment(self.disabled(child, &reason))
            });
            search.children.push(entry);
        }
        search
            .children
            .extend(pending.into_iter().map(SearchEntry::Comment));
        search
    }

    /// `<recent>`, `<menus>`, `<start-menu>` or `<plugin>`. `None` for
    /// anything else, and for one that holds children or comments, which
    /// the model has nowhere to keep.
    fn section(&mut self, element: Element<'_>) -> Option<SearchSection> {
        // Only `<actions>` has anything inside; for the others, children and
        // comments are something the model has nowhere to keep.
        if element.name() != "actions" && element.children_with_comments().next().is_some() {
            return None;
        }
        let mut attrs = Attrs::of(element);
        let kind = match element.name() {
            "actions" => SectionKind::Actions {
                menu_key: attrs.take("menu-key"),
                quick_pick: attrs.bool("quick-pick"),
                entries: action_entries(element)?,
            },
            "recent" => SectionKind::Recent {
                count: attrs.parse("count", parse_u32),
            },
            "menus" => SectionKind::Menus {
                browse: attrs.parse("browse", MenusMode::parse),
                include: attrs.take("include"),
            },
            "start-menu" => SectionKind::StartMenu {
                exclude: attrs.take("exclude"),
            },
            "plugin" => SectionKind::Plugin,
            _ => return None,
        };
        Some(SearchSection {
            lead: Vec::new(),
            blank_before: false,
            kind,
            extra_attrs: attrs.rest(),
        })
    }

    /// A menu element, or `None` if y-pop wouldn't load it.
    fn item(&mut self, element: Element<'_>) -> Option<Item> {
        let mut attrs = Attrs::of(element);

        let (kind, container) = match element.name() {
            "group" => (ItemKind::Group, Container::Group),
            "app" => (
                ItemKind::App(Box::new(self.app_fields(&mut attrs)?)),
                Container::Leaf,
            ),
            "separator" => (ItemKind::Separator, Container::Leaf),
            "about" => (ItemKind::About, Container::Leaf),
            "quit" => (ItemKind::Quit, Container::Leaf),
            "settings" => (ItemKind::Settings, Container::Leaf),
            "break" => (ItemKind::Break, Container::Leaf),
            _ => return None,
        };

        let mut item = Item::new(self.id(), kind);
        if item.kind != ItemKind::Break {
            item.name = attrs.take("name");
            item.icon = attrs.take("icon");
            item.tooltip = attrs.take("tooltip");
            item.keywords = attrs.take("keywords");
            item.default = attrs.bool("default").unwrap_or(false);
        }
        if matches!(item.kind, ItemKind::Group | ItemKind::App(_)) {
            item.show_shortcuts = attrs.bool("show-shortcuts");
        }
        if item.kind == ItemKind::Group {
            item.auto_open = attrs.bool("autoopen").unwrap_or(false);
            item.popup = attrs.parse("popup", Renderer::parse);
            item.popup_style = attrs.parse("popup-style", PopupStyle::parse);
        }
        item.extra_attrs = attrs.rest();

        let body = self.body(element, container);
        item.hotkeys = body.hotkeys;
        item.inner_comments = body.inner;
        item.children = body.entries;
        Some(item)
    }

    /// An `<app>`'s own attributes, taken off `attrs`. `None` when it names
    /// no target, which y-pop drops.
    fn app_fields(&mut self, attrs: &mut Attrs) -> Option<AppFields> {
        let exe = attrs.take("exe");
        let app_id = attrs.take("app-id");
        if exe.is_none() && app_id.is_none() {
            return None;
        }

        let mut app = AppFields {
            exe,
            app_id,
            param: attrs.take("param"),
            path: attrs.take("path"),
            shell: attrs.bool("shell"),
            elevate: attrs.bool("elevate").unwrap_or(false),
            browse: attrs.bool("browse").unwrap_or(false),
            index: attrs.bool("index").unwrap_or(false),
            filter: attrs.take("filter"),
            depth: attrs.parse("depth", |text| text.trim().parse().ok()),
            open_folder: attrs.parse("open-folder", parse_open_folder),
            ..AppFields::default()
        };
        if let Some(events) = attrs.take("events") {
            for event in events.split(',').map(str::trim) {
                match event {
                    "start" => app.on_start = true,
                    "exit" => app.on_exit = true,
                    "" => {}
                    other => app.other_events.push(other.to_owned()),
                }
            }
        }
        Some(app)
    }

    /// `None` without a `key`: y-pop drops such a hotkey.
    fn hotkey(&mut self, element: Element<'_>) -> Option<HotkeyEntry> {
        let mut attrs = Attrs::of(element);
        let key = attrs.take("key")?;
        let can_override = attrs.bool("override").unwrap_or(false);
        Some(HotkeyEntry {
            key,
            can_override,
            extra_attrs: attrs.rest(),
        })
    }

    fn variables(&mut self, section: Element<'_>) -> Vec<VarEntry> {
        let mut entries = Vec::new();
        for node in section.children_with_comments() {
            match node {
                Node::Comment(comment) => entries.push(VarEntry::Comment(comment_of(comment))),
                Node::Element(child) => {
                    let var = (child.name() == "var").then(|| {
                        let mut attrs = Attrs::of(child);
                        let name = attrs.take("name")?;
                        let value = attrs.take("value")?;
                        Some(Var {
                            name,
                            value,
                            blank_before: child.blank_before(),
                            extra_attrs: attrs.rest(),
                        })
                    });
                    match var.flatten() {
                        Some(var) => entries.push(VarEntry::Var(var)),
                        None => {
                            let kept = self.disabled(child, "not a complete <var>");
                            entries.push(VarEntry::Comment(kept));
                        }
                    }
                }
            }
        }
        entries
    }

    fn skin(&mut self, element: Element<'_>) -> Skin {
        let mut attrs = Attrs::of(element);
        let mut overrides = ThemeOverrides {
            back: attrs.parse("back-color", Color::parse),
            text: attrs.parse("text-color", Color::parse),
            hover_back: attrs.parse("hover-back-color", Color::parse),
            hover_text: attrs.parse("hover-text-color", Color::parse),
            disabled_text: attrs.parse("disabled-text-color", Color::parse),
            padding: attrs.parse("padding", parse_u32),
            margin: attrs.parse("margin", parse_u32),
            edge: attrs.parse("edge", parse_edge),
            separator: attrs.parse("separator", parse_separator),
            ..ThemeOverrides::default()
        };
        let mut skin = Skin {
            extra_attrs: attrs.rest(),
            ..Skin::default()
        };

        for node in element.children_with_comments() {
            let child = match node {
                Node::Comment(comment) => {
                    skin.notes.push(comment_of(comment));
                    continue;
                }
                Node::Element(child) => child,
            };
            let kept = match child.name() {
                "border" if overrides.border.is_none() => {
                    overrides.border = border(child);
                    overrides.border.is_none()
                }
                "background-gradient" if overrides.background_gradient.is_none() => {
                    overrides.background_gradient = gradient(child);
                    overrides.background_gradient.is_none()
                }
                "icon-gradient" if overrides.icon_gradient.is_none() => {
                    overrides.icon_gradient = gradient(child);
                    overrides.icon_gradient.is_none()
                }
                _ => true,
            };
            if kept {
                let comment = self.disabled(child, "not a complete, known <skin> part");
                skin.notes.push(comment);
            }
        }

        skin.overrides = overrides;
        skin
    }

    /// `element` as the XML of a disabled item, with a warning saying so.
    fn disabled(&mut self, element: Element<'_>, reason: &str) -> Comment {
        self.warnings.push(format!(
            "line {}: <{}> is kept in the file as a comment ({reason})",
            element.line(),
            element.name()
        ));
        Comment::new(element_xml(element))
    }
}

/// Takes the comments attached to `child` off the end of `pending`: the run
/// directly above it, each with no empty line before the next thing. Returns
/// them (with their own empty-line flag cleared) and whether an empty line
/// separates the whole attached group from what came before.
fn attached(pending: &mut Vec<Comment>, child: Element<'_>) -> (Vec<Comment>, bool) {
    let mut start = pending.len();
    let mut linked = !child.blank_before();
    while linked && start > 0 {
        start -= 1;
        linked = !pending[start].blank_before;
    }
    let lead: Vec<Comment> = pending.split_off(start);
    let blank_before = match lead.first() {
        Some(first_lead) => first_lead.blank_before,
        None => child.blank_before(),
    };
    let lead = lead
        .into_iter()
        .map(|comment| Comment {
            blank_before: false,
            ..comment
        })
        .collect();
    (lead, blank_before)
}

/// The XML of `element` and everything in it, without its comments.
fn element_xml(element: Element<'_>) -> String {
    fn write(element: Element<'_>, writer: &mut Writer) {
        writer.open(element.name());
        for (name, value) in element.attributes() {
            writer.attr(name, value);
        }
        for child in element.children() {
            write(child, writer);
        }
        writer.close();
    }
    let mut writer = Writer::new();
    write(element, &mut writer);
    writer.finish().trim().to_owned()
}

fn comments<'a>(source: impl Iterator<Item = XmlComment<'a>>) -> Vec<Comment> {
    source.map(comment_of).collect()
}

fn comment_of(comment: XmlComment<'_>) -> Comment {
    Comment {
        text: comment.text().to_owned(),
        blank_before: comment.blank_before(),
    }
}

fn border(element: Element<'_>) -> Option<BorderOverride> {
    let mut attrs = Attrs::of(element);
    let border = BorderOverride {
        size: attrs.parse("size", parse_u32)?,
        round: attrs.parse("round", parse_u32)?,
        color: attrs.parse("color", Color::parse)?,
    };
    // Anything else on it would be lost, so it stays a disabled item.
    (attrs.rest().is_empty() && element.children().next().is_none()).then_some(border)
}

fn gradient(element: Element<'_>) -> Option<GradientOverride> {
    let mut attrs = Attrs::of(element);
    let gradient = GradientOverride {
        start: attrs.parse("start", Color::parse)?,
        end: attrs.parse("end", Color::parse)?,
        direction: match attrs.take("direction").as_deref() {
            None | Some("h") => GradientDirection::Horizontal,
            Some("v") => GradientDirection::Vertical,
            Some(_) => return None,
        },
    };
    (attrs.rest().is_empty() && element.children().next().is_none()).then_some(gradient)
}

fn parse_u32(text: &str) -> Option<u32> {
    text.trim().parse().ok()
}

fn parse_theme(text: &str) -> Option<Theme> {
    Some(match text {
        "native" => Theme::Native,
        "default" => Theme::Default,
        "flat" => Theme::Flat,
        "dark" | "black" => Theme::Dark,
        "graphite" => Theme::Graphite,
        "midnight" => Theme::Midnight,
        "sand" => Theme::Sand,
        "auto" => Theme::Auto,
        _ => return None,
    })
}

fn parse_separator(text: &str) -> Option<SeparatorSpan> {
    Some(match text {
        "after-icons" => SeparatorSpan::AfterIcons,
        "full" => SeparatorSpan::Full,
        _ => return None,
    })
}

fn parse_edge(text: &str) -> Option<EdgeRule> {
    Some(match text {
        "idle" => EdgeRule::Idle,
        "hover" => EdgeRule::Hover,
        "always" => EdgeRule::Always,
        _ => return None,
    })
}

fn parse_open_folder(text: &str) -> Option<OpenFolder> {
    Some(match text {
        "all" => OpenFolder::All,
        "max-depth" => OpenFolder::MaxDepth,
        "never" => OpenFolder::Never,
        _ => return None,
    })
}

/// The same words [`Element::attr_bool`] accepts.
fn parse_bool(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

impl MonitorMode {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "cursor" => Some(Self::Cursor),
            "focus" => Some(Self::Focus),
            "primary" => Some(Self::Primary),
            _ => None,
        }
    }
}

impl ResultsMode {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "sections" => Some(Self::Sections),
            "flat" => Some(Self::Flat),
            _ => None,
        }
    }
}

impl MenusMode {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "columns" => Some(Self::Columns),
            "a-z" => Some(Self::AZ),
            "list" => Some(Self::List),
            _ => None,
        }
    }
}

impl Renderer {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "native" => Some(Self::Native),
            "overlay" => Some(Self::Overlay),
            _ => None,
        }
    }
}

impl PopupStyle {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "list" => Some(Self::List),
            "runner" => Some(Self::Runner),
            "radial" => Some(Self::Radial),
            _ => None,
        }
    }
}

/// An element's attributes, handed out one at a time. What is never taken —
/// or was taken by `parse`/`bool` but didn't parse, which puts it back — is
/// [`Attrs::rest`].
struct Attrs(Vec<(String, String)>);

impl Attrs {
    fn of(element: Element<'_>) -> Self {
        Self(
            element
                .attributes()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        )
    }

    fn take(&mut self, name: &str) -> Option<String> {
        let index = self.0.iter().position(|(key, _)| key == name)?;
        Some(self.0.remove(index).1)
    }

    /// Takes `name` only if `parse` understands its value.
    fn parse<T>(&mut self, name: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
        let index = self.0.iter().position(|(key, _)| key == name)?;
        let value = parse(&self.0[index].1)?;
        self.0.remove(index);
        Some(value)
    }

    fn bool(&mut self, name: &str) -> Option<bool> {
        self.parse(name, parse_bool)
    }

    fn rest(self) -> Vec<(String, String)> {
        self.0
    }
}

/// What is inside an `<actions>`: its `<action>`s and the comments among them.
/// `None` if there is anything else (the model has nowhere to keep it), which
/// leaves the whole element as a disabled item.
fn action_entries(element: Element<'_>) -> Option<Vec<ActionEntry>> {
    let mut entries = Vec::new();
    for node in element.children_with_comments() {
        match node {
            Node::Comment(comment) => entries.push(ActionEntry::Comment(comment_of(comment))),
            Node::Element(child) => {
                if child.name() != "action" || child.children_with_comments().next().is_some() {
                    return None;
                }
                let mut attrs = Attrs::of(child);
                let mut item = ActionItem {
                    id: attrs.take("do"),
                    key: attrs.take("key"),
                    button: attrs.bool("button"),
                    extra_attrs: Vec::new(),
                };
                item.extra_attrs = attrs.rest();
                entries.push(ActionEntry::Action(item));
            }
        }
    }
    Some(entries)
}
