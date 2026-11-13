//! What the app remembers about how the user likes to work: never part of
//! `y-coustic.xml`, which y-coustic reads and a person may share. The screen
//! that was open, in `y-coustic-config.ini` next to it (the `.ini` format
//! `y_core::config` reads for the other apps).

use std::path::{Path, PathBuf};

use y_core::config::Config;

use crate::Screen;

pub const FILE_NAME: &str = "y-coustic-config.ini";

const SECTION: &str = "ui";

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Prefs {
    pub last_screen: Screen,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            last_screen: Screen::General,
        }
    }
}

/// The preferences file that goes with the configuration file at `config`.
pub fn path_for(config: &Path) -> PathBuf {
    config.with_file_name(FILE_NAME)
}

impl Prefs {
    /// Missing or unreadable values are the defaults: a preference is never
    /// worth refusing to start over.
    pub fn load(path: &Path) -> Self {
        Self::from_config(&Config::load_from(path.to_owned()))
    }

    /// Best effort: a read-only folder just means the preference isn't kept.
    pub fn save(&self, path: &Path) {
        let mut config = Config::load_from(path.to_owned());
        config.set(SECTION, "screen", self.last_screen.tag());
        let _ = config.save();
    }

    fn from_config(config: &Config) -> Self {
        Self {
            last_screen: config
                .get(SECTION, "screen")
                .and_then(Screen::from_tag)
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Config {
        Config::load_from(std::env::temp_dir().join("y-coustic-config-prefs-never-written.ini"))
    }

    #[test]
    fn nothing_remembered_opens_on_general() {
        assert_eq!(Prefs::from_config(&empty()).last_screen, Screen::General);
    }

    #[test]
    fn a_screen_round_trips_and_a_stale_one_is_general() {
        let mut config = empty();
        config.set(SECTION, "screen", "hud");
        assert_eq!(Prefs::from_config(&config).last_screen, Screen::Hud);
        config.set(SECTION, "screen", "gone");
        assert_eq!(Prefs::from_config(&config).last_screen, Screen::General);
    }

    #[test]
    fn saving_and_loading_through_a_file() {
        let directory =
            std::env::temp_dir().join(format!("y-coustic-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = path_for(&directory.join("y-coustic.xml"));
        Prefs {
            last_screen: Screen::Keys,
        }
        .save(&path);
        assert_eq!(Prefs::load(&path).last_screen, Screen::Keys);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
