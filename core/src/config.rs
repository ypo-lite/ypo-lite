//! Settings in a plain `.ini` file, the way the GUIPro applications kept them.
//!
//! Deliberately hand-written rather than a serialisation crate: the whole
//! format is a dozen keys, a user is expected to open the file in Notepad,
//! and a parser that never fails — unreadable lines are kept verbatim and
//! written back — beats one that refuses to start over a stray character.
//!
//! ```ini
//! ; y-snap.ini
//! [hotkeys]
//! snap = Win+Shift+S
//!
//! [overlay]
//! opacity = 220
//! ```
//!
//! ```no_run
//! # use y_core::config::Config;
//! # use y_core::hotkey::Hotkey;
//! let mut config = Config::load("y-snap");
//! let hotkey = config.hotkey("hotkeys", "snap").unwrap_or_else(|| Hotkey::parse("Win+Shift+S").unwrap());
//! let opacity = config.int("overlay", "opacity").unwrap_or(220);
//!
//! config.set("overlay", "opacity", "255");
//! let _ = config.save();
//! ```

use std::path::{Path, PathBuf};

use crate::hotkey::Hotkey;

/// Key/value settings, section by section, in file order.
pub struct Config {
    path: PathBuf,
    lines: Vec<Line>,
}

/// The file is kept as a list of lines so that comments, blank lines and
/// anything unrecognised survive a load/save round trip.
enum Line {
    Section(String),
    Entry {
        section: String,
        key: String,
        value: String,
    },
    Verbatim(String),
}

impl Config {
    /// Loads the settings of `app` (`"y-snap"`), or an empty set if there are
    /// none yet. See [`path_for`] for where it looks.
    pub fn load(app: &str) -> Self {
        Self::load_from(path_for(app))
    }

    /// Loads a specific file. A missing or unreadable file yields an empty
    /// configuration that [`Config::save`] will create.
    pub fn load_from(path: PathBuf) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines = Vec::new();
        let mut section = String::new();

        for raw in text.lines() {
            let trimmed = raw.trim();

            if let Some(name) = trimmed
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                section = name.trim().to_ascii_lowercase();
                lines.push(Line::Section(section.clone()));
                continue;
            }

            // A comment (`;` or `#`), a blank line, or a line with no `=`:
            // nothing to interpret, everything to preserve.
            match trimmed.split_once('=') {
                Some((key, value)) if !trimmed.starts_with([';', '#']) => lines.push(Line::Entry {
                    section: section.clone(),
                    key: key.trim().to_ascii_lowercase(),
                    value: value.trim().to_owned(),
                }),
                _ => lines.push(Line::Verbatim(raw.to_owned())),
            }
        }

        Self { path, lines }
    }

    /// The file these settings came from, and where [`Config::save`] writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Raw value of `section.key`. Section and key are matched case
    /// insensitively.
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        let (section, key) = (section.to_ascii_lowercase(), key.to_ascii_lowercase());
        self.lines.iter().find_map(|line| match line {
            Line::Entry {
                section: s,
                key: k,
                value,
            } if *s == section && *k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// Value of `section.key` as an integer, or `None` if absent or not a
    /// number.
    pub fn int(&self, section: &str, key: &str) -> Option<i64> {
        self.get(section, key)?.parse().ok()
    }

    /// Value of `section.key` as a flag: `1`, `true`, `yes` and `on` are
    /// true; `0`, `false`, `no` and `off` are false; anything else is `None`.
    pub fn bool(&self, section: &str, key: &str) -> Option<bool> {
        match self.get(section, key)?.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    /// Value of `section.key` as a key combination — see [`Hotkey::parse`].
    pub fn hotkey(&self, section: &str, key: &str) -> Option<Hotkey> {
        Hotkey::parse(self.get(section, key)?)
    }

    /// Sets `section.key`, in place if it exists, appended to the section (or
    /// to a new section) otherwise. Call [`Config::save`] to persist it.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let (section, key) = (section.to_ascii_lowercase(), key.to_ascii_lowercase());

        // Existing entry: overwrite in place, keeping the file's layout.
        for line in &mut self.lines {
            if let Line::Entry {
                section: s,
                key: k,
                value: v,
            } = line
                && *s == section
                && *k == key
            {
                *v = value.to_owned();
                return;
            }
        }

        let entry = Line::Entry {
            section: section.clone(),
            key,
            value: value.to_owned(),
        };

        // New key in an existing section: insert after that section's last
        // entry, so entries stay grouped under their header.
        let last = self.lines.iter().rposition(|line| match line {
            Line::Section(name) => *name == section,
            Line::Entry { section: s, .. } => *s == section,
            Line::Verbatim(_) => false,
        });

        match last {
            Some(index) => self.lines.insert(index + 1, entry),
            None => {
                if !self.lines.is_empty() {
                    self.lines.push(Line::Verbatim(String::new()));
                }
                self.lines.push(Line::Section(section));
                self.lines.push(entry);
            }
        }
    }

    /// Removes `section.key` if it is there.
    pub fn remove(&mut self, section: &str, key: &str) {
        let (section, key) = (section.to_ascii_lowercase(), key.to_ascii_lowercase());
        self.lines.retain(|line| {
            !matches!(line, Line::Entry { section: s, key: k, .. } if *s == section && *k == key)
        });
    }

    /// Renders the file as it would be written.
    pub fn to_ini(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            match line {
                Line::Section(name) => text.push_str(&format!("[{name}]\n")),
                Line::Entry { key, value, .. } => text.push_str(&format!("{key} = {value}\n")),
                Line::Verbatim(raw) => {
                    text.push_str(raw);
                    text.push('\n');
                }
            }
        }
        text
    }

    /// Writes the settings back, creating the directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, self.to_ini())
    }
}

