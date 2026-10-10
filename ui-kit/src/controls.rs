//! Input controls, pre-configured the one way this app uses each.
//!
//! Every constructor here:
//! - calls [`Fill::fill`], so the control fills its cell and centres
//!   vertically whatever its native default alignment is;
//! - takes a [`Callback`] (build one with `context.callback(...)`) rather
//!   than a `ViewContext`, so `ui` never depends on the app's `Msg`;
//! - returns the concrete builder when the result is still one bare native
//!   control (`ComboBox`, `NumberBox`, `TextBox`) so the caller can chain
//!   `.is_enabled(..)` and friends — and a `View` once it has content or a
//!   tooltip, because `.content(..)` is what turns a builder into a `View`.
//!   Those take what a caller would have chained (`enabled`, `accent`) as
//!   parameters instead.

use crate::layout::{Fill, Stack, cell};
use crate::text;
use crate::tokens::{font, radius, size, space};
use windows_reactor::*;

/// A dropdown over plain strings, reporting the chosen index.
pub fn combo<S: Into<String>>(
    items: impl IntoIterator<Item = S>,
    selected: Option<usize>,
    on_change: Callback<Option<usize>>,
) -> ComboBox {
    ComboBox::new().items_source(items).selected_index(selected).on_selection_changed(on_change).fill()
}

/// A numeric field. `value: None` shows an empty box — what a group's
/// "inherit the default" override looks like.
pub fn number_box(value: Option<f64>, minimum: f64, maximum: f64, on_change: Callback<Option<f64>>) -> NumberBox {
    NumberBox::new().minimum(minimum).maximum(maximum).value(value).on_value_changed(on_change).fill()
}

/// A slider for feel plus a number box for an exact value, side by side —
/// a slider alone can't easily land on 700 rather than 694.
///
/// Both report through their own callback (the slider's never-empty `f64`,
/// the box's `Option<f64>`); map both to the same message. **WinUI fires
/// `ValueChanged` when a value is first set**, not only on user input, so
/// the message handler must ignore a value equal to the current one or
/// merely opening the screen marks it dirty.
pub fn range_input(
    value: f64,
    minimum: f64,
    maximum: f64,
    step: f64,
    on_slide: Callback<f64>,
    on_type: Callback<Option<f64>>,
) -> View {
    let slider = Slider::new()
        .minimum(minimum)
        .maximum(maximum)
        .step_frequency(step)
        .value(value)
        .on_value_changed(on_slide)
        .fill();
    Grid::new()
        .columns([GridLength::STAR, GridLength::Pixel(RANGE_NUMBER_WIDTH)])
        .column_spacing(space::M)
        .keyed_children([
            ("slider", cell(slider, 0, 0)),
            ("number", cell(number_box(Some(value), minimum, maximum, on_type), 0, 1)),
        ])
}

/// Width of [`range_input`]'s number box — four digits plus WinUI's own
/// text-box padding.
const RANGE_NUMBER_WIDTH: f64 = 96.0;

/// A single-line text field.
pub fn text_box(text: impl Into<String>, placeholder: &str, on_change: Callback<String>) -> TextBox {
    TextBox::new().text(text).placeholder_text(placeholder).on_text_changed(on_change).fill()
}

/// An on/off switch without WinUI's built-in "On"/"Off" caption, and
/// without its 154px style `MinWidth` — which is what made the old Hotkeys
/// "Steal" column spill into its neighbour. Just the 40px track.
///
/// The on/off slots get a zero-size `Border`, not `View::empty()`: an empty
/// fragment has no native element, so the crate sets the slot to `null` —
/// and a `null` `OnContent` is precisely what makes WinUI fall back to its
/// localized default text ("Activé"/"Désactivé"), which then got cropped.
pub fn switch(is_on: bool, on_toggle: Callback<bool>) -> View {
    let nothing = || -> View { Border::new().width(0.0).height(0.0).into() };
    ToggleSwitch::new().is_on(is_on).on_toggled(on_toggle).min_width(0.0).fill().slots([
        SlotView::new(ToggleSwitchSlot::OnContent, nothing()),
        SlotView::new(ToggleSwitchSlot::OffContent, nothing()),
    ])
}

/// A switch with its description beside it — clicking the text toggles it
/// too, which a separate `TextBlock` next to a [`switch`] wouldn't do.
pub fn labeled_switch(label: &str, is_on: bool, on_toggle: Callback<bool>) -> View {
    let label = || -> View { text::body(label).into() };
    ToggleSwitch::new().is_on(is_on).on_toggled(on_toggle).min_width(0.0).fill().slots([
        SlotView::new(ToggleSwitchSlot::OnContent, label()),
        SlotView::new(ToggleSwitchSlot::OffContent, label()),
    ])
}

/// A two-state chip (a modifier: Win/Ctrl/Alt/Shift; a key on the key
/// picker). Fills its cell, so chips in an [`crate::layout::AutoGrid`]
/// are all one width.
pub fn chip(label: &str, checked: bool, enabled: bool, on_toggle: Callback<bool>) -> View {
    ToggleButton::new()
        .is_checked(checked)
        .is_enabled(enabled)
        .on_is_checked_changed(on_toggle)
        .min_width(0.0)
        .fill()
        .content(label)
}

