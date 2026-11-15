//! The Mouse finder screen: what is global (the full-screen guard, the
//! excluded applications), the default look, and a [`Table`] of named styles.
//! A style's row opens an inline editor with its look and its triggers
//! (`<tap>` and `<shake>`).
//!
//! Everything that changes a value is a pure function here ([`apply_look`],
//! [`apply_tap`], [`apply_shake`], [`add_style`], [`delete_style`]...) so that
//! it is tested without a window, and `main.rs` only routes messages to them.
//! Each one answers whether it changed anything: WinUI's `NumberBox` and
//! `ComboBox` report a value when first drawn, which must not count as an
//! edit.
//!
//! **Not here yet:** a Test button (y-snap has no way to be asked to play an
//! unsaved style), a sensitivity preset for shakes, picking a style on a
//! `find-mouse` hotkey or shortcut (the file keeps what it has), and a warning
//! for two triggers that clash (y-snap says so when it reads the file).

use std::ops::RangeInclusive;

use crate::overlay::palette::{from_ui_color, to_ui_color};
use crate::reorder::{list_footer, row_actions};
use crate::ui::key_picker::MODIFIERS;
use crate::ui::prelude::*;
use crate::{ConfigApp, EditorClose, ListKind, Msg};
use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
use windows_reactor::*;
use y_snap::config::{
    Finder, Look, Recall, Settings, Shake, ShakeShape, Shape, Side, Style, Tap, TapKey,
    finder_range, normalize_process,
};

// ---------------------------------------------------------------------------
// What a message says
// ---------------------------------------------------------------------------

/// Which look an edit is about: the default one, or one style's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LookTarget {
    Base,
    Style(usize),
}

/// One change to a [`Look`].
pub enum LookEdit {
    Shape(Option<usize>),
    Duration(Option<f64>),
    Size(Option<f64>),
    Dim(Option<f64>),
    Recall(Option<usize>),
    AutoColor(bool),
    AllMonitors(bool),
    Color(Color),
}

/// One change to a [`Tap`].
pub enum TapEdit {
    Key(Option<usize>),
    Side(Option<usize>),
    Count(Option<f64>),
    Hold(Option<f64>),
    Gap(Option<f64>),
}

/// One change to a [`Shake`].
pub enum ShakeEdit {
    Modifier(HOT_KEY_MODIFIERS),
    Shape(ShakeShape, bool),
    Speed(Option<f64>),
    Reversals(Option<f64>),
    Cooldown(Option<f64>),
    Window(Option<f64>),
    Segment(Option<f64>),
    Dominance(Option<f64>),
    Degrees(Option<f64>),
    CircleSpeed(Option<f64>),
}

/// What the screen has open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Open {
    Nothing,
    Base,
    Style(usize),
}

const SIDES: [Side; 3] = [Side::Any, Side::Left, Side::Right];
const RECALLS: [Recall; 2] = [Recall::None, Recall::Center];

fn recall_title(recall: Recall) -> &'static str {
    match recall {
        Recall::None => "Only show where it is",
        Recall::Center => "Bring it to the active window",
    }
}

/// How a shape reads in the UI, with what it does.
pub fn shape_title(shape: Shape) -> &'static str {
    match shape {
        Shape::Beacon => "Beacon — rings close in on the pointer",
        Shape::Spotlight => "Spotlight — the screen dims around it",
        Shape::Crosshair => "Crosshair — lines across the monitor",
        Shape::Monitor => "Monitor — flashes its screen, maps the others",
        Shape::Halo => "Halo — a ring that follows the pointer",
        Shape::Negative => "Negative — a lens inverting what is behind",
        Shape::Contrast => "Contrast — a black or white ring",
    }
}

/// The short name for a table cell.
fn shape_name(shape: Shape) -> &'static str {
    shape_title(shape).split(" — ").next().unwrap_or("")
}

fn shape_index(shape: Shape) -> Option<usize> {
    Shape::ALL.iter().position(|s| *s == shape)
}

fn key_title(key: TapKey) -> &'static str {
    match key {
        TapKey::Ctrl => "Ctrl",
        TapKey::Shift => "Shift",
        TapKey::Alt => "Alt",
        TapKey::Win => "Win",
    }
}

fn shake_shape_title(shape: ShakeShape) -> &'static str {
    match shape {
        ShakeShape::Horizontal => "Side to side",
        ShakeShape::Vertical => "Up and down",
        ShakeShape::Circle => "Circles",
    }
}

fn side_title(side: Side) -> &'static str {
    match side {
        Side::Any => "Either",
        Side::Left => "Left",
        Side::Right => "Right",
    }
}

// ---------------------------------------------------------------------------
// Changing the model
// ---------------------------------------------------------------------------

/// Sets `slot` from a number box's value, kept in `allowed`. `false` when
/// there is nothing to set or nothing changes.
fn set_number(slot: &mut u32, value: Option<f64>, allowed: RangeInclusive<u32>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let value = (value.round().max(0.0) as u32).clamp(*allowed.start(), *allowed.end());
    let changed = *slot != value;
    *slot = value;
    changed
}

