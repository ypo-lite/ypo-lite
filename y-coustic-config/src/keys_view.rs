//! The Keys screen: the groups of volume keys, the layers that put a modifier on
//! them, the single hotkeys, and what all of that comes to when y-coustic
//! registers it. Every control reports an [`Edit`], or opens the key dialog.

use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use windows_reactor::*;
use y_core::hotkey::Hotkey;
use y_coustic::config::Settings;
use y_coustic::controls::{KeysGroup, Layer, Origin, Place, Plain, Problem, Scope};
use y_coustic::edit::List;
use y_coustic::model::{Action, Target};

use crate::edit::{ActionKind, Edit, GroupKey, ScopeKind, whole};
use crate::key_edit::{KeyMsg, KeyTarget};
use crate::simple::{choose, labels, position};
use crate::ui::key_picker::combo_labels;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};

pub(crate) const TARGETS: &[(Target, &str)] = &[
    (Target::Master, "Master volume"),
    (Target::Mic, "Microphone"),
    (Target::Focused, "Focused application"),
    (Target::Pointed, "Application under the pointer"),
];

pub(crate) const ACTIONS: &[(ActionKind, &str)] = &[
    (ActionKind::VolumeUp, "Volume up"),
    (ActionKind::VolumeDown, "Volume down"),
    (ActionKind::Mute, "Mute"),
    (ActionKind::SetVolume, "Set the level"),
    (ActionKind::Mixer, "Open the mixer"),
];

const PLAINS: &[(Plain, &str)] = &[
    (Plain::Windows, "Windows keeps them"),
    (Plain::Replace, "y-coustic takes them"),
];

pub(crate) const MODIFIERS: [(&str, HOT_KEY_MODIFIERS); 4] = [
    ("Win", MOD_WIN),
    ("Ctrl", MOD_CONTROL),
    ("Alt", MOD_ALT),
    ("Shift", MOD_SHIFT),
];

const LAYER_COLUMNS: [Column; 3] = [
    Column::flex("Modifiers held", 240.0),
    Column::flex("Controls", 190.0),
    Column::actions(THREE_ICON_BUTTONS),
];

const HOTKEY_COLUMNS: [Column; 5] = [
    Column::flex("Key", 140.0),
    Column::flex("Does", 150.0),
    Column::flex("On", 170.0),
    Column::fixed("Level %", 100.0),
    Column::actions(THREE_ICON_BUTTONS),
];

pub fn keys_view(
    settings: &Settings,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let mut body = Stack::vertical(space::XL);
    if settings.has_media_group() {
        body = body.child("replace", replace_card(settings, viewport, context));
    }
    body = body
        .child("groups", groups(settings, viewport, context))
        .child("hotkeys", hotkeys(settings, viewport, context))
        .child("registers", registers(settings));
    page(
        "Keys",
        "Groups of volume keys, each aimed by a target or by layers of its own, and single hotkeys.",
        body,
    )
}

fn replace_card(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    setting(
        "Replace the Windows volume indicator",
        "On, the bare volume keys are y-coustic's and show its HUD instead of the Windows flyout. Off, Windows keeps \
         them and y-coustic takes only the layers with a modifier, such as Shift for the microphone.",
        switch(
            settings.replaces_windows_volume(),
            context.callback(|on: bool| Msg::Edit(Edit::ReplaceWindowsVolume(on))),
        ),
        viewport,
    )
}

/// The messages of the problems whose place `matches`.
fn messages_at(problems: &[Problem], matches: impl Fn(Place) -> bool) -> Vec<String> {
    problems
        .iter()
        .filter(|problem| matches(problem.place))
        .map(|problem| problem.message.clone())
        .collect()
}

/// A warning glyph with the messages as its tooltip, or nothing. Never blocking:
/// a combination used twice is what the user may want while moving keys around.
fn marker(messages: &[String]) -> Option<View> {
    (!messages.is_empty()).then(|| {
        Border::new()
            .vertical_alignment(VerticalAlignment::Center)
            .content(SymbolIcon::new().symbol(Symbol::Important))
            .tooltip(messages.join("\n"))
    })
}

/// `view` with the marker beside it, when there is one.
fn with_marker(view: View, messages: &[String]) -> View {
    match marker(messages) {
        Some(marker) => Stack::horizontal(space::S)
            .child("view", view)
            .child("marker", marker)
            .into(),
        None => view,
    }
}

