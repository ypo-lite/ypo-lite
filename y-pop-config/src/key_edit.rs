//! Choosing a key: one dialog for everything in `y-pop.xml` that holds one, an
//! item's hotkey, the search's hotkey, an action's shortcut and the menu key.
//!
//! **A draft, not a live edit.** The combination stays in [`KeyEditUi`] until
//! OK, so half a combination never reaches the model and Cancel touches
//! nothing. That matters more than for most fields: the first edit of an
//! action writes out the whole `<actions>` list.
//!
//! Hotkeys and shortcuts are not the same thing. A hotkey is the system's, and
//! may be anything [`Hotkey`] reads. A shortcut only works inside the search
//! box, so what it can be is [`Chord`]'s to say: no Win, few keys, nothing that
//! would be typing. The rules here ask `Chord` rather than repeat it.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_DELETE, VK_F1, VK_RETURN,
};
use y_core::hotkey::{Hotkey, key_name};
use y_pop::model::ActionId;
use y_pop::search::hotkey::KeyCombo;
use y_pop::search::keycheck;
use y_pop::search::keys::{Chord, Key};
use y_pop::settings::{HotkeyEntry, ItemId, Settings};

use crate::menu_edit::{canonical_hotkey, join_hotkey, split_hotkey};
use crate::search_edit as se;
use crate::ui::editor::modal;
use crate::ui::key_picker::{KeyPickerEvents, KeyPickerUi, KeyRules, combo_labels, key_picker_with};
use crate::ui::keys::{self, KeyCategory};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg, roots, tree};
use windows_reactor::*;

/// What the dialog is choosing a key for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyTarget {
    /// The n-th hotkey of an item.
    Item(ItemId, usize),
    /// A hotkey the item doesn't have yet; made by OK.
    NewHotkey(ItemId),
    /// `<search hotkey>`.
    SearchHotkey,
    /// The shortcut of the n-th action.
    Action(usize),
    /// What opens the action menu.
    MenuKey,
}

impl KeyTarget {
    fn is_shortcut(self) -> bool {
        matches!(self, Self::Action(_) | Self::MenuKey)
    }

    fn item(self) -> Option<ItemId> {
        match self {
            Self::Item(id, _) | Self::NewHotkey(id) => Some(id),
            _ => None,
        }
    }
}

/// The dialog: what it is open on, the combination so far, and the picker's own
/// category and search. Not saved; one at a time.
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
    Closed { accepted: bool },
}

const SHORTCUT_MODIFIERS: [(&str, HOT_KEY_MODIFIERS); 3] = [
    ("Ctrl", MOD_CONTROL),
    ("Alt", MOD_ALT),
    ("Shift", MOD_SHIFT),
];

/// Whether a shortcut can hold this key, with some modifiers. Two sets, as
/// `Chord` keeps one key of each for something else: `Ctrl+A`, `Alt+F4`.
fn shortcut_key(key: VIRTUAL_KEY) -> bool {
    ["Ctrl", "Ctrl+Alt"]
        .iter()
        .any(|held| Chord::parse(&format!("{held}+{}", key_name(key))).is_ok())
}

const SHORTCUT_RULES: KeyRules = KeyRules {
    modifiers: &SHORTCUT_MODIFIERS,
    allows: shortcut_key,
};

/// A shortcut as written, as the picker shows it. `Hotkey` doesn't read
/// `Chord`'s `Del`, so this goes by what `Chord` understood.
pub fn split_shortcut(text: &str) -> (HOT_KEY_MODIFIERS, Option<VIRTUAL_KEY>) {
    let Ok(chord) = Chord::parse(text) else {
        return split_hotkey(text);
    };
    let held = [
        (chord.mods.ctrl, MOD_CONTROL),
        (chord.mods.alt, MOD_ALT),
        (chord.mods.shift, MOD_SHIFT),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |held, (_, modifier)| held | modifier.0);
    let key = match chord.key {
        Key::Enter => Some(VK_RETURN),
        Key::Delete => Some(VK_DELETE),
        Key::Letter(letter) => Some(VIRTUAL_KEY(letter as u16)),
        Key::Digit(digit) => Some(VIRTUAL_KEY(u16::from(b'0' + digit))),
        Key::Function(number) => Some(VIRTUAL_KEY(VK_F1.0 + u16::from(number) - 1)),
        _ => None,
    };
    (HOT_KEY_MODIFIERS(held), key)
}

