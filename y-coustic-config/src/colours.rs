//! The Colours screen's edits, as plain functions on a `Settings`: which colour
//! of which set is being edited, what a colour is when nothing is set, and what
//! the picker's reports do. No window, so every rule is tested.
//!
//! A colour is addressed by its **set** (everything, or one kind of volume) and
//! its **slot** (panel, text, track, accent, muted). Unset means inherited: from
//! the set for everything, then from the theme. What is shown for an unset one
//! is what y-coustic would draw, found with its own `Palette`, so the swatch and
//! the preview cannot disagree with the HUD.

use y_core::overlay::Color;
use y_coustic::config::{Colors, Settings, Tint};
use y_coustic::model::Kind;
use y_coustic::paint::Palette;

/// Which colours: the ones for everything, or for one kind of volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Set {
    All,
    Master,
    Microphone,
    Application,
}

impl Set {
    pub const ALL: [Set; 4] = [Set::All, Set::Master, Set::Microphone, Set::Application];

    pub fn title(self) -> &'static str {
        match self {
            Self::All => "Everything",
            Self::Master => "Master volume",
            Self::Microphone => "Microphone",
            Self::Application => "Application",
        }
    }

    /// The kind whose HUD shows what this set sets, for the colour it is shown at.
    fn kind(self) -> Kind {
        match self {
            Self::All | Self::Master => Kind::Master,
            Self::Microphone => Kind::Mic,
            Self::Application => Kind::App,
        }
    }
}

/// One colour of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// The window's background.
    Panel,
    /// Text and glyphs.
    Text,
    /// The empty part of a bar.
    Track,
    /// The filled part while sound is on.
    Accent,
    /// The filled part while muted.
    Muted,
}

impl Slot {
    pub const ALL: [Slot; 5] = [Slot::Panel, Slot::Text, Slot::Track, Slot::Accent, Slot::Muted];

    pub fn title(self) -> &'static str {
        match self {
            Self::Panel => "Background",
            Self::Text => "Text",
            Self::Track => "Empty bar",
            Self::Accent => "Bar",
            Self::Muted => "Bar, muted",
        }
    }
}

/// A colour of a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Addr {
    pub set: Set,
    pub slot: Slot,
}

fn tint(colors: &Colors, set: Set) -> &Tint {
    match set {
        Set::All => &colors.all,
        Set::Master => &colors.master,
        Set::Microphone => &colors.microphone,
        Set::Application => &colors.application,
    }
}

fn tint_mut(colors: &mut Colors, set: Set) -> &mut Tint {
    match set {
        Set::All => &mut colors.all,
        Set::Master => &mut colors.master,
        Set::Microphone => &mut colors.microphone,
        Set::Application => &mut colors.application,
    }
}

fn slot_mut(tint: &mut Tint, slot: Slot) -> &mut Option<Color> {
    match slot {
        Slot::Panel => &mut tint.panel,
        Slot::Text => &mut tint.text,
        Slot::Track => &mut tint.track,
        Slot::Accent => &mut tint.accent,
        Slot::Muted => &mut tint.muted,
    }
}

/// What the file sets for `addr`, if anything.
pub fn get(colors: &Colors, addr: Addr) -> Option<Color> {
    let tint = tint(colors, addr.set);
    match addr.slot {
        Slot::Panel => tint.panel,
        Slot::Text => tint.text,
        Slot::Track => tint.track,
        Slot::Accent => tint.accent,
        Slot::Muted => tint.muted,
    }
}

/// The colour y-coustic draws for `addr` in the light or dark theme: what is set,
/// or what it inherits. Opaque: the HUD's canvas is.
pub fn effective(colors: &Colors, addr: Addr, dark: bool) -> Color {
    let kind = addr.set.kind();
    let shown = |muted: bool| Palette::for_theme(dark, kind, muted, colors);
    let color = match addr.slot {
        Slot::Panel => shown(false).panel,
        Slot::Text => shown(false).text,
        Slot::Track => shown(false).track,
        Slot::Accent => shown(false).fill,
        Slot::Muted => shown(true).fill,
    };
    color.with_alpha(255)
}

/// How many colours of the set are set.
pub fn count(colors: &Colors, set: Set) -> usize {
    Slot::ALL
        .into_iter()
        .filter(|&slot| get(colors, Addr { set, slot }).is_some())
        .count()
}

/// A colour being edited, and what it was, for Cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Editing {
    pub addr: Addr,
    pub original: Option<Color>,
}

/// What the screen remembers besides the settings.
#[derive(Debug, Clone, Copy, Default)]
pub struct ColoursUi {
    pub editing: Option<Editing>,
}

