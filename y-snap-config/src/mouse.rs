//! The Mouse gestures screen: one card per gesture (drag/resize),
//! each a button plus modifier chips.
//!
//! Cards sit in an [`AutoGrid`] — two across when they fit, fewer as the
//! window narrows. It replaces `VariableSizedWrapGrid`, whose one fixed
//! item height (210px) clipped a card the moment its hint wrapped.

use crate::ui::key_picker::MODIFIERS;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
use windows_reactor::*;
use y_snap::config::{Button as MouseButton, Gesture, Mouse};

pub const BUTTONS: [MouseButton; 3] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];
const BUTTON_NAMES: [&str; 3] = ["Left", "Right", "Middle"];

/// Which mouse gesture a [`Msg`] refers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GestureKind {
    Drag,
    Resize,
}

impl GestureKind {
    pub const ALL: [Self; 2] = [Self::Drag, Self::Resize];

    pub fn title(self) -> &'static str {
        match self {
            Self::Drag => "Drag",
            Self::Resize => "Resize",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Drag => "Moves the window under the pointer.",
            Self::Resize => "Resizes from the nearest edge or corner.",
        }
    }

    fn get(self, mouse: &Mouse) -> Option<Gesture> {
        match self {
            Self::Drag => mouse.drag,
            Self::Resize => mouse.resize,
        }
    }

    pub fn get_mut(self, mouse: &mut Mouse) -> &mut Option<Gesture> {
        match self {
            Self::Drag => &mut mouse.drag,
            Self::Resize => &mut mouse.resize,
        }
    }
}

pub fn mouse_view(mouse: &Mouse, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let grid = AutoGrid::new(250.0).max_columns(2);
    let card_viewport = grid
        .item_viewport(viewport, GestureKind::ALL.len())
        .inset(CARD_INSET);
    let cards = GestureKind::ALL.map(|kind| {
        (
            kind.title(),
            gesture_card(kind, kind.get(mouse), card_viewport, context),
        )
    });

    page(
        "Mouse gestures",
        "Hold the modifiers and use the button anywhere on a window — no need to aim for the title bar or an edge.",
        Stack::vertical(space::L).child("cards", grid.items(cards).build(viewport)).child(
            "note",
            note("A gesture that's switched off has no button assigned; switching it on starts from Left with no modifiers."),
        ),
    )
}

fn gesture_card(
    kind: GestureKind,
    gesture: Option<Gesture>,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let on = gesture.is_some();
    let button = gesture.and_then(|g| BUTTONS.iter().position(|b| *b == g.button));
    let modifiers = gesture.map_or(HOT_KEY_MODIFIERS(0), |g| g.modifiers);

    let title: View = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .keyed_children([
            (
                "title",
                cell(
                    text::strong(kind.title()).vertical_alignment(VerticalAlignment::Center),
                    0,
                    0,
                ),
            ),
            (
                "switch",
                cell(
                    switch(
                        on,
                        context.callback(move |on| Msg::GestureToggled(kind, on)),
                    ),
                    0,
                    1,
                ),
            ),
        ]);

    let chips = AutoGrid::new(56.0)
        .max_columns(4)
        .gap(space::XS, space::XS)
        .items(MODIFIERS.map(|(name, modifier)| {
            let checked = modifiers.0 & modifier.0 != 0;
            (
                name,
                chip(
                    name,
                    checked,
                    on,
                    context.callback(move |_| Msg::GestureModifierToggled(kind, modifier)),
                ),
            )
        }));

    let body = Stack::vertical(space::M)
        .child("title", title)
        .child("hint", text::caption(kind.hint()))
        .child(
            "button",
            field(
                "Button",
                combo(
                    BUTTON_NAMES,
                    button,
                    context.callback(move |c| Msg::GestureButtonChanged(kind, c)),
                )
                .is_enabled(on),
            ),
        )
        .child("modifiers", field("Modifiers", chips.build(viewport)));

    // Dimmed rather than disabled when off, so it still reads at a glance.
    card()
        .opacity(if on { 1.0 } else { emphasis::OFF })
        .content(body)
}
