//! The pure vocabulary of y-coustic: what can be asked for, of what, and the
//! arithmetic on a volume level. No Win32, no COM: everything here is unit
//! tested, and the config reader and the settings app share it.

/// What a binding does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// One step up (a hotkey press, or a wheel notch).
    VolumeUp,
    /// One step down.
    VolumeDown,
    /// Toggle mute.
    Mute,
    /// Set the level to a percentage, 0–100 (clamped by [`Action::set_volume`]).
    SetVolume(u8),
    /// Open or close the panel.
    Mixer,
}

impl Action {
    /// The name used in the configuration file.
    pub fn name(self) -> &'static str {
        match self {
            Self::VolumeUp => "volume-up",
            Self::VolumeDown => "volume-down",
            Self::Mute => "mute",
            Self::SetVolume(_) => "set-volume",
            Self::Mixer => "mixer",
        }
    }

    /// `SetVolume` with the percentage clamped to 0–100.
    pub fn set_volume(percent: u8) -> Self {
        Self::SetVolume(percent.min(100))
    }

    /// The signed number of notches a stepping action stands for, `None` for
    /// the others.
    pub fn notches(self) -> Option<f32> {
        match self {
            Self::VolumeUp => Some(1.0),
            Self::VolumeDown => Some(-1.0),
            _ => None,
        }
    }
}

/// What an action applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The default render device's master volume.
    Master,
    /// The default capture device.
    Mic,
    /// The application owning the foreground window.
    Focused,
    /// The application under the mouse pointer.
    Pointed,
}

/// Which kind of thing the HUD or a panel row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Master,
    Mic,
    App,
}

impl Target {
    /// The name used in the configuration file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Mic => "mic",
            Self::Focused => "focused",
            Self::Pointed => "pointed",
        }
    }

    /// The target a configuration name designates, in any case.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "master" => Some(Self::Master),
            "mic" => Some(Self::Mic),
            "focused" => Some(Self::Focused),
            "pointed" => Some(Self::Pointed),
            _ => None,
        }
    }

    /// The kind of thing the target designates.
    pub fn kind(self) -> Kind {
        match self {
            Self::Master => Kind::Master,
            Self::Mic => Kind::Mic,
            Self::Focused | Self::Pointed => Kind::App,
        }
    }
}

/// A volume level as a fraction, 0.0 (silent) to 1.0 (full), the unit Core
/// Audio's scalar volume uses.
pub type Level = f32;

/// The level after `notches` steps of `step_pct` percent each, clamped to
/// 0..=1. A notch may be fractional (a touchpad wheel sends sub-notch deltas);
/// negative goes down. A non-finite input leaves the level unchanged.
pub fn step(level: Level, notches: f32, step_pct: f32) -> Level {
    let moved = level + notches * step_pct / 100.0;
    if moved.is_finite() {
        moved.clamp(0.0, 1.0)
    } else {
        level.clamp(0.0, 1.0)
    }
}

/// The level for `SetVolume(percent)`.
pub fn from_percent(percent: u8) -> Level {
    f32::from(percent.min(100)) / 100.0
}

/// A level as a whole percentage, for display.
pub fn to_percent(level: Level) -> u8 {
    (level.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// A volume and its mute switch, as read from or written to a device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub level: Level,
    pub muted: bool,
}

/// The state after `action`, stepping by `step_pct` percent.
///
/// Raising the volume (a step up, or an explicit level) unmutes: pressing
/// "louder" on a muted device and hearing nothing reads as a broken key. A
/// step down leaves the switch alone. `Mixer` changes nothing.
///
/// The microphone is the exception ([`Kind::Mic`]): nothing but an explicit
/// `Mute` unmutes it, so a stray wheel notch or a step key cannot open a live
/// microphone. Reaching 0 % does not mute either; the two states stay separate.
pub fn apply(kind: Kind, state: State, action: Action, step_pct: f32) -> State {
    let unmuted = if kind == Kind::Mic {
        state.muted
    } else {
        false
    };
    match action {
        Action::VolumeUp => State {
            level: step(state.level, 1.0, step_pct),
            muted: unmuted,
        },
        Action::VolumeDown => State {
            level: step(state.level, -1.0, step_pct),
            ..state
        },
        Action::Mute => State {
            muted: toggle(state.muted),
            ..state
        },
        Action::SetVolume(percent) => State {
            level: from_percent(percent),
            muted: unmuted,
        },
        Action::Mixer => state,
    }
}

/// What a binding changes: an action, or a wheel's fractional notches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Action(Action),
    Scroll(f32),
}