#[derive(Debug, Clone, Copy)]
pub enum ColourMsg {
    /// A swatch was clicked: open its picker, or close it if it is open.
    Open(Addr),
    /// The picker moved.
    Color(Color),
    /// One colour back to what it inherits.
    Reset(Addr),
    /// Every colour of a set back to what it inherits.
    ResetSet(Set),
    /// The picker's Cancel (`revert`: put the colour back) or Done.
    Close { revert: bool },
}

/// Settles `message`; whether the settings changed. `dark` is the theme the
/// swatches are shown in: the picker reports the colour it was given when it
/// opens, and that is not an edit.
pub fn apply(settings: &mut Settings, ui: &mut ColoursUi, message: ColourMsg, dark: bool) -> bool {
    let colors = &mut settings.hud.colors;
    match message {
        ColourMsg::Open(addr) => {
            ui.editing = match ui.editing {
                Some(editing) if editing.addr == addr => None,
                _ => Some(Editing {
                    addr,
                    original: get(colors, addr),
                }),
            };
            false
        }
        ColourMsg::Color(color) => {
            let Some(editing) = ui.editing else {
                return false;
            };
            // Opaque: the picker has no alpha here, and the HUD's surface has none
            // to spare.
            let color = Color::rgb(color.r, color.g, color.b);
            if color == effective(colors, editing.addr, dark) {
                return false;
            }
            set(colors, editing.addr, Some(color))
        }
        ColourMsg::Reset(addr) => set(colors, addr, None),
        ColourMsg::ResetSet(which) => {
            if ui.editing.is_some_and(|editing| editing.addr.set == which) {
                ui.editing = None;
            }
            let was = *tint(colors, which);
            *tint_mut(colors, which) = Tint::default();
            was != Tint::default()
        }
        ColourMsg::Close { revert } => {
            let Some(editing) = ui.editing.take() else {
                return false;
            };
            revert && set(colors, editing.addr, editing.original)
        }
    }
}

