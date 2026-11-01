//! The Skin screen's model: which colour is being edited, and every edit as a
//! plain function on `Settings` (the view is `skin_view`).
//!
//! The file holds `theme=` on `<y-pop>` and, optionally, a `<skin>` of
//! overrides on top of it. An override that is not set is "inherit": the
//! theme supplies the value each time y-pop opens a menu, which is what keeps
//! `auto` following Windows. So nothing here copies a theme's value into the
//! file unless the user changed it, with one exception: switching on a
//! composite (border, a gradient) starts it from the theme's own, because it
//! is written whole or not at all.
//!
//! Like y-pop's loader, a Native theme with any override is drawn as Auto
//! (`y_pop::skins::resolve_design`); [`effective_theme`] says so.

use std::cell::RefCell;

use y_core::menu::{Fill, MenuDesign};
use y_core::overlay::Color;
use y_pop::settings::Settings;
use y_pop::skins;
use y_pop::theme::{
    BorderOverride, EdgeRule, GradientDirection, GradientOverride, SeparatorSpan, Theme,
    ThemeOverrides,
};

use crate::skin_preview::Previews;
use crate::{ConfigApp, Undo};

/// Padding and margin are pixels; nobody needs a row gap past this.
pub const MAX_GAP: f64 = 64.0;
pub const MAX_BORDER: f64 = 16.0;
pub const MAX_ROUND: f64 = 64.0;

/// The themes, in the order the cards are shown.
pub const THEMES: [Theme; 8] = [
    Theme::Native,
    Theme::Default,
    Theme::Flat,
    Theme::Dark,
    Theme::Graphite,
    Theme::Midnight,
    Theme::Sand,
    Theme::Auto,
];

pub fn theme_title(theme: Theme) -> &'static str {
    match theme {
        Theme::Native => "Native",
        Theme::Default => "Default",
        Theme::Flat => "Flat",
        Theme::Dark => "Dark",
        Theme::Graphite => "Graphite",
        Theme::Midnight => "Midnight",
        Theme::Sand => "Sand",
        Theme::Auto => "Auto",
    }
}

/// One colour of the skin, the five plain ones and the colours inside the
/// composite overrides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorSlot {
    Back,
    Text,
    HoverBack,
    HoverText,
    DisabledText,
    BorderColor,
    BackgroundStart,
    BackgroundEnd,
    IconStart,
    IconEnd,
}

