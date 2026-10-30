//! What the tray menu screen's buttons and fields do to the settings.
//!
//! Each [`MenuMsg`] is settled here, in one `match`, by calling the plain
//! functions in [`crate::tree`] (moves, inserts) or by setting one field.
//! Anything destructive keeps the whole `Settings` for Undo first; a field
//! edit only marks the file as changed when the value really differs, because
//! WinUI controls report a value when they are first shown.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, VIRTUAL_KEY};
use y_core::hotkey::Hotkey;
use y_pop::model::OpenFolder;
use y_pop::settings::{AppFields, HotkeyEntry, ItemId, ItemKind, Renderer};

use crate::menu_view::HotkeyUi;
use crate::roots;
use crate::start_menu::Source;
use crate::tree::{self, MoveDir, NewKind};
use crate::ui::key_picker::{KeyPickerUi, MODIFIERS};
use crate::ui::keys::{self, KeyCategory};
use crate::{ConfigApp, Screen, Undo};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextField {
    Name,
    Tooltip,
    Icon,
    /// An app's program, file or folder.
    Target,
    /// A Store app's id.
    AppId,
    Param,
    StartIn,
    Filter,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlagField {
    /// Shown in bold, and what a double-click on the tray icon runs.
    Default,
    /// A tray menu that opens when the pointer rests on its icon.
    OpenOnHover,
    Elevate,
    OnStart,
    OnExit,
}

/// What an app launches: a program or file, a folder listed as a submenu, or
/// a Store app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetMode {
    Program,
    Folder,
    Store,
}

impl TargetMode {
    pub const ALL: [Self; 3] = [Self::Program, Self::Folder, Self::Store];

    pub fn of(app: &AppFields) -> Self {
        if app.app_id.is_some() {
            Self::Store
        } else if app.browse {
            Self::Folder
        } else {
            Self::Program
        }
    }
}

/// Moves `app` to `mode`. The value of the mode it leaves goes to `stash`
/// (`(exe, app-id)`) and comes back when it is chosen again, so a click on
/// the wrong choice loses nothing. Returns whether anything changed.
pub(crate) fn set_target_mode(
    app: &mut AppFields,
    mode: TargetMode,
    stash: &mut (Option<String>, Option<String>),
) -> bool {
    let from = TargetMode::of(app);
    if from == mode {
        return false;
    }
    if from == TargetMode::Store {
        stash.1 = app.app_id.take();
        app.exe = stash.0.take();
    }
    match mode {
        TargetMode::Store => {
            stash.0 = app.exe.take();
            app.app_id = Some(stash.1.take().unwrap_or_default());
            app.browse = false;
        }
        TargetMode::Folder => app.browse = true,
        TargetMode::Program => app.browse = false,
    }
    true
}

/// Depth 0–200 in words: what the number does to the listing.
pub(crate) fn depth_words(depth: u32) -> String {
    match depth {
        0 => "files only, no subfolders".to_owned(),
        1 => "1 level of subfolders".to_owned(),
        n => format!("{n} levels of subfolders"),
    }
}

/// Where files dropped on the outline go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropTarget {
    /// On a row: inside it when it is a group, else right after it.
    Row(ItemId),
    /// On the drop strip: at the insertion point, as the toolbar adds.
    Zone,
}

/// A choice on the strip that says how a dropped file was turned into an item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OriginChange {
    /// A shortcut: itself or its target.
    Shortcut,
    /// A folder: a submenu or opened.
    Browse,
    /// Paths with `%variables%` or absolute.
    Variables,
}

