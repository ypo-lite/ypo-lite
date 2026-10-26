//! Keys and shortcuts of the search box, with no Win32: the window turns
//! virtual-key codes into these, and the configuration writes shortcuts as
//! text (`Ctrl+Shift+Enter`) that [`Chord::parse`] reads.

use std::fmt;

/// Modifier keys held when a key went down.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// A key the box reacts to. Typed characters are `Event::Char`, not keys;
/// `Letter`, `Digit` and `Function` exist for shortcuts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    /// An upper-case letter.
    Letter(char),
    /// A digit `0`–`9` of the number row.
    Digit(u8),
    /// `F1`–`F12`.
    Function(u8),
    /// The Menu key, or `Shift+F10`: "open the context menu". Not a shortcut
    /// that can be configured; the action menu has its own `menu-key`.
    ContextMenu,
}

/// A shortcut: a key and the modifiers held with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

/// Why a shortcut was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ChordError {
    Empty,
    /// A key name nothing here knows, or one that cannot be a shortcut
    /// (`Esc`, `Tab`, the arrows…).
    UnknownKey(String),
    /// Two keys (`Ctrl+A+B`).
    TwoKeys,
    /// Only modifiers (`Ctrl+Alt`).
    NoKey,
    /// A letter, digit or `Delete` alone (or with Shift) would be typing.
    NeedsModifier,
    /// The key does something in the box already.
    Reserved(&'static str),
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "is empty"),
            Self::UnknownKey(key) => write!(f, "{key:?} cannot be part of a shortcut"),
            Self::TwoKeys => write!(f, "has two keys"),
            Self::NoKey => write!(f, "has no key, only modifiers"),
            Self::NeedsModifier => write!(f, "needs Ctrl or Alt, or it would be typing"),
            Self::Reserved(reason) => write!(f, "is taken: {reason}"),
        }
    }
}

impl Chord {
    pub const fn new(key: Key, mods: Mods) -> Self {
        Self { key, mods }
    }

    /// `Enter`, `Ctrl+E`, `Ctrl+Shift+Enter`, `Alt+Enter`, `F5`, `Ctrl+Del`.
    /// Case and order of modifiers do not matter. Only shortcuts that can
    /// work are accepted: not a bare letter, not what already edits the box
    /// (`Ctrl+A/C/V/X`), not `Alt+1…9` (quick pick) or `Alt+F4`.
    pub fn parse(text: &str) -> Result<Self, ChordError> {
        let mut mods = Mods::default();
        let mut key = None;

        for token in text.split('+').map(str::trim) {
            if token.is_empty() {
                return Err(if text.trim().is_empty() {
                    ChordError::Empty
                } else {
                    ChordError::UnknownKey(String::new())
                });
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" => mods.alt = true,
                name => {
                    if key.is_some() {
                        return Err(ChordError::TwoKeys);
                    }
                    key = Some(
                        key_named(name).ok_or_else(|| ChordError::UnknownKey(token.to_owned()))?,
                    );
                }
            }
        }
        let key = key.ok_or(ChordError::NoKey)?;
        let chord = Self { key, mods };
        chord.check()?;
        Ok(chord)
    }

    fn check(&self) -> Result<(), ChordError> {
        let Mods {
            ctrl,
            shift: _,
            alt,
        } = self.mods;
        match self.key {
            Key::Letter(letter) => {
                if !(ctrl || alt) {
                    return Err(ChordError::NeedsModifier);
                }
                if ctrl && !alt && !self.mods.shift && "ACVX".contains(letter) {
                    return Err(ChordError::Reserved("it edits the text"));
                }
            }
            Key::Digit(_) => {
                if !(ctrl || alt) {
                    return Err(ChordError::NeedsModifier);
                }
                if alt && !ctrl {
                    return Err(ChordError::Reserved("Alt+1…9 picks a result"));
                }
            }
            Key::Delete if !(ctrl || alt) => return Err(ChordError::NeedsModifier),
            Key::Function(4) if alt => return Err(ChordError::Reserved("Alt+F4 closes windows")),
            _ => {}
        }
        Ok(())
    }
}

/// The key a name stands for, among those a shortcut may use.
fn key_named(name: &str) -> Option<Key> {
    match name {
        "enter" | "return" => return Some(Key::Enter),
        "delete" | "del" => return Some(Key::Delete),
        _ => {}
    }
    let mut chars = name.chars();
    match (chars.next(), chars.as_str()) {
        (Some(letter), "") if letter.is_ascii_alphabetic() => {
            Some(Key::Letter(letter.to_ascii_uppercase()))
        }
        (Some(digit), "") if digit.is_ascii_digit() => Some(Key::Digit(digit as u8 - b'0')),
        (Some('f'), number) => match number.parse::<u8>() {
            Ok(number @ 1..=12) => Some(Key::Function(number)),
            _ => None,
        },
        _ => None,
    }
}

