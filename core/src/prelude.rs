//! What an application's `main.rs` and message handler almost always need.
//!
//! ```
//! use y_core::prelude::*;
//! ```

pub use crate::app::{AppInfo, Outcome};
pub use crate::hotkey::{Fallback, Hotkey, HotkeyManager, Registration};
pub use crate::menu::PopupMenu;
pub use crate::tray::TrayIcon;
pub use crate::window::{WindowHandler, WindowIcons};

pub use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
pub use windows::core::Result;