pub fn apply_look(look: &mut Look, edit: LookEdit) -> bool {
    let before = *look;
    match edit {
        LookEdit::Shape(Some(choice)) => {
            if let Some(shape) = Shape::ALL.get(choice) {
                look.shape = *shape;
            }
        }
        LookEdit::Shape(None) => {}
        LookEdit::Duration(value) => {
            set_number(&mut look.duration_ms, value, finder_range::DURATION_MS);
        }
        LookEdit::Size(value) => {
            set_number(&mut look.size, value, finder_range::SIZE);
        }
        LookEdit::Dim(value) => {
            set_number(&mut look.dim, value, finder_range::DIM);
        }
        LookEdit::Recall(Some(choice)) => {
            if let Some(recall) = RECALLS.get(choice) {
                look.recall = *recall;
            }
        }
        LookEdit::Recall(None) => {}
        LookEdit::AutoColor(on) => look.auto_color = on,
        LookEdit::AllMonitors(on) => look.all_monitors = on,
        LookEdit::Color(color) => {
            // The picker has no alpha here: an indicator is always opaque.
            let mut color = from_ui_color(color);
            color.a = 255;
            look.color = color;
        }
    }
    *look != before
}

pub fn apply_tap(tap: &mut Tap, edit: TapEdit) -> bool {
    let before = *tap;
    match edit {
        TapEdit::Key(Some(choice)) => {
            if let Some(key) = TapKey::ALL.get(choice) {
                tap.key = *key;
            }
        }
        TapEdit::Side(Some(choice)) => {
            if let Some(side) = SIDES.get(choice) {
                tap.side = *side;
            }
        }
        TapEdit::Key(None) | TapEdit::Side(None) => {}
        TapEdit::Count(value) => {
            set_number(&mut tap.count, value, finder_range::TAP_COUNT);
        }
        TapEdit::Hold(value) => {
            set_number(&mut tap.max_hold_ms, value, finder_range::TAP_HOLD_MS);
        }
        TapEdit::Gap(value) => {
            set_number(&mut tap.max_gap_ms, value, finder_range::TAP_GAP_MS);
        }
    }
    *tap != before
}

pub fn apply_shake(shake: &mut Shake, edit: ShakeEdit) -> bool {
    let before = *shake;
    match edit {
        ShakeEdit::Modifier(modifier) => shake.modifiers.0 ^= modifier.0,
        ShakeEdit::Shape(shape, on) => {
            // A shake that recognises nothing would be a trigger that never
            // fires: the last shape stays on.
            let mut shapes = shake.shapes;
            shapes[shape as usize] = on;
            if shapes.iter().any(|on| *on) {
                shake.shapes = shapes;
            }
        }
        ShakeEdit::Speed(value) => {
            set_number(&mut shake.speed, value, finder_range::SHAKE_SPEED);
        }
        ShakeEdit::Reversals(value) => {
            set_number(&mut shake.reversals, value, finder_range::SHAKE_REVERSALS);
        }
        ShakeEdit::Cooldown(value) => {
            set_number(
                &mut shake.cooldown_ms,
                value,
                finder_range::SHAKE_COOLDOWN_MS,
            );
        }
        ShakeEdit::Window(value) => {
            set_number(&mut shake.window_ms, value, finder_range::SHAKE_WINDOW_MS);
        }
        ShakeEdit::Segment(value) => {
            set_number(&mut shake.segment, value, finder_range::SHAKE_SEGMENT);
        }
        ShakeEdit::Dominance(Some(value)) => {
            let range = finder_range::SHAKE_DOMINANCE;
            shake.dominance = (value as f32).clamp(*range.start(), *range.end());
        }
        ShakeEdit::Dominance(None) => {}
        ShakeEdit::Degrees(value) => {
            set_number(
                &mut shake.circle_degrees,
                value,
                finder_range::SHAKE_CIRCLE_DEGREES,
            );
        }
        ShakeEdit::CircleSpeed(value) => {
            set_number(
                &mut shake.circle_speed,
                value,
                finder_range::SHAKE_CIRCLE_SPEED,
            );
        }
    }
    *shake != before
}

/// A new style named after the first free "style", "style 2"... that starts
/// from the default look and has no trigger yet. Returns its index.
pub fn add_style(finder: &mut Finder) -> usize {
    let taken = |name: &str| finder.styles.iter().any(|style| style.name == name);
    let name = std::iter::once("style".to_owned())
        .chain((2..).map(|n| format!("style {n}")))
        .find(|name| !taken(name))
        .expect("an unbounded range has a free name");
    finder.styles.push(Style::new(name, finder.look));
    finder.styles.len() - 1
}

/// Removes a style and fixes everything that points at one by position: the
/// default style, and the `style` of hotkeys and mouse shortcuts (which fall
/// back to the default style when theirs is gone).
pub fn delete_style(settings: &mut Settings, index: usize) {
    settings.finder.styles.remove(index);
    let finder = &mut settings.finder;
    if finder.default_style == index {
        finder.default_style = 0;
    } else if finder.default_style > index {
        finder.default_style -= 1;
    }
    let index = index as u16;
    let fix = |style: &mut Option<u16>| match *style {
        Some(i) if i == index => *style = None,
        Some(i) if i > index => *style = Some(i - 1),
        _ => {}
    };
    settings.hotkeys.iter_mut().for_each(|b| fix(&mut b.style));
    settings
        .mouse_shortcuts
        .iter_mut()
        .for_each(|s| fix(&mut s.style));
}

