use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_CONTROL, MOD_WIN, VK_A, VK_DELETE, VK_E, VK_F4, VK_F5, VK_F13, VK_K, VK_RETURN, VK_SPACE,
    VK_UP,
};
use y_pop::variables::Variable;

use super::*;

fn load(xml: &str) -> Settings {
    Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid")
}

fn open(settings: &mut Settings, target: KeyTarget) -> KeyEditUi {
    let mut ui = KeyEditUi::default();
    apply(settings, &mut ui, KeyMsg::Open(target));
    assert_eq!(ui.open, Some(target));
    ui
}

fn choose(settings: &mut Settings, ui: &mut KeyEditUi, modifier: HOT_KEY_MODIFIERS, key: VIRTUAL_KEY) {
    ui.modifiers = HOT_KEY_MODIFIERS(0);
    assert!(!apply(settings, ui, KeyMsg::Modifier(modifier)));
    assert!(!apply(settings, ui, KeyMsg::Key(key)));
}

const OK: KeyMsg = KeyMsg::Closed { accepted: true };
const CANCEL: KeyMsg = KeyMsg::Closed { accepted: false };

#[test]
fn cancel_leaves_every_target_as_it_was() {
    let xml = r#"<y-pop><search hotkey="Alt+Space"/><group name="A"><hotkey key="Win+A"/></group></y-pop>"#;
    let mut settings = load(xml);
    let before = settings.to_xml();
    let id = crate::roots::menus(&settings).next().expect("a menu").id;
    for target in [
        KeyTarget::Item(id, 0),
        KeyTarget::NewHotkey(id),
        KeyTarget::SearchHotkey,
        KeyTarget::Action(1),
        KeyTarget::MenuKey,
    ] {
        let mut ui = open(&mut settings, target);
        choose(&mut settings, &mut ui, MOD_CONTROL, VK_K);
        assert!(!apply(&mut settings, &mut ui, CANCEL), "{target:?}");
        assert_eq!(ui.open, None);
        assert_eq!(settings.to_xml(), before, "{target:?}");
    }
}

#[test]
fn nothing_is_written_until_ok() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let mut ui = open(&mut settings, KeyTarget::SearchHotkey);
    choose(&mut settings, &mut ui, MOD_CONTROL, VK_F5);
    assert_eq!(settings.search.as_ref().and_then(|s| s.hotkey.clone()), None);
    assert!(apply(&mut settings, &mut ui, OK));
    assert_eq!(settings.search.as_ref().and_then(|s| s.hotkey.as_deref()), Some("Ctrl+F5"));
}

#[test]
fn ok_needs_a_key() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let mut ui = open(&mut settings, KeyTarget::SearchHotkey);
    apply(&mut settings, &mut ui, KeyMsg::Modifier(MOD_WIN));
    assert!(!verdict(&settings, &ui).accepted);
    assert!(!apply(&mut settings, &mut ui, OK));
    assert!(!settings.to_xml().contains("hotkey"));
}

#[test]
fn opening_on_a_combination_shows_it() {
    let mut settings = load(r#"<y-pop><search hotkey="Ctrl+Win+F"/></y-pop>"#);
    let ui = open(&mut settings, KeyTarget::SearchHotkey);
    assert_eq!(combo_labels(ui.modifiers, ui.key), ["Win", "Ctrl", "F"]);
}

#[test]
fn a_shortcut_offers_only_what_a_chord_can_hold() {
    for key in [VK_A, VK_F4, VK_F5, VK_RETURN, VK_DELETE] {
        assert!(shortcut_key(key), "{key:?}");
    }
    for key in [VK_SPACE, VK_UP, VK_F13] {
        assert!(!shortcut_key(key), "{key:?}");
    }
    assert!(!SHORTCUT_MODIFIERS.iter().any(|(name, _)| *name == "Win"));
}

#[test]
fn what_a_chord_refuses_is_said_and_ok_stays_off() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let before = settings.to_xml();
    let mut ui = open(&mut settings, KeyTarget::Action(5));

    choose(&mut settings, &mut ui, HOT_KEY_MODIFIERS(0), VK_E);
    let said = verdict(&settings, &ui);
    assert!(!said.accepted);
    assert!(said.problem.expect("a reason").contains("typing"));

    choose(&mut settings, &mut ui, MOD_CONTROL, VK_A);
    assert!(verdict(&settings, &ui).problem.expect("a reason").contains("edits the text"));

    assert!(!apply(&mut settings, &mut ui, OK), "refused");
    assert_eq!(settings.to_xml(), before, "not even the list is written out");
}

