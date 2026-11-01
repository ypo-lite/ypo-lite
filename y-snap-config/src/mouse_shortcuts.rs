//! The Mouse shortcuts screen: a [`Table`] of modifier + mouse trigger
//! pairs, each bound to one [`Action`]. A row's Trigger cell opens the
//! trigger editor right under that row ([`Row::detail`]).
//!
//! The rules that make a combination unusable (a plain left or right click,
//! a clash with another shortcut or with Drag/Resize) are
//! [`y_snap::config::shortcut_problem`]'s, the same ones y-snap applies when
//! it reads the file, so what this screen accepts is what y-snap will load.

use crate::hotkeys::{ACTIONS, TARGETS};
use crate::ui::key_picker::{MODIFIERS, combo_labels};
use crate::ui::prelude::*;
use crate::{ConfigApp, EditorClose, Msg};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_SHIFT, MOD_WIN};
use windows_reactor::*;
use y_snap::action::{Action, Target};
use y_snap::config::{
    Mouse, MouseShortcut, MouseTrigger, Settings, ShortcutProblem, shortcut_problem,
};

const COLUMNS: [Column; 5] = [
    Column::flex("Trigger", 170.0),
    Column::flex("Action", 150.0),
    Column::flex("Target", 100.0),
    Column::fixed("Desktop", 96.0),
    Column::actions(size::ICON_BUTTON),
];

/// The triggers a row of buttons offers, one group per kind, as the plan's
/// mock lays them out.
const BUTTONS: [MouseTrigger; 3] = [
    MouseTrigger::Left,
    MouseTrigger::Middle,
    MouseTrigger::Right,
];
const TILTS: [MouseTrigger; 2] = [MouseTrigger::WheelLeft, MouseTrigger::WheelRight];
const SIDES: [MouseTrigger; 2] = [MouseTrigger::X1, MouseTrigger::X2];

/// How a trigger reads in the UI. The side buttons show as Back/Forward,
/// what most mice print on them.
pub fn trigger_label(trigger: MouseTrigger) -> &'static str {
    match trigger {
        MouseTrigger::Left => "Left",
        MouseTrigger::Middle => "Middle",
        MouseTrigger::Right => "Right",
        MouseTrigger::WheelLeft => "Tilt left",
        MouseTrigger::WheelRight => "Tilt right",
        MouseTrigger::X1 => "Back",
        MouseTrigger::X2 => "Forward",
    }
}

/// What a trigger with no modifier would take over, for the editor's note.
fn replaces(trigger: MouseTrigger) -> &'static str {
    match trigger {
        MouseTrigger::Middle => "middle click",
        MouseTrigger::WheelLeft | MouseTrigger::WheelRight => "sideways scrolling",
        MouseTrigger::X1 => "the Back button",
        MouseTrigger::X2 => "the Forward button",
        MouseTrigger::Left | MouseTrigger::Right => "every click",
    }
}

/// What is wrong with the shortcut at `index`, against every other shortcut
/// and the gestures. `None` when it is fine (or `index` is out of range).
pub fn problem_at(settings: &Settings, index: usize) -> Option<ShortcutProblem> {
    let candidate = settings.mouse_shortcuts.get(index)?;
    let others: Vec<MouseShortcut> = settings
        .mouse_shortcuts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != index)
        .map(|(_, shortcut)| *shortcut)
        .collect();
    shortcut_problem(candidate, &others, &settings.mouse)
}

/// The first combination nothing uses yet, `Win`+middle first: what a new
/// row starts as.
pub fn first_free(shortcuts: &[MouseShortcut], mouse: &Mouse) -> Option<MouseShortcut> {
    let modifier_sets = [
        MOD_WIN,
        HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_SHIFT.0),
        HOT_KEY_MODIFIERS(0),
    ];
    let order = [
        MouseTrigger::Middle,
        MouseTrigger::WheelRight,
        MouseTrigger::WheelLeft,
        MouseTrigger::X1,
        MouseTrigger::X2,
        MouseTrigger::Left,
        MouseTrigger::Right,
    ];
    modifier_sets.into_iter().find_map(|modifiers| {
        order.into_iter().find_map(|trigger| {
            let candidate = MouseShortcut {
                trigger,
                modifiers,
                action: Action::Minimize,
                target: MouseShortcut::DEFAULT_TARGET,
                desktop: 0,
            };
            shortcut_problem(&candidate, shortcuts, mouse)
                .is_none()
                .then_some(candidate)
        })
    })
}

