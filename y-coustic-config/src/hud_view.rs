//! The HUD screen: its style as five pictures, where it appears, how it looks and
//! how long it stays, with a preview drawn by y-coustic's own painter
//! (`y_coustic::paint`), so what is shown here is what y-coustic draws.
//!
//! Every control reports an [`Edit`], except the font, which is text.

use std::cell::RefCell;

use windows_reactor::*;
use y_coustic::config::{
    BORDER_RANGE, FADE_RANGE, HudConfig, OPACITY_RANGE, Position, RADIUS_RANGE, SCALE_RANGE,
    Settings, Style, TIMEOUT_RANGE,
};
use y_coustic::model::{Kind, State};
use y_coustic::paint::layout::design_size;

use crate::edit::Edit;
use crate::simple::number;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};

/// The dpi the preview is drawn at: twice the layout's own, so it stays sharp on
/// a scaled display. It is shown at half its pixels.
const PREVIEW_DPI: u32 = 192;

/// How many pictures the preview keeps before it starts again.
const CACHE_LIMIT: usize = 48;

const STYLES: [(Style, &str); 5] = [
    (Style::Bar, "Bar"),
    (Style::Compact, "Compact"),
    (Style::Text, "Text"),
    (Style::VBar, "Tall bar"),
    (Style::Ring, "Ring"),
];

const POSITIONS: [[(Position, &str); 3]; 3] = [
    [
        (Position::TopLeft, "\u{2196}"),
        (Position::TopCenter, "\u{2191}"),
        (Position::TopRight, "\u{2197}"),
    ],
    [
        (Position::CenterLeft, "\u{2190}"),
        (Position::Center, "\u{2022}"),
        (Position::CenterRight, "\u{2192}"),
    ],
    [
        (Position::BottomLeft, "\u{2199}"),
        (Position::BottomCenter, "\u{2193}"),
        (Position::BottomRight, "\u{2198}"),
    ],
];

/// What the preview shows: a volume of each kind the HUD can show.
/// A volume and whether it is muted; `None` for no device.
type Sample = Option<(f32, bool)>;

const SAMPLES: [(&str, Kind, Sample); 4] = [
    ("Master volume", Kind::Master, Some((0.6, false))),
    ("Microphone, live", Kind::Mic, Some((0.7, false))),
    ("Microphone, muted", Kind::Mic, Some((0.7, true))),
    ("A device that is not there", Kind::Mic, None),
];

/// A picture and the size to show it at, in layout units.
#[derive(Clone)]
pub(crate) struct Picture {
    image: EncodedImage,
    width: f64,
    height: f64,
}

#[derive(Clone, PartialEq)]
struct Key {
    hud: HudConfig,
    kind: Kind,
    /// Percent, and whether muted: `None` for no device.
    state: Option<(u8, bool)>,
    dark: bool,
}

/// What the HUD screen remembers besides the settings: the theme of the preview,
/// and the pictures already drawn.
pub struct HudUi {
    pub dark: bool,
    pictures: RefCell<Vec<(Key, Option<Picture>)>>,
}

impl Default for HudUi {
    fn default() -> Self {
        Self {
            dark: y_core::theme::is_dark(),
            pictures: RefCell::new(Vec::new()),
        }
    }
}

impl HudUi {
    /// The picture of `kind` in the look of `hud`, drawn once for as long as
    /// the settings that decide it stay as they are. Only what changes the
    /// picture is part of the key: not where it appears, nor how long it stays.
    pub(crate) fn picture(
        &self,
        hud: &HudConfig,
        kind: Kind,
        state: Option<(f32, bool)>,
        dark: bool,
    ) -> Option<Picture> {
        let mut looks = hud.clone();
        looks.interactive = false;
        looks.position = Position::BottomCenter;
        looks.timeout = *TIMEOUT_RANGE.start();
        looks.fade = 0;
        let key = Key {
            hud: looks,
            kind,
            state: state.map(|(level, muted)| ((level * 100.0).round() as u8, muted)),
            dark,
        };
        if let Some((_, picture)) = self.pictures.borrow().iter().find(|(known, _)| *known == key) {
            return picture.clone();
        }

        let shown = state.map(|(level, muted)| State { level, muted });
        let png = y_coustic::paint::render_png(&key.hud, kind, shown, None, dark, PREVIEW_DPI);
        let (width, height) = design_size(hud.style);
        let scale = f64::from(hud.scale) / 100.0;
        let picture = png.map(|bytes| Picture {
            image: EncodedImage::new(bytes),
            width: f64::from(width) * scale,
            height: f64::from(height) * scale,
        });
        let mut pictures = self.pictures.borrow_mut();
        if pictures.len() >= CACHE_LIMIT {
            pictures.clear();
        }
        pictures.push((key, picture.clone()));
        picture
    }
}

/// `picture`, shrunk to `width` if it is wider (never enlarged: that blurs).
pub(crate) fn fitted(picture: &Picture, width: f64) -> View {
    let scale = (width / picture.width).clamp(0.0, 1.0);
    Image::new()
        .source_data(picture.image.clone())
        .stretch(Stretch::Uniform)
        .width(picture.width * scale)
        .height(picture.height * scale)
        .horizontal_alignment(HorizontalAlignment::Left)
        .into()
}

