//! Containers that give content a visual home: page, section, card, panel,
//! note. One look per role, so a card on one screen and a card on another
//! are the same card.

use crate::layout::{Stack, cell};
use crate::text;
use crate::viewport::Viewport;
use crate::tokens::{radius, size, space};
use windows_reactor::*;

/// Padding inside a [`card`] — exported so a responsive card body can
/// compute its own [`crate::viewport::Viewport`] with
/// `viewport.inset(CARD_INSET)`.
pub const CARD_PADDING: f64 = space::L;
/// Everything a card takes off each side: padding plus its border.
pub const CARD_INSET: f64 = CARD_PADDING + size::STROKE;

/// The card frame: rounded, stroked, on the card background. Returns the
/// `Border` builder so a caller can still set `.opacity()` (an off gesture)
/// before `.content(..)`.
pub fn card() -> Border {
    Border::new()
        .padding(Thickness::uniform(CARD_PADDING))
        .corner_radius(CornerRadius::uniform(radius::CARD))
        .border_thickness(Thickness::uniform(size::STROKE))
        .border_brush(ThemeBrush::CardStroke)
        .background(ThemeBrush::CardBackground)
}

/// A card with a header row — `title` on the left, `trailing` (actions,
/// a status) on the right — over `body`. The editor panels and the live
/// preview are all one of these.
pub fn panel(title: impl Into<String>, trailing: Option<View>, body: impl Into<View>) -> View {
    let mut header = vec![("title", cell(text::strong(title).vertical_alignment(VerticalAlignment::Center), 0, 0))];
    if let Some(trailing) = trailing {
        header.push(("trailing", cell(trailing, 0, 1)));
    }
    let header: View =
        Grid::new().columns([GridLength::STAR, GridLength::Auto]).column_spacing(space::M).keyed_children(header);

    card().content(Stack::vertical(space::M).child("header", header).child("body", body))
}

/// A titled block inside a page ("Layout", "Palette", "Tiles — 16").
pub fn section(title: impl Into<String>, body: impl Into<View>) -> View {
    Stack::vertical(space::S).child("title", text::section_title(title)).child("body", body).into()
}

/// A short explanatory callout with an accent bar — the layout mock's
/// `.note`, for the one thing on a screen worth explaining.
pub fn note(message: impl Into<String>) -> View {
    Border::new()
        .padding(Thickness::xy(space::M, space::S + 2.0))
        .corner_radius(CornerRadius::uniform(radius::CONTROL))
        .border_thickness(Thickness::new(3.0, 0.0, 0.0, 0.0))
        .border_brush(ThemeBrush::Accent)
        .background(ThemeBrush::CardBackground)
        .content(text::caption(message))
}

/// A screen: title, lede, then `body`.
pub fn page(title: &str, lede: &str, body: impl Into<View>) -> View {
    Stack::vertical(space::XL)
        .child("header", Stack::vertical(space::XS).child("title", text::title(title)).child("lede", text::lede(lede)))
        .child("body", body)
        .into()
}

/// Narrowest content width at which a [`setting`] keeps its control beside
/// its text; narrower, the control goes underneath.
pub const SETTING_SIDE_BY_SIDE_MIN: f64 = 460.0;

/// One setting in a card: what it is and what it does on the left, its
/// control on the right — under the text when the card is too narrow for
/// both. `viewport` is the width the *card* gets (it takes its own padding
/// off).
pub fn setting(title: &str, description: &str, control: impl Into<View>, viewport: Viewport) -> View {
    let words = Stack::vertical(space::XXS)
        .child("title", text::body(title))
        .child("description", text::caption(description));
    let control = control.into();

    let body: View = if viewport.inset(CARD_INSET).fits(SETTING_SIDE_BY_SIDE_MIN) {
        Grid::new()
            .columns([GridLength::STAR, GridLength::Auto])
            .column_spacing(space::L)
            .keyed_children([
                ("words", cell(Border::new().vertical_alignment(VerticalAlignment::Center).content(words), 0, 0)),
                ("control", cell(Border::new().vertical_alignment(VerticalAlignment::Center).content(control), 0, 1)),
            ])
    } else {
        Stack::vertical(space::M).child("words", words).child("control", control).into()
    };
    card().content(body)
}
