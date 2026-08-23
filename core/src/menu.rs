//! RAII wrapper around a Win32 popup menu.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MENU_ITEM_FLAGS, MF_CHECKED, MF_DISABLED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR, Result};

pub struct PopupMenu {
    handle: HMENU,
    /// Submenus attached with [`PopupMenu::add_submenu`]. Windows destroys
    /// them with the parent, so they must not be destroyed twice — keeping
    /// them here (rather than as live `PopupMenu`s) is what prevents that.
    owned: Vec<HMENU>,
}

impl PopupMenu {
    pub fn new() -> Result<Self> {
        let handle = unsafe { CreatePopupMenu()? };
        Ok(Self {
            handle,
            owned: Vec::new(),
        })
    }

    pub fn add_item(&self, id: u32, text: &str) -> Result<()> {
        self.append(MF_STRING, id as usize, text)
    }

    /// An item with a check mark, for settings toggled from the tray menu.
    pub fn add_check_item(&self, id: u32, text: &str, checked: bool) -> Result<()> {
        let flags = if checked {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        self.append(flags, id as usize, text)
    }

    /// A greyed-out item: shown, not selectable. Handy as a status line
    /// ("No device found") at the top of a tray menu.
    pub fn add_disabled_item(&self, text: &str) -> Result<()> {
        self.append(MF_STRING | MF_DISABLED | MF_GRAYED, 0, text)
    }

    pub fn add_separator(&self) -> Result<()> {
        self.append(MF_SEPARATOR, 0, "")
    }

    /// Attaches `submenu` under `text`. The submenu's handle is taken over by
    /// this menu and destroyed with it.
    pub fn add_submenu(&mut self, text: &str, submenu: PopupMenu) -> Result<()> {
        let handle = submenu.into_raw();
        self.owned.push(handle);
        self.append(MF_POPUP, handle.0 as usize, text)
    }

    fn append(&self, flags: MENU_ITEM_FLAGS, id: usize, text: &str) -> Result<()> {
        let text = HSTRING::from(text);
        unsafe { AppendMenuW(self.handle, flags, id, PCWSTR(text.as_ptr())) }
    }

    /// Gives up ownership of the handle: the caller becomes responsible for
    /// destroying it (or for attaching it to a menu that will).
    fn into_raw(mut self) -> HMENU {
        let handle = self.handle;
        self.handle = HMENU::default();
        handle
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
            let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));

            match selection.0 {
                0 => None,
                id => Some(id as u32),
            }
        }
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // Destroying a menu destroys its submenus with it, which is why
            // `owned` holds bare handles and not `PopupMenu` values.
            let _ = unsafe { DestroyMenu(self.handle) };
        }
    }
}
