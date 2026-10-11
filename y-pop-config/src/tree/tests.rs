use std::collections::HashSet;

use y_pop::settings::{Entry, ItemId, ItemKind, Settings};
use y_pop::variables::Variable;

use super::*;

const MENU: &str = r#"<y-pop>
    <group name="Menu">
        <hotkey key="Win+M"/>
        <app name="A" exe="a.exe"><hotkey key="Win+A"/></app>
        <!-- free note -->

        <group name="G1">
            <app name="G1a" exe="x.exe"/>
            <app name="G1b" exe="y.exe"/>
        </group>
        <!-- about B -->
        <app name="B" exe="b.exe"/>
        <group name="G2"/>
    </group>
    <group name="Other"><app name="O" exe="o.exe"/></group>
</y-pop>"#;

fn settings() -> Settings {
    Settings::load_str(MENU, &mut Vec::<Variable>::new()).expect("valid")
}

/// An item's id from a fresh copy of the fixture: ids are handed out in
/// document order, so they are the same in every copy — and this can be used
/// in the arguments of a call that borrows `settings` mutably.
fn n(name: &str) -> ItemId {
    id(&settings(), name)
}

fn id(settings: &Settings, name: &str) -> ItemId {
    settings
        .items()
        .find(|item| item.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no item named {name}"))
        .id
}

/// The names of a list's items in order, comments shown as `#`.
fn names(settings: &Settings, parent: &str) -> Vec<String> {
    let list = entries(settings, Some(id(settings, parent))).expect("a group");
    list.iter()
        .map(|entry| match entry {
            Entry::Item(item) => item.name.clone().unwrap_or_default(),
            Entry::Comment(_) => "#".to_owned(),
        })
        .collect()
}

#[test]
fn an_item_is_located_by_its_group_and_place() {
    let settings = settings();
    let menu = id(&settings, "Menu");
    assert_eq!(locate(&settings, menu), Some(Location { parent: None, index: 0 }));
    assert_eq!(
        locate(&settings, id(&settings, "G1b")),
        Some(Location { parent: Some(id(&settings, "G1")), index: 1 })
    );
    // Comments are entries: A, #, G1, B, G2.
    assert_eq!(locate(&settings, id(&settings, "B")), Some(Location { parent: Some(menu), index: 3 }));
    let gone = settings.clone().new_id();
    assert_eq!(locate(&settings, gone), None);
}

#[test]
fn up_and_down_step_over_items_and_leave_comments_alone() {
    let mut settings = settings();
    let menu = id(&settings, "Menu");
    let g1 = id(&settings, "G1");

    // G1 is after A and a comment; up puts it where A was.
    assert!(move_item(&mut settings, g1, MoveDir::Up, menu));
    assert_eq!(names(&settings, "Menu"), ["G1", "A", "#", "B", "G2"]);

    // And back: down puts it after the next item, A.
    assert!(move_item(&mut settings, g1, MoveDir::Down, menu));
    assert_eq!(names(&settings, "Menu"), ["A", "G1", "#", "B", "G2"]);
}

#[test]
fn the_ends_refuse_and_change_nothing() {
    let mut settings = settings();
    let menu = id(&settings, "Menu");
    let before = settings.to_xml();

    assert!(!can_move(&settings, id(&settings, "A"), MoveDir::Up, menu));
    assert!(!move_item(&mut settings, n("A"), MoveDir::Up, menu));
    assert!(!move_item(&mut settings, n("G2"), MoveDir::Down, menu));
    assert!(!move_item(&mut settings, n("A"), MoveDir::Out, menu), "its group is the menu itself");
    assert!(!move_item(&mut settings, n("A"), MoveDir::Into, menu), "no group above A");
    assert_eq!(settings.to_xml(), before);
}

#[test]
fn a_tray_menu_moves_among_the_others_and_nothing_more() {
    let mut settings = settings();
    let menu = id(&settings, "Menu");
    assert!(move_item(&mut settings, menu, MoveDir::Down, menu));
    let order: Vec<String> = settings.items().filter(|i| locate(&settings, i.id).is_some_and(|l| l.parent.is_none())).map(|i| i.name.clone().unwrap()).collect();
    assert_eq!(order, ["Other", "Menu"]);

    assert!(!can_move(&settings, menu, MoveDir::Out, menu));
    assert!(!can_move(&settings, menu, MoveDir::Into, menu));
}

#[test]
fn out_puts_an_item_after_its_group() {
    let mut settings = settings();
    let menu = id(&settings, "Menu");
    assert!(move_item(&mut settings, n("G1b"), MoveDir::Out, menu));
    assert_eq!(names(&settings, "G1"), ["G1a"]);
    assert_eq!(names(&settings, "Menu"), ["A", "#", "G1", "G1b", "B", "G2"]);
}

#[test]
fn into_goes_to_the_end_of_the_nearest_group_above() {
    let mut settings = settings();
    let menu = id(&settings, "Menu");
    let b = id(&settings, "B");

    assert_eq!(group_above(&settings, b), Some(id(&settings, "G1")));
    assert!(move_item(&mut settings, b, MoveDir::Into, menu));
    assert_eq!(names(&settings, "G1"), ["G1a", "G1b", "B"]);
    assert_eq!(names(&settings, "Menu"), ["A", "#", "G1", "G2"]);
}

#[test]
fn move_to_crosses_tray_menus_and_refuses_going_inside_itself() {
    let mut settings = settings();
    let g1 = id(&settings, "G1");

    assert!(move_to_group(&mut settings, n("B"), n("Other")));
    assert_eq!(names(&settings, "Other"), ["O", "B"]);

    assert!(!move_to_group(&mut settings, g1, g1), "into itself");
    assert!(!move_to_group(&mut settings, g1, n("G1a")), "G1a is not a group");
    let before = settings.to_xml();
    assert!(!move_to_group(&mut settings, n("Menu"), g1), "into its own descendant");
    assert_eq!(settings.to_xml(), before);
}

#[test]
fn the_move_to_list_has_every_group_with_its_path() {
    let settings = settings();
    let listed: Vec<String> = groups(&settings, None).into_iter().map(|(path, _)| path).collect();
    assert_eq!(listed, ["Menu", "Menu › G1", "Menu › G2", "Other"]);

    // Without a group and what is inside it.
    let g1 = id(&settings, "G1");
    let listed: Vec<String> = groups(&settings, Some(g1)).into_iter().map(|(path, _)| path).collect();
    assert_eq!(listed, ["Menu", "Menu › G2", "Other"]);
}

#[test]
fn a_copy_has_new_ids_no_hotkeys_and_no_attached_comments() {
    let mut settings = settings();
    let g1 = id(&settings, "G1");
    let b = id(&settings, "B");

    let copy = duplicate(&mut settings, b).expect("an item in a menu");
    assert_ne!(copy, b);
    let copied = item(&settings, copy).expect("there");
    assert_eq!(copied.name.as_deref(), Some("B (copy)"));
    assert!(copied.lead.is_empty(), "the comment about B is B's");
    assert_eq!(names(&settings, "Menu"), ["A", "#", "G1", "B", "B (copy)", "G2"]);

    // Everything inside a copied group is new too, and only it says "(copy)".
    let group_copy = duplicate(&mut settings, g1).expect("a group");
    assert_eq!(names(&settings, "Menu").iter().filter(|n| n.as_str() == "G1 (copy)").count(), 1);
    let inner: Vec<String> = item(&settings, group_copy)
        .expect("there")
        .children
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => item.name.clone(),
            Entry::Comment(_) => None,
        })
        .collect();
    assert_eq!(inner, ["G1a", "G1b"]);

    let mut ids: Vec<ItemId> = settings.items().map(|i| i.id).collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count, "every id is still unique");

    // The hotkey of A is not copied.
    let a_copy = duplicate(&mut settings, n("A")).expect("an item");
    assert!(item(&settings, a_copy).expect("there").hotkeys.is_empty());
    assert_eq!(item(&settings, id(&settings, "A")).expect("there").hotkeys.len(), 1);
}

