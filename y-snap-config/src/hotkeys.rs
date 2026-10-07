//! The Hotkeys screen: a [`Table`] of global key combinations, each bound
//! to one [`Action`]. A row's Key cell opens the key picker right under
//! that row ([`Row::detail`]).

use crate::key_picker::{KeyPickerUi, combo_labels, key_picker};
use crate::ui::prelude::*;
use crate::{ConfigApp, EditorClose, Msg};
use windows_reactor::*;
use y_snap::action::{Action, Target};
use y_snap::config::Binding;

/// Every [`Action`] a hotkey (or gesture) can be bound to, in combo-box
/// order. Not [`Action::DesktopList`] — see [`crate::overlay::OVERLAY_ACTIONS`].
pub const ACTIONS: [Action; 20] = [
    Action::Overlay,
    Action::Reload,
    Action::Quit,
    Action::Minimize,
    Action::Maximize,
    Action::Close,
    Action::AlwaysOnTop,
    Action::Borderless,
    Action::Transparency,
    Action::Shade,
    Action::HideTaskbar,
    Action::CenterCursor,
    Action::Iconize,
    Action::Traynize,
    Action::Uniconize,
    Action::DesktopNext,
    Action::DesktopPrev,
    Action::DesktopNew,
    Action::DesktopMove,
    Action::PinToggle,
];

pub const TARGETS: [Target; 2] = [Target::Focused, Target::Pointed];

const COLUMNS: [Column; 6] = [
    Column::flex("Key", 150.0),
    Column::flex("Action", 150.0),
    Column::flex("Target", 100.0),
    Column::fixed("Desktop", 96.0),
    Column::fixed("Steal from shell", 72.0),
    Column::actions(size::ICON_BUTTON),
];

pub fn hotkeys_view(
    hotkeys: &[Binding],
    editing: Option<usize>,
    picker: &KeyPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let table = Table::new(&COLUMNS).empty_message("No hotkeys yet — add one below.");
    let detail_viewport = table.detail_viewport(viewport);

    let rows = hotkeys.iter().enumerate().map(|(index, binding)| {
        let detail = (editing == Some(index)).then(|| {
            editor(
                format!("Key combination for {}", binding.action),
                key_picker(
                    Some(binding.hotkey.key),
                    Some(binding.hotkey.modifiers),
                    picker,
                    detail_viewport.inset(CARD_INSET),
                    context,
                ),
                context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
                context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
            )
        });
        Row::new(index.to_string(), cells(index, binding, context)).detail(detail)
    });

    page(
        "Hotkeys",
        "Global key combinations, each bound to one action. Target decides whether it acts on the focused \
         window or whatever the pointer is over.",
        Stack::vertical(space::L)
            .child("table", table.rows(rows).build(viewport))
            .child("add", add_button("Add hotkey", context.callback(|()| Msg::HotkeyAdded)))
            .child(
                "note",
                note(
                    "Steal from shell takes the combination with a low-level keyboard hook when Windows already \
                     owns it (most Win+… combinations). It sees every keystroke on the machine, so it's off unless \
                     a row needs it.",
                ),
            ),
    )
}

fn cells(index: usize, binding: &Binding, context: &ViewContext<ConfigApp>) -> Vec<View> {
    let action = ACTIONS.iter().position(|a| *a == binding.action);
    let target = TARGETS.iter().position(|t| *t == binding.target);

    let key: View = Button::new()
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Left)
        .on_click(context.callback(move |()| Msg::EditHotkey(index)))
        .content(keycaps(&combo_labels(binding.hotkey.modifiers, Some(binding.hotkey.key)), false))
        .tooltip("Change the key combination");

    let desktop: View = if binding.action == Action::DesktopMove {
        number_box(
            (binding.desktop != 0).then_some(binding.desktop.into()),
            1.0,
            99.0,
            context.callback(move |v| Msg::HotkeyDesktopChanged(index, v)),
        )
        .into()
    } else {
        text::placeholder().into()
    };

    vec![
        key,
        combo(ACTIONS.map(Action::name), action, context.callback(move |c| Msg::HotkeyActionChanged(index, c))).into(),
        combo(TARGETS.map(Target::name), target, context.callback(move |c| Msg::HotkeyTargetChanged(index, c))).into(),
        desktop,
        switch(binding.steal, context.callback(move |on| Msg::HotkeyStealToggled(index, on))),
        icon_button(Symbol::Delete, "Delete hotkey", context.callback(move |()| Msg::HotkeyDeleted(index))),
    ]
}
