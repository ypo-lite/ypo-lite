//! The Skin screen: the five themes as cards, the overrides on top of the
//! chosen one, and a preview drawn by y-pop's own menu painter
//! (`skin_preview`).
//!
//! Overrides read "theme: #…" until set — an unset one is inherited, and the
//! theme supplies it each time y-pop opens a menu. Border and the two
//! gradients are written whole or not at all, so each sits behind a switch.

use y_core::menu::MenuDesign;
use y_core::overlay::Color as CoreColor;
use y_pop::settings::Settings;
use y_pop::theme::{EdgeRule, Theme, ThemeOverrides};

use crate::skin::{
    self, ColorSlot, DIRECTIONS, Editing, MAX_BORDER, MAX_GAP, MAX_ROUND, Part, SkinMsg, SkinUi,
    THEMES,
};
use crate::skin_preview::Picture;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;

/// Narrowest the overrides are comfortable at, and the narrowest the preview
/// (the sample menu with its submenu beside it is about this wide) is.
const OVERRIDES_MIN: f64 = 380.0;
const PREVIEW_MIN: f64 = 300.0;

pub fn skin_view(
    settings: &Settings,
    ui: &SkinUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let theme = skin::theme_of(settings);
    let overrides = &settings.skin.overrides;

    let split = Split::new(viewport)
        .min_widths(OVERRIDES_MIN, PREVIEW_MIN)
        .weights(1.3, 1.0);
    let overrides_pane = overrides_pane(theme, overrides, ui, split.primary(), context);
    let preview_pane = preview_pane(theme, overrides, ui, split.secondary());
    // Stacked, the picture goes first: it is what the controls change.
    let panes = if split.is_side_by_side() {
        split.build(overrides_pane, preview_pane)
    } else {
        split.build(preview_pane, overrides_pane)
    };

    page(
        "Skin",
        "The theme of the popup menus, and what you override on top of it.",
        Stack::vertical(space::XL)
            .child("themes", theme_cards(theme, ui, viewport, context))
            .child("panes", panes),
    )
}

// ---------------------------------------------------------------- themes ---

fn theme_cards(
    chosen: Theme,
    ui: &SkinUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let grid = AutoGrid::new(150.0).max_columns(5).gap(space::S, space::S);
    let inner = grid.item_viewport(viewport, THEMES.len()).width - 2.0 * CARD_INSET;
    let cards = THEMES.map(|theme| {
        let picture = ui.previews.borrow_mut().card(theme);
        (
            skin::theme_title(theme),
            theme_card(theme, theme == chosen, picture, inner, context),
        )
    });
    section("Theme", grid.items(cards).build(viewport))
}

fn theme_card(
    theme: Theme,
    selected: bool,
    picture: Option<Picture>,
    width: f64,
    context: &ViewContext<ConfigApp>,
) -> View {
    let about = match theme {
        Theme::Native => "Windows draws the menu".to_owned(),
        Theme::Default => "Light, blue hover".to_owned(),
        Theme::Flat => "Light, flat hover".to_owned(),
        Theme::Dark => "Dark, blue hover".to_owned(),
        Theme::Auto => format!(
            "Follows Windows — now {}",
            skin::theme_title(skin::auto_theme())
        ),
    };
    let mut body = Stack::vertical(space::XS);
    if let Some(picture) = &picture {
        body = body.child("picture", fitted(picture, width));
    }
    let body = body
        .child("title", text::strong(skin::theme_title(theme)))
        .child("about", text::caption(about).opacity(emphasis::MUTED));

    Button::new()
        .style(if selected {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Default
        })
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .on_click(context.callback(move |()| Msg::Skin(SkinMsg::Theme(theme))))
        .content(body)
        .tooltip(skin::theme_title(theme))
}

/// `picture`, shrunk to `width` if it is wider (never enlarged: that blurs).
fn fitted(picture: &Picture, width: f64) -> View {
    let scale = (width / picture.width).clamp(0.0, 1.0);
    Image::new()
        .source_data(picture.image.clone())
        .stretch(Stretch::Uniform)
        .width(picture.width * scale)
        .height(picture.height * scale)
        .horizontal_alignment(HorizontalAlignment::Left)
        .into()
}