#[test]
fn a_tray_menu_is_not_duplicated() {
    let mut settings = settings();
    assert_eq!(duplicate(&mut settings, n("Menu")), None);
}

#[test]
fn deleting_takes_the_attached_comment_and_picks_what_to_select() {
    let mut settings = settings();
    let b = id(&settings, "B");

    let (removed, next) = delete(&mut settings, b).expect("there");
    assert_eq!(removed.lead.len(), 1);
    assert_eq!(next, Some(id(&settings, "G1")), "the item before it");
    assert!(!settings.to_xml().contains("about B"));

    // The first item falls back on the one after it...
    let (_, next) = delete(&mut settings, n("A")).expect("there");
    assert_eq!(next, Some(id(&settings, "G1")));
    // ...and the only child on its group.
    let (_, next) = delete(&mut settings, n("O")).expect("there");
    assert_eq!(next, Some(id(&settings, "Other")));

    assert!(delete(&mut settings, b).is_none(), "already gone");
}

fn closed() -> HashSet<ItemId> {
    HashSet::new()
}

#[test]
fn new_items_go_after_the_selection_or_inside_an_open_group() {
    let settings = settings();
    let menu = id(&settings, "Menu");
    let g1 = id(&settings, "G1");
    let g1a = id(&settings, "G1a");
    let b = id(&settings, "B");

    // Nothing selected, or the menu itself: the end of the menu.
    let end = Insertion { parent: menu, index: 5 };
    assert_eq!(insertion_point(&settings, menu, None, &closed()), Some(end));
    assert_eq!(insertion_point(&settings, menu, Some(menu), &closed()), Some(end));

    // An item: right after it.
    assert_eq!(insertion_point(&settings, menu, Some(b), &closed()), Some(Insertion { parent: menu, index: 4 }));
    assert_eq!(insertion_point(&settings, menu, Some(g1a), &closed()), Some(Insertion { parent: g1, index: 1 }));

    // An open group: inside, at its end. A collapsed one: after it.
    assert_eq!(insertion_point(&settings, menu, Some(g1), &closed()), Some(Insertion { parent: g1, index: 2 }));
    let collapsed: HashSet<ItemId> = [g1].into();
    assert_eq!(insertion_point(&settings, menu, Some(g1), &collapsed), Some(Insertion { parent: menu, index: 3 }));

    // A selection from another menu does not count here.
    assert_eq!(insertion_point(&settings, menu, Some(id(&settings, "O")), &closed()), Some(end));
}

