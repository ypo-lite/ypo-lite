//! The parsed configuration tree.
//!
//! A flat arena addressed by [`NodeId`], not a recursive `Vec<Node>`-owns-
//! children tree — see `docs/y-pop-plan.md`'s "Data model" section for why:
//! one allocation for the whole tree instead of one per node, no drop
//! recursion on a deep tree, and hotkey dispatch that is a direct array
//! index rather than a scan or a hash lookup. The shape mirrors
//! `y_core::xml::Document`'s own node arena.
//!
//! This module only defines the tree and how to walk it — parsing
//! `y-pop.xml` into one lives in [`crate::config`].

use std::sync::Arc;

use y_core::hotkey::Hotkey;

use crate::search::keys::Chord;
use crate::search::matcher::{fold, plain};
use crate::theme::{Theme, ThemeOverrides};

/// Index into a [`ConfigTree`]'s node arena. `u32` rather than `usize`: half
/// the width of a pointer-sized index on x64, and no config file will ever
/// have four billion nodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct NodeId(u32);

impl NodeId {
    fn new(index: usize) -> Self {
        Self(u32::try_from(index).expect("more nodes than fit in a u32"))
    }

    fn index(self) -> usize {
        self.0 as usize
    }

    /// For a renderer to fold into its own id space (a native menu's command
    /// id, say) and recover with [`NodeId::from_u32`] once the user picks
    /// something — not used for arena indexing, which stays internal.
    pub fn as_u32(self) -> u32 {
        self.0
    }

    pub fn from_u32(value: u32) -> Self {
        Self(value)
    }
}

/// The whole parsed file: one node arena plus the global defaults every node
/// can inherit or override.
///
/// Built once by [`crate::config::load`] and replaced wholesale on reload —
/// never mutated node-by-node, so there is no window where a dependent
/// subsystem (hotkey registration, an open menu) can observe half of a new
/// tree and half of the old one.
#[derive(Debug)]
pub struct ConfigTree {
    nodes: Vec<NodeData>,
    /// Top-level nodes — each becomes its own tray icon (a `Group`) or an
    /// app launched directly on tray click (an `App`).
    pub roots: Vec<NodeId>,
    /// One entry per registered hotkey, not per node — a node with two
    /// `<hotkey>`s occupies two slots here. `WM_HOTKEY`'s `wParam` is the id
    /// `y_core::hotkey::HotkeyManager` registered it under; ids are handed
    /// out sequentially from `y_core::ids::IDH_FIRST`, so recovering the
    /// owning node and combination is a direct array read — see
    /// [`ConfigTree::hotkey_owner`] and [`ConfigTree::hotkey_bindings`] —
    /// not a scan over "every node with a hotkey" the way PortAL's flat
    /// `hotkeys` vector implied, and not a hash lookup either.
    hotkeys: Vec<(NodeId, HotkeyBinding)>,
    pub defaults: Defaults,
    /// `<search>`, if the file has one. Its search-only groups and folders
    /// live in the same arena as everything else (so `NodeId`, `launch`,
    /// icons and `browse` work on them unchanged) but are **not** in
    /// [`ConfigTree::roots`], which keeps meaning "tray icons"; they are
    /// reached through [`SearchConfig::sections`].
    pub search: Option<SearchConfig>,
    /// Problems that did not stop the file from loading, one line each — the
    /// same severity split `y-snap`'s config reader follows: a malformed
    /// document is fatal (see `crate::config::LoadError::Malformed`), a
    /// recognized-but-wrong attribute is a warning and the rest of the file
    /// still loads.
    pub warnings: Vec<String>,
}

/// Global settings every node inherits unless it overrides them.
#[derive(Clone, Debug)]
pub struct Defaults {
    pub shell_execute: bool,
    pub show_shortcuts: bool,
    pub popup: PopupMode,
    pub theme: Theme,
    pub theme_overrides: ThemeOverrides,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            shell_execute: false,
            show_shortcuts: true,
            popup: PopupMode::Native,
            theme: Theme::Native,
            theme_overrides: ThemeOverrides::default(),
        }
    }
}