/// A key as keycaps, or an empty marker.
fn caps(hotkey: Option<Hotkey>) -> View {
    match hotkey {
        Some(hotkey) => keycaps(&combo_labels(hotkey.modifiers, Some(hotkey.key)), false),
        None => text::placeholder().into(),
    }
}

/// A key you can click to change, with an ✕ when it can go.
fn key_cell(
    hotkey: Option<Hotkey>,
    target: KeyTarget,
    clear: Option<Edit>,
    context: &ViewContext<ConfigApp>,
) -> View {
    let clear = clear.map(|edit| {
        (
            "Remove this key",
            hotkey.is_some(),
            context.callback(move |()| Msg::Edit(edit)),
        )
    });
    key_field(
        caps(hotkey),
        "Choose the key",
        context.callback(move |()| Msg::Key(KeyMsg::Open(target))),
        clear,
    )
}

/// Move up, move down and delete: the actions column of a list.
fn actions(
    index: usize,
    count: usize,
    (up, down, delete): (Edit, Edit, Edit),
    what: &str,
    context: &ViewContext<ConfigApp>,
) -> View {
    Stack::horizontal(space::XS)
        .child(
            "move",
            move_buttons(
                index,
                count,
                context.callback(move |()| Msg::Edit(up)),
                context.callback(move |()| Msg::Edit(down)),
            ),
        )
        .child(
            "delete",
            icon_button(
                Symbol::Delete,
                what,
                context.callback(move |()| Msg::Edit(delete)),
            ),
        )
        .into()
}

/// [`actions`] of an entry of `list`.
pub(crate) fn list_actions(
    list: List,
    index: usize,
    count: usize,
    what: &str,
    context: &ViewContext<ConfigApp>,
) -> View {
    actions(
        index,
        count,
        (
            Edit::Move { list, index, up: true },
            Edit::Move { list, index, up: false },
            Edit::Remove(list, index),
        ),
        what,
        context,
    )
}

fn groups(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let count = settings.controls.keys.len();
    let problems = settings.expansion().problems;
    let cards = settings
        .controls
        .keys
        .iter()
        .enumerate()
        .map(|(index, group)| {
            (
                format!("group{index}"),
                group_card(settings, &problems, index, group, count, viewport, context),
            )
        });
    let mut body = Stack::vertical(space::M).child(
        "about",
        text::caption(
            "A group is up, down and mute together, aimed by one target, or by layers of its own: the modifiers you \
             hold with its keys. Nothing is shared between groups, so a set of keys that only handles the microphone is \
             a group with that one target.",
        ),
    );
    if count == 0 {
        body = body.child("none", note("No groups: add one to give some keys a use."));
    }
    body = body.children(cards).child(
        "add",
        add_button(
            "Add a group",
            context.callback(|()| Msg::Key(KeyMsg::Open(KeyTarget::NewGroup))),
        ),
    );
    section("Volume keys", body)
}