/// The `style` of a binding for a choice in [`style_combo`]: choice 0 is the
/// default style (`None`), the others are the styles in order.
pub fn style_from_choice(choice: usize) -> Option<u16> {
    choice
        .checked_sub(1)
        .and_then(|style| u16::try_from(style).ok())
}

/// The dropdown that picks which style a `find-mouse` hotkey or shortcut
/// plays. `current` is its `style`; a hand-edited one naming no style reads as
/// the default.
pub fn style_combo(
    finder: &Finder,
    current: Option<u16>,
    on_change: Callback<Option<usize>>,
) -> View {
    let names = std::iter::once("(default style)".to_owned())
        .chain(finder.styles.iter().map(|style| style.name.clone()));
    let selected = current
        .map(usize::from)
        .filter(|style| *style < finder.styles.len())
        .map_or(0, |style| style + 1);
    combo(names, Some(selected), on_change)
        .is_enabled(!finder.styles.is_empty())
        .into()
}

/// Swaps style `index` with the one above (`up`) or below it. The order is
/// the file's, and what points at a style by position follows it: the default
/// style, and the `style` of hotkeys and mouse shortcuts. `false`, and nothing
/// changes, when there is no neighbour that way.
pub fn move_style(settings: &mut Settings, index: usize, up: bool) -> bool {
    let other = if up {
        index.checked_sub(1)
    } else {
        index.checked_add(1)
    };
    let styles = settings.finder.styles.len();
    let Some(other) = other.filter(|other| index < styles && *other < styles) else {
        return false;
    };
    settings.finder.styles.swap(index, other);

    let swap = |position: usize| match position {
        p if p == index => other,
        p if p == other => index,
        p => p,
    };
    settings.finder.default_style = swap(settings.finder.default_style);
    let fix = |style: &mut Option<u16>| {
        if let Some(i) = *style {
            *style = Some(swap(usize::from(i)) as u16);
        }
    };
    settings.hotkeys.iter_mut().for_each(|b| fix(&mut b.style));
    settings
        .mouse_shortcuts
        .iter_mut()
        .for_each(|s| fix(&mut s.style));
    true
}

/// Adds an excluded application, as y-snap compares them. `false` for an
/// empty name or one already there.
pub fn add_exclude(finder: &mut Finder, name: &str) -> bool {
    let name = normalize_process(name);
    if name.is_empty() || finder.exclude.contains(&name) {
        return false;
    }
    finder.exclude.push(name);
    true
}

/// Why the style at `index` cannot be kept as it is: y-snap skips a style with
/// no name or with the name of an earlier one.
pub fn style_problem(finder: &Finder, index: usize) -> Option<&'static str> {
    let name = finder.styles.get(index)?.name.trim();
    if name.is_empty() {
        return Some("A style needs a name.");
    }
    let twin = finder
        .styles
        .iter()
        .enumerate()
        .any(|(i, style)| i != index && style.name.trim() == name);
    twin.then_some("Another style has this name.")
}

/// "Ctrl ×2 · Ctrl+shake": what plays a style, for the table.
pub fn trigger_summary(style: &Style) -> String {
    let mut parts = Vec::new();
    for tap in &style.taps {
        let side = match tap.side {
            Side::Any => "",
            Side::Left => "left ",
            Side::Right => "right ",
        };
        parts.push(format!("{side}{} ×{}", key_title(tap.key), tap.count));
    }
    for shake in &style.shakes {
        let held: Vec<&str> = MODIFIERS
            .iter()
            .filter(|(_, modifier)| shake.modifiers.0 & modifier.0 != 0)
            .map(|(name, _)| *name)
            .collect();
        parts.push(if held.is_empty() {
            "shake".to_owned()
        } else {
            format!("{}+shake", held.join("+"))
        });
    }
    if parts.is_empty() {
        "none (hotkey or action only)".to_owned()
    } else {
        parts.join(" · ")
    }
}

// ---------------------------------------------------------------------------
// The screen
// ---------------------------------------------------------------------------

const COLUMNS: [Column; 5] = [
    Column::flex("Style", 130.0),
    Column::flex("Look", 90.0),
    Column::flex("Triggers", 160.0),
    Column::fixed("Default", 96.0),
    // Room for the ↑/↓ pair while reordering; one delete button otherwise.
    Column::actions(TWO_ICON_BUTTONS),
];

