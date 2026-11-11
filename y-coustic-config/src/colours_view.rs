//! The Colours screen: five swatches for everything and for each kind of volume,
//! a colour picker under the set being edited, and a preview of the three kinds
//! drawn by y-coustic's own painter. What is shown for a colour nobody set is
//! what y-coustic would draw (see [`crate::colours`]).

use windows_reactor::*;
use y_core::overlay::Color as CoreColor;
use y_coustic::config::Settings;
use y_coustic::model::Kind;

use crate::colours::{self, Addr, ColourMsg, ColoursUi, Set, Slot};
use crate::hud_view::{HudUi, fitted};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};

/// What the preview shows: each kind the colours can reach.
const SAMPLES: [(&str, Kind, f32, bool); 4] = [
    ("Master volume", Kind::Master, 0.6, false),
    ("Microphone, live", Kind::Mic, 0.7, false),
    ("Microphone, muted", Kind::Mic, 0.7, true),
    ("Application", Kind::App, 0.5, false),
];

fn to_ui(color: CoreColor) -> Color {
    Color::argb(255, color.r, color.g, color.b)
}

fn hex(color: CoreColor) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

pub fn colours_view(
    settings: &Settings,
    colours_ui: &ColoursUi,
    hud_ui: &HudUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let split = Split::new(viewport).min_widths(440.0, 300.0).weights(1.5, 1.0);
    let sets = sets(settings, colours_ui, hud_ui.dark, split.primary(), context);
    let preview = preview_pane(settings, hud_ui, split.secondary(), context);

    // Stacked, the preview comes first: a colour is judged by looking at it.
    let body: View = if split.is_side_by_side() {
        split.build(sets, preview)
    } else {
        Stack::vertical(space::XL)
            .child("preview", preview)
            .child("sets", sets)
            .into()
    };
    page(
        "Colours",
        "The colours of the HUD and of the mixer, for everything and for each kind of volume.",
        body,
    )
}

fn sets(
    settings: &Settings,
    ui: &ColoursUi,
    dark: bool,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let colors = &settings.hud.colors;
    let mut body = Stack::vertical(space::L).child(
        "about",
        note(
            "A colour you leave alone follows the Windows theme, light or dark. A set for one kind wins over the set \
             for everything.",
        ),
    );
    for set in Set::ALL {
        let reset = (colours::count(colors, set) > 0).then(|| {
            button(
                "Reset all",
                false,
                true,
                context.callback(move |()| Msg::Colour(ColourMsg::ResetSet(set))),
            )
        });
        let swatches = AutoGrid::new(100.0)
            .max_columns(5)
            .gap(space::S, space::S)
            .items(Slot::ALL.map(|slot| {
                let addr = Addr { set, slot };
                (slot.title(), swatch(settings, ui, addr, dark, context))
            }))
            .build(viewport.inset(CARD_INSET));

        let mut inside = Stack::vertical(space::M);
        if let Some(line) = explanation(set) {
            inside = inside.child("about", text::caption(line));
        }
        inside = inside.child("swatches", swatches);
        if let Some(picker) = picker(settings, ui, set, dark, context) {
            inside = inside.child("picker", picker);
        }
        body = body.child(set.title(), panel(set.title(), reset, inside));
    }
    body.into()
}

/// What a set is for, where that is not obvious.
fn explanation(set: Set) -> Option<&'static str> {
    match set {
        Set::All => Some(
            "Reaches the master volume and applications. Not the microphone's live bar: its accent is its own.",
        ),
        Set::Microphone => Some(
            "The bar is red while the microphone is live unless you choose an accent here: taking a live microphone \
             for the master volume is the mistake that costs.",
        ),
        Set::Master | Set::Application => None,
    }
}

