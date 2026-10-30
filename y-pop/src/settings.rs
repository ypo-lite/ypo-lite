//! The editable form of `y-pop.xml` — what `y-pop-config` loads, changes and
//! saves.
//!
//! [`crate::model::ConfigTree`] is built to be *run*: it expands `%vars%`,
//! interns strings and never changes after loading, so writing it back would
//! turn `%win%\notepad.exe` into `C:\Windows\notepad.exe`. [`Settings`] is the
//! other shape: an owned tree of the file's own, **raw** text, cheap to insert
//! into and move around in, and able to write itself out again.
//!
//! # What a save keeps
//!
//! Layout is regenerated (tabs, one canonical attribute order, defaults left
//! out), but nothing the user wrote is dropped:
//!
//! * **Comments** stay where they were — see [`Comment`] for the placement
//!   rules ("attached" above an item, free between items, file notes).
//! * **Unknown attributes**, and known ones whose value doesn't parse
//!   (`popup="neon"`), stay in the owning element's `extra_attrs` and are
//!   written back; a field the editor sets itself wins over a stale extra.
//! * **An element the reader rejects** (an unknown element, an `<app>` with no
//!   target, a half-written `<border>`) is kept as a *disabled item*: a
//!   comment holding its XML. Its own inner comments are the one thing left
//!   out — a comment can't contain a comment.
//!
//! Warnings are the real reader's own ([`crate::config::load_str`] on the same
//! text), plus a note for each element kept as a comment, so what the editor
//! reports is exactly what y-pop would.
//!
//! The reading half is [`load`], the writing half [`save`]; this file is the
//! model they share.

use crate::model::OpenFolder;
use crate::theme::{Theme, ThemeOverrides};

mod load;
mod save;
mod vars;

pub use vars::{NameError, UsedIn, references, rename_references};

#[cfg(test)]
mod tests;

/// Identifies an [`Item`] for as long as a [`Settings`] lives. Assigned at load
/// (or by [`Settings::new_id`]) and kept when the item moves, so a screen can
/// key its rows and selection by it instead of by position.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ItemId(u32);

impl ItemId {
    pub fn get(self) -> u32 {
        self.0
    }
}

/// An XML comment, as the text between `<!--` and `-->` (trimmed, shared
/// indentation removed — see [`y_core::xml::Comment::text`]).
///
/// Where it lives depends on where it sat in the file:
///
/// * directly above an item with no empty line between them: that item's
///   [`Item::lead`] — it moves, is deleted and is commented out with it;
/// * anywhere else among the entries of a group, or of the roots: an
///   [`Entry::Comment`] of its own;
/// * before `<y-pop>`, after it, or before its first child: the file notes
///   ([`Settings::prolog`], [`Settings::epilog`], [`Settings::intro`]).
///
/// A comment can also hold a *disabled item*: the XML of an element that was
/// commented out, or that y-pop could not load.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Comment {
    pub text: String,
    /// An empty line separated it from what came before. Kept so a saved
    /// file's grouping stays close to the original. Always `false` on an
    /// attached comment ([`Item::blank_before`] carries it for the group).
    pub blank_before: bool,
}

impl Comment {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            blank_before: false,
        }
    }
}

/// One thing in a group's (or the root's) list: a menu item, or a comment of
/// its own.
// An `Item` is the common case and a `Comment` is rare: boxing the item would
// touch every walk of the tree for a few bytes saved per comment.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq, Debug)]
pub enum Entry {
    Item(Item),
    Comment(Comment),
}