pub fn finder_view(
    finder: &Finder,
    open: Open,
    reordering: bool,
    color_open: bool,
    draft: &str,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let table = Table::new(&COLUMNS)
        .empty_message("No style: only the find-mouse action works, with the default look.");
    let detail_viewport = table.detail_viewport(viewport).inset(CARD_INSET);

    let rows = finder.styles.iter().enumerate().map(|(index, style)| {
        let detail = (open == Open::Style(index)).then(|| {
            editor_checked(
                format!("Style “{}”", style.name),
                style_editor(finder, index, color_open, detail_viewport, context),
                style_problem(finder, index).is_none(),
                context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
                context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
            )
        });
        Row::new(
            index.to_string(),
            cells(finder, index, style, reordering, context),
        )
        .detail(detail)
    });

    let base_editor = (open == Open::Base).then(|| {
        editor(
            "Default look",
            look_editor(
                LookTarget::Base,
                &finder.look,
                color_open,
                detail_viewport,
                context,
            ),
            context.callback(|()| Msg::EditorClosed(EditorClose::Revert)),
            context.callback(|()| Msg::EditorClosed(EditorClose::Keep)),
        )
    });

    let general = Stack::vertical(space::S)
        .child(
            "guard",
            setting(
                "Stay quiet while a full-screen window is up",
                "A game in its menus or a full-screen video on the pointer's monitor: the automatic triggers do \
                 nothing. The find-mouse action still works.",
                switch(finder.guard, context.callback(Msg::FinderGuardToggled)),
                viewport,
            ),
        )
        .child("exclude", exclude_card(finder, draft, context))
        .child(
            "base",
            setting(
                "Default look",
                "What find-mouse plays when there is no style, and where a new style starts from.",
                button(
                    "Edit",
                    open == Open::Base,
                    true,
                    context.callback(|()| Msg::EditFinderBase),
                ),
                viewport,
            ),
        )
        .maybe("base-editor", base_editor);

    let body = Stack::vertical(space::XL)
        .child("general", general)
        .child(
            "styles",
            section(
                format!("Styles — {}", finder.styles.len()),
                Stack::vertical(space::L)
                    .child("table", table.rows(rows).build(viewport))
                    .child(
                        "add",
                        list_footer(
                            ListKind::FinderStyles,
                            add_button("Add style", context.callback(|()| Msg::FinderStyleAdded)),
                            finder.styles.len(),
                            reordering,
                            context,
                        ),
                    ),
            ),
        )
        .child(
            "note",
            note(
                "A style is a look plus what plays it: taps on a modifier key, shaking the pointer, or both. The \
                 find-mouse action plays the default style. The finder stays quiet in a game that confines the \
                 pointer, whatever is set here.",
            ),
        );
    page(
        "Mouse finder",
        "Find a lost pointer — mostly on several monitors — with a short animation around it.",
        body,
    )
}

fn cells(
    finder: &Finder,
    index: usize,
    style: &Style,
    reordering: bool,
    context: &ViewContext<ConfigApp>,
) -> Vec<View> {
    let name: View = Button::new()
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Left)
        .on_click(context.callback(move |()| Msg::EditFinderStyle(index)))
        .content(text::single_line(style.name.clone()))
        .tooltip("Edit this style");
    let is_default = finder.default_style == index;
    vec![
        name,
        text::body(shape_name(style.look.shape)).into(),
        text::caption(trigger_summary(style)).into(),
        button(
            "Default",
            is_default,
            true,
            context.callback(move |()| Msg::FinderStyleDefault(index)),
        ),
        row_actions(
            ListKind::FinderStyles,
            index,
            finder.styles.len(),
            reordering,
            || {
                icon_button(
                    Symbol::Delete,
                    "Delete style",
                    context.callback(move |()| Msg::FinderStyleDeleted(index)),
                )
            },
            context,
        ),
    ]
}

fn exclude_card(finder: &Finder, draft: &str, context: &ViewContext<ConfigApp>) -> View {
    let listed = finder.exclude.iter().enumerate().map(|(index, name)| {
        let row: View = Grid::new()
            .columns([GridLength::STAR, GridLength::Auto])
            .column_spacing(space::S)
            .keyed_children([
                (
                    "name",
                    cell(
                        text::body(name.clone()).vertical_alignment(VerticalAlignment::Center),
                        0,
                        0,
                    ),
                ),
                (
                    "remove",
                    cell(
                        icon_button(
                            Symbol::Delete,
                            "Stop excluding it",
                            context.callback(move |()| Msg::FinderExcludeRemoved(index)),
                        ),
                        0,
                        1,
                    ),
                ),
            ]);
        (name.clone(), row)
    });

    let add: View = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .column_spacing(space::S)
        .keyed_children([
            (
                "name",
                cell(
                    text_box(
                        draft,
                        "Executable name, such as game",
                        context.callback(Msg::FinderExcludeDraft),
                    ),
                    0,
                    0,
                ),
            ),
            (
                "add",
                cell(
                    button(
                        "Add",
                        false,
                        !draft.trim().is_empty(),
                        context.callback(|()| Msg::FinderExcludeAdded),
                    ),
                    0,
                    1,
                ),
            ),
        ]);

    card().content(
        Stack::vertical(space::M)
            .child(
                "words",
                Stack::vertical(space::XXS)
                    .child("title", text::body("Excluded applications"))
                    .child(
                        "description",
                        text::caption(
                            "While one of these has the focus, the automatic triggers do nothing. Without the \
                             folder or .exe.",
                        ),
                    ),
            )
            .child("list", Stack::vertical(space::XS).children(listed))
            .child("add", add),
    )
}

// -- the style editor ------------------------------------------------------

