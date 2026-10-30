//! Turns the config's [`Theme`] + [`ThemeOverrides`] (`y_pop::theme` — the
//! pure-data, no-Win32 side of `y-pop`'s theme system) into an actual
//! `Option<y_core::menu::MenuDesign>` — `None` for [`Theme::Native`] (plain
//! OS chrome, `PopupMenu::new()`), `Some` for `Default`/`Flat`/`Dark`/`Auto`
//! (owner-drawn, `PopupMenu::themed`). [`resolve_design`] is called once per
//! popup open, from `menu::build`, and the result carried on `menu::Session`
//! from there so every submenu themes consistently with the root.
//!
//! [`DEFAULT`]/[`FLAT`]/[`DARK`] are PortAL's own three built-in skins,
//! reproduced here (not `y_core::menu`'s to define — `MenuDesign` only
//! defines the shape) from `menu.cpp`'s actual literals (`design1`/
//! `design2`/`design3` at module scope), not from [[portal-analysis]]'s own
//! table — that table's hex values (`#f9d996` and friends) turned out to be
//! a misreading, corrected here: see [`BLUE_ACCENT`]'s own doc comment for
//! the exact bug. `padding`/`margin` are `0` on all three: PortAL has
//! neither concept (see `y_core::menu::MenuDesign`'s own doc comments on
//! both fields), so reproducing its skins faithfully means not inventing
//! extra spacing it never had either — a config can still opt into either
//! via `<skin padding="" margin="">`.
//!
//! PortAL leaves `base`/`selected` `NULL` (→ system menu/highlight colours)
//! on every slot but `flat`'s `selected` — [[portal-analysis]]'s own note on
//! this ("`y-pop`'s `theme="auto"` already tracks the live OS light/dark
//! setting for the *whole* palette, the better analogue to 'use system
//! colours'") is why every slot below is still a concrete value rather than
//! `Option`: `SYSTEM_LIKE_BASE`/`SYSTEM_LIKE_SELECTED` stand in for "system
//! menu colours"/"system highlight colours" specifically (plain black-on-
//! white, and the Windows default accent blue) — a reasonable modern
//! stand-in, not read from the registry, since `MenuDesign` has no
//! partial-system-fallback mode to read them into even if it did.

use y_core::menu::{self, Border, ColorPair, Fill, Gradient, MenuDesign};
use y_core::overlay::Color;

use crate::model::ConfigTree;
use crate::theme::{
    EdgeRule as ConfigEdgeRule, GradientDirection as ConfigGradientDirection, GradientOverride,
    SeparatorSpan as ConfigSeparatorSpan, Theme, ThemeOverrides,
};

const SYSTEM_LIKE_BASE: ColorPair = ColorPair {
    text: Color::rgb(0x00, 0x00, 0x00),
    back: Color::rgb(0xff, 0xff, 0xff),
};
const SYSTEM_LIKE_SELECTED: ColorPair = ColorPair {
    text: Color::rgb(0xff, 0xff, 0xff),
    back: Color::rgb(0x00, 0x78, 0xd7),
};
/// `GetSysColor(COLOR_GRAYTEXT)`'s usual value — PortAL reads this system
/// colour directly, the same one regardless of which skin is active
/// (`InitMenuVars`, confirmed by reading it), so it isn't part of any one
/// skin's own table row.
const SYSTEM_LIKE_DISABLED_TEXT: Color = Color::rgb(0x6d, 0x6d, 0x6d);