pub fn mouse_shortcuts_view(
    settings: &Settings,
    editing: Option<usize>,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let shortcuts = &settings.mouse_shortcuts;
    let table = Table::new(&COLUMNS).empty_message("No mouse shortcuts yet — add one below.");
    let detail_viewport = table.detail_viewport(viewport).inset(CARD_INSET);

    let rows = shortcuts.iter().enumerate().map(|(index, shortcut)| {
        let problem = problem_at(settings, index);
        let detail = (editing == Some(index)).then(|| {
            editor_checked(
                format!("Trigger for {}", shortcut.action),
                trigger_editor(index, shortcut, problem, detail_viewport, context),
                problem.is_none(),
                context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
                context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
            )
        });
        Row::new(index.to_string(), cells(index, shortcut, context)).detail(detail)
    });

    // A clash can come from the Mouse gestures screen too: Drag changed to a
    // combination a shortcut already has. Say so before a save drops one.
    let clashes = (0..shortcuts.len())
        .filter(|i| editing != Some(*i) && problem_at(settings, *i).is_some())
        .count();
    let clash_note = (clashes > 0).then(|| {
        note(format!(
            "{clashes} shortcut{} can't be used as it stands: its combination is taken by a gesture or another \
             shortcut. y-snap would skip it — open it and pick another.",
            if clashes == 1 { "" } else { "s" }
        ))
    });

    page(
        "Mouse shortcuts",
        "A modifier and a mouse button, each bound to one action. Fires when pressed, on the window under the \
         pointer unless Target says otherwise.",
        Stack::vertical(space::L)
            .child("table", table.rows(rows).build(viewport))
            .child(
                "add",
                add_button("Add shortcut", context.callback(|()| Msg::MouseShortcutAdded)),
            )
            .maybe("clashes", clash_note)
            .child(
                "note",
                note(
                    "Tilt and the side buttons depend on your mouse: its own software may need them left at their \
                     default. A shortcut with no modifier replaces that button's normal job everywhere.",
                ),
            ),
    )
}

fn cells(index: usize, shortcut: &MouseShortcut, context: &ViewContext<ConfigApp>) -> Vec<View> {
    let action = ACTIONS.iter().position(|a| *a == shortcut.action);
    let target = TARGETS.iter().position(|t| *t == shortcut.target);

    let mut labels = combo_labels(shortcut.modifiers, None);
    labels.push(trigger_label(shortcut.trigger).to_owned());
    let trigger: View = Button::new()
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Left)
        .on_click(context.callback(move |()| Msg::EditMouseShortcut(index)))
        .content(keycaps(&labels, false))
        .tooltip("Change the trigger");

    let desktop: View = if shortcut.action.takes_desktop() {
        number_box(
            (shortcut.desktop != 0).then_some(shortcut.desktop.into()),
            1.0,
            99.0,
            context.callback(move |v| Msg::MouseShortcutDesktopChanged(index, v)),
        )
        .into()
    } else {
        text::placeholder().into()
    };

    vec![
        trigger,
        combo(
            ACTIONS.map(Action::name),
            action,
            context.callback(move |c| Msg::MouseShortcutActionChanged(index, c)),
        )
        .into(),
        combo(
            TARGETS.map(Target::name),
            target,
            context.callback(move |c| Msg::MouseShortcutTargetChanged(index, c)),
        )
        .into(),
        desktop,
        icon_button(
            Symbol::Delete,
            "Delete shortcut",
            context.callback(move |()| Msg::MouseShortcutDeleted(index)),
        ),
    ]
}

