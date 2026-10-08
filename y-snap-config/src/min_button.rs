//! The Minimize button screen: how long a pointer must rest on a window's
//! minimize button before y-snap's small popup appears, the tiles in that
//! popup, and what a plain right-click on the button does instead.
//!
//! Same shape as the Overlay screen — settings beside a live preview, as a
//! [`Split`] — built entirely from the toolkit (`docs/ui-toolkit.md`).
//! The tile list is its own rather than shared with Overlay's Tiles tab:
//! a popup tile has no key, group or desktop, so the shared part would be
//! little more than the toolkit's own `Table` + `editor` already are.
//!
//! No reordering yet (same as overlay tiles): the popup shows tiles in file
//! order, so a new tile goes at the end.

use crate::icon_picker::{IconPickerUi, icon_field, icon_picker};
use crate::overlay::OVERLAY_ACTIONS;
use crate::overlay::palette::to_ui_color;
use crate::ui::prelude::*;
use crate::ui::tokens::font;
use crate::{ConfigApp, EditorClose, Msg};
use windows_reactor::*;
use y_snap::action::Action;
use y_snap::config::{MinButton, Overlay, QuickAction, is_minbutton_action};

/// Every action a popup tile or the right-click can fire — exactly what
/// `y-snap`'s config reader accepts in `<minbutton>`.
pub fn actions() -> Vec<Action> {
    OVERLAY_ACTIONS.into_iter().filter(|&action| is_minbutton_action(action)).collect()
}

/// The slider's range. A hand-edited file may hold more; the range then
/// stretches to include it rather than clamping it on display.
const DELAY_MAX_MS: f64 = 2000.0;
const DELAY_STEP_MS: f64 = 50.0;

const COLUMNS: [Column; 3] =
    [Column::fixed("Icon", 48.0), Column::flex("Tile", 160.0), Column::actions(2.0 * size::ICON_BUTTON + space::XS)];

/// Smallest the preview gets beside the settings before moving below them.
const PREVIEW_MIN_WIDTH: f64 = 300.0;
const SETTINGS_MIN_WIDTH: f64 = 360.0;

pub fn min_button_view(
    min_button: &MinButton,
    overlay: &Overlay,
    editing: Option<usize>,
    icons: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let split = Split::new(viewport).min_widths(SETTINGS_MIN_WIDTH, PREVIEW_MIN_WIDTH).weights(1.2, 1.0);
    let settings = settings(min_button, editing, icons, split.primary(), context);
    let preview = preview(min_button, overlay, split.secondary());

    page(
        "Minimize button",
        "Rest the pointer on any window's minimize button to get a small row of tiles instead of minimizing \
         right away. A plain right-click on the button skips the popup and fires one action directly.",
        split.build(settings, preview),
    )
}

fn settings(
    min_button: &MinButton,
    editing: Option<usize>,
    icons: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let actions = actions();
    let names = || actions.iter().map(|action| action.name());

    let delay = min_button.hover_delay_ms as f64;
    // Choice 0 is "(off)" — `Msg::MinButtonRightClickChanged` reads it so.
    let right_click = std::iter::once("(off — a normal right-click)").chain(names());
    let right_click_index =
        min_button.right_click_action.and_then(|a| actions.iter().position(|b| *b == a)).map_or(0, |i| i + 1);

    let behaviour = AutoGrid::new(240.0).max_columns(2).items([
        (
            "delay",
            field(
                "Hover delay before the popup appears (ms)",
                range_input(
                    delay,
                    0.0,
                    DELAY_MAX_MS.max(delay),
                    DELAY_STEP_MS,
                    context.callback(|v: f64| Msg::MinButtonDelayChanged(Some(v))),
                    context.callback(Msg::MinButtonDelayChanged),
                ),
            ),
        ),
        (
            "right-click",
            field(
                "Right-click action",
                combo(right_click, Some(right_click_index), context.callback(Msg::MinButtonRightClickChanged)),
            ),
        ),
    ]);

    let table = Table::new(&COLUMNS).empty_message("No tiles — hovering the button shows nothing.");
    let detail_viewport = table.detail_viewport(viewport).inset(CARD_INSET);
    let rows = min_button.items.iter().enumerate().map(|(index, item)| {
        let open = editing == Some(index);
        let detail = open.then(|| tile_editor(index, item, &actions, icons, detail_viewport, context));
        Row::new(index.to_string(), cells(index, item, open, context)).detail(detail)
    });

    Stack::vertical(space::XL)
        .child("behaviour", section("Behaviour", behaviour.build(viewport)))
        .child(
            "tiles",
            section(
                format!("Popup tiles — {}", min_button.items.len()),
                Stack::vertical(space::L)
                    .child("table", table.rows(rows).build(viewport))
                    .child("add", add_button("Add tile", context.callback(|()| Msg::QuickActionAdded))),
            ),
        )
        .into()
}

