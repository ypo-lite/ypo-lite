//! Design tokens: the only place a spacing, font size, radius or opacity is
//! allowed to be a literal number. Everything else in the app names one of
//! these, so two screens built months apart still line up.
//!
//! The type ramp and radii are WinUI's own (Fluent 2: Caption 12, Body 14,
//! Subtitle 20, `ControlCornerRadius` 4, `OverlayCornerRadius` 8) rather
//! than the layout mock's web sizes — the native controls on the page
//! (`ComboBox`, `NumberBox`, `Button`, …) already render at Body 14 with a
//! 4px radius, and text sized to match them is what keeps a label and the
//! control next to it reading as one row.

/// Spacing scale, in DIPs. A 4px grid: every gap, padding and margin in the
/// app is one of these.
pub mod space {
    /// Between a field's label and its control; inside a keycap.
    pub const XXS: f64 = 2.0;
    /// Between tightly related items: chips in a row, keycaps in a combo.
    pub const XS: f64 = 4.0;
    /// Between controls in a row; table cell vertical padding.
    pub const S: f64 = 8.0;
    /// Between fields in a form grid; card padding on narrow layouts.
    pub const M: f64 = 12.0;
    /// Card padding; between blocks inside a section.
    pub const L: f64 = 16.0;
    /// Between sections of a page; page padding on wide layouts.
    pub const XL: f64 = 24.0;
}

/// Font sizes — WinUI's type ramp.
pub mod font {
    /// Field labels, column headers, hints, secondary metadata.
    pub const CAPTION: f64 = 12.0;
    /// Everything else, matching the text inside native controls.
    pub const BODY: f64 = 14.0;
    /// The big key combination readout in the key picker.
    pub const BODY_LARGE: f64 = 18.0;
    /// A page title.
    pub const SUBTITLE: f64 = 20.0;
}

/// Corner radii — WinUI's own control and overlay radii.
pub mod radius {
    /// Small things: keycaps, swatch chips, inner borders.
    pub const CONTROL: f64 = 4.0;
    /// Cards, panels, the preview stage.
    pub const CARD: f64 = 8.0;
}

/// Opacities standing in for secondary/tertiary text colour.
/// `windows-reactor`'s [`windows_reactor::ThemeBrush`] has no
/// secondary-text brush, and a hand-picked gray would be wrong in one of the
/// two themes; dimming the primary text colour is correct in both.
pub mod emphasis {
    /// Labels, ledes, column headers.
    pub const MUTED: f64 = 0.72;
    /// Placeholders ("—"), hex codes, disabled-looking-but-not-disabled text.
    pub const SUBTLE: f64 = 0.5;
    /// A whole card whose feature is switched off.
    pub const OFF: f64 = 0.6;
}

/// Fixed sizes of things that are not content-sized.
pub mod size {
    /// Width to reserve for one [`crate::ui::controls::icon_button`]: its
    /// 20px glyph plus the `Button`'s native 11px side padding, rounded up.
    /// A reservation, not a size the button is forced to — forcing 32px is
    /// what cropped the icons.
    pub const ICON_BUTTON: f64 = 44.0;
    /// The colour bar of a palette swatch.
    pub const SWATCH_HEIGHT: f64 = 24.0;
    /// Border thickness of cards, table rules and keycaps.
    pub const STROKE: f64 = 1.0;
}
