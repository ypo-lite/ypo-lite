//! The native `HMENU` popup, built straight from the `ConfigTree` — icons
//! (`MIIM_BITMAP`/owner-drawn, depending on theme) and, per `<y-pop
//! theme="...">`, owner-draw colour/background theming via
//! `y_core::menu::PopupMenu::themed`. No `MenuLayout` indirection yet. See
//! `docs/y-pop-plan.md`'s phased roadmap: this exists to prove the tree,
//! tray icons, hotkeys and launcher work end to end before the overlay
//! renderer is built on top of a shared `MenuLayout` step.
//!
//! # Theme
//!
//! [`y_pop::skins::resolve_design`] turns the config's `Theme` into
//! `Option<MenuDesign>` — `None` for `Theme::Native` (plain OS chrome,
//! `PopupMenu::new()`), `Some` for `Default`/`Flat`/`Dark`/`Auto`
//! (owner-drawn, `PopupMenu::themed`). Resolved once per popup open, in
//! [`build`], and carried on [`Session`] from there so every submenu —
//! built up front in [`append_children`] or lazily in
//! [`on_init_menu_popup`] — themes consistently with the root. The skin
//! tables and the resolution logic itself live in `crate::skins`, not here
//! — see that module's own doc comment.
//!
//! # Icons
//!
//! A menu wants a bitmap (`hbmpItem`), not an icon — [`y_core::icon::Icon::
//! to_bitmap`] does that conversion, and [`Session`] is what keeps the
//! resulting [`y_core::icon::Bitmap`]s alive: Windows does not free a
//! custom `hbmpItem` when the item or menu holding it is destroyed, so
//! something has to own them for exactly as long as the popup is open, and
//! stop owning them (dropping, and so destroying, every one) the moment it
//! closes — the same lifetime `Session`'s browse bookkeeping already has,
//! which is why they live together. [`resolve_icon`] is `node.icon` if the
//! config gave one, else — for an `App` only — the icon its own `exe`
//! carries; nothing else gets a default. A leaf item's icon is set by
//! command id (it already has one); a submenu's (a `Group`, or a browsed
//! subfolder) has no command id — the value doubling as one in
//! `AppendMenuW`'s `MF_POPUP` call is its submenu's own handle — so that
//! one is set by position instead, immediately after the item is added.
//!
//! # Folder browsing is lazy, per submenu
//!
//! [`build`] does *not* list a `browse="true"` folder — it creates an empty
//! submenu for it and records the mapping in the returned [`Session`].
//! Listing happens in [`on_init_menu_popup`], called from `main`'s
//! `WM_INITMENUPOPUP` handler the moment Windows is about to show that
//! specific submenu — not when the popup containing it first opens, and not
//! for a submenu the user never navigates into at all. This is PortAL's own
//! trick (`menu.cpp`'s `OnInitMenuPopup`/`HandleSubDirectory`, confirmed by
//! reading them), not a guess: a group with several browsable folders would
//! otherwise list every one of them — network share, removable disk,
//! whatever's slow, all of it — just to open the group at all.
//!
//! `WM_INITMENUPOPUP` arrives while `PopupMenu::track` is still blocked on
//! the stack (`TrackPopupMenu` runs its own nested message loop, and that
//! loop dispatches messages for our window like any other — the same fact
//! the async-icon-refresh design in `docs/y-pop-plan.md` already leans on).
//! That reentrant call needs to reach the same [`Session`] the blocked call
//! built, so it lives in `YPop::session`, not a local in `YPop::show_menu` —
//! see that method.
//!
//! # Tooltips
//!
//! A native menu has no per-item window to hang a tooltip control off, so
//! `main` watches `WM_MENUSELECT` instead: [`tooltip_for`] turns what it
//! names into the item's `tooltip=` text and on-screen rectangle (recorded
//! in [`Session`] as items are appended), and `crate::tooltip` shows it after
//! a hover delay. Only tree nodes have tooltips — files `browse` discovers
//! don't, there being no config to write one in.
//!
//! # Two id spaces, one command range each
//!
//! A selection out of the tree (`app`, `about`, `quit`) uses
//! `IDM_FIRST + NodeId`, same as before. A selection out of a *browsed*
//! folder has no `NodeId` — the file was never in `y-pop.xml` — so it gets
//! an id from a separate range starting at [`DISCOVERED_ID_BASE`], far
//! above anything a real config tree will ever have that many nodes to
//! reach. [`Session::discovered`] maps that id straight to a ready-to-launch
//! [`App`], built once, right when the folder is listed, from the *browse*
//! node's own `shell`/`elevate` — a browsed PDF opened via `ShellExecuteExW`
//! needs that inherited, not defaulted, the same as PortAL's
//! `prog->options = p_menu->options` on every discovered file.

use std::collections::HashMap;
use std::sync::Arc;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::WindowsAndMessaging::HMENU;
use windows::core::Result;

use y_core::icon::{Bitmap, Icon, IconCache, StockIcon};
use y_core::ids::IDM_FIRST;
use y_core::menu::{self, MenuDesign, MenuSelect, PopupMenu};

use y_pop::model::{
    App, Browse, ConfigTree, Defaults, EventMask, NodeData, NodeId, NodeKind, OpenFolder,
    TrayExtras,
};

use y_pop::skins::resolve_design;

/// First id handed to a discovered (browsed) file — see the module doc.
const DISCOVERED_ID_BASE: u32 = 1_000_000;

/// Command ids of the entries a right click adds ([`build_with`]): just
/// below the discovered files', far above any node's `IDM_FIRST + index`.
const EXTRA_SEARCH: u32 = DISCOVERED_ID_BASE - 3;
const EXTRA_ABOUT: u32 = DISCOVERED_ID_BASE - 2;
const EXTRA_QUIT: u32 = DISCOVERED_ID_BASE - 1;

/// One of the built-in entries of a tray icon's right-click menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extra {
    Search,
    About,
    Quit,
}

/// Extra runtime state for one open popup — the parts a `ConfigTree` alone
/// can't answer, because they only exist once a `browse` folder is actually
/// listed, or an icon actually loaded. Built alongside the [`PopupMenu`] in
/// [`build`], consulted (and grown) by [`on_init_menu_popup`] while the
/// popup is open, and dropped once it closes — none of it needs to, or
/// should, outlive one open.
pub struct Session {
    /// Raw submenu handle (as `usize`, since `HMENU` isn't `Hash`) → what to
    /// list in it, once asked.
    pending: HashMap<usize, PendingFolder>,
    discovered: HashMap<u32, App>,
    next_discovered_id: u32,
    icons: IconCache,
    /// Every bitmap handed to a menu item's `hbmpItem` this open — see the
    /// module doc's "Icons" section for why these live exactly as long as
    /// `Session` does and no longer.
    bitmaps: Vec<Bitmap>,
    /// Resolved once in [`build`] — see the module doc's "Theme" section.
    /// Every submenu, whether built up front by [`append_children`] or
    /// lazily by [`on_init_menu_popup`], reads this rather than re-resolving
    /// [`Theme`] itself, so a whole popup always themes consistently.
    design: Option<MenuDesign>,
    /// Every item that has a `tooltip=` — see the module doc's "Tooltips"
    /// section. Only items that have one are recorded, so a config without
    /// tooltips costs nothing here.
    tips: HashMap<TipKey, Arc<str>>,
}