/// Any menu node. The fields that don't apply to its [`ItemKind`] stay at
/// their defaults, and are not written.
#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    pub id: ItemId,
    pub kind: ItemKind,
    /// Comments attached above the element, in order.
    pub lead: Vec<Comment>,
    /// An empty line separates this item (with its `lead`) from the entry
    /// before it.
    pub blank_before: bool,
    /// `None` is "no attribute" — which is not the same as `name=""` for a
    /// separator, an `<about/>` or a `<quit/>`.
    pub name: Option<String>,
    /// `path` or `path,index`, raw.
    pub icon: Option<String>,
    pub tooltip: Option<String>,
    /// Extra words search finds it by (`keywords="mail outlook"`).
    pub keywords: Option<String>,
    pub default: bool,
    /// Groups: open the menu when the pointer hovers its tray icon.
    pub auto_open: bool,
    /// `None` inherits [`Globals::show_shortcuts`].
    pub show_shortcuts: Option<bool>,
    /// Groups: override of [`Globals::popup`] for this menu and below.
    pub popup: Option<Renderer>,
    pub popup_style: Option<PopupStyle>,
    pub hotkeys: Vec<HotkeyEntry>,
    /// Comments among the element's own `<hotkey>`s (or, for a leaf, any
    /// comment inside it), and disabled children a leaf can't hold.
    pub inner_comments: Vec<Comment>,
    pub extra_attrs: Vec<(String, String)>,
    /// A group's entries, in order. Empty for every other kind.
    pub children: Vec<Entry>,
}

impl Item {
    pub fn new(id: ItemId, kind: ItemKind) -> Self {
        Self {
            id,
            kind,
            lead: Vec::new(),
            blank_before: false,
            name: None,
            icon: None,
            tooltip: None,
            keywords: None,
            default: false,
            auto_open: false,
            show_shortcuts: None,
            popup: None,
            popup_style: None,
            hotkeys: Vec::new(),
            inner_comments: Vec::new(),
            extra_attrs: Vec::new(),
            children: Vec::new(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum ItemKind {
    Group,
    App(Box<AppFields>),
    Separator,
    About,
    Quit,
    Settings,
    Break,
}

impl ItemKind {
    /// The element's name in the file.
    pub fn element(&self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::App(_) => "app",
            Self::Separator => "separator",
            Self::About => "about",
            Self::Quit => "quit",
            Self::Settings => "settings",
            Self::Break => "break",
        }
    }
}

/// What only an `<app>` has. Strings are raw (`%win%\notepad.exe`).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct AppFields {
    pub exe: Option<String>,
    /// A Store/RT app id. Wins over `exe` when both are set.
    pub app_id: Option<String>,
    pub param: Option<String>,
    /// The working folder.
    pub path: Option<String>,
    /// `None` inherits [`Globals::shell_execute`].
    pub shell: Option<bool>,
    pub elevate: bool,
    /// `events="start,exit"`.
    pub on_start: bool,
    pub on_exit: bool,
    /// Event names y-pop doesn't know, kept so they aren't lost.
    pub other_events: Vec<String>,
    /// `exe` names a folder, listed as a submenu.
    pub browse: bool,
    /// A browsed folder on a network share: list it for `/` search too.
    pub index: bool,
    pub filter: Option<String>,
    pub depth: Option<u32>,
    pub open_folder: Option<OpenFolder>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct HotkeyEntry {
    /// `Win+Y`, raw.
    pub key: String,
    pub can_override: bool,
    pub extra_attrs: Vec<(String, String)>,
}

/// `popup="native|overlay"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Renderer {
    Native,
    Overlay,
}

/// `popup-style="list|runner|radial"`. Only `list` is implemented; the file
/// may still say the others, and a save keeps them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupStyle {
    List,
    Runner,
    Radial,
}

/// The attributes of `<y-pop>` itself. `None` is "not written", which is the
/// reader's default — and is a different thing from writing that default.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Globals {
    pub shell_execute: Option<bool>,
    pub show_shortcuts: Option<bool>,
    pub popup: Option<Renderer>,
    pub popup_style: Option<PopupStyle>,
    pub theme: Option<Theme>,
    pub extra_attrs: Vec<(String, String)>,
}