/// A shortcut's keycaps. Text that isn't one has none.
pub fn shortcut_labels(text: &str) -> Vec<String> {
    match split_shortcut(text) {
        (modifiers, Some(key)) if Chord::parse(text).is_ok() => combo_labels(modifiers, Some(key)),
        _ => Vec::new(),
    }
}

/// A hotkey's keycaps. Text that isn't one has none.
pub fn hotkey_labels(text: &str) -> Vec<String> {
    match Hotkey::parse(text) {
        Some(_) => {
            let (modifiers, key) = split_hotkey(text);
            combo_labels(modifiers, key)
        }
        None => Vec::new(),
    }
}

/// What the target holds now; `None` if it is gone.
fn current(settings: &Settings, target: KeyTarget) -> Option<String> {
    match target {
        KeyTarget::Item(id, index) => tree::item(settings, id)?
            .hotkeys
            .get(index)
            .map(|entry| entry.key.clone()),
        KeyTarget::NewHotkey(id) => tree::item(settings, id).map(|_| String::new()),
        KeyTarget::SearchHotkey => Some(
            settings
                .search
                .as_ref()
                .and_then(|search| search.hotkey.clone())
                .unwrap_or_default(),
        ),
        KeyTarget::Action(index) => Some(se::actions(settings).rows.get(index)?.key.clone().unwrap_or_default()),
        KeyTarget::MenuKey => Some(se::actions(settings).menu_key),
    }
}

/// The draft as the file writes it. `None` until it has a key, and for a
/// shortcut `Chord` refuses.
fn draft_text(ui: &KeyEditUi, target: KeyTarget) -> Option<String> {
    ui.key?;
    let text = join_hotkey(ui.modifiers, ui.key);
    if target.is_shortcut() {
        Chord::parse(&text).ok().map(|chord| chord.to_string())
    } else {
        Some(text)
    }
}

/// What to say about a hotkey another hotkey shares, or Windows keeps: y-pop
/// registers them one by one, and the second to ask loses. A warning and not a
/// refusal: with `override`, or a key freed later, it can be what's wanted.
pub fn hotkey_warning(settings: &Settings, text: &str, own: KeyTarget) -> Option<String> {
    let canonical = canonical_hotkey(text)?;
    let same = |other: &str| canonical_hotkey(other).as_deref() == Some(canonical.as_str());

    let search = settings.search.as_ref().and_then(|search| search.hotkey.as_deref());
    if own != KeyTarget::SearchHotkey && search.is_some_and(same) {
        return Some("Also used by the search: only one of them will work.".to_owned());
    }
    for other in settings.items() {
        for (index, entry) in other.hotkeys.iter().enumerate() {
            if same(&entry.key) && own != KeyTarget::Item(other.id, index) {
                let who = if own.item() == Some(other.id) {
                    "another hotkey of this item".to_owned()
                } else {
                    roots::label(other)
                };
                return Some(format!("Also used by {who}: only one of them will work."));
            }
        }
    }
    let reserved = keycheck::reserved_for(&KeyCombo::parse(&canonical)?)?;
    Some(format!("Reserved for {reserved}: y-pop may not get it."))
}

/// Who already has this shortcut, among the others of the search.
fn shortcut_taken_by(settings: &Settings, chord: Chord, own: KeyTarget) -> Option<String> {
    let view = se::actions(settings);
    if own != KeyTarget::MenuKey && Chord::parse(&view.menu_key).is_ok_and(|key| key == chord) {
        return Some("what opens the action menu".to_owned());
    }
    view.rows
        .iter()
        .enumerate()
        .filter(|(index, row)| own != KeyTarget::Action(*index) && !row.ignored)
        .find(|(_, row)| {
            let text = row.key.as_deref();
            text.is_some_and(|text| Chord::parse(text).is_ok_and(|key| key == chord))
        })
        .map(|(_, row)| match ActionId::parse(&row.id) {
            Some(id) => format!("{}'s", id.label()),
            None => row.id.clone(),
        })
}