/// Which renderer builds the popup — see `docs/y-pop-plan.md`'s "The menu".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupMode {
    /// Owner-draw native `HMENU`. The only backend required for v1.
    Native,
    Overlay(PresenterStyle),
}

/// Which overlay presenter renders the popup. Only [`PresenterStyle::List`]
/// is implemented so far — [`crate::config`] warns rather than silently
/// falling back when a file asks for one of the others.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PresenterStyle {
    List,
    Runner,
    Radial,
}

/// The built-in entries a right click on a tray icon adds after its menu —
/// see [`ConfigTree::tray_extras`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrayExtras {
    pub search: bool,
    pub about: bool,
    pub quit: bool,
}

/// What can be done to a result — the built-in actions of the search box,
/// configured by `<actions>` (`docs/overlay-modes.md` §2.9).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ActionId {
    /// Launch it; on a group, go inside it.
    Open,
    /// Launch it the other way round for Windows's history, this once: not
    /// recorded if it normally is, recorded if it normally is not.
    OpenToggled,
    /// Launch it elevated, this once.
    RunAsAdmin,
    /// Show the file in Explorer.
    ShowInFolder,
    /// Put its path on the clipboard.
    CopyPath,
    /// Show where it is in its tray menu.
    Reveal,
    /// Edit it in `y-pop-config`.
    Edit,
    /// Make a tray-menu entry of a Start-menu item.
    AddToMenu,
    /// Remove it from the history.
    Forget,
}

impl ActionId {
    /// Every action, in the order of the default menu.
    pub const ALL: [ActionId; 9] = [
        Self::Open,
        Self::OpenToggled,
        Self::RunAsAdmin,
        Self::ShowInFolder,
        Self::CopyPath,
        Self::Reveal,
        Self::Edit,
        Self::AddToMenu,
        Self::Forget,
    ];

    /// The `do=""` name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::OpenToggled => "open-toggle-history",
            Self::RunAsAdmin => "run-as-admin",
            Self::ShowInFolder => "show-in-folder",
            Self::CopyPath => "copy-path",
            Self::Reveal => "reveal",
            Self::Edit => "edit",
            Self::AddToMenu => "add-to-menu",
            Self::Forget => "forget",
        }
    }

    /// What the action menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::OpenToggled => "Open without history",
            Self::RunAsAdmin => "Run as administrator",
            Self::ShowInFolder => "Show in folder",
            Self::CopyPath => "Copy path",
            Self::Reveal => "Reveal in menu",
            Self::Edit => "Edit in settings",
            Self::AddToMenu => "Add to a menu",
            Self::Forget => "Forget",
        }
    }

    /// What a row button calls it: short, since several share the row.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::OpenToggled => "No history",
            Self::RunAsAdmin => "Admin",
            Self::ShowInFolder => "Folder",
            Self::CopyPath => "Copy",
            Self::Reveal => "Reveal",
            Self::Edit => "Edit",
            Self::AddToMenu => "Add",
            Self::Forget => "Forget",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.name() == name)
    }
}

/// One entry of `<actions>`: an action, its shortcut if it has one, and
/// whether the selected row shows it as a button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionBinding {
    pub id: ActionId,
    pub key: Option<Chord>,
    pub button: bool,
}

/// `<actions>`: which actions exist, in menu order, and how they are reached.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActionsConfig {
    /// Opens the action menu on the selected result (besides a right click
    /// and `Shift+F10`).
    pub menu_key: Chord,
    /// `Alt+1…9` open the n-th result, and the rows say so.
    pub quick_pick: bool,
    /// Always holds [`ActionId::Open`].
    pub actions: Vec<ActionBinding>,
}

impl ActionsConfig {
    /// The shortcut of `id`, if it is configured and has one.
    pub fn key_of(&self, id: ActionId) -> Option<Chord> {
        self.actions.iter().find(|a| a.id == id)?.key
    }
}

