//! Asking the shell to open something — the one `ShellExecute` call the
//! applications need for "open this file/folder/program", kept here so an
//! application crate that forbids `unsafe` (the config apps) can still do it.

use std::path::Path;

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

/// `ShellExecute`'s "open" on `path`: runs an executable, opens a folder in
/// Explorer, or opens a file with whatever is registered for it. Fire and
/// forget — if nothing can open it, the shell shows its own message.
pub fn open(path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    // SAFETY: every argument is a valid, NUL-terminated wide string or null,
    // and the call does not retain any of them.
    unsafe { ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL) };
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
