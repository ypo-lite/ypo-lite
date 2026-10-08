//! The Overlay screen's Palette tab: [`PaletteField::ALL`]'s ten swatches,
//! plus the colour editor one opens below them.

use crate::ui::prelude::*;
use crate::{ConfigApp, EditorClose, Msg};
use windows_reactor::*;
use y_core::overlay::Color as CoreColor;
use y_snap::config::{Overlay, Palette, Theme};

/// [`CoreColor`] (straight, as `y-snap.xml` writes it) as [`Color`]
/// (`windows_reactor`'s, for [`ColorPicker`] and brushes) — same four
/// channels, different field order.
pub fn to_ui_color(color: CoreColor) -> Color {
    Color::argb(color.a, color.r, color.g, color.b)
}

pub fn from_ui_color(color: Color) -> CoreColor {
    CoreColor::rgba(color.r, color.g, color.b, color.a)
}

/// One editable colour of [`Overlay::palette`] — one small `enum` naming
/// which field a click is about, rather than ten near-identical message
/// triples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteField {
    Back,
    Text,
    HoverBack,
    HoverText,
    ActiveBack,
    ActiveText,
    DisabledBack,
    DisabledText,
    Border,
    PanelBorder,
}

impl PaletteField {
    pub const ALL: [Self; 10] = [
        Self::Back,
        Self::Text,
        Self::HoverBack,
        Self::HoverText,
        Self::ActiveBack,
        Self::ActiveText,
        Self::DisabledBack,
        Self::DisabledText,
        Self::Border,
        Self::PanelBorder,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Back => "Back",
            Self::Text => "Text",
            Self::HoverBack => "Hover back",
            Self::HoverText => "Hover text",
            Self::ActiveBack => "Active back",
            Self::ActiveText => "Active text",
            Self::DisabledBack => "Disabled back",
            Self::DisabledText => "Disabled text",
            Self::Border => "Border",
            Self::PanelBorder => "Panel border",
        }
    }

    pub fn get(self, palette: &Palette) -> CoreColor {
        match self {
            Self::Back => palette.back,
            Self::Text => palette.text,
            Self::HoverBack => palette.hover_back,
            Self::HoverText => palette.hover_text,
            Self::ActiveBack => palette.active_back,
            Self::ActiveText => palette.active_text,
            Self::DisabledBack => palette.disabled_back,
            Self::DisabledText => palette.disabled_text,
            Self::Border => palette.border,
            Self::PanelBorder => palette.panel_border,
        }
    }

    pub fn get_mut(self, palette: &mut Palette) -> &mut CoreColor {
        match self {
            Self::Back => &mut palette.back,
            Self::Text => &mut palette.text,
            Self::HoverBack => &mut palette.hover_back,
            Self::HoverText => &mut palette.hover_text,
            Self::ActiveBack => &mut palette.active_back,
            Self::ActiveText => &mut palette.active_text,
            Self::DisabledBack => &mut palette.disabled_back,
            Self::DisabledText => &mut palette.disabled_text,
            Self::Border => &mut palette.border,
            Self::PanelBorder => &mut palette.panel_border,
        }
    }
}

/// Swatches pair up naturally (back/text per state), so the grid tops out
/// at two pairs-and-a-half per row — five — and drops to fewer as the pane
/// narrows, every swatch one width whatever its title.
pub fn palette_tab(
    overlay: &Overlay,
    editing: Option<PaletteField>,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let swatches = AutoGrid::new(96.0).max_columns(5).gap(space::S, space::S).items(
        PaletteField::ALL
            .map(|field| (field.title(), swatch(field, &overlay.palette, editing == Some(field), context))),
    );

    let editor = editing.map(|field| {
        let picker: View = ColorPicker::new()
            .color(to_ui_color(field.get(&overlay.palette)))
            .is_alpha_enabled(true)
            .on_color_changed(context.callback(Msg::OverlayPaletteColorChanged))
            .into();
        editor(
            format!("{} colour", field.title()),
            picker,
            context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
            context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
        )
    });

    Stack::vertical(space::L)
        .child("theme", theme_row(overlay, viewport, context))
        .child("swatches", swatches.build(viewport))
        .maybe("editor", editor)
        .into()
}

/// How each [`Theme`] reads in the dropdown, in [`Theme::ALL`] order.
fn theme_title(theme: Theme) -> &'static str {
    match theme {
        Theme::Auto => "Auto — follows Windows light/dark",
        Theme::Light => "Light",
        Theme::Dark => "Dark",
    }
}

/// The theme the colours start from, and how many of them differ from it.
/// Only the differing colours are written to `y-snap.xml`; the theme
/// supplies the rest each time y-snap loads — which is what keeps `auto`
/// following the system.
fn theme_row(overlay: &Overlay, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let selected = Theme::ALL.iter().position(|&theme| theme == overlay.theme);
    let theme = field(
        "Theme — switching resets the colours to it",
        combo(Theme::ALL.map(theme_title), selected, context.callback(Msg::OverlayThemeChanged)),
    );

    let customised = customised(overlay);
    let status: View = if customised == 0 {
        text::caption(format!("All colours come from the {} theme.", overlay.theme.name())).into()
    } else {
        Stack::vertical(space::XS)
            .child(
                "count",
                text::caption(format!(
                    "{customised} of {} colours differ from the {} theme.",
                    PaletteField::ALL.len(),
                    overlay.theme.name()
                )),
            )
            .child("reset", button("Reset to theme", false, true, context.callback(|()| Msg::OverlayPaletteReset)))
            .into()
    };

    AutoGrid::new(240.0).max_columns(2).item("theme", theme).item("status", status).build(viewport)
}

/// How many palette colours differ from what the theme gives.
fn customised(overlay: &Overlay) -> usize {
    PaletteField::ALL.iter().filter(|field| field.get(&overlay.palette) != field.get(&overlay.theme_palette)).count()
}

/// One swatch: title, colour chip, hex — a button that opens the editor.
/// The open one is drawn with the Accent style, so it's clear which colour
/// the editor below is changing.
fn swatch(field: PaletteField, palette: &Palette, open: bool, context: &ViewContext<ConfigApp>) -> View {
    let color = field.get(palette);
    let hex = format!("#{:02X}{:02X}{:02X}{:02X}", color.r, color.g, color.b, color.a);

    let chip: View = Border::new()
        .height(size::SWATCH_HEIGHT)
        .corner_radius(CornerRadius::uniform(radius::CONTROL))
        .border_thickness(Thickness::uniform(size::STROKE))
        .border_brush(ThemeBrush::CardStroke)
        .background(to_ui_color(color))
        .into();

    let body = Stack::vertical(space::XS)
        .child(
            "title",
            text::caption(field.title())
                .text_wrapping(TextWrapping::NoWrap)
                .text_trimming(TextTrimming::CharacterEllipsis),
        )
        .child("chip", chip)
        .child("hex", text::caption(hex).opacity(emphasis::SUBTLE));

    Button::new()
        .style(if open { ButtonStyle::Accent } else { ButtonStyle::Default })
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .on_click(context.callback(move |()| Msg::EditPalette(field)))
        .content(body)
        .tooltip(field.title())
}
