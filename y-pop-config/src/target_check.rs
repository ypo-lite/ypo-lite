//! Whether what an app points at exists, for the readout under the Target
//! field. A look at the disk, nothing more: it can't know that a program
//! launches, only that its file is there.
//!
//! Done on the UI thread, so it refuses the one thing that can stall: a
//! network path (`\\server\share`) is reported as not checked rather than
//! waited for. (The plan's off-thread check is the better answer if a local
//! disk ever turns out to be slow too.)

use std::path::Path;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Found {
    Yes,
    No,
    /// Not looked at, and why ("empty", "network path"…).
    Unchecked(&'static str),
}

/// `expanded` is the target after `%variables%`; `folder` is for the Folder
/// mode, where only a directory counts.
pub fn check(expanded: &str, folder: bool) -> Found {
    let text = expanded.trim().trim_matches('"');
    if text.is_empty() {
        return Found::Unchecked("nothing entered yet");
    }
    if text.contains('%') {
        return Found::Unchecked("has a variable that is not defined");
    }
    if text.starts_with(r"\\") || text.starts_with("//") {
        return Found::Unchecked("network path, not checked");
    }
    if text.contains("://") {
        return Found::Unchecked("an address, not checked");
    }

    let path = Path::new(text);
    let exists = |path: &Path| if folder { path.is_dir() } else { path.exists() };
    // A bare name (`notepad.exe`) is looked up on PATH, as CreateProcess does.
    if !folder && !text.contains(['\\', '/']) {
        return if on_path(text) { Found::Yes } else { Found::No };
    }
    if exists(path) { Found::Yes } else { Found::No }
}

fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let with_exe = format!("{name}.exe");
    std::env::split_paths(&path)
        .any(|dir| dir.join(name).is_file() || dir.join(&with_exe).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_exists_is_found_and_one_that_does_not_is_not() {
        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_string_lossy();
        assert_eq!(check(&exe, false), Found::Yes);
        assert_eq!(check(&format!("\"{exe}\""), false), Found::Yes);
        assert_eq!(check(&format!("{exe}.nope"), false), Found::No);
    }

    #[test]
    fn folder_mode_only_accepts_a_directory() {
        let dir = std::env::temp_dir();
        let exe = std::env::current_exe().unwrap();
        assert_eq!(check(&dir.to_string_lossy(), true), Found::Yes);
        assert_eq!(check(&exe.to_string_lossy(), true), Found::No);
    }

    #[test]
    fn a_bare_name_is_searched_on_path() {
        assert_eq!(check("cmd", false), Found::Yes);
        assert_eq!(check("definitely-not-a-program-xyz.exe", false), Found::No);
    }

    #[test]
    fn what_cannot_be_checked_says_so() {
        assert!(matches!(check("", false), Found::Unchecked(_)));
        assert!(matches!(
            check(r"\\server\share\a.exe", false),
            Found::Unchecked(_)
        ));
        assert!(matches!(check(r"%nope%\a.exe", false), Found::Unchecked(_)));
        assert!(matches!(
            check("https://example.com", false),
            Found::Unchecked(_)
        ));
    }
}
