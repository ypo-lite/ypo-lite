//! The built-in icons: `icon="@name"` in `y-pop.xml`.
//!
//! Things y-pop can draw without any file of the user's: its own icon, the
//! ones it gives About / Quit / Settings and the folder, and a set of the
//! shell's stock icons (the same ones Windows itself uses, so they follow the
//! user's OS version and theme). `@app` is not one of these — it is a Store
//! app's own logo, see [`crate::model::APP_LOGO`].
//!
//! One catalogue serves both programs: y-pop resolves the name to an icon
//! when it builds a popup, and y-pop-config lists [`ALL`] in its icon picker.
//! A name that is not in the catalogue is not an internal icon, and is read
//! as a file path like any other value.

use y_core::icon::{Icon, IconCache, StockIcon};

/// Where a built-in icon's pixels come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    /// y-pop's own icon, as embedded in `y-pop.exe`.
    YPop,
    /// The shell's generic folder.
    Folder,
    Stock(StockIcon),
}

/// One entry of the catalogue.
#[derive(Clone, Copy, Debug)]
pub struct Internal {
    /// What follows the `@` in `icon="@name"`. Lower case.
    pub name: &'static str,
    /// What the icon picker calls it.
    pub title: &'static str,
    pub glyph: Glyph,
}

impl Internal {
    /// The value to write in `icon=`.
    pub fn value(&self) -> String {
        format!("@{}", self.name)
    }

    /// The icon itself. `None` if Windows has none to give.
    pub fn load(&self, cache: &mut IconCache) -> Option<Icon> {
        match self.glyph {
            Glyph::YPop => cache.for_file(&ypop_exe(), Some(0)),
            Glyph::Folder => cache.for_directory(),
            Glyph::Stock(stock) => cache.for_stock(stock),
        }
    }
}

/// The file y-pop's icon is read from: `y-pop.exe` next to the running
/// program (which is y-pop itself, or the settings app that ships beside it),
/// else the running program's own icon.
fn ypop_exe() -> String {
    let Ok(current) = std::env::current_exe() else {
        return String::new();
    };
    let sibling = current.with_file_name("y-pop.exe");
    let chosen = if sibling.is_file() { sibling } else { current };
    chosen.to_string_lossy().into_owned()
}

const fn entry(name: &'static str, title: &'static str, glyph: Glyph) -> Internal {
    Internal { name, title, glyph }
}

const fn stock(name: &'static str, title: &'static str, icon: StockIcon) -> Internal {
    entry(name, title, Glyph::Stock(icon))
}

/// Every built-in icon, in the order the picker shows them.
pub const ALL: &[Internal] = &[
    entry("ypop", "Y-Pop", Glyph::YPop),
    stock("about", "About", StockIcon::Info),
    stock("quit", "Quit", StockIcon::Delete),
    stock("settings", "Settings", StockIcon::Settings),
    entry("folder", "Folder", Glyph::Folder),
    stock("folder-open", "Open folder", StockIcon::FolderOpen),
    stock("program", "Program", StockIcon::Application),
    stock("computer", "Computer", StockIcon::Computer),
    stock("drive", "Drive", StockIcon::Drive),
    stock("network-drive", "Network drive", StockIcon::NetworkDrive),
    stock("network", "Network", StockIcon::Network),
    stock("server", "Server", StockIcon::Server),
    stock("world", "World", StockIcon::World),
    stock("users", "Users", StockIcon::Users),
    stock("find", "Search", StockIcon::Find),
    stock("help", "Help", StockIcon::Help),
    stock("key", "Key", StockIcon::Key),
    stock("lock", "Lock", StockIcon::Lock),
    stock("shield", "Shield", StockIcon::Shield),
    stock("warning", "Warning", StockIcon::Warning),
    stock("error", "Error", StockIcon::Error),
    stock("link", "Link", StockIcon::Link),
    stock("share", "Share", StockIcon::Share),
    stock("printer", "Printer", StockIcon::Printer),
    stock("recycle-bin", "Recycle bin", StockIcon::RecycleBin),
    stock("images", "Pictures", StockIcon::Images),
    stock("audio", "Music", StockIcon::Audio),
    stock("video", "Videos", StockIcon::Video),
    stock("zip", "Archive", StockIcon::Zip),
];

/// The built-in icon `text` (an `icon=` value) names, if any: `@name`, case
/// and surrounding spaces ignored.
pub fn find(text: &str) -> Option<&'static Internal> {
    let name = text.trim().strip_prefix('@')?;
    ALL.iter()
        .find(|internal| internal.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_lower_case_and_never_the_store_logo() {
        for (index, internal) in ALL.iter().enumerate() {
            assert_eq!(internal.name, internal.name.to_ascii_lowercase());
            assert!(
                !internal.name.eq_ignore_ascii_case("app"),
                "@app is a Store app's logo"
            );
            assert!(
                ALL[..index].iter().all(|other| other.name != internal.name),
                "{} twice",
                internal.name
            );
        }
    }

    #[test]
    fn a_value_finds_its_icon_whatever_its_case() {
        assert_eq!(find("@about").map(|i| i.name), Some("about"));
        assert_eq!(find("  @QUIT ").map(|i| i.name), Some("quit"));
        assert!(find("about").is_none(), "no @, so a file name");
        assert!(find("@app").is_none());
        assert!(find("@nonsense").is_none());
        for internal in ALL {
            assert_eq!(find(&internal.value()).map(|i| i.name), Some(internal.name));
        }
    }
}
