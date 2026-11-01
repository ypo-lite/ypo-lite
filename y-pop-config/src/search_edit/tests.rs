use y_pop::model::{ActionId, MenusBrowse, Section};
use y_pop::variables::Variable;

use super::*;

fn load(xml: &str) -> Settings {
    Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid")
}

/// What y-pop itself reads from what would be saved, with no warning.
fn read_back(settings: &Settings) -> y_pop::model::ConfigTree {
    let tree = y_pop::config::load_str(&settings.to_xml(), &mut Vec::<Variable>::new())
        .expect("a file y-pop can read");
    assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
    tree
}

fn keys(settings: &Settings) -> Vec<SectionKey> {
    sections(settings).rows.iter().map(|row| row.key).collect()
}

const MENUS: &str = r#"<group name="Work"/><group name="Admin tools"/><group name="Games"/>"#;

#[test]
fn a_search_with_nothing_in_it_lists_the_defaults() {
    let settings = load(&format!(
        r#"<y-pop><search hotkey="Alt+Space"/>{MENUS}</y-pop>"#
    ));
    let list = sections(&settings);
    assert!(list.defaults);
    assert_eq!(
        keys(&settings),
        [SectionKey::Recent, SectionKey::Menus, SectionKey::StartMenu]
    );
    assert!(list.rows.iter().all(|row| !row.written));
}