/// How `WM_MENUSELECT` names an item: a leaf by command id, a submenu item
/// by its position in its parent (it has no command id).
#[derive(PartialEq, Eq, Hash)]
enum TipKey {
    Command(u32),
    Submenu(usize, u32),
}

/// A tooltip to show: its text and the on-screen rectangle of the row it
/// belongs to.
pub struct Tip {
    pub text: Arc<str>,
    pub row: RECT,
}

struct PendingFolder {
    dir: String,
    browse: Browse,
    /// Inherited by every file (and, recursively, every subfolder) this
    /// folder turns out to contain — see the module doc's second section.
    shell: Option<bool>,
    elevate: bool,
    /// This folder's own distance from the browse app's root — `0` for the
    /// root itself. Threaded through to [`y_pop::browse::list`] so it can
    /// tell whether *its* children have room left under `browse.depth` to
    /// become their own submenus, or have reached the deepest allowed level
    /// and should be flattened instead — see `Browse::depth`. `u8`, matching
    /// `Browse::depth` itself: `level` only ever climbs one past a config's
    /// own (already `MAX_DEPTH`-clamped) `depth`, so it never comes close to
    /// wrapping.
    level: u8,
}

impl Session {
    pub(crate) fn new() -> Self {
        Self::with_design(None)
    }

    fn with_design(design: Option<MenuDesign>) -> Self {
        Self {
            pending: HashMap::new(),
            discovered: HashMap::new(),
            next_discovered_id: DISCOVERED_ID_BASE,
            icons: IconCache::new(),
            bitmaps: Vec::new(),
            design,
            tips: HashMap::new(),
        }
    }

    /// The design this popup is painted with — `None` for `theme="native"`.
    /// A tooltip borrows its colours from it.
    pub fn design(&self) -> Option<MenuDesign> {
        self.design
    }

    fn add_tip(&mut self, key: TipKey, node: &NodeData) {
        if let Some(text) = &node.tooltip {
            self.tips.insert(key, Arc::clone(text));
        }
    }

    fn next_discovered_id(&mut self) -> u32 {
        let id = self.next_discovered_id;
        self.next_discovered_id += 1;
        id
    }

    /// Converts `icon` to a bitmap and remembers it (see the module doc's
    /// "Icons" section for why), then hands the raw handle to `set` to
    /// actually attach it — by command id or by position, whichever the
    /// caller's item needs. A no-op, not an error, if `icon` is `None` or
    /// the conversion fails: a menu item with no icon is a normal item, not
    /// a broken one.
    fn attach_icon(&mut self, icon: Option<Icon>, set: impl FnOnce(HBITMAP) -> Result<()>) {
        let Some(icon) = icon else { return };
        let Some(bitmap) = icon.to_bitmap(menu::ICON_SIZE) else {
            return;
        };
        let _ = set(bitmap.handle());
        self.bitmaps.push(bitmap);
    }
}

/// `node.icon` if the config gave one (a file, or a built-in `@name` — see
/// `y_pop::internal_icons`), else a default depending on kind:
/// the generic folder icon for a `Group` (PortAL's own default — confirmed
/// against a screenshot of PortAL's stock menu, not assumed — every
/// submenu shows a folder unless its config gives it something else), the
/// icon its own `exe` carries for an `App`, a shell stock icon for `About`
/// (info), `Quit` (red cross) and `Settings` (cog) — a divergence from
/// PortAL, which left those bare; `icon=` on the node still overrides, and
/// `icon="@app"` on a Store app asks for its own logo. A titled `Separator`
/// gets none.
pub(crate) fn resolve_icon(icons: &mut IconCache, node: &NodeData) -> Option<Icon> {
    if let Some(internal) = node
        .icon
        .as_ref()
        .and_then(|source| y_pop::internal_icons::find(&source.path))
    {
        return internal.load(icons);
    }
    // `@app` is the Store app's own logo; on anything else it is as if no
    // icon had been given.
    let given = node.icon.as_ref().filter(|source| !source.is_app_logo());
    if let Some(source) = given {
        return icons.for_file(&source.path, source.index);
    }
    match &node.kind {
        NodeKind::App(app) if node.icon.is_some() && app.app_id.is_some() => {
            icons.for_store_app(app.app_id.as_deref()?)
        }
        NodeKind::Group => icons.for_directory(),
        // A browse app's own "exe" names a folder, not a file — the
        // generic folder icon, not whatever `for_file` would guess from a
        // directory name that happens to contain a '.'.
        NodeKind::App(app) if app.browse.is_some() => icons.for_directory(),
        NodeKind::App(app) => icons.for_file(app.exe.as_deref()?, None),
        NodeKind::About => icons.for_stock(StockIcon::Info),
        NodeKind::Quit => icons.for_stock(StockIcon::Delete),
        NodeKind::Settings => icons.for_stock(StockIcon::Settings),
        _ => None,
    }
}

/// The right-aligned shortcut hint for `node`'s own menu row — `None` if
/// `show-shortcuts` (this node's own override, else [`Defaults::
/// show_shortcuts`]) is off, or the node has no hotkey to show at all.
/// Applies to any node kind, not just a leaf `App` — a `<group>` (or
/// `browse` folder) opened by its own `<hotkey>` gets a shortcut column too,
/// matching PortAL's own `MENU_SUB | MENU_SHOWSC` measure-time check
/// (`menu.cpp::OnMeasureItem`, confirmed by reading it). Only the first
/// hotkey is shown for a node bound to more than one — `y-pop.xml` in
/// practice never gives one node two (PortAL's own config didn't either),
/// and there is no natural way to show more than one hint in one column
/// anyway.
pub(crate) fn shortcut_text(defaults: &Defaults, node: &NodeData) -> Option<String> {
    if !node.show_shortcut.unwrap_or(defaults.show_shortcuts) {
        return None;
    }
    node.hotkeys
        .first()
        .map(|binding| binding.hotkey.to_string())
}

/// What a native menu's `track()` result turned out to mean.
pub enum Selection {
    /// A real node — walk `ConfigTree` from here, same as before browsing
    /// existed.
    Node(NodeId),
    /// A file discovered while browsing — nothing to look up, launch it.
    Launch(App),
    /// A built-in entry of the right-click menu.
    Extra(Extra),
}

