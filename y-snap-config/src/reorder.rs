//! Reordering, the same way on every list (hotkeys, overlay groups and
//! tiles, popup tiles): a "Reorder" toggle beside the list's add button
//! turns each row's actions (edit/delete) into ↑/↓ until it's toggled off.
//!
//! A mode rather than permanent ↑/↓ on every row: the extra ~92px per
//! table pushed the Overlay preview below the tile list at ordinary window
//! sizes, for something done occasionally. The ↑/↓ pair takes exactly the
//! actions column's width, so nothing moves when the mode switches.
//! Entering it closes any open editor (`ConfigApp::update`), so no editor
//! is ever left pointing at a row that moved.

use crate::ui::prelude::*;
use crate::{ConfigApp, ListKind, Msg};
use windows_reactor::*;

/// A row's actions cell: ↑/↓ while `reordering`, otherwise `actions()`.
pub fn row_actions(
    list: ListKind,
    index: usize,
    count: usize,
    reordering: bool,
    actions: impl FnOnce() -> View,
    context: &ViewContext<ConfigApp>,
) -> View {
    if reordering {
        move_buttons(
            index,
            count,
            context.callback(move |()| Msg::Moved {
                list,
                index,
                up: true,
            }),
            context.callback(move |()| Msg::Moved {
                list,
                index,
                up: false,
            }),
        )
    } else {
        actions()
    }
}

/// Under a list: its add button, plus the Reorder toggle once there are
/// two rows to reorder (or while the mode is on, so it can be left).
pub fn list_footer(
    list: ListKind,
    add: View,
    count: usize,
    reordering: bool,
    context: &ViewContext<ConfigApp>,
) -> View {
    let toggle = (count > 1 || reordering).then(|| {
        reorder_toggle(
            reordering,
            context.callback(move |()| Msg::ReorderToggled(list)),
        )
    });
    Stack::horizontal(space::S)
        .child("add", add)
        .maybe("reorder", toggle)
        .into()
}