impl Default for ActionsConfig {
    /// What `<search>` without `<actions>` means: `Enter` opens, `Ctrl+R`
    /// reveals, `Ctrl+E` edits, `Ctrl+Del` forgets, the rest in the menu only.
    fn default() -> Self {
        let chord = |text: &str| Some(Chord::parse(text).expect("a valid default shortcut"));
        let bind = |id, key: Option<Chord>, button| ActionBinding { id, key, button };
        Self {
            menu_key: Chord::parse("Alt+Enter").expect("a valid default shortcut"),
            quick_pick: true,
            actions: vec![
                bind(ActionId::Open, chord("Enter"), false),
                bind(ActionId::OpenToggled, chord("Ctrl+Shift+Enter"), false),
                bind(ActionId::RunAsAdmin, None, false),
                bind(ActionId::ShowInFolder, None, false),
                bind(ActionId::CopyPath, None, false),
                bind(ActionId::Reveal, chord("Ctrl+R"), true),
                bind(ActionId::Edit, chord("Ctrl+E"), true),
                bind(ActionId::AddToMenu, None, true),
                bind(ActionId::Forget, chord("Ctrl+Del"), true),
            ],
        }
    }
}

/// `<search>` — see `docs/overlay-modes.md` §2.
#[derive(Clone, Debug)]
pub struct SearchConfig {
    /// `hotkey="…"` (and `override="true"`): search has exactly one entry
    /// point, so it is an attribute rather than a `<hotkey>` child.
    pub hotkey: Option<HotkeyBinding>,
    /// Open on the input row alone instead of on Recent and the sections.
    pub collapsed: bool,
    pub results: ResultsLayout,
    /// Buttons on the selected result row; the keys work either way.
    pub row_actions: bool,
    /// Whether Windows's history is read, and told about launches.
    pub history: bool,
    /// In the order the empty box lists them; always contains a
    /// [`Section::Menus`].
    pub sections: Vec<Section>,
    /// What can be done to a result, and how.
    pub actions: ActionsConfig,
}

impl SearchConfig {
    /// What `<search/>` with no children means: `<recent/><menus/><start-menu/>`.
    pub fn default_sections() -> Vec<Section> {
        vec![
            Section::Recent {
                count: Section::DEFAULT_RECENT,
            },
            Section::Menus {
                browse: MenusBrowse::default(),
                include: Vec::new(),
            },
            Section::StartMenu {
                exclude: Vec::new(),
            },
        ]
    }