/// Modifier chips, then one single-choice group per kind of trigger, then
/// the problem (or what a bare trigger replaces) as a note.
fn trigger_editor(
    index: usize,
    shortcut: &MouseShortcut,
    problem: Option<ShortcutProblem>,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let modifiers = AutoGrid::new(56.0)
        .max_columns(4)
        .gap(space::XS, space::XS)
        .items(MODIFIERS.map(|(name, modifier)| {
            (
                name,
                chip(
                    name,
                    shortcut.modifiers.0 & modifier.0 != 0,
                    true,
                    context.callback(move |_| Msg::MouseShortcutModifierToggled(index, modifier)),
                ),
            )
        }));

    let groups = AutoGrid::new(180.0).max_columns(3);
    let group_viewport = groups.item_viewport(viewport, 3);
    let groups = groups.items([
        (
            "buttons",
            field(
                "Button",
                trigger_group(index, &BUTTONS, shortcut.trigger, group_viewport, context),
            ),
        ),
        (
            "tilt",
            field(
                "Wheel tilt",
                trigger_group(index, &TILTS, shortcut.trigger, group_viewport, context),
            ),
        ),
        (
            "side",
            field(
                "Side button",
                trigger_group(index, &SIDES, shortcut.trigger, group_viewport, context),
            ),
        ),
    ]);

    let message = match problem {
        Some(problem) => Some(format!("⚠ {problem}.")),
        None if !shortcut.trigger.is_primary() && shortcut.modifiers.0 == 0 => Some(format!(
            "Replaces {} everywhere.",
            replaces(shortcut.trigger)
        )),
        None => None,
    };

    Stack::vertical(space::M)
        .child("modifiers", field("Modifiers", modifiers.build(viewport)))
        .child("groups", groups.build(viewport))
        .maybe("message", message.map(note))
        .into()
}

/// A single-choice row of buttons. The chosen one is Accent, re-rendered
/// from state every time, instead of a `ToggleButton` that would uncheck
/// itself when clicked again (see the toolkit guide's WinUI traps).
fn trigger_group(
    index: usize,
    triggers: &[MouseTrigger],
    selected: MouseTrigger,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    AutoGrid::new(70.0)
        .max_columns(3)
        .gap(space::XS, space::XS)
        .items(triggers.iter().map(|&trigger| {
            let view: View = Button::new()
                .fill()
                .style(if trigger == selected {
                    ButtonStyle::Accent
                } else {
                    ButtonStyle::Default
                })
                .on_click(
                    context.callback(move |()| Msg::MouseShortcutTriggerChanged(index, trigger)),
                )
                .content(trigger_label(trigger));
            (trigger.name(), view)
        }))
        .build(viewport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_snap::config::{Button, Gesture};

    fn shortcut(trigger: MouseTrigger, modifiers: HOT_KEY_MODIFIERS) -> MouseShortcut {
        MouseShortcut {
            trigger,
            modifiers,
            action: Action::Close,
            target: MouseShortcut::DEFAULT_TARGET,
            desktop: 0,
        }
    }

    #[test]
    fn a_new_shortcut_starts_as_win_middle() {
        let first = first_free(&[], &Mouse::default()).expect("something is free");
        assert_eq!(first.trigger, MouseTrigger::Middle);
        assert_eq!(first.modifiers, MOD_WIN);
    }

    #[test]
    fn a_new_shortcut_skips_what_is_taken() {
        let taken = [shortcut(MouseTrigger::Middle, MOD_WIN)];
        let next = first_free(&taken, &Mouse::default()).expect("something is free");
        assert_eq!(
            (next.trigger, next.modifiers),
            (MouseTrigger::WheelRight, MOD_WIN)
        );

        // Win+left is a drag here, so it is never offered.
        let mouse = Mouse {
            drag: Some(Gesture {
                button: Button::Middle,
                modifiers: MOD_WIN,
            }),
            resize: None,
            allow_maximized: false,
        };
        let next = first_free(&[], &mouse).expect("something is free");
        assert_ne!(
            (next.trigger, next.modifiers),
            (MouseTrigger::Middle, MOD_WIN)
        );
    }

    #[test]
    fn every_trigger_has_a_label_and_a_place_in_the_editor() {
        for trigger in MouseTrigger::ALL {
            assert!(!trigger_label(trigger).is_empty());
            let places = [&BUTTONS[..], &TILTS[..], &SIDES[..]]
                .iter()
                .filter(|group| group.contains(&trigger))
                .count();
            assert_eq!(places, 1, "{trigger:?}");
        }
    }
}
