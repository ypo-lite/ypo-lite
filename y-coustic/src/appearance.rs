//! What the tray icon looks like, decided from what the audio devices say.
//!
//! The icon is the same whatever happens, unless `<tray icon>` names a rule.
//! A rule is a pure decision: [`decide`] takes the rule and what was read from
//! the devices ([`Inputs`]) and answers with an [`Appearance`]; the application
//! only reads the devices a rule needs ([`Rule::reads_microphone`]) and puts the
//! answer on the icon when it differs from what is there. Rules for the master
//! and the microphone levels (a different icon per volume range, say) come
//! here: a new [`Rule`], a new [`Appearance`], the field of [`Inputs`] they read,
//! and a branch in [`decide`], without touching the message loop.

use crate::model::State;

/// How the tray icon is chosen: `<tray icon>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Rule {
    /// Always the application's icon.
    #[default]
    Fixed,
    /// The muted variant while the microphone is muted.
    MicMuted,
}

impl Rule {
    /// The name used in the configuration file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::MicMuted => "mic-muted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "fixed" => Self::Fixed,
            "mic-muted" => Self::MicMuted,
            _ => return None,
        })
    }

    /// Whether the rule looks at the microphone: with none that does, nothing
    /// needs reading when an endpoint changes.
    pub fn reads_microphone(self) -> bool {
        matches!(self, Self::MicMuted)
    }
}

/// What the rules can look at; what a rule does not read may stay `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Inputs {
    /// The default microphone, `None` when there is none or it cannot be read.
    pub microphone: Option<State>,
}

/// An icon the tray can show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Appearance {
    /// The application's own icon.
    #[default]
    Normal,
    /// The microphone is muted.
    MicMuted,
}

/// The appearance `rule` gives for `inputs`.
pub fn decide(rule: Rule, inputs: &Inputs) -> Appearance {
    match rule {
        Rule::Fixed => Appearance::Normal,
        Rule::MicMuted if inputs.microphone.is_some_and(|state| state.muted) => {
            Appearance::MicMuted
        }
        Rule::MicMuted => Appearance::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mic(muted: bool) -> Inputs {
        Inputs {
            microphone: Some(State { level: 0.5, muted }),
        }
    }

    #[test]
    fn the_icon_is_fixed_unless_a_rule_says_otherwise() {
        assert_eq!(Rule::default(), Rule::Fixed);
        assert!(!Rule::Fixed.reads_microphone());
        assert_eq!(decide(Rule::Fixed, &mic(true)), Appearance::Normal);
        assert_eq!(decide(Rule::Fixed, &Inputs::default()), Appearance::Normal);
    }

    #[test]
    fn mic_muted_follows_the_microphone() {
        assert_eq!(decide(Rule::MicMuted, &mic(true)), Appearance::MicMuted);
        assert_eq!(decide(Rule::MicMuted, &mic(false)), Appearance::Normal);
        // No microphone, or one that cannot be read: nothing to warn about.
        assert_eq!(
            decide(Rule::MicMuted, &Inputs::default()),
            Appearance::Normal
        );
    }

    #[test]
    fn names_round_trip() {
        for rule in [Rule::Fixed, Rule::MicMuted] {
            assert_eq!(Rule::from_name(rule.name()), Some(rule));
        }
        assert_eq!(Rule::from_name("sparkle"), None);
    }
}
