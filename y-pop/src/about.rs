//! The text of the *About* balloon.
//!
//! A balloon body holds 255 UTF-16 units (see [`y_core::tray::Notification`]),
//! and the shell will not scroll it, so the text is built to a budget rather
//! than trusted to fit: the one line of unpredictable length, the
//! configuration path, takes whatever the other lines leave.

use std::path::Path;

/// What the balloon reports.
pub struct Facts<'a> {
    /// Tray icons (one per root of the configuration).
    pub icons: usize,
    /// Entries in the whole tree, groups and separators included.
    pub entries: usize,
    pub config: &'a Path,
    /// Resident memory in bytes, when it could be read.
    pub working_set: Option<usize>,
    /// Whether `y-pop-config.exe` sits next to us.
    pub settings_app: bool,
}

/// The body limit of a balloon, in UTF-16 units, less the terminating NUL.
const BODY_BUDGET: usize = 255;

/// Never squeeze the path below this: shorter says nothing.
const MIN_PATH: usize = 16;

pub fn body(facts: &Facts) -> String {
    let counts = format!("{} tray icons · {} entries", facts.icons, facts.entries);
    let memory = facts.working_set.map_or_else(
        || "Memory: unknown".to_owned(),
        |bytes| format!("Memory: {}", megabytes(bytes)),
    );
    let settings = if facts.settings_app {
        "Settings app: installed"
    } else {
        "Settings app: not installed"
    };

    let fixed = ["Tray menus, hotkeys and search", &counts, &memory, settings];

    // The path's line is `Config: ` + path, and there are four newlines
    // between five lines.
    const LABEL: &str = "Config: ";
    let others: usize = fixed.iter().map(|line| units(line)).sum();
    let room = BODY_BUDGET
        .saturating_sub(others + 4 + units(LABEL))
        .max(MIN_PATH);

    let config = format!(
        "{LABEL}{}",
        shorten_middle(&facts.config.display().to_string(), room)
    );

    [fixed[0], fixed[1], &config, fixed[2], fixed[3]].join("\n")
}

/// `text` cut to `max` characters by dropping the middle: the drive and the
/// file name are what identify a path, and they sit at its two ends. Two
/// thirds of what is kept goes to the tail.
fn shorten_middle(text: &str, max: usize) -> String {
    let characters: Vec<char> = text.chars().collect();
    if characters.len() <= max {
        return text.to_owned();
    }

    let kept = max.saturating_sub(1);
    let tail = kept * 2 / 3;
    let head = kept - tail;

    let mut shortened: String = characters[..head].iter().collect();
    shortened.push('…');
    shortened.extend(&characters[characters.len() - tail..]);
    shortened
}

fn megabytes(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn facts(config: &Path) -> Facts<'_> {
        Facts {
            icons: 2,
            entries: 31,
            config,
            working_set: Some(4_404_019),
            settings_app: true,
        }
    }

    #[test]
    fn a_short_path_is_shown_whole() {
        let path = PathBuf::from(r"C:\Users\me\y-pop.xml");
        let text = body(&facts(&path));
        assert!(text.contains(r"Config: C:\Users\me\y-pop.xml"));
        assert!(text.contains("2 tray icons · 31 entries"));
        assert!(text.contains("Memory: 4.2 MB"));
        assert!(text.ends_with("Settings app: installed"));
    }

    #[test]
    fn the_body_always_fits_a_balloon() {
        let long = PathBuf::from(format!(
            r"C:\{}\y-pop.xml",
            "very-long-folder-name\\".repeat(30)
        ));
        for facts in [
            facts(&long),
            Facts {
                working_set: None,
                settings_app: false,
                ..facts(&long)
            },
        ] {
            let text = body(&facts);
            assert!(units(&text) <= BODY_BUDGET, "{} units", units(&text));
            assert!(text.contains("y-pop.xml"), "{text}");
            assert!(text.contains(r"Config: C:\"), "{text}");
        }
    }

    #[test]
    fn shortening_keeps_both_ends() {
        assert_eq!(shorten_middle("short", 10), "short");
        let shortened = shorten_middle(r"C:\aaaaaaaaaaaaaaaaaaaa\y-pop.xml", 16);
        assert_eq!(shortened.chars().count(), 16);
        assert!(shortened.starts_with("C:"));
        assert!(shortened.ends_with("y-pop.xml"));
        assert!(shortened.contains('…'));
    }
}
