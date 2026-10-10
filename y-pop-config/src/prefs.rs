//! What the app remembers about how the user likes to work — never part of
//! `y-pop.xml`, which y-pop reads and a person may share: "show comments"
//! and the screen that was open, in `y-pop-config.ini` next to it (the
//! `.ini` format `y_core::config` reads for the other apps).

use std::path::{Path, PathBuf};

use y_core::config::Config;

pub const FILE_NAME: &str = "y-pop-config.ini";

const SECTION: &str = "ui";

/// The screen to open on, in terms that survive a reload: a tray menu is
/// remembered by its place among the roots, since ids only live as long as
/// one load.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LastScreen {
    #[default]
    General,
    Skin,
    Variables,
    Menu(usize),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Prefs {
    /// Whether comments are shown as rows (tree, variables, file notes).
    pub show_comments: bool,
    pub last_screen: LastScreen,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            show_comments: true,
            last_screen: LastScreen::General,
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
        self.write_to(&mut config);
        let _ = config.save();
    }

    fn from_config(config: &Config) -> Self {
        let defaults = Self::default();
        Self {
            show_comments: config
                .bool(SECTION, "show-comments")
                .unwrap_or(defaults.show_comments),
            last_screen: config
                .get(SECTION, "screen")
                .and_then(LastScreen::parse)
                .unwrap_or_default(),
        }
    }

    fn write_to(&self, config: &mut Config) {
        config.set(SECTION, "show-comments", if self.show_comments { "true" } else { "false" });
        config.set(SECTION, "screen", &self.last_screen.format());
    }
}

impl LastScreen {
    fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "general" => Some(Self::General),
            "skin" => Some(Self::Skin),
            "variables" => Some(Self::Variables),
            other => other.strip_prefix("menu:")?.parse().ok().map(Self::Menu),
        }
    }

    fn format(self) -> String {
        match self {
            Self::General => "general".to_owned(),
            Self::Skin => "skin".to_owned(),
            Self::Variables => "variables".to_owned(),
            Self::Menu(position) => format!("menu:{position}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Config {
        Config::load_from(std::env::temp_dir().join("y-pop-config-prefs-never-written.ini"))
    }

    #[test]
    fn nothing_saved_means_the_defaults() {
        let prefs = Prefs::from_config(&empty());
        assert_eq!(prefs, Prefs::default());
        assert!(prefs.show_comments);
    }

    #[test]
    fn what_is_written_is_read_back() {
        for screen in [
            LastScreen::General,
            LastScreen::Skin,
            LastScreen::Variables,
            LastScreen::Menu(0),
            LastScreen::Menu(7),
        ] {
            let prefs = Prefs {
                show_comments: false,
                last_screen: screen,
            };
            let mut config = empty();
            prefs.write_to(&mut config);
            assert_eq!(Prefs::from_config(&config), prefs);
        }
    }

    #[test]
    fn nonsense_falls_back_to_the_defaults() {
        let mut config = empty();
        config.set(SECTION, "screen", "menu:x");
        config.set(SECTION, "show-comments", "maybe");
        assert_eq!(Prefs::from_config(&config), Prefs::default());
    }

    #[test]
    fn the_file_sits_next_to_the_configuration() {
        let path = path_for(Path::new(r"C:\Users\me\AppData\Roaming\Ypo-Lite\y-pop.xml"));
        assert_eq!(path, Path::new(r"C:\Users\me\AppData\Roaming\Ypo-Lite\y-pop-config.ini"));
    }

    #[test]
    fn saving_creates_the_file_and_loading_reads_it() {
        let directory = std::env::temp_dir().join(format!("y-pop-config-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join(FILE_NAME);

        let prefs = Prefs {
            show_comments: false,
            last_screen: LastScreen::Menu(2),
        };
        prefs.save(&path);
        assert_eq!(Prefs::load(&path), prefs);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
