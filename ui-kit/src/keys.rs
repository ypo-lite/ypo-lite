//! The key catalog behind the key picker: every key a hotkey or tile can
//! be bound to, grouped the way people look for them, with the label
//! printed on the key (`'`, not `OEM7`).
//!
//! Each entry is `(label, name)`, where `name` is the spelling
//! [`y_core::hotkey::key_from_name`] reads from `y-snap.xml` — so the
//! virtual-key code comes from the same table the config file uses, and
//! the `every_catalog_name_resolves` test catches a typo here.
//!
//! Compared with the old on-screen keyboard this also reaches F13–F24 and
//! the media/browser keys, which had no place on a keyboard-shaped layout.

use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
use y_core::hotkey::{key_from_name, key_name};

/// A tab of the key picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KeyCategory {
    #[default]
    Letters,
    Digits,
    Function,
    Navigation,
    Editing,
    Symbols,
    Numpad,
    Media,
}

type Entry = (&'static str, &'static str);

const LETTERS: &[Entry] = &[
    ("A", "A"),
    ("B", "B"),
    ("C", "C"),
    ("D", "D"),
    ("E", "E"),
    ("F", "F"),
    ("G", "G"),
    ("H", "H"),
    ("I", "I"),
    ("J", "J"),
    ("K", "K"),
    ("L", "L"),
    ("M", "M"),
    ("N", "N"),
    ("O", "O"),
    ("P", "P"),
    ("Q", "Q"),
    ("R", "R"),
    ("S", "S"),
    ("T", "T"),
    ("U", "U"),
    ("V", "V"),
    ("W", "W"),
    ("X", "X"),
    ("Y", "Y"),
    ("Z", "Z"),
];
const DIGITS: &[Entry] = &[
    ("1", "1"),
    ("2", "2"),
    ("3", "3"),
    ("4", "4"),
    ("5", "5"),
    ("6", "6"),
    ("7", "7"),
    ("8", "8"),
    ("9", "9"),
    ("0", "0"),
];
const FUNCTION: &[Entry] = &[
    ("F1", "F1"),
    ("F2", "F2"),
    ("F3", "F3"),
    ("F4", "F4"),
    ("F5", "F5"),
    ("F6", "F6"),
    ("F7", "F7"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
    ("F11", "F11"),
    ("F12", "F12"),
    ("F13", "F13"),
    ("F14", "F14"),
    ("F15", "F15"),
    ("F16", "F16"),
    ("F17", "F17"),
    ("F18", "F18"),
    ("F19", "F19"),
    ("F20", "F20"),
    ("F21", "F21"),
    ("F22", "F22"),
    ("F23", "F23"),
    ("F24", "F24"),
];
const NAVIGATION: &[Entry] = &[
    ("←", "Left"),
    ("↑", "Up"),
    ("↓", "Down"),
    ("→", "Right"),
    ("Home", "Home"),
    ("End", "End"),
    ("PgUp", "PageUp"),
    ("PgDn", "PageDown"),
    ("Ins", "Insert"),
    ("Del", "Delete"),
];
const EDITING: &[Entry] = &[
    ("Esc", "Esc"),
    ("Tab", "Tab"),
    ("Space", "Space"),
    ("Enter", "Enter"),
    ("Backspace", "Backspace"),
    ("Caps Lock", "CapsLock"),
    ("PrtScn", "PrintScreen"),
    ("Scroll Lock", "ScrollLock"),
    ("Pause", "Pause"),
];
// US-layout positions — see `key_from_name`'s own note on what the OEM
// names mean on other layouts.
const SYMBOLS: &[Entry] = &[
    ("`", "OEM3"),
    ("-", "Minus"),
    ("=", "Plus"),
    ("[", "OEM4"),
    ("]", "OEM6"),
    ("\\", "OEM5"),
    (";", "OEM1"),
    ("'", "OEM7"),
    (",", "Comma"),
    (".", "Period"),
    ("/", "OEM2"),
    ("< (ISO)", "<"),
];
const NUMPAD: &[Entry] = &[
    ("Num 0", "Numpad0"),
    ("Num 1", "Numpad1"),
    ("Num 2", "Numpad2"),
    ("Num 3", "Numpad3"),
    ("Num 4", "Numpad4"),
    ("Num 5", "Numpad5"),
    ("Num 6", "Numpad6"),
    ("Num 7", "Numpad7"),
    ("Num 8", "Numpad8"),
    ("Num 9", "Numpad9"),
    ("Num +", "NumAdd"),
    ("Num −", "NumSub"),
    ("Num ×", "NumMul"),
    ("Num ÷", "NumDiv"),
    ("Num .", "NumDecimal"),
    ("Num Lock", "NumLock"),
];
const MEDIA: &[Entry] = &[
    ("Play/Pause", "MediaPlayPause"),
    ("Next track", "MediaNext"),
    ("Prev track", "MediaPrev"),
    ("Stop", "MediaStop"),
    ("Mute", "VolumeMute"),
    ("Volume −", "VolumeDown"),
    ("Volume +", "VolumeUp"),
    ("Browser back", "BrowserBack"),
    ("Browser fwd", "BrowserForward"),
    ("Refresh", "BrowserRefresh"),
    ("Search", "BrowserSearch"),
    ("Favorites", "BrowserFavorites"),
    ("Home page", "BrowserHome"),
    ("Mail", "LaunchMail"),
    ("Media player", "LaunchMedia"),
    ("App 1", "LaunchApp1"),
    ("App 2", "LaunchApp2"),
];

impl KeyCategory {
    pub const ALL: [Self; 8] = [
        Self::Letters,
        Self::Digits,
        Self::Function,
        Self::Navigation,
        Self::Editing,
        Self::Symbols,
        Self::Numpad,
        Self::Media,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Letters => "Letters",
            Self::Digits => "Digits",
            Self::Function => "F-keys",
            Self::Navigation => "Navigation",
            Self::Editing => "Editing",
            Self::Symbols => "Symbols",
            Self::Numpad => "Numpad",
            Self::Media => "Media",
        }
    }

    pub fn from_title(title: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.title() == title)
    }

    fn entries(self) -> &'static [Entry] {
        match self {
            Self::Letters => LETTERS,
            Self::Digits => DIGITS,
            Self::Function => FUNCTION,
            Self::Navigation => NAVIGATION,
            Self::Editing => EDITING,
            Self::Symbols => SYMBOLS,
            Self::Numpad => NUMPAD,
            Self::Media => MEDIA,
        }
    }

    /// This category's keys, as `(label, key)`.
    pub fn keys(self) -> impl Iterator<Item = (&'static str, VIRTUAL_KEY)> {
        self.entries()
            .iter()
            .filter_map(|&(label, name)| Some((label, key_from_name(name)?)))
    }

    /// Narrowest a key button of this category can be and still show its
    /// longest label — every button in one category gets the same width, so
    /// a grid of them stays a grid.
    pub fn min_button_width(self) -> f64 {
        let longest = self
            .entries()
            .iter()
            .map(|(label, _)| label.chars().count())
            .max()
            .unwrap_or(1);
        (longest as f64 * 8.0 + 24.0).max(44.0)
    }

    /// The category `key` is listed under, if any.
    pub fn of(key: VIRTUAL_KEY) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.keys().any(|(_, k)| k == key))
    }
}

