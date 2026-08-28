//! RAII wrapper around a Win32 popup menu.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemCount, HMENU, MENU_ITEM_FLAGS,
    MENUITEMINFOW, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    MIIM_BITMAP, PostMessageW, SetForegroundWindow, SetMenuItemInfoW, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
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

    /// Sets `id`'s icon — call after [`PopupMenu::add_item`] (or
    /// equivalent) has already given it that id. `bitmap` must outlive this
    /// menu: Windows does not free a custom `hbmpItem` when the item or
    /// menu holding it is destroyed, unlike the icons `hbmpChecked`/
    /// `hbmpUnchecked` conventionally point at, so the caller keeps owning
    /// it — typically for exactly as long as the popup stays open.
    pub fn set_item_icon(&self, id: u32, bitmap: HBITMAP) -> Result<()> {
        set_item_icon(self.handle, id, bitmap)
    }

    /// Sets the icon of the item at `position` (0-based, in the order it
    /// was added) rather than by command id — the only way to address a
    /// submenu (`MF_POPUP`) item: the value doubling as an id in
    /// [`PopupMenu::add_submenu`]'s `AppendMenuW` call is that submenu's own
    /// handle, not a command id, so there is no id to address it by. Call
    /// with `self.item_count() - 1` right after adding the item this icon
    /// belongs to.
    pub fn set_item_icon_by_position(&self, position: u32, bitmap: HBITMAP) -> Result<()> {
        set_item_icon_by_position(self.handle, position, bitmap)
    }

    /// How many items this menu holds — for computing the position an item
    /// was just added at (`item_count() - 1`), to feed
    /// [`PopupMenu::set_item_icon_by_position`].
    pub fn item_count(&self) -> u32 {
        item_count(self.handle)
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

    /// The raw handle — for matching against `WM_INITMENUPOPUP`'s `wParam`,
    /// which is how a caller populates a submenu lazily, the moment it is
    /// about to be shown rather than up front when the whole popup is built
    /// (see `y-pop`'s folder-browsing menu, the motivating case: listing a
    /// folder only once the user actually opens that particular submenu).
    pub fn handle(&self) -> HMENU {
        self.handle
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

/// Sets `id`'s icon directly on a raw handle — the free-function form of
/// [`PopupMenu::set_item_icon`], for the moment a submenu is populated
/// lazily (see [`PopupMenu::handle`]'s doc comment) and only the raw handle
/// `WM_INITMENUPOPUP` names is available, no `&PopupMenu` to call the
/// method through.
pub fn set_item_icon(menu: HMENU, id: u32, bitmap: HBITMAP) -> Result<()> {
    set_menu_item_bitmap(menu, id, false, bitmap)
}

/// The free-function form of [`PopupMenu::set_item_icon_by_position`], for
/// the same reason [`set_item_icon`] exists: a submenu populated lazily has
/// only a raw `HMENU` to work with, no `&PopupMenu`.
pub fn set_item_icon_by_position(menu: HMENU, position: u32, bitmap: HBITMAP) -> Result<()> {
    set_menu_item_bitmap(menu, position, true, bitmap)
}

fn set_menu_item_bitmap(menu: HMENU, item: u32, by_position: bool, bitmap: HBITMAP) -> Result<()> {
    // No `Default` impl on `MENUITEMINFOW` to build from — zeroing it
    // directly is equivalent and just as valid for a plain-old-data C
    // struct with no field that isn't a plain pointer or integer.
    let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MENUITEMINFOW>() as u32;
    info.fMask = MIIM_BITMAP;
    info.hbmpItem = bitmap;
    unsafe { SetMenuItemInfoW(menu, item, by_position, &info) }
}

/// The free-function form of [`PopupMenu::item_count`], for a submenu
/// populated lazily via its raw `HMENU`.
pub fn item_count(menu: HMENU) -> u32 {
    unsafe { GetMenuItemCount(Some(menu)) }.max(0) as u32
}
