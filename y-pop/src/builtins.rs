//! The lazily-resolved half of `y-pop`'s builtin `%token%` table.
//!
//! `%win%`, `%mydocs%`, … each cost a Win32 call to resolve; a config file
//! that only ever uses a handful of them shouldn't pay for the rest. This
//! wraps `y_core::known_folders::KnownFolders` — which does the actual
//! lazy-resolve-and-cache work, and is generic over any Windows app, not
//! just this one's `%token%` names — adding the two builtins that are not
//! Win32 known folders at all (`%pop%`, `%config%`: values this struct is
//! simply constructed with) and the mapping from a `y-pop.xml` token name to
//! which of the two paths (a known folder, or one of those) it means.
//!
//! In the `y_pop` library, but kept apart from `y_pop::config` on purpose:
//! that module is pure Rust so its tests never touch Win32 (see its doc
//! comment), and plugging this in happens at the edge — the tray app, and
//! the settings app, which shows what each token resolves to on this PC.

use std::path::Path;

use y_core::known_folders::{KnownFolder, KnownFolders};
use crate::config::{self, LoadError};
use crate::model::ConfigTree;
use crate::variables::Lookup;

/// Loads `y-pop.xml` from wherever it lives, resolving its builtins from
/// Win32 lazily as the file is parsed. The one entry point `main` needs:
/// "where is the file" and "what are its builtins" are both this function's
/// business, not its caller's — a caller that had to gather `exe_dir` and
/// the config path itself first would be doing this function's job for it.
pub fn load() -> Result<ConfigTree, LoadError> {
    let path = config::path();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default();

    let mut builtins = Builtins::new(&exe_dir, &path);
    config::load_from(path, &mut builtins)
}

/// Every token name [`Lookup::get`] answers for, in the order a settings
/// screen lists them.
pub const NAMES: &[&str] = &[
    "pop",
    "config",
    "win",
    "system",
    "system86",
    "programfiles",
    "programfiles86",
    "mydocs",
    "mymusic",
    "myvideo",
    "desktop",
    "appdata",
    "appdata-r",
    "user",
];

pub struct Builtins {
    folders: KnownFolders,
    exe_dir: String,
    config_path: String,
}

impl Builtins {
    /// `exe_dir`/`config_path` back `%pop%`/`%config%` — resolving both is
    /// free (the caller already has them), so unlike the Win32 folders below
    /// there is nothing to gain by deferring it.
    pub fn new(exe_dir: &Path, config_path: &Path) -> Self {
        Self {
            folders: KnownFolders::new(),
            exe_dir: exe_dir.display().to_string(),
            config_path: config_path.display().to_string(),
        }
    }
}

impl Lookup for Builtins {
    fn get(&mut self, name: &str) -> Option<String> {
        let folder = match name {
            "pop" => return Some(self.exe_dir.clone()),
            "config" => return Some(self.config_path.clone()),
            "win" => KnownFolder::Windows,
            "system" => KnownFolder::System,
            "system86" => KnownFolder::SystemX86,
            "programfiles" => KnownFolder::ProgramFiles,
            "programfiles86" => KnownFolder::ProgramFilesX86,
            "mydocs" => KnownFolder::Documents,
            "mymusic" => KnownFolder::Music,
            "myvideo" => KnownFolder::Videos,
            "desktop" => KnownFolder::Desktop,
            "appdata" => KnownFolder::LocalAppData,
            "appdata-r" => KnownFolder::RoamingAppData,
            "user" => KnownFolder::Profile,
            _ => return None,
        };
        self.folders.get(folder).map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_name_resolves() {
        let mut builtins = Builtins::new(Path::new("exe"), Path::new("config"));
        for name in NAMES {
            assert!(builtins.get(name).is_some(), "%{name}% has no value");
        }
    }
}