impl Change {
    /// The state after the change, by the rules of [`apply`] and
    /// [`apply_scroll`].
    pub fn apply(self, kind: Kind, state: State, step_pct: f32) -> State {
        match self {
            Self::Action(action) => apply(kind, state, action, step_pct),
            Self::Scroll(notches) => apply_scroll(kind, state, notches, step_pct),
        }
    }
}

/// The state after a wheel scrolled `notches` (fractional on a touchpad,
/// negative down), by the same rules as [`apply`]: raising unmutes, except the
/// microphone, and lowering leaves the switch alone. No movement changes
/// nothing, so a zero-sum burst of notches does not unmute.
pub fn apply_scroll(kind: Kind, state: State, notches: f32, step_pct: f32) -> State {
    if notches > 0.0 {
        State {
            level: step(state.level, notches, step_pct),
            muted: kind == Kind::Mic && state.muted,
        }
    } else if notches < 0.0 {
        State {
            level: step(state.level, notches, step_pct),
            ..state
        }
    } else {
        state
    }
}

/// The mute state after a `Mute` action.
pub fn toggle(muted: bool) -> bool {
    !muted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn scrolling_up_unmutes_all_but_the_microphone() {
        let muted = State {
            level: 0.5,
            muted: true,
        };
        let master = apply_scroll(Kind::Master, muted, 1.0, 5.0);
        assert!(near(master.level, 0.55) && !master.muted);
        let app = apply_scroll(Kind::App, muted, 0.25, 4.0);
        assert!(near(app.level, 0.51) && !app.muted);
        let mic = apply_scroll(Kind::Mic, muted, 1.0, 5.0);
        assert!(near(mic.level, 0.55) && mic.muted);
    }

    #[test]
    fn scrolling_down_leaves_the_switch_alone() {
        let muted = State {
            level: 0.5,
            muted: true,
        };
        let after = apply_scroll(Kind::Master, muted, -2.0, 5.0);
        assert!(near(after.level, 0.40) && after.muted);
    }

    #[test]
    fn scrolling_nothing_changes_nothing() {
        let muted = State {
            level: 0.5,
            muted: true,
        };
        assert_eq!(apply_scroll(Kind::Master, muted, 0.0, 5.0), muted);
    }

    #[test]
    fn a_step_moves_by_the_configured_percentage() {
        assert!(near(step(0.50, 1.0, 2.0), 0.52));
        assert!(near(step(0.50, -1.0, 5.0), 0.45));
    }

    #[test]
    fn a_step_clamps_at_both_ends() {
        assert_eq!(step(0.99, 1.0, 5.0), 1.0);
        assert_eq!(step(0.01, -1.0, 5.0), 0.0);
        assert_eq!(step(1.0, 3.0, 30.0), 1.0);
        assert_eq!(step(0.0, -3.0, 30.0), 0.0);
    }

    #[test]
    fn a_fractional_notch_moves_proportionally() {
        // A touchpad delta of 30 of 120 is a quarter notch.
        assert!(near(step(0.50, 0.25, 4.0), 0.51));
        assert!(near(step(0.50, -0.5, 4.0), 0.48));
    }

    #[test]
    fn many_notches_at_once_add_up() {
        assert!(near(step(0.10, 4.0, 5.0), 0.30));
    }

    #[test]
    fn a_non_finite_input_changes_nothing() {
        assert_eq!(step(0.4, f32::NAN, 2.0), 0.4);
        assert_eq!(step(0.4, f32::INFINITY, 2.0), 0.4);
        assert_eq!(step(0.4, 1.0, f32::NAN), 0.4);
    }

    #[test]
    fn percent_conversions_round_trip_and_clamp() {
        for p in 0..=100u8 {
            assert_eq!(to_percent(from_percent(p)), p);
        }
        assert_eq!(from_percent(250), 1.0);
        assert_eq!(to_percent(1.7), 100);
        assert_eq!(to_percent(-0.3), 0);
        assert_eq!(Action::set_volume(200), Action::SetVolume(100));
    }

    #[test]
    fn target_names_round_trip() {
        for target in [
            Target::Master,
            Target::Mic,
            Target::Focused,
            Target::Pointed,
        ] {
            assert_eq!(Target::from_name(target.name()), Some(target));
        }
        assert_eq!(Target::from_name(" MIC "), Some(Target::Mic));
        assert_eq!(Target::from_name("speakers"), None);
    }

    #[test]
    fn apply_follows_the_unmute_rules() {
        let quiet = State {
            level: 0.5,
            muted: true,
        };
        let loud = State {
            level: 0.5,
            muted: false,
        };

        assert_eq!(
            apply(Kind::Master, quiet, Action::VolumeUp, 10.0),
            State {
                level: 0.6,
                muted: false
            }
        );
        assert_eq!(
            apply(Kind::Master, quiet, Action::VolumeDown, 10.0),
            State {
                level: 0.4,
                muted: true
            }
        );
        assert_eq!(apply(Kind::Master, loud, Action::Mute, 10.0), quiet);
        assert_eq!(apply(Kind::Master, quiet, Action::Mute, 10.0), loud);
        assert_eq!(
            apply(Kind::Master, quiet, Action::SetVolume(30), 10.0),
            State {
                level: 0.3,
                muted: false
            }
        );
        assert_eq!(apply(Kind::Master, quiet, Action::Mixer, 10.0), quiet);
    }

    #[test]
    fn only_an_explicit_mute_unmutes_the_microphone() {
        let quiet = State {
            level: 0.5,
            muted: true,
        };
        let live = State {
            level: 0.5,
            muted: false,
        };
        let up = apply(Kind::Mic, quiet, Action::VolumeUp, 10.0);
        assert!(up.muted && up.level > 0.5);
        assert!(apply(Kind::Mic, quiet, Action::VolumeDown, 10.0).muted);
        assert_eq!(
            apply(Kind::Mic, quiet, Action::SetVolume(30), 10.0),
            State {
                level: 0.3,
                muted: true
            }
        );
        assert_eq!(apply(Kind::Mic, quiet, Action::Mute, 10.0), live);
        // Reaching zero does not mute.
        assert!(
            !apply(
                Kind::Mic,
                State {
                    level: 0.02,
                    ..live
                },
                Action::VolumeDown,
                10.0
            )
            .muted
        );
    }

    #[test]
    fn toggle_flips() {
        assert!(toggle(false));
        assert!(!toggle(true));
    }

    #[test]
    fn only_stepping_actions_have_notches() {
        assert_eq!(Action::VolumeUp.notches(), Some(1.0));
        assert_eq!(Action::VolumeDown.notches(), Some(-1.0));
        assert_eq!(Action::Mute.notches(), None);
        assert_eq!(Action::SetVolume(10).notches(), None);
        assert_eq!(Action::Mixer.notches(), None);
    }

    #[test]
    fn targets_map_to_the_kind_the_hud_shows() {
        assert_eq!(Target::Master.kind(), Kind::Master);
        assert_eq!(Target::Mic.kind(), Kind::Mic);
        assert_eq!(Target::Focused.kind(), Kind::App);
        assert_eq!(Target::Pointed.kind(), Kind::App);
    }
}
