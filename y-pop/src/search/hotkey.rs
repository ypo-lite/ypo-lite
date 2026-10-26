//! A key combination as a *set*, for searching by hotkey (spec §2.6): a
//! query that starts with a modifier — `win`, `ctrl`, `alt`, `shift` — is
//! read as a combination, so `shift+ctrl+esc` finds `Ctrl+Shift+Esc`
//! whatever order either was written in, and `ctrl+alt` finds every hotkey
//! holding both.

use y_core::hotkey::{key_from_name, key_name};

const CTRL: u8 = 1;
const ALT: u8 = 2;
const SHIFT: u8 = 4;
const WIN: u8 = 8;

/// Modifiers as a bit set, plus at most one other key, lower-cased and in the
/// canonical spelling of `y_core::hotkey::key_name` (`escape` → `esc`).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeyCombo {
    mods: u8,
    key: Option<Box<str>>,
}

fn modifier(token: &str) -> Option<u8> {
    match token {
        "ctrl" | "control" => Some(CTRL),
        "alt" => Some(ALT),
        "shift" => Some(SHIFT),
        "win" | "windows" | "super" | "meta" => Some(WIN),
        _ => None,
    }
}

impl KeyCombo {
    /// `Win+Shift+F12`, or something a user is still typing (`win+`, `ctrl+es`).
    /// Spaces are ignored; `None` for text with no key at all or two keys.
    pub fn parse(text: &str) -> Option<Self> {
        let mut combo = Self::default();
        let mut any = false;
        for token in text.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            any = true;
            let lower = token.to_lowercase();
            match modifier(&lower) {
                Some(bit) => combo.mods |= bit,
                None if combo.key.is_some() => return None,
                None => {
                    // A complete key name is spelled as `key_name` would; a
                    // part of one stays as typed, to be matched as a prefix.
                    let canonical =
                        key_from_name(token).map_or(lower, |key| key_name(key).to_lowercase());
                    combo.key = Some(canonical.into_boxed_str());
                }
            }
        }
        any.then_some(combo)
    }

    /// Whether `text` should be read as a hotkey: its first word is a
    /// modifier name. `win` alone is, and so is `ctrl+`; `wind` is not.
    pub fn is_query(text: &str) -> bool {
        let first = text.split('+').next().unwrap_or("");
        modifier(&first.trim().to_lowercase()).is_some()
    }

    /// How well a typed `self` matches the hotkey `entry`, `0..=1000`, or
    /// `None`: every modifier typed must be in the entry, and a typed key
    /// must begin the entry's key. The same set scores highest; the same
    /// modifiers with the key still being typed next; a partial combination
    /// last.
    pub fn match_score(&self, entry: &KeyCombo) -> Option<u32> {
        if self.mods & !entry.mods != 0 {
            return None;
        }
        let key_matches = match (&self.key, &entry.key) {
            (None, _) => true,
            (Some(typed), Some(key)) => key.starts_with(&**typed),
            (Some(_), None) => false,
        };
        if !key_matches {
            return None;
        }
        Some(if self.mods == entry.mods && self.key == entry.key {
            1000
        } else if self.mods == entry.mods {
            900
        } else {
            700
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(text: &str) -> KeyCombo {
        KeyCombo::parse(text).expect("a combination")
    }

    #[test]
    fn order_and_case_do_not_matter() {
        assert_eq!(combo("shift+ctrl+esc"), combo("Ctrl+Shift+Esc"));
        assert_eq!(combo("Control + Escape"), combo("ctrl+esc"));
        assert_eq!(combo("Windows+N"), combo("win+n"));
    }

    #[test]
    fn two_keys_or_nothing_is_not_a_combination() {
        assert_eq!(KeyCombo::parse("ctrl+a+b"), None);
        assert_eq!(KeyCombo::parse(""), None);
        assert_eq!(KeyCombo::parse("+"), None);
    }

    #[test]
    fn only_a_leading_modifier_makes_a_hotkey_query() {
        assert!(KeyCombo::is_query("win"));
        assert!(KeyCombo::is_query("Ctrl+"));
        assert!(KeyCombo::is_query("shift+ctrl+esc"));
        assert!(!KeyCombo::is_query("wind"));
        assert!(!KeyCombo::is_query("esc"));
        assert!(!KeyCombo::is_query(""));
    }

    #[test]
    fn a_typed_combination_matches_by_modifiers_and_key_prefix() {
        let task_manager = combo("Ctrl+Shift+Esc");
        let exact = combo("shift+ctrl+esc").match_score(&task_manager);
        let typing = combo("ctrl+shift+es").match_score(&task_manager);
        let partial = combo("ctrl").match_score(&task_manager);
        assert_eq!(exact, Some(1000));
        assert_eq!(typing, Some(900));
        assert_eq!(partial, Some(700));

        assert_eq!(combo("alt").match_score(&task_manager), None, "not held");
        assert_eq!(combo("ctrl+x").match_score(&task_manager), None);
        assert_eq!(combo("ctrl+shift+esc+").match_score(&task_manager), exact);
    }

    #[test]
    fn a_key_typed_with_modifiers_missing_still_needs_every_typed_modifier() {
        // `win+n` does not find `Win+Shift+N` as an exact match, but does as a
        // looser one: the typed modifiers are a subset of the entry's.
        let entry = combo("Win+Shift+N");
        assert_eq!(combo("win+n").match_score(&entry), Some(700));
    }
}