#[test]
fn inserted_items_land_in_order_and_the_last_is_reported() {
    let mut settings = settings();
    let menu = id(&settings, "Menu");
    let first = new_item(&mut settings, NewKind::Separator);
    let second = new_item(&mut settings, NewKind::Titled);
    let second_id = second.id;

    let at = Insertion { parent: menu, index: 1 };
    assert_eq!(insert(&mut settings, at, vec![first, second]), Some(second_id));
    assert_eq!(names(&settings, "Menu"), ["A", "", "Section", "#", "G1", "B", "G2"]);
    assert_eq!(insert(&mut settings, at, vec![]), None);
}

#[test]
fn new_items_are_what_the_toolbar_says() {
    let mut settings = Settings::empty();
    let mut kinds = |kind| new_item(&mut settings, kind);

    assert!(matches!(kinds(NewKind::Group).kind, ItemKind::Group));
    let app = kinds(NewKind::App);
    assert!(matches!(&app.kind, ItemKind::App(fields) if !fields.browse && fields.exe.is_some()));
    assert!(matches!(&kinds(NewKind::Folder).kind, ItemKind::App(fields) if fields.browse));
    assert_eq!(kinds(NewKind::Separator).name, None, "a plain rule");
    assert_eq!(kinds(NewKind::Titled).name.as_deref(), Some("Section"));
    assert_eq!(kinds(NewKind::Quit).name, None, "keeps y-pop's own default title");
    assert!(matches!(kinds(NewKind::Break).kind, ItemKind::Break));
}

#[test]
fn the_outline_shows_the_tree_with_its_marker() {
    let settings = settings();
    let menu = id(&settings, "Menu");
    let b = id(&settings, "B");
    let at = insertion_point(&settings, menu, Some(b), &closed());

    let outline = rows(&settings, menu, &closed(), true, at);
    let shape: Vec<String> = outline
        .iter()
        .map(|row| {
            let label = match &row.kind {
                TreeRowKind::Item { id, .. } => item(&settings, *id).and_then(|i| i.name.clone()).unwrap_or_default(),
                TreeRowKind::Comment { attached, first_line, .. } => {
                    format!("{}{first_line}", if *attached { "^" } else { "#" })
                }
                TreeRowKind::Marker => "<here>".to_owned(),
                TreeRowKind::HiddenAtEnd(n) => format!("({n} hidden)"),
            };
            format!("{}{label}", "  ".repeat(row.depth))
        })
        .collect();
    assert_eq!(
        shape,
        [
            "Menu",
            "  A",
            "  #free note",
            "  G1",
            "    G1a",
            "    G1b",
            "  ^about B",
            "  B",
            "  <here>",
            "  G2",
        ]
    );
}

