//! Identifiers reserved by the core, and the ranges left to applications.
//!
//! Windows carves the message space into ranges: `WM_APP..=0xBFFF` belongs to
//! the application, `0xC000..=0xFFFF` to messages registered by name with
//! `RegisterWindowMessageW`. The core takes the bottom of the first range and
//! leaves the rest.

use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// Private message the shell posts to our window for tray icon events.
pub const WM_TRAY_CALLBACK: u32 = WM_APP + 1;

/// Posted to the running instance when a second one starts and hands over
/// (see [`crate::single_instance`]).
pub const WM_ACTIVATE_INSTANCE: u32 = WM_APP + 2;

/// First message id an application may define. Everything below belongs to
/// the core.
pub const WM_APP_FIRST: u32 = WM_APP + 0x100;

/// Identifier of the application's main tray icon, unique per owner window.
pub const TRAY_ICON_ID: u32 = 1;

/// Menu commands start above 0: `TrackPopupMenu` with `TPM_RETURNCMD` uses 0
/// to report "nothing was selected". Applications should number their own
/// commands from here.
pub const IDM_FIRST: u32 = 100;

/// Hotkey ids live in their own numbering space (`0..=0xBFFF`), unrelated to
/// the menu commands.
pub const IDH_FIRST: u32 = 1;
