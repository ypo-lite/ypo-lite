//! `theme="..."` / `<skin>` on `<y-pop>` — the config-level side of `y-pop`'s
//! theme system: the data model (this module's types) and the XML parsing
//! that fills it in ([`Reader`]'s methods below, split out of
//! `crate::config` into their own `impl` block here).
//!
//! Pure data, no Win32 despite reusing [`y_core::overlay::Color`] — that
//! type itself makes no Win32 call, only its `colorref` conversion does, and
//! that only ever runs from `y_core::menu` at paint time. Resolving a
//! [`Theme`] + [`ThemeOverrides`] pair into an actual
//! `y_core::menu::MenuDesign` (including [`Theme::Auto`] against the OS's
//! live light/dark setting, and PortAL's own three built-in skin tables) is
//! the `y-pop` binary's own concern, not this crate's — see
//! `y-pop/src/skins.rs::resolve_design`, the sole place this module's types
//! are consumed downstream of `crate::config::load`.

use crate::config::Reader;
use y_core::overlay::Color;
use y_core::xml::Element;

/// `theme="native" | "default" | "flat" | "dark" | "auto"` on `<y-pop>` —
/// `default`/`flat`/`dark` are PortAL's own three built-in skin names, not
/// `y-pop`'s. Not a "light"/"dark" pair either: an earlier version of this
/// enum had `Light`/`Dark` instead, values `y-pop` made up itself rather
/// than the three PortAL actually ships, which is the gap this corrects.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    /// Plain OS chrome, no owner-draw — the zero-surprise default: a config
    /// that never mentions `theme` keeps looking exactly like it always
    /// has.
    #[default]
    Native,
    /// PortAL's `default` skin — warm, light, a subtle gradient hover.
    Default,
    /// PortAL's `flat` skin — light, no gradient, a flat hover fill.
    Flat,
    /// PortAL's `dark`/`black` skin.
    Dark,
    /// Follows `AppsUseLightTheme`, resolved live each time a popup opens
    /// (not cached at load) — same reasoning `y-snap`'s own `"auto"` uses.
    /// Resolves to [`Theme::Default`] or [`Theme::Dark`] — `Flat` has no
    /// natural dark counterpart in PortAL either, so it's only reachable by
    /// naming it explicitly.
    Auto,
}

/// Overrides on top of whichever base [`Theme`] resolves to, read from
/// `<y-pop>`'s `<skin>` child element — PortAL's own answer to "I like dark
/// but want my own accent colour", generalized past just colour once
/// `y-pop` gained a border/gradient/margin design to override too. `None`
/// (per field, or the whole `border`/`background_gradient`) means "keep the
/// theme's own value there" — a `<skin>` that sets only `hover-back-color`
/// still gets every other slot from `theme`. Merged onto a resolved
/// `y_core::menu::MenuDesign` in `y-pop/src/skins.rs`, the same split
/// [`Theme`] itself draws between "what the config says" and "what it's
/// resolved to".
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ThemeOverrides {
    pub back: Option<Color>,
    pub text: Option<Color>,
    pub hover_back: Option<Color>,
    pub hover_text: Option<Color>,
    pub disabled_text: Option<Color>,
    /// Gap **between** rows — see [`y_core::menu::MenuDesign::padding`]'s
    /// doc comment for why this isn't called `margin` (CSS's name for it).
    pub padding: Option<u32>,
    /// Gap **within** a row, between its border and its own content — see
    /// [`y_core::menu::MenuDesign::margin`]'s doc comment for why this
    /// isn't called `padding` (CSS's name for it).
    pub margin: Option<u32>,
    /// `<border size="" round="" color=""/>`, a child of `<skin>` — every
    /// slot on this `Some` is itself concrete (not a second layer of
    /// per-field `Option`s): setting `<border>` at all means setting a
    /// complete border, same "half-set is a mistake, not a feature" stance
    /// [`Reader`]'s parser takes for it.
    pub border: Option<BorderOverride>,
    /// `<background-gradient start="" end="" direction=""/>`, a child of
    /// `<skin>` — same "fully set or not present" shape as `border`.
    pub background_gradient: Option<GradientOverride>,
    /// `edge="idle"|"hover"|"always"` — see [`EdgeRule`]'s own doc comment
    /// for why this uses those three names rather than PortAL's own
    /// `edge="true"|"sel"|"unsel"`.
    pub edge: Option<EdgeRule>,
    /// `<icon-gradient start="" end="" direction=""/>`, a child of `<skin>`
    /// — PortAL's own `<icon color="a:b" direction="h|v">` (`xmlconfig.cpp`'s
    /// `loadMenuSkin`), reshaped to match `<background-gradient>`'s own
    /// "fully set or not present" contract rather than PortAL's: its own
    /// single-colour fallback (`<icon color="X"/>`, no `:`) is provably
    /// buggy there — `getColorPair`'s `wcstok_s` still returns a non-`NULL`
    /// pair for an un-delimited string, `background` defaulting to
    /// `getColor(NULL) == 0` (black) rather than reaching the single-colour
    /// branch the surrounding code clearly intends — so it isn't carried
    /// forward; a flat colour here is written as `start`/`end` both equal,
    /// the same convention [`GradientOverride`] and [`y_core::menu::Fill::
    /// Gradient`] already use.
    pub icon_gradient: Option<GradientOverride>,
}