    /// How the tray menus section opens.
    pub fn menus_browse(&self) -> MenusBrowse {
        self.sections
            .iter()
            .find_map(|section| match section {
                Section::Menus { browse, .. } => Some(*browse),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The names `<menus include>` lists; empty means every tray menu.
    pub fn included_menus(&self) -> &[Arc<str>] {
        self.sections
            .iter()
            .find_map(|section| match section {
                Section::Menus { include, .. } => Some(include.as_slice()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Whether the tray menu called `name` is in the tray menus section.
    pub fn includes_menu(&self, name: &str) -> bool {
        let included = self.included_menus();
        included.is_empty() || included.iter().any(|menu| same_menu(menu, name))
    }

    /// The nodes of the search-only groups and folders, in section order.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.sections.iter().filter_map(|section| match section {
            Section::Group(id) | Section::Folder(id) => Some(*id),
            _ => None,
        })
    }
}

/// Whether two tray menu names are the same to `<menus include>`: compared as
/// search compares, so case, accents and a menu's `&` marker make no difference.
pub(crate) fn same_menu(a: &str, b: &str) -> bool {
    fold(&plain(a)) == fold(&plain(b))
}

/// `results="sections|flat"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ResultsLayout {
    /// Top hit, then groups of at most five.
    #[default]
    Sections,
    Flat,
}

/// `<menus browse="columns|a-z|list"/>`: how the tray menus section opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MenusBrowse {
    #[default]
    Columns,
    AZ,
    /// The tray icons as a plain list; a group is entered as a chip.
    List,
}

/// One child of `<search>`.
#[derive(Clone, Debug)]
pub enum Section {
    /// `<recent count="5"/>`: lists items of the other sections.
    Recent { count: u32 },
    /// `<menus/>`: every tray menu. `include` names the tray menus to limit
    /// it to (empty: all of them).
    Menus {
        browse: MenusBrowse,
        include: Vec<Arc<str>>,
    },
    /// `<start-menu/>`. `exclude` adds name patterns to the built-in filter.
    StartMenu { exclude: Vec<Arc<str>> },
    /// A search-only `<group>`: a node outside [`ConfigTree::roots`].
    Group(NodeId),
    /// A search-only browsed folder (`<app browse="true">`).
    Folder(NodeId),
    /// `<plugin>`: parsed and kept, not run yet (spec §2.11).
    Plugin(PluginSection),
}

impl Section {
    pub const DEFAULT_RECENT: u32 = 5;
}

#[derive(Clone, Debug)]
pub struct PluginSection {
    pub name: Arc<str>,
    pub src: Arc<str>,
    pub prefix: Option<char>,
}

/// A small set of per-node booleans, packed into a byte instead of separate
/// fields — `show_shortcut` is not in here because it is tri-state (inherit
/// the global default, or override it either way), which does not fit a
/// plain bit. A menu-column break used to live here too (PortAL's own
/// `break="true"`, one more flag on the item *after* which the break
/// happens) — it's [`NodeKind::Break`] instead now, its own node between two
/// siblings rather than a property of one of them; see that variant's doc
/// comment for why.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct NodeFlags(u8);

impl NodeFlags {
    const DEFAULT: u8 = 1 << 0;
    const AUTO_OPEN: u8 = 1 << 2;

    pub fn is_default(self) -> bool {
        self.0 & Self::DEFAULT != 0
    }

    /// `<group autoopen="true">` — opens on tray hover, no click. PortAL's
    /// own docs call this "not recommended but requested."
    pub fn auto_opens(self) -> bool {
        self.0 & Self::AUTO_OPEN != 0
    }

    pub fn set_default(&mut self, value: bool) {
        self.set(Self::DEFAULT, value);
    }

    pub fn set_auto_open(&mut self, value: bool) {
        self.set(Self::AUTO_OPEN, value);
    }

    fn set(&mut self, bit: u8, value: bool) {
        if value {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

/// One `<hotkey>` binding, plus whether it's allowed to fall back to a
/// low-level keyboard hook when `RegisterHotKey` refuses the combination —
/// `y-pop.xml`'s `override="true"` (named `over` in PortAL, `steal-from-
/// shell` in `y-snap.xml` — see `docs/portal-analysis.md`; `override` reads
/// more plainly in a config file than either). Named `can_override` here
/// rather than `override` itself, which is a reserved word in Rust; the
/// config-facing name and this field don't need to match syntactically, the
/// same way `show-shortcuts` doesn't match `show_shortcuts`. The hook is
/// installed only once the normal registration has already failed, never
/// proactively — see `y_core::hotkey::Fallback::LowLevelHook`, which this
/// maps straight onto. Off by default: a combination another application
/// already owns belongs to that application, until the user explicitly says
/// otherwise for this one.
#[derive(Clone, Copy, Debug)]
pub struct HotkeyBinding {
    pub hotkey: Hotkey,
    pub can_override: bool,
}

/// `start`/`exit` events an `App` fires itself on, in addition to being
/// launched from the menu. PortAL also defined `lock`/`unlock` but never
/// implemented either — dropped rather than carried over unfinished.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct EventMask(u8);

impl EventMask {
    pub const START: EventMask = EventMask(1 << 0);
    pub const EXIT: EventMask = EventMask(1 << 1);

    pub fn contains(self, other: EventMask) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: EventMask) {
        self.0 |= other.0;
    }
}

/// The reserved `icon=` value that asks for a Store app's own logo, resolved
/// from its `app-id` when the icon is loaded. On anything but a Store app it
/// means nothing, and the item gets its default icon.
pub const APP_LOGO: &str = "@app";

/// Where an icon comes from: a file path plus an optional index, the same
/// `"path,index"` shorthand PortAL's `ico="...,6"` uses for a multi-icon
/// `.exe`/`.dll`/`.ico`. `None` on an `App` node means "extract the icon
/// from `exe` itself" — resolved when the icon is actually loaded, not here.
/// A path of [`APP_LOGO`] is not a file.
#[derive(Clone, Debug)]
pub struct IconSource {
    pub path: Arc<str>,
    pub index: Option<i32>,
}

impl IconSource {
    /// Whether this is [`APP_LOGO`] rather than a file.
    pub fn is_app_logo(&self) -> bool {
        self.path.trim().eq_ignore_ascii_case(APP_LOGO)
    }
}

/// One node of the tree — a menu item, in whatever the active renderer turns
/// that into.
#[derive(Clone, Debug)]
pub struct NodeData {
    pub kind: NodeKind,
    pub name: Arc<str>,
    pub icon: Option<IconSource>,
    pub tooltip: Option<Arc<str>>,
    /// `keywords="…"`: extra words search matches this node by.
    pub keywords: Option<Arc<str>>,
    /// Almost always 0 or 1 element. An empty `Vec` is one pointer-sized
    /// word and no heap allocation, so the common case costs nothing extra
    /// without reaching for a small-vec crate the workspace otherwise has no
    /// need for.
    pub hotkeys: Vec<HotkeyBinding>,
    pub flags: NodeFlags,
    /// `None` means "inherit [`Defaults::show_shortcuts`]".
    pub show_shortcut: Option<bool>,
    /// Overrides [`Defaults::popup`] for this node's own submenu and
    /// everything under it, unless a descendant overrides it again.
    pub popup: Option<PopupMode>,
    /// Empty for a leaf, and for an unexpanded `Browse` app — see
    /// `docs/y-pop-plan.md`'s "Data model" for why `Browse` children are
    /// generated later, at menu-build time, rather than stored here.
    pub children: Vec<NodeId>,
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Group,
    App(App),
    /// An optionally titled rule between items.
    Separator,
    About,
    Quit,
    /// `<settings/>` — opens `y-pop-config` (see [`crate::ipc::CONFIG_EXE`]). A
    /// y-pop menu is entirely the user's own, so there is no fixed place for a
    /// "Settings…" entry; like `<about/>` and `<quit/>`, it goes wherever they
    /// put it.
    Settings,
    /// `<break/>` — a menu-column break at exactly this point among its
    /// siblings (`y_core::menu::PopupMenu::add_break`, Win32's
    /// `MF_MENUBREAK`), not an item of its own: [`crate::menu::
    /// append_children`] doesn't append anything for it, only arms the
    /// break flag the sibling right after it picks up. PortAL's own
    /// `break="true"` instead sat as one more attribute on the item the
    /// break follows — a node of its own is a better fit for `y-pop.xml`:
    /// it names the point between two children directly, doesn't need
    /// carrying on whichever sibling happens to precede it, and reads the
    /// same as `<separator/>` already does for "a thing between items that
    /// isn't itself choosable." A `<break/>` with nothing after it (the
    /// last child of its parent) has nothing to visibly do — same as
    /// PortAL's own `break="true"` on a group's last item.
    Break,
}

#[derive(Clone, Debug)]
pub struct App {
    /// Mutually exclusive with `app_id`: exactly one is set once a node
    /// finishes parsing (enforced in `crate::config`, not here — an
    /// in-arena `NodeData` is otherwise plain data with no invariants of its
    /// own to check).
    pub exe: Option<Arc<str>>,
    /// A Store/RT app id, launched via `IApplicationActivationManager`
    /// instead of `CreateProcess`/`ShellExecute`.
    pub app_id: Option<Arc<str>>,
    pub param: Option<Arc<str>>,
    pub path: Option<Arc<str>>,
    /// `shell=""` as written. `None` launches through the shell like
    /// `Some(true)` when the launch is recorded, and directly otherwise;
    /// `Some(false)` is always direct, and is never recorded unless the user
    /// asks for it ([`crate::launch::Usage`]).
    pub shell: Option<bool>,
    pub elevate: bool,
    pub events: EventMask,
    pub browse: Option<Browse>,
}

impl App {
    /// What opens the folder `dir` in Explorer: through the shell, the only
    /// route that can, and never elevated.
    pub fn open_folder(dir: &str) -> Self {
        Self {
            exe: Some(Arc::from(dir)),
            app_id: None,
            param: None,
            path: None,
            shell: Some(true),
            elevate: false,
            events: EventMask::default(),
            browse: None,
        }
    }
}

/// `browse="true"`: `exe` names a folder, listed as this node's children —
/// see the `NodeData::children` doc for when that listing actually happens.
#[derive(Clone, Debug)]
pub struct Browse {
    pub filter: Option<Arc<str>>,
    /// How many levels of subfolders become their own, further-browsable
    /// submenus — PortAL's own `browse`/`path` only ever went one level deep
    /// or unbounded (`recurse="true"`/`"false"`); `depth` generalises that to
    /// any level. `0`, the default (no `depth` attribute at all), means no
    /// folders are shown in the listing whatsoever — this app's own files
    /// only — the same as `y-pop`'s previous `recurse="false"` default, kept
    /// as a deliberate special case rather than falling out of the general
    /// rule below so that a plain `browse="true"` with no other attributes
    /// keeps behaving exactly as it always has.
    ///
    /// For `depth >= 1`: a subfolder at level *n* (the browsed folder's own
    /// immediate children are level 1) gets a real submenu — listed lazily,
    /// same as any other browsed folder — as long as `n <= depth`. A
    /// subfolder at level `depth + 1` has reached the deepest allowed level:
    /// it is shown flattened, a plain item with no submenu of its own, that
    /// opens the folder directly via the default application (Explorer)
    /// instead of listing further — see [`OpenFolder`] for whether it's
    /// shown at all. Nothing deeper than `depth + 1` is ever reached, since
    /// a flattened folder has no submenu left to list from.
    ///
    /// `u8`, not `usize`/`u32`: nobody sanely nests 200 levels of submenus
    /// (see [`MAX_DEPTH`], which `crate::config` clamps a config file's own
    /// `depth=""` to), and every one of Ypo-Lite's apps is built to stay
    /// low-memory — a byte that can't represent a value the feature has no
    /// real use for beats a wider int "just in case".
    pub depth: u8,
    /// Whether — and when — a browsed submenu also carries an extra entry to
    /// open the folder it lists directly, rather than only being reachable
    /// by digging into its contents.
    pub open_folder: OpenFolder,
    /// `index="true"`: search's `/` lists this folder even on a network share
    /// (`\\…`), which it otherwise skips, and says so.
    pub index: bool,
}

/// The hard ceiling [`crate::config`] clamps a `depth=""` attribute to — see
/// [`Browse::depth`]'s doc for why a `u8` at all. Chosen well above anything
/// a real config would ever ask for (a filesystem doesn't usefully nest this
/// deep either) while leaving headroom under `u8::MAX` for
/// [`crate::menu::PendingFolder::level`] to count one past it without
/// wrapping.
pub const MAX_DEPTH: u8 = 200;

/// See [`Browse::open_folder`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OpenFolder {
    /// Every real submenu (the browsed folder itself, and every subfolder
    /// that still has one of its own per [`Browse::depth`]) gets an extra
    /// "open this folder" entry inside it, in addition to a folder at the
    /// deepest allowed level being shown flattened (see below).
    All,
    /// The default: no extra entry is added to any real submenu. A folder at
    /// the deepest allowed level is still shown, flattened — with no
    /// submenu of its own, it would otherwise be unreachable at all.
    #[default]
    MaxDepth,
    /// No such affordance anywhere, at any level: a folder that has reached
    /// the deepest allowed level (and so cannot get a real submenu) is
    /// simply left out of the listing entirely, rather than shown with no
    /// way to act on it.
    Never,
}

impl ConfigTree {
    /// An empty tree — same defaults as an absent `y-pop.xml`, `warnings`
    /// populated by the caller with whatever kept the real one from loading.
    /// Lets the application start (with nothing to show) instead of failing
    /// outright on a missing or unreadable configuration file.
    pub fn empty(warnings: Vec<String>) -> Self {
        Builder::new().finish(Vec::new(), Defaults::default(), warnings)
    }

    pub fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.index()]
    }

    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> {
        self.node(id).children.iter().copied()
    }

    /// Every node in the tree, roots and descendants alike — and the
    /// search-only groups and folders, which no tray icon reaches — in no
    /// particular order. For a whole-tree walk (firing every `start`/`exit`
    /// event, say) rather than one that cares about menu order.
    pub fn all_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack = self.roots.clone();
        stack.extend(self.search.iter().flat_map(SearchConfig::nodes));
        std::iter::from_fn(move || {
            let id = stack.pop()?;
            stack.extend(self.children(id));
            Some(id)
        })
    }