// --------------------------------------------------------------- preview ---

fn preview_pane(theme: Theme, overrides: &ThemeOverrides, ui: &SkinUi, viewport: Viewport) -> View {
    let picture = ui.previews.borrow_mut().main(theme, *overrides);
    let body: View = match picture {
        Some(picture) => fitted(&picture, viewport.inset(CARD_INSET).width),
        None => text::caption(
            "A Native menu is drawn by Windows itself, so there is nothing to preview. \
             Choose a theme, or set an override, to see how y-pop draws it.",
        )
        .into(),
    };
    panel("Preview", None, body)
}

// ------------------------------------------------------------- overrides ---

fn overrides_pane(
    theme: Theme,
    overrides: &ThemeOverrides,
    ui: &SkinUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let base = skin::base_design(theme);
    let env = Env {
        overrides,
        base: &base,
        ui,
        viewport,
        context,
    };

    Stack::vertical(space::L)
        .child("readout", readout(theme, overrides, context))
        .child("colours", colours(&env))
        .child("spacing", spacing(&env))
        .child("edge", edge(&env))
        .child("border", border(&env))
        .child(
            "background",
            gradient(
                &env,
                Part::Background,
                "Background gradient",
                "Fills the hovered row.",
                [ColorSlot::BackgroundStart, ColorSlot::BackgroundEnd],
            ),
        )
        .child(
            "icon",
            gradient(
                &env,
                Part::Icon,
                "Icon column gradient",
                "Fills the icon column of the rows that are not hovered.",
                [ColorSlot::IconStart, ColorSlot::IconEnd],
            ),
        )
        .into()
}

/// What every part of the overrides pane reads.
struct Env<'a> {
    overrides: &'a ThemeOverrides,
    base: &'a MenuDesign,
    ui: &'a SkinUi,
    viewport: Viewport,
    context: &'a ViewContext<ConfigApp>,
}

/// "N overrides on top of Dark" and the way to drop them all.
fn readout(theme: Theme, overrides: &ThemeOverrides, context: &ViewContext<ConfigApp>) -> View {
    let count = skin::count(overrides);
    let drawn = skin::effective_theme(theme, overrides);
    let name = if drawn == Theme::Auto {
        format!("Auto ({})", skin::theme_title(skin::auto_theme()))
    } else {
        skin::theme_title(drawn).to_owned()
    };
    let summary = match count {
        0 => format!("No overrides: the menu is the {name} theme as it is."),
        1 => format!("1 override on top of {name}."),
        n => format!("{n} overrides on top of {name}."),
    };

    let mut body = Stack::vertical(space::S).child(
        "summary",
        Stack::horizontal(space::M)
            .child(
                "count",
                text::body(summary).vertical_alignment(VerticalAlignment::Center),
            )
            .child(
                "reset",
                button(
                    "Reset all",
                    false,
                    count > 0,
                    context.callback(|()| Msg::Skin(SkinMsg::ResetAll)),
                ),
            ),
    );
    // Mirrors the loader: an override has to be painted onto something.
    if theme == Theme::Native {
        body = body.child(
            "native",
            text::caption(
                "Native menus are drawn by Windows and cannot be restyled. Setting any \
                 override below makes y-pop draw its own menu, as the Auto theme.",
            )
            .opacity(emphasis::MUTED),
        );
    }
    section("Overrides", body)
}

fn hex(color: CoreColor) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

fn to_ui_color(color: CoreColor) -> Color {
    Color::argb(color.a, color.r, color.g, color.b)
}

fn colours(env: &Env<'_>) -> View {
    let swatches = AutoGrid::new(100.0)
        .max_columns(5)
        .gap(space::S, space::S)
        .items(ColorSlot::PLAIN.map(|slot| (slot.title(), swatch(slot, env))));
    let editor = colour_editor(env, |slot| slot.part().is_none());
    section(
        "Colours",
        Stack::vertical(space::M)
            .child(
                "help",
                text::caption("Until you pick one, a colour comes from the theme.")
                    .opacity(emphasis::MUTED),
            )
            .child("swatches", swatches.build(env.viewport))
            .maybe("editor", editor),
    )
}

