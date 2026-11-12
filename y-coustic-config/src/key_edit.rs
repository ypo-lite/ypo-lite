//! Choosing a key: one dialog for everything in `y-coustic.xml` that holds a
//! combination: the keys of a `<keys>` group and a single `<hotkey>`.
//!
//! **A draft, not a live edit.** The combination stays in [`KeyEditUi`] until
//! OK, so half a combination never reaches the settings and Cancel touches
//! nothing.
//!
//! What the dialog says about the draft is found by trying it: the draft is
//! written into a copy of the settings, and what that copy would warn about
//! and the settings do not already is what the dialog shows. A combination
//! used twice is a warning and not a refusal: with a key freed later it can be
//! what is wanted.

use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_NOREPEAT, VIRTUAL_KEY};
use y_coustic::config::{Binding, Settings};
use y_coustic::controls::{KeysGroup, Plain, Scope, is_media_key};
use y_coustic::edit::List;
use y_coustic::model::{Action, Target};
use y_core::hotkey::Hotkey;

use crate::edit::GroupKey;
use crate::ui::editor::modal;
use crate::ui::key_picker::{KeyPickerEvents, KeyPickerUi, KeyRules, key_picker_with};
use crate::ui::keys::{self, KeyCategory};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;

/// What the dialog is choosing a key for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyTarget {
    /// One of the keys of the n-th `<keys>` group.
    Group(usize, GroupKey),
    /// A group that does not exist yet; its first key, up. Made by OK.
    NewGroup,
    /// The n-th single hotkey.
    Hotkey(usize),
    /// A hotkey that does not exist yet; made by OK as *mute*.
    NewHotkey,
}

/// The dialog: what it is open on, the combination so far, and the picker's
/// own category and search. Not saved; one at a time.
#[derive(Clone, Default, Debug)]
pub struct KeyEditUi {
    pub open: Option<KeyTarget>,
    pub modifiers: HOT_KEY_MODIFIERS,
    pub key: Option<VIRTUAL_KEY>,
    pub picker: KeyPickerUi,
}

#[derive(Clone, Debug)]
pub enum KeyMsg {
    Open(KeyTarget),
    Key(VIRTUAL_KEY),
    Modifier(HOT_KEY_MODIFIERS),
    Category(KeyCategory),
    Search(String),
    SearchChosen(String),
    /// OK keeps the combination; anything else leaves the file as it was.
    Closed {
        accepted: bool,
    },
}

/// What the target holds now, as the dialog's starting point; `None` if the
/// target is gone.
fn start(settings: &Settings, target: KeyTarget) -> Option<(HOT_KEY_MODIFIERS, Option<VIRTUAL_KEY>)> {
    let fresh = (HOT_KEY_MODIFIERS(MOD_NOREPEAT.0), None);
    let held = |hotkey: Option<Hotkey>| match hotkey {
        Some(hotkey) => (hotkey.modifiers, Some(hotkey.key)),
        None => fresh,
    };
    match target {
        KeyTarget::Group(index, which) => {
            let group = settings.controls.keys.get(index)?;
            Some(held(match which {
                GroupKey::Up => group.up,
                GroupKey::Down => group.down,
                GroupKey::Mute => group.mute,
            }))
        }
        KeyTarget::Hotkey(index) => settings.hotkeys.get(index).map(|b| held(Some(b.hotkey))),
        KeyTarget::NewGroup | KeyTarget::NewHotkey => Some(fresh),
    }
}

/// The draft as a hotkey: `None` until it has a key. Auto-repeat is always off,
/// as `Hotkey::parse` makes it.
fn draft(ui: &KeyEditUi) -> Option<Hotkey> {
    Some(Hotkey::new(
        HOT_KEY_MODIFIERS(ui.modifiers.0 | MOD_NOREPEAT.0),
        ui.key?,
    ))
}

/// Writes `hotkey` where `target` is; whether that changed the settings.
fn commit(settings: &mut Settings, target: KeyTarget, hotkey: Hotkey) -> bool {
    match target {
        KeyTarget::Group(index, which) => {
            let Some(group) = settings.controls.keys.get_mut(index) else {
                return false;
            };
            let slot = match which {
                GroupKey::Up => &mut group.up,
                GroupKey::Down => &mut group.down,
                GroupKey::Mute => &mut group.mute,
            };
            if *slot == Some(hotkey) {
                return false;
            }
            *slot = Some(hotkey);
            true
        }
        KeyTarget::NewGroup => {
            let at = settings.len_of(List::Keys);
            settings.insert_keys(
                at,
                KeysGroup {
                    up: Some(hotkey),
                    down: None,
                    mute: None,
                    plain: Plain::Windows,
                    // A set of keys aimed at the master, until it is told otherwise.
                    scope: Scope::Target(Target::Master),
                },
            );
            true
        }
        KeyTarget::Hotkey(index) => {
            let Some(binding) = settings.hotkeys.get_mut(index) else {
                return false;
            };
            if binding.hotkey == hotkey {
                return false;
            }
            binding.hotkey = hotkey;
            binding.hook_only = is_media_key(hotkey.key);
            true
        }
        KeyTarget::NewHotkey => {
            let at = settings.len_of(List::Hotkeys);
            settings.insert_hotkey(
                at,
                Binding {
                    hotkey,
                    action: Action::Mute,
                    target: Target::Master,
                    hook_only: is_media_key(hotkey.key),
                },
            );
            true
        }
    }
}