pub enum MenuMsg {
    /// Switch one of the strip's choices.
    OriginSwitch(OriginChange),
    /// Close the strip.
    OriginDismiss,
    /// A file is being dragged over a target (`None`: it left).
    DragOver(Option<DropTarget>),
    /// Files were dropped, as full paths in drop order.
    Dropped(DropTarget, Vec<String>),
    Select(ItemId),
    ToggleCollapsed(ItemId),
    Add(NewKind),
    /// The row of less common things to add.
    MoreToggled,
    /// "App…": opens or closes the app picker under the toolbar.
    AppPickerToggled,
    /// The picker's Browse…: files from the open dialog, added as a drop on the strip would.
    AppBrowse,
    /// Opens the dialog on one list: the picker's Start menu… and Store apps…
    /// (to add several), or, with an item, the Store field's Browse… (to choose one).
    StartOpened(Source, Option<ItemId>),
    /// The dialog's Cancel: closes it and forgets the ticks.
    StartClosed,
    /// The Start menu list's search box.
    StartSearch(String),
    /// The Start menu list's "Show more".
    StartMore,
    /// Ticks or unticks a Start menu shortcut (by its path).
    StartToggle(String),
    /// "Add N apps": the ticked shortcuts, as a drop on the strip would add them.
    StartAdd,
    /// The "written to y-pop.xml" preview.
    XmlToggled,
    Move {
        id: ItemId,
        dir: MoveDir,
    },
    /// Into the group at this place of `tree::groups(settings, Some(id))`.
    MoveTo {
        id: ItemId,
        group: usize,
    },
    Duplicate(ItemId),
    Delete(ItemId),
    Text(ItemId, TextField, String),
    Flag(ItemId, FlagField, bool),
    /// 0 inherit, 1 on, 2 off.
    ShowShortcuts(ItemId, usize),
    /// 0 inherit, 1 native, 2 overlay.
    PopupStyle(ItemId, usize),
    TurnIntoMenu(ItemId),
    /// Launches the app as y-pop would.
    RunNow(ItemId),
    /// Browse… beside the target: a file or folder dialog, whose answer becomes the target.
    Browse(ItemId),

    // An app.
    TargetMode(ItemId, TargetMode),
    /// 0 inherit, 1 on, 2 off.
    ShellExecute(ItemId, usize),
    Depth(ItemId, Option<f64>),
    /// 0 only at the deepest level (the default), 1 in every submenu, 2 never.
    OpenFolder(ItemId, usize),
    /// Adds `%name%` to the end of a text.
    InsertVariable(ItemId, TextField, String),

    // Hotkeys: `(item, position among its hotkeys)` addresses one.
    HotkeyAdd(ItemId),
    /// Opens the picker dialog on that hotkey.
    HotkeyEdit(ItemId, usize),
    /// The dialog closed: OK keeps the edit, anything else restores the
    /// hotkey as it was (and drops one that was just added).
    HotkeyClosed {
        accepted: bool,
    },
    HotkeyRemove(ItemId, usize),
    HotkeyOverride(ItemId, usize, bool),
    HotkeyKey(ItemId, usize, VIRTUAL_KEY),
    HotkeyModifier(ItemId, usize, HOT_KEY_MODIFIERS),
    HotkeySearchChosen(ItemId, usize, String),
    HotkeyCategory(KeyCategory),
    HotkeySearch(String),
}

/// A hotkey as written in the file, as the picker shows it: the modifiers it
/// names and its key. An unreadable text keeps whatever modifiers it names,
/// so chips can be set before a key is.
pub(crate) fn split_hotkey(raw: &str) -> (HOT_KEY_MODIFIERS, Option<VIRTUAL_KEY>) {
    if let Some(hotkey) = Hotkey::parse(raw) {
        // `parse` adds MOD_NOREPEAT, which the picker has no chip for.
        let held = MODIFIERS.iter().fold(0, |held, (_, modifier)| {
            held | (hotkey.modifiers.0 & modifier.0)
        });
        return (HOT_KEY_MODIFIERS(held), Some(hotkey.key));
    }
    let held = raw
        .split('+')
        .filter_map(|part| {
            MODIFIERS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(part.trim()))
        })
        .fold(0, |held, (_, modifier)| held | modifier.0);
    (HOT_KEY_MODIFIERS(held), None)
}

/// The inverse of [`split_hotkey`]: without a key, just the modifiers' names.
pub(crate) fn join_hotkey(modifiers: HOT_KEY_MODIFIERS, key: Option<VIRTUAL_KEY>) -> String {
    match key {
        Some(key) => Hotkey::new(modifiers, key).to_string(),
        None => MODIFIERS
            .iter()
            .filter(|(_, modifier)| modifiers.0 & modifier.0 != 0)
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join("+"),
    }
}

/// A hotkey text in one canonical spelling, so `ctrl+alt+f5` and `Ctrl+Alt+F5`
/// count as the same combination. `None` when it isn't one.
pub(crate) fn canonical_hotkey(raw: &str) -> Option<String> {
    Hotkey::parse(raw).map(|hotkey| hotkey.to_string())
}