    /// The chain of nodes from a tray icon's root down to `node`, both
    /// included. `None` for what no tray menu reaches (search-only groups).
    pub fn path_to(&self, node: NodeId) -> Option<Vec<NodeId>> {
        fn walk(tree: &ConfigTree, at: NodeId, target: NodeId, path: &mut Vec<NodeId>) -> bool {
            path.push(at);
            if at == target
                || tree
                    .children(at)
                    .any(|child| walk(tree, child, target, path))
            {
                return true;
            }
            path.pop();
            false
        }
        let mut path = Vec::new();
        self.roots
            .iter()
            .any(|&root| walk(self, root, node, &mut path))
            .then_some(path)
    }

    /// What a right click on the tray icon of `root` adds to its menu, after
    /// the menu itself: **Open Search** if the file has a `<search>`, and
    /// **About** and **Quit** unless the menu already holds one at its top
    /// level (or `root` is one itself): the user placed it where they want
    /// it, and a second copy right next to it would only be noise. One in a
    /// submenu does not count — it is not what a right click would show.
    pub fn tray_extras(&self, root: NodeId) -> TrayExtras {
        let (mut about, mut quit) = (true, true);
        for id in std::iter::once(root).chain(self.children(root)) {
            match self.node(id).kind {
                NodeKind::About => about = false,
                NodeKind::Quit => quit = false,
                _ => {}
            }
        }
        TrayExtras {
            search: self.search.is_some(),
            about,
            quit,
        }
    }