#[test]
fn a_collapsed_group_hides_its_children_but_the_menu_itself_never_collapses() {
    let settings = settings();
    let menu = id(&settings, "Menu");
    let g1 = id(&settings, "G1");

    let collapsed: HashSet<ItemId> = [g1, menu].into();
    let outline = rows(&settings, menu, &collapsed, true, None);
    let items: Vec<ItemId> = outline
        .iter()
        .filter_map(|row| match row.kind {
            TreeRowKind::Item { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert!(items.contains(&g1));
    assert!(!items.contains(&id(&settings, "G1a")));
    assert!(items.contains(&id(&settings, "B")), "the menu still lists its own children");
}

#[test]
fn hidden_comments_are_counted_on_the_item_they_belong_to() {
    let settings = settings();
    let menu = id(&settings, "Menu");
    let outline = rows(&settings, menu, &closed(), false, None);

    assert!(outline.iter().all(|row| !matches!(row.kind, TreeRowKind::Comment { .. })));
    let hidden = |name: &str| {
        let wanted = id(&settings, name);
        outline.iter().find_map(|row| match row.kind {
            TreeRowKind::Item { id, hidden_comments } if id == wanted => Some(hidden_comments),
            _ => None,
        })
    };
    // The free note counts towards the next item (G1), B's own comment on B.
    assert_eq!(hidden("G1"), Some(1));
    assert_eq!(hidden("B"), Some(1));
    assert_eq!(hidden("A"), Some(0));
}

#[test]
fn comments_with_nothing_after_them_get_a_row_of_their_own() {
    let settings = Settings::load_str(
        "<y-pop><group name=\"M\"><app name=\"A\" exe=\"a\"/><!-- the end --></group></y-pop>",
        &mut Vec::<Variable>::new(),
    )
    .expect("valid");
    let menu = id(&settings, "M");
    let hidden = rows(&settings, menu, &closed(), false, None);
    assert!(matches!(hidden.last().map(|row| &row.kind), Some(TreeRowKind::HiddenAtEnd(1))));
    let shown = rows(&settings, menu, &closed(), true, None);
    assert!(matches!(shown.last().map(|row| &row.kind), Some(TreeRowKind::Comment { .. })));
}

#[test]
fn counts_cover_everything_inside() {
    let settings = settings();
    let menu = id(&settings, "Menu");
    assert_eq!(count_inside(&settings, menu), 6, "A, G1, G1a, G1b, B, G2");
    assert_eq!(count_comments(&settings, menu), 2, "the free note and B's");
    assert_eq!(count_comments(&settings, id(&settings, "Other")), 0);
}

#[test]
fn an_item_knows_its_tray_menu() {
    let settings = settings();
    assert_eq!(root_of(&settings, id(&settings, "G1b")), Some(id(&settings, "Menu")));
    assert_eq!(root_of(&settings, id(&settings, "O")), Some(id(&settings, "Other")));
    assert_eq!(root_of(&settings, id(&settings, "Menu")), Some(id(&settings, "Menu")));
}

#[test]
fn a_root_app_becomes_a_menu_holding_it() {
    let mut settings = Settings::load_str(
        "<y-pop><group name=\"First\"/>\n\n<!-- the task manager -->\n<app name=\"Task Manager\" exe=\"taskmgr.exe\"><hotkey key=\"Win+T\"/></app></y-pop>",
        &mut Vec::<Variable>::new(),
    )
    .expect("valid");
    let app = id(&settings, "Task Manager");

    let group = wrap_in_group(&mut settings, app).expect("a root");
    assert_eq!(locate(&settings, group), Some(Location { parent: None, index: 1 }));
    let wrapper = item(&settings, group).expect("there");
    assert_eq!(wrapper.name.as_deref(), Some("Task Manager"));
    assert_eq!(wrapper.lead.len(), 1, "the comment is about the whole thing");
    assert!(wrapper.blank_before);

    let inner = item(&settings, app).expect("still there");
    assert!(inner.lead.is_empty());
    assert_eq!(inner.hotkeys.len(), 1, "it still launches with its key");
    assert_eq!(locate(&settings, app), Some(Location { parent: Some(group), index: 0 }));

    // y-pop loads what was written: two tray icons, the second a menu.
    let tree = y_pop::config::load_str(&settings.to_xml(), &mut Vec::<Variable>::new()).expect("valid");
    assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
    assert_eq!(tree.roots.len(), 2);

    // Not for something already inside a menu.
    let mut settings = self::settings();
    assert_eq!(wrap_in_group(&mut settings, n("B")), None);
}