/// One group: its keys, what it is aimed by, and the layers or the target.
fn group_card(
    settings: &Settings,
    problems: &[Problem],
    index: usize,
    group: &KeysGroup,
    count: usize,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let inner = viewport.inset(CARD_INSET);

    let keys = [
        (GroupKey::Up, "Up", group.up),
        (GroupKey::Down, "Down", group.down),
        (GroupKey::Mute, "Mute", group.mute),
    ];
    let present = keys.iter().filter(|(_, _, key)| key.is_some()).count();
    let mut grid = AutoGrid::new(170.0).max_columns(4).gap(space::M, space::M);
    for (which, label, key) in keys {
        // The last key of a group cannot go: a group with none is not one.
        let clear = (present > 1).then_some(Edit::ClearGroupKey(index, which));
        grid = grid.item(
            label,
            field(label, key_cell(key, KeyTarget::Group(index, which), clear, context)),
        );
    }
    if group.has_media_keys() {
        grid = grid.item(
            "plain",
            field(
                "Bare volume keys",
                combo(
                    labels(PLAINS),
                    position(PLAINS, group.plain),
                    choose(context, PLAINS, move |plain| Edit::Plain(index, plain)),
                ),
            ),
        );
    }

    let kind = ScopeKind::of(&group.scope);
    let aim = field(
        "Controls",
        segmented(
            &[Choice::new("One target"), Choice::new("Layers by modifier")],
            Some(if kind == ScopeKind::Target { 0 } else { 1 }),
            context.callback(move |choice: usize| {
                Msg::Edit(Edit::GroupScope(
                    index,
                    if choice == 0 { ScopeKind::Target } else { ScopeKind::Layers },
                ))
            }),
        ),
    );

    let detail: View = match &group.scope {
        Scope::Target(target) => field(
            "What the keys control",
            combo(
                labels(TARGETS),
                position(TARGETS, *target),
                choose(context, TARGETS, move |target| Edit::GroupTarget(index, target)),
            ),
        ),
        Scope::Layers(layers) => layers_block(settings, problems, index, layers, inner, context),
    };

    // A group aimed by one target has no layer rows: its one layer's problems
    // are the group's.
    let one_target = matches!(group.scope, Scope::Target(_));
    let header_problems = messages_at(problems, |place| match place {
        Place::Group(of) => of == index,
        Place::Layer { group, .. } => one_target && group == index,
        Place::Hotkey(_) => false,
    });
    let trailing: View = Stack::horizontal(space::XS)
        .maybe("problem", marker(&header_problems))
        .child(
            "move",
            move_buttons(
                index,
                count,
                context.callback(move |()| {
                    Msg::Edit(Edit::Move { list: List::Keys, index, up: true })
                }),
                context.callback(move |()| {
                    Msg::Edit(Edit::Move { list: List::Keys, index, up: false })
                }),
            ),
        )
        .child(
            "duplicate",
            icon_button(
                Symbol::Copy,
                "Duplicate this group, with its layers",
                context.callback(move |()| Msg::Edit(Edit::DuplicateGroup(index))),
            ),
        )
        .child(
            "delete",
            icon_button(
                Symbol::Delete,
                "Delete this group",
                context.callback(move |()| Msg::Edit(Edit::Remove(List::Keys, index))),
            ),
        )
        .into();

    panel(
        format!("Group {}", index + 1),
        Some(trailing),
        Stack::vertical(space::L)
            .child("keys", grid.build(inner))
            .child("aim", aim)
            .child("detail", detail),
    )
}

/// The layers of a group aimed by layers: a table, and what adds to it.
fn layers_block(
    settings: &Settings,
    problems: &[Problem],
    group: usize,
    layers: &[Layer],
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let count = layers.len();
    let rows = layers.iter().enumerate().map(|(index, layer)| {
        let chips =
            Stack::horizontal(space::XS).children(MODIFIERS.iter().map(|&(name, modifier)| {
                let on = layer.modifiers & modifier.0 != 0;
                (
                    name,
                    chip(
                        name,
                        on,
                        true,
                        context.callback(move |_: bool| {
                            Msg::Edit(Edit::GroupLayerModifier(group, index, modifier))
                        }),
                    ),
                )
            }));
        let target = combo(
            labels(TARGETS),
            position(TARGETS, layer.target),
            choose(context, TARGETS, move |target| {
                Edit::GroupLayerTarget(group, index, target)
            }),
        );
        let here = messages_at(problems, |place| {
            place == Place::Layer { group, layer: index }
        });
        Row::new(
            format!("layer{index}"),
            vec![
                with_marker(chips.into(), &here),
                target.into(),
                actions(
                    index,
                    count,
                    (
                        Edit::MoveGroupLayer { group, index, up: true },
                        Edit::MoveGroupLayer { group, index, up: false },
                        Edit::RemoveGroupLayer(group, index),
                    ),
                    "Delete this layer",
                    context,
                ),
            ],
        )
    });
    let table = Table::new(&LAYER_COLUMNS)
        .empty_message("No layers: this group controls nothing until it has one.")
        .rows(rows)
        .build(viewport);

    // Copying is offered from the other groups that have layers of their own and
    // differ from this one: a button each, not a menu, which would keep showing
    // its last choice.
    let copies = settings
        .controls
        .keys
        .iter()
        .enumerate()
        .filter(|(other, other_group)| {
            *other != group
                && matches!(&other_group.scope, Scope::Layers(other_layers)
                    if !other_layers.is_empty() && other_layers.as_slice() != layers)
        })
        .map(|(other, _)| {
            (
                format!("copy{other}"),
                button(
                    &format!("Copy the layers of group {}", other + 1),
                    false,
                    true,
                    context.callback(move |()| {
                        Msg::Edit(Edit::CopyLayers { to: group, from: other })
                    }),
                ),
            )
        });

    Stack::vertical(space::M)
        .child(
            "about",
            text::caption(
                "The modifiers held with the group's keys, and what they then control. No modifier is the bare keys. \
                 Avoid an Alt layer: letting go of Alt alone afterwards focuses the window's menu bar.",
            ),
        )
        .child("table", table)
        .child(
            "buttons",
            Stack::horizontal(space::S)
                .child(
                    "add",
                    add_button(
                        "Add a layer",
                        context.callback(move |()| Msg::Edit(Edit::AddGroupLayer(group))),
                    ),
                )
                .children(copies),
        )
        .into()
}