fn style_editor(
    finder: &Finder,
    index: usize,
    color_open: bool,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let style = &finder.styles[index];
    let problem = style_problem(finder, index).map(|text| note(format!("⚠ {text}")));

    let taps = style.taps.iter().enumerate().map(|(t, tap)| {
        (
            format!("tap-{t}"),
            tap_card(index, t, tap, viewport, context),
        )
    });
    let shakes = style.shakes.iter().enumerate().map(|(s, shake)| {
        (
            format!("shake-{s}"),
            shake_card(index, s, shake, viewport, context),
        )
    });
    let add = Stack::horizontal(space::S)
        .child(
            "tap",
            add_button(
                "Add a tap",
                context.callback(move |()| Msg::FinderTapAdded(index)),
            ),
        )
        .child(
            "shake",
            add_button(
                "Add a shake",
                context.callback(move |()| Msg::FinderShakeAdded(index)),
            ),
        );

    Stack::vertical(space::L)
        .child(
            "name",
            field(
                "Name",
                text_box(
                    style.name.clone(),
                    "Name",
                    context.callback(move |text| Msg::FinderStyleName(index, text)),
                ),
            ),
        )
        .maybe("problem", problem)
        .child(
            "look",
            section(
                "Look",
                look_editor(
                    LookTarget::Style(index),
                    &style.look,
                    color_open,
                    viewport,
                    context,
                ),
            ),
        )
        .child(
            "triggers",
            section(
                "Triggers",
                Stack::vertical(space::M)
                    .children(taps)
                    .children(shakes)
                    .child("add", add),
            ),
        )
        .into()
}

fn look_editor(
    target: LookTarget,
    look: &Look,
    color_open: bool,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let number = |value: u32, range: RangeInclusive<u32>, make: fn(Option<f64>) -> LookEdit| {
        number_box(
            Some(value.into()),
            f64::from(*range.start()),
            f64::from(*range.end()),
            context.callback(move |v| Msg::FinderLook(target, make(v))),
        )
    };

    let fields = AutoGrid::new(190.0).max_columns(3).items([
        (
            "shape",
            field(
                "Shape",
                combo(
                    Shape::ALL.map(shape_title),
                    shape_index(look.shape),
                    context.callback(move |c| Msg::FinderLook(target, LookEdit::Shape(c))),
                ),
            ),
        ),
        (
            "duration",
            field(
                "Duration (ms)",
                number(
                    look.duration_ms,
                    finder_range::DURATION_MS,
                    LookEdit::Duration,
                ),
            ),
        ),
        (
            "size",
            field(
                "Size (px)",
                number(look.size, finder_range::SIZE, LookEdit::Size),
            ),
        ),
        (
            "dim",
            field(
                "Screen dimming (%)",
                number(look.dim, finder_range::DIM, LookEdit::Dim),
            ),
        ),
        (
            "recall",
            field(
                "When it plays",
                combo(
                    RECALLS.map(recall_title),
                    RECALLS.iter().position(|r| *r == look.recall),
                    context.callback(move |c| Msg::FinderLook(target, LookEdit::Recall(c))),
                ),
            ),
        ),
    ]);

    // A swatch with its hex code, as small as a field; the picker itself is
    // tall, so it only opens under it when asked.
    let hex = format!(
        "#{:02X}{:02X}{:02X}",
        look.color.r, look.color.g, look.color.b
    );
    let swatch: View = Border::new()
        .width(size::SWATCH_HEIGHT)
        .height(size::SWATCH_HEIGHT)
        .corner_radius(CornerRadius::uniform(radius::CONTROL))
        .border_thickness(Thickness::uniform(size::STROKE))
        .border_brush(ThemeBrush::CardStroke)
        .background(to_ui_color(look.color))
        .into();
    let color_button: View = Button::new()
        .style(if color_open {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Default
        })
        .horizontal_alignment(HorizontalAlignment::Left)
        .on_click(context.callback(|()| Msg::FinderColorToggled))
        .content(Stack::horizontal(space::S).child("swatch", swatch).child(
            "hex",
            text::body(hex).vertical_alignment(VerticalAlignment::Center),
        ))
        .tooltip(if color_open {
            "Close the colour picker"
        } else {
            "Change the colour"
        });
    let picker = color_open.then(|| -> View {
        ColorPicker::new()
            .color(to_ui_color(look.color))
            .is_alpha_enabled(false)
            .on_color_changed(
                context.callback(move |c| Msg::FinderLook(target, LookEdit::Color(c))),
            )
            .into()
    });

    let persistent = look
        .shape
        .is_persistent()
        .then(|| note("This shape stays up, for four times the duration (ten seconds at most)."));

    let spread = look.shape.spreads().then(|| -> View {
        labeled_switch(
            "Show it on every monitor",
            look.all_monitors,
            context.callback(move |on| Msg::FinderLook(target, LookEdit::AllMonitors(on))),
        )
    });

    Stack::vertical(space::M)
        .child("fields", fields.build(viewport))
        .maybe("persistent", persistent)
        .maybe("spread", spread)
        .child(
            "auto",
            labeled_switch(
                "Adapt the colour to what is behind it",
                look.auto_color,
                context.callback(move |on| Msg::FinderLook(target, LookEdit::AutoColor(on))),
            ),
        )
        .child("color", field("Colour", color_button))
        .maybe("picker", picker)
        .child("test", test_row(target, context))
        .into()
}

