//! Writing [`Settings`] back as `y-pop.xml`.
//!
//! Attributes come out in one fixed order per element, defaults are left
//! out, and tri-states (`Option<bool>`) are written only when set, so loading
//! and saving an untouched file twice gives the same bytes. Comments and the
//! empty lines between entries are written where they were read.
//!
//! Order inside `<y-pop>`: intro comments, `<variables>`, `<skin>`,
//! `<search>`, then the tray menus — the order the reader needs them in (a
//! variable only sees the ones above it, and the menus and search use them).

use std::io;
use std::path::{Path, PathBuf};

use y_core::overlay::Color;
use y_core::xml::Writer;

use super::{
    ActionEntry, Comment, Entry, Globals, HotkeyEntry, Item, ItemKind, MenusMode, PopupStyle,
    Renderer, ResultsMode, SearchEntry, SearchSection, SearchSettings, SectionKind, Settings, Skin,
    Var, VarEntry,
};
use crate::model::OpenFolder;
use crate::theme::{EdgeRule, GradientDirection, GradientOverride, SeparatorSpan, Theme};

impl Settings {
    /// The whole file as text.
    pub fn to_xml(&self) -> String {
        let mut writer = Writer::new();
        writer.declaration();
        comments(&mut writer, &self.prolog);

        writer.open("y-pop");
        globals(&mut writer, &self.globals);
        comments(&mut writer, &self.intro);
        variables(&mut writer, &self.variables);
        skin(&mut writer, &self.skin);
        if let Some(search) = &self.search {
            write_search(&mut writer, search);
        }
        entries(&mut writer, &self.roots);
        writer.close();

        comments(&mut writer, &self.epilog);
        writer.finish()
    }

    /// Writes the file at `path`, replacing it without ever leaving half a
    /// file: the text goes to a sibling file first and is renamed over the
    /// original. The first save of a session also keeps what was there as
    /// `y-pop.xml.bak`, once — so the version from before this session's
    /// edits is always one file away.
    pub fn save_to(&mut self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(directory)?;
        }

        if !self.backed_up && path.is_file() {
            std::fs::copy(path, sibling(path, ".bak"))?;
        }
        self.backed_up = true;

        let temporary = sibling(path, ".tmp");
        std::fs::write(&temporary, self.to_xml())?;
        std::fs::rename(&temporary, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }
}

/// `path` with `suffix` appended to its file name (`y-pop.xml` →
/// `y-pop.xml.bak`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(suffix);
    path.with_file_name(name)
}

/// Attributes written so far on the open element, so an `extra_attrs` entry
/// the editor has since set itself isn't written twice.
struct Attributes<'w> {
    writer: &'w mut Writer,
    written: Vec<&'static str>,
}

impl<'w> Attributes<'w> {
    fn new(writer: &'w mut Writer) -> Self {
        Self {
            writer,
            written: Vec::new(),
        }
    }

    fn text(&mut self, name: &'static str, value: Option<&str>) {
        if let Some(value) = value {
            self.writer.attr(name, value);
            self.written.push(name);
        }
    }

    fn bool(&mut self, name: &'static str, value: Option<bool>) {
        self.text(
            name,
            value.map(|value| if value { "true" } else { "false" }),
        );
    }

    /// Written only when set: `false` is what an absent attribute means.
    fn flag(&mut self, name: &'static str, value: bool) {
        self.bool(name, value.then_some(true));
    }

    fn number(&mut self, name: &'static str, value: Option<u32>) {
        self.text(name, value.map(|value| value.to_string()).as_deref());
    }

    fn color(&mut self, name: &'static str, value: Option<Color>) {
        self.text(name, value.map(hex).as_deref());
    }

    /// What was kept from the file and not claimed by a field above.
    fn extras(self, extras: &[(String, String)]) {
        for (name, value) in extras {
            if !self.written.contains(&name.as_str()) {
                self.writer.attr(name, value);
            }
        }
    }
}

fn globals(writer: &mut Writer, globals: &Globals) {
    // The root is opened by the caller; its attributes follow `open` here.
    let mut attributes = Attributes::new(writer);
    attributes.bool("shell-execute", globals.shell_execute);
    attributes.bool("show-shortcuts", globals.show_shortcuts);
    attributes.text("popup", globals.popup.map(Renderer::name));
    attributes.text("popup-style", globals.popup_style.map(PopupStyle::name));
    attributes.text("theme", globals.theme.map(theme_name));
    attributes.extras(&globals.extra_attrs);
}