/// One swatch: title, chip, hex — a button that opens the editor, with a way
/// back to the theme's colour once it is set (a colour inside a gradient or
/// the border cannot be unset on its own: it goes with its switch).
fn swatch(slot: ColorSlot, env: &Env<'_>) -> View {
    let set = slot.get(env.overrides);
    let color = slot.effective(env.overrides, env.base);
    let label = match set {
        Some(color) => hex(color),
        None => format!("theme: {}", hex(color)),
    };
    let open = env.ui.editing.is_some_and(|editing| editing.slot == slot);

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
            text::caption(slot.title())
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
    let button: View = Button::new()
        .style(if open {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Default
        })
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .on_click(
            env.context
                .callback(move |()| Msg::Skin(SkinMsg::Open(slot))),
        )
        .content(body)
        .tooltip(slot.title());

    let reset = (set.is_some() && slot.part().is_none()).then(|| {
        icon_button(
            Symbol::Undo,
            "Back to the theme's colour",
            env.context
                .callback(move |()| Msg::Skin(SkinMsg::ResetColor(slot))),
        )
    });
    Stack::vertical(space::XS)
        .child("swatch", button)
        .maybe("reset", reset)
        .into()
}

/// The colour picker, inline, when the colour being edited is one of the
/// `group`. Cancel puts the colour back; Done keeps it.
fn colour_editor(env: &Env<'_>, group: impl Fn(ColorSlot) -> bool) -> Option<View> {
    let Editing { slot, .. } = env.ui.editing.filter(|editing| group(editing.slot))?;
    let picker: View = ColorPicker::new()
        .color(to_ui_color(slot.effective(env.overrides, env.base)))
        // Menus are painted with GDI, which has no alpha.
        .is_alpha_enabled(false)
        .on_color_changed(env.context.callback(|color: Color| {
            Msg::Skin(SkinMsg::Color(CoreColor::rgb(color.r, color.g, color.b)))
        }))
        .into();
    Some(editor(
        format!("{} colour", full_title(slot)),
        picker,
        env.context
            .callback(|()| Msg::Skin(SkinMsg::Close { revert: true })),
        env.context
            .callback(|()| Msg::Skin(SkinMsg::Close { revert: false })),
    ))
}

fn full_title(slot: ColorSlot) -> String {
    match slot {
        ColorSlot::BorderColor => "Border".to_owned(),
        ColorSlot::BackgroundStart => "Background gradient start".to_owned(),
        ColorSlot::BackgroundEnd => "Background gradient end".to_owned(),
        ColorSlot::IconStart => "Icon gradient start".to_owned(),
        ColorSlot::IconEnd => "Icon gradient end".to_owned(),
        plain => plain.title().to_owned(),
    }
}

fn spacing(env: &Env<'_>) -> View {
    let gap = |key: &'static str,
               title: &str,
               value: Option<u32>,
               theme: u32,
               message: fn(Option<f64>) -> SkinMsg|
     -> (&'static str, View) {
        (
            key,
            field(
                title,
                Stack::vertical(space::XS)
                    .child(
                        "box",
                        number_box(
                            value.map(f64::from),
                            0.0,
                            MAX_GAP,
                            env.context.callback(move |value| Msg::Skin(message(value))),
                        ),
                    )
                    .child(
                        "hint",
                        text::caption(format!("Empty: the theme's ({theme} px)"))
                            .opacity(emphasis::MUTED),
                    ),
            ),
        )
    };
    let grid = AutoGrid::new(150.0)
        .max_columns(2)
        .gap(space::S, space::S)
        .items([
            gap(
                "padding",
                "Space between rows",
                env.overrides.padding,
                env.base.padding,
                SkinMsg::Padding,
            ),
            gap(
                "margin",
                "Space inside a row",
                env.overrides.margin,
                env.base.margin,
                SkinMsg::Margin,
            ),
        ]);
    section("Spacing", grid.build(env.viewport))
}

