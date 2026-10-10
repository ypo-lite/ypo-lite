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

/// A whole `y-pop.xml`.
#[derive(Clone, Debug)]
pub struct Settings {
    pub globals: Globals,
    pub skin: Skin,
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
}