/// `menu.cpp`'s `0xf9d996` (both `design1.border_color` and `cg1`/`cg2`'s
/// warm-looking end/start), decoded *correctly* this time. Caught by the
/// user testing a real build and seeing yellow where they expected blue —
/// the bug: `0xf9d996` there is a raw C `COLORREF` integer literal, not an
/// `#RRGGBB` hex string, and `COLORREF` is little-endian `0x00BBGGRR` — so
/// reading its digits left-to-right as R,G,B (`Color::rgb(0xf9, 0xd9,
/// 0x96)`, what an earlier version of this module did, and what
/// [[portal-analysis]]'s own table also did despite correctly explaining
/// this exact BGR/RGB gotcha elsewhere, for the *custom `<skin>` XML*
/// parser's bug specifically) reads it backwards: the true channels are
/// B=0xf9, G=0xd9, R=0x96 → `Color::rgb(0x96, 0xd9, 0xf9)`, a light blue,
/// not tan. Both docs are corrected alongside this const.
const BLUE_ACCENT: Color = Color::rgb(0x96, 0xd9, 0xf9);
/// `menu.cpp`'s `design2.border_color = 0xFDD2A8` — same `COLORREF`
/// decoding as [`BLUE_ACCENT`]: B=0xFD, G=0xD2, R=0xA8.
const FLAT_BORDER: Color = Color::rgb(0xa8, 0xd2, 0xfd);
/// `menu.cpp`'s `cp1.background = 0xFCF4ED` (the `flat` skin's hover fill)
/// — same decoding again: B=0xFC, G=0xF4, R=0xED.
const FLAT_SELECTED_BACK: Color = Color::rgb(0xed, 0xf4, 0xfc);

/// PortAL's `default` skin (`menu.cpp`'s `design1`): 1px border, 10px
/// round, [`BLUE_ACCENT`], system base, a white-to-`BLUE_ACCENT` vertical
/// hover gradient.
///
/// `selected` departs from PortAL on purpose, which leaves it `NULL` (system
/// highlight colours): white text is unreadable on this light gradient. The
/// back matters too, as the search window paints it solid where a menu
/// paints the gradient.
pub const DEFAULT: MenuDesign = MenuDesign {
    base: SYSTEM_LIKE_BASE,
    selected: ColorPair {
        text: Color::rgb(0x00, 0x00, 0x00),
        back: BLUE_ACCENT,
    },
    disabled_text: SYSTEM_LIKE_DISABLED_TEXT,
    border: Border {
        size: 1,
        round: 10,
        color: BLUE_ACCENT,
    },
    fill: Fill::Gradient(Gradient {
        direction: menu::GradientDirection::Vertical,
        start: Color::rgb(0xff, 0xff, 0xff),
        end: BLUE_ACCENT,
    }),
    padding: 0,
    margin: 0,
    // `design1.edge = PORTAL_ST_SEL` — idle rows only, per its actual
    // (inverted-from-its-name) runtime meaning; see `y_core::menu::
    // EdgeRule`'s own doc comment.
    edge: Some(menu::EdgeRule::Idle),
    separator: menu::SeparatorSpan::AfterIcons,
    icon_backdrop: None,
};

/// PortAL's `flat` skin (`menu.cpp`'s `design2`): 1px border, no round,
/// [`FLAT_BORDER`], system base, black-on-[`FLAT_SELECTED_BACK`] selected,
/// no gradient.
pub const FLAT: MenuDesign = MenuDesign {
    base: SYSTEM_LIKE_BASE,
    selected: ColorPair {
        text: Color::rgb(0x00, 0x00, 0x00),
        back: FLAT_SELECTED_BACK,
    },
    disabled_text: SYSTEM_LIKE_DISABLED_TEXT,
    border: Border {
        size: 1,
        round: 0,
        color: FLAT_BORDER,
    },
    fill: Fill::Solid,
    padding: 0,
    margin: 0,
    // `design2.edge = PORTAL_ST_NONE` — no rule at all.
    edge: None,
    separator: menu::SeparatorSpan::AfterIcons,
    icon_backdrop: None,
};