/// Icon-only button with a tooltip naming what it does — the edit/delete
/// buttons in table rows.
///
/// Deliberately *not* given a fixed width/height: `SymbolIcon` always
/// draws its glyph at 20px, and `Button` keeps its native ~11px side
/// padding — this crate version exposes neither a `Button` padding nor an
/// icon size builder — so a 32×32 box left ~10px for a 20px glyph and
/// cropped it. Sized by its content instead, it comes out about
/// [`size::ICON_BUTTON`] wide; columns holding one reserve that much.
pub fn icon_button(symbol: Symbol, tooltip: &str, on_click: Callback<()>) -> View {
    subtle_icon_button(SymbolIcon::new().symbol(symbol).into(), tooltip, true, on_click)
}

/// [`icon_button`]'s body, for any icon view and with an enabled state.
fn subtle_icon_button(icon: View, tooltip: &str, enabled: bool, on_click: Callback<()>) -> View {
    Button::new()
        .style(ButtonStyle::Subtle)
        .is_enabled(enabled)
        .min_width(0.0)
        .horizontal_content_alignment(HorizontalAlignment::Center)
        .vertical_content_alignment(VerticalAlignment::Center)
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .on_click(on_click)
        .content(icon)
        .tooltip(tooltip)
}

/// Segoe Fluent Icons / MDL2 arrow glyphs. `Symbol` has `Up` but no
/// matching `Down`, so both come from the font for a matched pair.
const GLYPH_UP: &str = "\u{E74A}";
const GLYPH_DOWN: &str = "\u{E74B}";

/// The ↑/↓ pair that replaces a row's actions while its list is being
/// reordered — the same width as two [`icon_button`]s, so the actions
/// column doesn't change size between modes. The first row can't go up
/// and the last can't go down; those buttons are disabled, not hidden, so
/// the pair stays aligned down the column.
pub fn move_buttons(index: usize, count: usize, on_up: Callback<()>, on_down: Callback<()>) -> View {
    Stack::horizontal(space::XS)
        .child("up", subtle_icon_button(FontIcon::new().glyph(GLYPH_UP).into(), "Move up", index > 0, on_up))
        .child(
            "down",
            subtle_icon_button(FontIcon::new().glyph(GLYPH_DOWN).into(), "Move down", index + 1 < count, on_down),
        )
        .into()
}

/// Width an actions column reserves for [`move_buttons`] (or any two
/// [`icon_button`]s side by side).
pub const TWO_ICON_BUTTONS: f64 = 2.0 * size::ICON_BUTTON + space::XS;

/// The "Reorder" toggle beside a list's add button. While on, rows show
/// [`move_buttons`] instead of their usual actions; Accent marks the mode,
/// and the label says how to leave it.
pub fn reorder_toggle(on: bool, on_toggle: Callback<()>) -> View {
    Button::new()
        .style(if on { ButtonStyle::Accent } else { ButtonStyle::Default })
        .horizontal_alignment(HorizontalAlignment::Left)
        .on_click(on_toggle)
        .content(
            Stack::horizontal(space::S)
                .child("icon", SymbolIcon::new().symbol(Symbol::Sort))
                .child("label", text::body(if on { "Done reordering" } else { "Reorder" })),
        )
}

/// "+ Add hotkey" — the add-a-row action under a list.
pub fn add_button(label: &str, on_click: Callback<()>) -> View {
    Button::new().on_click(on_click).horizontal_alignment(HorizontalAlignment::Left).content(
        Stack::horizontal(space::S)
            .child("icon", SymbolIcon::new().symbol(Symbol::Add))
            .child("label", text::body(label)),
    )
}

/// A plain push button. `accent` marks the one primary action of its area
/// (Save, Done) — at most one per area.
pub fn button(label: &str, accent: bool, enabled: bool, on_click: Callback<()>) -> View {
    Button::new()
        .style(if accent { ButtonStyle::Accent } else { ButtonStyle::Default })
        .is_enabled(enabled)
        .vertical_alignment(VerticalAlignment::Center)
        .on_click(on_click)
        .content(label)
}

/// A label above a control — every form field.
pub fn field(label: &str, control: impl Into<View>) -> View {
    Stack::vertical(space::XS).child("label", text::caption(label)).child("control", control).into()
}

/// A key combination as keycaps: `[Ctrl] [Alt] [F5]`. `large` is the
/// picker's readout; the default size fits inside a table row's button.
pub fn keycaps(keys: &[String], large: bool) -> View {
    let (font_size, padding) = if large {
        (font::BODY_LARGE, Thickness::xy(space::M, space::XS))
    } else {
        (font::CAPTION, Thickness::xy(space::S, 1.0))
    };

    Stack::horizontal(space::XS)
        .children(keys.iter().enumerate().map(|(index, key)| {
            // A thicker bottom edge is what makes a box read as a key.
            let cap: View = Border::new()
                .padding(padding.clone())
                .corner_radius(CornerRadius::uniform(radius::CONTROL))
                .border_thickness(Thickness::new(size::STROKE, size::STROKE, size::STROKE, 2.0 * size::STROKE))
                .border_brush(ThemeBrush::CardStroke)
                .background(ThemeBrush::CardBackground)
                .content(TextBlock::new().text(key.clone()).font_size(font_size).font_weight(FontWeight::SEMI_BOLD));
            (index, cap)
        }))
        .into()
}

/// A small filled circle — a status light.
pub fn dot(brush: impl Into<Brush>) -> View {
    Border::new()
        .width(8.0)
        .height(8.0)
        .corner_radius(CornerRadius::uniform(4.0))
        .background(brush)
        .vertical_alignment(VerticalAlignment::Center)
        .into()
}