/// Sets `addr` to `color`; whether that changed anything.
fn set(colors: &mut Colors, addr: Addr, color: Option<Color>) -> bool {
    let slot = slot_mut(tint_mut(colors, addr.set), addr.slot);
    if *slot == color {
        return false;
    }
    *slot = color;
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn settings() -> Settings {
        Settings::parse(PathBuf::from("test.xml"), y_coustic::config::DEFAULT_XML)
            .expect("the shipped file loads")
    }

    fn addr(set: Set, slot: Slot) -> Addr {
        Addr { set, slot }
    }

    fn run(settings: &mut Settings, ui: &mut ColoursUi, message: ColourMsg) -> bool {
        apply(settings, ui, message, true)
    }

    const GREEN: Color = Color::rgb(0x3F, 0xB9, 0x50);

    #[test]
    fn an_unset_colour_shows_what_y_coustic_would_draw() {
        let settings = settings();
        let colors = &settings.hud.colors;
        for dark in [true, false] {
            let master = Palette::for_theme(dark, Kind::Master, false, colors);
            assert_eq!(
                effective(colors, addr(Set::Master, Slot::Accent), dark),
                master.fill.with_alpha(255)
            );
            // The microphone's bar is red while live, whatever the theme's accent.
            let mic = Palette::for_theme(dark, Kind::Mic, false, colors);
            assert_eq!(
                effective(colors, addr(Set::Microphone, Slot::Accent), dark),
                mic.fill.with_alpha(255)
            );
            assert_ne!(mic.fill, master.fill);
        }
        // Dark and light differ.
        assert_ne!(
            effective(colors, addr(Set::All, Slot::Panel), true),
            effective(colors, addr(Set::All, Slot::Panel), false)
        );
    }

    #[test]
    fn a_colour_is_set_by_the_picker_and_an_unchanged_report_is_not_an_edit() {
        let mut settings = settings();
        let mut ui = ColoursUi::default();
        let target = addr(Set::Master, Slot::Accent);

        run(&mut settings, &mut ui, ColourMsg::Open(target));
        assert_eq!(ui.editing.map(|editing| editing.addr), Some(target));

        // What the picker reports when it opens: the colour it was given.
        let shown = effective(&settings.hud.colors, target, true);
        assert!(!run(&mut settings, &mut ui, ColourMsg::Color(shown)));
        assert_eq!(get(&settings.hud.colors, target), None);

        assert!(run(&mut settings, &mut ui, ColourMsg::Color(GREEN)));
        assert_eq!(get(&settings.hud.colors, target), Some(GREEN));
        assert!(!run(&mut settings, &mut ui, ColourMsg::Color(GREEN)));
        // A colour with an alpha is stored opaque.
        assert!(run(&mut settings, &mut ui, ColourMsg::Color(Color::rgba(1, 2, 3, 40))));
        assert_eq!(get(&settings.hud.colors, target), Some(Color::rgb(1, 2, 3)));
    }

    #[test]
    fn cancel_puts_the_colour_back_and_done_keeps_it() {
        let mut settings = settings();
        let mut ui = ColoursUi::default();
        let target = addr(Set::All, Slot::Panel);

        run(&mut settings, &mut ui, ColourMsg::Open(target));
        run(&mut settings, &mut ui, ColourMsg::Color(GREEN));
        assert!(run(&mut settings, &mut ui, ColourMsg::Close { revert: true }));
        assert_eq!(get(&settings.hud.colors, target), None);
        assert!(ui.editing.is_none());

        run(&mut settings, &mut ui, ColourMsg::Open(target));
        run(&mut settings, &mut ui, ColourMsg::Color(GREEN));
        assert!(!run(&mut settings, &mut ui, ColourMsg::Close { revert: false }));
        assert_eq!(get(&settings.hud.colors, target), Some(GREEN));

        // Opening the open swatch again closes it.
        run(&mut settings, &mut ui, ColourMsg::Open(target));
        run(&mut settings, &mut ui, ColourMsg::Open(target));
        assert!(ui.editing.is_none());
    }

    #[test]
    fn a_reset_goes_back_to_what_is_inherited() {
        let mut settings = settings();
        let mut ui = ColoursUi::default();
        let target = addr(Set::Microphone, Slot::Accent);
        let inherited = effective(&settings.hud.colors, target, true);

        run(&mut settings, &mut ui, ColourMsg::Open(target));
        run(&mut settings, &mut ui, ColourMsg::Color(GREEN));
        run(&mut settings, &mut ui, ColourMsg::Close { revert: false });
        assert_eq!(effective(&settings.hud.colors, target, true), GREEN);

        assert!(run(&mut settings, &mut ui, ColourMsg::Reset(target)));
        assert!(!run(&mut settings, &mut ui, ColourMsg::Reset(target)));
        assert_eq!(effective(&settings.hud.colors, target, true), inherited);
    }

    #[test]
    fn the_general_accent_does_not_reach_the_microphone_but_its_own_does() {
        let mut settings = settings();
        let mut ui = ColoursUi::default();
        let before = effective(&settings.hud.colors, addr(Set::Microphone, Slot::Accent), true);

        run(&mut settings, &mut ui, ColourMsg::Open(addr(Set::All, Slot::Accent)));
        run(&mut settings, &mut ui, ColourMsg::Color(GREEN));
        run(&mut settings, &mut ui, ColourMsg::Close { revert: false });
        assert_eq!(effective(&settings.hud.colors, addr(Set::Master, Slot::Accent), true), GREEN);
        assert_eq!(effective(&settings.hud.colors, addr(Set::Application, Slot::Accent), true), GREEN);
        assert_eq!(
            effective(&settings.hud.colors, addr(Set::Microphone, Slot::Accent), true),
            before
        );

        run(&mut settings, &mut ui, ColourMsg::Open(addr(Set::Microphone, Slot::Accent)));
        run(&mut settings, &mut ui, ColourMsg::Color(Color::rgb(1, 2, 3)));
        assert_eq!(
            effective(&settings.hud.colors, addr(Set::Microphone, Slot::Accent), true),
            Color::rgb(1, 2, 3)
        );
    }

    #[test]
    fn resetting_a_set_clears_only_that_set() {
        let mut settings = settings();
        let mut ui = ColoursUi::default();
        for set in [Set::Master, Set::Microphone] {
            for slot in [Slot::Panel, Slot::Text] {
                run(&mut settings, &mut ui, ColourMsg::Open(addr(set, slot)));
                run(&mut settings, &mut ui, ColourMsg::Color(GREEN));
                run(&mut settings, &mut ui, ColourMsg::Close { revert: false });
            }
        }
        assert_eq!(count(&settings.hud.colors, Set::Master), 2);

        run(&mut settings, &mut ui, ColourMsg::Open(addr(Set::Master, Slot::Track)));
        assert!(run(&mut settings, &mut ui, ColourMsg::ResetSet(Set::Master)));
        assert!(ui.editing.is_none(), "a picker on a cleared set closes");
        assert_eq!(count(&settings.hud.colors, Set::Master), 0);
        assert_eq!(count(&settings.hud.colors, Set::Microphone), 2);
        assert!(!run(&mut settings, &mut ui, ColourMsg::ResetSet(Set::Master)));
    }

    #[test]
    fn what_was_chosen_is_what_y_coustic_reads_back() {
        let mut settings = settings();
        let mut ui = ColoursUi::default();
        run(&mut settings, &mut ui, ColourMsg::Open(addr(Set::Application, Slot::Muted)));
        run(&mut settings, &mut ui, ColourMsg::Color(GREEN));
        run(&mut settings, &mut ui, ColourMsg::Close { revert: false });
        let again = Settings::parse(PathBuf::from("test.xml"), &settings.to_xml()).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.hud.colors, settings.hud.colors);
        assert_eq!(again.hud.colors.application.muted, Some(GREEN));
    }
}