fn variables(writer: &mut Writer, entries: &[VarEntry]) {
    if entries.is_empty() {
        return;
    }
    writer.open("variables");
    for entry in entries {
        match entry {
            VarEntry::Comment(comment) => write_comment(writer, comment),
            VarEntry::Var(Var {
                name,
                value,
                blank_before,
                extra_attrs,
            }) => {
                if *blank_before {
                    writer.blank_line();
                }
                writer.open("var");
                let mut attributes = Attributes::new(writer);
                attributes.text("name", Some(name));
                attributes.text("value", Some(value));
                attributes.extras(extra_attrs);
                writer.close();
            }
        }
    }
    writer.close();
}

fn skin(writer: &mut Writer, skin: &Skin) {
    if skin.is_empty() {
        return;
    }
    let overrides = &skin.overrides;

    writer.open("skin");
    let mut attributes = Attributes::new(writer);
    attributes.color("back-color", overrides.back);
    attributes.color("text-color", overrides.text);
    attributes.color("hover-back-color", overrides.hover_back);
    attributes.color("hover-text-color", overrides.hover_text);
    attributes.color("disabled-text-color", overrides.disabled_text);
    attributes.number("padding", overrides.padding);
    attributes.number("margin", overrides.margin);
    attributes.text("edge", overrides.edge.map(edge_name));
    attributes.text("separator", overrides.separator.map(separator_name));
    attributes.extras(&skin.extra_attrs);

    comments(writer, &skin.notes);
    if let Some(border) = overrides.border {
        writer.open("border");
        writer.attr("size", &border.size.to_string());
        writer.attr("round", &border.round.to_string());
        writer.attr("color", &hex(border.color));
        writer.close();
    }
    gradient(writer, "background-gradient", overrides.background_gradient);
    gradient(writer, "icon-gradient", overrides.icon_gradient);
    writer.close();
}

fn gradient(writer: &mut Writer, name: &str, gradient: Option<GradientOverride>) {
    let Some(gradient) = gradient else { return };
    writer.open(name);
    writer.attr("start", &hex(gradient.start));
    writer.attr("end", &hex(gradient.end));
    if gradient.direction == GradientDirection::Vertical {
        writer.attr("direction", "v");
    }
    writer.close();
}

fn comments(writer: &mut Writer, comments: &[Comment]) {
    for comment in comments {
        write_comment(writer, comment);
    }
}

fn write_comment(writer: &mut Writer, comment: &Comment) {
    if comment.blank_before {
        writer.blank_line();
    }
    writer.comment(&comment.text);
}

fn entries(writer: &mut Writer, entries: &[Entry]) {
    for entry in entries {
        match entry {
            Entry::Comment(comment) => write_comment(writer, comment),
            Entry::Item(item) => write_item(writer, item),
        }
    }
}

fn write_item(writer: &mut Writer, item: &Item) {
    if item.blank_before {
        writer.blank_line();
    }
    comments(writer, &item.lead);

    writer.open(item.kind.element());
    let mut attributes = Attributes::new(writer);
    if item.kind != ItemKind::Break {
        attributes.text("name", item.name.as_deref());
        attributes.text("icon", item.icon.as_deref());
        attributes.text("tooltip", item.tooltip.as_deref());
        attributes.text("keywords", item.keywords.as_deref());
        attributes.flag("default", item.default);
    }
    match &item.kind {
        ItemKind::Group => {
            attributes.flag("autoopen", item.auto_open);
            attributes.bool("show-shortcuts", item.show_shortcuts);
            attributes.text("popup", item.popup.map(Renderer::name));
            attributes.text("popup-style", item.popup_style.map(PopupStyle::name));
        }
        ItemKind::App(app) => {
            attributes.bool("show-shortcuts", item.show_shortcuts);
            attributes.text("exe", app.exe.as_deref());
            attributes.text("app-id", app.app_id.as_deref());
            attributes.text("param", app.param.as_deref());
            attributes.text("path", app.path.as_deref());
            attributes.bool("shell", app.shell);
            attributes.flag("elevate", app.elevate);

            let events: Vec<&str> = [(app.on_start, "start"), (app.on_exit, "exit")]
                .into_iter()
                .filter_map(|(on, name)| on.then_some(name))
                .chain(app.other_events.iter().map(String::as_str))
                .collect();
            let events = events.join(",");
            attributes.text("events", (!events.is_empty()).then_some(events.as_str()));

            attributes.flag("browse", app.browse);
            attributes.flag("index", app.index);
            attributes.text("filter", app.filter.as_deref());
            attributes.number("depth", app.depth);
            attributes.text("open-folder", app.open_folder.map(open_folder_name));
        }
        _ => {}
    }
    attributes.extras(&item.extra_attrs);

    comments(writer, &item.inner_comments);
    for hotkey in &item.hotkeys {
        write_hotkey(writer, hotkey);
    }
    entries(writer, &item.children);
    writer.close();
}

