//! The programs that have a window open right now, the way the taskbar would
//! list them.
//!
//! A window counts when it is visible, has a title, is not cloaked (another
//! virtual desktop, a suspended Store app) and is an application window: no
//! owner and not a tool window, unless it asks to be on the taskbar
//! (`WS_EX_APPWINDOW`). The shell's own windows are dropped by class.
//!
//! Windows of packaged apps are hosted by `ApplicationFrameHost`, whose
//! process is not the app's: they are left out rather than listed under the
//! wrong executable. (A Store app is picked from `apps_folder`.)
//!
//! This is a different test from `y-snap`'s "window to act on under the
//! pointer": that one answers for one window and refuses by class; this one
//! lists the windows a user would call "open programs".

use std::collections::HashSet;
use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetWindow, GetWindowLongW,
    GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::process;

/// One running program.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunningProgram {
    /// The title of its first window in z-order (the topmost), for want of
    /// a better name; callers usually prefer the executable's own description.
    pub title: String,
    /// The full path of its executable.
    pub exe: String,
}

/// Classes that are windows of the shell, not programs.
const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "ApplicationFrameWindow",
    "Windows.UI.Core.CoreWindow",
];

/// Every program with an application window, once each (by executable, case
/// aside), topmost first.
pub fn running_programs() -> Vec<RunningProgram> {
    let mut found: Vec<RunningProgram> = Vec::new();
    // SAFETY: `collect` only runs during this call, and `found` outlives it.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&raw mut found as isize));
    }
    let mut seen = HashSet::new();
    found.retain(|program| seen.insert(program.exe.to_lowercase()));
    found
}

unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `&mut Vec` `running_programs` passed in.
    let found = unsafe { &mut *(data.0 as *mut Vec<RunningProgram>) };
    if let Some(program) = program_of(window) {
        found.push(program);
    }
    BOOL(1)
}

fn program_of(window: HWND) -> Option<RunningProgram> {
    if !unsafe { IsWindowVisible(window) }.as_bool() || is_cloaked(window) {
        return None;
    }
    let ex_style = unsafe { GetWindowLongW(window, GWL_EXSTYLE) } as u32;
    let on_taskbar = ex_style & WS_EX_APPWINDOW.0 != 0;
    let owned = unsafe { GetWindow(window, GW_OWNER) }.is_ok_and(|owner| !owner.is_invalid());
    if !on_taskbar && (owned || ex_style & WS_EX_TOOLWINDOW.0 != 0) {
        return None;
    }

    let length = unsafe { GetWindowTextLengthW(window) };
    if length <= 0 {
        return None;
    }
    let mut title = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(window, &mut title) };
    if copied <= 0 {
        return None;
    }
    let title = String::from_utf16_lossy(&title[..copied as usize]);

    let mut class = [0u16; 256];
    let class_length = unsafe { GetClassNameW(window, &mut class) };
    if class_length <= 0 {
        return None;
    }
    let class = String::from_utf16_lossy(&class[..class_length as usize]);
    if SHELL_CLASSES.contains(&class.as_str()) {
        return None;
    }

    let exe = process::executable_path(process::window_process_id(window))?;
    Some(RunningProgram { title, exe })
}

fn is_cloaked(window: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast::<c_void>(),
            size_of::<u32>() as u32,
        )
    }
    .is_ok()
        && cloaked != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real desktop's window list: asserts the shape of the answer,
    /// not its content (a build machine may have no window at all).
    #[test]
    fn the_programs_are_titled_and_listed_once() {
        let programs = running_programs();
        let mut exes = HashSet::new();
        for program in &programs {
            assert!(!program.title.is_empty() && !program.exe.is_empty());
            assert!(exes.insert(program.exe.to_lowercase()), "{}", program.exe);
        }
    }
}
