//! The icon field and its picker, shared by the Overlay and Minimize-button
//! tile editors.
//!
//! The field stays a text box — anyone who knows Win + . or has a glyph to
//! paste keeps doing that — with an emoji button beside it that opens the
//! picker under the editor's fields:
//! - **suggested for this action** first (one click usually does it);
//! - a search box over the catalog's keywords ("pin", "arrow", "close");
//! - otherwise one category at a time, as equal buttons in an
//!   [`AutoGrid`], like the key picker;
//! - a closing line pointing at Windows' own emoji panel for the rest.
//!
//! Picking closes the picker. There is no Cancel/Done of its own: it lives
//! inside a tile editor that already has them, and Cancel there restores
//! the icon along with the rest of the tile.
//!
//! The picker can't open Windows' emoji panel itself: a WinUI 3 desktop
//! app has no supported API for it (`CoreInputView` needs a `CoreWindow`),
//! and faking a Win + . keypress is the kind of input injection the key
//! picker already turned down.

use crate::icons::{self, IconCategory};
use crate::ui::prelude::*;
use crate::ui::tokens::font;
use crate::{ConfigApp, Msg};
use windows_reactor::*;
use y_snap::action::Action;

/// Page-local state of the icon picker inside the open tile editor.
#[derive(Clone, Default, Debug)]
pub struct IconPickerUi {
    pub open: bool,
    pub category: IconCategory,
    pub search: String,
}

impl IconPickerUi {
    /// Opened on `icon`: its own category showing, if it's in the catalog.
    pub fn opening_on(icon: Option<&str>) -> Self {
        Self {
            open: true,
            category: icon.and_then(IconCategory::of).unwrap_or_default(),
            search: String::new(),
        }
    }
}

/// Width the category tabs need before they're swapped for a dropdown.
const TABS_MIN_WIDTH: f64 = 640.0;

/// The Icon form field: the text box plus the button opening the picker.
/// The button keeps the default (bordered) style rather than the subtle
/// one row actions use — it has to be noticed to help anyone.
pub fn icon_field(
    icon: Option<&str>,
    picker_open: bool,
    on_text: Callback<String>,
    context: &ViewContext<ConfigApp>,
) -> View {
    let input = text_box(
        icon.unwrap_or_default(),
        "Emoji or glyph (Win + . for all emoji)",
        on_text,
    );
    let button: View = Button::new()
        .style(if picker_open {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Default
        })
        .vertical_alignment(VerticalAlignment::Stretch)
        .on_click(context.callback(|()| Msg::IconPickerToggled))
        .content(SymbolIcon::new().symbol(Symbol::Emoji2))
        .tooltip(if picker_open {
            "Close the icon picker"
        } else {
            "Choose an icon"
        });

    let row: View = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .column_spacing(space::XS)
        .keyed_children([("input", cell(input, 0, 0)), ("button", cell(button, 0, 1))]);
    field("Icon", row)
}

/// The picker panel, full editor width, for a tile whose action is `action`
/// and whose icon is currently `current`.
pub fn icon_picker(
    current: Option<&str>,
    action: Action,
    ui: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let inner = viewport.inset(CARD_INSET);

    let suggested =
        Stack::horizontal(space::XS).children(icons::suggestions(action).iter().map(|&icon| {
            (
                icon,
                icon_choice(icon, current, Some(size::ICON_CHOICE), context),
            )
        }));

    let searching = !ui.search.trim().is_empty();
    let search = field(
        "Search",
        text_box(
            ui.search.clone(),
            "pin, arrow, close, desktop…",
            context.callback(Msg::IconSearchChanged),
        ),
    );

    let (heading, choices): (String, Vec<&'static str>) = if searching {
        let results = icons::search(&ui.search);
        (
            format!(
                "{} match{}",
                results.len(),
                if results.len() == 1 { "" } else { "es" }
            ),
            results,
        )
    } else {
        (
            ui.category.title().to_owned(),
            ui.category.icons().collect(),
        )
    };
    let grid: View = if choices.is_empty() {
        text::caption("Nothing matches — try another word, or use Win + . in the Icon box.").into()
    } else {
        AutoGrid::new(size::ICON_CHOICE)
            .max_columns(16)
            .gap(space::XS, space::XS)
            .keep_columns()
            .items(
                choices
                    .into_iter()
                    .map(|icon| (icon, icon_choice(icon, current, None, context))),
            )
            .build(inner)
    };

    let body = Stack::vertical(space::L)
        .child("suggested", field(&format!("Suggested for {action}"), suggested))
        .child("search", search)
        // Hidden while searching: results come from every category.
        .maybe("categories", (!searching).then(|| category_selector(ui.category, inner, context)))
        .child(format!("choices/{heading}"), section(heading.clone(), grid))
        .child(
            "hint",
            text::caption("Not in the list? Click in the Icon box and press Win + . to open Windows' emoji panel."),
        );

    panel(
        "Choose an icon",
        Some(icon_button(
            Symbol::Cancel,
            "Close the icon picker",
            context.callback(|()| Msg::IconPickerToggled),
        )),
        body,
    )
}

/// One pickable icon: fixed `width` in the suggestion row, filling its
/// cell in the grid. The current one is drawn with the Accent style — same
/// reason as the key picker: a style always follows state, a native
/// toggle's checked state doesn't.
fn icon_choice(
    icon: &'static str,
    current: Option<&str>,
    width: Option<f64>,
    context: &ViewContext<ConfigApp>,
) -> View {
    let button = Button::new()
        .style(if current == Some(icon) {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Default
        })
        .min_width(0.0)
        .fill();
    let button = match width {
        Some(width) => button
            .width(width)
            .horizontal_alignment(HorizontalAlignment::Left),
        None => button,
    };
    button
        .on_click(context.callback(move |()| Msg::IconPicked(icon)))
        .content(glyph(icon))
}

fn category_selector(
    current: IconCategory,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    if viewport.fits(TABS_MIN_WIDTH) {
        let items = IconCategory::ALL.map(|category| {
            let item: View = SelectorBarItem::new()
                .text(category.title())
                .is_selected(category == current)
                .into();
            (category.title(), item)
        });
        SelectorBar::new()
            .on_selected_text_changed(context.callback(move |title: Option<String>| {
                Msg::IconCategorySelected(
                    title
                        .as_deref()
                        .and_then(IconCategory::from_title)
                        .unwrap_or(current),
                )
            }))
            .slots([SlotView::collection(SelectorBarSlot::Items, items)])
    } else {
        let selected = IconCategory::ALL
            .iter()
            .position(|&category| category == current);
        let picker = combo(
            IconCategory::ALL.map(IconCategory::title),
            selected,
            context.callback(move |choice: Option<usize>| {
                Msg::IconCategorySelected(choice.map_or(current, |index| IconCategory::ALL[index]))
            }),
        );
        field("Category", picker)
    }
}

/// An emoji at a size that reads as an icon, centred in its button.
fn glyph(icon: &'static str) -> TextBlock {
    TextBlock::new()
        .text(icon)
        .font_size(font::BODY_LARGE)
        .horizontal_alignment(HorizontalAlignment::Center)
}