/// What to print on a key's cap: its catalog label, or the config-file name
/// for a key the catalog doesn't list (a raw `0x..` code from a hand-edited
/// file still displays, as itself).
pub fn label(key: VIRTUAL_KEY) -> String {
    KeyCategory::ALL
        .into_iter()
        .flat_map(KeyCategory::keys)
        .find(|&(_, k)| k == key)
        .map(|(label, _)| label.to_owned())
        .unwrap_or_else(|| key_name(key))
}

/// Search suggestions for `query`: every key whose label or config name
/// contains it (case-insensitive), as `"label — Category"` strings for the
/// suggestion list. Empty query, no suggestions.
pub fn search(query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    KeyCategory::ALL
        .into_iter()
        .flat_map(|category| {
            category
                .entries()
                .iter()
                .map(move |entry| (category, entry))
        })
        .filter(|(_, (label, name))| {
            label.to_lowercase().contains(&query) || name.to_lowercase().contains(&query)
        })
        .map(|(category, (label, _))| suggestion(label, category))
        .take(12)
        .collect()
}

/// The key a [`search`] suggestion stands for.
pub fn from_suggestion(text: &str) -> Option<VIRTUAL_KEY> {
    KeyCategory::ALL
        .into_iter()
        .flat_map(|category| {
            category
                .keys()
                .map(move |(label, key)| (category, label, key))
        })
        .find(|&(category, label, _)| suggestion(label, category) == text)
        .map(|(_, _, key)| key)
}

fn suggestion(label: &str, category: KeyCategory) -> String {
    format!("{label} — {}", category.title())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_name_resolves() {
        for category in KeyCategory::ALL {
            for (label, name) in category.entries() {
                assert!(
                    key_from_name(name).is_some(),
                    "{label:?} ({name}) in {category:?}"
                );
            }
        }
    }

    #[test]
    fn no_key_is_listed_twice() {
        let mut seen = Vec::new();
        for category in KeyCategory::ALL {
            for (label, key) in category.keys() {
                assert!(
                    !seen.contains(&key.0),
                    "{label:?} in {category:?} duplicates another entry"
                );
                seen.push(key.0);
            }
        }
    }

    #[test]
    fn labels_are_printed_characters_not_oem_names() {
        assert_eq!(label(key_from_name("OEM7").unwrap()), "'");
        assert_eq!(label(key_from_name("F13").unwrap()), "F13");
        assert_eq!(label(VIRTUAL_KEY(0xE8)), "0xE8");
    }

    #[test]
    fn a_suggestion_round_trips_to_its_key() {
        let suggestions = search("pgd");
        assert_eq!(suggestions, ["PgDn — Navigation"]);
        assert_eq!(from_suggestion(&suggestions[0]), key_from_name("PageDown"));
    }
}
