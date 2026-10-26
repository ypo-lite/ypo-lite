//! Is this key combination free? (spec §2.14). The answer comes from asking the
//! system — the window tries `RegisterHotKey` and lets go at once — and, for a
//! combination that fails, this table of the keys Windows keeps for itself, so
//! the row can say who has it.

use super::hotkey::KeyCombo;

/// What a combination is, as far as y-pop can tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Availability {
    /// The system registered it for a moment: nothing holds it.
    Free,
    /// Refused, and Windows is known to keep it for this.
    Reserved(&'static str),
    /// Refused, by a program Windows does not name.
    Taken,
}

/// The combinations Windows (and its shell) keep, with what they do.
const RESERVED: &[(&str, &str)] = &[
    ("win+e", "File Explorer (Windows)"),
    ("win+r", "Run (Windows)"),
    ("win+d", "Show desktop (Windows)"),
    ("win+l", "Lock the PC (Windows)"),
    ("win+tab", "Task view (Windows)"),
    ("win+i", "Settings (Windows)"),
    ("win+x", "the Quick Link menu (Windows)"),
    ("win+s", "Search (Windows)"),
    ("win+a", "Quick settings (Windows)"),
    ("win+n", "Notifications (Windows)"),
    ("win+w", "Widgets (Windows)"),
    ("win+v", "Clipboard history (Windows)"),
    ("win+shift+s", "Screenshot (Windows)"),
    ("win+m", "Minimize all windows (Windows)"),
    ("win+p", "Project to a screen (Windows)"),
    ("win+k", "Cast (Windows)"),
    ("win+g", "Game Bar (Windows)"),
    ("win+u", "Accessibility (Windows)"),
    ("win+space", "Switch input language (Windows)"),
    ("ctrl+shift+esc", "Task Manager (Windows)"),
    ("ctrl+alt+del", "the secure attention sequence (Windows)"),
    ("alt+tab", "Switch windows (Windows)"),
    ("alt+f4", "Close the window (Windows)"),
];

/// Who Windows keeps `combo` for, if it is in the table.
pub fn reserved_for(combo: &KeyCombo) -> Option<&'static str> {
    RESERVED
        .iter()
        .find(|(text, _)| KeyCombo::parse(text).as_ref() == Some(combo))
        .map(|&(_, owner)| owner)
}

/// What to tell about a combination the system refused (`registered` false) or
/// let through (`true`).
pub fn availability(combo: &KeyCombo, registered: bool) -> Availability {
    if registered {
        Availability::Free
    } else {
        reserved_for(combo).map_or(Availability::Taken, Availability::Reserved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(text: &str) -> KeyCombo {
        KeyCombo::parse(text).unwrap()
    }

    #[test]
    fn the_table_is_read_whatever_the_order_or_case() {
        assert_eq!(
            reserved_for(&combo("Win+E")),
            Some("File Explorer (Windows)")
        );
        assert_eq!(
            reserved_for(&combo("shift+win+s")),
            Some("Screenshot (Windows)")
        );
        assert_eq!(reserved_for(&combo("Ctrl+Alt+X")), None);
    }

    #[test]
    fn every_entry_of_the_table_parses_and_is_unique() {
        let mut seen: Vec<KeyCombo> = Vec::new();
        for (text, _) in RESERVED {
            let parsed = KeyCombo::parse(text).unwrap_or_else(|| panic!("{text} does not parse"));
            assert!(!seen.contains(&parsed), "{text} twice");
            seen.push(parsed);
        }
    }

    #[test]
    fn a_refused_key_is_named_when_known() {
        assert_eq!(availability(&combo("ctrl+alt+x"), true), Availability::Free);
        assert_eq!(
            availability(&combo("ctrl+alt+x"), false),
            Availability::Taken
        );
        assert_eq!(
            availability(&combo("win+e"), false),
            Availability::Reserved("File Explorer (Windows)")
        );
    }
}
