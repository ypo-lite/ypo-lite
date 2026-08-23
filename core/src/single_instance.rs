//! One process per user, and a way for the second one to say hello.
//!
//! A named mutex is the usual Win32 answer: creating it always succeeds, but
//! `ERROR_ALREADY_EXISTS` tells the newcomer that somebody got there first.
//! The guard must live as long as the process — the kernel destroys the mutex
//! when the last handle closes, so dropping it early lets a second instance
//! through.

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, PostMessageW};
use windows::core::{Error, HSTRING, PCWSTR, Result};

/// Holds the named mutex for as long as this process is the running instance.
pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    /// Claims `name` for this process.
    ///
    /// Returns `Ok(None)` when another instance already holds it. The name is
    /// prefixed with `Local\`, which scopes it to the logon session: two users
    /// on the same machine each get their own instance, which is what a
    /// per-user tray application wants. (`Global\` would be one per machine
    /// and needs a matching ACL to work across sessions.)
    pub fn acquire(name: &str) -> Result<Option<Self>> {
        let name = HSTRING::from(format!("Local\\{name}"));
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr()))? };

        // CreateMutexW succeeds either way; only the last error distinguishes
        // "created" from "opened an existing one".
        if Error::from_thread().code() == ERROR_ALREADY_EXISTS.to_hresult() {
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }

        Ok(Some(Self { handle }))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

/// Tells the instance already running that the user started the application
/// again — the moment to pop the UI up, not to exit silently.
///
/// Broadcast rather than addressed: finding the other window would mean
/// enumerating top-level windows, and the running instance is the only one
/// that has registered `message`. Registered messages (`0xC000..=0xFFFF`, see
/// `RegisterWindowMessageW`) are the only ones safe to broadcast; every other
/// window sees an id it does not know and ignores it.
pub fn notify_running_instance(message: u32) {
    let _ = unsafe { PostMessageW(Some(HWND_BROADCAST), message, WPARAM(0), LPARAM(0)) };
}

/// Registers (and on later calls, looks up) a broadcast message id by name.
///
/// Both instances call this with the same name and get the same id back.
pub fn broadcast_message(name: &str) -> u32 {
    let name = HSTRING::from(name);
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(PCWSTR(name.as_ptr()))
    }
}