pub fn hud_view(
    settings: &Settings,
    ui: &HudUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let split = Split::new(viewport).min_widths(440.0, 300.0).weights(1.5, 1.0);
    let controls = controls(settings, ui, split.primary(), context);
    let preview = preview_pane(settings, ui, split.secondary(), context);

    // Side by side the preview stays beside the fields; stacked, it comes first,
    // so a change is seen without scrolling past all of them.
    let body: View = if split.is_side_by_side() {
        split.build(controls, preview)
    } else {
        Stack::vertical(space::XL)
            .child("preview", preview)
            .child("controls", controls)
            .into()
    };
    page(
        "HUD",
        "The small window that shows the level after a change, and when the pointer rests on the tray icon.",
        body,
    )
}

fn controls(
    settings: &Settings,
    ui: &HudUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let hud = &settings.hud;
    Stack::vertical(space::XL)
        .child("style", section("Style", style_cards(hud, ui, viewport, context)))
        .child("position", section("Where it appears", position_grid(hud, context)))
        .child("look", section("Size and look", look(hud, viewport, context)))
        .child("timing", section("How long it stays", timing(hud, context)))
        .child("using", section("Using it", using(hud, viewport, context)))
        .into()
}

fn style_cards(
    hud: &HudConfig,
    ui: &HudUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let grid = AutoGrid::new(150.0).max_columns(5);
    let card_viewport = grid.item_viewport(viewport, STYLES.len());
    let inner = card_viewport.inset(CARD_INSET).width;
    grid.items(STYLES.iter().map(|&(style, title)| {
        let mut shown = hud.clone();
        shown.style = style;
        let picture = ui.picture(&shown, Kind::Master, Some((0.6, false)), ui.dark);
        let mut body = Stack::vertical(space::S);
        if let Some(picture) = &picture {
            body = body.child("picture", fitted(picture, inner));
        }
        body = body.child("title", text::caption(title));
        let card: View = Button::new()
            .style(if hud.style == style {
                ButtonStyle::Accent
            } else {
                ButtonStyle::Default
            })
            .fill()
            .horizontal_content_alignment(HorizontalAlignment::Stretch)
            .on_click(context.callback(move |()| Msg::Edit(Edit::HudStyle(style))))
            .content(body)
            .tooltip(title);
        (style.name(), card)
    }))
    .build(viewport)
}

fn position_grid(hud: &HudConfig, context: &ViewContext<ConfigApp>) -> View {
    let rows = POSITIONS.iter().enumerate().map(|(row, places)| {
        let buttons = Stack::horizontal(space::XS).children(places.iter().map(|&(position, glyph)| {
            let button: View = Button::new()
                .style(if hud.position == position {
                    ButtonStyle::Accent
                } else {
                    ButtonStyle::Default
                })
                .min_width(0.0)
                .width(size::ICON_BUTTON)
                .on_click(context.callback(move |()| Msg::Edit(Edit::HudPosition(position))))
                .content(glyph)
                .tooltip(position.name());
            (position.name(), button)
        }));
        (format!("row{row}"), View::from(buttons))
    });
    Stack::vertical(space::M)
        .child(
            "about",
            text::caption(
                "A place on the screen under the pointer, at the work area's edge, or next to the pointer itself.",
            ),
        )
        .child(
            "places",
            Stack::horizontal(space::XL)
                .child("grid", Stack::vertical(space::XS).children(rows))
                .child(
                    "follow",
                    Border::new()
                        .vertical_alignment(VerticalAlignment::Center)
                        .content(button(
                            "Follow the pointer",
                            hud.position == Position::FollowCursor,
                            true,
                            context.callback(|()| Msg::Edit(Edit::HudPosition(Position::FollowCursor))),
                        )),
                ),
        )
        .into()
}

/// A slider with its number box, in a card.
fn slider(
    title: &str,
    description: &str,
    value: u32,
    range: &std::ops::RangeInclusive<u32>,
    step: f64,
    make: fn(u32) -> Edit,
    context: &ViewContext<ConfigApp>,
) -> View {
    let (slide, type_in) = number(context, make);
    setting_wide(
        title,
        description,
        range_input(
            f64::from(value),
            f64::from(*range.start()),
            f64::from(*range.end()),
            step,
            slide,
            type_in,
        ),
    )
}

fn look(hud: &HudConfig, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let font = setting(
        "Font",
        "The font family of the text. A name Windows does not have falls back to its default font.",
        text_box(
            hud.font.clone(),
            "Segoe UI",
            context.callback(Msg::HudFont),
        ),
        viewport,
    );
    Stack::vertical(space::M)
        .child(
            "scale",
            slider(
                "Size",
                "In percent of the design size; the screen's own scaling applies on top.",
                hud.scale,
                &SCALE_RANGE,
                5.0,
                Edit::HudScale,
                context,
            ),
        )
        .child(
            "radius",
            slider("Corner radius", "In design pixels.", hud.radius, &RADIUS_RANGE, 1.0, Edit::HudRadius, context),
        )
        .child(
            "border",
            slider(
                "Border",
                "Its width in design pixels, in the colour of the empty part of the bar. 0 is none.",
                hud.border,
                &BORDER_RANGE,
                1.0,
                Edit::HudBorder,
                context,
            ),
        )
        .child(
            "opacity",
            slider(
                "Opacity",
                "How opaque the whole window is, in percent.",
                hud.opacity,
                &OPACITY_RANGE,
                5.0,
                Edit::HudOpacity,
                context,
            ),
        )
        .child("font", font)
        .into()
}