impl ColorSlot {
    pub const PLAIN: [Self; 5] = [
        Self::Back,
        Self::Text,
        Self::HoverBack,
        Self::HoverText,
        Self::DisabledText,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Back => "Back",
            Self::Text => "Text",
            Self::HoverBack => "Hover back",
            Self::HoverText => "Hover text",
            Self::DisabledText => "Disabled text",
            Self::BorderColor => "Border",
            Self::BackgroundStart => "Start",
            Self::BackgroundEnd => "End",
            Self::IconStart => "Start",
            Self::IconEnd => "End",
        }
    }

    /// [`ColorSlot::title`] for what the colour is in `scope`: the search
    /// window has a selected row, and hints where a menu has disabled rows.
    pub fn title_in(self, scope: Scope) -> &'static str {
        match (scope, self) {
            (Scope::Search, Self::HoverBack) => "Selected back",
            (Scope::Search, Self::HoverText) => "Selected text",
            (Scope::Search, Self::DisabledText) => "Hints",
            _ => self.title(),
        }
    }

    /// Which composite a colour belongs to, if it does.
    pub fn part(self) -> Option<Part> {
        match self {
            Self::BorderColor => Some(Part::Border),
            Self::BackgroundStart | Self::BackgroundEnd => Some(Part::Background),
            Self::IconStart | Self::IconEnd => Some(Part::Icon),
            _ => None,
        }
    }

    /// What the skin overrides, if it does.
    pub fn get(self, overrides: &ThemeOverrides) -> Option<Color> {
        match self {
            Self::Back => overrides.back,
            Self::Text => overrides.text,
            Self::HoverBack => overrides.hover_back,
            Self::HoverText => overrides.hover_text,
            Self::DisabledText => overrides.disabled_text,
            Self::BorderColor => overrides.border.map(|border| border.color),
            Self::BackgroundStart => overrides.background_gradient.map(|g| g.start),
            Self::BackgroundEnd => overrides.background_gradient.map(|g| g.end),
            Self::IconStart => overrides.icon_gradient.map(|g| g.start),
            Self::IconEnd => overrides.icon_gradient.map(|g| g.end),
        }
    }

    /// What the theme gives it.
    pub fn of_design(self, design: &MenuDesign) -> Color {
        let gradient = match design.fill {
            Fill::Gradient(gradient) => Some(gradient),
            Fill::Solid => None,
        };
        match self {
            Self::Back => design.base.back,
            Self::Text => design.base.text,
            Self::HoverBack => design.selected.back,
            Self::HoverText => design.selected.text,
            Self::DisabledText => design.disabled_text,
            Self::BorderColor => design.border.color,
            Self::BackgroundStart => gradient.map_or(design.selected.back, |g| g.start),
            Self::BackgroundEnd => gradient.map_or(design.selected.back, |g| g.end),
            Self::IconStart => design
                .icon_backdrop
                .map_or(design.selected.back, |g| g.start),
            Self::IconEnd => design.icon_backdrop.map_or(design.selected.back, |g| g.end),
        }
    }

    /// What the menu uses: the override, else the theme's.
    pub fn effective(self, overrides: &ThemeOverrides, base: &MenuDesign) -> Color {
        self.get(overrides).unwrap_or_else(|| self.of_design(base))
    }

    /// Sets or clears (`None`) the override. A colour inside a composite
    /// can only be changed while that composite is on, and not cleared on its
    /// own. Whether anything changed.
    fn set(self, overrides: &mut ThemeOverrides, color: Option<Color>) -> bool {
        if self.get(overrides) == color {
            return false;
        }
        match self {
            Self::Back => overrides.back = color,
            Self::Text => overrides.text = color,
            Self::HoverBack => overrides.hover_back = color,
            Self::HoverText => overrides.hover_text = color,
            Self::DisabledText => overrides.disabled_text = color,
            composite => {
                let (Some(color), true) = (color, composite.get(overrides).is_some()) else {
                    return false;
                };
                // `get` was `Some`, so the composite exists.
                if let Some(border) = &mut overrides.border
                    && composite == Self::BorderColor
                {
                    border.color = color;
                }
                if let Some(gradient) = &mut overrides.background_gradient {
                    match composite {
                        Self::BackgroundStart => gradient.start = color,
                        Self::BackgroundEnd => gradient.end = color,
                        _ => {}
                    }
                }
                if let Some(gradient) = &mut overrides.icon_gradient {
                    match composite {
                        Self::IconStart => gradient.start = color,
                        Self::IconEnd => gradient.end = color,
                        _ => {}
                    }
                }
            }
        }
        true
    }
}

/// An override written whole or not at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Border,
    Background,
    Icon,
}

/// The colour being edited, and what it was when the editor opened (`None`:
/// it was inherited), for Cancel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Editing {
    pub slot: ColorSlot,
    original: Option<Color>,
}

/// Everything the Skin screen remembers besides the settings.
#[derive(Default)]
pub struct SkinUi {
    pub editing: Option<Editing>,
    pub previews: RefCell<Previews>,
    pub scope: Scope,
}

pub enum SkinMsg {
    /// Switch between the menus' look and the search window's.
    Scope(Scope),
    /// Give the search window a look of its own, starting from the menus'.
    Customize,
    /// Drop the search window's own look: it follows the menus again.
    Follow,
    Theme(Theme),
    /// A swatch was clicked: open its editor, or close it if it is open.
    Open(ColorSlot),
    /// The colour picker moved.
    Color(Color),
    ResetColor(ColorSlot),
    /// The editor's Cancel (`true`: put the colour back) or Done.
    Close {
        revert: bool,
    },
    Padding(Option<f64>),
    Margin(Option<f64>),
    /// The combo's index: 0 inherits, then [`EDGES`].
    Edge(Option<usize>),
    /// The combo's index: 0 inherits, then [`SEPARATORS`].
    Separator(Option<usize>),
    PartOn(Part, bool),
    BorderSize(Option<f64>),
    BorderRound(Option<f64>),
    /// Index into [`DIRECTIONS`].
    Direction(Part, usize),
    ResetAll,
}

pub const EDGES: [EdgeRule; 3] = [EdgeRule::Idle, EdgeRule::Hover, EdgeRule::Always];
pub const SEPARATORS: [SeparatorSpan; 2] = [SeparatorSpan::AfterIcons, SeparatorSpan::Full];
pub const DIRECTIONS: [GradientDirection; 2] =
    [GradientDirection::Horizontal, GradientDirection::Vertical];

pub fn theme_of(settings: &Settings) -> Theme {
    settings.globals.theme.unwrap_or(Theme::Native)
}

/// Which look the screen is editing: the tray menus', or the search
/// window's own.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Scope {
    #[default]
    Menus,
    Search,
}