#[test]
fn actions_and_comments_alone_leave_the_defaults() {
    let settings =
        load(r#"<y-pop><search><!-- c --><actions menu-key="Ctrl+K"/></search></y-pop>"#);
    assert!(sections(&settings).defaults);
}

#[test]
fn a_menus_the_file_lacks_is_implied_after_recent_or_first() {
    let after = load(r#"<y-pop><search><recent/><start-menu/></search></y-pop>"#);
    assert_eq!(
        keys(&after),
        [SectionKey::Recent, SectionKey::Menus, SectionKey::StartMenu]
    );
    assert_eq!(
        sections(&after)
            .rows
            .iter()
            .map(|r| r.written)
            .collect::<Vec<_>>(),
        [true, false, true]
    );

    let first = load(r#"<y-pop><search><start-menu/></search></y-pop>"#);
    assert_eq!(keys(&first), [SectionKey::Menus, SectionKey::StartMenu]);
}

/// The list the screen shows is the list y-pop builds, whatever the file.
#[test]
fn the_sections_are_the_ones_y_pop_reads() {
    for xml in [
        r#"<y-pop><search/></y-pop>"#,
        r#"<y-pop><search><actions/></search></y-pop>"#,
        r#"<y-pop><search><recent/></search></y-pop>"#,
        r#"<y-pop><search><start-menu/><recent/></search></y-pop>"#,
        r#"<y-pop><search><group name="G"/><recent/></search></y-pop>"#,
        r#"<y-pop><search><menus/><app name="F" exe="x" browse="true"/></search></y-pop>"#,
    ] {
        let settings = load(xml);
        let tree = y_pop::config::load_str(xml, &mut Vec::<Variable>::new()).expect("valid");
        let theirs: Vec<&str> = tree
            .search
            .as_ref()
            .expect("search")
            .sections
            .iter()
            .map(|section| match section {
                Section::Recent { .. } => "recent",
                Section::Menus { .. } => "menus",
                Section::StartMenu { .. } => "start",
                Section::Group(_) => "item",
                Section::Folder(_) => "item",
                Section::Plugin(_) => "plugin",
            })
            .collect();
        let ours: Vec<&str> = keys(&settings)
            .iter()
            .map(|key| match key {
                SectionKey::Recent => "recent",
                SectionKey::Menus => "menus",
                SectionKey::StartMenu => "start",
                SectionKey::Plugin(_) => "plugin",
                SectionKey::Item(_) => "item",
            })
            .collect();
        assert_eq!(ours, theirs, "{xml}");
    }
}

#[test]
fn an_attribute_edit_creates_search_only_when_it_changes_something() {
    let mut settings = load("<y-pop/>");
    assert!(!set_hotkey(&mut settings, ""));
    assert!(!set_collapsed(&mut settings, None));
    assert!(!is_on(&settings));

    assert!(set_hotkey(&mut settings, "Alt+Space"));
    assert!(set_override(&mut settings, true));
    assert!(set_results(&mut settings, Some(ResultsMode::Flat)));
    assert!(set_history(&mut settings, Some(false)));
    assert!(!set_history(&mut settings, Some(false)));
    let tree = read_back(&settings);
    let search = tree.search.expect("search");
    assert!(search.hotkey.expect("a hotkey").can_override);
    assert!(!search.history);
    assert_eq!(search.results, y_pop::model::ResultsLayout::Flat);

    // Clearing the hotkey leaves `<search/>`: search stays on without one.
    assert!(set_hotkey(&mut settings, "  "));
    assert!(is_on(&settings));
    assert!(turn_off(&mut settings).is_some());
    assert!(!settings.to_xml().contains("<search"));
}

#[test]
fn the_first_edit_of_a_defaults_list_writes_the_defaults_out() {
    let mut settings = load(r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#);
    assert!(set_recent_count(&mut settings, Some(8)));

    let list = sections(&settings);
    assert!(!list.defaults);
    assert!(list.rows.iter().all(|row| row.written));
    assert_eq!(list.rows.len(), 3, "the other defaults are kept");
    let tree = read_back(&settings);
    let search = tree.search.expect("search");
    assert!(matches!(search.sections[0], Section::Recent { count: 8 }));
    assert_eq!(search.sections.len(), 3);
}

#[test]
fn setting_what_is_already_so_writes_nothing() {
    let mut settings = load(r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#);
    let before = settings.to_xml();
    assert!(!set_recent_count(&mut settings, None));
    assert!(!set_menus_browse(&mut settings, None));
    assert!(!set_start_exclude(&mut settings, ""));
    assert_eq!(settings.to_xml(), before);
}

#[test]
fn adding_a_section_keeps_the_defaults_and_refuses_a_second_one() {
    let mut settings = load(r#"<y-pop><search><recent/></search></y-pop>"#);
    assert_eq!(addable(&settings), [SectionAdd::StartMenu]);
    assert!(add_section(&mut settings, SectionAdd::StartMenu));
    assert!(!add_section(&mut settings, SectionAdd::StartMenu));
    assert!(!add_section(&mut settings, SectionAdd::Recent));
    assert_eq!(
        keys(&settings),
        [SectionKey::Recent, SectionKey::Menus, SectionKey::StartMenu]
    );
    read_back(&settings);

    // From the bare defaults: recent is already there, so only the rest.
    let mut bare = load(r#"<y-pop><search/></y-pop>"#);
    assert!(addable(&bare).is_empty());
    assert!(!add_section(&mut bare, SectionAdd::Recent));
}

#[test]
fn a_removed_section_goes_and_menus_stays() {
    let mut settings = load(r#"<y-pop><search><recent/><menus/><start-menu/></search></y-pop>"#);
    assert!(!can_remove(SectionKey::Menus));
    assert!(remove_section(&mut settings, SectionKey::Menus).is_none());
    assert!(remove_section(&mut settings, SectionKey::Recent).is_some());
    assert_eq!(keys(&settings), [SectionKey::Menus, SectionKey::StartMenu]);
    assert_eq!(addable(&settings), [SectionAdd::Recent]);

    // The list can't empty itself: `reset_sections` is the way back.
    remove_section(&mut settings, SectionKey::StartMenu);
    let mut only_menus = load(r#"<y-pop><search><menus/></search></y-pop>"#);
    assert!(!sections(&only_menus).defaults);
    assert!(remove_section(&mut only_menus, SectionKey::Menus).is_none());
}

#[test]
fn removing_what_is_not_there_writes_nothing() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let group = load(r#"<y-pop><search><menus/></search></y-pop>"#);
    assert!(remove_section(&mut settings, SectionKey::Plugin(0)).is_none());
    assert!(
        sections(&settings).defaults,
        "the defaults were not written out"
    );
    assert!(
        sections(&group)
            .rows
            .iter()
            .all(|row| row.key != SectionKey::Recent)
    );
}

#[test]
fn a_second_recent_y_pop_ignored_goes_with_the_first() {
    let mut settings =
        load(r#"<y-pop><search><recent count="3"/><recent count="9"/><menus/></search></y-pop>"#);
    assert!(remove_section(&mut settings, SectionKey::Recent).is_some());
    assert_eq!(keys(&settings), [SectionKey::Menus]);
}

#[test]
fn plugins_are_removed_one_at_a_time() {
    let mut settings = load(
        r#"<y-pop><search><menus/><plugin name="A" src="a"/><plugin name="B" src="b"/></search></y-pop>"#,
    );
    assert!(remove_section(&mut settings, SectionKey::Plugin(0)).is_some());
    assert_eq!(keys(&settings), [SectionKey::Menus, SectionKey::Plugin(0)]);
    assert!(settings.to_xml().contains(r#"name="B""#));
}

#[test]
fn sections_can_go_back_to_the_defaults() {
    let mut settings = load(&format!(
        r#"<y-pop><search><menus include="Work"/><actions menu-key="Ctrl+K"/></search>{MENUS}</y-pop>"#
    ));
    assert!(sections_customised(&settings));
    assert!(reset_sections(&mut settings));
    assert!(
        sections(&settings).defaults,
        "nothing left: y-pop's own defaults"
    );
    assert!(!sections_customised(&settings));
    assert!(!reset_sections(&mut settings));
    assert!(
        settings.to_xml().contains("Ctrl+K"),
        "the actions are another matter"
    );
}

#[test]
fn going_back_to_the_defaults_keeps_groups_and_writes_the_three() {
    let mut settings = load(
        r#"<y-pop><search><group name="G"><app name="x" exe="x.exe"/></group><recent count="9"/></search></y-pop>"#,
    );
    assert!(reset_sections(&mut settings));
    let rows = keys(&settings);
    assert_eq!(
        rows[..3],
        [SectionKey::Recent, SectionKey::Menus, SectionKey::StartMenu]
    );
    assert!(matches!(rows[3], SectionKey::Item(_)));
    assert_eq!(
        recent_count(&settings),
        y_pop::model::Section::DEFAULT_RECENT
    );
    assert!(!sections_customised(&settings));
    read_back(&settings);
}

#[test]
fn only_the_browse_list_moves_past_comments_and_actions() {
    let mut settings = load(
        r#"<y-pop><search>
            <recent/>
            <menus/>
            <!-- between -->
            <actions/>
            <app name="F" exe="C:\\f" browse="true"/>
            <group name="G"/>
            <start-menu/>
        </search></y-pop>"#,
    );
    let browse = |settings: &Settings| -> Vec<SectionKey> {
        browse_rows(settings).iter().map(|row| row.key).collect()
    };
    let group = browse(&settings)[1];
    assert_eq!(browse(&settings)[0], SectionKey::Menus);

    assert!(move_section(&mut settings, group, true));
    assert_eq!(browse(&settings), [group, SectionKey::Menus]);
    assert!(!move_section(&mut settings, group, true), "already first");
    assert!(move_section(&mut settings, SectionKey::Menus, true));
    assert!(!move_section(&mut settings, group, false), "already last");

    // y-pop never shows these as rows, so they don't move.
    assert!(!move_section(&mut settings, SectionKey::Recent, false));
    assert!(!move_section(&mut settings, SectionKey::StartMenu, true));
    let xml = settings.to_xml();
    assert!(xml.contains("between") && xml.contains("<actions"), "{xml}");
    read_back(&settings);
}

#[test]
fn a_move_that_can_not_happen_writes_nothing() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(!move_section(&mut settings, SectionKey::Menus, true));
    assert!(
        sections(&settings).defaults,
        "the defaults were not written out"
    );
}

#[test]
fn an_implied_menus_is_written_before_it_is_moved() {
    let mut settings = load(r#"<y-pop><search><recent/><group name="G"/></search></y-pop>"#);
    assert!(move_section(&mut settings, SectionKey::Menus, false));
    let rows = browse_rows(&settings);
    assert_eq!(rows[1].key, SectionKey::Menus);
    assert!(sections(&settings).rows.iter().all(|row| row.written));
}

#[test]
fn a_renamed_tray_menu_stays_in_include() {
    let mut settings = load(&format!(
        r#"<y-pop><search><menus include="Work;Admin tools"/></search>{MENUS}</y-pop>"#
    ));
    assert_eq!(
        rename_in_include(&mut settings, "Admin tools", "Tools"),
        Rename::Renamed
    );
    assert_eq!(include_names(&settings), ["Work", "Tools"]);
    // A name that can't be written waits, for the caller to try again.
    assert_eq!(
        rename_in_include(&mut settings, "Tools", "A;B"),
        Rename::Waiting
    );
    assert_eq!(
        rename_in_include(&mut settings, "Tools", ""),
        Rename::Waiting
    );
    assert_eq!(
        rename_in_include(&mut settings, "Games", "Fun"),
        Rename::NotListed
    );
    assert_eq!(include_names(&settings), ["Work", "Tools"]);

    let mut without = load(&format!(
        r#"<y-pop><search><menus/></search>{MENUS}</y-pop>"#
    ));
    assert_eq!(
        rename_in_include(&mut without, "Work", "Jobs"),
        Rename::NotListed
    );
}

#[test]
fn a_deleted_tray_menu_leaves_include() {
    fn deleted(include: &str, name: &str) -> Settings {
        let mut settings = load(&format!(
            r#"<y-pop><search><menus include="{include}"/></search>{MENUS}</y-pop>"#
        ));
        let id = crate::roots::menus(&settings)
            .find(|item| item.name.as_deref() == Some(name))
            .expect("the menu")
            .id;
        crate::roots::delete(&mut settings, id);
        drop_from_include(&mut settings, name);
        settings
    }

    let settings = deleted("Work;Games", "Work");
    assert_eq!(include_names(&settings), ["Games"]);
    assert!(
        read_back(&settings)
            .search
            .expect("search")
            .includes_menu("Games")
    );

    // The only one listed: not "nothing counts", as y-pop would read a name
    // that matches nothing, but every menu again.
    let settings = deleted("Work", "Work");
    assert!(!settings.to_xml().contains("include"));
    assert!(
        read_back(&settings)
            .search
            .expect("search")
            .includes_menu("Games")
    );
}

#[test]
fn a_menu_of_the_same_name_keeps_the_entry() {
    let mut settings =
        load(r#"<y-pop><search><menus include="Work"/></search><group name="Work"/></y-pop>"#);
    assert!(
        !drop_from_include(&mut settings, "Work"),
        "this one still exists"
    );
    assert_eq!(include_names(&settings), ["Work"]);
}

/// What y-pop keeps of each list, compared with what the screen shows.
#[test]
fn rows_y_pop_drops_are_marked() {
    let settings = load(
        r#"<y-pop><search>
            <app name="Plain" exe="x.exe"/>
            <plugin name="No src"/>
            <plugin name="Whole" src="p.js"/>
            <actions>
                <action do="nope" key="Ctrl+E"/>
                <action do="edit" key="Ctrl+E"/>
                <action do="edit" key="Ctrl+K"/>
            </actions>
        </search></y-pop>"#,
    );
    let rows = sections(&settings).rows;
    assert_eq!(
        rows.iter().map(|row| row.ignored).collect::<Vec<_>>(),
        [false, true, true, false],
        "{rows:?}"
    );
    assert!(browse_rows(&settings).iter().all(|row| !row.ignored));
    assert_eq!(other_rows(&settings).len(), 3);

    let view = actions(&settings);
    assert_eq!(
        view.rows.iter().map(|row| row.ignored).collect::<Vec<_>>(),
        [false, true, false, true],
        "open, then what was written"
    );
    // y-pop gives Ctrl+E to edit, so nothing is wrong with that row.
    assert!(
        key_problems(&settings).is_empty(),
        "{:?}",
        key_problems(&settings)
    );
}

#[test]
fn an_implied_open_has_no_enter_when_the_menu_key_has_it() {
    let xml = r#"<y-pop><search><actions menu-key="Enter"><action do="edit" key="Ctrl+E"/></actions></search></y-pop>"#;
    let settings = load(xml);
    let tree = y_pop::config::load_str(xml, &mut Vec::<Variable>::new()).expect("valid");
    let theirs = tree.search.expect("search").actions.key_of(ActionId::Open);
    assert_eq!(theirs, None);
    assert_eq!(actions(&settings).rows[0].key, None);
    assert!(!enter_is_unused(&settings), "Enter opens the menu");
}

#[test]
fn enter_is_said_to_be_unused_when_nothing_has_it() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(!enter_is_unused(&settings));
    assert!(set_action_key(&mut settings, 0, ""));
    assert!(enter_is_unused(&settings));
    assert!(set_action_key(&mut settings, 1, "Enter"));
    assert!(!enter_is_unused(&settings), "moving it elsewhere is fine");
}

#[test]
fn browse_and_the_other_fields_round_trip() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(set_menus_browse(&mut settings, Some(MenusMode::AZ)));
    assert!(set_start_exclude(&mut settings, "*uninstall*;*readme*"));
    assert_eq!(menus_browse(&settings), Some(MenusMode::AZ));
    assert_eq!(start_exclude(&settings), "*uninstall*;*readme*");

    let tree = read_back(&settings);
    let search = tree.search.expect("search");
    assert!(search.sections.iter().any(|section| matches!(
        section,
        Section::Menus {
            browse: MenusBrowse::AZ,
            ..
        }
    )));
}

// ---- actions -----------------------------------------------------------

fn ids(settings: &Settings) -> Vec<String> {
    actions(settings)
        .rows
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[test]
fn without_actions_the_rows_are_the_defaults() {
    let settings = load(r#"<y-pop><search/></y-pop>"#);
    let view = actions(&settings);
    assert!(!view.written);
    assert_eq!(view.menu_key, "Alt+Enter");
    assert!(view.quick_pick);
    assert_eq!(view.rows.len(), ActionId::ALL.len());
    assert_eq!(view.rows[0].key.as_deref(), Some("Enter"));
    assert!(view.rows.iter().all(|row| !row.implied));
}

#[test]
fn written_actions_are_exactly_those_and_open_is_implied_first() {
    let settings = load(
        r#"<y-pop><search><actions><action do="copy-path" button="true"/></actions></search></y-pop>"#,
    );
    let view = actions(&settings);
    assert_eq!(ids(&settings), ["open", "copy-path"]);
    assert!(view.rows[0].implied && !view.rows[1].implied);
    assert_eq!(view.rows[0].key.as_deref(), Some("Enter"));
    assert!(view.rows[1].button);
}

#[test]
fn the_actions_are_the_ones_y_pop_reads() {
    for xml in [
        r#"<y-pop><search/></y-pop>"#,
        r#"<y-pop><search><actions/></search></y-pop>"#,
        r#"<y-pop><search><actions><action do="edit" key="Enter"/></actions></search></y-pop>"#,
        r#"<y-pop><search><actions><action do="reveal" key="Ctrl+R"/><action do="edit" key="Ctrl+R"/></actions></search></y-pop>"#,
    ] {
        let settings = load(xml);
        let tree = y_pop::config::load_str(xml, &mut Vec::<Variable>::new()).expect("valid");
        let theirs: Vec<(String, Option<String>)> = tree
            .search
            .as_ref()
            .expect("search")
            .actions
            .actions
            .iter()
            .map(|a| (a.id.name().to_owned(), a.key.map(|k| k.to_string())))
            .collect();
        // A shortcut y-pop drops as taken is a problem, not a rewrite: compare
        // the actions, and the keys only where nothing was refused.
        let ours: Vec<String> = ids(&settings);
        let their_ids: Vec<String> = theirs.iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ours, their_ids, "{xml}");
        if key_problems(&settings).is_empty() {
            let our_keys: Vec<Option<String>> =
                actions(&settings).rows.into_iter().map(|r| r.key).collect();
            let their_keys: Vec<Option<String>> = theirs.into_iter().map(|(_, k)| k).collect();
            assert_eq!(our_keys, their_keys, "{xml}");
        }
    }
}

#[test]
fn the_first_action_edit_writes_the_defaults_out() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(set_action_key(&mut settings, 1, "Ctrl+Shift+O"));

    let view = actions(&settings);
    assert!(view.written);
    assert_eq!(
        view.rows.len(),
        ActionId::ALL.len(),
        "the other actions stay"
    );
    assert_eq!(view.rows[1].key.as_deref(), Some("Ctrl+Shift+O"));
    let tree = read_back(&settings);
    let read = &tree.search.expect("search").actions;
    assert_eq!(read.actions.len(), ActionId::ALL.len());
    assert_eq!(
        read.actions[1].key.map(|k| k.to_string()).as_deref(),
        Some("Ctrl+Shift+O")
    );
}

#[test]
fn actions_are_added_removed_and_moved_but_open_stays() {
    let mut settings = load(
        r#"<y-pop><search><actions><action do="open" key="Enter"/><!-- c --><action do="edit"/></actions></search></y-pop>"#,
    );
    assert!(!addable_actions(&settings).contains(&ActionId::Edit));
    assert!(add_action(&mut settings, ActionId::Forget));
    assert!(!add_action(&mut settings, ActionId::Forget));
    assert_eq!(ids(&settings), ["open", "edit", "forget"]);

    assert!(!remove_action(&mut settings, 0), "open can't go");
    assert!(move_action(&mut settings, 2, true));
    assert_eq!(ids(&settings), ["open", "forget", "edit"]);
    assert!(!move_action(&mut settings, 0, true));
    assert!(!move_action(&mut settings, 2, false));
    assert!(remove_action(&mut settings, 1));
    assert_eq!(ids(&settings), ["open", "edit"]);
    assert!(
        settings.to_xml().contains("<!-- c -->"),
        "the comment stays"
    );
    read_back(&settings);
}

#[test]
fn an_implied_open_is_written_when_the_list_is_edited() {
    let mut settings =
        load(r#"<y-pop><search><actions><action do="edit"/></actions></search></y-pop>"#);
    assert!(actions(&settings).rows[0].implied);
    assert!(set_action_button(&mut settings, 1, true));
    let view = actions(&settings);
    assert!(view.rows.iter().all(|row| !row.implied));
    assert_eq!(ids(&settings), ["open", "edit"]);
    assert!(view.rows[1].button);
    read_back(&settings);
}

#[test]
fn the_menu_key_and_quick_pick_are_written_only_when_not_the_default() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(!set_menu_key(&mut settings, "Alt+Enter"));
    assert!(!set_quick_pick(&mut settings, true));
    assert!(!actions(&settings).written);

    assert!(set_menu_key(&mut settings, "Ctrl+K"));
    assert!(set_quick_pick(&mut settings, false));
    let view = actions(&settings);
    assert_eq!((view.menu_key.as_str(), view.quick_pick), ("Ctrl+K", false));
    let tree = read_back(&settings);
    assert!(!tree.search.expect("search").actions.quick_pick);

    // Back to the default key removes the attribute.
    assert!(set_menu_key(&mut settings, ""));
    assert!(!settings.to_xml().contains("menu-key"));
}

#[test]
fn reset_removes_actions_and_nothing_else() {
    let mut settings =
        load(r#"<y-pop><search><recent/><actions><action do="edit"/></actions></search></y-pop>"#);
    assert!(reset_actions(&mut settings));
    assert!(!reset_actions(&mut settings));
    assert!(!actions(&settings).written);
    assert_eq!(keys(&settings), [SectionKey::Recent, SectionKey::Menus]);
}

#[test]
fn the_shortcuts_y_pop_would_refuse_are_found_with_their_row() {
    let settings = load(
        r#"<y-pop><search><actions menu-key="Ctrl+A">
            <action do="open" key="Enter"/>
            <action do="edit" key="Enter"/>
            <action do="reveal" key="R"/>
            <action do="forget" key="Ctrl+Del"/>
        </actions></search></y-pop>"#,
    );
    let problems = key_problems(&settings);
    let rows: Vec<Option<usize>> = problems.iter().map(|(row, _)| *row).collect();
    assert_eq!(rows, [None, Some(1), Some(2)], "{problems:?}");
    assert!(problems[1].1.contains("already another action's"));
    assert!(problems[2].1.contains("Ctrl or Alt"));

    // y-pop agrees on each: it warns once per problem.
    let tree =
        y_pop::config::load_str(&settings.to_xml(), &mut Vec::<Variable>::new()).expect("valid");
    assert_eq!(tree.warnings.len(), problems.len(), "{:?}", tree.warnings);
}

#[test]
fn the_defaults_have_no_key_problem() {
    assert!(key_problems(&load(r#"<y-pop><search/></y-pop>"#)).is_empty());
}
