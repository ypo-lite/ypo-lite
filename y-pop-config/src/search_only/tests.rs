use y_pop::settings::Entry;
use y_pop::variables::Variable;

use super::*;
use crate::search_edit::{self, SectionKey};
use crate::tree::{self, MoveDir};

fn load(xml: &str) -> Settings {
    Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid")
}

const XML: &str = r#"<y-pop>
    <search hotkey="Alt+Space">
        <recent/>
        <!-- tools -->
        <group name="Admin tools">
            <app name="Registry" exe="regedit.exe"/>
            <group name="Inner"><app name="Deep" exe="d.exe"/></group>
            <app name="Services" exe="services.msc"/>
        </group>
        <menus/>
        <app name="Scripts" exe="C:\Scripts" browse="true"/>
    </search>
    <group name="Work"><app name="Notepad" exe="notepad.exe"/></group>
</y-pop>"#;

/// The id of the first item called `name`, in the tray menus or the search.
fn named(settings: &Settings, name: &str) -> ItemId {
    fn walk(entries: &[y_pop::settings::Entry], name: &str) -> Option<ItemId> {
        entries.iter().find_map(|entry| match entry {
            Entry::Item(item) if item.name.as_deref() == Some(name) => Some(item.id),
            Entry::Item(item) => walk(&item.children, name),
            Entry::Comment(_) => None,
        })
    }
    walk(&settings.roots, name)
        .or_else(|| {
            roots(settings).find_map(|item| {
                if item.name.as_deref() == Some(name) {
                    Some(item.id)
                } else {
                    walk(&item.children, name)
                }
            })
        })
        .expect("an item by that name")
}

#[test]
fn search_only_items_are_found_by_id_at_any_depth() {
    let settings = load(XML);
    for name in ["Admin tools", "Registry", "Deep", "Scripts"] {
        let id = named(&settings, name);
        assert_eq!(
            tree::item(&settings, id).and_then(|item| item.name.as_deref()),
            Some(name)
        );
    }
    let admin = named(&settings, "Admin tools");
    let deep = named(&settings, "Deep");
    assert!(tree::is_inside(&settings, admin, deep));
    assert_eq!(tree::root_of(&settings, deep), Some(admin));
    assert!(tree::is_search_only(&settings, deep));
    assert!(!tree::is_search_only(
        &settings,
        named(&settings, "Notepad")
    ));

    // The top level of the list is `search_only`'s, not `tree`'s.
    assert!(tree::locate(&settings, admin).is_none());
    assert_eq!(
        tree::locate(&settings, deep).map(|l| (l.parent, l.index)),
        Some((Some(named(&settings, "Inner")), 0))
    );
    assert_eq!(
        tree::locate(&settings, named(&settings, "Services")).map(|l| (l.parent, l.index)),
        Some((Some(admin), 2))
    );
}

#[test]
fn the_top_level_is_listed_in_order_with_positions() {
    let settings = load(XML);
    let names: Vec<&str> = roots(&settings).filter_map(|i| i.name.as_deref()).collect();
    assert_eq!(names, ["Admin tools", "Scripts"]);
    let scripts = named(&settings, "Scripts");
    assert_eq!(position(&settings, scripts), Some(1));
    assert_eq!(at_position(&settings, 1), Some(scripts));
    assert_eq!(position(&settings, named(&settings, "Notepad")), None);
}