/// PortAL's `dark`/`black` skin (`menu.cpp`'s `design3`): no border,
/// white-on-black base, system selected, a [`BLUE_ACCENT`]-to-black
/// horizontal hover gradient, the identical gradient again as an idle-row
/// icon-column backdrop, and an edge rule on every row.
pub const DARK: MenuDesign = MenuDesign {
    base: ColorPair {
        text: Color::rgb(0xff, 0xff, 0xff),
        back: Color::rgb(0x00, 0x00, 0x00),
    },
    selected: SYSTEM_LIKE_SELECTED,
    disabled_text: SYSTEM_LIKE_DISABLED_TEXT,
    border: Border {
        size: 0,
        round: 0,
        color: Color::rgb(0x00, 0x00, 0x00),
    },
    fill: Fill::Gradient(Gradient {
        direction: menu::GradientDirection::Horizontal,
        start: BLUE_ACCENT,
        end: Color::rgb(0x00, 0x00, 0x00),
    }),
    // `design3.edge = PORTAL_ST_ALL` — every row, idle or selected.
    // `design3.icon_gradiant = &cg2`, the exact same colours/direction as
    // `background_gradiant` above (confirmed by reading `menu.cpp`'s own
    // `cg2` literal, reused for both fields) — an idle row previews the
    // same accent gradient in just its icon column that a hovered row
    // shows across its whole cell.
    edge: Some(menu::EdgeRule::Always),
    separator: menu::SeparatorSpan::AfterIcons,
    icon_backdrop: Some(Gradient {
        direction: menu::GradientDirection::Horizontal,
        start: BLUE_ACCENT,
        end: Color::rgb(0x00, 0x00, 0x00),
    }),
    padding: 0,
    margin: 0,
};

/// y-pop's own `graphite` skin, not one of PortAL's: the neutral dark of a
/// Windows 11 menu, with a soft rounded hover floating between the rows.
pub const GRAPHITE: MenuDesign = MenuDesign {
    base: ColorPair {
        text: Color::rgb(0xf3, 0xf3, 0xf3),
        back: Color::rgb(0x2b, 0x2b, 0x2b),
    },
    selected: ColorPair {
        text: Color::rgb(0xff, 0xff, 0xff),
        back: Color::rgb(0x40, 0x40, 0x40),
    },
    disabled_text: Color::rgb(0x8a, 0x8a, 0x8a),
    border: Border {
        size: 0,
        round: 6,
        color: Color::rgb(0x4a, 0x4a, 0x4a),
    },
    fill: Fill::Solid,
    padding: 2,
    margin: 0,
    edge: None,
    separator: menu::SeparatorSpan::AfterIcons,
    icon_backdrop: None,
};

/// y-pop's own `midnight` skin: deep navy, with a saturated blue hover.
pub const MIDNIGHT: MenuDesign = MenuDesign {
    base: ColorPair {
        text: Color::rgb(0xe6, 0xed, 0xf7),
        back: Color::rgb(0x14, 0x1b, 0x2d),
    },
    selected: ColorPair {
        text: Color::rgb(0xff, 0xff, 0xff),
        back: Color::rgb(0x25, 0x63, 0xeb),
    },
    disabled_text: Color::rgb(0x7b, 0x88, 0xa1),
    border: Border {
        size: 0,
        round: 6,
        color: Color::rgb(0x2a, 0x35, 0x50),
    },
    fill: Fill::Gradient(Gradient {
        direction: menu::GradientDirection::Horizontal,
        start: Color::rgb(0x25, 0x63, 0xeb),
        end: Color::rgb(0x1e, 0x40, 0xaf),
    }),
    padding: 2,
    margin: 0,
    edge: None,
    separator: menu::SeparatorSpan::AfterIcons,
    icon_backdrop: None,
};

/// y-pop's own `sand` skin: warm paper, with a golden hover.
pub const SAND: MenuDesign = MenuDesign {
    base: ColorPair {
        text: Color::rgb(0x3b, 0x2f, 0x20),
        back: Color::rgb(0xfb, 0xf6, 0xec),
    },
    selected: ColorPair {
        text: Color::rgb(0x2b, 0x21, 0x14),
        back: Color::rgb(0xf3, 0xdf, 0xb8),
    },
    disabled_text: Color::rgb(0x8c, 0x7f, 0x6b),
    border: Border {
        size: 1,
        round: 6,
        color: Color::rgb(0xd9, 0xb8, 0x77),
    },
    fill: Fill::Gradient(Gradient {
        direction: menu::GradientDirection::Vertical,
        start: Color::rgb(0xff, 0xf3, 0xdc),
        end: Color::rgb(0xef, 0xd1, 0x9a),
    }),
    padding: 2,
    margin: 0,
    edge: None,
    separator: menu::SeparatorSpan::AfterIcons,
    icon_backdrop: None,
};

