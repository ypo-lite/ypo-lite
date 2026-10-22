//! The menu screen's edits, through the same messages its buttons send.

use y_pop::settings::{Entry, ItemKind};
use y_pop::variables::Variable;

use super::*;
use crate::menu_edit::{FlagField, MenuMsg, TextField};
use crate::tree::{self, MoveDir, NewKind};

const XML: &str = r#"<y-pop>
    <group name="Menu">
        <app name="A" exe="a.exe"/>
        <group name="G1"><app name="G1a" exe="x.exe"/></group>
        <app name="B" exe="b.exe"/>
    </group>
    <group name="Other"/>
    <app name="Task" exe="t.exe"/>
</y-pop>"#;

fn app() -> ConfigApp {
    let settings = Settings::load_str(XML, &mut Vec::<Variable>::new()).expect("valid");
    let root = roots::menus(&settings).next().expect("a menu").id;
    let mut app = test_app(settings);
    app.screen = Screen::Menu(root);
    app
}

fn id(app: &ConfigApp, name: &str) -> ItemId {
    let settings = app.settings.as_ref().expect("loaded");
    settings.items().find(|item| item.name.as_deref() == Some(name)).expect("an item by that name").id
}

fn settings(app: &ConfigApp) -> &Settings {
    app.settings.as_ref().expect("loaded")
}

/// The names of a group's items, in order.
fn names(app: &ConfigApp, parent: &str) -> Vec<String> {
    let list = tree::entries(settings(app), Some(id(app, parent))).expect("a group");
    list.iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item.name.clone().unwrap_or_default()),
            Entry::Comment(_) => None,
        })
        .collect()
}

fn root(app: &ConfigApp) -> ItemId {
    app.open_root().expect("a menu is open")
}

/// What pressing Undo does, without a `ComponentContext`.
fn undo(app: &mut ConfigApp) {
    if let Some(undo) = app.undo.take() {
        app.settings = Ok(undo.settings);
        app.dirty = true;
    }
}

#[test]
fn nothing_selected_means_the_menu_itself_and_new_items_go_to_its_end() {
    let mut app = app();
    assert_eq!(app.selected(root(&app)), root(&app));

    app.edit_menu(MenuMsg::Add(NewKind::Group));
    assert_eq!(names(&app, "Menu"), ["A", "G1", "B", "New group"]);
    assert!(app.dirty);
    assert_eq!(app.selected(root(&app)), id(&app, "New group"), "the new item is selected");
}

#[test]
fn adding_follows_the_selection() {
    let mut app = app();
    let menu = root(&app);

    // After a selected item.
    app.edit_menu(MenuMsg::Select(id(&app, "A")));
    app.edit_menu(MenuMsg::Add(NewKind::Separator));
    assert_eq!(names(&app, "Menu"), ["A", "", "G1", "B"]);

    // Inside a selected open group...
    app.edit_menu(MenuMsg::Select(id(&app, "G1")));
    app.edit_menu(MenuMsg::Add(NewKind::Quit));
    assert_eq!(names(&app, "G1"), ["G1a", ""]);

    // ...but after it once it is folded.
    app.edit_menu(MenuMsg::Select(id(&app, "G1")));
    app.edit_menu(MenuMsg::ToggleCollapsed(id(&app, "G1")));
    app.edit_menu(MenuMsg::Add(NewKind::Titled));
    assert_eq!(names(&app, "Menu"), ["A", "", "G1", "Section", "B"]);
    assert_eq!(app.selected(menu), id(&app, "Section"));
}

#[test]
fn moving_is_undoable_and_names_what_moved() {
    let mut app = app();
    app.edit_menu(MenuMsg::Move { id: id(&app, "B"), dir: MoveDir::Up });
    assert_eq!(names(&app, "Menu"), ["A", "B", "G1"]);
    assert_eq!(app.undo.as_ref().map(|u| u.label.as_str()), Some("Moved B"));
    assert!(app.dirty);

    undo(&mut app);
    assert_eq!(names(&app, "Menu"), ["A", "G1", "B"]);

    // A move the tree refuses is not an edit, and not an Undo.
    app.dirty = false;
    app.undo = None;
    app.edit_menu(MenuMsg::Move { id: id(&app, "A"), dir: MoveDir::Up });
    assert!(!app.dirty && app.undo.is_none());
}

#[test]
fn into_a_group_opens_it() {
    let mut app = app();
    let g1 = id(&app, "G1");
    app.edit_menu(MenuMsg::ToggleCollapsed(g1));
    assert!(app.collapsed.contains(&g1));

    app.edit_menu(MenuMsg::Move { id: id(&app, "B"), dir: MoveDir::Into });
    assert_eq!(names(&app, "G1"), ["G1a", "B"]);
    assert!(!app.collapsed.contains(&g1), "so the moved item is visible");
}