fn cells(index: usize, item: &QuickAction, open: bool, context: &ViewContext<ConfigApp>) -> Vec<View> {
    let icon: View = match &item.icon {
        Some(icon) => text::body(icon.clone()).into(),
        None => text::placeholder().into(),
    };
    let tile: View = Stack::vertical(0.0)
        .child("label", text::single_line(item.label.clone().unwrap_or_else(|| "(no label)".to_owned())))
        .child("action", text::caption(item.action.name()))
        .into();
    let actions: View = Stack::horizontal(space::XS)
        .child(
            "edit",
            icon_button(
                if open { Symbol::Accept } else { Symbol::Edit },
                if open { "Close editor" } else { "Edit tile" },
                context.callback(move |()| Msg::EditQuickAction(index)),
            ),
        )
        .child(
            "delete",
            icon_button(Symbol::Delete, "Delete tile", context.callback(move |()| Msg::QuickActionDeleted(index))),
        )
        .into();
    vec![icon, tile, actions]
}

fn tile_editor(
    index: usize,
    item: &QuickAction,
    actions: &[Action],
    icons: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let action = actions.iter().position(|a| *a == item.action);
    let fields = AutoGrid::new(170.0)
        .max_columns(3)
        .item(
            "action",
            field(
                "Action",
                combo(
                    actions.iter().map(|a| a.name()),
                    action,
                    context.callback(move |c| Msg::QuickActionActionChanged(index, c)),
                ),
            ),
        )
        .item(
            "label",
            field(
                "Label",
                text_box(
                    item.label.clone().unwrap_or_default(),
                    "Used when there's no icon",
                    context.callback(move |t| Msg::QuickActionLabelChanged(index, t)),
                ),
            ),
        )
        .item(
            "icon",
            icon_field(
                item.icon.as_deref(),
                icons.open,
                context.callback(move |t| Msg::QuickActionIconChanged(index, t)),
                context,
            ),
        );

    editor(
        format!("Popup tile — {}", item.label.as_deref().unwrap_or(item.action.name())),
        Stack::vertical(space::L)
            .child("fields", fields.build(viewport))
            .maybe(
                "icon-picker",
                icons.open.then(|| icon_picker(item.icon.as_deref(), item.action, icons, viewport, context)),
            )
            .child("note", note("The popup shows the icon; the label is shown only when there's no icon.")),
        context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
        context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
    )
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// The popup's own geometry (`y-snap/src/min_overlay.rs`): one row of
/// 72×40 tiles, an 18px glyph centred in each.
const POPUP_TILE: (f64, f64) = (72.0, 40.0);
const POPUP_GLYPH_SIZE: f64 = 18.0;
/// A Windows 11 caption button.
const CAPTION_BUTTON: (f64, f64) = (46.0, 32.0);
/// Room the stub window keeps for its title left of the caption buttons.
const STUB_TITLE_WIDTH: f64 = 120.0;

/// A stub window's title bar with the popup hanging under its minimize
/// button, scaled to fit like the overlay preview — drawn with the
/// overlay's palette and corner radius, which the real popup shares.
fn preview(min_button: &MinButton, overlay: &Overlay, viewport: Viewport) -> View {
    let tiles = min_button.items.len();
    let popup_width = POPUP_TILE.0 * tiles as f64;
    let stub_width = STUB_TITLE_WIDTH + 3.0 * CAPTION_BUTTON.0;
    let available = viewport.inset(CARD_INSET).inset(space::L + size::STROKE).width;
    let scale = (available / popup_width.max(stub_width)).clamp(0.4, 1.0);

    let caption = |glyph: &'static str, highlighted: bool| -> View {
        let mut button = Border::new().width(CAPTION_BUTTON.0 * scale).height(CAPTION_BUTTON.1 * scale);
        if highlighted {
            button = button.background(ThemeBrush::Accent);
        }
        button.content(
            TextBlock::new()
                .text(glyph)
                .font_size((font::CAPTION * scale).max(6.0))
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center),
        )
    };
    let title_bar: View =
        Grid::new().columns([GridLength::STAR, GridLength::Auto, GridLength::Auto, GridLength::Auto]).keyed_children([
            (
                "title",
                cell(
                    text::caption("Any window")
                        .vertical_alignment(VerticalAlignment::Center)
                        .margin(Thickness::xy(space::S, 0.0)),
                    0,
                    0,
                ),
            ),
            ("min", cell(caption("—", true), 0, 1)),
            ("max", cell(caption("☐", false), 0, 2)),
            ("close", cell(caption("✕", false), 0, 3)),
        ]);
    let window: View = Border::new()
        .corner_radius(CornerRadius::new(radius::CARD, radius::CARD, 0.0, 0.0))
        .border_thickness(Thickness::new(size::STROKE, size::STROKE, size::STROKE, 0.0))
        .border_brush(ThemeBrush::CardStroke)
        .background(ThemeBrush::CardBackground)
        .content(title_bar);

    let popup: View = if tiles == 0 {
        text::caption("No tiles — nothing appears on hover.").into()
    } else {
        popup(min_button, overlay, scale, available)
    };

    let right_click = match min_button.right_click_action {
        Some(action) => format!("Right-click: {action}"),
        None => "Right-click: normal".to_owned(),
    };

    let stage = Stack::vertical(0.0).child("window", window).child("popup", popup);
    let live: View = Stack::horizontal(space::XS)
        .child("dot", dot(ThemeBrush::Accent))
        .child(
            "text",
            text::caption(if scale < 1.0 { format!("Live · {:.0}%", scale * 100.0) } else { "Live".to_owned() }),
        )
        .into();

    panel(
        "Preview",
        Some(live),
        Stack::vertical(space::M)
            .child("stage", stage)
            .child(
                "delay",
                text::caption(format!("After resting {} ms on the minimize button.", min_button.hover_delay_ms)),
            )
            .child("right-click", text::caption(right_click)),
    )
}