/// What the dialog says about the draft, and whether OK can take it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub problem: Option<String>,
    pub accepted: bool,
}

pub fn verdict(settings: &Settings, ui: &KeyEditUi) -> Verdict {
    let (Some(target), Some(hotkey)) = (ui.open, draft(ui)) else {
        return Verdict {
            problem: None,
            accepted: false,
        };
    };
    let mut trial = settings.clone();
    commit(&mut trial, target, hotkey);
    let known = settings.crossing_warnings();
    let problem = trial
        .crossing_warnings()
        .into_iter()
        .find(|warning| !known.contains(warning));
    Verdict {
        problem,
        accepted: true,
    }
}

/// Settles `message`; whether the settings changed. Only OK can change them.
pub fn apply(settings: &mut Settings, ui: &mut KeyEditUi, message: KeyMsg) -> bool {
    match message {
        KeyMsg::Open(target) => {
            let Some((modifiers, key)) = start(settings, target) else {
                return false;
            };
            *ui = KeyEditUi {
                open: Some(target),
                modifiers,
                key,
                picker: KeyPickerUi::opening_on(key),
            };
        }
        KeyMsg::Key(key) => ui.key = Some(key),
        KeyMsg::Modifier(modifier) => ui.modifiers = HOT_KEY_MODIFIERS(ui.modifiers.0 ^ modifier.0),
        KeyMsg::Category(category) => ui.picker.category = category,
        KeyMsg::Search(text) => ui.picker.search = text,
        KeyMsg::SearchChosen(suggestion) => {
            if let Some(key) = keys::from_suggestion(&suggestion) {
                ui.picker = KeyPickerUi::opening_on(Some(key));
                ui.key = Some(key);
            }
        }
        KeyMsg::Closed { accepted } => {
            let done = std::mem::take(ui);
            if !accepted {
                return false;
            }
            return match (done.open, draft(&done)) {
                (Some(target), Some(hotkey)) => commit(settings, target, hotkey),
                _ => false,
            };
        }
    }
    false
}

