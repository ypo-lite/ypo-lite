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
//! [`resolve_colors`] turns the config's [`Theme`] into
//! `Option<MenuColors>` — `None` for `Theme::Native` (plain OS chrome,
//! `PopupMenu::new()`), `Some` for `Light`/`Dark`/`Auto` (owner-drawn,
//! `PopupMenu::themed`). Resolved once per popup open, in [`build`], and
//! carried on [`Session`] from there so every submenu — built up front in
//! [`append_children`] or lazily in [`on_init_menu_popup`] — themes
//! consistently with the root. [`LIGHT`]/[`DARK`] are `y-pop`'s own palette
//! choice, not `y-core`'s: `y_core::menu::MenuColors` only defines the
//! shape, the same split `y-snap`'s `Palette::LIGHT`/`DARK` already draws
//! between its own app and `y-core`.
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

use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::WindowsAndMessaging::HMENU;
use windows::core::Result;

use y_core::icon::{Bitmap, Icon, IconCache};
use y_core::ids::IDM_FIRST;
use y_core::menu::{self, MenuColors, PopupMenu};
use y_core::overlay::Color;

use y_pop::model::{
    App, Browse, ConfigTree, EventMask, NodeData, NodeId, NodeKind, Theme, ThemeOverrides,
};

/// `y-pop`'s own palette choice for a themed menu — see the module doc's
/// "Theme" section for why these live here and not in `y_core::menu`.
/// Values close to Windows 11's own flyouts, the same reference point
/// `y-snap`'s own `Palette::LIGHT` uses.
const LIGHT: MenuColors = MenuColors {
    back: Color::rgb(249, 249, 249),
    text: Color::rgb(0, 0, 0),
    hover_back: Color::rgb(0, 120, 215),
    hover_text: Color::rgb(255, 255, 255),
    disabled_text: Color::rgb(160, 160, 160),
    border: Color::rgb(0, 120, 215),
};

const DARK: MenuColors = MenuColors {
    back: Color::rgb(32, 32, 32),
    text: Color::rgb(255, 255, 255),
    disabled_text: Color::rgb(120, 120, 120),
    ..LIGHT
};

/// [`Theme`] + [`ThemeOverrides`] → `Option<MenuColors>` — `None` means
/// plain native chrome. The one place `Theme::Auto` is resolved against the
/// live OS setting (`y_core::theme::is_dark`, a registry read); called
/// fresh from [`build`] each time a popup opens rather than cached at
/// config-load time, so flipping Windows' light/dark setting takes effect
/// on the very next popup, not after a restart.
///
/// `theme="native"` (the default) with no `*-color` attribute at all stays
/// plain chrome — the zero-surprise case [`Theme::Native`]'s own doc
/// comment promises. But a config that sets a colour without also setting
/// `theme` clearly wants *something* themed to paint that colour onto —
/// PortAL's own custom-skin users never had to pick a base skin first
/// either — so any override present resolves as if `theme="auto"` had been
/// written too, then every set slot overrides that base.
fn resolve_colors(theme: Theme, overrides: ThemeOverrides) -> Option<MenuColors> {
    let effective = if theme == Theme::Native && overrides.any_set() {
        Theme::Auto
    } else {
        theme
    };
    let base = match effective {
        Theme::Native => return None,
        Theme::Light => LIGHT,
        Theme::Dark => DARK,
        Theme::Auto if y_core::theme::is_dark() => DARK,
        Theme::Auto => LIGHT,
    };
    Some(MenuColors {
        back: overrides.back.unwrap_or(base.back),
        text: overrides.text.unwrap_or(base.text),
        hover_back: overrides.hover_back.unwrap_or(base.hover_back),
        hover_text: overrides.hover_text.unwrap_or(base.hover_text),
        disabled_text: overrides.disabled_text.unwrap_or(base.disabled_text),
        border: overrides.border.unwrap_or(base.border),
    })
}

/// First id handed to a discovered (browsed) file — see the module doc.
const DISCOVERED_ID_BASE: u32 = 1_000_000;


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
    colors: Option<MenuColors>,
}

struct PendingFolder {
    dir: String,
    browse: Browse,
    /// Inherited by every file (and, recursively, every subfolder) this
    /// folder turns out to contain — see the module doc's second section.
    shell: bool,
    elevate: bool,
}

