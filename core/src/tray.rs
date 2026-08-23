//! RAII wrapper around a shell notification (tray) icon.
//!
//! Dropping a [`TrayIcon`] removes it from the notification area, so the icon
//! can never survive its owner — the classic "ghost icon left behind" bug.
//!
//! The owner window must forward the `TaskbarCreated` broadcast to
//! [`TrayIcon::restore`]; [`crate::app::TrayApp`] does it for you.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_NONE,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFY_ICON_INFOTIP_FLAGS,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_RBUTTONUP,
};
use windows::core::Result;

use crate::resources::copy_to_buffer;

pub struct TrayIcon {
    data: NOTIFYICONDATAW,
}

impl TrayIcon {
    /// Registers an icon in the notification area.
    ///
    /// * `owner` receives `callback_message` for every mouse event on the icon
    ///   — decode it with [`TrayEvent::decode`].
    /// * `id` must be unique among the icons owned by `owner`
    ///   ([`crate::ids::TRAY_ICON_ID`] for the usual single one).
    /// * `icon` should be a small (`SM_CXSMICON`) icon; see
    ///   [`crate::resources::load_app_icon`]. The shell does not take
    ///   ownership of the handle, so a shared resource icon is the right fit.
    pub fn new(
        owner: HWND,
        id: u32,
        callback_message: u32,
        icon: HICON,
        tooltip: &str,
    ) -> Result<Self> {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: owner,
            uID: id,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: callback_message,
            hIcon: icon,
            ..Default::default()
        };
        copy_to_buffer(tooltip, &mut data.szTip);

        let mut icon = Self { data };
        icon.register()?;
        Ok(icon)
    }

    /// Adds the icon to the shell and opts into the version 4 callback
    /// protocol (screen coordinates in `wParam`, event id in `lParam`).
    fn register(&mut self) -> Result<()> {
        unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) }.ok()?;

        self.data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        unsafe { Shell_NotifyIconW(NIM_SETVERSION, &self.data) }.ok()?;
        Ok(())
    }

    /// Re-adds the icon after Explorer restarted (the `TaskbarCreated`
    /// broadcast).
    pub fn restore(&mut self) -> Result<()> {
        // The previous instance is already gone with the old taskbar; a stale
        // NIM_DELETE would fail harmlessly, so go straight to NIM_ADD.
        self.register()
    }

    /// Updates the tooltip shown when hovering the icon.
    pub fn set_tooltip(&mut self, tooltip: &str) -> Result<()> {
        copy_to_buffer(tooltip, &mut self.data.szTip);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok()
    }

    /// Swaps the icon, for applications that reflect state in the tray
    /// (muted / unmuted, active / idle).
    pub fn set_icon(&mut self, icon: HICON) -> Result<()> {
        self.data.hIcon = icon;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok()
    }

    /// Shows a balloon / toast notification from the icon.
    ///
    /// The shell may drop it silently — quiet hours, focus assist, or the user
    /// having turned notifications off for the app — so never make anything
    /// depend on it having been seen.
    pub fn notify(&mut self, title: &str, message: &str, level: Notify) -> Result<()> {
        let flags = self.data.uFlags;
        self.data.uFlags |= NIF_INFO;
        self.data.dwInfoFlags = level.flags();
        copy_to_buffer(title, &mut self.data.szInfoTitle);
        copy_to_buffer(message, &mut self.data.szInfo);

        let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok();

        // NIF_INFO is a one-shot: leaving it set would repeat the balloon on
        // the next unrelated NIM_MODIFY.
        self.data.uFlags = flags;
        result
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // Nothing useful to do on failure while unwinding.
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data) };
    }
}

/// Icon shown next to a balloon notification.
#[derive(Clone, Copy, Default)]
pub enum Notify {
    #[default]
    None,
    Info,
    Warning,
    Error,
}

impl Notify {
    fn flags(self) -> NOTIFY_ICON_INFOTIP_FLAGS {
        match self {
            Notify::None => NIIF_NONE,
            Notify::Info => NIIF_INFO,
            Notify::Warning => NIIF_WARNING,
            Notify::Error => NIIF_ERROR,
        }
    }
}

/// A decoded tray callback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayEvent {
    /// Right click, or the keyboard menu key: show the context menu.
    ContextMenu,
    /// Double click, conventionally the icon's default action.
    DoubleClick,
    /// Anything else (hover, single clicks, balloon events).
    Other(u32),
}

impl TrayEvent {
    /// Unpacks a version 4 tray callback: the cursor position is packed into
    /// `wParam`, the event id into the low word of `lParam`.
    pub fn decode(wparam: WPARAM, lparam: LPARAM) -> (Self, POINT) {
        let position = POINT {
            x: (wparam.0 & 0xFFFF) as i16 as i32,
            y: ((wparam.0 >> 16) & 0xFFFF) as i16 as i32,
        };

        let event = match (lparam.0 as u32) & 0xFFFF {
            // WM_CONTEXTMENU covers right click and the keyboard menu key;
            // WM_RBUTTONUP is kept for older shells.
            WM_CONTEXTMENU | WM_RBUTTONUP => TrayEvent::ContextMenu,
            WM_LBUTTONDBLCLK => TrayEvent::DoubleClick,
            other => TrayEvent::Other(other),
        };

        (event, position)
    }
}