#[test]
fn a_shortcut_another_action_has_is_refused_naming_it() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let rows = se::actions(&settings).rows;
    let reveal = rows.iter().position(|row| row.id == "reveal").expect("reveal");
    let edit = rows.iter().position(|row| row.id == "edit").expect("edit");

    let mut ui = open(&mut settings, KeyTarget::Action(reveal));
    choose(&mut settings, &mut ui, MOD_CONTROL, VK_E);
    let said = verdict(&settings, &ui);
    assert!(!said.accepted);
    assert!(said.problem.expect("who").contains("Ctrl+E is already"));

    // Its own key is no conflict.
    let ui = open(&mut settings, KeyTarget::Action(edit));
    assert!(verdict(&settings, &ui).accepted);
}

#[test]
fn a_chosen_shortcut_is_written_as_y_pop_reads_it() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let mut ui = open(&mut settings, KeyTarget::Action(2));
    choose(&mut settings, &mut ui, MOD_CONTROL, VK_DELETE);
    // The default list has Ctrl+Del on another action.
    assert!(!verdict(&settings, &ui).accepted);

    let mut ui = open(&mut settings, KeyTarget::MenuKey);
    choose(&mut settings, &mut ui, MOD_CONTROL, VK_K);
    assert!(apply(&mut settings, &mut ui, OK));
    let xml = settings.to_xml();
    assert!(xml.contains(r#"menu-key="Ctrl+K""#), "{xml}");
    let tree = y_pop::config::load_str(&xml, &mut Vec::<Variable>::new()).expect("readable");
    assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
}

#[test]
fn shortcuts_show_as_keycaps_even_when_a_hotkey_could_not_read_them() {
    assert_eq!(shortcut_labels("Ctrl+Del"), ["Ctrl", "Del"]);
    assert_eq!(shortcut_labels("Ctrl+Shift+Enter"), ["Ctrl", "Shift", "Enter"]);
    assert_eq!(shortcut_labels("F5"), ["F5"]);
    assert_eq!(shortcut_labels("Ctrl+7"), ["Ctrl", "7"]);
    assert!(shortcut_labels("nonsense").is_empty());
    assert!(hotkey_labels("Nonsense+").is_empty());
    assert_eq!(hotkey_labels("Alt+Space"), ["Alt", "Space"]);
}

#[test]
fn opening_on_a_shortcut_keeps_it() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let rows = se::actions(&settings).rows;
    let forget = rows.iter().position(|row| row.id == "forget").expect("forget");
    let mut ui = open(&mut settings, KeyTarget::Action(forget));
    assert_eq!(ui.key, Some(VK_DELETE));
    assert!(!apply(&mut settings, &mut ui, OK), "nothing changed");
    assert!(se::actions(&settings).written.eq(&false), "so nothing is written out");
}

const SHARED: &str = r#"<y-pop><search hotkey="Alt+Space"/>
    <group name="A"><hotkey key="alt+space"/><hotkey key="Win+Y"/><hotkey key="Win+Y"/></group>
    <group name="B"><hotkey key="Win+J"/></group></y-pop>"#;

#[test]
fn a_hotkey_conflict_is_seen_from_both_sides() {
    let settings = load(SHARED);
    let menus: Vec<ItemId> = crate::roots::menus(&settings).map(|item| item.id).collect();
    let warning = |text, own| hotkey_warning(&settings, text, own);

    assert!(warning("Alt+Space", KeyTarget::Item(menus[0], 0)).expect("search").contains("the search"));
    assert!(warning("Alt+Space", KeyTarget::SearchHotkey).expect("A").contains("Also used by A"));
    assert!(warning("Win+Y", KeyTarget::Item(menus[0], 1)).expect("own").contains("another hotkey of this item"));
    assert!(warning("Win+Y", KeyTarget::NewHotkey(menus[1])).expect("A").contains("Also used by A"));
    assert_eq!(warning("Win+J", KeyTarget::Item(menus[1], 0)), None);
    assert_eq!(warning("Ctrl+F5", KeyTarget::SearchHotkey), None);
}

#[test]
fn a_key_windows_keeps_is_a_warning_ok_still_works() {
    let mut settings = load(r#"<y-pop><search/></y-pop>"#);
    let mut ui = open(&mut settings, KeyTarget::SearchHotkey);
    choose(&mut settings, &mut ui, MOD_WIN, VK_E);
    let said = verdict(&settings, &ui);
    assert!(said.problem.expect("who").contains("File Explorer"));
    assert!(said.accepted);
    assert!(apply(&mut settings, &mut ui, OK));
}

#[test]
fn removing_the_hotkey_takes_override_with_it() {
    let mut settings = load(r#"<y-pop><search hotkey="Alt+Space" override="true"/></y-pop>"#);
    assert!(se::set_hotkey(&mut settings, ""));
    assert!(!settings.to_xml().contains("override"));
}