impl Session {
    pub(crate) fn new() -> Self {
        Self::with_colors(None)
    }

    fn with_colors(colors: Option<MenuColors>) -> Self {
        Self {
            pending: HashMap::new(),
            discovered: HashMap::new(),
            next_discovered_id: DISCOVERED_ID_BASE,
            icons: IconCache::new(),
            bitmaps: Vec::new(),
            colors,
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

/// `node.icon` if the config gave one, else a default depending on kind:
/// the generic folder icon for a `Group` (PortAL's own default — confirmed
/// against a screenshot of PortAL's stock menu, not assumed — every
/// submenu shows a folder unless its config gives it something else), the
/// icon its own `exe` carries for an `App`. Nothing else (`About`, `Quit`,
/// a titled `Separator`) gets a default; PortAL does not derive one for
/// those either.
fn resolve_icon(icons: &mut IconCache, node: &NodeData) -> Option<Icon> {
    if let Some(source) = &node.icon {
        return icons.for_file(&source.path, source.index);
    }
    match &node.kind {
        NodeKind::Group => icons.for_directory(),
        // A browse app's own "exe" names a folder, not a file — the
        // generic folder icon, not whatever `for_file` would guess from a
        // directory name that happens to contain a '.'.
        NodeKind::App(app) if app.browse.is_some() => icons.for_directory(),
        NodeKind::App(app) => icons.for_file(app.exe.as_deref()?, None),
        _ => None,
    }
}

/// What a native menu's `track()` result turned out to mean.
pub enum Selection {
    /// A real node — walk `ConfigTree` from here, same as before browsing
    /// existed.
    Node(NodeId),
    /// A file discovered while browsing — nothing to look up, launch it.
    Launch(App),
}

/// Recovers what a native menu's `track()` returned, checking `session`'s
/// discovered files first (their ids are unambiguous — see the module doc —
/// so this never mistakes one for a stale/foreign command id and vice
/// versa). `None` for an id neither space recognises, which should not
/// happen for a selection this popup itself produced, but a stray
/// `WM_COMMAND` from somewhere else is exactly the case worth failing
/// softly on rather than panicking.
pub fn resolve_selection(id: u32, session: &mut Session) -> Option<Selection> {
    if let Some(app) = session.discovered.remove(&id) {
        return Some(Selection::Launch(app));
    }
    id.checked_sub(IDM_FIRST)
        .map(NodeId::from_u32)
        .map(Selection::Node)
}

/// Builds a popup for `group`'s children.
pub fn build(tree: &ConfigTree, group: NodeId) -> (PopupMenu, Session) {
    let colors = resolve_colors(tree.defaults.theme, tree.defaults.theme_overrides);
    let mut menu = PopupMenu::new_maybe_themed(colors).expect("CreatePopupMenu");
    let mut session = Session::with_colors(colors);
    append_children(tree, group, &mut menu, &mut session);
    (menu, session)
}

fn append_children(tree: &ConfigTree, parent: NodeId, menu: &mut PopupMenu, session: &mut Session) {
    for child in tree.children(parent) {
        let node = tree.node(child);

        match &node.kind {
            NodeKind::Group => {
                let mut submenu = PopupMenu::new_maybe_themed(session.colors).expect("CreatePopupMenu");
                append_children(tree, child, &mut submenu, session);

                let icon = resolve_icon(&mut session.icons, node);
                let _ = menu.add_submenu(&node.name, submenu);
                let position = menu.item_count().saturating_sub(1);
                session.attach_icon(icon, |bitmap| menu.set_item_icon_by_position(position, bitmap));
            }
            NodeKind::App(app) if app.browse.is_some() => {
                let submenu = PopupMenu::new_maybe_themed(session.colors).expect("CreatePopupMenu");
                session.pending.insert(
                    submenu.handle().0 as usize,
                    PendingFolder {
                        dir: app.exe.as_deref().unwrap_or_default().to_owned(),
                        browse: app.browse.clone().expect("checked by this arm's guard"),
                        shell: app.shell,
                        elevate: app.elevate,
                    },
                );

                let icon = resolve_icon(&mut session.icons, node);
                let _ = menu.add_submenu(&node.name, submenu);
                let position = menu.item_count().saturating_sub(1);
                session.attach_icon(icon, |bitmap| menu.set_item_icon_by_position(position, bitmap));
            }
            // Draws as a thin rule when untitled, its own title text when
            // titled — themed or not, `add_titled_separator` picks the
            // right rendering for each (see the module doc's "Theme"
            // section and `y_core::menu::PopupMenu::add_titled_separator`).
            NodeKind::Separator => {
                let _ = menu.add_titled_separator(&node.name);
            }
            NodeKind::App(_) | NodeKind::About | NodeKind::Quit => {
                let id = IDM_FIRST + child.as_u32();
                let icon = resolve_icon(&mut session.icons, node);
                let _ = menu.add_item(id, &node.name);
                session.attach_icon(icon, |bitmap| menu.set_item_icon(id, bitmap));
            }
        }
    }
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

    let entries = crate::browse::list(&pending.dir, &pending.browse);
    if entries.is_empty() {
        let _ = menu::append_disabled_item(hmenu, "(empty)", session.colors);
        return;
    }

    for entry in entries {
        if entry.is_dir {
            let icon = session.icons.for_directory();
            let Ok(submenu) = menu::append_submenu(hmenu, &entry.name, session.colors) else {
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
                },
            );
        } else {
            let icon = session.icons.for_file(&entry.path, None);
            let id = session.next_discovered_id();
            let _ = menu::append_item(hmenu, id, &entry.name, session.colors);
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
// `session.colors` is threaded through so a browsed folder's own rows theme
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
        MF_BYPOSITION, MIIM_BITMAP,
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
        let xml = format!(r#"<y-pop><group name="Root"><app name="App" exe="{exe}"/></group></y-pop>"#);
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
                assert!(
                    app.shell,
                    "inherited from the browse node's own shell=\"true\""
                );
                assert!(app.exe.unwrap().ends_with("notes.txt"));
            }
            Selection::Node(_) => panic!("expected a discovered file, not a tree node"),
        }
    }

    #[test]
    fn recurse_lists_subfolders_and_they_stay_lazy_too() {
        let _guard = serialized();
        let dir = TempDir::new("recurse");
        dir.mkdir("Sub");
        dir.touch("top.txt");
        let (tree, root) = tree_with_browse(&dir.path(), r#"recurse="true""#);

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
        assert!(item_has_icon(
            popup.handle(),
            IDM_FIRST + app_id.as_u32()
        ));
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
    fn quit_gets_no_icon_by_default() {
        let _guard = serialized();
        let tree = config::load_str(
            r#"<y-pop><group name="Root"><quit/></group></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let root = tree.roots[0];
        let quit_id = tree.children(root).next().expect("one child");

        let (popup, _session) = build(&tree, root);
        assert!(!item_has_icon(popup.handle(), IDM_FIRST + quit_id.as_u32()));
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
        let (tree, root) = tree_with_browse(&dir.path(), r#"recurse="true""#);

        let (_popup, mut session) = build(&tree, root);
        let top = only_pending_handle(&session);
        on_init_menu_popup(&mut session, top);

        // Folders sort before files (see `browse`), and this folder is the
        // only entry — position 0.
        assert!(item_has_icon_by_position(top, 0));
    }

    #[test]
    fn theme_native_with_no_overrides_stays_unthemed() {
        assert!(resolve_colors(Theme::Native, ThemeOverrides::default()).is_none());
    }

    #[test]
    fn an_override_with_theme_left_native_still_themes_the_popup() {
        let overrides = ThemeOverrides {
            hover_back: Some(Color::rgb(0xff, 0x88, 0x00)),
            ..ThemeOverrides::default()
        };
        let colors = resolve_colors(Theme::Native, overrides).expect(
            "a colour override with no explicit theme must still resolve to a themed popup",
        );
        assert_eq!(colors.hover_back, Color::rgb(0xff, 0x88, 0x00));
    }

    #[test]
    fn an_override_wins_over_the_base_theme_everywhere_else_matches_it() {
        let overrides = ThemeOverrides {
            back: Some(Color::rgb(1, 2, 3)),
            ..ThemeOverrides::default()
        };
        let colors = resolve_colors(Theme::Light, overrides).expect("theme=\"light\" themes");
        assert_eq!(colors.back, Color::rgb(1, 2, 3));
        assert_eq!(colors.text, LIGHT.text, "every other slot keeps the base theme's value");
    }
}