/// `edge="idle"|"hover"|"always"` on `<skin>` — mirrors
/// [`y_core::menu::EdgeRule`] rather than reusing it directly, the same
/// "config-level type, converted downstream" split every other `<skin>`
/// setting here follows (`GradientDirection` next to `y_core::menu::
/// GradientDirection`, `BorderOverride`/`GradientOverride` next to `y_core::
/// menu::Border`/`Gradient`) — converted in `y-pop/src/skins.rs::
/// resolve_design`, the one place a `y-pop.xml` concept turns into a
/// `y_core::menu` one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeRule {
    Idle,
    Hover,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderOverride {
    pub size: u32,
    pub round: u32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradientOverride {
    pub start: Color,
    pub end: Color,
    pub direction: GradientDirection,
}

/// `direction="h"`/`"v"` on `<background-gradient>` — mirrors
/// `y_core::menu::GradientDirection` rather than reusing it directly, the
/// same reasoning [`Theme`] itself is a config-level type distinct from
/// anything `y_core::menu` defines: this module stays pure data, no Win32,
/// and the conversion happens where every other config→`y_core::menu`
/// conversion already does, `y-pop/src/skins.rs::resolve_design`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientDirection {
    Horizontal,
    Vertical,
}

impl ThemeOverrides {
    /// Whether any slot was actually set — a config that gives a `<skin>`
    /// attribute/child but leaves `theme` unset (or `"native"`) still wants
    /// a themed, owner-drawn popup to paint it onto; see
    /// `y-pop/src/skins.rs::resolve_design`, the one place this is checked.
    pub fn any_set(&self) -> bool {
        self.back.is_some()
            || self.text.is_some()
            || self.hover_back.is_some()
            || self.hover_text.is_some()
            || self.disabled_text.is_some()
            || self.padding.is_some()
            || self.margin.is_some()
            || self.border.is_some()
            || self.background_gradient.is_some()
            || self.edge.is_some()
            || self.icon_gradient.is_some()
    }
}