impl ConfigApp {
    /// The tray menu being edited, if a menu screen is open.
    pub(crate) fn open_root(&self) -> Option<ItemId> {
        match self.screen {
            Screen::Menu(id) => Some(id),
            _ => None,
        }
    }

    /// What is selected in the tray menu `root`: the menu's own item when
    /// nothing is, or when what was selected is gone.
    pub(crate) fn selected(&self, root: ItemId) -> ItemId {
        let Ok(settings) = &self.settings else {
            return root;
        };
        self.selection
            .get(&root)
            .copied()
            .filter(|&id| tree::is_inside(settings, root, id))
            .unwrap_or(root)
    }

    /// The text of one hotkey, if it exists.
    fn hotkey_text(&self, id: ItemId, index: usize) -> Option<String> {
        let settings = self.settings.as_ref().ok()?;
        tree::item(settings, id)?
            .hotkeys
            .get(index)
            .map(|entry| entry.key.clone())
    }

    /// Asks for the target of the app `id` — a file, or a folder in Folder
    /// mode — opening where the current one is. `None` if cancelled.
    fn browse_target(&self, id: ItemId) -> Option<String> {
        let settings = self.settings.as_ref().ok()?;
        let item = tree::item(settings, id)?;
        let ItemKind::App(fields) = &item.kind else {
            return None;
        };
        let folder = TargetMode::of(fields) == TargetMode::Folder;

        let current = settings.expand_text(
            fields.exe.as_deref().unwrap_or_default(),
            &mut *self.builtins.borrow_mut(),
        );
        let current = std::path::PathBuf::from(current.trim().trim_matches('"'));
        let start = if current.is_dir() {
            Some(current)
        } else {
            current.parent().map(std::path::Path::to_path_buf)
        };

        let options = y_core::dialog::OpenOptions {
            title: Some(if folder {
                "Choose a folder"
            } else {
                "Choose a program or file"
            }),
            filters: if folder {
                &[]
            } else {
                &[
                    ("Programs and shortcuts", "*.exe;*.lnk;*.bat;*.cmd"),
                    ("All files", "*.*"),
                ]
            },
            start_folder: start.as_deref(),
            pick_folders: folder,
            ..Default::default()
        };
        // The window being clicked is the foreground one.
        let owner = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        let picked = y_core::dialog::open_files(Some(owner), &options).ok()?;
        picked
            .into_iter()
            .next()
            .map(|path| path.to_string_lossy().into_owned())
    }

