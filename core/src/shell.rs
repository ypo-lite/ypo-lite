//! Asking the shell to open something — the one `ShellExecute` call the
//! applications need for "open this file/folder/program", kept here so an
//! application crate that forbids `unsafe` (the config apps) can still do it.

use std::path::{Path, PathBuf};

use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// `ShellExecute`'s "open" on `path`: runs an executable, opens a folder in
/// Explorer, or opens a file with whatever is registered for it. Fire and
/// forget — if nothing can open it, the shell shows its own message.
pub fn open(path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    // SAFETY: every argument is a valid, NUL-terminated wide string or null,
    // and the call does not retain any of them.
    unsafe { ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL) };
}

/// Opens Explorer on the folder holding `path`, with `path` selected — "show
/// me where this is". Fire and forget, like [`open`].
pub fn reveal(path: &Path) {
    let argument = HSTRING::from(format!("/select,\"{}\"", path.display()));
    // SAFETY: as in `open`.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("explorer.exe"),
            &argument,
            None,
            SW_SHOWNORMAL,
        )
    };
}

/// Where the program called `name` (`notepad.exe`, `services.msc`) is, found
/// the way a launch would: the application's folder, the system folders, then
/// `PATH`. `None` if it is not found, or if `name` is too long for a path.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let name = HSTRING::from(name);
    let mut buffer = [0u16; 520];
    // SAFETY: `name` is a valid wide string; the buffer is ours and its length
    // is passed with it.
    let length = unsafe {
        SearchPathW(
            PCWSTR::null(),
            &name,
            PCWSTR::null(),
            Some(&mut buffer),
            None,
        )
    } as usize;
    (length > 0 && length < buffer.len())
        .then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

/// Keeps what the user copies from outliving a WinUI window the wrong way.
///
/// A WinUI `TextBox` copies lazily: the clipboard holds a data object that
/// calls back into the application when something pastes. If the process
/// ends while it is still there, the callback has nowhere to land — the
/// process stalls for some seconds after its window closes and then ends
/// with an access violation (`0xc0000005`). Flushing at exit does not help:
/// by then the runtime that would answer the callback is already gone, and
/// the flush is what stalls.
///
/// So the data is rendered *when it is copied*, while the window is alive:
/// this subscribes to the clipboard's change event and calls
/// `Clipboard.Flush` from it. Nothing lazy is left at exit. Dropping the
/// value unsubscribes.
///
/// Create it once the UI runtime is up — in the main component's `create`.
pub struct ClipboardFlusher {
    token: i64,
}

impl ClipboardFlusher {
    /// `None` if the clipboard events can't be subscribed to (no runtime
    /// yet, say): the application still works, and only keeps the old
    /// behaviour at exit.
    pub fn start() -> Option<Self> {
        use windows::ApplicationModel::DataTransfer::Clipboard;
        use windows::Foundation::EventHandler;

        let handler = EventHandler::<windows::core::IInspectable>::new(|_, _| {
            // Fails, harmlessly, when the change was another program's.
            let _ = Clipboard::Flush();
            Ok(())
        });
        Clipboard::ContentChanged(&handler)
            .ok()
            .map(|token| Self { token })
    }
}

impl Drop for ClipboardFlusher {
    fn drop(&mut self) {
        use windows::ApplicationModel::DataTransfer::Clipboard;
        let _ = Clipboard::RemoveContentChanged(self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::find_program;

    #[test]
    fn a_system_program_is_found_by_its_bare_name() {
        let found = find_program("notepad.exe").expect("notepad is on every Windows");
        assert!(found.is_absolute(), "{found:?}");
        assert!(found.ends_with("notepad.exe"), "{found:?}");
        assert!(found.is_file());
    }

    #[test]
    fn a_program_that_does_not_exist_is_not_found() {
        assert_eq!(find_program("no-such-program-ypo-lite.exe"), None);
    }
}
