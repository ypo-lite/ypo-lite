//! The Mouse screen: the wheel and button bindings that change a volume. Every
//! control reports an [`Edit`].

use windows_reactor::*;
use y_coustic::config::Settings;
use y_coustic::edit::List;
use y_coustic::input::{Input, MouseBinding};
use y_coustic::model::Action;

use crate::edit::{ActionKind, Edit, MouseKind, whole};
use crate::keys_view::{ACTIONS, MODIFIERS, TARGETS, list_actions};
use crate::simple::{choose, labels, position};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};

const INPUTS: &[(Input, &str)] = &[
    (Input::Wheel, "Wheel"),
    (Input::WheelTilt, "Wheel, tilted sideways"),
    (Input::Left, "Left button"),
    (Input::Right, "Right button"),
    (Input::Middle, "Middle button"),
    (Input::X1, "Back button (X1)"),
    (Input::X2, "Forward button (X2)"),
];

const COLUMNS: [Column; 5] = [
    Column::flex("Listens to", 170.0),
    Column::flex("Modifiers held", 240.0),
    Column::flex("Does", 200.0),
    Column::flex("On", 170.0),
    Column::actions(THREE_ICON_BUTTONS),
];

pub fn mouse_view(
    settings: &Settings,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let count = settings.mouse.len();
    let shadowed = settings.shadowed_mouse_bindings();
    let rows = settings
        .mouse
        .iter()
        .enumerate()
        .map(|(index, binding)| row(index, count, binding, shadowed.contains(&index), context));
    let table = Table::new(&COLUMNS)
        .empty_message("No mouse bindings: the mouse does nothing to the volume.")
        .rows(rows)
        .build(viewport);

    let body = Stack::vertical(space::M)
        .child(
            "rule",
            note(
                "A wheel or one of the three main buttons needs a modifier: held bare, it would take over every scroll \
                 or click. The side buttons may go without. A button acts when it is released; the first binding with \
                 the same input and modifiers wins.",
            ),
        )
        .child("table", table)
        .child(
            "add",
            Stack::horizontal(space::S)
                .child(
                    "wheel",
                    add_button(
                        "Add a wheel binding",
                        context.callback(|()| Msg::Edit(Edit::AddMouse(MouseKind::Wheel))),
                    ),
                )
                .child(
                    "button",
                    add_button(
                        "Add a button binding",
                        context.callback(|()| Msg::Edit(Edit::AddMouse(MouseKind::Button))),
                    ),
                ),
        );

    page(
        "Mouse",
        "The mouse wheel and buttons that change a volume.",
        section("Bindings", body),
    )
}

fn row(
    index: usize,
    count: usize,
    binding: &MouseBinding,
    shadowed: bool,
    context: &ViewContext<ConfigApp>,
) -> Row {
    let input = combo(
        labels(INPUTS),
        position(INPUTS, binding.input),
        choose(context, INPUTS, move |input| Edit::MouseInput(index, input)),
    );
    let mut listens = Stack::vertical(space::XXS).child("input", input);
    if shadowed {
        listens = listens.child(
            "shadowed",
            text::caption("An earlier binding has the same input and modifiers: this one never fires."),
        );
    }

    let chips = Stack::horizontal(space::XS).children(MODIFIERS.iter().map(|&(name, modifier)| {
        let on = binding.modifiers & modifier.0 != 0;
        (
            name,
            chip(
                name,
                on,
                true,
                context.callback(move |_: bool| Msg::Edit(Edit::MouseModifier(index, modifier))),
            ),
        )
    }));

    let does: View = match binding.action {
        // A wheel changes the volume in the direction it turns.
        None => labeled_switch(
            "Reverse direction",
            binding.invert,
            context.callback(move |on: bool| Msg::Edit(Edit::MouseInvert(index, on))),
        ),
        Some(action) => {
            let kind = combo(
                labels(ACTIONS),
                position(ACTIONS, ActionKind::of(action)),
                choose(context, ACTIONS, move |kind| Edit::MouseAction(index, kind)),
            );
            match action {
                Action::SetVolume(level) => Grid::new()
                    .columns([GridLength::STAR, GridLength::Pixel(LEVEL_WIDTH)])
                    .column_spacing(space::XS)
                    .keyed_children([
                        ("action", cell(kind, 0, 0)),
                        (
                            "level",
                            cell(
                                number_box(
                                    Some(f64::from(level)),
                                    0.0,
                                    100.0,
                                    context.callback(move |value: Option<f64>| match value {
                                        Some(value) => Msg::Edit(Edit::MouseLevel(
                                            index,
                                            whole(value).min(100) as u8,
                                        )),
                                        None => Msg::Ignored,
                                    }),
                                ),
                                0,
                                1,
                            ),
                        ),
                    ]),
                _ => kind.into(),
            }
        }
    };

    // The mixer is opened, not aimed.
    let on: View = if binding.action == Some(Action::Mixer) {
        text::placeholder().into()
    } else {
        combo(
            labels(TARGETS),
            position(TARGETS, binding.target),
            choose(context, TARGETS, move |target| Edit::MouseTarget(index, target)),
        )
        .into()
    };

    Row::new(
        format!("mouse{index}"),
        vec![
            listens.into(),
            chips.into(),
            does,
            on,
            list_actions(List::Mouse, index, count, "Delete this binding", context),
        ],
    )
}

/// Width of the level box of a button that sets one.
const LEVEL_WIDTH: f64 = 96.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_input_the_file_can_hold_has_an_option() {
        for input in [
            Input::Wheel,
            Input::WheelTilt,
            Input::Left,
            Input::Right,
            Input::Middle,
            Input::X1,
            Input::X2,
        ] {
            assert!(position(INPUTS, input).is_some(), "{input:?}");
        }
        assert_eq!(INPUTS.len(), 7);
    }
}