/// Recovers what a native menu's `track()` returned, checking `session`'s
/// discovered files first (their ids are unambiguous — see the module doc —
/// so this never mistakes one for a stale/foreign command id and vice
/// versa). `None` for an id neither space recognises, which should not
/// happen for a selection this popup itself produced, but a stray
/// `WM_COMMAND` from somewhere else is exactly the case worth failing
/// softly on rather than panicking.
pub fn resolve_selection(id: u32, session: &mut Session) -> Option<Selection> {
    match id {
        EXTRA_SEARCH => return Some(Selection::Extra(Extra::Search)),
        EXTRA_ABOUT => return Some(Selection::Extra(Extra::About)),
        EXTRA_QUIT => return Some(Selection::Extra(Extra::Quit)),
        _ => {}
    }
    if let Some(app) = session.discovered.remove(&id) {
        return Some(Selection::Launch(app));
    }
    id.checked_sub(IDM_FIRST)
        .map(NodeId::from_u32)
        .map(Selection::Node)
}

/// A key press that walks an open tray menu toward an item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// Highlight the next item.
    Down,
    /// Open the highlighted submenu (which highlights its first item).
    Right,
}

/// Where **Reveal in menu** goes: the tray icon whose menu holds `node`, and
/// the keys that walk that menu down to it (task 5.6). `None` for what no tray
/// menu reaches.
///
/// `TrackPopupMenu` cannot preselect a row, but a menu answers the keyboard:
/// the first `Down` highlights the first item, `Right` opens a submenu and
/// highlights its first item. So the steps are, for each level, `Down` once for
/// each item above the target, then `Right` to go in. They are plain counting,
/// so they hold only while the menu is what [`build_with`] makes: separators
/// are skipped by the keyboard (a titled one in a native menu is a greyed item,
/// which is not), and breaks add nothing.
pub fn reveal_steps(tree: &ConfigTree, node: NodeId) -> Option<(NodeId, Vec<Step>)> {
    let path = tree.path_to(node)?;
    let themed = resolve_design(tree.defaults.theme, tree.defaults.theme_overrides).is_some();

    let mut steps = Vec::new();
    for (depth, pair) in path.windows(2).enumerate() {
        let [parent, child] = [pair[0], pair[1]];
        let above = tree
            .children(parent)
            .take_while(|&sibling| sibling != child)
            .filter(|&sibling| match tree.node(sibling).kind {
                NodeKind::Break => false,
                NodeKind::Separator => !themed && !tree.node(sibling).name.is_empty(),
                _ => true,
            })
            .count();
        // Nothing is highlighted yet in the first menu: one more to get on it.
        let downs = above + usize::from(depth == 0);
        steps.extend(std::iter::repeat_n(Step::Down, downs));
        if depth + 2 < path.len() {
            steps.push(Step::Right);
        }
    }
    Some((path[0], steps))
}

/// Builds a popup for `group`'s children, with nothing added — what the
/// tests that are not about the right-click menu build.
#[cfg(test)]
pub fn build(tree: &ConfigTree, group: NodeId) -> (PopupMenu, Session) {
    build_with(tree, Some(group), None)
}

/// [`build`], for a tray icon's right click: `group`'s children (none, for an
/// icon that is not a group) followed by `extras` — **Open Search**, **About**
/// and **Quit**, whichever [`ConfigTree::tray_extras`] says are wanted — set
/// apart by a rule when there is anything above them.
pub fn build_with(
    tree: &ConfigTree,
    group: Option<NodeId>,
    extras: Option<TrayExtras>,
) -> (PopupMenu, Session) {
    let design = resolve_design(tree.defaults.theme, tree.defaults.theme_overrides);
    let mut menu = PopupMenu::new_maybe_themed(design).expect("CreatePopupMenu");
    let mut session = Session::with_design(design);
    if let Some(group) = group {
        append_children(tree, group, &mut menu, &mut session);
    }
    if let Some(extras) = extras {
        append_extras(tree, extras, &mut menu, &mut session);
    }
    (menu, session)
}

fn append_extras(
    tree: &ConfigTree,
    extras: TrayExtras,
    menu: &mut PopupMenu,
    session: &mut Session,
) {
    if !(extras.search || extras.about || extras.quit) {
        return;
    }
    if menu.item_count() > 0 {
        let _ = menu.add_titled_separator("");
    }

    let mut add = |id: u32, label: &str, stock: StockIcon, shortcut: Option<&str>| {
        let icon = session.icons.for_stock(stock);
        let _ = menu.add_item_with_shortcut(id, label, shortcut);
        session.attach_icon(icon, |bitmap| menu.set_item_icon(id, bitmap));
    };
    if extras.search {
        // The hotkey that opens it, as any other entry shows its own.
        let hotkey = tree
            .search_hotkey()
            .filter(|_| tree.defaults.show_shortcuts)
            .map(|(_, binding)| binding.hotkey.to_string());
        add(
            EXTRA_SEARCH,
            "Open Search",
            StockIcon::Find,
            hotkey.as_deref(),
        );
    }
    if extras.about {
        add(EXTRA_ABOUT, "About", StockIcon::Info, None);
    }
    if extras.quit {
        add(EXTRA_QUIT, "Quit", StockIcon::Delete, None);
    }
}