#[test]
fn edits_inside_a_search_only_group_work_like_a_tray_menu() {
    let mut settings = load(XML);
    let admin = named(&settings, "Admin tools");
    let services = named(&settings, "Services");
    let registry = named(&settings, "Registry");

    assert!(tree::item_mut(&mut settings, registry).is_some());
    assert!(tree::move_item(&mut settings, services, MoveDir::Up, admin));
    assert!(tree::move_item(&mut settings, services, MoveDir::Up, admin));
    let names: Vec<_> = tree::entries(&settings, Some(admin))
        .expect("group")
        .iter()
        .filter_map(|e| match e {
            Entry::Item(i) => i.name.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["Services", "Registry", "Inner"]);

    // A direct child of a search-only group can't leave it, as in a tray menu.
    assert!(!tree::can_move(&settings, services, MoveDir::Out, admin));

    let copy = tree::duplicate(&mut settings, registry).expect("copied");
    assert_ne!(copy, registry);
    let (removed, _) = tree::delete(&mut settings, copy).expect("deleted");
    assert_eq!(removed.name.as_deref(), Some("Registry (copy)"));

    // The sections around it are untouched.
    assert!(settings.to_xml().contains("<recent/>"));
    assert!(settings.to_xml().contains("<!-- tools -->"));
}

#[test]
fn move_to_lists_search_only_groups_and_an_item_can_go_there() {
    let mut settings = load(XML);
    let groups = tree::groups(&settings, None);
    let labels: Vec<&str> = groups.iter().map(|(label, _)| label.as_str()).collect();
    assert!(labels.contains(&"Work"));
    assert!(labels.contains(&"Only in search › Admin tools"));
    assert!(labels.contains(&"Only in search › Admin tools › Inner"));

    let notepad = named(&settings, "Notepad");
    let inner = named(&settings, "Inner");
    assert!(tree::move_to_group(&mut settings, notepad, inner));
    assert!(tree::is_search_only(&settings, notepad));
    assert!(!tree::is_inside(
        &settings,
        named(&settings, "Work"),
        notepad
    ));
}

#[test]
fn adding_writes_the_defaults_first_and_a_group_or_a_folder_only() {
    let mut settings = load(r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#);
    assert!(add(&mut settings, NewKind::App).is_none());
    assert!(add(&mut settings, NewKind::Separator).is_none());
    assert!(!search_edit::is_on(&load("<y-pop/>")));

    let group = add(&mut settings, NewKind::Group).expect("group");
    let folder = add(&mut settings, NewKind::Folder).expect("folder");
    assert_ne!(group, folder);

    let keys: Vec<_> = search_edit::sections(&settings)
        .rows
        .iter()
        .map(|row| row.key)
        .collect();
    assert_eq!(
        keys,
        [
            SectionKey::Recent,
            SectionKey::Menus,
            SectionKey::StartMenu,
            SectionKey::Item(group),
            SectionKey::Item(folder)
        ],
        "the defaults are still there"
    );

    let tree =
        y_pop::config::load_str(&settings.to_xml(), &mut Vec::<Variable>::new()).expect("valid");
    // A folder with an empty target is the editor's placeholder; y-pop reads it.
    assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
    assert_eq!(tree.search.expect("search").nodes().count(), 2);
}

#[test]
fn deleting_takes_the_group_with_its_content_and_nothing_else() {
    let mut settings = load(XML);
    let admin = named(&settings, "Admin tools");

    let (notepad, registry) = (named(&settings, "Notepad"), named(&settings, "Registry"));
    assert!(delete(&mut settings, notepad).is_none(), "a tray item");
    assert!(delete(&mut settings, registry).is_none(), "not top-level");

    let removed = delete(&mut settings, admin).expect("removed");
    assert_eq!(removed.children.len(), 3);
    assert!(tree::item(&settings, admin).is_none());
    assert!(tree::item(&settings, named(&settings, "Scripts")).is_some());
    let xml = settings.to_xml();
    assert!(
        xml.contains("<recent/>") && xml.contains("<menus/>"),
        "{xml}"
    );
    assert!(!xml.contains("Registry"));
}

#[test]
fn a_top_level_group_moves_with_the_browse_list_and_a_folder_does_not() {
    let mut settings = load(XML);
    let admin = named(&settings, "Admin tools");
    let scripts = named(&settings, "Scripts");
    assert!(search_edit::move_section(
        &mut settings,
        SectionKey::Item(admin),
        false
    ));
    let first = search_edit::browse_rows(&settings)[0].key;
    assert_eq!(first, SectionKey::Menus);
    assert!(!search_edit::move_section(
        &mut settings,
        SectionKey::Item(scripts),
        true
    ));
    assert_eq!(
        position(&settings, admin),
        Some(0),
        "among the top-level items"
    );
}

#[test]
fn everything_only_search_has_is_in_all_items() {
    let settings = load(XML);
    assert_eq!(settings.items().count(), 2, "the tray's");
    let names: Vec<String> = settings
        .all_items()
        .filter_map(|item| item.name.clone())
        .collect();
    for name in ["Work", "Notepad", "Admin tools", "Deep", "Scripts"] {
        assert!(names.iter().any(|known| known == name), "{name}: {names:?}");
    }
}

#[test]
fn ids_stay_unique_across_both_lists() {
    let mut settings = load(XML);
    let before: Vec<u32> = settings.items().map(|i| i.id.get()).collect();
    let added = add(&mut settings, NewKind::Group).expect("group").get();
    assert!(!before.contains(&added));
    let mut all: Vec<u32> = roots(&settings).map(|i| i.id.get()).collect();
    all.extend(before);
    all.sort_unstable();
    let count = all.len();
    all.dedup();
    assert_eq!(all.len(), count);
}