fn write_search(writer: &mut Writer, search: &SearchSettings) {
    if search.blank_before {
        writer.blank_line();
    }
    comments(writer, &search.lead);

    writer.open("search");
    let mut attributes = Attributes::new(writer);
    attributes.text("hotkey", search.hotkey.as_deref());
    attributes.flag("override", search.can_override);
    attributes.bool("collapsed", search.collapsed);
    attributes.text("results", search.results.map(ResultsMode::name));
    attributes.bool("row-actions", search.row_actions);
    attributes.bool("history", search.history);
    attributes.text("theme", search.theme.map(theme_name));
    attributes.extras(&search.extra_attrs);

    skin(writer, &search.skin);
    for child in &search.children {
        match child {
            SearchEntry::Comment(comment) => write_comment(writer, comment),
            SearchEntry::Item(item) => write_item(writer, item),
            SearchEntry::Section(section) => write_section(writer, section),
        }
    }
    writer.close();
}

fn write_section(writer: &mut Writer, section: &SearchSection) {
    if section.blank_before {
        writer.blank_line();
    }
    comments(writer, &section.lead);

    writer.open(section.kind.element());
    let mut attributes = Attributes::new(writer);
    match &section.kind {
        SectionKind::Recent { count } => attributes.number("count", *count),
        SectionKind::Menus { browse, include } => {
            attributes.text("browse", browse.map(MenusMode::name));
            attributes.text("include", include.as_deref());
        }
        SectionKind::StartMenu { exclude } => attributes.text("exclude", exclude.as_deref()),
        SectionKind::Plugin => {}
        SectionKind::Actions {
            menu_key,
            quick_pick,
            ..
        } => {
            attributes.text("menu-key", menu_key.as_deref());
            attributes.bool("quick-pick", *quick_pick);
        }
    }
    attributes.extras(&section.extra_attrs);

    if let SectionKind::Actions { entries, .. } = &section.kind {
        for entry in entries {
            match entry {
                ActionEntry::Comment(comment) => write_comment(writer, comment),
                ActionEntry::Action(item) => {
                    writer.open("action");
                    let mut attributes = Attributes::new(writer);
                    attributes.text("do", item.id.as_deref());
                    attributes.text("key", item.key.as_deref());
                    attributes.bool("button", item.button);
                    attributes.extras(&item.extra_attrs);
                    writer.close();
                }
            }
        }
    }
    writer.close();
}

impl ResultsMode {
    fn name(self) -> &'static str {
        match self {
            Self::Sections => "sections",
            Self::Flat => "flat",
        }
    }
}

impl MenusMode {
    fn name(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::AZ => "a-z",
            Self::List => "list",
        }
    }
}

fn write_hotkey(writer: &mut Writer, hotkey: &HotkeyEntry) {
    writer.open("hotkey");
    let mut attributes = Attributes::new(writer);
    attributes.text("key", Some(&hotkey.key));
    attributes.flag("override", hotkey.can_override);
    attributes.extras(&hotkey.extra_attrs);
    writer.close();
}

/// `RRGGBB`, or `RRGGBBAA` when not opaque — the form the reader takes.
fn hex(color: Color) -> String {
    if color.a == 255 {
        format!("{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        )
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Native => "native",
        Theme::Default => "default",
        Theme::Flat => "flat",
        Theme::Dark => "dark",
        Theme::Graphite => "graphite",
        Theme::Midnight => "midnight",
        Theme::Sand => "sand",
        Theme::Auto => "auto",
    }
}

fn separator_name(span: SeparatorSpan) -> &'static str {
    match span {
        SeparatorSpan::AfterIcons => "after-icons",
        SeparatorSpan::Full => "full",
    }
}

fn edge_name(edge: EdgeRule) -> &'static str {
    match edge {
        EdgeRule::Idle => "idle",
        EdgeRule::Hover => "hover",
        EdgeRule::Always => "always",
    }
}

fn open_folder_name(open_folder: OpenFolder) -> &'static str {
    match open_folder {
        OpenFolder::All => "all",
        OpenFolder::MaxDepth => "max-depth",
        OpenFolder::Never => "never",
    }
}

impl Renderer {
    fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Overlay => "overlay",
        }
    }
}

impl PopupStyle {
    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Runner => "runner",
            Self::Radial => "radial",
        }
    }
}

impl Item {
    /// This item as `y-pop.xml` would hold it: its attached comments, then
    /// its element and everything inside it. What an inspector previews —
    /// written by the same code as a save, so it can't drift from it.
    pub fn to_xml(&self) -> String {
        let mut writer = Writer::new();
        write_item(&mut writer, self);
        writer.finish().trim_end().to_owned()
    }
}