/// Where `app`'s settings live.
///
/// Portable first: an `.ini` sitting next to the executable wins, so the
/// application can be carried on a USB stick with its settings — that is what
/// these tools are for. Otherwise `%APPDATA%\Ypo-Lite\<app>.ini`, which is
/// writable even when the executable is in `Program Files`.
pub fn path_for(app: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        let portable = directory.join(format!("{app}.ini"));
        if portable.is_file() {
            return portable;
        }
    }

    match std::env::var_os("APPDATA") {
        Some(appdata) => PathBuf::from(appdata)
            .join("Ypo-Lite")
            .join(format!("{app}.ini")),
        None => PathBuf::from(format!("{app}.ini")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips `text` through a real file: the parser only exists behind
    /// [`Config::load_from`], and there is no point testing a copy of it.
    fn parse(name: &str, text: &str) -> Config {
        let path = std::env::temp_dir().join(format!("y-core-{name}.ini"));
        std::fs::write(&path, text).expect("write the test file");
        Config::load_from(path)
    }

    #[test]
    fn reads_values() {
        let config = parse("read", "; comment\n[Overlay]\nOpacity = 220\nenabled=yes\n");
        assert_eq!(config.get("overlay", "opacity"), Some("220"));
        assert_eq!(config.int("overlay", "opacity"), Some(220));
        assert_eq!(config.bool("overlay", "enabled"), Some(true));
        assert_eq!(config.get("overlay", "missing"), None);
    }

    #[test]
    fn keeps_comments_and_updates_in_place() {
        let mut config = parse("keep", "; keep me\n[overlay]\nopacity = 220\n");
        config.set("overlay", "opacity", "255");
        assert_eq!(config.to_ini(), "; keep me\n[overlay]\nopacity = 255\n");
    }

    #[test]
    fn appends_new_sections_and_keys() {
        let mut config = parse("append", "[overlay]\nopacity = 220\n");
        config.set("overlay", "radius", "8");
        config.set("hotkeys", "snap", "Win+Shift+S");
        assert_eq!(
            config.to_ini(),
            "[overlay]\nopacity = 220\nradius = 8\n\n[hotkeys]\nsnap = Win+Shift+S\n"
        );
    }
}