/// `<skin>`: the overrides on top of the theme, plus what the file also held.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Skin {
    pub overrides: ThemeOverrides,
    /// Comments inside `<skin>`, and disabled children.
    pub notes: Vec<Comment>,
    pub extra_attrs: Vec<(String, String)>,
}

impl Skin {
    /// Nothing to write: no `<skin>` element at all.
    pub fn is_empty(&self) -> bool {
        !self.overrides.any_set() && self.notes.is_empty() && self.extra_attrs.is_empty()
    }
}

/// One line of `<variables>`.
#[derive(Clone, PartialEq, Debug)]
pub enum VarEntry {
    Var(Var),
    Comment(Comment),
}

/// `<var name="" value=""/>`. A variable only sees the ones above it, so the
/// order of [`Settings::variables`] matters.
#[derive(Clone, PartialEq, Debug)]
pub struct Var {
    pub name: String,
    /// Raw: may use `%builtin%` and earlier variables.
    pub value: String,
    pub blank_before: bool,
    pub extra_attrs: Vec<(String, String)>,
}

/// `<search>`: its own attributes and its children, in order. Like [`Item`],
/// raw and tolerant — a value the reader wouldn't understand stays in
/// `extra_attrs` rather than being lost.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct SearchSettings {
    /// Comments attached above the element.
    pub lead: Vec<Comment>,
    pub blank_before: bool,
    /// `Win+Space`, raw.
    pub hotkey: Option<String>,
    pub can_override: bool,
    pub collapsed: Option<bool>,
    pub results: Option<ResultsMode>,
    pub row_actions: Option<bool>,
    pub history: Option<bool>,
    /// The window's own base theme; `None` follows the tray menus'.
    pub theme: Option<Theme>,
    /// The window's own `<skin>`, written first inside `<search>`; empty
    /// follows the tray menus' (unless [`SearchSettings::theme`] is set).
    pub skin: Skin,
    pub extra_attrs: Vec<(String, String)>,
    pub children: Vec<SearchEntry>,
}

/// `results="sections|flat"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResultsMode {
    Sections,
    Flat,
}

/// One child of `<search>`.
#[derive(Clone, PartialEq, Debug)]
pub enum SearchEntry {
    Comment(Comment),
    /// A search-only `<group>`, or a browsed-folder `<app>`.
    Item(Item),
    Section(SearchSection),
}

/// A section element that is not a menu item.
#[derive(Clone, PartialEq, Debug)]
pub struct SearchSection {
    pub lead: Vec<Comment>,
    pub blank_before: bool,
    pub kind: SectionKind,
    pub extra_attrs: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum SectionKind {
    Recent {
        count: Option<u32>,
    },
    Menus {
        browse: Option<MenusMode>,
        /// `;`-separated, raw.
        include: Option<String>,
    },
    StartMenu {
        exclude: Option<String>,
    },
    /// Every attribute stays in `extra_attrs`: plugins have no editor yet.
    Plugin,
    /// `<actions>`, with its `<action>`s and the comments among them.
    Actions {
        menu_key: Option<String>,
        quick_pick: Option<bool>,
        entries: Vec<ActionEntry>,
    },
}

/// One thing inside `<actions>`.
#[derive(Clone, PartialEq, Debug)]
pub enum ActionEntry {
    Action(ActionItem),
    Comment(Comment),
}

/// `<action do="" key="" button=""/>`, raw: a value the reader would not
/// understand is kept as it was written.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ActionItem {
    pub id: Option<String>,
    pub key: Option<String>,
    pub button: Option<bool>,
    pub extra_attrs: Vec<(String, String)>,
}

impl SectionKind {
    /// The element's name in the file.
    pub fn element(&self) -> &'static str {
        match self {
            Self::Recent { .. } => "recent",
            Self::Menus { .. } => "menus",
            Self::StartMenu { .. } => "start-menu",
            Self::Plugin => "plugin",
            Self::Actions { .. } => "actions",
        }
    }
}

/// `<menus browse="columns|a-z|list">`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenusMode {
    Columns,
    AZ,
    List,
}