fn hotkeys(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let count = settings.hotkeys.len();
    let problems = settings.expansion().problems;
    let rows = settings.hotkeys.iter().enumerate().map(|(index, binding)| {
        let level: View = match binding.action {
            Action::SetVolume(level) => number_box(
                Some(f64::from(level)),
                0.0,
                100.0,
                context.callback(move |value: Option<f64>| match value {
                    Some(value) => Msg::Edit(Edit::HotkeyLevel(index, whole(value).min(100) as u8)),
                    None => Msg::Ignored,
                }),
            )
            .into(),
            _ => text::placeholder().into(),
        };
        // The mixer is opened, not aimed.
        let target: View = if binding.action == Action::Mixer {
            text::placeholder().into()
        } else {
            combo(
                labels(TARGETS),
                position(TARGETS, binding.target),
                choose(context, TARGETS, move |target| Edit::HotkeyTarget(index, target)),
            )
            .into()
        };
        Row::new(
            format!("hotkey{index}"),
            vec![
                with_marker(
                    key_cell(Some(binding.hotkey), KeyTarget::Hotkey(index), None, context),
                    &messages_at(&problems, |place| place == Place::Hotkey(index)),
                ),
                combo(
                    labels(ACTIONS),
                    position(ACTIONS, ActionKind::of(binding.action)),
                    choose(context, ACTIONS, move |kind| Edit::HotkeyAction(index, kind)),
                )
                .into(),
                target,
                level,
                list_actions(List::Hotkeys, index, count, "Delete this hotkey", context),
            ],
        )
    });
    let table = Table::new(&HOTKEY_COLUMNS)
        .empty_message("No single hotkeys.")
        .rows(rows)
        .build(viewport);

    section(
        "Single hotkeys",
        Stack::vertical(space::M)
            .child(
                "about",
                text::caption("One key for one thing, for whatever the groups and layers do not cover."),
            )
            .child("table", table)
            .child(
                "add",
                add_button(
                    "Add a hotkey",
                    context.callback(|()| Msg::Key(KeyMsg::Open(KeyTarget::NewHotkey))),
                ),
            ),
    )
}

/// What the groups, the layers and the hotkeys come to: every key y-coustic
/// registers, and what each does; with what would not work, said first.
fn registers(settings: &Settings) -> View {
    let expansion = settings.expansion();
    let bindings = expansion.bindings;
    let mut body = Stack::vertical(space::S);

    let warnings: Vec<String> = expansion
        .problems
        .iter()
        .map(|problem| problem.message.clone())
        .collect();
    if !warnings.is_empty() {
        body = body.child("warnings", note(warnings.join("\n")));
    }

    let lines = bindings.iter().enumerate().map(|(index, (binding, origin))| {
        let line: View = Stack::horizontal(space::M)
            .child(
                "key",
                keycaps(&combo_labels(binding.hotkey.modifiers, Some(binding.hotkey.key)), false),
            )
            .child(
                "does",
                text::caption(describe(binding.action, binding.target)).vertical_alignment(VerticalAlignment::Center),
            )
            .child(
                "from",
                text::caption(origin_text(settings, *origin))
                    .vertical_alignment(VerticalAlignment::Center)
                    .opacity(emphasis::SUBTLE),
            )
            .into();
        (format!("binding{index}"), line)
    });
    body = body.child("list", Stack::vertical(space::XS).children(lines));

    section(
        format!("What y-coustic registers: {}", bindings.len()),
        body,
    )
}