#[test]
fn moving_to_a_group_in_another_menu_follows_the_item() {
    let mut app = app();
    let b = id(&app, "B");
    let other = id(&app, "Other");

    let choices = tree::groups(settings(&app), Some(b));
    let index = choices.iter().position(|(_, group)| *group == other).expect("listed");
    app.edit_menu(MenuMsg::MoveTo { id: b, group: index });

    assert_eq!(names(&app, "Other"), ["B"]);
    assert_eq!(app.screen, Screen::Menu(other), "the screen goes where it went");
    assert_eq!(app.selected(other), b);
}

#[test]
fn deleting_selects_a_neighbour_and_can_be_undone() {
    let mut app = app();
    let menu = root(&app);
    app.edit_menu(MenuMsg::Select(id(&app, "B")));
    app.edit_menu(MenuMsg::Delete(id(&app, "B")));

    assert_eq!(names(&app, "Menu"), ["A", "G1"]);
    assert_eq!(app.selected(menu), id(&app, "G1"));
    assert_eq!(app.undo.as_ref().map(|u| u.label.as_str()), Some("Deleted B"));

    app.edit_menu(MenuMsg::Delete(id(&app, "G1")));
    assert_eq!(app.undo.as_ref().map(|u| u.label.as_str()), Some("Deleted G1 and 1 item inside"));
    undo(&mut app);
    assert_eq!(names(&app, "Menu"), ["A", "G1"]);
}

#[test]
fn a_selection_that_is_gone_falls_back_to_the_menu() {
    let mut app = app();
    let menu = root(&app);
    app.edit_menu(MenuMsg::Select(id(&app, "G1a")));
    // Gone by some other route than the Delete button (Undo, a Reload).
    let g1 = id(&app, "G1");
    tree::delete(app.settings.as_mut().expect("loaded"), g1);
    assert_eq!(app.selected(menu), menu);
}

#[test]
fn duplicating_selects_the_copy() {
    let mut app = app();
    app.edit_menu(MenuMsg::Duplicate(id(&app, "A")));
    assert_eq!(names(&app, "Menu"), ["A", "A (copy)", "G1", "B"]);
    assert_eq!(app.selected(root(&app)), id(&app, "A (copy)"));
}

#[test]
fn fields_are_edited_in_place_and_an_unchanged_value_is_not_an_edit() {
    let mut app = app();
    let a = id(&app, "A");

    app.edit_menu(MenuMsg::Text(a, TextField::Name, "A".into()));
    app.edit_menu(MenuMsg::Text(a, TextField::Tooltip, String::new()));
    app.edit_menu(MenuMsg::Flag(a, FlagField::Default, false));
    app.edit_menu(MenuMsg::ShowShortcuts(a, 0));
    assert!(!app.dirty);

    app.edit_menu(MenuMsg::Text(a, TextField::Name, "Alpha".into()));
    app.edit_menu(MenuMsg::Text(a, TextField::Target, "%win%\\alpha.exe".into()));
    app.edit_menu(MenuMsg::Text(a, TextField::Tooltip, "tip".into()));
    app.edit_menu(MenuMsg::Flag(a, FlagField::Default, true));
    app.edit_menu(MenuMsg::ShowShortcuts(a, 2));

    let item = tree::item(settings(&app), a).expect("there");
    assert_eq!(item.name.as_deref(), Some("Alpha"));
    assert_eq!(item.tooltip.as_deref(), Some("tip"));
    assert!(item.default);
    assert_eq!(item.show_shortcuts, Some(false));
    assert!(matches!(&item.kind, ItemKind::App(a) if a.exe.as_deref() == Some("%win%\\alpha.exe")));
    assert!(app.dirty);
}

#[test]
fn an_empty_title_means_the_default_where_the_default_is_a_thing() {
    let mut app = app();
    app.edit_menu(MenuMsg::Add(NewKind::Titled));
    let title = app.selected(root(&app));
    app.edit_menu(MenuMsg::Text(title, TextField::Name, String::new()));
    assert_eq!(tree::item(settings(&app), title).expect("there").name, None, "a plain rule");

    // A group always has a name, even an empty one.
    let group = id(&app, "G1");
    app.edit_menu(MenuMsg::Text(group, TextField::Name, String::new()));
    assert_eq!(tree::item(settings(&app), group).expect("there").name.as_deref(), Some(""));
}

#[test]
fn a_root_app_can_become_a_menu() {
    let mut app = app();
    let task = id(&app, "Task");
    app.go(Screen::Menu(task));

    app.edit_menu(MenuMsg::TurnIntoMenu(task));
    let Screen::Menu(menu) = app.screen else { panic!("a menu screen") };
    assert_ne!(menu, task);
    assert_eq!(app.selected(menu), task, "the app is what is selected inside");
    assert_eq!(tree::item(settings(&app), menu).and_then(|i| i.name.clone()).as_deref(), Some("Task"));
    assert_eq!(app.undo.as_ref().map(|u| u.label.as_str()), Some("Turned Task into a menu"));
}