impl Reader<'_> {
    /// `theme="native" | "default" | "flat" | "dark" | "auto"` on
    /// `<y-pop>` — `default`/`flat`/`dark` are PortAL's own three built-in
    /// skin names, not `y-pop`'s (see [`Theme`]'s own doc comment). `None`
    /// if the attribute is absent, same "not set" vs. "set" distinction
    /// [`Reader::popup_mode`](crate::config::Reader) makes, for the same
    /// reason: `Defaults::default`'s own choice (`Theme::Native`) is what a
    /// config omitting `theme` gets, not a second, possibly-diverging copy
    /// of that default written here too.
    pub(crate) fn theme(&mut self, element: Element<'_>) -> Option<Theme> {
        let value = element.attr("theme")?;
        match value {
            "native" => Some(Theme::Native),
            "default" => Some(Theme::Default),
            "flat" => Some(Theme::Flat),
            "dark" | "black" => Some(Theme::Dark),
            "auto" => Some(Theme::Auto),
            other => {
                self.warn(element, format!("unknown theme {other:?}"));
                None
            }
        }
    }

    /// `<skin>`, a child of `<y-pop>` — a user-editable design on top of
    /// (or instead of) the built-in `theme=` choice. A dedicated element
    /// rather than a flat `*-color`/`border-*`/`background-gradient-*`
    /// attribute soup on `<y-pop>` itself (an earlier design this
    /// superseded — see `docs/y-pop-plan.md`'s "Theme system v2" section):
    /// every theme parameter reads in one place instead of scattered root
    /// attributes, the same "give a whole config domain its own element"
    /// convention `Reader::read_variables`'s `<variables>` already follows.
    /// Absent entirely, this returns [`ThemeOverrides::default`] —
    /// unchanged behaviour from before `<skin>` existed.
    pub(crate) fn theme_overrides(&mut self, root: Element<'_>) -> ThemeOverrides {
        let Some(skin) = root.child("skin") else {
            return ThemeOverrides::default();
        };
        for extra in root.children_named("skin").skip(1) {
            self.warn(extra, "a second <skin> is ignored — only the first one is read");
        }
        self.check(
            skin,
            &[
                "back-color",
                "text-color",
                "hover-back-color",
                "hover-text-color",
                "disabled-text-color",
                "padding",
                "margin",
                "edge",
            ],
        );

        let mut overrides = ThemeOverrides::default();
        for (name, slot) in [
            ("back-color", &mut overrides.back),
            ("text-color", &mut overrides.text),
            ("hover-back-color", &mut overrides.hover_back),
            ("hover-text-color", &mut overrides.hover_text),
            ("disabled-text-color", &mut overrides.disabled_text),
        ] {
            let Some(text) = skin.attr(name) else {
                continue;
            };
            match Color::parse(text) {
                Some(color) => *slot = Some(color),
                // Borrowing `overrides` mutably here, so the warning cannot
                // go through `self.warn` until the loop is over.
                None => self
                    .warnings
                    .push(format!("line {}: {text:?} is not a colour", skin.line())),
            }
        }
        // `padding` (gap *between* rows) and `margin` (gap *within* a row,
        // between its border and its content) — two independent knobs, see
        // `y_core::menu::MenuDesign`'s own doc comments on both for why
        // neither is named the way CSS names it.
        for (name, slot) in [
            ("padding", &mut overrides.padding),
            ("margin", &mut overrides.margin),
        ] {
            let Some(text) = skin.attr(name) else {
                continue;
            };
            match skin.attr_u32(name) {
                Some(value) => *slot = Some(value),
                None => self
                    .warnings
                    .push(format!("line {}: {text:?} is not a number", skin.line())),
            }
        }

        overrides.edge = match skin.attr("edge") {
            None => None,
            Some("idle") => Some(EdgeRule::Idle),
            Some("hover") => Some(EdgeRule::Hover),
            Some("always") => Some(EdgeRule::Always),
            Some(other) => {
                self.warn(skin, format!("unknown edge {other:?}"));
                None
            }
        };

        overrides.border = self.border_override(skin);
        overrides.background_gradient = self.gradient_override(skin, "background-gradient");
        overrides.icon_gradient = self.gradient_override(skin, "icon-gradient");
        overrides
    }

    /// `<border size="" round="" color=""/>`, a child of `<skin>` — fully
    /// set or not present at all, same stance the flat design already took
    /// for a gradient's `start`/`end` (see [`Reader::gradient_override`]):
    /// a `<border>` a user bothered to write clearly wants a real border,
    /// so a missing/unparseable attribute warns and the whole element is
    /// dropped rather than silently defaulting that one slot to `0`/black.
    fn border_override(&mut self, skin: Element<'_>) -> Option<BorderOverride> {
        let border = skin.child("border")?;
        self.check(border, &["size", "round", "color"]);

        let size = self.numeric_attr(border, "size")?;
        let round = self.numeric_attr(border, "round")?;
        let Some(text) = border.attr("color") else {
            self.warn(border, "<border> needs a color attribute");
            return None;
        };
        let Some(color) = Color::parse(text) else {
            self.warn(border, format!("{text:?} is not a colour"));
            return None;
        };
        Some(BorderOverride { size, round, color })
    }

    /// `<background-gradient start="" end="" direction=""/>` or
    /// `<icon-gradient .../>` (same shape, different child name — see
    /// [`ThemeOverrides::icon_gradient`]'s own doc comment for why the
    /// latter doesn't reuse PortAL's own `<icon color="a:b">`), a child of
    /// `<skin>` — named explicitly (not PortAL's bare `<background>`,
    /// which [[portal-analysis]] documents as meaning three different
    /// things depending on context) since this element is always exactly a
    /// gradient. `start`/`end` are both required — a gradient with only one
    /// colour set is a config mistake, not a flat-colour fallback (see
    /// [[portal-analysis]]'s note on PortAL's own inconsistent handling of
    /// this exact case).
    fn gradient_override(&mut self, skin: Element<'_>, child_name: &str) -> Option<GradientOverride> {
        let gradient = skin.child(child_name)?;
        self.check(gradient, &["start", "end", "direction"]);

        let start = self.color_attr(gradient, "start")?;
        let end = self.color_attr(gradient, "end")?;
        let direction = match gradient.attr("direction") {
            None | Some("h") => GradientDirection::Horizontal,
            Some("v") => GradientDirection::Vertical,
            Some(other) => {
                self.warn(gradient, format!("unknown gradient direction {other:?}"));
                GradientDirection::Horizontal
            }
        };
        Some(GradientOverride {
            start,
            end,
            direction,
        })
    }

    /// A required `u32` attribute — warns and returns `None` (dropping the
    /// whole enclosing element, same as a missing attribute) whether the
    /// attribute is absent or just not a number, since `Element::attr_u32`
    /// itself doesn't distinguish the two.
    fn numeric_attr(&mut self, element: Element<'_>, name: &str) -> Option<u32> {
        match element.attr_u32(name) {
            Some(value) => Some(value),
            None if element.attr(name).is_some() => {
                self.warn(element, format!("{name} is not a number"));
                None
            }
            None => {
                self.warn(element, format!("<{}> needs a {name} attribute", element.name()));
                None
            }
        }
    }

    /// A required [`Color`] attribute — same "warn and drop the whole
    /// element" shape as [`Reader::numeric_attr`].
    fn color_attr(&mut self, element: Element<'_>, name: &str) -> Option<Color> {
        match element.attr(name) {
            Some(text) => match Color::parse(text) {
                Some(color) => Some(color),
                None => {
                    self.warn(element, format!("{text:?} is not a colour"));
                    None
                }
            },
            None => {
                self.warn(element, format!("<{}> needs a {name} attribute", element.name()));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_str;
    use crate::variables::Variable;

    fn builtins() -> Vec<Variable> {
        vec![Variable {
            name: "win".into(),
            value: r"C:\Windows".into(),
        }]
    }

    #[test]
    fn theme_defaults_to_native() {
        let tree = load_str("<y-pop><quit/></y-pop>", &mut builtins()).expect("valid");
        assert_eq!(tree.defaults.theme, Theme::Native);
    }

    #[test]
    fn theme_is_read_when_given() {
        for (value, expected) in [
            ("native", Theme::Native),
            ("default", Theme::Default),
            ("flat", Theme::Flat),
            ("dark", Theme::Dark),
            ("black", Theme::Dark),
            ("auto", Theme::Auto),
        ] {
            let xml = format!(r#"<y-pop theme="{value}"><quit/></y-pop>"#);
            let tree = load_str(&xml, &mut builtins()).expect("valid");
            assert_eq!(tree.defaults.theme, expected, "theme={value:?}");
            assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        }
    }

    #[test]
    fn an_unknown_theme_warns_and_falls_back_to_native() {
        let tree = load_str(r#"<y-pop theme="neon"><quit/></y-pop>"#, &mut builtins())
            .expect("valid");

        assert_eq!(tree.defaults.theme, Theme::Native);
        assert!(
            tree.warnings.iter().any(|warning| warning.contains("theme")),
            "{:?}",
            tree.warnings
        );
    }

    #[test]
    fn theme_color_overrides_are_read() {
        let tree = load_str(
            r#"<y-pop><skin back-color="101010" hover-back-color="ff8800"/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert_eq!(
            tree.defaults.theme_overrides.back,
            Some(Color::rgb(0x10, 0x10, 0x10))
        );
        assert_eq!(
            tree.defaults.theme_overrides.hover_back,
            Some(Color::rgb(0xff, 0x88, 0x00))
        );
        assert_eq!(tree.defaults.theme_overrides.text, None);
    }

    #[test]
    fn an_invalid_theme_color_warns_and_is_left_unset() {
        let tree = load_str(
            r#"<y-pop><skin text-color="not-a-colour"/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert_eq!(tree.defaults.theme_overrides.text, None);
        assert!(
            tree.warnings
                .iter()
                .any(|warning| warning.contains("not-a-colour")),
            "{:?}",
            tree.warnings
        );
    }

    #[test]
    fn a_second_skin_is_ignored_with_a_warning() {
        let tree = load_str(
            r#"<y-pop><skin back-color="101010"/><skin back-color="202020"/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert_eq!(
            tree.defaults.theme_overrides.back,
            Some(Color::rgb(0x10, 0x10, 0x10)),
            "only the first <skin> is read"
        );
        assert!(
            tree.warnings.iter().any(|warning| warning.contains("second <skin>")),
            "{:?}",
            tree.warnings
        );
    }

    #[test]
    fn a_border_element_sets_size_round_and_color_together() {
        let tree = load_str(
            r#"<y-pop><skin><border size="2" round="10" color="3a3a3a"/></skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let border = tree.defaults.theme_overrides.border.expect("a <border> was given");
        assert_eq!(border.size, 2);
        assert_eq!(border.round, 10);
        assert_eq!(border.color, Color::rgb(0x3a, 0x3a, 0x3a));
    }

    #[test]
    fn a_border_missing_an_attribute_warns_and_is_dropped_entirely() {
        let tree = load_str(
            r#"<y-pop><skin><border size="2" round="10"/></skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert_eq!(
            tree.defaults.theme_overrides.border, None,
            "a half-set border is dropped, not defaulted"
        );
        assert!(
            tree.warnings.iter().any(|warning| warning.contains("color")),
            "{:?}",
            tree.warnings
        );
    }

    #[test]
    fn a_background_gradient_needs_both_start_and_end() {
        let tree = load_str(
            r#"<y-pop><skin><background-gradient start="000000" end="ffffff" direction="v"/></skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let gradient = tree
            .defaults
            .theme_overrides
            .background_gradient
            .expect("a <background-gradient> was given");
        assert_eq!(gradient.start, Color::rgb(0, 0, 0));
        assert_eq!(gradient.end, Color::rgb(255, 255, 255));
        assert_eq!(gradient.direction, GradientDirection::Vertical);

        let missing_end = load_str(
            r#"<y-pop><skin><background-gradient start="000000"/></skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(
            missing_end.defaults.theme_overrides.background_gradient, None,
            "a half-set gradient is dropped, not defaulted"
        );
    }

    #[test]
    fn margin_is_read_off_skin_directly() {
        let tree = load_str(
            r#"<y-pop><skin margin="5"/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert_eq!(tree.defaults.theme_overrides.margin, Some(5));
    }

    #[test]
    fn edge_is_read_off_skin_directly() {
        for (value, expected) in [
            ("idle", EdgeRule::Idle),
            ("hover", EdgeRule::Hover),
            ("always", EdgeRule::Always),
        ] {
            let xml = format!(r#"<y-pop><skin edge="{value}"/><quit/></y-pop>"#);
            let tree = load_str(&xml, &mut builtins()).expect("valid");
            assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
            assert_eq!(tree.defaults.theme_overrides.edge, Some(expected), "edge={value:?}");
        }
    }

    #[test]
    fn an_unknown_edge_value_warns_and_is_left_unset() {
        let tree = load_str(
            r#"<y-pop><skin edge="sel"/><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert_eq!(
            tree.defaults.theme_overrides.edge, None,
            "PortAL's own \"sel\"/\"unsel\"/\"true\" values are not carried over — see EdgeRule's own doc comment"
        );
        assert!(
            tree.warnings.iter().any(|warning| warning.contains("edge")),
            "{:?}",
            tree.warnings
        );
    }

    #[test]
    fn an_icon_gradient_needs_both_start_and_end() {
        let tree = load_str(
            r#"<y-pop><skin><icon-gradient start="000000" end="ffffff" direction="v"/></skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let gradient = tree
            .defaults
            .theme_overrides
            .icon_gradient
            .expect("an <icon-gradient> was given");
        assert_eq!(gradient.start, Color::rgb(0, 0, 0));
        assert_eq!(gradient.end, Color::rgb(255, 255, 255));
        assert_eq!(gradient.direction, GradientDirection::Vertical);

        let missing_end = load_str(
            r#"<y-pop><skin><icon-gradient start="000000"/></skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");
        assert_eq!(
            missing_end.defaults.theme_overrides.icon_gradient, None,
            "a half-set gradient is dropped, not defaulted"
        );
    }

    /// `<background-gradient>` and `<icon-gradient>` are independent slots
    /// on the same `<skin>` — one being set must not be read as, or drop,
    /// the other.
    #[test]
    fn background_gradient_and_icon_gradient_are_independent() {
        let tree = load_str(
            r#"<y-pop><skin>
                <background-gradient start="000000" end="ffffff"/>
                <icon-gradient start="ff0000" end="00ff00"/>
            </skin><quit/></y-pop>"#,
            &mut builtins(),
        )
        .expect("valid");

        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        let background = tree.defaults.theme_overrides.background_gradient.expect("set");
        let icon = tree.defaults.theme_overrides.icon_gradient.expect("set");
        assert_eq!(background.start, Color::rgb(0, 0, 0));
        assert_eq!(icon.start, Color::rgb(255, 0, 0));
        assert_eq!(icon.end, Color::rgb(0, 255, 0));
    }
}
