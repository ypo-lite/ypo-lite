use y_pop::model::ActionId;
use y_pop::variables::Variable;

use super::*;

fn load(xml: &str) -> Settings {
    Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid")
}

fn run(settings: &mut Settings, message: SearchMsg) -> bool {
    apply(settings, &mut SearchUi::default(), message)
}

const MENUS: &str = r#"<group name="Work"/><group name="Admin tools"/><group name="Games"/>"#;

#[test]
fn turning_search_on_and_off() {
    let mut settings = load("<y-pop/>");
    assert!(run(&mut settings, SearchMsg::TurnOn));
    assert!(!run(&mut settings, SearchMsg::TurnOn), "already on");
    assert!(settings.to_xml().contains("<search"));
    assert!(run(&mut settings, SearchMsg::TurnOff));
    assert!(!settings.to_xml().contains("<search"));
}

#[test]
fn a_switch_equal_to_its_default_writes_nothing() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    // WinUI reports a switch's value when it first appears.
    assert!(!run(&mut settings, SearchMsg::Collapsed(false)));
    assert!(!run(&mut settings, SearchMsg::RowActions(true)));
    assert!(!run(&mut settings, SearchMsg::History(true)));
    assert!(!run(&mut settings, SearchMsg::Results(0)));
    assert!(!settings.to_xml().contains("collapsed"));

    assert!(run(&mut settings, SearchMsg::Collapsed(true)));
    assert!(run(&mut settings, SearchMsg::History(false)));
    assert!(run(&mut settings, SearchMsg::Results(1)));
    let xml = settings.to_xml();
    assert!(xml.contains(r#"collapsed="true""#) && xml.contains(r#"history="false""#), "{xml}");
    assert!(xml.contains(r#"results="flat""#));

    // Back to the default removes the attribute instead of writing it.
    assert!(run(&mut settings, SearchMsg::Collapsed(false)));
    assert!(run(&mut settings, SearchMsg::Results(0)));
    let xml = settings.to_xml();
    assert!(!xml.contains("collapsed") && !xml.contains("results"), "{xml}");
}

#[test]
fn the_monitor_writes_only_what_differs_from_the_cursor() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(!run(&mut settings, SearchMsg::Monitor(0)));
    assert!(!settings.to_xml().contains("monitor"));

    assert!(run(&mut settings, SearchMsg::Monitor(1)));
    assert!(settings.to_xml().contains(r#"monitor="focus""#));
    assert!(run(&mut settings, SearchMsg::Monitor(2)));
    assert!(settings.to_xml().contains(r#"monitor="primary""#));
    assert!(!run(&mut settings, SearchMsg::Monitor(2)));

    assert!(run(&mut settings, SearchMsg::Monitor(0)));
    assert!(!settings.to_xml().contains("monitor"));

    // Written out, it is already the cursor: nothing to change.
    let mut settings = load(r#"<y-pop><search monitor="cursor"/></y-pop>"#);
    assert!(!run(&mut settings, SearchMsg::Monitor(0)));
    assert!(settings.to_xml().contains(r#"monitor="cursor""#));
}

#[test]
fn a_section_opens_and_closes_without_changing_the_file() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let mut ui = SearchUi::default();
    assert!(!apply(&mut settings, &mut ui, SearchMsg::Select(SectionKey::Recent)));
    assert_eq!(ui.selected, Some(SectionKey::Recent));
    assert!(!apply(&mut settings, &mut ui, SearchMsg::Select(SectionKey::Recent)));
    assert_eq!(ui.selected, None);
}

#[test]
fn removing_the_open_section_closes_it() {
    let mut settings = load(r#"<y-pop><search><recent/><menus/></search></y-pop>"#);
    let mut ui = SearchUi {
        selected: Some(SectionKey::Recent),
        ..SearchUi::default()
    };
    assert!(apply(&mut settings, &mut ui, SearchMsg::Remove(SectionKey::Recent)));
    assert_eq!(ui.selected, None);
    assert!(!apply(&mut settings, &mut ui, SearchMsg::Remove(SectionKey::Menus)));
}

#[test]
fn the_recent_count_ignores_the_value_a_box_reports_on_opening() {
    let mut settings = load(r#"<y-pop><search><recent count="8"/><menus/></search></y-pop>"#);
    assert!(!run(&mut settings, SearchMsg::RecentCount(Some(8.0))));
    assert!(run(&mut settings, SearchMsg::RecentCount(Some(12.0))));
    assert_eq!(se::recent_count(&settings), 12);
}

#[test]
fn a_count_over_the_usual_maximum_is_not_cut_down() {
    let mut settings = load(r#"<y-pop><search><recent count="200"/><menus/></search></y-pop>"#);
    assert!(!run(&mut settings, SearchMsg::RecentCount(Some(200.0))));
    assert_eq!(se::recent_count(&settings), 200);
}

#[test]
fn recent_and_the_start_menu_are_switches() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    // A switch reports its value when it first shows.
    assert!(!run(&mut settings, SearchMsg::Recent(true)));
    assert!(!run(&mut settings, SearchMsg::StartMenu(true)));
    assert!(se::sections(&settings).defaults, "nothing written yet");

    assert!(run(&mut settings, SearchMsg::Recent(false)));
    let keys: Vec<SectionKey> = se::sections(&settings).rows.iter().map(|row| row.key).collect();
    assert_eq!(keys, [SectionKey::Menus, SectionKey::StartMenu], "the others stay");
    assert!(!run(&mut settings, SearchMsg::Recent(false)));

    assert!(run(&mut settings, SearchMsg::Recent(true)));
    assert_eq!(se::sections(&settings).rows[0].key, SectionKey::Recent);
}

#[test]
fn a_menu_with_a_semicolon_does_not_stop_the_others_from_being_chosen() {
    let mut settings = load(
        r#"<y-pop><search><menus/></search><group name="A;B"/><group name="Work"/><group name="Games"/></y-pop>"#,
    );
    assert!(run(&mut settings, SearchMsg::Include("Games".into())));
    assert_eq!(counted_menus(&settings), ["Work"]);
    assert!(settings.to_xml().contains(r#"include="Work""#));

    // Choosing all those that can be is "every menu", which counts it again.
    assert!(run(&mut settings, SearchMsg::Include("Games".into())));
    assert!(!settings.to_xml().contains("include"));
    assert_eq!(counted_menus(&settings), ["A;B", "Work", "Games"]);
}

#[test]
fn turning_off_asks_only_when_it_would_lose_work() {
    let mut plain = load(r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#);
    let mut ui = SearchUi::default();
    assert!(apply(&mut plain, &mut ui, SearchMsg::AskTurnOff), "nothing to lose");
    assert!(!se::is_on(&plain));

    let mut settings = load(r#"<y-pop><search><group name="G"/><actions/></search></y-pop>"#);
    assert!(lost_by_turning_off(&settings).expect("work").contains("your actions"));
    assert!(!apply(&mut settings, &mut ui, SearchMsg::AskTurnOff));
    assert!(ui.confirming_off && se::is_on(&settings));
    assert!(!apply(&mut settings, &mut ui, SearchMsg::KeepOn));
    assert!(!ui.confirming_off && se::is_on(&settings));

    assert!(!apply(&mut settings, &mut ui, SearchMsg::AskTurnOff));
    assert!(apply(&mut settings, &mut ui, SearchMsg::TurnOff));
    assert!(!ui.confirming_off && !se::is_on(&settings));
}

#[test]
fn removing_the_hotkey_is_one_message() {
    let mut settings = load(r#"<y-pop><search hotkey="Alt+Space" override="true"/></y-pop>"#);
    assert!(run(&mut settings, SearchMsg::RemoveHotkey));
    assert!(!run(&mut settings, SearchMsg::RemoveHotkey));
    let xml = settings.to_xml();
    assert!(!xml.contains("hotkey") && !xml.contains("override"), "{xml}");
}

#[test]
fn browse_columns_is_the_default_whether_written_or_not() {
    let mut settings = load(r#"<y-pop><search><menus browse="columns"/></search></y-pop>"#);
    assert!(!run(&mut settings, SearchMsg::Browse(0)));
    assert!(run(&mut settings, SearchMsg::Browse(1)));
    assert_eq!(se::menus_browse(&settings), Some(MenusMode::AZ));
    assert!(run(&mut settings, SearchMsg::Browse(0)));
    assert!(!settings.to_xml().contains("browse"));
}

#[test]
fn tray_menus_are_counted_one_by_one_and_the_last_one_stays() {
    let mut settings = load(&format!(r#"<y-pop><search><menus/></search>{MENUS}</y-pop>"#));
    assert_eq!(counted_menus(&settings), ["Work", "Admin tools", "Games"]);

    // Clicking one when all are counted leaves the others.
    assert!(run(&mut settings, SearchMsg::Include("Games".into())));
    assert_eq!(counted_menus(&settings), ["Work", "Admin tools"]);
    assert!(settings.to_xml().contains(r#"include="Work;Admin tools""#));

    assert!(run(&mut settings, SearchMsg::Include("Work".into())));
    assert!(!run(&mut settings, SearchMsg::Include("Admin tools".into())), "the last");
    assert_eq!(counted_menus(&settings), ["Admin tools"]);

    // Counting the rest again returns to "every menu": no include.
    assert!(run(&mut settings, SearchMsg::Include("Work".into())));
    assert!(run(&mut settings, SearchMsg::Include("Games".into())));
    assert!(!settings.to_xml().contains("include"));
}

#[test]
fn a_summary_says_what_each_row_shows() {
    let settings = load(&format!(
        r#"<y-pop><search><recent count="3"/><menus browse="a-z" include="Work"/><start-menu/>
            <group name="Tools"><app name="x" exe="x.exe"/></group>
            <app name="Scripts" exe="C:\s" browse="true"/></search>{MENUS}</y-pop>"#
    ));
    let rows = se::sections(&settings).rows;
    let summaries: Vec<String> = rows
        .iter()
        .map(|row| section_summary(&settings, row.key))
        .collect();
    assert_eq!(summaries[1], "1 of 3 tray menus, as A–Z");
    assert_eq!(summaries[3], "A group of 1");
    assert!(summaries[4].contains("found with /"));
    assert_eq!(section_title(&settings, rows[3].key), "Tools");
}

#[test]
fn the_actions_edit_through_the_messages() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    assert!(!run(&mut settings, SearchMsg::QuickPick(true)));
    assert!(run(&mut settings, SearchMsg::ClearActionKey(0)));
    assert!(!run(&mut settings, SearchMsg::ClearActionKey(0)));
    assert!(run(&mut settings, SearchMsg::ActionButton(2, true)));
    assert!(run(&mut settings, SearchMsg::MoveAction(2, true)));
    assert!(!run(&mut settings, SearchMsg::RemoveAction(0)), "open stays");
    assert!(run(&mut settings, SearchMsg::RemoveAction(8)));
    assert!(se::actions(&settings)
        .rows
        .iter()
        .all(|row| row.id != ActionId::Forget.name() || row.implied));
    assert!(run(&mut settings, SearchMsg::ResetActions));
    assert!(!se::actions(&settings).written);
}

#[test]
fn only_removals_are_offered_back_by_undo() {
    assert!(SearchMsg::TurnOff.undo_label().is_some());
    assert!(SearchMsg::Remove(SectionKey::Recent).undo_label().is_some());
    assert!(SearchMsg::RemoveAction(1).undo_label().is_some());
    assert!(SearchMsg::ResetActions.undo_label().is_some());
    assert!(SearchMsg::Recent(false).undo_label().is_some());
    assert!(SearchMsg::Recent(true).undo_label().is_none());
    assert!(SearchMsg::RemoveHotkey.undo_label().is_none());
    assert!(SearchMsg::Move(SectionKey::Recent, true).undo_label().is_none());
}

#[test]
fn edit_search_marks_the_file_changed_and_keeps_undo() {
    let settings = load(r#"<y-pop><search><recent/><menus/></search></y-pop>"#);
    let mut app = crate::test_app(settings);
    app.edit_search(SearchMsg::Collapsed(false));
    assert!(!app.dirty, "nothing changed");

    app.edit_search(SearchMsg::Remove(SectionKey::Recent));
    assert!(app.dirty);
    let undo = app.undo.take().expect("undo kept");
    assert_eq!(undo.label, "Removed a section");
    assert!(se::sections(&undo.settings).rows.iter().any(|r| r.key == SectionKey::Recent && r.written));
}

#[test]
fn adding_a_group_or_folder_asks_to_open_its_page() {
    let mut settings = load(r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#);
    let mut ui = SearchUi::default();
    assert!(apply(&mut settings, &mut ui, SearchMsg::AddGroup));
    let group = ui.goto.take().expect("opens the new group");
    assert!(crate::tree::is_search_only(&settings, group));

    assert!(apply(&mut settings, &mut ui, SearchMsg::AddFolder));
    let folder = ui.goto.take().expect("opens the new folder");
    assert_ne!(group, folder);

    // Opening an existing one changes nothing in the file.
    assert!(!apply(&mut settings, &mut ui, SearchMsg::Open(group)));
    assert_eq!(ui.goto, Some(group));
}

#[test]
fn edit_search_goes_to_the_new_page_and_a_removed_one_leaves_it() {
    let settings = load(r#"<y-pop><search hotkey="Alt+Space"/></y-pop>"#);
    let mut app = crate::test_app(settings);
    app.screen = crate::Screen::Search;

    app.edit_search(SearchMsg::AddGroup);
    let crate::Screen::SearchOnly(group) = app.screen else {
        panic!("on the group's page, not {:?}", app.screen);
    };
    assert!(app.dirty);

    // The page edits like a tray menu.
    app.edit_menu(crate::menu_edit::MenuMsg::Add(crate::tree::NewKind::App));
    let inside = app.selected(group);
    assert_ne!(inside, group);
    assert!(crate::tree::is_search_only(app.settings.as_ref().unwrap(), inside));

    // Deleting the group from its page goes back to General, with Undo.
    app.delete_menu(group);
    assert_eq!(app.screen, crate::Screen::General);
    assert!(app.undo.is_some());
}