/// Whether the search window has no look of its own and so draws like the
/// tray menus. `false` when search is off: there is no window to style.
pub fn search_follows_menus(settings: &Settings) -> bool {
    settings
        .search
        .as_ref()
        .is_some_and(|search| search.theme.is_none() && !search.skin.overrides.any_set())
}

/// The theme `scope` draws with.
pub fn scoped_theme(settings: &Settings, scope: Scope) -> Theme {
    match (scope, &settings.search) {
        (Scope::Search, Some(search)) => search.theme.unwrap_or_else(|| theme_of(settings)),
        _ => theme_of(settings),
    }
}

/// The overrides `scope` draws with. A search window that follows the menus
/// draws with theirs; one with a look of its own, with what it takes from its
/// `<skin>`.
pub fn scoped_overrides(settings: &Settings, scope: Scope) -> ThemeOverrides {
    match (scope, &settings.search) {
        (Scope::Search, Some(search)) if !search_follows_menus(settings) => {
            search.skin.overrides.for_search()
        }
        _ => settings.skin.overrides,
    }
}

/// Whether the search window's `<skin>` sets what the window ignores: a file
/// written by hand. Nothing on the screen shows it, but Reset all clears it.
pub fn has_ignored(settings: &Settings, scope: Scope) -> bool {
    match (scope, &settings.search) {
        (Scope::Search, Some(search)) => !search.skin.overrides.ignored_by_search().is_empty(),
        _ => false,
    }
}

/// What y-pop draws: a Native theme with overrides is Auto.
pub fn effective_theme(theme: Theme, overrides: &ThemeOverrides) -> Theme {
    if theme == Theme::Native && overrides.any_set() {
        Theme::Auto
    } else {
        theme
    }
}

/// The design a theme gives before any override. Native has none of its own
/// (Windows draws it): overrides sit on Auto, so that is its base here.
pub fn base_design(theme: Theme) -> MenuDesign {
    let theme = if theme == Theme::Native {
        Theme::Auto
    } else {
        theme
    };
    skins::resolve_design(theme, ThemeOverrides::default()).expect("a themed menu has a design")
}

/// The design y-pop would use, `None` for plain Native.
pub fn resolved(theme: Theme, overrides: ThemeOverrides) -> Option<MenuDesign> {
    skins::resolve_design(theme, overrides)
}

/// What Auto is right now.
pub fn auto_theme() -> Theme {
    if y_core::theme::is_dark() {
        Theme::Dark
    } else {
        Theme::Default
    }
}

