//! The Overlay screen's Groups tab: per-group grid overrides (a group can
//! override the overlay's columns and tile size for its own tiles).

use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;
use y_snap::config::{Group, Overlay};

const COLUMNS: [Column; 5] = [
    Column::flex("Group", 90.0),
    Column::flex("Columns", 90.0),
    Column::flex("Tile width", 90.0),
    Column::flex("Tile height", 90.0),
    Column::actions(size::ICON_BUTTON),
];

pub fn min_width() -> f64 {
    Table::new(&COLUMNS).min_width()
}

pub fn groups_tab(overlay: &Overlay, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let rows = overlay
        .groups
        .iter()
        .enumerate()
        .map(|(index, group)| Row::new(group.id.clone(), cells(index, group, context)));

    Stack::vertical(space::L)
        .child(
            "table",
            Table::new(&COLUMNS)
                .empty_message("No groups. Tiles in a group get their own row in the popup, sized independently.")
                .rows(rows)
                .build(viewport),
        )
        .child("add", add_button("Add group", context.callback(|()| Msg::OverlayGroupAdded)))
        .child(
            "note",
            note(
                "Leave a number empty to use the overlay's own value. A new group is named group-N; rename it in \
                 y-snap.xml for now — renaming here would also mean rewriting every tile that refers to it.",
            ),
        )
        .into()
}

fn cells(index: usize, group: &Group, context: &ViewContext<ConfigApp>) -> Vec<View> {
    let number = |value: Option<u32>, maximum: f64, message: fn(usize, Option<f64>) -> Msg| -> View {
        number_box(value.map(f64::from), 1.0, maximum, context.callback(move |v| message(index, v))).into()
    };
    vec![
        text::strong(group.id.clone()).vertical_alignment(VerticalAlignment::Center).into(),
        number(group.columns, 20.0, Msg::OverlayGroupColumnsChanged),
        number(group.tile_width, 400.0, Msg::OverlayGroupTileWidthChanged),
        number(group.tile_height, 400.0, Msg::OverlayGroupTileHeightChanged),
        icon_button(Symbol::Delete, "Delete group", context.callback(move |()| Msg::OverlayGroupDeleted(index))),
    ]
}

/// A fresh `group-N` id, not already used by an existing group.
pub fn next_group_id(groups: &[Group]) -> String {
    (1..).map(|n| format!("group-{n}")).find(|id| !groups.iter().any(|g| &g.id == id)).expect("infinite iterator")
}