/// [`Theme`] + [`ThemeOverrides`] → `Option<MenuDesign>` — `None` means
/// plain native chrome. The one place `Theme::Auto` is resolved against the
/// live OS setting (`y_core::theme::is_dark`, a registry read); called
/// fresh from `menu::build` each time a popup opens rather than cached at
/// config-load time, so flipping Windows' light/dark setting takes effect
/// on the very next popup, not after a restart.
///
/// `theme="native"` (the default) with no `<skin>` at all stays plain
/// chrome — the zero-surprise case [`Theme::Native`]'s own doc comment
/// promises. But a config that sets a `<skin>` slot without also setting
/// `theme` clearly wants *something* themed to paint it onto — PortAL's own
/// custom-skin users never had to pick a base skin first either — so any
/// override present resolves as if `theme="auto"` had been written too,
/// then every set slot overrides that base.
pub fn resolve_design(theme: Theme, overrides: ThemeOverrides) -> Option<MenuDesign> {
    let effective = if theme == Theme::Native && overrides.any_set() {
        Theme::Auto
    } else {
        theme
    };
    let base = match effective {
        Theme::Native => return None,
        Theme::Default => DEFAULT,
        Theme::Flat => FLAT,
        Theme::Dark => DARK,
        Theme::Graphite => GRAPHITE,
        Theme::Midnight => MIDNIGHT,
        Theme::Sand => SAND,
        Theme::Auto if y_core::theme::is_dark() => DARK,
        Theme::Auto => DEFAULT,
    };
    Some(MenuDesign {
        base: ColorPair {
            text: overrides.text.unwrap_or(base.base.text),
            back: overrides.back.unwrap_or(base.base.back),
        },
        selected: ColorPair {
            text: overrides.hover_text.unwrap_or(base.selected.text),
            back: overrides.hover_back.unwrap_or(base.selected.back),
        },
        disabled_text: overrides.disabled_text.unwrap_or(base.disabled_text),
        border: overrides
            .border
            .map(|b| Border {
                size: b.size,
                round: b.round,
                color: b.color,
            })
            .unwrap_or(base.border),
        fill: overrides
            .background_gradient
            .map(|g| Fill::Gradient(convert_gradient(g)))
            .unwrap_or(base.fill),
        padding: overrides.padding.unwrap_or(base.padding),
        margin: overrides.margin.unwrap_or(base.margin),
        edge: overrides.edge.map(convert_edge).or(base.edge),
        separator: overrides
            .separator
            .map_or(base.separator, convert_separator),
        icon_backdrop: overrides
            .icon_gradient
            .map(convert_gradient)
            .or(base.icon_backdrop),
    })
}

/// The search window's design: its own `theme` and `<skin>` when `<search>`
/// has either, else the tray menus' ([`resolve_design`]).
///
/// The two are not merged. A search `theme` brings no menu `<skin>` with it —
/// a menu `back-color` written for a light theme would otherwise keep the
/// window light under `theme="dark"` — and a search `<skin>` replaces the
/// menus' wholesale, on the menus' theme unless it names its own. Of a search
/// `<skin>`, only [`ThemeOverrides::for_search`] counts.
pub fn resolve_search_design(tree: &ConfigTree) -> Option<MenuDesign> {
    let defaults = &tree.defaults;
    match tree
        .search
        .as_ref()
        .filter(|search| search.theme.is_some() || search.skin.is_some())
    {
        None => resolve_design(defaults.theme, defaults.theme_overrides),
        Some(search) => resolve_design(
            search.theme.unwrap_or(defaults.theme),
            search.skin.unwrap_or_default().for_search(),
        ),
    }
}

fn convert_gradient(g: GradientOverride) -> Gradient {
    Gradient {
        direction: match g.direction {
            ConfigGradientDirection::Horizontal => menu::GradientDirection::Horizontal,
            ConfigGradientDirection::Vertical => menu::GradientDirection::Vertical,
        },
        start: g.start,
        end: g.end,
    }
}