#[test]
fn a_tray_menu_moves_among_the_others_and_its_screen_stays_open() {
    let mut app = app();
    let menu = root(&app);
    app.edit_menu(MenuMsg::Move { id: menu, dir: MoveDir::Down });

    let order: Vec<String> = roots::menus(settings(&app)).map(roots::label).collect();
    assert_eq!(order, ["Other", "Menu", "Task"]);
    assert_eq!(app.screen, Screen::Menu(menu), "it is addressed by id");
}

fn hotkeys_of(app: &ConfigApp, name: &str) -> Vec<(String, bool)> {
    let item = tree::item(settings(app), id(app, name)).expect("an item");
    item.hotkeys.iter().map(|entry| (entry.key.clone(), entry.can_override)).collect()
}

#[test]
fn a_hotkey_is_built_from_modifiers_then_a_key_and_can_be_removed_with_undo() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_WIN, VK_F5};

    let mut app = app();
    let a = id(&app, "A");
    app.edit_menu(MenuMsg::HotkeyAdd(a));
    assert_eq!(app.hotkeys.open, Some((a, 0)));
    assert_eq!(hotkeys_of(&app, "A"), [(String::new(), false)]);

    // Chips before the key: the modifiers are kept in the text.
    app.edit_menu(MenuMsg::HotkeyModifier(a, 0, MOD_WIN));
    app.edit_menu(MenuMsg::HotkeyModifier(a, 0, MOD_CONTROL));
    assert_eq!(hotkeys_of(&app, "A")[0].0, "Win+Ctrl");
    app.edit_menu(MenuMsg::HotkeyKey(a, 0, VK_F5));
    assert_eq!(hotkeys_of(&app, "A")[0].0, "Win+Ctrl+F5");
    app.edit_menu(MenuMsg::HotkeyModifier(a, 0, MOD_WIN));
    assert_eq!(hotkeys_of(&app, "A")[0].0, "Ctrl+F5");

    app.edit_menu(MenuMsg::HotkeyOverride(a, 0, true));
    assert_eq!(hotkeys_of(&app, "A"), [("Ctrl+F5".to_owned(), true)]);

    app.edit_menu(MenuMsg::HotkeyRemove(a, 0));
    assert!(hotkeys_of(&app, "A").is_empty());
    assert_eq!(app.hotkeys.open, None);
    app.update_undo_for_test();
    assert_eq!(hotkeys_of(&app, "A").len(), 1);
}

#[test]
fn hotkey_texts_split_and_join_both_ways() {
    use crate::menu_edit::{canonical_hotkey, join_hotkey, split_hotkey};

    let (modifiers, key) = split_hotkey("win+SHIFT+f12");
    assert_eq!(join_hotkey(modifiers, key), "Win+Shift+F12");
    let (modifiers, key) = split_hotkey("Alt");
    assert!(key.is_none());
    assert_eq!(join_hotkey(modifiers, key), "Alt");
    assert_eq!(canonical_hotkey("ctrl+alt+a"), canonical_hotkey("Alt+Ctrl+A"));
    assert_eq!(canonical_hotkey("Ctrl+"), None);
}

#[test]
fn cancelling_the_hotkey_dialog_restores_the_hotkey_and_drops_a_new_one() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F5, VK_F6};

    let mut app = app();
    let a = id(&app, "A");

    // A new hotkey that is cancelled never reaches the file.
    app.edit_menu(MenuMsg::HotkeyAdd(a));
    app.edit_menu(MenuMsg::HotkeyKey(a, 0, VK_F5));
    app.edit_menu(MenuMsg::HotkeyClosed { accepted: false });
    assert!(hotkeys_of(&app, "A").is_empty());
    assert_eq!(app.hotkeys.open, None);

    // OK keeps it; editing it again and cancelling goes back to it.
    app.edit_menu(MenuMsg::HotkeyAdd(a));
    app.edit_menu(MenuMsg::HotkeyKey(a, 0, VK_F5));
    app.edit_menu(MenuMsg::HotkeyClosed { accepted: true });
    app.edit_menu(MenuMsg::HotkeyEdit(a, 0));
    app.edit_menu(MenuMsg::HotkeyKey(a, 0, VK_F6));
    assert_eq!(hotkeys_of(&app, "A")[0].0, "F6");
    app.edit_menu(MenuMsg::HotkeyClosed { accepted: false });
    assert_eq!(hotkeys_of(&app, "A")[0].0, "F5");
}

fn app_fields<'a>(app: &'a ConfigApp, name: &str) -> &'a y_pop::settings::AppFields {
    match &tree::item(settings(app), id(app, name)).expect("an item").kind {
        ItemKind::App(fields) => fields,
        other => panic!("not an app: {other:?}"),
    }
}