/// The popup row, centred under the minimize button the way
/// `MinOverlay::show` positions it: its centre on the button's centre, so
/// it pokes out past the window's right edge once it's wider than the
/// two buttons right of it allow. Clamped to the stage.
fn popup(min_button: &MinButton, overlay: &Overlay, scale: f64, available: f64) -> View {
    let (width, height) = (POPUP_TILE.0 * scale, POPUP_TILE.1 * scale);
    let popup_width = width * min_button.items.len() as f64;
    let button_centre_from_right = 2.5 * CAPTION_BUTTON.0 * scale;
    let right = (button_centre_from_right - popup_width / 2.0).clamp(0.0, (available - popup_width).max(0.0));

    let glyphs = min_button.items.iter().enumerate().map(|(index, item)| {
        let glyph = item.icon.as_deref().or(item.label.as_deref()).unwrap_or("?").to_owned();
        let text = TextBlock::new()
            .text(glyph)
            .font_size((POPUP_GLYPH_SIZE * scale).max(6.0))
            .foreground(to_ui_color(overlay.palette.text))
            .text_trimming(TextTrimming::CharacterEllipsis)
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center);
        (index, cell(text, 0, index))
    });
    let row: View = Grid::new()
        .columns(vec![GridLength::Pixel(width); min_button.items.len()])
        .rows([GridLength::Pixel(height)])
        .keyed_children(glyphs.collect::<Vec<_>>());

    Border::new()
        .horizontal_alignment(HorizontalAlignment::Right)
        .margin(Thickness::new(0.0, 0.0, right, 0.0))
        .corner_radius(CornerRadius::uniform(overlay.corner_radius as f64 * scale))
        .background(to_ui_color(y_core::overlay::Color { a: overlay.alpha, ..overlay.palette.back }))
        .content(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offered_actions_are_exactly_what_the_config_reader_accepts() {
        let offered = actions();
        assert!(offered.contains(&Action::Traynize) && offered.contains(&Action::AlwaysOnTop));
        assert!(!offered.contains(&Action::Overlay) && !offered.contains(&Action::DesktopList));
        assert!(offered.iter().all(|&action| is_minbutton_action(action)));
    }
}