    /// The node a registered hotkey id belongs to, or `None` if the id is
    /// out of range — which should not happen for an id this tree itself
    /// handed out, but a stale id from a previous, since-reloaded tree is
    /// exactly the case worth failing softly on rather than panicking.
    pub fn hotkey_owner(&self, id: u32) -> Option<NodeId> {
        let slot = id.checked_sub(y_core::ids::IDH_FIRST)? as usize;
        self.hotkeys.get(slot).map(|&(node, _)| node)
    }

    /// The search hotkey and the id to register it under: the one after the
    /// last node hotkey, so [`ConfigTree::hotkey_owner`] answers `None` for
    /// it and a `WM_HOTKEY` handler can tell the two apart by asking this
    /// first.
    pub fn search_hotkey(&self) -> Option<(u32, HotkeyBinding)> {
        let binding = self.search.as_ref()?.hotkey?;
        Some((y_core::ids::IDH_FIRST + self.hotkeys.len() as u32, binding))
    }

    /// Every `(id, binding)` pair to register, in the order hotkey ids were
    /// handed out. `id` is what a caller passes to
    /// `y_core::hotkey::HotkeyManager::register` and gets back on
    /// `WM_HOTKEY`; feeding it to [`ConfigTree::hotkey_owner`] recovers the
    /// node.
    pub fn hotkey_bindings(&self) -> impl Iterator<Item = (u32, HotkeyBinding)> {
        self.hotkeys
            .iter()
            .enumerate()
            .map(|(slot, &(_, binding))| (y_core::ids::IDH_FIRST + slot as u32, binding))
    }
}