/// The Test button: plays this look, as it is now, saved or not, in the running
/// y-snap — around the pointer, which is on this window.
fn test_row(target: LookTarget, context: &ViewContext<ConfigApp>) -> View {
    let running = y_snap::ipc::is_running();
    let words = if running {
        "Plays this look once around the pointer, without saving."
    } else {
        "y-snap isn't running: start it to try a look."
    };
    Stack::horizontal(space::M)
        .child(
            "button",
            button(
                "Test",
                false,
                running,
                context.callback(move |()| Msg::FinderTest(target)),
            ),
        )
        .child(
            "words",
            text::caption(words).vertical_alignment(VerticalAlignment::Center),
        )
        .into()
}

/// A card for one trigger: its title, a delete button, then `body`.
fn trigger_card(title: &str, on_delete: Callback<()>, body: impl Into<View>) -> View {
    let header: View = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .keyed_children([
            (
                "title",
                cell(
                    text::strong(title).vertical_alignment(VerticalAlignment::Center),
                    0,
                    0,
                ),
            ),
            (
                "delete",
                cell(
                    icon_button(Symbol::Delete, "Delete this trigger", on_delete),
                    0,
                    1,
                ),
            ),
        ]);
    card().content(
        Stack::vertical(space::M)
            .child("header", header)
            .child("body", body),
    )
}

fn tap_card(
    style: usize,
    tap_index: usize,
    tap: &Tap,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let edit = |make: fn(Option<f64>) -> TapEdit| {
        context.callback(move |v| Msg::FinderTap(style, tap_index, make(v)))
    };
    let body = AutoGrid::new(120.0).max_columns(5).items([
        (
            "key",
            field(
                "Key",
                combo(
                    TapKey::ALL.map(key_title),
                    TapKey::ALL.iter().position(|k| *k == tap.key),
                    context.callback(move |c| Msg::FinderTap(style, tap_index, TapEdit::Key(c))),
                ),
            ),
        ),
        (
            "side",
            field(
                "Side",
                combo(
                    SIDES.map(side_title),
                    SIDES.iter().position(|s| *s == tap.side),
                    context.callback(move |c| Msg::FinderTap(style, tap_index, TapEdit::Side(c))),
                ),
            ),
        ),
        (
            "count",
            field(
                "Taps",
                number_box(
                    Some(tap.count.into()),
                    f64::from(*finder_range::TAP_COUNT.start()),
                    f64::from(*finder_range::TAP_COUNT.end()),
                    edit(TapEdit::Count),
                ),
            ),
        ),
        (
            "hold",
            field(
                "Longest press (ms)",
                number_box(
                    Some(tap.max_hold_ms.into()),
                    f64::from(*finder_range::TAP_HOLD_MS.start()),
                    f64::from(*finder_range::TAP_HOLD_MS.end()),
                    edit(TapEdit::Hold),
                ),
            ),
        ),
        (
            "gap",
            field(
                "Longest gap (ms)",
                number_box(
                    Some(tap.max_gap_ms.into()),
                    f64::from(*finder_range::TAP_GAP_MS.start()),
                    f64::from(*finder_range::TAP_GAP_MS.end()),
                    edit(TapEdit::Gap),
                ),
            ),
        ),
    ]);
    trigger_card(
        "Tap",
        context.callback(move |()| Msg::FinderTapDeleted(style, tap_index)),
        body.build(viewport.inset(CARD_INSET)),
    )
}