/// What the dialog says about the draft, and whether OK can take it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub problem: Option<String>,
    pub accepted: bool,
}

pub fn verdict(settings: &Settings, ui: &KeyEditUi) -> Verdict {
    let refused = |problem| Verdict {
        problem,
        accepted: false,
    };
    let (Some(target), Some(_)) = (ui.open, ui.key) else {
        return refused(None);
    };
    let text = join_hotkey(ui.modifiers, ui.key);
    if !target.is_shortcut() {
        return Verdict {
            problem: hotkey_warning(settings, &text, target),
            accepted: true,
        };
    }
    match Chord::parse(&text) {
        Err(error) => refused(Some(format!("{text} {error}."))),
        Ok(chord) => match shortcut_taken_by(settings, chord, target) {
            Some(who) => refused(Some(format!("{chord} is already {who}."))),
            None => Verdict {
                problem: None,
                accepted: true,
            },
        },
    }
}

/// Writes `text` where it goes; whether that changed the file.
fn commit(settings: &mut Settings, target: KeyTarget, text: &str) -> bool {
    match target {
        KeyTarget::Item(id, index) => tree::item_mut(settings, id)
            .and_then(|item| item.hotkeys.get_mut(index))
            .is_some_and(|entry| {
                let changed = entry.key != text;
                text.clone_into(&mut entry.key);
                changed
            }),
        KeyTarget::NewHotkey(id) => tree::item_mut(settings, id).is_some_and(|item| {
            item.hotkeys.push(HotkeyEntry {
                key: text.to_owned(),
                can_override: false,
                extra_attrs: Vec::new(),
            });
            true
        }),
        KeyTarget::SearchHotkey => se::set_hotkey(settings, text),
        KeyTarget::Action(index) => se::set_action_key(settings, index, text),
        KeyTarget::MenuKey => se::set_menu_key(settings, text),
    }
}

/// Settles `message`; whether the settings changed. Only OK can change them.
pub fn apply(settings: &mut Settings, ui: &mut KeyEditUi, message: KeyMsg) -> bool {
    match message {
        KeyMsg::Open(target) => {
            let Some(text) = current(settings, target) else {
                return false;
            };
            let (modifiers, key) = if target.is_shortcut() {
                split_shortcut(&text)
            } else {
                split_hotkey(&text)
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
            let target = done.open.filter(|_| accepted && verdict(settings, &done).accepted);
            let write = target.and_then(|target| Some((target, draft_text(&done, target)?)));
            return write.is_some_and(|(target, text)| commit(settings, target, &text));
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
        KeyTarget::Item(id, _) | KeyTarget::NewHotkey(id) => {
            format!("Hotkey for {}", roots::label(tree::item(settings, id)?))
        }
        KeyTarget::SearchHotkey => "Hotkey of the search".to_owned(),
        KeyTarget::Action(index) => {
            let id = se::actions(settings).rows.get(index)?.id.clone();
            match ActionId::parse(&id) {
                Some(id) => format!("Shortcut for {}", id.label()),
                None => format!("Shortcut for {id}"),
            }
        }
        KeyTarget::MenuKey => "Key that opens the action menu".to_owned(),
    };
    let rules = if target.is_shortcut() {
        SHORTCUT_RULES
    } else {
        KeyRules::ANY
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
                &rules,
                verdict.problem.as_deref(),
                &ui.picker,
                viewport,
                &events,
            )
        },
        // A combination needs a key, and one that can work.
        verdict.accepted,
        context.callback(|()| Msg::Key(KeyMsg::Closed { accepted: false })),
        context.callback(|()| Msg::Key(KeyMsg::Closed { accepted: true })),
    ))
}

impl ConfigApp {
    /// Settles a message of the key dialog.
    pub(crate) fn edit_key(&mut self, message: KeyMsg) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        self.run_status = None;
        if apply(settings, &mut self.key_edit, message) {
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests;