    /// The app picker's Browse…: any number of programs, shortcuts or files.
    fn browse_apps(&self) -> Vec<String> {
        let options = y_core::dialog::OpenOptions {
            title: Some("Add programs, shortcuts or files"),
            filters: &[
                ("Programs and shortcuts", "*.exe;*.lnk;*.bat;*.cmd"),
                ("All files", "*.*"),
            ],
            multiple: true,
            ..Default::default()
        };
        // The window being clicked is the foreground one.
        let owner = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        y_core::dialog::open_files(Some(owner), &options)
            .unwrap_or_default()
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    /// "Run now": launches the app as y-pop would — expanded, with the
    /// current clipboard — and puts the outcome in the status bar.
    fn run_now(&mut self, id: ItemId) {
        let Ok(settings) = &self.settings else { return };
        let Some(item) = tree::item(settings, id) else {
            return;
        };
        let ItemKind::App(fields) = &item.kind else {
            return;
        };

        let label = roots::label(item);
        let app = settings.launch_app(fields, &mut *self.builtins.borrow_mut());
        let launched = y_pop::launch::launch(HWND::default(), &app, y_pop::launch::Usage::Silent);
        self.run_status = Some(match launched {
            Ok(()) => (format!("Ran {label}"), true),
            Err(error) => (format!("Couldn't run {label}: {error}"), false),
        });
    }

    pub(crate) fn edit_menu(&mut self, message: MenuMsg) {
        let Some(root) = self.open_root() else { return };

        // Only the screen's own state.
        match message {
            MenuMsg::Select(id) => {
                self.selection.insert(root, id);
                if self.origin.as_ref().is_some_and(|(item, _)| *item != id) {
                    self.origin = None;
                }
                return;
            }
            MenuMsg::ToggleCollapsed(id) => {
                if !self.collapsed.remove(&id) {
                    self.collapsed.insert(id);
                }
                return;
            }
            MenuMsg::MoreToggled => {
                self.more_open = !self.more_open;
                self.app_picker = false;
                return;
            }
            MenuMsg::AppPickerToggled => {
                self.app_picker = !self.app_picker;
                self.more_open = false;
                return;
            }
            MenuMsg::StartSearch(text) => {
                self.start_menu.set_search(text);
                return;
            }
            MenuMsg::StartMore => {
                self.start_menu.show_more();
                return;
            }
            MenuMsg::StartToggle(path) => {
                self.start_menu.toggle(path);
                return;
            }
            MenuMsg::StartOpened(source, target) => {
                self.start_menu.open(source, target);
                self.start_open = true;
                return;
            }
            MenuMsg::StartClosed => {
                self.start_open = false;
                self.start_menu.checked.clear();
                return;
            }
            MenuMsg::StartAdd => {
                self.app_picker = false;
                self.start_open = false;
                let paths = self.start_menu.checked_in_order(self.source_entries());
                self.start_menu.checked.clear();
                match self.start_menu.target.take() {
                    // One Store app for an item being edited.
                    Some(item) => {
                        if let Some(entry) = self
                            .start_entries
                            .iter()
                            .find(|entry| paths.contains(&entry.path))
                        {
                            let app_id =
                                entry.path[y_core::apps_folder::FOLDER.len() + 1..].to_owned();
                            let unnamed = self.settings.as_ref().ok().is_none_or(|settings| {
                                tree::item(settings, item).is_none_or(|item| {
                                    item.name.as_deref().is_none_or(str::is_empty)
                                })
                            });
                            let name = entry.name.clone();
                            let no_icon = self.settings.as_ref().ok().is_none_or(|settings| {
                                tree::item(settings, item).is_none_or(|item| {
                                    item.icon.as_deref().is_none_or(str::is_empty)
                                })
                            });
                            self.edit_menu(MenuMsg::Text(item, TextField::AppId, app_id));
                            if no_icon {
                                let logo = y_pop::model::APP_LOGO.to_owned();
                                self.edit_menu(MenuMsg::Text(item, TextField::Icon, logo));
                            }
                            if unnamed {
                                self.edit_menu(MenuMsg::Text(item, TextField::Name, name));
                            }
                        }
                    }
                    None if !paths.is_empty() => {
                        self.edit_menu(MenuMsg::Dropped(DropTarget::Zone, paths));
                    }
                    None => {}
                }
                return;
            }
            MenuMsg::AppBrowse => {
                self.app_picker = false;
                let paths = self.browse_apps();
                if !paths.is_empty() {
                    self.edit_menu(MenuMsg::Dropped(DropTarget::Zone, paths));
                }
                return;
            }
            MenuMsg::OriginDismiss => {
                self.origin = None;
                return;
            }
            MenuMsg::DragOver(target) => {
                self.drag_over = target;
                return;
            }
            MenuMsg::XmlToggled => {
                self.show_xml = !self.show_xml;
                return;
            }
            MenuMsg::Browse(id) => {
                if let Some(path) = self.browse_target(id) {
                    self.run_status = None;
                    if let Ok(settings) = &mut self.settings
                        && let Some(item) = tree::item_mut(settings, id)
                        && set_text(item, TextField::Target, path)
                    {
                        self.dirty = true;
                    }
                }
                return;
            }
            MenuMsg::RunNow(id) => {
                self.run_now(id);
                return;
            }
            MenuMsg::HotkeyEdit(id, index) => {
                let raw = self.hotkey_text(id, index);
                self.hotkeys = HotkeyUi {
                    open: raw.is_some().then_some((id, index)),
                    picker: KeyPickerUi::opening_on(
                        raw.as_deref().and_then(|raw| split_hotkey(raw).1),
                    ),
                    original: raw.unwrap_or_default(),
                };
                return;
            }
            MenuMsg::HotkeyCategory(category) => {
                self.hotkeys.picker.category = category;
                return;
            }
            MenuMsg::HotkeySearch(text) => {
                self.hotkeys.picker.search = text;
                return;
            }
            _ => {}
        }

        self.run_status = None;
        let selected = self.selected(root);
        let Ok(settings) = &mut self.settings else {
            return;
        };
        let mut changed = false;
        // Where to go after this edit, if it moved something out of this menu.
        let mut navigate: Option<(ItemId, ItemId)> = None;

        let name_of = |settings: &y_pop::settings::Settings, id: ItemId| {
            tree::item(settings, id)
                .map(roots::label)
                .unwrap_or_default()
        };

        match message {
            MenuMsg::Dropped(target, paths) => {
                self.drag_over = None;
                let over = match target {
                    DropTarget::Row(id) => id,
                    DropTarget::Zone => selected,
                };
                // Rows insert inside a group even when it is folded; the strip
                // goes where the toolbar would add.
                let none = std::collections::HashSet::new();
                let folded = if target == DropTarget::Zone {
                    &self.collapsed
                } else {
                    &none
                };
                if let Some(at) = tree::insertion_point(settings, root, Some(over), folded) {
                    let mut items = Vec::new();
                    let mut origin = None;
                    for path in &paths {
                        let resolved = crate::from_file::resolve(path);
                        let builtins = &mut *self.builtins.borrow_mut();
                        let id = settings.new_id();
                        let mut tokenize = |text: &str| {
                            settings
                                .tokenize(text, &mut *builtins)
                                .unwrap_or_else(|| text.to_owned())
                        };
                        let (item, made) = crate::from_file::item_from_file(
                            id,
                            path,
                            &resolved,
                            &mut tokenize,
                            crate::from_file::Variant::default(),
                        );
                        origin = Some((id, made));
                        items.push(item);
                    }
                    if let Some(id) = tree::insert(settings, at, items) {
                        self.collapsed.remove(&at.parent);
                        self.selection.insert(root, id);
                        self.origin = origin;
                        changed = true;
                    }
                }
            }
            MenuMsg::OriginSwitch(change) => {
                if let Some((id, origin)) = self.origin.clone().filter(|(id, _)| *id == selected) {
                    let mut variant = origin.variant;
                    match change {
                        OriginChange::Shortcut => variant.keep_shortcut = !variant.keep_shortcut,
                        OriginChange::Browse => variant.browse = !variant.browse,
                        OriginChange::Variables => variant.variables = !variant.variables,
                    }
                    let resolved = crate::from_file::resolve(&origin.path);
                    let builtins = &mut *self.builtins.borrow_mut();
                    let mut tokenize = |text: &str| {
                        settings
                            .tokenize(text, &mut *builtins)
                            .unwrap_or_else(|| text.to_owned())
                    };
                    let was = crate::from_file::item_from_file(
                        id,
                        &origin.path,
                        &resolved,
                        &mut tokenize,
                        origin.variant,
                    )
                    .0;
                    let (now, remade) = crate::from_file::item_from_file(
                        id,
                        &origin.path,
                        &resolved,
                        &mut tokenize,
                        variant,
                    );
                    if let Some(item) = tree::item_mut(settings, id) {
                        changed = crate::from_file::switch(item, &was, &now);
                    }
                    self.origin = Some((id, remade));
                }
            }
            MenuMsg::Add(kind) => {
                if let Some(at) =
                    tree::insertion_point(settings, root, Some(selected), &self.collapsed)
                {
                    let item = tree::new_item(settings, kind);
                    if let Some(id) = tree::insert(settings, at, vec![item]) {
                        // The new item must be visible where it landed.
                        self.collapsed.remove(&at.parent);
                        self.selection.insert(root, id);
                        self.more_open = false;
                        self.app_picker = false;
                        changed = true;
                    }
                }
            }

            MenuMsg::Move { id, dir } => {
                if tree::can_move(settings, id, dir, root) {
                    let before = settings.clone();
                    let label = name_of(settings, id);
                    let into = (dir == MoveDir::Into)
                        .then(|| tree::group_above(settings, id))
                        .flatten();
                    if tree::move_item(settings, id, dir, root) {
                        if let Some(group) = into {
                            self.collapsed.remove(&group);
                        }
                        self.undo = Some(Undo {
                            label: format!("Moved {label}"),
                            settings: before,
                        });
                        changed = true;
                    }
                }
            }
            MenuMsg::MoveTo { id, group } => {
                let target = tree::groups(settings, Some(id))
                    .get(group)
                    .map(|(_, group)| *group);
                if let Some(target) = target {
                    let before = settings.clone();
                    let label = name_of(settings, id);
                    if tree::move_to_group(settings, id, target) {
                        self.undo = Some(Undo {
                            label: format!("Moved {label}"),
                            settings: before,
                        });
                        self.collapsed.remove(&target);
                        // Follow it: it just left this menu if the group is elsewhere.
                        if let Some(destination) = tree::root_of(settings, target) {
                            navigate = Some((destination, id));
                        }
                        changed = true;
                    }
                }
            }
            MenuMsg::Duplicate(id) => {
                if let Some(copy) = tree::duplicate(settings, id) {
                    self.selection.insert(root, copy);
                    changed = true;
                }
            }
            MenuMsg::Delete(id) => {
                let before = settings.clone();
                let inside = tree::count_inside(settings, id);
                if let Some((removed, next)) = tree::delete(settings, id) {
                    let mut label = format!("Deleted {}", roots::label(&removed));
                    if inside > 0 {
                        label.push_str(&format!(
                            " and {inside} item{} inside",
                            if inside == 1 { "" } else { "s" }
                        ));
                    }
                    self.undo = Some(Undo {
                        label,
                        settings: before,
                    });
                    self.selection.insert(root, next.unwrap_or(root));
                    changed = true;
                }
            }

            MenuMsg::Text(id, field, text) => {
                if let Some(item) = tree::item_mut(settings, id) {
                    changed = set_text(item, field, text);
                }
            }
            MenuMsg::Flag(id, field, on) => {
                if let Some(item) = tree::item_mut(settings, id) {
                    let slot = match (field, &mut item.kind) {
                        (FlagField::Default, _) => Some(&mut item.default),
                        (FlagField::OpenOnHover, _) => Some(&mut item.auto_open),
                        (FlagField::Elevate, ItemKind::App(app)) => Some(&mut app.elevate),
                        (FlagField::OnStart, ItemKind::App(app)) => Some(&mut app.on_start),
                        (FlagField::OnExit, ItemKind::App(app)) => Some(&mut app.on_exit),
                        _ => None,
                    };
                    if let Some(slot) = slot.filter(|slot| **slot != on) {
                        *slot = on;
                        changed = true;
                    }
                }
            }
            MenuMsg::ShowShortcuts(id, choice) => {
                if let Some(item) = tree::item_mut(settings, id) {
                    let value = match choice {
                        1 => Some(true),
                        2 => Some(false),
                        _ => None,
                    };
                    if item.show_shortcuts != value {
                        item.show_shortcuts = value;
                        changed = true;
                    }
                }
            }
            MenuMsg::PopupStyle(id, choice) => {
                if let Some(item) = tree::item_mut(settings, id) {
                    let value = match choice {
                        1 => Some(Renderer::Native),
                        2 => Some(Renderer::Overlay),
                        _ => None,
                    };
                    if item.popup != value {
                        item.popup = value;
                        changed = true;
                    }
                }
            }
            MenuMsg::TargetMode(id, mode) => {
                let stash = self.target_stash.entry(id).or_default();
                if let Some(ItemKind::App(app)) =
                    tree::item_mut(settings, id).map(|item| &mut item.kind)
                {
                    changed = set_target_mode(app, mode, stash);
                }
            }
            MenuMsg::ShellExecute(id, choice) => {
                if let Some(ItemKind::App(app)) =
                    tree::item_mut(settings, id).map(|item| &mut item.kind)
                {
                    let value = match choice {
                        1 => Some(true),
                        2 => Some(false),
                        _ => None,
                    };
                    changed = app.shell != value;
                    app.shell = value;
                }
            }
            MenuMsg::Depth(id, depth) => {
                if let Some(ItemKind::App(app)) =
                    tree::item_mut(settings, id).map(|item| &mut item.kind)
                {
                    // An empty box is the default: no `depth` attribute.
                    let value = depth.map(|depth| depth.clamp(0.0, 200.0) as u32);
                    changed = app.depth != value;
                    app.depth = value;
                }
            }
            MenuMsg::OpenFolder(id, choice) => {
                if let Some(ItemKind::App(app)) =
                    tree::item_mut(settings, id).map(|item| &mut item.kind)
                {
                    let value = match choice {
                        1 => Some(OpenFolder::All),
                        2 => Some(OpenFolder::Never),
                        _ => None,
                    };
                    // The file may say `max-depth` outright: that is choice 0 too.
                    let same = app.open_folder == value
                        || (choice == 0 && app.open_folder == Some(OpenFolder::MaxDepth));
                    if !same {
                        app.open_folder = value;
                        changed = true;
                    }
                }
            }
            MenuMsg::InsertVariable(id, field, name) => {
                if let Some(item) = tree::item_mut(settings, id) {
                    let current = text_of(item, field);
                    changed = set_text(item, field, format!("{current}%{name}%"));
                }
            }
            MenuMsg::TurnIntoMenu(id) => {
                let before = settings.clone();
                let label = name_of(settings, id);
                if let Some(group) = tree::wrap_in_group(settings, id) {
                    self.undo = Some(Undo {
                        label: format!("Turned {label} into a menu"),
                        settings: before,
                    });
                    navigate = Some((group, id));
                    changed = true;
                }
            }

            MenuMsg::HotkeyClosed { accepted } => {
                let open = self.hotkeys.open.take();
                let item = open
                    .filter(|_| !accepted)
                    .and_then(|(id, index)| tree::item_mut(settings, id).map(|item| (item, index)))
                    .filter(|(item, index)| *index < item.hotkeys.len());
                if let Some((item, index)) = item {
                    if self.hotkeys.original.is_empty() {
                        item.hotkeys.remove(index);
                    } else {
                        item.hotkeys[index].key = std::mem::take(&mut self.hotkeys.original);
                    }
                    changed = true;
                }
                self.hotkeys = HotkeyUi::default();
            }
            MenuMsg::HotkeyAdd(id) => {
                if let Some(item) = tree::item_mut(settings, id) {
                    item.hotkeys.push(HotkeyEntry {
                        key: String::new(),
                        can_override: false,
                        extra_attrs: Vec::new(),
                    });
                    self.hotkeys = HotkeyUi {
                        open: Some((id, item.hotkeys.len() - 1)),
                        ..HotkeyUi::default()
                    };
                    changed = true;
                }
            }
            MenuMsg::HotkeyRemove(id, index) => {
                let before = settings.clone();
                if let Some(item) =
                    tree::item_mut(settings, id).filter(|item| index < item.hotkeys.len())
                {
                    let removed = item.hotkeys.remove(index);
                    let shown = if removed.key.is_empty() {
                        "a hotkey".to_owned()
                    } else {
                        removed.key
                    };
                    self.undo = Some(Undo {
                        label: format!("Removed {shown}"),
                        settings: before,
                    });
                    self.hotkeys = HotkeyUi::default();
                    changed = true;
                }
            }
            MenuMsg::HotkeyOverride(id, index, on) => {
                if let Some(entry) =
                    tree::item_mut(settings, id).and_then(|item| item.hotkeys.get_mut(index))
                {
                    changed = entry.can_override != on;
                    entry.can_override = on;
                }
            }
            MenuMsg::HotkeyKey(id, index, key) => {
                changed = set_hotkey(settings, id, index, |m, _| (m, Some(key)))
            }
            MenuMsg::HotkeyModifier(id, index, modifier) => {
                changed = set_hotkey(settings, id, index, |m, key| {
                    (HOT_KEY_MODIFIERS(m.0 ^ modifier.0), key)
                });
            }
            MenuMsg::HotkeySearchChosen(id, index, suggestion) => {
                if let Some(key) = keys::from_suggestion(&suggestion) {
                    self.hotkeys.picker = KeyPickerUi::opening_on(Some(key));
                    changed = set_hotkey(settings, id, index, |m, _| (m, Some(key)));
                }
            }

            // Settled above.
            MenuMsg::OriginDismiss
            | MenuMsg::DragOver(_)
            | MenuMsg::Select(_)
            | MenuMsg::ToggleCollapsed(_)
            | MenuMsg::MoreToggled
            | MenuMsg::AppPickerToggled
            | MenuMsg::AppBrowse
            | MenuMsg::StartOpened(..)
            | MenuMsg::StartClosed
            | MenuMsg::StartSearch(_)
            | MenuMsg::StartMore
            | MenuMsg::StartToggle(_)
            | MenuMsg::StartAdd
            | MenuMsg::XmlToggled
            | MenuMsg::HotkeyEdit(..)
            | MenuMsg::RunNow(_)
            | MenuMsg::Browse(_)
            | MenuMsg::HotkeyCategory(_)
            | MenuMsg::HotkeySearch(_) => {}
        }

        if changed {
            self.dirty = true;
        }
        if self
            .origin
            .as_ref()
            .is_some_and(|(item, _)| *item != self.selected(root))
        {
            self.origin = None;
        }
        if let Some((menu, item)) = navigate {
            self.go(Screen::Menu(menu));
            self.selection.insert(menu, item);
        }
    }
}

/// Rewrites one hotkey through `change(modifiers, key)`. The other attributes
/// of the `<hotkey>` stay as they were. Returns whether the text changed.
fn set_hotkey(
    settings: &mut y_pop::settings::Settings,
    id: ItemId,
    index: usize,
    change: impl FnOnce(
        HOT_KEY_MODIFIERS,
        Option<VIRTUAL_KEY>,
    ) -> (HOT_KEY_MODIFIERS, Option<VIRTUAL_KEY>),
) -> bool {
    let Some(entry) = tree::item_mut(settings, id).and_then(|item| item.hotkeys.get_mut(index))
    else {
        return false;
    };
    let (modifiers, key) = split_hotkey(&entry.key);
    let (modifiers, key) = change(modifiers, key);
    let text = join_hotkey(modifiers, key);
    let changed = entry.key != text;
    entry.key = text;
    changed
}

/// The raw text of one of `item`'s text fields ("" when absent).
pub(crate) fn text_of(item: &y_pop::settings::Item, field: TextField) -> String {
    let app = match &item.kind {
        ItemKind::App(app) => Some(app),
        _ => None,
    };
    let text = match field {
        TextField::Name => item.name.as_deref(),
        TextField::Tooltip => item.tooltip.as_deref(),
        TextField::Icon => item.icon.as_deref(),
        TextField::Target => app.and_then(|app| app.exe.as_deref()),
        TextField::AppId => app.and_then(|app| app.app_id.as_deref()),
        TextField::Param => app.and_then(|app| app.param.as_deref()),
        TextField::StartIn => app.and_then(|app| app.path.as_deref()),
        TextField::Filter => app.and_then(|app| app.filter.as_deref()),
    };
    text.unwrap_or_default().to_owned()
}

/// Sets one text of `item`. Whether an empty text means "the attribute is
/// absent" depends on what the attribute is: a group or an app always has a
/// name, but an empty title on a separator is a plain rule, and on About or
/// Quit it is y-pop's own default word. Returns whether anything changed.
pub(crate) fn set_text(item: &mut y_pop::settings::Item, field: TextField, text: String) -> bool {
    let absent_when_empty = |text: String| (!text.is_empty()).then_some(text);

    // Set `slot` to `new` if it differs.
    fn put(slot: &mut Option<String>, new: Option<String>) -> bool {
        if *slot == new {
            return false;
        }
        *slot = new;
        true
    }

    match field {
        TextField::Name if matches!(item.kind, ItemKind::Group | ItemKind::App(_)) => {
            put(&mut item.name, Some(text))
        }
        TextField::Name => put(&mut item.name, absent_when_empty(text)),
        TextField::Tooltip => put(&mut item.tooltip, absent_when_empty(text)),
        TextField::Icon => put(&mut item.icon, absent_when_empty(text)),
        // The target and the Store id are always present on an app, even empty.
        TextField::Target
        | TextField::AppId
        | TextField::Param
        | TextField::StartIn
        | TextField::Filter => {
            let ItemKind::App(app) = &mut item.kind else {
                return false;
            };
            match field {
                TextField::Target => put(&mut app.exe, Some(text)),
                TextField::AppId => put(&mut app.app_id, Some(text)),
                TextField::Param => put(&mut app.param, absent_when_empty(text)),
                TextField::StartIn => put(&mut app.path, absent_when_empty(text)),
                _ => put(&mut app.filter, absent_when_empty(text)),
            }
        }
    }
}