fn shake_card(
    style: usize,
    shake_index: usize,
    shake: &Shake,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let inner = viewport.inset(CARD_INSET);
    let modifiers = AutoGrid::new(56.0)
        .max_columns(4)
        .gap(space::XS, space::XS)
        .items(MODIFIERS.map(|(name, modifier)| {
            (
                name,
                chip(
                    name,
                    shake.modifiers.0 & modifier.0 != 0,
                    true,
                    context.callback(move |_| {
                        Msg::FinderShake(style, shake_index, ShakeEdit::Modifier(modifier))
                    }),
                ),
            )
        }));
    let shapes = AutoGrid::new(110.0)
        .max_columns(3)
        .gap(space::XS, space::XS)
        .items(ShakeShape::ALL.map(|kind| {
            (
                kind.name(),
                chip(
                    shake_shape_title(kind),
                    shake.has(kind),
                    true,
                    context.callback(move |on| {
                        Msg::FinderShake(style, shake_index, ShakeEdit::Shape(kind, on))
                    }),
                ),
            )
        }));

    let number =
        |label: &str, value: f64, range: (u32, u32), make: fn(Option<f64>) -> ShakeEdit| {
            field(
                label,
                number_box(
                    Some(value),
                    f64::from(range.0),
                    f64::from(range.1),
                    context.callback(move |v| Msg::FinderShake(style, shake_index, make(v))),
                ),
            )
        };
    let bounds = |range: RangeInclusive<u32>| (*range.start(), *range.end());
    let dominance = finder_range::SHAKE_DOMINANCE;
    let tuning = AutoGrid::new(130.0).max_columns(4).items([
        (
            "speed",
            number(
                "Slowest speed (px/s)",
                shake.speed.into(),
                bounds(finder_range::SHAKE_SPEED),
                ShakeEdit::Speed,
            ),
        ),
        (
            "reversals",
            number(
                "Turns back and forth",
                shake.reversals.into(),
                bounds(finder_range::SHAKE_REVERSALS),
                ShakeEdit::Reversals,
            ),
        ),
        (
            "cooldown",
            number(
                "Quiet after (ms)",
                shake.cooldown_ms.into(),
                bounds(finder_range::SHAKE_COOLDOWN_MS),
                ShakeEdit::Cooldown,
            ),
        ),
        (
            "window",
            number(
                "Within (ms)",
                shake.window_ms.into(),
                bounds(finder_range::SHAKE_WINDOW_MS),
                ShakeEdit::Window,
            ),
        ),
        (
            "segment",
            number(
                "Smallest stroke (px)",
                shake.segment.into(),
                bounds(finder_range::SHAKE_SEGMENT),
                ShakeEdit::Segment,
            ),
        ),
        (
            "dominance",
            field(
                "Straightness (1–4)",
                number_box(
                    Some(f64::from(shake.dominance)),
                    f64::from(*dominance.start()),
                    f64::from(*dominance.end()),
                    context.callback(move |v| {
                        Msg::FinderShake(style, shake_index, ShakeEdit::Dominance(v))
                    }),
                ),
            ),
        ),
        (
            "degrees",
            number(
                "Circle turn (°)",
                shake.circle_degrees.into(),
                bounds(finder_range::SHAKE_CIRCLE_DEGREES),
                ShakeEdit::Degrees,
            ),
        ),
        (
            "circle-speed",
            number(
                "Slowest circle (px/s)",
                shake.circle_speed.into(),
                bounds(finder_range::SHAKE_CIRCLE_SPEED),
                ShakeEdit::CircleSpeed,
            ),
        ),
    ]);

    let hint = if shake.modifiers.0 == 0 {
        "No key held: a firm shake is needed, so that moving the pointer fast never sets it off."
    } else {
        "With the keys held you are already looking for the pointer, so a gentler shake is enough."
    };

    let body = Stack::vertical(space::M)
        .child(
            "modifiers",
            field("Keys held while shaking", modifiers.build(inner)),
        )
        .child("shapes", field("Kinds of shake", shapes.build(inner)))
        .child("tuning", tuning.build(inner))
        .child("hint", text::caption(hint));
    trigger_card(
        "Shake",
        context.callback(move |()| Msg::FinderShakeDeleted(style, shake_index)),
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_SHIFT};
    use y_snap::config::Binding;

    fn settings_with_styles(names: &[&str]) -> Settings {
        let mut settings = Settings::empty(std::path::PathBuf::from("y-snap.xml"));
        for name in names {
            settings
                .finder
                .styles
                .push(Style::new(*name, Look::default()));
        }
        settings
    }

    #[test]
    fn numbers_are_kept_in_range_and_unchanged_is_not_an_edit() {
        let mut look = Look::default();
        let same = f64::from(look.size);
        assert!(!apply_look(&mut look, LookEdit::Size(Some(same))));
        assert!(apply_look(&mut look, LookEdit::Size(Some(100_000.0))));
        assert_eq!(look.size, *finder_range::SIZE.end());
        assert!(!apply_look(&mut look, LookEdit::Duration(None)));
    }

    #[test]
    fn the_colour_is_always_opaque() {
        let mut look = Look::default();
        assert!(apply_look(
            &mut look,
            LookEdit::Color(Color::argb(10, 1, 2, 3))
        ));
        assert_eq!(
            (look.color.r, look.color.g, look.color.b, look.color.a),
            (1, 2, 3, 255)
        );
    }

    #[test]
    fn a_shake_keeps_at_least_one_shape() {
        let mut shake = Shake::new(HOT_KEY_MODIFIERS(0));
        assert!(apply_shake(
            &mut shake,
            ShakeEdit::Shape(ShakeShape::Circle, false)
        ));
        assert!(apply_shake(
            &mut shake,
            ShakeEdit::Shape(ShakeShape::Vertical, false)
        ));
        assert!(!apply_shake(
            &mut shake,
            ShakeEdit::Shape(ShakeShape::Horizontal, false)
        ));
        assert_eq!(shake.shapes, [true, false, false]);
    }

    #[test]
    fn a_modifier_chip_toggles() {
        let mut shake = Shake::new(HOT_KEY_MODIFIERS(0));
        assert!(apply_shake(&mut shake, ShakeEdit::Modifier(MOD_CONTROL)));
        assert_eq!(shake.modifiers, MOD_CONTROL);
        assert!(apply_shake(&mut shake, ShakeEdit::Modifier(MOD_CONTROL)));
        assert_eq!(shake.modifiers.0, 0);
    }

    #[test]
    fn recall_is_chosen_from_the_list() {
        let mut look = Look::default();
        assert!(!apply_look(&mut look, LookEdit::Recall(Some(0))));
        assert!(apply_look(&mut look, LookEdit::Recall(Some(1))));
        assert_eq!(look.recall, Recall::Center);
        assert!(!apply_look(&mut look, LookEdit::Recall(Some(9))));
        assert!(!apply_look(&mut look, LookEdit::Recall(None)));
    }

    #[test]
    fn a_tap_changes_by_choice_and_number() {
        let mut tap = Tap::new(TapKey::Ctrl);
        assert!(apply_tap(&mut tap, TapEdit::Key(Some(2))));
        assert_eq!(tap.key, TapKey::Alt);
        assert!(!apply_tap(&mut tap, TapEdit::Side(Some(0))));
        assert!(apply_tap(&mut tap, TapEdit::Count(Some(9.0))));
        assert_eq!(tap.count, *finder_range::TAP_COUNT.end());
    }

    #[test]
    fn new_styles_get_free_names_and_the_default_look() {
        let mut finder = Finder::default();
        finder.look.size = 222;
        assert_eq!(add_style(&mut finder), 0);
        assert_eq!(add_style(&mut finder), 1);
        assert_eq!(finder.styles[0].name, "style");
        assert_eq!(finder.styles[1].name, "style 2");
        assert_eq!(finder.styles[1].look.size, 222);
        finder.styles.remove(0);
        assert_eq!(add_style(&mut finder), 1);
        assert_eq!(finder.styles[1].name, "style");
    }

    #[test]
    fn deleting_a_style_fixes_what_points_at_one() {
        let mut settings = settings_with_styles(&["a", "b", "c"]);
        settings.finder.default_style = 2;
        let binding = |style| Binding {
            hotkey: y_core::hotkey::Hotkey::parse("Ctrl+Alt+F13").unwrap(),
            action: y_snap::action::Action::FindMouse,
            target: y_snap::action::Target::Focused,
            desktop: 0,
            style,
            steal: false,
        };
        settings.hotkeys = vec![
            binding(Some(0)),
            binding(Some(1)),
            binding(Some(2)),
            binding(None),
        ];

        delete_style(&mut settings, 1);

        let names: Vec<_> = settings
            .finder
            .styles
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["a", "c"]);
        assert_eq!(settings.finder.default_style, 1);
        let styles: Vec<_> = settings.hotkeys.iter().map(|b| b.style).collect();
        assert_eq!(styles, [Some(0), None, Some(1), None]);

        delete_style(&mut settings, 1);
        assert_eq!(settings.finder.default_style, 0);
    }

    #[test]
    fn moving_a_style_keeps_what_points_at_one() {
        let mut settings = settings_with_styles(&["a", "b", "c"]);
        settings.finder.default_style = 1;
        let binding = |style| Binding {
            hotkey: y_core::hotkey::Hotkey::parse("Ctrl+Alt+F13").unwrap(),
            action: y_snap::action::Action::FindMouse,
            target: y_snap::action::Target::Focused,
            desktop: 0,
            style,
            steal: false,
        };
        settings.hotkeys = vec![
            binding(Some(0)),
            binding(Some(1)),
            binding(Some(2)),
            binding(None),
        ];

        assert!(move_style(&mut settings, 1, true));

        let names: Vec<_> = settings
            .finder
            .styles
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["b", "a", "c"]);
        // "b" is still the default and still what the second hotkey plays.
        assert_eq!(settings.finder.default_style, 0);
        let styles: Vec<_> = settings.hotkeys.iter().map(|b| b.style).collect();
        assert_eq!(styles, [Some(1), Some(0), Some(2), None]);

        assert!(!move_style(&mut settings, 0, true));
        assert!(!move_style(&mut settings, 2, false));
        assert!(!move_style(&mut settings, 9, true));
    }

    #[test]
    fn excluded_applications_are_spelled_as_y_snap_compares_them() {
        let mut finder = Finder::default();
        assert!(add_exclude(&mut finder, "  Game.EXE "));
        assert_eq!(finder.exclude, ["game"]);
        assert!(!add_exclude(&mut finder, "game"));
        assert!(!add_exclude(&mut finder, "   "));
    }

    #[test]
    fn a_style_needs_its_own_name() {
        let settings = settings_with_styles(&["a", "a", " "]);
        let finder = &settings.finder;
        assert!(style_problem(finder, 0).is_some());
        assert!(style_problem(finder, 1).is_some());
        assert!(style_problem(finder, 2).is_some());
        let fine = settings_with_styles(&["a", "b"]);
        assert!(style_problem(&fine.finder, 0).is_none());
    }

    #[test]
    fn the_summary_names_each_trigger() {
        let mut style = Style::new("quick", Look::default());
        assert!(trigger_summary(&style).starts_with("none"));
        let mut tap = Tap::new(TapKey::Ctrl);
        tap.side = Side::Right;
        style.taps.push(tap);
        style.shakes.push(Shake::new(HOT_KEY_MODIFIERS(0)));
        style
            .shakes
            .push(Shake::new(HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_SHIFT.0)));
        assert_eq!(
            trigger_summary(&style),
            "right Ctrl ×2 · shake · Ctrl+Shift+shake"
        );
    }
}
