//! RAII wrapper around a shell notification (tray) icon.
//!
//! Dropping a [`TrayIcon`] removes it from the notification area, so the icon
//! can never survive its owner — the classic "ghost icon left behind" bug.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::Result;

pub struct TrayIcon {
    data: NOTIFYICONDATAW,
}

impl TrayIcon {
    /// Registers an icon in the notification area.
    ///
    /// * `owner` receives `callback_message` for every mouse event on the icon.
    /// * `id` must be unique among the icons owned by `owner`.
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

    /// Re-adds the icon after Explorer restarted (see the `TaskbarCreated`
    /// broadcast handled in `app.rs`).
    pub fn restore(&mut self) -> Result<()> {
        // The previous instance is already gone with the old taskbar; a stale
        // NIM_DELETE would fail harmlessly, so go straight to NIM_ADD.
        self.register()
    }

    /// Updates the tooltip shown when hovering the icon.
    #[allow(dead_code)] // part of the skeleton's public surface
    pub fn set_tooltip(&mut self, tooltip: &str) -> Result<()> {
        copy_to_buffer(tooltip, &mut self.data.szTip);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok()
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // Nothing useful to do on failure while unwinding.
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data) };
    }
}

/// Copies `text` as UTF-16 into a fixed-size Win32 buffer, truncating if
/// needed and always leaving a terminating NUL.
fn copy_to_buffer(text: &str, buffer: &mut [u16]) {
    let max = buffer.len() - 1;
    let mut written = 0;
    for unit in text.encode_utf16().take(max) {
        buffer[written] = unit;
        written += 1;
    }
    buffer[written..].fill(0);
}