/// How many of the twelve overrides are set.
pub fn count(overrides: &ThemeOverrides) -> usize {
    [
        overrides.back.is_some(),
        overrides.text.is_some(),
        overrides.hover_back.is_some(),
        overrides.hover_text.is_some(),
        overrides.disabled_text.is_some(),
        overrides.padding.is_some(),
        overrides.margin.is_some(),
        overrides.edge.is_some(),
        overrides.separator.is_some(),
        overrides.border.is_some(),
        overrides.background_gradient.is_some(),
        overrides.icon_gradient.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count()
}

/// A number box's value as a pixel count, clamped; `None` for an empty box.
fn pixels(value: Option<f64>, max: f64) -> Option<u32> {
    value
        .filter(|value| value.is_finite())
        .map(|value| value.round().clamp(0.0, max) as u32)
}

/// A composite's starting point when it is switched on: what the theme gives,
/// so the menu does not jump and the user changes from what they see.
fn start_of(part: Part, base: &MenuDesign) -> PartValue {
    let gradient = |start, end| GradientOverride {
        start,
        end,
        direction: GradientDirection::Horizontal,
    };
    match part {
        Part::Border => PartValue::Border(BorderOverride {
            // A theme without a border (Dark) would start invisible.
            size: base.border.size.max(1),
            round: base.border.round,
            color: base.border.color,
        }),
        Part::Background => PartValue::Gradient(match base.fill {
            Fill::Gradient(g) => GradientOverride {
                start: g.start,
                end: g.end,
                direction: direction_of(g.direction),
            },
            Fill::Solid => gradient(base.selected.back, base.selected.back),
        }),
        Part::Icon => PartValue::Gradient(match base.icon_backdrop {
            Some(g) => GradientOverride {
                start: g.start,
                end: g.end,
                direction: direction_of(g.direction),
            },
            None => gradient(base.selected.back, base.selected.back),
        }),
    }
}

enum PartValue {
    Border(BorderOverride),
    Gradient(GradientOverride),
}

fn direction_of(direction: y_core::menu::GradientDirection) -> GradientDirection {
    match direction {
        y_core::menu::GradientDirection::Horizontal => GradientDirection::Horizontal,
        y_core::menu::GradientDirection::Vertical => GradientDirection::Vertical,
    }
}

/// Settles one message on `settings`. Whether the file now differs.
pub fn apply(settings: &mut Settings, ui: &mut SkinUi, message: SkinMsg) -> bool {
    let scope = ui.scope;
    match message {
        SkinMsg::Scope(chosen) => {
            ui.editing = None;
            return std::mem::replace(&mut ui.scope, chosen) != chosen;
        }
        SkinMsg::Customize => {
            let (theme, menus) = (theme_of(settings), settings.skin.overrides);
            let Some(search) = settings.search.as_mut() else {
                return false;
            };
            if search.theme.is_some() || search.skin.overrides.any_set() {
                return false;
            }
            // The look it had: the menus' overrides may be all that makes a
            // Native theme drawn, and the window does not take all of them.
            search.theme = Some(effective_theme(theme, &menus));
            search.skin.overrides = menus.for_search();
            return true;
        }
        SkinMsg::Follow => {
            ui.editing = None;
            let Some(search) = settings.search.as_mut() else {
                return false;
            };
            let changed = search.theme.is_some() || !search.skin.is_empty();
            search.theme = None;
            search.skin = y_pop::settings::Skin::default();
            return changed;
        }
        SkinMsg::Theme(chosen) => {
            return match (scope, settings.search.as_mut()) {
                (Scope::Search, Some(search)) => search.theme.replace(chosen) != Some(chosen),
                (Scope::Search, None) => false,
                (Scope::Menus, _) => {
                    if chosen == theme_of(settings) {
                        return false;
                    }
                    settings.globals.theme = Some(chosen);
                    true
                }
            };
        }
        _ => {}
    }

    let theme = scoped_theme(settings, scope);
    let base = base_design(theme);
    let overrides = match scope {
        Scope::Menus => &mut settings.skin.overrides,
        // Nothing to edit until the window has a look of its own.
        Scope::Search => match settings.search.as_mut() {
            Some(search) if search.theme.is_some() || search.skin.overrides.any_set() => {
                &mut search.skin.overrides
            }
            _ => return false,
        },
    };

    match message {
        SkinMsg::Scope(_) | SkinMsg::Customize | SkinMsg::Follow | SkinMsg::Theme(_) => false,

        SkinMsg::Open(slot) => {
            ui.editing = match ui.editing {
                Some(editing) if editing.slot == slot => None,
                _ => Some(Editing {
                    slot,
                    original: slot.get(overrides),
                }),
            };
            false
        }
        SkinMsg::Color(color) => {
            let Some(editing) = ui.editing else {
                return false;
            };
            // The picker reports the colour it was given when it opens;
            // that is not an edit, and must not turn an inherited colour
            // into an override. Alpha: menus are painted with GDI, which
            // has none, so the colour is stored opaque.
            let color = Color::rgb(color.r, color.g, color.b);
            if color == editing.slot.effective(overrides, &base) {
                return false;
            }
            editing.slot.set(overrides, Some(color))
        }
        SkinMsg::ResetColor(slot) => slot.set(overrides, None),
        SkinMsg::Close { revert } => {
            let Some(editing) = ui.editing.take() else {
                return false;
            };
            revert && editing.slot.set(overrides, editing.original)
        }

        SkinMsg::Padding(value) => set_if_changed(&mut overrides.padding, pixels(value, MAX_GAP)),
        SkinMsg::Margin(value) => set_if_changed(&mut overrides.margin, pixels(value, MAX_GAP)),
        SkinMsg::Edge(choice) => {
            let edge = match choice {
                Some(index) if index > 0 => EDGES.get(index - 1).copied(),
                _ => None,
            };
            set_if_changed(&mut overrides.edge, edge)
        }
        SkinMsg::Separator(choice) => {
            let span = match choice {
                Some(index) if index > 0 => SEPARATORS.get(index - 1).copied(),
                _ => None,
            };
            set_if_changed(&mut overrides.separator, span)
        }

        SkinMsg::PartOn(part, on) => {
            let present = match part {
                Part::Border => overrides.border.is_some(),
                Part::Background => overrides.background_gradient.is_some(),
                Part::Icon => overrides.icon_gradient.is_some(),
            };
            if present == on {
                return false;
            }
            if !on
                && ui
                    .editing
                    .is_some_and(|editing| editing.slot.part() == Some(part))
            {
                ui.editing = None;
            }
            match (part, on.then(|| start_of(part, &base))) {
                (Part::Border, Some(PartValue::Border(border))) => overrides.border = Some(border),
                (Part::Background, Some(PartValue::Gradient(g))) => {
                    overrides.background_gradient = Some(g)
                }
                (Part::Icon, Some(PartValue::Gradient(g))) => overrides.icon_gradient = Some(g),
                (Part::Border, _) => overrides.border = None,
                (Part::Background, _) => overrides.background_gradient = None,
                (Part::Icon, _) => overrides.icon_gradient = None,
            }
            true
        }
        SkinMsg::BorderSize(value) => {
            let (Some(border), Some(value)) = (&mut overrides.border, pixels(value, MAX_BORDER))
            else {
                return false;
            };
            std::mem::replace(&mut border.size, value) != value
        }
        SkinMsg::BorderRound(value) => {
            let (Some(border), Some(value)) = (&mut overrides.border, pixels(value, MAX_ROUND))
            else {
                return false;
            };
            std::mem::replace(&mut border.round, value) != value
        }
        SkinMsg::Direction(part, index) => {
            let gradient = match part {
                Part::Background => &mut overrides.background_gradient,
                Part::Icon => &mut overrides.icon_gradient,
                Part::Border => return false,
            };
            let (Some(gradient), Some(&direction)) = (gradient, DIRECTIONS.get(index)) else {
                return false;
            };
            std::mem::replace(&mut gradient.direction, direction) != direction
        }

        SkinMsg::ResetAll => {
            ui.editing = None;
            if !overrides.any_set() {
                return false;
            }
            *overrides = ThemeOverrides::default();
            true
        }
    }
}

fn set_if_changed<T: PartialEq>(slot: &mut Option<T>, value: Option<T>) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

impl ConfigApp {
    pub(crate) fn edit_skin(&mut self, message: SkinMsg) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        let label = match message {
            SkinMsg::ResetAll => Some("Reset the skin overrides"),
            SkinMsg::Follow => Some("Make search follow the menus' look"),
            _ => None,
        };
        let before = label.map(|_| settings.clone());
        if apply(settings, &mut self.skin_ui, message) {
            if let (Some(before), Some(label)) = (before, label) {
                self.undo = Some(Undo {
                    label: label.to_owned(),
                    settings: before,
                });
            }
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_pop::variables::Variable;

    fn settings(xml: &str) -> Settings {
        Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid")
    }

    fn dark() -> Settings {
        settings(r#"<y-pop theme="dark"><quit/></y-pop>"#)
    }

    fn run(settings: &mut Settings, ui: &mut SkinUi, message: SkinMsg) -> bool {
        apply(settings, ui, message)
    }

    fn with_search() -> Settings {
        settings(r#"<y-pop theme="flat"><skin back-color="ebebeb"/><search/><quit/></y-pop>"#)
    }

    fn in_search() -> SkinUi {
        SkinUi {
            scope: Scope::Search,
            ..SkinUi::default()
        }
    }

    #[test]
    fn search_follows_the_menus_until_customized() {
        let mut settings = with_search();
        let mut ui = in_search();
        assert!(search_follows_menus(&settings));
        // Nothing to edit yet.
        assert!(!run(&mut settings, &mut ui, SkinMsg::Padding(Some(3.0))));
        assert!(run(&mut settings, &mut ui, SkinMsg::Customize));
        let search = settings.search.as_ref().expect("search");
        assert_eq!(search.theme, Some(Theme::Flat));
        assert_eq!(search.skin.overrides, settings.skin.overrides);
        assert!(!run(&mut settings, &mut ui, SkinMsg::Customize), "once");
    }

    #[test]
    fn editing_the_search_look_leaves_the_menus_alone() {
        let mut settings = with_search();
        let mut ui = in_search();
        run(&mut settings, &mut ui, SkinMsg::Customize);
        assert!(run(&mut settings, &mut ui, SkinMsg::Theme(Theme::Graphite)));
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::Text));
        let red = Color::rgb(255, 0, 0);
        assert!(run(&mut settings, &mut ui, SkinMsg::Color(red)));

        assert_eq!(theme_of(&settings), Theme::Flat);
        assert_eq!(settings.skin.overrides.text, None);
        let search = settings.search.as_ref().expect("search");
        assert_eq!(search.theme, Some(Theme::Graphite));
        assert_eq!(search.skin.overrides.text, Some(red));

        // And it survives a save.
        let again = self::settings(&settings.to_xml());
        assert_eq!(again.search.as_ref().expect("search").theme, search.theme);
    }

    #[test]
    fn a_look_of_its_own_takes_only_what_the_window_uses_from_the_menus() {
        let mut settings = settings(
            r#"<y-pop><skin back-color="101010" padding="4" edge="always"/><search/><quit/></y-pop>"#,
        );
        assert!(run(&mut settings, &mut in_search(), SkinMsg::Customize));
        let search = settings.search.as_ref().expect("search");
        let back = Color::rgb(0x10, 0x10, 0x10);
        assert_eq!(search.skin.overrides.back, Some(back));
        assert_eq!(search.skin.overrides.padding, None);
        assert_eq!(search.skin.overrides.edge, None);
        assert_eq!(
            settings.skin.overrides.padding,
            Some(4),
            "the menus keep it"
        );
    }

    /// Native menus with a `<skin>` are drawn as Auto, and when the padding is
    /// all it has, there would be nothing left to keep the window drawn.
    #[test]
    fn a_look_of_its_own_stays_the_look_it_had() {
        let mut settings = settings(r#"<y-pop><skin padding="4"/><search/><quit/></y-pop>"#);
        assert!(run(&mut settings, &mut in_search(), SkinMsg::Customize));
        let search = settings.search.as_ref().expect("search");
        assert_eq!(search.theme, Some(Theme::Auto));
    }

    #[test]
    fn what_the_window_ignores_is_not_counted_but_reset_all_clears_it() {
        let mut settings = settings(
            r#"<y-pop><search theme="dark"><skin padding="4" text-color="ffffff"/></search><quit/></y-pop>"#,
        );
        let mut ui = in_search();
        let overrides = scoped_overrides(&settings, Scope::Search);
        assert_eq!(count(&overrides), 1);
        assert!(has_ignored(&settings, Scope::Search));

        assert!(run(&mut settings, &mut ui, SkinMsg::ResetAll));
        assert!(!has_ignored(&settings, Scope::Search));
        let search = settings.search.as_ref().expect("search");
        assert!(!search.skin.overrides.any_set());
    }

    #[test]
    fn following_the_menus_drops_the_search_look() {
        let mut settings = with_search();
        let mut ui = in_search();
        run(&mut settings, &mut ui, SkinMsg::Customize);
        assert!(run(&mut settings, &mut ui, SkinMsg::Follow));
        assert!(search_follows_menus(&settings));
        assert!(!run(&mut settings, &mut ui, SkinMsg::Follow));
    }

    #[test]
    fn nothing_changes_without_a_search_element() {
        let mut settings = dark();
        let mut ui = in_search();
        for message in [
            SkinMsg::Customize,
            SkinMsg::Theme(Theme::Flat),
            SkinMsg::Follow,
        ] {
            assert!(!run(&mut settings, &mut ui, message));
        }
        assert_eq!(theme_of(&settings), Theme::Dark);
    }

    /// The loader and the saver each have their own list of names.
    #[test]
    fn every_theme_is_written_and_read_back() {
        for theme in THEMES {
            let mut edited = dark();
            run(&mut edited, &mut SkinUi::default(), SkinMsg::Theme(theme));
            assert_eq!(theme_of(&settings(&edited.to_xml())), theme);
            assert!(resolved(theme, ThemeOverrides::default()).is_some() || theme == Theme::Native);
        }
    }

    #[test]
    fn choosing_the_theme_it_already_has_changes_nothing() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        assert!(!run(&mut settings, &mut ui, SkinMsg::Theme(Theme::Dark)));
        assert!(run(&mut settings, &mut ui, SkinMsg::Theme(Theme::Flat)));
        assert_eq!(settings.globals.theme, Some(Theme::Flat));
    }

    #[test]
    fn no_theme_in_the_file_is_native_and_choosing_native_writes_nothing() {
        let mut settings = settings("<y-pop><quit/></y-pop>");
        let mut ui = SkinUi::default();
        assert_eq!(theme_of(&settings), Theme::Native);
        assert!(!run(&mut settings, &mut ui, SkinMsg::Theme(Theme::Native)));
        assert_eq!(settings.globals.theme, None);
    }

    #[test]
    fn a_native_theme_with_overrides_is_drawn_as_auto() {
        let mut overrides = ThemeOverrides::default();
        assert_eq!(effective_theme(Theme::Native, &overrides), Theme::Native);
        overrides.padding = Some(1);
        assert_eq!(effective_theme(Theme::Native, &overrides), Theme::Auto);
        assert_eq!(effective_theme(Theme::Flat, &overrides), Theme::Flat);
    }

    #[test]
    fn editing_a_colour_overrides_it_and_cancel_puts_it_back() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::HoverBack));
        let red = Color::rgb(255, 0, 0);
        assert!(run(&mut settings, &mut ui, SkinMsg::Color(red)));
        assert_eq!(settings.skin.overrides.hover_back, Some(red));

        assert!(run(&mut settings, &mut ui, SkinMsg::Close { revert: true }));
        assert_eq!(settings.skin.overrides.hover_back, None);
        assert!(ui.editing.is_none());
    }

    #[test]
    fn done_keeps_the_colour() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::Text));
        run(&mut settings, &mut ui, SkinMsg::Color(Color::rgb(1, 2, 3)));
        assert!(!run(
            &mut settings,
            &mut ui,
            SkinMsg::Close { revert: false }
        ));
        assert_eq!(settings.skin.overrides.text, Some(Color::rgb(1, 2, 3)));
    }

    #[test]
    fn the_picker_reporting_the_inherited_colour_is_not_an_edit() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::Back));
        let inherited = ColorSlot::Back.of_design(&base_design(Theme::Dark));
        assert!(!run(&mut settings, &mut ui, SkinMsg::Color(inherited)));
        assert_eq!(settings.skin.overrides.back, None);
    }

    #[test]
    fn a_colour_is_stored_opaque() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::Back));
        run(
            &mut settings,
            &mut ui,
            SkinMsg::Color(Color::rgba(9, 9, 9, 10)),
        );
        assert_eq!(settings.skin.overrides.back, Some(Color::rgb(9, 9, 9)));
    }

    #[test]
    fn clicking_the_open_swatch_closes_its_editor() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::Back));
        assert!(ui.editing.is_some());
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::Back));
        assert!(ui.editing.is_none());
    }

    #[test]
    fn resetting_a_colour_makes_it_inherit_again() {
        let mut settings =
            settings(r#"<y-pop theme="dark"><skin back-color="101010"/><quit/></y-pop>"#);
        let mut ui = SkinUi::default();
        assert!(run(
            &mut settings,
            &mut ui,
            SkinMsg::ResetColor(ColorSlot::Back)
        ));
        assert_eq!(settings.skin.overrides.back, None);
        assert!(!run(
            &mut settings,
            &mut ui,
            SkinMsg::ResetColor(ColorSlot::Back)
        ));
    }

    #[test]
    fn spacing_is_clamped_and_an_empty_box_inherits() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        assert!(run(&mut settings, &mut ui, SkinMsg::Padding(Some(500.0))));
        assert_eq!(settings.skin.overrides.padding, Some(MAX_GAP as u32));
        assert!(!run(
            &mut settings,
            &mut ui,
            SkinMsg::Padding(Some(MAX_GAP))
        ));
        assert!(run(&mut settings, &mut ui, SkinMsg::Padding(None)));
        assert_eq!(settings.skin.overrides.padding, None);
        assert!(!run(&mut settings, &mut ui, SkinMsg::Margin(None)));
    }

    #[test]
    fn the_separator_combo_has_inherit_first() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        assert!(!run(&mut settings, &mut ui, SkinMsg::Separator(Some(0))));
        assert!(run(&mut settings, &mut ui, SkinMsg::Separator(Some(2))));
        assert_eq!(settings.skin.overrides.separator, Some(SeparatorSpan::Full));
        let design = resolved(Theme::Dark, settings.skin.overrides).expect("themed");
        assert_eq!(design.separator, y_core::menu::SeparatorSpan::Full);
        assert!(run(&mut settings, &mut ui, SkinMsg::Separator(Some(0))));
        assert_eq!(settings.skin.overrides.separator, None);
    }

    #[test]
    fn the_edge_combo_has_inherit_first() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        assert!(!run(&mut settings, &mut ui, SkinMsg::Edge(Some(0))));
        assert!(run(&mut settings, &mut ui, SkinMsg::Edge(Some(2))));
        assert_eq!(settings.skin.overrides.edge, Some(EdgeRule::Hover));
        assert!(run(&mut settings, &mut ui, SkinMsg::Edge(Some(0))));
        assert_eq!(settings.skin.overrides.edge, None);
    }

    #[test]
    fn a_border_starts_from_the_themes_and_is_never_invisible() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        assert!(run(
            &mut settings,
            &mut ui,
            SkinMsg::PartOn(Part::Border, true)
        ));
        let border = settings.skin.overrides.border.expect("on");
        assert_eq!(
            border.size, 1,
            "Dark has none; a border you switch on shows"
        );

        assert!(run(&mut settings, &mut ui, SkinMsg::BorderSize(Some(3.0))));
        assert!(run(&mut settings, &mut ui, SkinMsg::BorderRound(Some(8.0))));
        let border = settings.skin.overrides.border.expect("on");
        assert_eq!((border.size, border.round), (3, 8));

        assert!(run(
            &mut settings,
            &mut ui,
            SkinMsg::PartOn(Part::Border, false)
        ));
        assert_eq!(settings.skin.overrides.border, None);
        assert!(!run(&mut settings, &mut ui, SkinMsg::BorderSize(Some(2.0))));
    }

    #[test]
    fn a_gradient_starts_from_the_themes_fill_and_takes_a_direction() {
        let mut settings = settings(r#"<y-pop theme="default"><quit/></y-pop>"#);
        let mut ui = SkinUi::default();
        assert!(run(
            &mut settings,
            &mut ui,
            SkinMsg::PartOn(Part::Background, true)
        ));
        let gradient = settings.skin.overrides.background_gradient.expect("on");
        assert_eq!(gradient.direction, GradientDirection::Vertical);
        assert_eq!(gradient.start, Color::rgb(0xff, 0xff, 0xff));

        assert!(run(
            &mut settings,
            &mut ui,
            SkinMsg::Direction(Part::Background, 0)
        ));
        assert_eq!(
            settings
                .skin
                .overrides
                .background_gradient
                .map(|g| g.direction),
            Some(GradientDirection::Horizontal)
        );
        // The icon gradient is its own override.
        assert_eq!(settings.skin.overrides.icon_gradient, None);
    }

    #[test]
    fn a_flat_theme_starts_a_gradient_as_its_hover_colour() {
        let mut settings = settings(r#"<y-pop theme="flat"><quit/></y-pop>"#);
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::PartOn(Part::Icon, true));
        let gradient = settings.skin.overrides.icon_gradient.expect("on");
        assert_eq!(gradient.start, gradient.end);
        assert_eq!(gradient.start, Color::rgb(0xed, 0xf4, 0xfc));
    }

    #[test]
    fn colours_inside_a_composite_need_it_on_and_follow_it_off() {
        let mut settings = dark();
        let mut ui = SkinUi::default();
        run(&mut settings, &mut ui, SkinMsg::Open(ColorSlot::IconStart));
        assert!(!run(
            &mut settings,
            &mut ui,
            SkinMsg::Color(Color::rgb(1, 1, 1))
        ));

        run(&mut settings, &mut ui, SkinMsg::PartOn(Part::Icon, true));
        assert!(run(
            &mut settings,
            &mut ui,
            SkinMsg::Color(Color::rgb(1, 1, 1))
        ));
        assert_eq!(
            settings.skin.overrides.icon_gradient.map(|g| g.start),
            Some(Color::rgb(1, 1, 1))
        );

        run(&mut settings, &mut ui, SkinMsg::PartOn(Part::Icon, false));
        assert!(ui.editing.is_none(), "its editor closes with it");
    }

    #[test]
    fn reset_all_clears_the_overrides_and_keeps_the_notes() {
        let mut settings = settings(
            r#"<y-pop theme="dark"><skin back-color="101010" padding="2"><!-- keep --></skin><quit/></y-pop>"#,
        );
        let mut ui = SkinUi::default();
        assert_eq!(count(&settings.skin.overrides), 2);
        assert!(run(&mut settings, &mut ui, SkinMsg::ResetAll));
        assert_eq!(count(&settings.skin.overrides), 0);
        assert_eq!(settings.skin.notes.len(), 1);
        assert!(!run(&mut settings, &mut ui, SkinMsg::ResetAll));
    }

    #[test]
    fn an_edit_through_the_app_marks_it_dirty_and_reset_all_offers_undo() {
        let mut app = crate::test_app(settings(
            r#"<y-pop theme="dark"><skin padding="2"/><quit/></y-pop>"#,
        ));
        app.edit_skin(SkinMsg::Margin(Some(3.0)));
        assert!(app.dirty);
        assert!(app.undo.is_none(), "an ordinary edit is not undoable");

        app.edit_skin(SkinMsg::ResetAll);
        let undo = app.undo.take().expect("reset all can be undone");
        let before = &undo.settings.skin.overrides;
        assert_eq!((before.padding, before.margin), (Some(2), Some(3)));
        assert_eq!(
            count(&app.settings.as_ref().expect("loaded").skin.overrides),
            0
        );
    }

    #[test]
    fn the_readout_counts_every_kind_of_override() {
        let overrides = ThemeOverrides {
            back: Some(Color::rgb(0, 0, 0)),
            padding: Some(1),
            edge: Some(EdgeRule::Idle),
            border: Some(BorderOverride {
                size: 1,
                round: 0,
                color: Color::rgb(0, 0, 0),
            }),
            ..ThemeOverrides::default()
        };
        assert_eq!(count(&overrides), 4);
    }
}