fn timing(hud: &HudConfig, context: &ViewContext<ConfigApp>) -> View {
    Stack::vertical(space::M)
        .child(
            "timeout",
            slider(
                "Stays",
                "Milliseconds after the last change before it goes.",
                hud.timeout,
                &TIMEOUT_RANGE,
                100.0,
                Edit::HudTimeout,
                context,
            ),
        )
        .child(
            "fade",
            slider(
                "Fade",
                "Milliseconds it takes to fade in and out. 0 shows and hides it at once.",
                hud.fade,
                &FADE_RANGE,
                10.0,
                Edit::HudFade,
                context,
            ),
        )
        .into()
}

fn using(hud: &HudConfig, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    Stack::vertical(space::M)
        .child(
            "interactive",
            setting(
                "Use it with the pointer",
                "The wheel, a click or a drag on its bar, a click on its icon to mute. Off, it is click-through.",
                switch(
                    hud.interactive,
                    context.callback(|on: bool| Msg::Edit(Edit::HudInteractive(on))),
                ),
                viewport,
            ),
        )
        .maybe(
            "note",
            (!hud.interactive).then(|| {
                note(
                    "While it cannot be used, resting the pointer on the tray icon shows no HUD either: a bar that \
                     cannot be used is not worth resting for.",
                )
            }),
        )
        .into()
}

fn preview_pane(
    settings: &Settings,
    ui: &HudUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let width = viewport.inset(CARD_INSET).width;
    let mut body = Stack::vertical(space::L);
    for (index, &(title, kind, state)) in SAMPLES.iter().enumerate() {
        let mut sample = Stack::vertical(space::XS).child("title", text::caption(title));
        match ui.picture(&settings.hud, kind, state, ui.dark) {
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
    fn every_style_and_position_the_file_can_hold_has_a_choice() {
        for name in ["bar", "vbar", "ring", "text", "compact"] {
            let style = Style::from_name(name).expect("a known style");
            assert!(STYLES.iter().any(|(known, _)| *known == style), "{name}");
        }
        let places: Vec<Position> = POSITIONS.iter().flatten().map(|&(position, _)| position).collect();
        assert_eq!(places.len(), 9);
        for name in [
            "top-left",
            "top-center",
            "top-right",
            "center-left",
            "center",
            "center-right",
            "bottom-left",
            "bottom-center",
            "bottom-right",
        ] {
            let position = Position::from_name(name).expect("a known position");
            assert!(places.contains(&position), "{name}");
        }
    }

    #[test]
    fn the_preview_draws_each_picture_once_and_again_only_when_it_would_look_different() {
        let ui = HudUi::default();
        let hud = HudConfig::default();
        let first = ui.picture(&hud, Kind::Master, Some((0.6, false)), true);
        assert!(first.is_some());
        assert_eq!(ui.pictures.borrow().len(), 1);

        // Where it appears, how long it stays and whether it takes the pointer
        // do not change the picture.
        let mut same = hud.clone();
        same.position = Position::TopLeft;
        same.timeout = 5000;
        same.fade = 300;
        same.interactive = false;
        ui.picture(&same, Kind::Master, Some((0.6, false)), true);
        assert_eq!(ui.pictures.borrow().len(), 1);

        // A look, a theme, a kind or a level does.
        let mut other = hud.clone();
        other.radius = 0;
        ui.picture(&other, Kind::Master, Some((0.6, false)), true);
        ui.picture(&hud, Kind::Master, Some((0.6, false)), false);
        ui.picture(&hud, Kind::Mic, Some((0.6, false)), true);
        ui.picture(&hud, Kind::Master, Some((0.2, false)), true);
        assert_eq!(ui.pictures.borrow().len(), 5);
    }

    #[test]
    fn a_picture_is_shown_at_its_design_size_times_the_scale() {
        let ui = HudUi::default();
        let hud = HudConfig {
            scale: 150,
            ..HudConfig::default()
        };
        let picture = ui.picture(&hud, Kind::Master, Some((0.5, false)), true).unwrap();
        let (width, height) = design_size(hud.style);
        assert_eq!(picture.width, f64::from(width) * 1.5);
        assert_eq!(picture.height, f64::from(height) * 1.5);
    }

    #[test]
    fn the_cache_does_not_grow_without_end() {
        let ui = HudUi::default();
        let hud = HudConfig::default();
        for level in 0..=100 {
            ui.picture(&hud, Kind::Master, Some((level as f32 / 100.0, false)), true);
        }
        assert!(ui.pictures.borrow().len() <= CACHE_LIMIT);
    }
}
