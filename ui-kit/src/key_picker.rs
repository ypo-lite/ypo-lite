//! The key picker: what a hotkey's combination and an overlay tile's key
//! are chosen with. It replaces the full on-screen keyboard, which was
//! wider than most windows and clipped off the right edge, and offered no
//! way to say "I want F13" except to find it.
//!
//! Top to bottom:
//! - the current combination as keycaps, so a click's result is never in
//!   question;
//! - modifier chips (hotkeys only — a tile fires on one bare key) beside a
//!   search box ("pgd" → PgDn), side by side when there's room;
//! - a category selector (Letters, F-keys, Navigation, …), tabs when they
//!   fit and a dropdown when they don't;
//! - that category's keys as equal-width buttons in an
//!   [`AutoGrid`] — they wrap to the width available, so nothing is ever
//!   off-window, and only one category is on screen at a time, so the panel
//!   stays short.
//!
//! Still never typed and never captured from a live keypress
//! (`windows-reactor` has no key events; a raw Win32 hook was considered
//! and turned down): every choice is a real key from [`crate::keys`], so
//! there is no invalid state to design around.
//!
//! The picker names no app `Msg`: the app owns a [`KeyPickerUi`] (category
//! and search text), and passes [`KeyPickerEvents`] — one [`Callback`] per
//! thing the user can do.
//!
//! The current key is shown with the Accent button style, not as a checked
//! `ToggleButton`: clicking a checked toggle unchecks it natively while the
//! model (and so the view) says nothing changed, and the reconciler, seeing
//! the same `is_checked(true)` as last time, never re-checks it. A style is
//! re-derived from state on every render; a native toggle's own state isn't.

use crate::keys::{self, KeyCategory};
use crate::prelude::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
};
use windows_reactor::*;

/// Page-local state of whichever picker is open (only one ever is).
#[derive(Clone, Default, Debug)]
pub struct KeyPickerUi {
    pub category: KeyCategory,
    pub search: String,
}

impl KeyPickerUi {
    /// Fresh state for a picker opening on `key`: its own category showing.
    pub fn opening_on(key: Option<VIRTUAL_KEY>) -> Self {
        Self {
            category: key.and_then(KeyCategory::of).unwrap_or_default(),
            search: String::new(),
        }
    }
}

/// What the user can do in the picker, as the app wants to hear about it.
#[derive(Clone)]
pub struct KeyPickerEvents {
    pub on_key: Callback<VIRTUAL_KEY>,
    pub on_modifier: Callback<HOT_KEY_MODIFIERS>,
    pub on_category: Callback<KeyCategory>,
    pub on_search_changed: Callback<String>,
    pub on_search_chosen: Callback<String>,
}

pub const MODIFIERS: [(&str, HOT_KEY_MODIFIERS); 4] = [
    ("Win", MOD_WIN),
    ("Ctrl", MOD_CONTROL),
    ("Alt", MOD_ALT),
    ("Shift", MOD_SHIFT),
];

/// A combination as the keycap labels [`keycaps`] shows: held modifiers in
/// Windows' own order, then the key.
pub fn combo_labels(modifiers: HOT_KEY_MODIFIERS, key: Option<VIRTUAL_KEY>) -> Vec<String> {
    MODIFIERS
        .iter()
        .filter(|(_, modifier)| modifiers.0 & modifier.0 != 0)
        .map(|(name, _)| (*name).to_owned())
        .chain(key.map(keys::label))
        .collect()
}

/// Width the category tabs need before they're swapped for a dropdown.
const TABS_MIN_WIDTH: f64 = 720.0;

/// `modifiers: None` hides the modifier chips (an overlay tile).
pub fn key_picker(
    key: Option<VIRTUAL_KEY>,
    modifiers: Option<HOT_KEY_MODIFIERS>,
    ui: &KeyPickerUi,
    viewport: Viewport,
    events: &KeyPickerEvents,
) -> View {
    let labels = combo_labels(modifiers.unwrap_or_default(), key);
    let readout: View = if key.is_some() {
        keycaps(&labels, true)
    } else {
        text::body("No key yet — pick one below.").into()
    };

    let search = AutoSuggestBox::new()
        .text(ui.search.clone())
        .placeholder_text("Type a key name — F13, PgDn, Num 5…")
        .items_source(keys::search(&ui.search))
        .on_text_changed(events.on_search_changed.clone())
        .on_suggestion_chosen(events.on_search_chosen.clone())
        .fill();

    let mut controls = AutoGrid::new(260.0).max_columns(2).gap(space::L, space::M);
    if let Some(modifiers) = modifiers {
        let chip_viewport = controls.item_viewport(viewport, 2);
        let chips = AutoGrid::new(56.0)
            .max_columns(4)
            .gap(space::XS, space::XS)
            .items(MODIFIERS.map(|(name, modifier)| {
                let checked = modifiers.0 & modifier.0 != 0;
                let on_modifier = events.on_modifier.clone();
                (
                    name,
                    chip(
                        name,
                        checked,
                        true,
                        Callback::new(move |_: bool| {
                            let _ = on_modifier.call(modifier);
                        }),
                    ),
                )
            }));
        controls = controls.item(
            "modifiers",
            field("Modifiers — any combination", chips.build(chip_viewport)),
        );
    }
    controls = controls.item("search", field("Find a key", search));

    Stack::vertical(space::L)
        .child("readout", readout)
        .child("controls", controls.build(viewport))
        .child(
            "category",
            category_selector(ui.category, viewport, &events.on_category),
        )
        .child(
            format!("keys/{:?}", ui.category),
            key_grid(ui.category, key, viewport, &events.on_key),
        )
        .into()
}

fn category_selector(
    current: KeyCategory,
    viewport: Viewport,
    on_category: &Callback<KeyCategory>,
) -> View {
    if viewport.fits(TABS_MIN_WIDTH) {
        let items = KeyCategory::ALL.map(|category| {
            let item: View = SelectorBarItem::new()
                .text(category.title())
                .is_selected(category == current)
                .into();
            (category.title(), item)
        });
        let on_category = on_category.clone();
        SelectorBar::new()
            .on_selected_text_changed(Callback::new(move |title: Option<String>| {
                let _ = on_category.call(
                    title
                        .as_deref()
                        .and_then(KeyCategory::from_title)
                        .unwrap_or(current),
                );
            }))
            .slots([SlotView::collection(SelectorBarSlot::Items, items)])
    } else {
        let selected = KeyCategory::ALL
            .iter()
            .position(|&category| category == current);
        let on_category = on_category.clone();
        let picker = combo(
            KeyCategory::ALL.map(KeyCategory::title),
            selected,
            Callback::new(move |choice: Option<usize>| {
                let _ = on_category.call(choice.map_or(current, |index| KeyCategory::ALL[index]));
            }),
        );
        field("Keys", picker)
    }
}

fn key_grid(
    category: KeyCategory,
    current: Option<VIRTUAL_KEY>,
    viewport: Viewport,
    on_key: &Callback<VIRTUAL_KEY>,
) -> View {
    AutoGrid::new(category.min_button_width())
        .max_columns(13)
        .gap(space::XS, space::XS)
        .items(category.keys().map(|(label, key)| {
            let on_key = on_key.clone();
            let button: View = Button::new()
                .style(if Some(key) == current {
                    ButtonStyle::Accent
                } else {
                    ButtonStyle::Default
                })
                .min_width(0.0)
                .fill()
                .on_click(Callback::new(move |()| {
                    let _ = on_key.call(key);
                }))
                .content(label);
            (label, button)
        }))
        .build(viewport)
}