/// One colour: its swatch, what it is, and a way back to what it inherits.
fn swatch(
    settings: &Settings,
    ui: &ColoursUi,
    addr: Addr,
    dark: bool,
    context: &ViewContext<ConfigApp>,
) -> View {
    let colors = &settings.hud.colors;
    let set = colours::get(colors, addr);
    let shown = colours::effective(colors, addr, dark);
    let label = match set {
        Some(color) => hex(color),
        None => format!("inherited: {}", hex(shown)),
    };
    let open = ui.editing.is_some_and(|editing| editing.addr == addr);

    let chip: View = Border::new()
        .height(size::SWATCH_HEIGHT)
        .corner_radius(CornerRadius::uniform(radius::CONTROL))
        .border_thickness(Thickness::uniform(size::STROKE))
        .border_brush(ThemeBrush::CardStroke)
        .background(to_ui(shown))
        .into();
    let body = Stack::vertical(space::XS)
        .child(
            "title",
            text::caption(addr.slot.title())
                .text_wrapping(TextWrapping::NoWrap)
                .text_trimming(TextTrimming::CharacterEllipsis),
        )
        .child("chip", chip)
        .child(
            "hex",
            text::caption(label)
                .text_wrapping(TextWrapping::NoWrap)
                .text_trimming(TextTrimming::CharacterEllipsis)
                .opacity(emphasis::SUBTLE),
        );
    let open_button: View = Button::new()
        .style(if open { ButtonStyle::Accent } else { ButtonStyle::Default })
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .on_click(context.callback(move |()| Msg::Colour(ColourMsg::Open(addr))))
        .content(body)
        .tooltip(addr.slot.title());

    let reset = set.is_some().then(|| {
        icon_button(
            Symbol::Undo,
            "Back to what it inherits",
            context.callback(move |()| Msg::Colour(ColourMsg::Reset(addr))),
        )
    });
    Stack::vertical(space::XS)
        .child("swatch", open_button)
        .maybe("reset", reset)
        .into()
}

/// The colour picker, inline, when the colour being edited is one of `set`'s.
/// Cancel puts the colour back; Done keeps it.
fn picker(
    settings: &Settings,
    ui: &ColoursUi,
    set: Set,
    dark: bool,
    context: &ViewContext<ConfigApp>,
) -> Option<View> {
    let editing = ui.editing.filter(|editing| editing.addr.set == set)?;
    let current = colours::effective(&settings.hud.colors, editing.addr, dark);
    let picker: View = ColorPicker::new()
        .color(to_ui(current))
        // The HUD's surface is drawn with GDI, which has no alpha to spare.
        .is_alpha_enabled(false)
        .on_color_changed(context.callback(|color: Color| {
            Msg::Colour(ColourMsg::Color(CoreColor::rgb(color.r, color.g, color.b)))
        }))
        .into();
    Some(editor(
        format!("{}: {}", set.title(), editing.addr.slot.title()),
        picker,
        context.callback(|()| Msg::Colour(ColourMsg::Close { revert: true })),
        context.callback(|()| Msg::Colour(ColourMsg::Close { revert: false })),
    ))
}

fn preview_pane(
    settings: &Settings,
    ui: &HudUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let width = viewport.inset(CARD_INSET).width;
    let mut body = Stack::vertical(space::L);
    for (index, &(title, kind, level, muted)) in SAMPLES.iter().enumerate() {
        let mut sample = Stack::vertical(space::XS).child("title", text::caption(title));
        match ui.picture(&settings.hud, kind, Some((level, muted)), ui.dark) {
            Some(picture) => sample = sample.child("picture", fitted(&picture, width)),
            None => sample = sample.child("failed", text::caption("The preview could not be drawn.")),
        }
        body = body.child(index, sample);
    }
    panel(
        "Preview",
        Some(labeled_switch(
            "Dark",
            ui.dark,
            context.callback(|on: bool| Msg::HudDark(on)),
        )),
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hex_of_a_colour_reads_back_through_the_library() {
        let color = CoreColor::rgb(0x3F, 0xB9, 0x50);
        assert_eq!(hex(color), "#3FB950");
        assert_eq!(CoreColor::parse(&hex(color)), Some(color));
    }

    #[test]
    fn the_preview_covers_every_kind_the_colours_can_reach() {
        for kind in [Kind::Master, Kind::Mic, Kind::App] {
            assert!(SAMPLES.iter().any(|(_, known, _, _)| *known == kind), "{kind:?}");
        }
        assert!(SAMPLES.iter().any(|(_, kind, _, muted)| *kind == Kind::Mic && *muted));
    }
}
