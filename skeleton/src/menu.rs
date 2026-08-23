//! RAII wrapper around a Win32 popup menu.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MF_SEPARATOR, MF_STRING, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
};
use windows::core::{HSTRING, PCWSTR, Result};

pub struct PopupMenu {
    handle: HMENU,
}

impl PopupMenu {
    pub fn new() -> Result<Self> {
        let handle = unsafe { CreatePopupMenu()? };
        Ok(Self { handle })
    }

    pub fn add_item(&self, id: u32, text: &str) -> Result<()> {
        let text = HSTRING::from(text);
        unsafe { AppendMenuW(self.handle, MF_STRING, id as usize, PCWSTR(text.as_ptr())) }
    }

    #[allow(dead_code)] // part of the skeleton's public surface
    pub fn add_separator(&self) -> Result<()> {
        unsafe { AppendMenuW(self.handle, MF_SEPARATOR, 0, PCWSTR::null()) }
    }

    /// Shows the menu at the given screen coordinates and returns the command
    /// id that was chosen, or `None` if the user dismissed the menu.
    ///
    /// The `SetForegroundWindow` / `WM_NULL` dance is required for tray menus:
    /// without it the menu would not close when the user clicks elsewhere
    /// (see Microsoft KB135788).
    pub fn track(&self, owner: HWND, position: POINT) -> Option<u32> {
        unsafe {
            let _ = SetForegroundWindow(owner);

            let selection = TrackPopupMenu(
                self.handle,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_BOTTOMALIGN,
                position.x,
                position.y,
                None,
                owner,
                None,
            );

            // Let the menu dismiss properly once we return.
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(owner),
                windows::Win32::UI::WindowsAndMessaging::WM_NULL,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );

            match selection.0 {
                0 => None,
                id => Some(id as u32),
            }
        }
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        let _ = unsafe { DestroyMenu(self.handle) };
    }
}