fn append_children(tree: &ConfigTree, parent: NodeId, menu: &mut PopupMenu, session: &mut Session) {
    for child in tree.children(parent) {
        let node = tree.node(child);

        match &node.kind {
            NodeKind::Group => {
                let mut submenu =
                    PopupMenu::new_maybe_themed(session.design).expect("CreatePopupMenu");
                append_children(tree, child, &mut submenu, session);

                let icon = resolve_icon(&mut session.icons, node);
                let shortcut = shortcut_text(&tree.defaults, node);
                let _ = menu.add_submenu_with_shortcut(&node.name, submenu, shortcut.as_deref());
                let position = menu.item_count().saturating_sub(1);
                session.add_tip(TipKey::Submenu(menu.handle().0 as usize, position), node);
                session.attach_icon(icon, |bitmap| {
                    menu.set_item_icon_by_position(position, bitmap)
                });
            }
            NodeKind::App(app) if app.browse.is_some() => {
                let submenu = PopupMenu::new_maybe_themed(session.design).expect("CreatePopupMenu");
                session.pending.insert(
                    submenu.handle().0 as usize,
                    PendingFolder {
                        dir: app.exe.as_deref().unwrap_or_default().to_owned(),
                        browse: app.browse.clone().expect("checked by this arm's guard"),
                        shell: app.shell,
                        elevate: app.elevate,
                        level: 0,
                    },
                );

                let icon = resolve_icon(&mut session.icons, node);
                let shortcut = shortcut_text(&tree.defaults, node);
                let _ = menu.add_submenu_with_shortcut(&node.name, submenu, shortcut.as_deref());
                let position = menu.item_count().saturating_sub(1);
                session.add_tip(TipKey::Submenu(menu.handle().0 as usize, position), node);
                session.attach_icon(icon, |bitmap| {
                    menu.set_item_icon_by_position(position, bitmap)
                });
            }
            // Draws as a thin rule when untitled, its own title text when
            // titled — themed or not, `add_titled_separator` picks the
            // right rendering for each (see the module doc's "Theme"
            // section and `y_core::menu::PopupMenu::add_titled_separator`).
            NodeKind::Separator => {
                let _ = menu.add_titled_separator(&node.name);
            }
            // Appends nothing of its own — arms `menu`'s next append (the
            // following sibling, or nothing at all if this was the last
            // child) to start a new column. See `NodeKind::Break`'s own doc
            // comment and `y_core::menu::PopupMenu::add_break`.
            NodeKind::Break => menu.add_break(),
            NodeKind::App(_) | NodeKind::About | NodeKind::Quit | NodeKind::Settings => {
                let id = IDM_FIRST + child.as_u32();
                let icon = resolve_icon(&mut session.icons, node);
                let shortcut = shortcut_text(&tree.defaults, node);
                let _ = menu.add_item_with_shortcut(id, &node.name, shortcut.as_deref());
                session.add_tip(TipKey::Command(id), node);
                session.attach_icon(icon, |bitmap| menu.set_item_icon(id, bitmap));
            }
        }
    }
}

/// The tooltip for what `WM_MENUSELECT` just highlighted, if that item has
/// one and is on screen. Items listed by `browse` never do.
pub fn tooltip_for(session: &Session, select: MenuSelect) -> Option<Tip> {
    let (key, menu, position) = match select {
        MenuSelect::Command { menu, id } => {
            (TipKey::Command(id), menu, menu::position_of(menu, id)?)
        }
        MenuSelect::Submenu { menu, position } => {
            (TipKey::Submenu(menu.0 as usize, position), menu, position)
        }
        MenuSelect::Closed | MenuSelect::Other => return None,
    };
    let text = Arc::clone(session.tips.get(&key)?);
    let row = menu::item_rect(menu, position)?;
    Some(Tip { text, row })
}

/// Populates `hmenu` the moment it is about to be shown — call from `main`'s
/// `WM_INITMENUPOPUP` handler with the message's `wParam` cast to an
/// `HMENU`. A no-op for a handle not in `session`'s pending table: every
/// ordinary submenu already has its items from [`append_children`], and a
/// `browse` submenu already listed once (Windows can send
/// `WM_INITMENUPOPUP` again for a menu the user is merely revisiting with
/// the keyboard) is removed from that table the first time, so a repeat
/// call finds nothing to do and leaves what's already there alone.
pub fn on_init_menu_popup(session: &mut Session, hmenu: HMENU) {
    let Some(pending) = session.pending.remove(&(hmenu.0 as usize)) else {
        return;
    };

    // `open-folder="all"`: every real submenu — this one included — gets an
    // extra entry up front to open the folder it lists directly, set apart
    // from the listing itself by a separator. Every `pending` this function
    // is ever called for is, by construction, a real submenu (a folder shown
    // flattened per `Browse::depth` never gets registered as one — see the
    // branch below), so this check needs no level comparison of its own.
    if pending.browse.open_folder == OpenFolder::All {
        let id = session.next_discovered_id();
        let _ = menu::append_item(hmenu, id, "Open folder", session.design);
        session.discovered.insert(id, open_folder_app(&pending.dir));
        let _ = menu::append_separator(hmenu, session.design);
    }

    let entries = y_pop::browse::list(&pending.dir, &pending.browse, pending.level);
    if entries.is_empty() {
        let _ = menu::append_disabled_item(hmenu, "(empty)", session.design);
        return;
    }

    for entry in entries {
        // A file called "R&D" is not a menu mnemonic.
        let label = entry.name.replace('&', "&&");
        if entry.is_dir {
            let icon = session.icons.for_directory();
            if entry.browsable {
                let Ok(submenu) = menu::append_submenu(hmenu, &label, session.design) else {
                    continue;
                };
                let position = menu::item_count(hmenu).saturating_sub(1);
                session.attach_icon(icon, |bitmap| {
                    menu::set_item_icon_by_position(hmenu, position, bitmap)
                });

                session.pending.insert(
                    submenu.0 as usize,
                    PendingFolder {
                        dir: entry.path,
                        browse: pending.browse.clone(),
                        shell: pending.shell,
                        elevate: pending.elevate,
                        level: pending.level + 1,
                    },
                );
            } else {
                // Reached `Browse::depth` — shown flattened: a plain item,
                // no submenu, that opens the folder itself instead of
                // listing further.
                let id = session.next_discovered_id();
                let _ = menu::append_item(hmenu, id, &label, session.design);
                session.attach_icon(icon, |bitmap| menu::set_item_icon(hmenu, id, bitmap));
                session.discovered.insert(id, open_folder_app(&entry.path));
            }
        } else {
            let icon = session.icons.for_file(&entry.path, None);
            let id = session.next_discovered_id();
            let _ = menu::append_item(hmenu, id, &label, session.design);
            session.attach_icon(icon, |bitmap| menu::set_item_icon(hmenu, id, bitmap));

            session.discovered.insert(
                id,
                App {
                    exe: Some(Arc::from(entry.path.as_str())),
                    app_id: None,
                    param: None,
                    path: None,
                    shell: pending.shell,
                    elevate: pending.elevate,
                    events: EventMask::default(),
                    browse: None,
                },
            );
        }
    }
}

/// An `App` that opens `dir` itself — via `ShellExecuteExW`'s default
/// "open" verb on a directory, which is Explorer, the same way double-
/// clicking a folder there does — for a flattened max-depth folder item or
/// an `open-folder="all"` entry. Always `shell: true`: `CreateProcessW`
/// can't launch a directory at all, so this can't inherit the browse node's
/// own (possibly `false`) `shell` the way a discovered *file* does. Always
/// `elevate: false` too, deliberately not inherited either — opening a
/// folder in Explorer never needs elevation, and eagerly asking for it here
/// would be a UAC prompt for something that doesn't need one.
fn open_folder_app(dir: &str) -> App {
    App::open_folder(dir)
}

