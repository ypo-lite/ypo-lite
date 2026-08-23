//! Minimal Windows tray application, meant to be copied as a starting point.
//!
//! Module layout:
//!   `app`    - application state and message handling  (edit this one)
//!   `window` - window class, window procedure, message loop  (reusable)
//!   `tray`   - notification area icon, RAII              (reusable)
//!   `menu`   - popup menu, RAII                          (reusable)
//!   `hotkey` - system-wide hotkeys, RAII                 (reusable)
//!   `resources` - access to the icon / manifest embedded by `build.rs`
//!   `ids`    - message / command identifiers

// Build a GUI application: no console window is allocated.
// Comment this out temporarily if you want `println!` debugging.
#![windows_subsystem = "windows"]

mod app;
mod hotkey;
mod ids;
mod memory;
mod menu;
mod resources;
mod tray;
mod window;

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::{HSTRING, PCWSTR, w};

fn main() {
    if let Err(error) = app::run() {
        // Without a console, a message box is the only way to report a
        // start-up failure.
        let message = HSTRING::from(format!("{error}"));
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                w!("Win Rust Min"),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
