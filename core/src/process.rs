//! Facts about the process behind a window: who owns it, what it is called,
//! and whether we are even allowed to drive it.
//!
//! The last one matters because of UIPI (User Interface Privilege
//! Isolation): a process cannot send window messages to, or show and hide the
//! windows of, a process of higher integrity — an elevated program is out of
//! reach of a normal one. The calls fail *silently*, so an application that
//! wants to say "I can't do that" has to ask first.

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::PWSTR;

/// A process handle closed on drop.
struct Process(HANDLE);

impl Process {
    fn open(pid: u32) -> Option<Self> {
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .ok()
            .map(Self)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The id of the process that owns `window`, `0` if the window is gone.
pub fn window_process_id(window: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    pid
}

/// The executable name of process `pid`, without directory or `.exe` —
/// `"notepad"`. `None` if the process is gone or closed to us.
pub fn executable_stem(pid: u32) -> Option<String> {
    let path = executable_path(pid)?;
    Path::new(&path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

/// The full path of the executable of process `pid`. `None` if the process is
/// gone or closed to us.
pub fn executable_path(pid: u32) -> Option<String> {
    let process = Process::open(pid)?;

    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .ok()?;

    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

/// Whether process `pid` runs elevated. `None` when that cannot be read —
/// which, from a non-elevated caller, is itself the usual sign of an elevated
/// (or protected) target: its token is closed to us.
pub fn is_elevated(pid: u32) -> Option<bool> {
    let process = Process::open(pid)?;
    token_is_elevated(process.0)
}

/// Whether this very process runs elevated.
pub fn current_is_elevated() -> bool {
    token_is_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false)
}

/// Whether windows of process `pid` can be driven from here: shown, hidden,
/// sent messages. False for an elevated process when we are not, and for any
/// process whose elevation cannot be told (see [`is_elevated`]) — erring
/// toward refusing, because the failure otherwise is silent.
pub fn can_drive(pid: u32) -> bool {
    if pid == unsafe { GetCurrentProcessId() } || current_is_elevated() {
        return true;
    }
    is_elevated(pid) == Some(false)
}

fn token_is_elevated(process: HANDLE) -> Option<bool> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
    let token = Process(token);

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    }
    .ok()?;
    Some(elevation.TokenIsElevated != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_its_own_process() {
        let pid = unsafe { GetCurrentProcessId() };
        assert!(is_elevated(pid).is_some());
        assert!(can_drive(pid));

        let name = executable_stem(pid).expect("own executable name");
        let expected = std::env::current_exe().unwrap();
        assert_eq!(
            Some(name.as_str()),
            expected.file_stem().and_then(|stem| stem.to_str())
        );
    }

    #[test]
    fn a_vanished_process_is_not_drivable() {
        // Process ids are multiples of four; this one is not, so no process
        // can ever have it.
        assert!(executable_stem(0xFFFF_FFFD).is_none());
        assert!(!can_drive(0xFFFF_FFFD));
    }
}
