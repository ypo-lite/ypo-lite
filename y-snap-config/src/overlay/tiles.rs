//! The Overlay screen's Tiles tab: every tile the popup shows, as a compact
//! list — key, icon, label, group — with each tile's full editor (action,
//! label, icon, group, desktop and the key picker) opening in place under
//! its row.
//!
//! The old tab put all seven attributes of all tiles in one wide grid of
//! inputs: ~760px minimum, so beside the preview it always scrolled
//! sideways, and sixteen rows of combo boxes made the list itself hard to
//! scan. A list of what tiles *are*, plus one editor for the tile being
//! changed, fits the editor pane and reads like the popup it configures.

use super::OVERLAY_ACTIONS;
use crate::icon_picker::{IconPickerUi, icon_field, icon_picker};
use crate::key_events;
use crate::keys;
use crate::reorder::{list_footer, row_actions};
use crate::ui::key_picker::{KeyPickerUi, key_picker};
use crate::ui::prelude::*;
use crate::{ConfigApp, EditorClose, ListKind, Msg};
use windows_reactor::*;
use y_snap::action::Action;
use y_snap::config::{Item, Overlay};

const COLUMNS: [Column; 5] = [
    Column::fixed("Key", 64.0),
    Column::fixed("Icon", 40.0),
    Column::flex("Tile", 140.0),
    Column::flex("Group", 80.0),
    Column::actions(TWO_ICON_BUTTONS),
];

pub fn min_width() -> f64 {
    Table::new(&COLUMNS).min_width()
}

pub fn tiles_tab(
    overlay: &Overlay,
    editing: Option<usize>,
    reordering: bool,
    picker: &KeyPickerUi,
    icons: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let table = Table::new(&COLUMNS).empty_message("No tiles — the overlay would open empty.");
    let detail_viewport = table.detail_viewport(viewport).inset(CARD_INSET);

    let rows = overlay.items.iter().enumerate().map(|(index, item)| {
        let detail = (editing == Some(index)).then(|| {
            tile_editor(
                index,
                item,
                overlay,
                picker,
                icons,
                detail_viewport,
                context,
            )
        });
        Row::new(
            index.to_string(),
            cells(
                index,
                item,
                editing == Some(index),
                overlay.items.len(),
                reordering,
                context,
            ),
        )
        .detail(detail)
    });

    Stack::vertical(space::L)
        .child(
            "count",
            text::section_title(format!("{} tiles", overlay.items.len())),
        )
        .child("table", table.rows(rows).build(viewport))
        .child(
            "add",
            list_footer(
                ListKind::OverlayTiles,
                add_button("Add tile", context.callback(|()| Msg::OverlayItemAdded)),
                overlay.items.len(),
                reordering,
                context,
            ),
        )
        .into()
}

fn cells(
    index: usize,
    item: &Item,
    open: bool,
    count: usize,
    reordering: bool,
    context: &ViewContext<ConfigApp>,
) -> Vec<View> {
    let key: View = match item.key {
        Some(key) => keycaps(&[keys::label(key)], false),
        None => text::placeholder().into(),
    };
    let icon: View = match &item.icon {
        Some(icon) => text::body(icon.clone()).into(),
        None => text::placeholder().into(),
    };
    let tile: View = Stack::vertical(0.0)
        .child(
            "label",
            text::single_line(
                item.label
                    .clone()
                    .unwrap_or_else(|| "(no label)".to_owned()),
            ),
        )
        .child("action", text::caption(item.action.name()))
        .into();
    let group: View = match &item.group {
        Some(group) => text::caption(group.clone()).into(),
        None => text::placeholder().into(),
    };
    let actions = row_actions(
        ListKind::OverlayTiles,
        index,
        count,
        reordering,
        || {
            Stack::horizontal(space::XS)
                .child(
                    "edit",
                    icon_button(
                        if open { Symbol::Accept } else { Symbol::Edit },
                        if open { "Close editor" } else { "Edit tile" },
                        context.callback(move |()| Msg::EditTile(index)),
                    ),
                )
                .child(
                    "delete",
                    icon_button(
                        Symbol::Delete,
                        "Delete tile",
                        context.callback(move |()| Msg::OverlayItemDeleted(index)),
                    ),
                )
                .into()
        },
        context,
    );

    vec![key, icon, tile, group, actions]
}

fn tile_editor(
    index: usize,
    item: &Item,
    overlay: &Overlay,
    picker: &KeyPickerUi,
    icons: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let action = OVERLAY_ACTIONS.iter().position(|a| *a == item.action);
    // Choice 0 is always "(none)" — `Msg::OverlayItemGroupChanged` reads it so.
    let group_names =
        std::iter::once("(none)".to_owned()).chain(overlay.groups.iter().map(|g| g.id.clone()));
    let group = item
        .group
        .as_deref()
        .and_then(|id| overlay.groups.iter().position(|g| g.id == id))
        .map_or(0, |p| p + 1);

    let mut fields = AutoGrid::new(170.0)
        .max_columns(3)
        .item(
            "action",
            field(
                "Action",
                combo(
                    OVERLAY_ACTIONS.map(Action::name),
                    action,
                    context.callback(move |c| Msg::OverlayItemActionChanged(index, c)),
                ),
            ),
        )
        .item(
            "label",
            field(
                "Label",
                text_box(
                    item.label.clone().unwrap_or_default(),
                    "Shown under the icon",
                    context.callback(move |t| Msg::OverlayItemLabelChanged(index, t)),
                ),
            ),
        )
        .item(
            "icon",
            icon_field(
                item.icon.as_deref(),
                icons.open,
                context.callback(move |t| Msg::OverlayItemIconChanged(index, t)),
                context,
            ),
        )
        .item(
            "group",
            field(
                "Group",
                combo(
                    group_names,
                    Some(group),
                    context.callback(move |c| Msg::OverlayItemGroupChanged(index, c)),
                ),
            ),
        );
    if item.action.takes_desktop() {
        fields = fields.item(
            "desktop",
            field(
                "Desktop",
                number_box(
                    (item.desktop != 0).then_some(item.desktop.into()),
                    1.0,
                    99.0,
                    context.callback(move |v| Msg::OverlayItemDesktopChanged(index, v)),
                ),
            ),
        );
    }

    let key: View = if item.action.is_desktop_list() {
        note(
            "A desktop-list tile has no key of its own: it becomes one tile per virtual desktop, keyed 1, 2, 3….",
        )
    } else {
        section(
            "Key",
            key_picker(item.key, None, picker, viewport, &key_events(context)),
        )
    };

    editor(
        format!(
            "Tile — {}",
            item.label.as_deref().unwrap_or(item.action.name())
        ),
        Stack::vertical(space::XL)
            .child("fields", fields.build(viewport))
            .maybe(
                "icon-picker",
                icons.open.then(|| {
                    icon_picker(item.icon.as_deref(), item.action, icons, viewport, context)
                }),
            )
            .child("key", key),
        context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
        context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
    )
}