/// Builds a [`ConfigTree`] node-by-node in document order — used only by
/// [`crate::config`]'s parser, which is the sole place a tree's shape is
/// decided.
pub(crate) struct Builder {
    nodes: Vec<NodeData>,
    hotkeys: Vec<(NodeId, HotkeyBinding)>,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Self {
            nodes: Vec::new(),
            hotkeys: Vec::new(),
        }
    }

    /// Adds one node and returns its id. Each of its `hotkeys` gets its own
    /// sequential id, recorded so [`ConfigTree::hotkey_owner`] can find its
    /// way back here.
    pub(crate) fn push(&mut self, mut data: NodeData) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        for &binding in &data.hotkeys {
            self.hotkeys.push((id, binding));
        }
        // Children are attached by the caller after their own `push` calls,
        // since a node's id has to exist before it can be listed as a
        // parent's child.
        data.children.clear();
        self.nodes.push(data);
        id
    }

    pub(crate) fn set_children(&mut self, parent: NodeId, children: Vec<NodeId>) {
        self.nodes[parent.index()].children = children;
    }

    pub(crate) fn finish(
        self,
        roots: Vec<NodeId>,
        defaults: Defaults,
        warnings: Vec<String>,
    ) -> ConfigTree {
        ConfigTree {
            nodes: self.nodes,
            roots,
            hotkeys: self.hotkeys,
            defaults,
            search: None,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(hotkeys: Vec<HotkeyBinding>) -> NodeData {
        NodeData {
            kind: NodeKind::Quit,
            name: Arc::from("Quit"),
            icon: None,
            tooltip: None,
            keywords: None,
            hotkeys,
            flags: NodeFlags::default(),
            show_shortcut: None,
            popup: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn hotkey_ids_round_trip_to_their_owning_node() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_WIN, VIRTUAL_KEY};
        let hotkey = HotkeyBinding {
            hotkey: Hotkey::new(MOD_WIN, VIRTUAL_KEY(b'Q' as u16)),
            can_override: false,
        };

        let mut builder = Builder::new();
        let a = builder.push(leaf(vec![hotkey]));
        let b = builder.push(leaf(vec![hotkey, hotkey])); // two bindings, one node
        let tree = builder.finish(vec![a, b], Defaults::default(), Vec::new());

        let bindings: Vec<_> = tree.hotkey_bindings().collect();
        assert_eq!(bindings.len(), 3);

        for (id, _) in &bindings[..1] {
            assert_eq!(tree.hotkey_owner(*id), Some(a));
        }
        for (id, _) in &bindings[1..] {
            assert_eq!(tree.hotkey_owner(*id), Some(b));
        }
        assert_eq!(tree.hotkey_owner(0), None); // below IDH_FIRST
    }

    #[test]
    fn children_are_walkable_after_being_attached() {
        let mut builder = Builder::new();
        let child = builder.push(leaf(Vec::new()));
        let parent = builder.push(NodeData {
            kind: NodeKind::Group,
            ..leaf(Vec::new())
        });
        builder.set_children(parent, vec![child]);
        let tree = builder.finish(vec![parent], Defaults::default(), Vec::new());

        assert_eq!(tree.children(parent).collect::<Vec<_>>(), [child]);
    }

    #[test]
    fn all_nodes_reaches_every_descendant() {
        let mut builder = Builder::new();
        let grandchild = builder.push(leaf(Vec::new()));
        let child = builder.push(NodeData {
            kind: NodeKind::Group,
            ..leaf(Vec::new())
        });
        builder.set_children(child, vec![grandchild]);
        let other_root = builder.push(leaf(Vec::new()));
        let tree = builder.finish(vec![child, other_root], Defaults::default(), Vec::new());

        let mut all: Vec<_> = tree.all_nodes().collect();
        all.sort_by_key(|id| id.as_u32());
        let mut expected = [child, grandchild, other_root];
        expected.sort_by_key(|id| id.as_u32());
        assert_eq!(all, expected);
    }
}
