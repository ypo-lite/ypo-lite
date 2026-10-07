//! The Overlay screen's Layout tab: six numeric geometry fields and the
//! focus-border switch.

use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;
use y_snap::config::Overlay;

/// Up to three fields abreast, all one width — the [`AutoGrid`] decides,
/// not the label lengths.
pub fn layout_tab(overlay: &Overlay, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let number = |label: &str, value: u32, min: f64, max: f64, message: fn(Option<f64>) -> Msg| {
        field(label, number_box(Some(value.into()), min, max, context.callback(message)))
    };

    let fields = AutoGrid::new(110.0).max_columns(3).items([
        ("columns", number("Columns", overlay.columns, 1.0, 12.0, Msg::OverlayColumnsChanged)),
        ("tile-width", number("Tile width", overlay.tile.0, 16.0, 400.0, Msg::OverlayTileWidthChanged)),
        ("tile-height", number("Tile height", overlay.tile.1, 16.0, 400.0, Msg::OverlayTileHeightChanged)),
        ("corner-radius", number("Corner radius", overlay.corner_radius, 0.0, 40.0, Msg::OverlayCornerRadiusChanged)),
        ("border-width", number("Border width", overlay.border_width, 0.0, 10.0, Msg::OverlayBorderWidthChanged)),
        ("alpha", number("Background alpha", overlay.alpha.into(), 0.0, 255.0, Msg::OverlayAlphaChanged)),
    ]);

    Stack::vertical(space::L)
        .child("fields", fields.build(viewport))
        .child(
            "focus-border",
            labeled_switch(
                "Outline the target window while the overlay is open",
                overlay.focus_border,
                context.callback(Msg::OverlayFocusBorderToggled),
            ),
        )
        .into()
}