fn convert_separator(span: ConfigSeparatorSpan) -> menu::SeparatorSpan {
    match span {
        ConfigSeparatorSpan::AfterIcons => menu::SeparatorSpan::AfterIcons,
        ConfigSeparatorSpan::Full => menu::SeparatorSpan::Full,
    }
}

fn convert_edge(edge: ConfigEdgeRule) -> menu::EdgeRule {
    match edge {
        ConfigEdgeRule::Idle => menu::EdgeRule::Idle,
        ConfigEdgeRule::Hover => menu::EdgeRule::Hover,
        ConfigEdgeRule::Always => menu::EdgeRule::Always,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::BorderOverride;

    #[test]
    fn theme_native_with_no_overrides_stays_unthemed() {
        assert!(resolve_design(Theme::Native, ThemeOverrides::default()).is_none());
    }

    #[test]
    fn an_override_with_theme_left_native_still_themes_the_popup() {
        let overrides = ThemeOverrides {
            hover_back: Some(Color::rgb(0xff, 0x88, 0x00)),
            ..ThemeOverrides::default()
        };
        let design = resolve_design(Theme::Native, overrides).expect(
            "a colour override with no explicit theme must still resolve to a themed popup",
        );
        assert_eq!(design.selected.back, Color::rgb(0xff, 0x88, 0x00));
    }

    #[test]
    fn an_override_wins_over_the_base_theme_everywhere_else_matches_it() {
        let overrides = ThemeOverrides {
            back: Some(Color::rgb(1, 2, 3)),
            ..ThemeOverrides::default()
        };
        let design = resolve_design(Theme::Default, overrides).expect("theme=\"default\" themes");
        assert_eq!(design.base.back, Color::rgb(1, 2, 3));
        assert_eq!(
            design.base.text, DEFAULT.base.text,
            "every other slot keeps the base theme's value"
        );
    }

    #[test]
    fn a_border_override_replaces_the_whole_border_not_one_field() {
        let overrides = ThemeOverrides {
            border: Some(BorderOverride {
                size: 2,
                round: 12,
                color: Color::rgb(9, 9, 9),
            }),
            ..ThemeOverrides::default()
        };
        let design = resolve_design(Theme::Dark, overrides).expect("theme=\"dark\" themes");
        assert_eq!(design.border.size, 2);
        assert_eq!(design.border.round, 12);
        assert_eq!(design.border.color, Color::rgb(9, 9, 9));
    }

    #[test]
    fn a_gradient_override_replaces_the_base_themes_solid_fill() {
        let overrides = ThemeOverrides {
            background_gradient: Some(GradientOverride {
                start: Color::rgb(1, 2, 3),
                end: Color::rgb(4, 5, 6),
                direction: ConfigGradientDirection::Vertical,
            }),
            ..ThemeOverrides::default()
        };
        let design = resolve_design(Theme::Dark, overrides).expect("theme=\"dark\" themes");
        match design.fill {
            menu::Fill::Gradient(gradient) => {
                assert_eq!(gradient.start, Color::rgb(1, 2, 3));
                assert_eq!(gradient.end, Color::rgb(4, 5, 6));
                assert_eq!(gradient.direction, menu::GradientDirection::Vertical);
            }
            menu::Fill::Solid => {
                panic!("expected the override's gradient, not the base theme's solid fill")
            }
        }
    }

    #[test]
    fn a_margin_override_replaces_the_base_themes_margin() {
        let overrides = ThemeOverrides {
            margin: Some(9),
            ..ThemeOverrides::default()
        };
        let design = resolve_design(Theme::Default, overrides).expect("theme=\"default\" themes");
        assert_eq!(design.margin, 9);
    }

    #[test]
    fn a_padding_override_replaces_the_base_themes_padding() {
        let overrides = ThemeOverrides {
            padding: Some(7),
            ..ThemeOverrides::default()
        };
        let design = resolve_design(Theme::Default, overrides).expect("theme=\"default\" themes");
        assert_eq!(design.padding, 7);
    }

    /// Pins the three built-in skins to `menu.cpp`'s actual `COLORREF`
    /// literals, correctly decoded (`BGR`, not `RGB` — see [`BLUE_ACCENT`]'s
    /// doc comment) — a regression test for two mistakes now, not one: the
    /// original made-up `LIGHT`/`DARK` values (not PortAL's skins at all),
    /// and then the yellow-instead-of-blue channel-order bug in the first
    /// attempt to reproduce them for real, caught by the user against a
    /// real build.
    #[test]
    fn the_three_built_in_skins_match_portals_own_documented_values() {
        assert_eq!(DEFAULT.border.size, 1);
        assert_eq!(DEFAULT.border.round, 10);
        assert_eq!(DEFAULT.border.color, Color::rgb(0x96, 0xd9, 0xf9));
        match DEFAULT.fill {
            menu::Fill::Gradient(g) => {
                assert_eq!(g.direction, menu::GradientDirection::Vertical);
                assert_eq!(g.start, Color::rgb(0xff, 0xff, 0xff));
                assert_eq!(g.end, Color::rgb(0x96, 0xd9, 0xf9));
            }
            menu::Fill::Solid => panic!("PortAL's default skin has a background gradient"),
        }

        assert_eq!(FLAT.border.size, 1);
        assert_eq!(FLAT.border.round, 0);
        assert_eq!(FLAT.border.color, Color::rgb(0xa8, 0xd2, 0xfd));
        assert_eq!(FLAT.selected.text, Color::rgb(0x00, 0x00, 0x00));
        assert_eq!(FLAT.selected.back, Color::rgb(0xed, 0xf4, 0xfc));
        assert!(
            matches!(FLAT.fill, menu::Fill::Solid),
            "PortAL's flat skin has no gradient"
        );

        assert_eq!(DARK.border.size, 0);
        assert_eq!(DARK.border.round, 0);
        assert_eq!(DARK.base.text, Color::rgb(0xff, 0xff, 0xff));
        assert_eq!(DARK.base.back, Color::rgb(0x00, 0x00, 0x00));
        match DARK.fill {
            menu::Fill::Gradient(g) => {
                assert_eq!(g.direction, menu::GradientDirection::Horizontal);
                assert_eq!(g.start, Color::rgb(0x96, 0xd9, 0xf9));
                assert_eq!(g.end, Color::rgb(0x00, 0x00, 0x00));
            }
            menu::Fill::Solid => panic!("PortAL's dark skin has a background gradient"),
        }

        assert_eq!(
            DEFAULT.edge,
            Some(menu::EdgeRule::Idle),
            "design1.edge = PORTAL_ST_SEL"
        );
        assert_eq!(FLAT.edge, None, "design2.edge = PORTAL_ST_NONE");
        assert_eq!(
            DARK.edge,
            Some(menu::EdgeRule::Always),
            "design3.edge = PORTAL_ST_ALL"
        );

        assert!(DEFAULT.icon_backdrop.is_none());
        assert!(FLAT.icon_backdrop.is_none());
        let dark_gradient = match DARK.fill {
            menu::Fill::Gradient(g) => g,
            menu::Fill::Solid => unreachable!("checked above"),
        };
        assert_eq!(
            DARK.icon_backdrop,
            Some(dark_gradient),
            "design3's icon_gradiant and background_gradiant are the identical &cg2"
        );
    }

    /// WCAG's contrast ratio, from 1 (none) to 21 (black on white).
    fn contrast(a: Color, b: Color) -> f32 {
        fn luminance(color: Color) -> f32 {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// A menu paints the fill and the search window paints `selected.back`,
    /// so hover text has to read on both. `DARK` isn't here: its gradient
    /// runs from light blue to black, and no one colour reads on both ends.
    #[test]
    fn hover_text_reads_on_everything_it_is_drawn_over() {
        for (name, design) in [
            ("default", DEFAULT),
            ("flat", FLAT),
            ("graphite", GRAPHITE),
            ("midnight", MIDNIGHT),
            ("sand", SAND),
        ] {
            let mut backs = vec![design.selected.back];
            if let menu::Fill::Gradient(gradient) = design.fill {
                backs.extend([gradient.start, gradient.end]);
            }
            for back in backs {
                let ratio = contrast(design.selected.text, back);
                assert!(ratio >= 4.5, "{name}: {ratio} on {back:?}");
            }
            let ratio = contrast(design.base.text, design.base.back);
            assert!(ratio >= 4.5, "{name}: {ratio} on an idle row");
        }
    }

    #[test]
    fn an_edge_override_replaces_the_base_themes_edge() {
        let overrides = ThemeOverrides {
            edge: Some(ConfigEdgeRule::Hover),
            ..ThemeOverrides::default()
        };
        // `default` has its own `Idle` — confirm the override actually wins.
        let design = resolve_design(Theme::Default, overrides).expect("theme=\"default\" themes");
        assert_eq!(design.edge, Some(menu::EdgeRule::Hover));
    }

    #[test]
    fn an_edge_override_is_not_lost_when_the_base_theme_has_none() {
        let overrides = ThemeOverrides {
            edge: Some(ConfigEdgeRule::Always),
            ..ThemeOverrides::default()
        };
        // `flat` has no edge of its own (`None`) — the override must still
        // come through, not be swallowed by `.or(base.edge)`'s fallback.
        let design = resolve_design(Theme::Flat, overrides).expect("theme=\"flat\" themes");
        assert_eq!(design.edge, Some(menu::EdgeRule::Always));
    }

    #[test]
    fn an_icon_gradient_override_replaces_the_base_themes_icon_backdrop() {
        let overrides = ThemeOverrides {
            icon_gradient: Some(GradientOverride {
                start: Color::rgb(1, 2, 3),
                end: Color::rgb(4, 5, 6),
                direction: ConfigGradientDirection::Vertical,
            }),
            ..ThemeOverrides::default()
        };
        // `default` has no icon backdrop of its own (`None`) — same "not
        // swallowed by the fallback" case as the edge override above.
        let design = resolve_design(Theme::Default, overrides).expect("theme=\"default\" themes");
        let icon_backdrop = design.icon_backdrop.expect("the override set one");
        assert_eq!(icon_backdrop.start, Color::rgb(1, 2, 3));
        assert_eq!(icon_backdrop.end, Color::rgb(4, 5, 6));
        assert_eq!(icon_backdrop.direction, menu::GradientDirection::Vertical);
    }

    fn search_back(xml: &str) -> Option<Color> {
        let tree = crate::config::load_str(xml, &mut Vec::new()).expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        resolve_search_design(&tree).map(|design| design.base.back)
    }

    #[test]
    fn search_follows_the_menus_unless_it_says_otherwise() {
        let menus = r#"theme="flat"><skin back-color="ebebeb"/"#;
        let light = Some(Color::rgb(0xeb, 0xeb, 0xeb));
        assert_eq!(
            search_back(&format!("<y-pop {menus}><search/><quit/></y-pop>")),
            light
        );
    }

    #[test]
    fn a_search_theme_does_not_inherit_the_menus_skin() {
        let xml = r#"<y-pop theme="flat"><skin back-color="ebebeb"/>
            <search theme="graphite"/><quit/></y-pop>"#;
        assert_eq!(search_back(xml), Some(GRAPHITE.base.back));
    }

    #[test]
    fn a_search_skin_replaces_the_menus_skin_on_the_menus_theme() {
        let xml = r#"<y-pop theme="dark"><skin back-color="ebebeb"/>
            <search><skin back-color="102030"/></search><quit/></y-pop>"#;
        assert_eq!(search_back(xml), Some(Color::rgb(0x10, 0x20, 0x30)));

        let xml = r#"<y-pop theme="dark"><skin back-color="ebebeb"/>
            <search><skin hover-back-color="102030"/></search><quit/></y-pop>"#;
        assert_eq!(search_back(xml), Some(DARK.base.back));
    }

    #[test]
    fn a_search_skin_of_what_the_window_ignores_does_not_theme_it() {
        let xml = r#"<y-pop><search><skin padding="4"/></search><quit/></y-pop>"#;
        let tree = crate::config::load_str(xml, &mut Vec::new()).expect("valid");
        assert!(resolve_search_design(&tree).is_none());
        assert!(
            tree.warnings
                .iter()
                .any(|warning| warning.contains("padding")),
            "{:?}",
            tree.warnings
        );
    }
}