/// The dialog, while one is open: over the whole window, bar included.
pub fn key_modal(
    settings: &Settings,
    ui: &KeyEditUi,
    window: Viewport,
    context: &ViewContext<ConfigApp>,
) -> Option<View> {
    let target = ui.open?;
    let title = match target {
        KeyTarget::Group(_, GroupKey::Up) => "Key for volume up",
        KeyTarget::Group(_, GroupKey::Down) => "Key for volume down",
        KeyTarget::Group(_, GroupKey::Mute) => "Key for mute",
        KeyTarget::NewGroup => "Add a group of volume keys",
        KeyTarget::Hotkey(_) => "Key of the hotkey",
        KeyTarget::NewHotkey => "Add a hotkey",
    };
    let verdict = verdict(settings, ui);
    let events = KeyPickerEvents {
        on_key: context.callback(|key| Msg::Key(KeyMsg::Key(key))),
        on_modifier: context.callback(|modifier| Msg::Key(KeyMsg::Modifier(modifier))),
        on_category: context.callback(|category| Msg::Key(KeyMsg::Category(category))),
        on_search_changed: context.callback(|text| Msg::Key(KeyMsg::Search(text))),
        on_search_chosen: context.callback(|text| Msg::Key(KeyMsg::SearchChosen(text))),
    };
    Some(modal(
        window,
        title,
        |viewport| {
            key_picker_with(
                ui.key,
                Some(ui.modifiers),
                &KeyRules::ANY,
                verdict.problem.as_deref(),
                &ui.picker,
                viewport,
                &events,
            )
        },
        // A combination needs a key.
        verdict.accepted,
        context.callback(|()| Msg::Key(KeyMsg::Closed { accepted: false })),
        context.callback(|()| Msg::Key(KeyMsg::Closed { accepted: true })),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, VK_F5, VK_F6};

    use super::*;

    fn settings() -> Settings {
        Settings::parse(PathBuf::from("test.xml"), y_coustic::config::DEFAULT_XML)
            .expect("the shipped file loads")
    }

    fn open(settings: &mut Settings, ui: &mut KeyEditUi, target: KeyTarget) {
        apply(settings, ui, KeyMsg::Open(target));
        assert_eq!(ui.open, Some(target));
    }

    #[test]
    fn opening_starts_from_what_the_key_is_now() {
        let mut settings = settings();
        let mut ui = KeyEditUi::default();
        open(&mut settings, &mut ui, KeyTarget::Group(0, GroupKey::Up));
        let up = settings.controls.keys[0].up.unwrap();
        assert_eq!(ui.key, Some(up.key));
        assert_eq!(ui.modifiers, up.modifiers);

        // A key the group does not have starts empty, not on some other key.
        let mut ui = KeyEditUi::default();
        settings.controls.keys[0].mute = None;
        open(&mut settings, &mut ui, KeyTarget::Group(0, GroupKey::Mute));
        assert_eq!(ui.key, None);
    }

    #[test]
    fn a_target_that_is_gone_does_not_open() {
        let mut settings = settings();
        let mut ui = KeyEditUi::default();
        assert!(!apply(&mut settings, &mut ui, KeyMsg::Open(KeyTarget::Hotkey(99))));
        assert_eq!(ui.open, None);
    }

    #[test]
    fn nothing_changes_until_ok_and_cancel_changes_nothing() {
        let mut settings = settings();
        let before = settings.clone();
        let mut ui = KeyEditUi::default();
        open(&mut settings, &mut ui, KeyTarget::Hotkey(0));
        apply(&mut settings, &mut ui, KeyMsg::Key(VK_F5));
        assert_eq!(settings.hotkeys, before.hotkeys);
        assert!(!apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: false }));
        assert_eq!(settings.hotkeys, before.hotkeys);
        assert_eq!(ui.open, None);
    }

    #[test]
    fn ok_writes_the_key_and_keeps_auto_repeat_off() {
        let mut settings = settings();
        let mut ui = KeyEditUi::default();
        open(&mut settings, &mut ui, KeyTarget::Hotkey(0));
        apply(&mut settings, &mut ui, KeyMsg::Key(VK_F5));
        apply(&mut settings, &mut ui, KeyMsg::Modifier(MOD_CONTROL));
        assert!(apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: true }));
        let hotkey = settings.hotkeys[0].hotkey;
        assert_eq!(hotkey.key, VK_F5);
        assert_ne!(hotkey.modifiers.0 & MOD_CONTROL.0, 0);
        assert_ne!(hotkey.modifiers.0 & MOD_NOREPEAT.0, 0);
    }

    #[test]
    fn a_media_key_makes_a_hook_only_hotkey_and_another_key_does_not() {
        let mut settings = settings();
        let mut ui = KeyEditUi::default();
        open(&mut settings, &mut ui, KeyTarget::Hotkey(0));
        apply(&mut settings, &mut ui, KeyMsg::Key(y_core::hotkey::key_from_name("VolumeUp").unwrap()));
        apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: true });
        assert!(settings.hotkeys[0].hook_only);

        open(&mut settings, &mut ui, KeyTarget::Hotkey(0));
        apply(&mut settings, &mut ui, KeyMsg::Key(VK_F6));
        apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: true });
        assert!(!settings.hotkeys[0].hook_only);
    }

    #[test]
    fn a_new_group_and_a_new_hotkey_are_made_by_ok() {
        let mut settings = settings();
        let (groups, hotkeys) = (settings.controls.keys.len(), settings.hotkeys.len());
        let mut ui = KeyEditUi::default();

        open(&mut settings, &mut ui, KeyTarget::NewGroup);
        // No key yet: OK is not offered, and closing with OK adds nothing.
        assert!(!verdict(&settings, &ui).accepted);
        assert!(!apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: true }));
        assert_eq!(settings.controls.keys.len(), groups);

        open(&mut settings, &mut ui, KeyTarget::NewGroup);
        apply(&mut settings, &mut ui, KeyMsg::Key(VK_F5));
        assert!(apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: true }));
        assert_eq!(settings.controls.keys.len(), groups + 1);
        assert_eq!(settings.controls.keys[groups].up.unwrap().key, VK_F5);

        open(&mut settings, &mut ui, KeyTarget::NewHotkey);
        apply(&mut settings, &mut ui, KeyMsg::Key(VK_F6));
        assert!(apply(&mut settings, &mut ui, KeyMsg::Closed { accepted: true }));
        assert_eq!(settings.hotkeys.len(), hotkeys + 1);
        assert_eq!(settings.hotkeys[hotkeys].action, Action::Mute);
    }

    #[test]
    fn a_combination_already_in_use_is_a_warning_and_not_a_refusal() {
        let mut settings = settings();
        let taken = settings.hotkeys[0].hotkey;
        let mut ui = KeyEditUi::default();
        open(&mut settings, &mut ui, KeyTarget::NewHotkey);
        apply(&mut settings, &mut ui, KeyMsg::Key(taken.key));
        ui.modifiers = taken.modifiers;
        let verdict = verdict(&settings, &ui);
        assert!(verdict.accepted);
        assert!(verdict.problem.is_some(), "no warning for a repeated key");

        ui.key = Some(VK_F5);
        ui.modifiers = HOT_KEY_MODIFIERS(MOD_NOREPEAT.0);
        assert_eq!(self::verdict(&settings, &ui).problem, None);
    }
}