/// Where a registered binding comes from, for the readout: "hotkey 2", or the
/// group and, when it has layers of its own, the layer.
fn origin_text(settings: &Settings, origin: Origin) -> String {
    match origin {
        Origin::Hotkey(index) => format!("hotkey {}", index + 1),
        Origin::Layer { group, layer } => match settings.controls.keys.get(group).map(|g| &g.scope) {
            Some(Scope::Layers(_)) => format!("group {}, layer {}", group + 1, layer + 1),
            _ => format!("group {}", group + 1),
        },
    }
}

/// "Volume up · Master volume", for the readout.
fn describe(action: Action, target: Target) -> String {
    let kind = ActionKind::of(action);
    let what = ACTIONS
        .iter()
        .find(|(known, _)| *known == kind)
        .map_or("", |(_, label)| *label);
    let what = match action {
        Action::SetVolume(level) => format!("{what} to {level}%"),
        _ => what.to_owned(),
    };
    if action == Action::Mixer {
        return what;
    }
    let on = TARGETS
        .iter()
        .find(|(known, _)| *known == target)
        .map_or("", |(_, label)| *label);
    format!("{what} · {on}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_and_target_the_file_can_hold_has_an_option() {
        for target in [Target::Master, Target::Mic, Target::Focused, Target::Pointed] {
            assert!(position(TARGETS, target).is_some(), "{target:?}");
        }
        for action in [
            Action::VolumeUp,
            Action::VolumeDown,
            Action::Mute,
            Action::SetVolume(30),
            Action::Mixer,
        ] {
            assert!(position(ACTIONS, ActionKind::of(action)).is_some(), "{action:?}");
        }
        for plain in [Plain::Windows, Plain::Replace] {
            assert!(position(PLAINS, plain).is_some());
        }
    }

    #[test]
    fn a_binding_says_where_it_comes_from() {
        let settings = Settings::parse(
            std::path::PathBuf::from("test.xml"),
            y_coustic::config::DEFAULT_XML,
        )
        .expect("the shipped file loads");
        assert_eq!(origin_text(&settings, Origin::Hotkey(0)), "hotkey 1");
        assert_eq!(
            origin_text(&settings, Origin::Layer { group: 1, layer: 2 }),
            "group 2, layer 3"
        );
        let mut targets = settings.clone();
        targets.controls.keys[0].scope = Scope::Target(Target::Mic);
        assert_eq!(
            origin_text(&targets, Origin::Layer { group: 0, layer: 0 }),
            "group 1"
        );
    }

    #[test]
    fn a_problem_is_marked_on_its_own_row_and_nowhere_else() {
        let mut settings = Settings::parse(
            std::path::PathBuf::from("test.xml"),
            y_coustic::config::DEFAULT_XML,
        )
        .expect("the shipped file loads");
        // Group 2 controls nothing; the first group's second layer gets Alt.
        settings.controls.keys[1].scope = Scope::default();
        settings.controls.keys[0].scope = Scope::Layers(vec![Layer {
            modifiers: 0,
            target: Target::Master,
        }, Layer {
            modifiers: windows::Win32::UI::Input::KeyboardAndMouse::MOD_ALT.0,
            target: Target::Mic,
        }]);
        let problems = settings.expansion().problems;
        assert_eq!(
            messages_at(&problems, |place| place == Place::Group(1)).len(),
            1
        );
        assert_eq!(
            messages_at(&problems, |place| place == Place::Layer { group: 0, layer: 1 }).len(),
            1
        );
        assert!(messages_at(&problems, |place| place == Place::Layer { group: 0, layer: 0 }).is_empty());
        assert!(messages_at(&problems, |place| place == Place::Hotkey(0)).is_empty());
        assert!(marker(&[]).is_none());
    }

    #[test]
    fn the_readout_says_what_a_binding_does() {
        assert_eq!(describe(Action::VolumeUp, Target::Master), "Volume up · Master volume");
        assert_eq!(describe(Action::SetVolume(30), Target::Mic), "Set the level to 30% · Microphone");
        assert_eq!(describe(Action::Mixer, Target::Master), "Open the mixer");
    }
}
