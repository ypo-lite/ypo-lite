//! Application-wide identifiers: window messages, control ids, menu commands.
//!
//! Keeping them in one place avoids accidental collisions once the skeleton
//! grows more windows, more tray icons or more menu entries.

use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// Private message the shell posts to our window for tray icon events.
/// Anything in the `WM_APP..=0xBFFF` range is reserved for the application.
pub const WM_TRAY_CALLBACK: u32 = WM_APP + 1;

/// Identifier of our (single) tray icon, unique per owner window.
pub const TRAY_ICON_ID: u32 = 1;

/// Menu command identifiers. Start above 0 because `TrackPopupMenu` with
/// `TPM_RETURNCMD` uses 0 to report "nothing was selected".
pub const IDM_EXIT: u32 = 100;

/// Hotkey identifiers, arriving as the `wParam` of `WM_HOTKEY`. They live in
/// their own numbering space, unrelated to the menu commands above, and must
/// stay within `0..=0xBFFF`.
pub const IDH_SHOW_MENU: u32 = 1;