// `on_init_menu_popup` above uses `y_core::menu::append_item`/
// `append_disabled_item`/`append_submenu` directly rather than
// `PopupMenu`'s own methods: the submenu being populated there was already
// handed to its parent through `PopupMenu::add_submenu`, which *moves* the
// wrapper in — by the time `WM_INITMENUPOPUP` names that submenu's handle,
// no `&mut PopupMenu` for it exists anywhere to add through, only the raw
// `HMENU` the message carries. Safe to build on regardless: Windows
// destroys a submenu with whatever parent menu owns it, recursively, so
// anything appended here is still cleaned up correctly (owner-drawn items'
// `ItemData` included — see `y_core::menu`'s module doc) when the root
// `PopupMenu` (the one thing that actually owns a `Drop` impl) is dropped.
// `session.design` is threaded through so a browsed folder's own rows theme
// the same as the rest of the popup — see this module's "Theme" section.

/// These exercise the real thing: `CreatePopupMenu`/`AppendMenuW` and the
/// `HMENU` round trip through a `usize`, not a mock of them — nothing here
/// shows a window or touches input, so it's exactly as safe to run
/// unattended as any other unit test. What it can't reach is whether
/// `WM_INITMENUPOPUP` actually arrives at `main`'s handler in a live popup —
/// that's Win32 message delivery, outside what a unit test can drive; this
/// suite exists to rule everything *else* out first.
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use windows::Win32::UI::WindowsAndMessaging::{
        GetMenuItemCount, GetMenuItemID, GetMenuItemInfoW, GetMenuStringW, MENUITEMINFOW,
        MF_BYPOSITION, MFT_MENUBREAK, MIIM_BITMAP, MIIM_FTYPE,
    };

    use y_pop::config;
    use y_pop::model::ConfigTree;
    use y_pop::variables::Variable;

    use super::*;

    /// Same reasoning as `y_core::icon`'s own test module: the shell/COM
    /// machinery behind icon extraction is not safe to call concurrently
    /// from multiple uninitialized threads, which is exactly how `cargo
    /// test`'s default runner gives each test its own. `build` and
    /// `on_init_menu_popup` both resolve icons unconditionally now, not
    /// just in the tests this module named for it, so every test that calls
    /// either takes this — confirmed necessary, not just cautious: an
    /// earlier, narrower version of this lock (icon-specific tests only)
    /// still saw one of *this* module's non-icon tests interfere.
    static SHELL_CALLS: Mutex<()> = Mutex::new(());

    fn serialized() -> std::sync::MutexGuard<'static, ()> {
        SHELL_CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A throwaway directory under the system temp folder, removed on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("y-pop-menu-test-{name}"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> String {
            self.0
                .to_str()
                .expect("temp dir path is valid UTF-8")
                .to_owned()
        }

        fn touch(&self, name: &str) {
            std::fs::write(self.0.join(name), b"").expect("create file");
        }

        fn mkdir(&self, name: &str) {
            std::fs::create_dir(self.0.join(name)).expect("create subdir");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A tree with one root `<group>` containing one `browse="true"` `<app>`
    /// pointing at `dir` — the shape `build` needs to reach the lazy path.
    fn tree_with_browse(dir: &str, extra_attrs: &str) -> (ConfigTree, NodeId) {
        let xml = format!(
            r#"<y-pop><group name="Root"><app name="Files" exe="{dir}" browse="true" {extra_attrs}/></group></y-pop>"#
        );
        let tree = config::load_str(&xml, &mut Vec::<Variable>::new()).expect("valid XML");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let root = tree.roots[0];
        (tree, root)
    }

    /// A tree with one root `<group>` containing one plain `<app>` — the
    /// shape the icon tests need, distinct from `tree_with_browse`'s.
    fn tree_with_app(exe: &str) -> (ConfigTree, NodeId) {
        let xml =
            format!(r#"<y-pop><group name="Root"><app name="App" exe="{exe}"/></group></y-pop>"#);
        let tree = config::load_str(&xml, &mut Vec::<Variable>::new()).expect("valid XML");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let root = tree.roots[0];
        (tree, root)
    }

    fn item_count(menu: HMENU) -> i32 {
        unsafe { GetMenuItemCount(Some(menu)) }
    }

    fn item_text(menu: HMENU, index: u32) -> String {
        let mut buffer = [0u16; 256];
        let len = unsafe { GetMenuStringW(menu, index, Some(&mut buffer), MF_BYPOSITION) };
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    fn tree_from(xml: &str) -> ConfigTree {
        let tree = config::load_str(xml, &mut Vec::<Variable>::new()).expect("valid XML");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        tree
    }

    fn all_items(menu: HMENU) -> Vec<String> {
        (0..item_count(menu) as u32)
            .map(|index| item_text(menu, index))
            .collect()
    }

    #[test]
    fn a_right_click_menu_is_the_menu_then_a_rule_then_search_about_and_quit() {
        let _guard = serialized();
        let tree = tree_from(
            r#"<y-pop><search/><group name="Work"><app name="A" exe="a.exe"/></group></y-pop>"#,
        );
        let root = tree.roots[0];
        let extras = tree.tray_extras(root);

        let (popup, mut session) = build_with(&tree, Some(root), Some(extras));
        let texts = all_items(popup.handle());
        assert_eq!(texts.len(), 5, "{texts:?}");
        assert_eq!(texts[0], "A");
        assert_eq!(texts[1], "", "the rule");
        assert_eq!(texts[2], "Open Search");
        assert_eq!(texts[3], "About");
        assert_eq!(texts[4], "Quit");

        // And each one resolves to what it says.
        for (position, expected) in [(2, Extra::Search), (3, Extra::About), (4, Extra::Quit)] {
            let id = unsafe { GetMenuItemID(popup.handle(), position) };
            assert!(
                matches!(
                    resolve_selection(id, &mut session),
                    Some(Selection::Extra(found)) if found == expected
                ),
                "item {position}"
            );
        }
    }

    #[test]
    fn what_the_menu_already_holds_is_not_added_again() {
        let _guard = serialized();
        let tree = tree_from(
            r#"<y-pop><group name="Work"><app name="A" exe="a.exe"/><quit/></group></y-pop>"#,
        );
        let root = tree.roots[0];
        let (popup, _session) = build_with(&tree, Some(root), Some(tree.tray_extras(root)));
        // Its own Quit, then the rule, then About — no second Quit and, with
        // no <search>, no Open Search.
        let texts = all_items(popup.handle());
        assert_eq!(texts, ["A", "Quit", "", "About"], "{texts:?}");
    }

    #[test]
    fn an_icon_with_no_menu_gets_the_extras_without_a_rule() {
        let _guard = serialized();
        let tree = tree_from(r#"<y-pop><search/><app name="A" exe="a.exe"/></y-pop>"#);
        let root = tree.roots[0];
        let (popup, _session) = build_with(&tree, None, Some(tree.tray_extras(root)));
        let texts = all_items(popup.handle());
        assert_eq!(texts, ["Open Search", "About", "Quit"], "{texts:?}");
    }

    #[test]
    fn without_extras_nothing_is_added() {
        let _guard = serialized();
        let tree =
            tree_from(r#"<y-pop><group name="Work"><app name="A" exe="a.exe"/></group></y-pop>"#);
        let (popup, _session) = build_with(&tree, Some(tree.roots[0]), None);
        assert_eq!(all_items(popup.handle()), ["A"]);
    }

    #[test]
    fn the_search_entry_shows_the_search_hotkey() {
        let _guard = serialized();
        let tree = tree_from(
            r#"<y-pop><search hotkey="Alt+Space"/><group name="Work"><app name="A" exe="a.exe"/></group></y-pop>"#,
        );
        let root = tree.roots[0];
        let (popup, _session) = build_with(&tree, Some(root), Some(tree.tray_extras(root)));
        let texts = all_items(popup.handle());
        assert!(
            texts[2].starts_with("Open Search") && texts[2].contains("Alt+Space"),
            "{texts:?}"
        );
    }

    fn item_has_icon(menu: HMENU, id: u32) -> bool {
        has_icon(menu, id, false)
    }

    fn item_has_icon_by_position(menu: HMENU, position: u32) -> bool {
        has_icon(menu, position, true)
    }

    fn has_icon(menu: HMENU, item: u32, by_position: bool) -> bool {
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_BITMAP;
        let ok = unsafe { GetMenuItemInfoW(menu, item, by_position, &mut info) };
        ok.is_ok() && !info.hbmpItem.is_invalid()
    }

    /// Whether the item at `position` starts a new menu column —
    /// `MFT_MENUBREAK`, what [`NodeKind::Break`] turns into.
    fn item_starts_new_column(menu: HMENU, position: u32) -> bool {
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_FTYPE;
        let ok = unsafe { GetMenuItemInfoW(menu, position, true, &mut info) };
        ok.is_ok() && info.fType.contains(MFT_MENUBREAK)
    }

    /// The one `browse` submenu's raw handle — panics if `session` doesn't
    /// have exactly one pending entry, which every test here arranges for.
    fn only_pending_handle(session: &Session) -> HMENU {
        assert_eq!(
            session.pending.len(),
            1,
            "expected exactly one pending folder"
        );
        let &raw = session.pending.keys().next().unwrap();
        HMENU(raw as *mut _)
    }

    #[test]
    fn a_browse_app_gets_an_empty_placeholder_submenu_up_front() {
        let _guard = serialized();
        let dir = TempDir::new("empty-up-front");
        dir.touch("file.txt");
        let (tree, root) = tree_with_browse(&dir.path(), "");

        let (_popup, session) = build(&tree, root);
        let submenu = only_pending_handle(&session);
        // Nothing listed yet — the whole point of laziness (see the module
        // doc): build() must not have touched the filesystem at all.
        assert_eq!(item_count(submenu), 0);
    }

    #[test]
    fn opening_the_submenu_lists_its_folder_alphabetically() {
        let _guard = serialized();
        let dir = TempDir::new("lists-folder");
        dir.touch("b.txt");
        dir.touch("a.txt");
        let (tree, root) = tree_with_browse(&dir.path(), "");

        let (_popup, mut session) = build(&tree, root);
        let submenu = only_pending_handle(&session);

        on_init_menu_popup(&mut session, submenu);

        assert_eq!(item_count(submenu), 2);
        assert_eq!(item_text(submenu, 0), "a.txt");
        assert_eq!(item_text(submenu, 1), "b.txt");
        assert!(session.pending.is_empty(), "consumed once populated");
    }

    #[test]
    fn a_second_open_does_not_duplicate_items() {
        let _guard = serialized();
        let dir = TempDir::new("no-duplicate");
        dir.touch("only.txt");
        let (tree, root) = tree_with_browse(&dir.path(), "");

        let (_popup, mut session) = build(&tree, root);
        let submenu = only_pending_handle(&session);

        on_init_menu_popup(&mut session, submenu);
        on_init_menu_popup(&mut session, submenu); // Windows can re-send this

        assert_eq!(item_count(submenu), 1);
    }

    #[test]
    fn an_empty_folder_shows_a_disabled_placeholder() {
        let _guard = serialized();
        let dir = TempDir::new("truly-empty");
        let (tree, root) = tree_with_browse(&dir.path(), "");

        let (_popup, mut session) = build(&tree, root);
        let submenu = only_pending_handle(&session);
        on_init_menu_popup(&mut session, submenu);

        assert_eq!(item_count(submenu), 1);
        assert_eq!(item_text(submenu, 0), "(empty)");
    }

    #[test]
    fn selecting_a_discovered_file_resolves_to_a_launchable_app() {
        let _guard = serialized();
        let dir = TempDir::new("resolve-selection");
        dir.touch("notes.txt");
        let (tree, root) = tree_with_browse(&dir.path(), r#"shell="true""#);

        let (_popup, mut session) = build(&tree, root);
        let submenu = only_pending_handle(&session);
        on_init_menu_popup(&mut session, submenu);

        let id = unsafe { GetMenuItemID(submenu, 0) };
        assert!(
            id >= DISCOVERED_ID_BASE,
            "a discovered file's id must come from its own range, not IDM_FIRST + NodeId"
        );

        match resolve_selection(id, &mut session).expect("a known id") {
            Selection::Launch(app) => {
                assert_eq!(
                    app.shell,
                    Some(true),
                    "inherited from the browse node's own shell=\"true\""
                );
                assert!(app.exe.unwrap().ends_with("notes.txt"));
            }
            Selection::Node(_) | Selection::Extra(_) => {
                panic!("expected a discovered file, not a tree node")
            }
        }
    }

    #[test]
    fn depth_lists_subfolders_and_they_stay_lazy_too() {
        let _guard = serialized();
        let dir = TempDir::new("depth");
        dir.mkdir("Sub");
        dir.touch("top.txt");
        let (tree, root) = tree_with_browse(&dir.path(), r#"depth="1""#);

        let (_popup, mut session) = build(&tree, root);
        let top = only_pending_handle(&session);
        on_init_menu_popup(&mut session, top);

        assert_eq!(item_count(top), 2, "the Sub folder, then top.txt");
        assert_eq!(item_text(top, 0), "Sub");

        // The subfolder is its own pending entry, not listed eagerly along
        // with its parent.
        assert_eq!(session.pending.len(), 1);
        let sub = HMENU(*session.pending.keys().next().unwrap() as *mut _);
        assert_eq!(item_count(sub), 0);

        on_init_menu_popup(&mut session, sub); // opening it in turn still works
        assert_eq!(item_count(sub), 1); // Sub itself is empty -> the placeholder
        assert_eq!(item_text(sub, 0), "(empty)");
    }

    /// A folder exactly at `depth` is shown, but flattened: a plain item —
    /// not a submenu, and not one more entry in `session.pending` — that
    /// resolves to opening the folder itself. Needs two levels of nesting:
    /// with `depth="1"`, the *first* level (`Sub`) is still a real,
    /// browsable submenu — it's `Sub`'s own child (`SubSub`, level 2) that
    /// has run out of room.
    #[test]
    fn a_folder_past_max_depth_is_flattened_and_opens_via_the_shell() {
        let _guard = serialized();
        let dir = TempDir::new("max-depth-flatten");
        dir.mkdir("Sub");
        dir.mkdir("Sub/SubSub");
        let (tree, root) = tree_with_browse(&dir.path(), r#"depth="1""#);

        let (_popup, mut session) = build(&tree, root);
        let top = only_pending_handle(&session);
        on_init_menu_popup(&mut session, top);
        assert_eq!(item_text(top, 0), "Sub");
        assert_eq!(
            session.pending.len(),
            1,
            "Sub is still browsable at level 1"
        );

        let sub = HMENU(*session.pending.keys().next().unwrap() as *mut _);
        on_init_menu_popup(&mut session, sub);
        assert_eq!(item_count(sub), 1);
        assert_eq!(item_text(sub, 0), "SubSub");

        // Not a submenu this time — level 2 is past `depth`, so nothing was
        // registered in `session.pending` for it.
        assert!(session.pending.is_empty());

        let id = unsafe { GetMenuItemID(sub, 0) };
        match resolve_selection(id, &mut session).expect("a known id") {
            Selection::Launch(app) => {
                assert_eq!(
                    app.shell,
                    Some(true),
                    "a directory can only be launched via the shell"
                );
                assert!(app.exe.unwrap().ends_with("SubSub"));
            }
            Selection::Node(_) | Selection::Extra(_) => {
                panic!("expected a discovered folder, not a tree node")
            }
        }
    }

    /// `open-folder="never"` drops a folder past `depth` entirely rather
    /// than flattening it — there would be nothing left the user could do
    /// with it (no submenu, and no "open" affordance either). Same two-level
    /// nesting as the flattening test above: `Sub` (level 1) is still a real
    /// submenu regardless of `open-folder`; it's `SubSub` (level 2, past
    /// `depth`) that gets dropped.
    #[test]
    fn open_folder_never_drops_a_folder_past_max_depth() {
        let _guard = serialized();
        let dir = TempDir::new("open-folder-never");
        dir.mkdir("Sub");
        dir.mkdir("Sub/SubSub");
        dir.touch("Sub/inside.txt");
        let (tree, root) = tree_with_browse(&dir.path(), r#"depth="1" open-folder="never""#);

        let (_popup, mut session) = build(&tree, root);
        let top = only_pending_handle(&session);
        on_init_menu_popup(&mut session, top);
        assert_eq!(item_text(top, 0), "Sub");

        let sub = HMENU(*session.pending.keys().next().unwrap() as *mut _);
        on_init_menu_popup(&mut session, sub);

        assert_eq!(item_count(sub), 1, "inside.txt only — SubSub is dropped");
        assert_eq!(item_text(sub, 0), "inside.txt");
    }

    /// `open-folder="all"` adds an "Open folder" entry (and a separator)
    /// ahead of a real submenu's own listing.
    #[test]
    fn open_folder_all_adds_an_open_entry_ahead_of_the_listing() {
        let _guard = serialized();
        let dir = TempDir::new("open-folder-all");
        dir.touch("top.txt");
        let (tree, root) = tree_with_browse(&dir.path(), r#"open-folder="all""#);

        let (_popup, mut session) = build(&tree, root);
        let top = only_pending_handle(&session);
        on_init_menu_popup(&mut session, top);

        assert_eq!(item_text(top, 0), "Open folder");
        // Position 1 is the separator this adds ahead of the (empty, since
        // depth defaults to 0 and this folder has no other files) listing —
        // `open-folder` doesn't change `depth`'s own default.
        assert_eq!(item_count(top), 3, "Open folder, a separator, then top.txt");
        assert_eq!(item_text(top, 2), "top.txt");

        let id = unsafe { GetMenuItemID(top, 0) };
        match resolve_selection(id, &mut session).expect("a known id") {
            Selection::Launch(app) => {
                assert_eq!(app.shell, Some(true));
                assert!(app.exe.unwrap().ends_with("open-folder-all"));
            }
            Selection::Node(_) | Selection::Extra(_) => {
                panic!("expected the discovered open-folder entry")
            }
        }
    }

    #[test]
    fn a_filter_is_applied_lazily_too() {
        let _guard = serialized();
        let dir = TempDir::new("filtered");
        dir.touch("keep.sln");
        dir.touch("skip.txt");
        let (tree, root) = tree_with_browse(&dir.path(), r#"filter="sln""#);

        let (_popup, mut session) = build(&tree, root);
        let submenu = only_pending_handle(&session);
        on_init_menu_popup(&mut session, submenu);

        assert_eq!(item_count(submenu), 1);
        assert_eq!(item_text(submenu, 0), "keep.sln");
    }

    #[test]
    fn a_leaf_item_gets_the_icon_from_its_own_exe() {
        let _guard = serialized();
        let notepad = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\notepad.exe";
        let (tree, root) = tree_with_app(&notepad);
        let app_id = tree.children(root).next().expect("one child");

        let (popup, _session) = build(&tree, root);
        assert!(item_has_icon(popup.handle(), IDM_FIRST + app_id.as_u32()));
    }

    #[test]
    fn a_group_gets_the_folder_icon_by_default() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root"><group name="Sub"><quit/></group></group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];

        let (popup, _session) = build(&tree, root);
        // A submenu item has no command id of its own to address it by
        // (`AppendMenuW`'s `MF_POPUP` slot is the submenu's own handle
        // instead — see `PopupMenu::set_item_icon_by_position`'s doc
        // comment) — position 0, the only item this popup has.
        assert!(item_has_icon_by_position(popup.handle(), 0));
    }

    #[test]
    fn quit_gets_a_stock_icon_by_default() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root"><quit/></group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];
        let quit_id = tree.children(root).next().expect("one child");

        let (popup, _session) = build(&tree, root);
        assert!(item_has_icon(popup.handle(), IDM_FIRST + quit_id.as_u32()));
    }

    /// The bug report this feature exists for: `show-shortcuts="true"` (the
    /// default) plus a node with a hotkey must actually draw the hint — on
    /// the native (unthemed) popup `build` uses by default, that means
    /// `add_item_with_shortcut`'s `"{text}\t{shortcut}"` composition
    /// actually reached `AppendMenuW`, not just that `NodeData::hotkeys`
    /// parsed correctly (see `config`'s own hotkey tests for that half).
    #[test]
    fn a_node_with_a_hotkey_shows_a_shortcut_by_default() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root"><app name="Notepad" exe="notepad.exe"><hotkey key="Win+N"/></app></group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];

        let (popup, _session) = build(&tree, root);
        assert_eq!(item_text(popup.handle(), 0), "Notepad\tWin+N");
    }

    /// `show-shortcuts="false"` on the node itself overrides the (on-by-
    /// default) global setting — see [`shortcut_text`]'s own doc comment.
    #[test]
    fn show_shortcuts_false_on_a_node_suppresses_its_own_hint() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root"><app name="Notepad" exe="notepad.exe" show-shortcuts="false"><hotkey key="Win+N"/></app></group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];

        let (popup, _session) = build(&tree, root);
        assert_eq!(item_text(popup.handle(), 0), "Notepad");
    }

    /// A node with no hotkey at all shows no hint even with `show-shortcuts`
    /// on — PortAL's own `getHotkeyText` returns `NULL` for exactly this
    /// (see `hotKey.cpp`), not an empty string drawn anyway.
    #[test]
    fn a_node_with_no_hotkey_shows_no_shortcut() {
        let _guard = serialized();
        let (tree, root) = tree_with_app("notepad.exe");

        let (popup, _session) = build(&tree, root);
        assert_eq!(item_text(popup.handle(), 0), "App");
    }

    #[test]
    fn a_discovered_file_gets_an_icon_once_the_submenu_opens() {
        let _guard = serialized();
        let dir = TempDir::new("icon-file");
        dir.touch("notes.txt");
        let (tree, root) = tree_with_browse(&dir.path(), "");

        let (_popup, mut session) = build(&tree, root);
        let submenu = only_pending_handle(&session);
        on_init_menu_popup(&mut session, submenu);

        let id = unsafe { GetMenuItemID(submenu, 0) };
        assert!(item_has_icon(submenu, id));
    }

    #[test]
    fn a_discovered_subfolder_gets_the_generic_folder_icon() {
        let _guard = serialized();
        let dir = TempDir::new("icon-folder");
        dir.mkdir("Sub");
        let (tree, root) = tree_with_browse(&dir.path(), r#"depth="1""#);

        let (_popup, mut session) = build(&tree, root);
        let top = only_pending_handle(&session);
        on_init_menu_popup(&mut session, top);

        // Folders sort before files (see `browse`), and this folder is the
        // only entry — position 0.
        assert!(item_has_icon_by_position(top, 0));
    }

    /// `<break/>` appends nothing of its own — item_count only ever grows by
    /// two, not three — but arms `MFT_MENUBREAK` on whichever sibling comes
    /// right after it.
    #[test]
    fn a_break_starts_a_new_column_on_the_item_right_after_it() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root">
                <app name="Before" exe="notepad.exe"/>
                <break/>
                <app name="After" exe="notepad.exe"/>
            </group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];

        let (popup, _session) = build(&tree, root);
        assert_eq!(
            item_count(popup.handle()),
            2,
            "<break/> adds no item of its own"
        );
        assert_eq!(item_text(popup.handle(), 0), "Before");
        assert_eq!(item_text(popup.handle(), 1), "After");
        assert!(!item_starts_new_column(popup.handle(), 0));
        assert!(item_starts_new_column(popup.handle(), 1));
    }

    /// A `<break/>` with nothing after it (the last child of its parent) has
    /// nothing to visibly do — the flag it arms is simply never consumed,
    /// not a dangling break carried onto whatever gets added to this menu
    /// next by some *other* caller.
    #[test]
    fn a_trailing_break_is_harmless() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root">
                <app name="Only" exe="notepad.exe"/>
                <break/>
            </group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];

        let (popup, _session) = build(&tree, root);
        assert_eq!(item_count(popup.handle()), 1);
        assert!(!item_starts_new_column(popup.handle(), 0));
    }

    fn named(tree: &ConfigTree, name: &str) -> NodeId {
        tree.all_nodes()
            .find(|&id| tree.node(id).name.as_ref() == name)
            .unwrap_or_else(|| panic!("no node named {name:?}"))
    }

    const TREE: &str = r#"<y-pop theme="native">
        <group name="Work">
            <app name="First" exe="a.exe"/>
            <separator/>
            <separator name="Tools"/>
            <break/>
            <group name="Dev">
                <app name="Editor" exe="e.exe"/>
                <group name="Deep"><app name="Target" exe="t.exe"/></group>
            </group>
        </group>
        <app name="Alone" exe="x.exe"/>
        <search><group name="Hidden"><app name="Lost" exe="l.exe"/></group></search>
    </y-pop>"#;

    #[test]
    fn revealing_an_item_counts_the_items_above_it_and_goes_down_the_submenus() {
        use Step::{Down, Right};
        let tree = tree_from(TREE);
        let work = tree.roots[0];

        // The first item: one Down gets on it.
        assert_eq!(
            reveal_steps(&tree, named(&tree, "First")),
            Some((work, vec![Down]))
        );
        // Dev: First, the rule (skipped), Tools (a greyed item in a native
        // menu, so counted), the break (nothing) — two above it, one more to
        // start. Then into it.
        assert_eq!(
            reveal_steps(&tree, named(&tree, "Dev")),
            Some((work, vec![Down, Down, Down]))
        );
        // Through Dev (Right, and its first item is already lit), past Editor
        // to Deep, in, and the first item there is Target.
        assert_eq!(
            reveal_steps(&tree, named(&tree, "Target")),
            Some((work, vec![Down, Down, Down, Right, Down, Right]))
        );
    }

    #[test]
    fn a_themed_menus_titled_separators_are_skipped_too() {
        use Step::Down;
        let tree = tree_from(&TREE.replace(r#"theme="native""#, r#"theme="dark""#));
        assert_eq!(
            reveal_steps(&tree, named(&tree, "Dev")).map(|(_, steps)| steps),
            Some(vec![Down, Down]),
            "only First is above it"
        );
    }

    #[test]
    fn a_tray_icon_is_revealed_by_opening_its_menu() {
        let tree = tree_from(TREE);
        let work = tree.roots[0];
        assert_eq!(reveal_steps(&tree, work), Some((work, vec![])));
        let alone = named(&tree, "Alone");
        assert_eq!(reveal_steps(&tree, alone), Some((alone, vec![])));
    }

    #[test]
    fn what_no_tray_menu_holds_cannot_be_revealed() {
        let tree = tree_from(TREE);
        assert_eq!(reveal_steps(&tree, named(&tree, "Lost")), None);
        assert_eq!(reveal_steps(&tree, named(&tree, "Hidden")), None);
    }

    // `resolve_design` and the three built-in skins have their own test
    // suite alongside the code that implements them — see `crate::skins`'s
    // own `mod tests`.
}