/// A whole `y-pop.xml`.
#[derive(Clone, Debug)]
pub struct Settings {
    pub globals: Globals,
    pub skin: Skin,
    /// `<search>`, written right after `<skin>`.
    pub search: Option<SearchSettings>,
    pub variables: Vec<VarEntry>,
    /// Comments before `<y-pop>`.
    pub prolog: Vec<Comment>,
    /// Comments inside `<y-pop>`, before its first child.
    pub intro: Vec<Comment>,
    /// Comments after `</y-pop>`.
    pub epilog: Vec<Comment>,
    /// The tray menus (groups, or apps that launch on click), in order, with
    /// the comments between them.
    pub roots: Vec<Entry>,
    /// Why parts of the file need a look; shown above every screen.
    pub warnings: Vec<String>,
    next_id: u32,
    /// `y-pop.xml.bak` was already written this session.
    backed_up: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self::empty()
    }
}

impl Settings {
    /// No menus, no variables: what a missing file starts as.
    pub fn empty() -> Self {
        Self {
            globals: Globals::default(),
            skin: Skin::default(),
            search: None,
            variables: Vec::new(),
            prolog: Vec::new(),
            intro: Vec::new(),
            epilog: Vec::new(),
            roots: Vec::new(),
            warnings: Vec::new(),
            next_id: 1,
            backed_up: false,
        }
    }

    /// An id no item of these settings has.
    pub fn new_id(&mut self) -> ItemId {
        let id = ItemId(self.next_id);
        self.next_id += 1;
        id
    }

    /// A new, empty item of `kind` with a fresh id. The caller inserts it.
    pub fn new_item(&mut self, kind: ItemKind) -> Item {
        Item::new(self.new_id(), kind)
    }

    /// All the notes the file starts and ends with, as one text: before
    /// `<y-pop>`, then at the top inside it, then after it, separated by an
    /// empty line. What a single "notes" box shows.
    pub fn file_notes_text(&self) -> String {
        let notes: Vec<&str> = self
            .prolog
            .iter()
            .chain(&self.intro)
            .chain(&self.epilog)
            .map(|comment| comment.text.as_str())
            .collect();
        notes.join(
            "

",
        )
    }

    /// How many separate notes [`Settings::file_notes_text`] joins.
    pub fn file_notes_count(&self) -> usize {
        self.prolog.len() + self.intro.len() + self.epilog.len()
    }

    /// Replaces every file note with `text`, as one comment before `<y-pop>`
    /// — or with none if `text` is blank. Returns `false`, changing nothing,
    /// when `text` is what [`Settings::file_notes_text`] already says: a
    /// notes box reports its text on first render, and that is not an edit.
    ///
    /// Notes that sat at the top inside `<y-pop>` or after it move to the
    /// top once they are edited; until then they stay where they were.
    pub fn set_file_notes_text(&mut self, text: &str) -> bool {
        // A text box separates lines with `\r`; the file's notes use `\n`.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let text = text.as_str();
        if text == self.file_notes_text() {
            return false;
        }
        self.prolog = if text.trim().is_empty() {
            Vec::new()
        } else {
            vec![Comment::new(text)]
        };
        self.intro.clear();
        self.epilog.clear();
        true
    }

    /// Every item, depth first in document order (an item before its
    /// children). Comments are not items.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        fn walk<'a>(entries: &'a [Entry], out: &mut Vec<&'a Item>) {
            for entry in entries {
                if let Entry::Item(item) = entry {
                    out.push(item);
                    walk(&item.children, out);
                }
            }
        }
        let mut items = Vec::new();
        walk(&self.roots, &mut items);
        items.into_iter()
    }

    /// [`Self::items`], then the items only search has: its groups and
    /// folders and what is in them.
    pub fn all_items(&self) -> impl Iterator<Item = &Item> {
        self.items()
            .chain(self.search.iter().flat_map(vars::search_items))
    }
}