/// Written as [`Chord::parse`] reads it: `Ctrl+Alt+Shift+Key`.
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.mods.ctrl, "Ctrl"),
            (self.mods.alt, "Alt"),
            (self.mods.shift, "Shift"),
        ] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Enter => write!(f, "Enter"),
            Key::Delete => write!(f, "Del"),
            Key::Letter(letter) => write!(f, "{letter}"),
            Key::Digit(digit) => write!(f, "{digit}"),
            Key::Function(number) => write!(f, "F{number}"),
            // Not shortcuts; for completeness of the debug-ish output.
            other => write!(f, "{other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Chord {
        Chord::parse(text).unwrap_or_else(|error| panic!("{text:?}: {error}"))
    }

    fn mods(ctrl: bool, shift: bool, alt: bool) -> Mods {
        Mods { ctrl, shift, alt }
    }

    #[test]
    fn shortcuts_parse_whatever_the_case_or_the_order() {
        assert_eq!(
            ok("Enter"),
            Chord::new(Key::Enter, mods(false, false, false))
        );
        assert_eq!(
            ok("ctrl+e"),
            Chord::new(Key::Letter('E'), mods(true, false, false))
        );
        assert_eq!(ok("Shift + Ctrl + Enter"), ok("ctrl+shift+enter"));
        assert_eq!(ok("Alt+Return"), ok("Alt+Enter"));
        assert_eq!(
            ok("Ctrl+Del"),
            Chord::new(Key::Delete, mods(true, false, false))
        );
        assert_eq!(
            ok("F5"),
            Chord::new(Key::Function(5), mods(false, false, false))
        );
        assert_eq!(
            ok("Ctrl+0"),
            Chord::new(Key::Digit(0), mods(true, false, false))
        );
        assert_eq!(ok("Ctrl+Alt+K").mods, mods(true, false, true));
    }

    #[test]
    fn a_shortcut_is_written_back_as_it_reads() {
        for text in [
            "Enter",
            "Ctrl+E",
            "Ctrl+Alt+Shift+Enter",
            "Alt+Enter",
            "F12",
            "Ctrl+Del",
            "Ctrl+7",
        ] {
            assert_eq!(ok(text).to_string(), text);
            assert_eq!(ok(&ok(text).to_string()), ok(text));
        }
        assert_eq!(ok("shift+ctrl+enter").to_string(), "Ctrl+Shift+Enter");
    }

    #[test]
    fn bad_shortcuts_say_why() {
        assert_eq!(Chord::parse(""), Err(ChordError::Empty));
        assert_eq!(Chord::parse("  "), Err(ChordError::Empty));
        assert_eq!(Chord::parse("Ctrl+Alt"), Err(ChordError::NoKey));
        assert_eq!(Chord::parse("Ctrl+A+B"), Err(ChordError::TwoKeys));
        assert!(matches!(
            Chord::parse("Ctrl+Esc"),
            Err(ChordError::UnknownKey(_))
        ));
        assert!(matches!(
            Chord::parse("Ctrl+Up"),
            Err(ChordError::UnknownKey(_))
        ));
        assert!(matches!(
            Chord::parse("F13"),
            Err(ChordError::UnknownKey(_))
        ));
        assert!(matches!(
            Chord::parse("Ctrl+"),
            Err(ChordError::UnknownKey(_))
        ));
    }

    #[test]
    fn typing_keys_need_a_modifier() {
        assert_eq!(Chord::parse("E"), Err(ChordError::NeedsModifier));
        assert_eq!(Chord::parse("Shift+E"), Err(ChordError::NeedsModifier));
        assert_eq!(Chord::parse("5"), Err(ChordError::NeedsModifier));
        assert_eq!(Chord::parse("Del"), Err(ChordError::NeedsModifier));
        assert_eq!(Chord::parse("Shift+Del"), Err(ChordError::NeedsModifier));
        // These do not type anything.
        ok("Enter");
        ok("Shift+Enter");
        ok("F2");
    }

    #[test]
    fn what_the_box_already_does_is_taken() {
        for text in [
            "Ctrl+A", "Ctrl+C", "Ctrl+V", "Ctrl+X", "Alt+1", "Alt+9", "Alt+F4",
        ] {
            assert!(
                matches!(Chord::parse(text), Err(ChordError::Reserved(_))),
                "{text}"
            );
        }
        // A neighbour of a taken key is free.
        ok("Ctrl+Shift+C");
        ok("Ctrl+Alt+1");
        ok("Ctrl+1");
        ok("Alt+F5");
    }
}