fn edge(env: &Env<'_>) -> View {
    let describe = |edge: Option<y_core::menu::EdgeRule>| match edge {
        None => "none",
        Some(y_core::menu::EdgeRule::Idle) => "on rows that are not hovered",
        Some(y_core::menu::EdgeRule::Hover) => "on the hovered row",
        Some(y_core::menu::EdgeRule::Always) => "on every row",
    };
    let selected = match env.overrides.edge {
        None => 0,
        Some(EdgeRule::Idle) => 1,
        Some(EdgeRule::Hover) => 2,
        Some(EdgeRule::Always) => 3,
    };
    section(
        "Edge rule",
        Stack::vertical(space::S)
            .child(
                "help",
                text::caption(format!(
                    "A vertical rule between the icons and the text. The theme's: {}.",
                    describe(env.base.edge)
                ))
                .opacity(emphasis::MUTED),
            )
            .child(
                "choice",
                combo(
                    [
                        "Use the theme's",
                        "On rows that are not hovered",
                        "On the hovered row",
                        "On every row",
                    ],
                    Some(selected),
                    env.context
                        .callback(|choice| Msg::Skin(SkinMsg::Edge(choice))),
                ),
            ),
    )
}

/// A switch for an override that is written whole, and `body` under it when on.
fn part_section(
    env: &Env<'_>,
    part: Part,
    title: &str,
    help: &str,
    on: bool,
    body: Option<View>,
) -> View {
    section(
        title,
        Stack::vertical(space::M)
            .child("help", text::caption(help).opacity(emphasis::MUTED))
            .child(
                "switch",
                labeled_switch(
                    "Override",
                    on,
                    env.context
                        .callback(move |on| Msg::Skin(SkinMsg::PartOn(part, on))),
                ),
            )
            .maybe("body", body),
    )
}

fn border(env: &Env<'_>) -> View {
    let body = env.overrides.border.map(|border| {
        let numbers = AutoGrid::new(110.0)
            .max_columns(3)
            .gap(space::S, space::S)
            .item(
                "size",
                field(
                    "Width (px)",
                    number_box(
                        Some(f64::from(border.size)),
                        0.0,
                        MAX_BORDER,
                        env.context
                            .callback(|value| Msg::Skin(SkinMsg::BorderSize(value))),
                    ),
                ),
            )
            .item(
                "round",
                field(
                    "Corner radius (px)",
                    number_box(
                        Some(f64::from(border.round)),
                        0.0,
                        MAX_ROUND,
                        env.context
                            .callback(|value| Msg::Skin(SkinMsg::BorderRound(value))),
                    ),
                ),
            )
            .item("color", swatch(ColorSlot::BorderColor, env));
        Stack::vertical(space::M)
            .child("numbers", numbers.build(env.viewport))
            .maybe(
                "editor",
                colour_editor(env, |slot| slot.part() == Some(Part::Border)),
            )
            .into()
    });
    part_section(
        env,
        Part::Border,
        "Border",
        "A ring around the hovered row, with optionally rounded corners.",
        env.overrides.border.is_some(),
        body,
    )
}

fn gradient(
    env: &Env<'_>,
    part: Part,
    title: &str,
    help: &str,
    [start, end]: [ColorSlot; 2],
) -> View {
    let current = match part {
        Part::Background => env.overrides.background_gradient,
        _ => env.overrides.icon_gradient,
    };
    let body = current.map(|gradient| {
        let direction = field(
            "Direction",
            segmented(
                &[Choice::new("Horizontal"), Choice::new("Vertical")],
                DIRECTIONS
                    .iter()
                    .position(|&direction| direction == gradient.direction),
                env.context
                    .callback(move |index| Msg::Skin(SkinMsg::Direction(part, index))),
            ),
        );
        let row = AutoGrid::new(110.0)
            .max_columns(3)
            .gap(space::S, space::S)
            .item("start", swatch(start, env))
            .item("end", swatch(end, env))
            .item("direction", direction);
        Stack::vertical(space::M)
            .child("row", row.build(env.viewport))
            .maybe(
                "editor",
                colour_editor(env, move |slot| slot.part() == Some(part)),
            )
            .into()
    });
    part_section(env, part, title, help, current.is_some(), body)
}
