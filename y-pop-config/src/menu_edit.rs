//! What the tray menu screen's buttons and fields do to the settings.
//!
//! Each [`MenuMsg`] is settled here, in one `match`, by calling the plain
//! functions in [`crate::tree`] (moves, inserts) or by setting one field.
//! Anything destructive keeps the whole `Settings` for Undo first; a field
//! edit only marks the file as changed when the value really differs, because
//! WinUI controls report a value when they are first shown.

use y_pop::settings::{ItemId, ItemKind, Renderer};

use crate::roots;
use crate::tree::{self, MoveDir, NewKind};
use crate::{ConfigApp, Screen, Undo};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextField {
    Name,
    Tooltip,
    Icon,
    /// An app's program, file or folder.
    Target,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlagField {
    /// Shown in bold, and what a double-click on the tray icon runs.
    Default,
    /// A tray menu that opens when the pointer rests on its icon.
    OpenOnHover,
}

pub enum MenuMsg {
    Select(ItemId),
    ToggleCollapsed(ItemId),
    Add(NewKind),
    /// The row of less common things to add.
    MoreToggled,
    /// The "written to y-pop.xml" preview.
    XmlToggled,
    Move { id: ItemId, dir: MoveDir },
    /// Into the group at this place of `tree::groups(settings, Some(id))`.
    MoveTo { id: ItemId, group: usize },
    Duplicate(ItemId),
    Delete(ItemId),
    Text(ItemId, TextField, String),
    Flag(ItemId, FlagField, bool),
    /// 0 inherit, 1 on, 2 off.
    ShowShortcuts(ItemId, usize),
    /// 0 inherit, 1 native, 2 overlay.
    PopupStyle(ItemId, usize),
    TurnIntoMenu(ItemId),
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
        let Ok(settings) = &self.settings else { return root };
        self.selection
            .get(&root)
            .copied()
            .filter(|&id| tree::is_inside(settings, root, id))
            .unwrap_or(root)
    }

    pub(crate) fn edit_menu(&mut self, message: MenuMsg) {
        let Some(root) = self.open_root() else { return };

        // Only the screen's own state.
        match message {
            MenuMsg::Select(id) => {
                self.selection.insert(root, id);
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
                return;
            }
            MenuMsg::XmlToggled => {
                self.show_xml = !self.show_xml;
                return;
            }
            _ => {}
        }

        let selected = self.selected(root);
        let Ok(settings) = &mut self.settings else { return };
        let mut changed = false;
        // Where to go after this edit, if it moved something out of this menu.
        let mut navigate: Option<(ItemId, ItemId)> = None;

        let name_of = |settings: &y_pop::settings::Settings, id: ItemId| {
            tree::item(settings, id).map(roots::label).unwrap_or_default()
        };

        match message {
            MenuMsg::Add(kind) => {
                if let Some(at) = tree::insertion_point(settings, root, Some(selected), &self.collapsed) {
                    let item = tree::new_item(settings, kind);
                    if let Some(id) = tree::insert(settings, at, vec![item]) {
                        // The new item must be visible where it landed.
                        self.collapsed.remove(&at.parent);
                        self.selection.insert(root, id);
                        self.more_open = false;
                        changed = true;
                    }
                }
            }

            MenuMsg::Move { id, dir } => {
                if tree::can_move(settings, id, dir, root) {
                    let before = settings.clone();
                    let label = name_of(settings, id);
                    let into = (dir == MoveDir::Into).then(|| tree::group_above(settings, id)).flatten();
                    if tree::move_item(settings, id, dir, root) {
                        if let Some(group) = into {
                            self.collapsed.remove(&group);
                        }
                        self.undo = Some(Undo { label: format!("Moved {label}"), settings: before });
                        changed = true;
                    }
                }
            }
            MenuMsg::MoveTo { id, group } => {
                let target = tree::groups(settings, Some(id)).get(group).map(|(_, group)| *group);
                if let Some(target) = target {
                    let before = settings.clone();
                    let label = name_of(settings, id);
                    if tree::move_to_group(settings, id, target) {
                        self.undo = Some(Undo { label: format!("Moved {label}"), settings: before });
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
                        label.push_str(&format!(" and {inside} item{} inside", if inside == 1 { "" } else { "s" }));
                    }
                    self.undo = Some(Undo { label, settings: before });
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
                    let slot = match field {
                        FlagField::Default => &mut item.default,
                        FlagField::OpenOnHover => &mut item.auto_open,
                    };
                    if *slot != on {
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
            MenuMsg::TurnIntoMenu(id) => {
                let before = settings.clone();
                let label = name_of(settings, id);
                if let Some(group) = tree::wrap_in_group(settings, id) {
                    self.undo = Some(Undo { label: format!("Turned {label} into a menu"), settings: before });
                    navigate = Some((group, id));
                    changed = true;
                }
            }

            // Settled above.
            MenuMsg::Select(_) | MenuMsg::ToggleCollapsed(_) | MenuMsg::MoreToggled | MenuMsg::XmlToggled => {}
        }

        if changed {
            self.dirty = true;
        }
        if let Some((menu, item)) = navigate {
            self.go(Screen::Menu(menu));
            self.selection.insert(menu, item);
        }
    }
}

/// Sets one text of `item`. Whether an empty text means "the attribute is
/// absent" depends on what the attribute is: a group or an app always has a
/// name, but an empty title on a separator is a plain rule, and on About or
/// Quit it is y-pop's own default word. Returns whether anything changed.
fn set_text(item: &mut y_pop::settings::Item, field: TextField, text: String) -> bool {
    let absent_when_empty = |text: String| (!text.is_empty()).then_some(text);

    let new = match field {
        TextField::Name if matches!(item.kind, ItemKind::Group | ItemKind::App(_)) => Some(text),
        TextField::Name => absent_when_empty(text),
        TextField::Tooltip | TextField::Icon => absent_when_empty(text),
        TextField::Target => {
            let ItemKind::App(app) = &mut item.kind else { return false };
            if app.exe.as_deref() == Some(text.as_str()) {
                return false;
            }
            app.exe = Some(text);
            return true;
        }
    };

    let slot = match field {
        TextField::Name => &mut item.name,
        TextField::Tooltip => &mut item.tooltip,
        TextField::Icon => &mut item.icon,
        TextField::Target => return false,
    };
    if *slot == new {
        return false;
    }
    *slot = new;
    true
}