#[test]
fn switching_the_target_mode_keeps_what_was_typed_in_the_other_one() {
    use crate::menu_edit::TargetMode;

    let mut app = app();
    let a = id(&app, "A");
    app.edit_menu(MenuMsg::TargetMode(a, TargetMode::Store));
    assert_eq!(app_fields(&app, "A").exe, None);
    assert_eq!(app_fields(&app, "A").app_id.as_deref(), Some(""));
    app.edit_menu(MenuMsg::Text(a, TextField::AppId, "Pub.App_x!App".into()));

    app.edit_menu(MenuMsg::TargetMode(a, TargetMode::Folder));
    assert_eq!(app_fields(&app, "A").exe.as_deref(), Some("a.exe"));
    assert!(app_fields(&app, "A").browse && app_fields(&app, "A").app_id.is_none());

    app.edit_menu(MenuMsg::TargetMode(a, TargetMode::Store));
    assert_eq!(app_fields(&app, "A").app_id.as_deref(), Some("Pub.App_x!App"));
    assert!(!app_fields(&app, "A").browse);
}

#[test]
fn launch_and_folder_fields_set_and_clear_their_attributes() {
    let mut app = app();
    let a = id(&app, "A");

    app.edit_menu(MenuMsg::Text(a, TextField::Param, "--new".into()));
    app.edit_menu(MenuMsg::InsertVariable(a, TextField::Param, "clipboard".into()));
    assert_eq!(app_fields(&app, "A").param.as_deref(), Some("--new%clipboard%"));
    app.edit_menu(MenuMsg::Text(a, TextField::Param, String::new()));
    assert_eq!(app_fields(&app, "A").param, None);

    app.edit_menu(MenuMsg::Flag(a, FlagField::Elevate, true));
    app.edit_menu(MenuMsg::ShellExecute(a, 2));
    assert!(app_fields(&app, "A").elevate);
    assert_eq!(app_fields(&app, "A").shell, Some(false));
    app.edit_menu(MenuMsg::ShellExecute(a, 0));
    assert_eq!(app_fields(&app, "A").shell, None);

    app.edit_menu(MenuMsg::Depth(a, Some(3.0)));
    app.edit_menu(MenuMsg::OpenFolder(a, 1));
    assert_eq!(app_fields(&app, "A").depth, Some(3));
    assert_eq!(app_fields(&app, "A").open_folder, Some(y_pop::model::OpenFolder::All));
    app.edit_menu(MenuMsg::Depth(a, None));
    assert_eq!(app_fields(&app, "A").depth, None);
    assert_eq!(crate::menu_edit::depth_words(0), "files only, no subfolders");
    assert_eq!(crate::menu_edit::depth_words(1), "1 level of subfolders");
}

fn icon_of(app: &ConfigApp, name: &str) -> Option<String> {
    tree::item(settings(app), id(app, name)).expect("an item").icon.clone()
}

#[test]
fn the_icon_picker_commits_live_and_cancel_puts_the_old_value_back() {
    use crate::icon_picker::{IconMsg, Source};

    let mut app = app();
    let a = id(&app, "A");

    app.edit_icon(IconMsg::Open(a));
    assert_eq!(app.icon_picker.open, Some(a));
    // `a.exe` is not a file: its own program offers nothing to pick from.
    assert!(app.icon_picker.visible().is_empty());

    app.edit_icon(IconMsg::Source(Source::Named(0)));
    assert_eq!(app.icon_picker.visible().len(), 96);
    app.edit_icon(IconMsg::Pick(Some(3)));
    assert_eq!(icon_of(&app, "A").as_deref(), Some(r"%system%\shell32.dll,3"));
    assert!(app.dirty);

    // Next page, then Cancel: the item is as it was.
    app.edit_icon(IconMsg::Page(1));
    assert_eq!(app.icon_picker.visible().len(), 96);
    app.edit_icon(IconMsg::Closed { accepted: false });
    assert_eq!(icon_of(&app, "A"), None);
    assert_eq!(app.icon_picker.open, None);

    // OK keeps; it reopens on the source the icon already comes from.
    app.edit_icon(IconMsg::Open(a));
    app.edit_icon(IconMsg::Source(Source::Named(1)));
    app.edit_icon(IconMsg::Pick(Some(7)));
    app.edit_icon(IconMsg::Closed { accepted: true });
    assert_eq!(icon_of(&app, "A").as_deref(), Some(r"%system%\imageres.dll,7"));
    app.edit_icon(IconMsg::Open(a));
    assert_eq!(app.icon_picker.source, Some(Source::Named(1)));

    // "From target" clears the attribute.
    app.edit_icon(IconMsg::Pick(None));
    assert_eq!(icon_of(&app, "A"), None);
}
