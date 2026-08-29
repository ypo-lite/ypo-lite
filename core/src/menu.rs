//! RAII wrapper around a Win32 popup menu, plain or owner-drawn (themed).
//!
//! # Owner-draw theming
//!
//! [`PopupMenu::new`] behaves exactly as before: plain `MF_STRING` items,
//! native colours, `hbmpItem` for icons — the shape every tray/context menu
//! in the workspace still wants. [`PopupMenu::themed`] instead builds every
//! item `MFT_OWNERDRAW`, which hands *all* painting to this process via
//! `WM_MEASUREITEM`/`WM_DRAWITEM` — [`handle_measure_item`]/
//! [`handle_draw_item`], which the owner window's message loop must forward
//! both messages to (see their own doc comments for the exact call shape).
//! `y-pop`'s themed native backend (`docs/y-pop-plan.md`'s roadmap step 5)
//! is the motivating, and so far only, caller.
//!
//! Once `MFT_OWNERDRAW` is set on an item, two things Windows otherwise does
//! for free stop happening, which is why this module looks the way it does:
//!
//! - **`dwTypeData` (the item's text) is no longer text at all** — per
//!   `AppendMenuW`'s own documented behaviour, `lpNewItem`/`dwItemData`
//!   becomes an application-defined value instead, read back in
//!   `MEASUREITEMSTRUCT`/`DRAWITEMSTRUCT::itemData`. So a themed item's text
//!   (and everything else it needs to paint itself: separator/submenu kind,
//!   its slice of the palette) is boxed into an [`ItemData`] up front and
//!   that value used *as* `dwItemData` — one `AppendMenuW` call still
//!   creates the item, same as the unthemed path, just with different
//!   flags and a pointer instead of a string in the same argument slot.
//! - **`hbmpItem` (`MIIM_BITMAP`) is no longer auto-painted** — an
//!   owner-drawn item's entire `rcItem`, icon included, is this process's
//!   job to fill in `WM_DRAWITEM`. Rather than plumb a second, themed-only
//!   icon-attachment API through every caller, [`set_item_icon`]/
//!   [`set_item_icon_by_position`] (and the `PopupMenu` methods wrapping
//!   them) stay exactly as they were and detect the difference themselves:
//!   check the item's own `fType` for `MFT_OWNERDRAW` and, if set, write the
//!   bitmap into that item's already-boxed [`ItemData`] instead of
//!   `hbmpItem`. Every existing call site (`y-pop/src/menu.rs`'s
//!   `resolve_icon`/`attach_icon`) needed no change at all for this.
//!
//! [`ItemData`] is never freed by hand — [`PopupMenu::drop`] walks the whole
//! menu tree (recursing through every submenu `DestroyMenu` would also
//! reach) reclaiming each item's `dwItemData` right before destroying the
//! `HMENU`s themselves. Walking the live tree at drop time, rather than
//! tracking `Box<ItemData>` ownership through `add_submenu`'s handle move,
//! is what keeps this correct regardless of how a submenu was populated —
//! including a `browse` folder's items, appended straight to a raw `HMENU`
//! from `WM_INITMENUPOPUP` (see [`append_item`]/[`append_submenu`]) with no
//! `PopupMenu` wrapper in the loop to have tracked them otherwise.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BF_BOTTOM, BITMAP, BLENDFUNCTION, BitBlt,
    CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DFC_MENU, DFCS_MENUARROW,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawEdge,
    DrawFrameControl, DrawTextW, EDGE_ETCHED, ExcludeClipRect, FillRect, FrameRect, GetDC,
    GetObjectW, GetTextExtentPoint32W, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCAND, SRCINVERT,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_SELECTED, ODT_MENU};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU,
    MENU_ITEM_FLAGS, MENUITEMINFOW, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_OWNERDRAW, MF_POPUP,
    MF_SEPARATOR, MF_STRING, MFT_OWNERDRAW, MIIM_BITMAP, MIIM_DATA, MIIM_FTYPE, MIIM_SUBMENU,
    PostMessageW, SetForegroundWindow, SetMenuItemInfoW, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR, Result};

use crate::overlay::Color;

pub struct PopupMenu {
    handle: HMENU,
    /// Submenus attached with [`PopupMenu::add_submenu`]. Windows destroys
    /// them with the parent, so they must not be destroyed twice — keeping
    /// them here (rather than as live `PopupMenu`s) is what prevents that.
    owned: Vec<HMENU>,
    /// `Some` for a menu built with [`PopupMenu::themed`] — every item this
    /// menu (and any submenu it attaches) appends from here on is
    /// owner-drawn with this palette. `None` is the plain, native-rendered
    /// path every other caller in the workspace still uses.
    colors: Option<MenuColors>,
}

/// The palette an owner-drawn [`PopupMenu`] paints itself with — PortAL's
/// `PortalMenuDesign` (`menu.cpp`) trimmed to what `y-pop` actually themes:
/// a base and a hovered/selected state, a dimmed disabled text, and a thin
/// border drawn around the currently hovered row (not around the whole
/// popup — PortAL's own "border" is per-row too, confirmed by reading
/// `OnDrawItem`, not a window frame). Reuses [`crate::overlay::Color`]
/// rather than a new colour type — the same reasoning `y-snap`'s own
/// `Palette` already follows for its overlay.
#[derive(Clone, Copy)]
pub struct MenuColors {
    pub back: Color,
    pub text: Color,
    pub hover_back: Color,
    pub hover_text: Color,
    pub disabled_text: Color,
    pub border: Color,
}

/// What one owner-drawn item paints itself as — boxed and handed to Windows
/// as `dwItemData` (see the module doc's "Owner-draw theming" section for
/// why that, and not `dwTypeData`, is where an owner-drawn item's text has
/// to live).
struct ItemData {
    text: Vec<u16>,
    kind: ItemKind,
    colors: MenuColors,
    /// Set later, once an icon is resolved — see [`set_item_icon`]'s doc
    /// comment for why this, and not `hbmpItem`, is where a themed item's
    /// icon ends up.
    bitmap: Option<HBITMAP>,
    /// Whether this item opens a submenu — see [`ItemKind::Normal`]'s doc
    /// comment for why `draw_normal_item` needs to know this even though
    /// `kind` itself doesn't distinguish the two.
    submenu: bool,
}

enum ItemKind {
    /// A leaf command, or a submenu (`MF_POPUP`) — see [`ItemData::
    /// submenu`] for how a submenu item is told apart at draw time.
    /// Windows still draws its own arrow glyph for *any* `MF_POPUP` item
    /// after `WM_DRAWITEM` returns, even one that is `MFT_OWNERDRAW`, in a
    /// fixed system colour — confirmed by reading PortAL's own
    /// `menu.cpp::OnDrawItem`, which paints its own themed arrow and then
    /// calls `ExcludeClipRect` over that same rectangle specifically "in
    /// order to not redraw the system arrow" (their own comment). Skipping
    /// that step (an earlier version of this module did) does *not* avoid
    /// a double arrow, whatever it looks like against a light background —
    /// it just leaves Windows' own fixed-colour arrow as the only one,
    /// which goes invisible against a dark theme's background until the
    /// row is hovered into a lighter highlight. `draw_normal_item` does the
    /// same clip-exclusion PortAL does, for the same reason.
    Normal,
    /// An untitled `<separator/>` draws as a thin etched rule, same
    /// `EDGE_ETCHED` groove a plain (non-owner-draw) `MF_SEPARATOR` gets for
    /// free — a titled one draws its `text` centred with the same rule
    /// flanking it on both sides. Both are PortAL's own look
    /// (`menu.cpp::OnDrawItem`'s `MENU_SEP`/`MENU_TITLE` branches, confirmed
    /// by reading them), not a `y-pop` invention — a themed separator still
    /// needs owner-draw only so the row's *background* matches the rest of
    /// a dark-themed popup instead of showing through in whatever colour
    /// Windows would otherwise fill a native separator with.
    Separator,
}

/// The icon size every themed row's icon draws at, and the width reserved
/// for it on *every* `Normal` row regardless of whether that particular
/// item has an icon — matching a native (non-owner-draw) menu, which
/// reserves the same fixed icon/check gutter for every item so a mix of
/// icon and no-icon rows still lines its text up in one column instead of
/// each row starting wherever its own icon (or lack of one) happened to
/// end. `y-pop`'s own `Icon::to_bitmap` call sites use this directly rather
/// than keeping a second copy of the same number, so the two can never
/// drift apart.
pub const ICON_SIZE: i32 = 16;
/// Padding either side of an item's text.
const TEXT_PAD: i32 = 8;
/// Gap between the icon column and the text that follows it. `16`, not a
/// tighter value, to match PortAL's own spacing: `menu.cpp::OnDrawItem`
/// starts text at `rect.left + iconSize.cx + 24` while its icon itself
/// starts at `rect.left + 8` — so PortAL's icon-to-text gap is `24 -
/// TEXT_PAD(8) = 16`, confirmed against a real screenshot comparison
/// (`y-pop/docs/portal-analysis.md`'s pixel-numbers section), not a value
/// chosen by eye.
const ICON_GAP: i32 = 16;
/// Reserved on a submenu item's right edge for its own themed arrow glyph —
/// see [`ItemKind::Normal`]'s doc comment for why one is drawn at all.
const ARROW_WIDTH: i32 = 16;
/// An untitled separator's row height — PortAL's own fixed `10` for
/// `MENU_SEP` (`menu.cpp::OnMeasureItem`), carried over as-is: thinner than
/// a real item on purpose, the same visual cue a native `MF_SEPARATOR` row
/// already gives for free.
const SEPARATOR_HEIGHT: i32 = 10;
/// Never shorter than this even for a single-line item with no icon — a
/// themed row that's *too* tight to comfortably click is a worse trade than
/// a few extra pixels of height.
const MIN_ITEM_HEIGHT: i32 = 22;

impl PopupMenu {
    pub fn new() -> Result<Self> {
        let handle = unsafe { CreatePopupMenu()? };
        Ok(Self {
            handle,
            owned: Vec::new(),
            colors: None,
        })
    }

    /// Like [`PopupMenu::new`], but every item this menu (and every submenu
    /// it goes on to attach) appends is owner-drawn with `colors` — see the
    /// module doc's "Owner-draw theming" section.
    pub fn themed(colors: MenuColors) -> Result<Self> {
        let handle = unsafe { CreatePopupMenu()? };
        Ok(Self {
            handle,
            owned: Vec::new(),
            colors: Some(colors),
        })
    }

    /// [`PopupMenu::new`] or [`PopupMenu::themed`] depending on whether a
    /// palette is in play — the shape every submenu-constructing call site
    /// in `y-pop/src/menu.rs` actually wants: build every level of the tree
    /// the same way, themed or not, without an `if` at each one.
    pub fn new_maybe_themed(colors: Option<MenuColors>) -> Result<Self> {
        match colors {
            Some(colors) => Self::themed(colors),
            None => Self::new(),
        }
    }

    pub fn add_item(&self, id: u32, text: &str) -> Result<()> {
        match self.colors {
            Some(colors) => {
                append_owner_draw(self.handle, id as usize, false, false, text, ItemKind::Normal, colors)
            }
            None => self.append(MF_STRING, id as usize, text),
        }
    }

    /// Sets `id`'s icon — call after [`PopupMenu::add_item`] (or
    /// equivalent) has already given it that id. `bitmap` must outlive this
    /// menu: Windows does not free a custom `hbmpItem` when the item or
    /// menu holding it is destroyed, unlike the icons `hbmpChecked`/
    /// `hbmpUnchecked` conventionally point at, so the caller keeps owning
    /// it — typically for exactly as long as the popup stays open. On a
    /// themed menu this ends up on the item's own [`ItemData`] instead of
    /// `hbmpItem` — see the module doc.
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
    /// Always native — no themed caller exists yet, and a themed check mark
    /// needs its own glyph handling in [`handle_draw_item`] that isn't worth
    /// building ahead of a use for it.
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
        match self.colors {
            Some(colors) => append_owner_draw(self.handle, 0, true, false, text, ItemKind::Normal, colors),
            None => self.append(MF_STRING | MF_DISABLED | MF_GRAYED, 0, text),
        }
    }

    pub fn add_separator(&self) -> Result<()> {
        self.add_titled_separator("")
    }

    /// An optionally titled separator — on the themed path, an untitled one
    /// draws as a thin rule and a titled one draws its text centred, no
    /// rule (see [`ItemKind::Separator`]). Native `HMENU` has no
    /// titled-separator rendering of its own, so an untitled one stays a
    /// plain `MF_SEPARATOR` there and a titled one falls back to the
    /// closest native analogue, a disabled row showing the title — the same
    /// placeholder `y-pop` used before this module could draw a themed
    /// separator at all.
    pub fn add_titled_separator(&self, title: &str) -> Result<()> {
        match self.colors {
            Some(colors) => {
                append_owner_draw(self.handle, 0, false, false, title, ItemKind::Separator, colors)
            }
            None if title.is_empty() => self.append(MF_SEPARATOR, 0, ""),
            None => self.append(MF_STRING | MF_DISABLED | MF_GRAYED, 0, title),
        }
    }

    /// Attaches `submenu` under `text`. The submenu's handle is taken over by
    /// this menu and destroyed with it.
    pub fn add_submenu(&mut self, text: &str, submenu: PopupMenu) -> Result<()> {
        let handle = submenu.into_raw();
        self.owned.push(handle);
        match self.colors {
            Some(colors) => {
                append_owner_draw(self.handle, handle.0 as usize, false, true, text, ItemKind::Normal, colors)
            }
            None => self.append(MF_POPUP, handle.0 as usize, text),
        }
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
            // Reclaim every owner-drawn item's boxed `ItemData` before
            // `DestroyMenu` frees the `HMENU`s themselves — see the module
            // doc's closing paragraph for why this walks the live tree
            // instead of tracking Rust-side ownership through
            // `add_submenu`.
            unsafe { free_owner_draw_data(self.handle) };
            let _ = unsafe { DestroyMenu(self.handle) };
        }
    }
}

/// Appends one owner-drawn item to a raw `HMENU` — the free-function form
/// [`PopupMenu`]'s own themed methods build on, and the one a caller with no
/// `&mut PopupMenu` in hand can use directly (`y-pop`'s
/// `on_init_menu_popup`, populating a submenu from its raw handle — see
/// `core::menu`'s module doc). `id` is a command id for a leaf item, or the
/// submenu's own `HMENU` (as `usize`) for one created with `submenu` set —
/// `submenu` only changes the `AppendMenuW` flags (`MF_POPUP`, and that
/// handle-as-id convention); it plays no part in how the item paints itself
/// (see [`ItemKind::Normal`]'s doc comment for why).
fn append_owner_draw(
    menu: HMENU,
    id: usize,
    disabled: bool,
    submenu: bool,
    text: &str,
    kind: ItemKind,
    colors: MenuColors,
) -> Result<()> {
    let separator = matches!(kind, ItemKind::Separator);

    let data = Box::new(ItemData {
        text: text.encode_utf16().collect(),
        kind,
        colors,
        bitmap: None,
        submenu,
    });
    // Windows owns this pointer from here on, via `dwItemData` — reclaimed
    // by `free_owner_draw_data`, never dropped directly.
    let data = Box::into_raw(data);

    let mut flags = MF_OWNERDRAW;
    if submenu {
        flags |= MF_POPUP;
    }
    if separator {
        flags |= MF_SEPARATOR;
    }
    if disabled {
        flags |= MF_DISABLED | MF_GRAYED;
    }

    let result = unsafe { AppendMenuW(menu, flags, id, PCWSTR(data as *const u16)) };
    if result.is_err() {
        // Never appended — Windows will never hand this pointer back, so
        // nothing else will ever reclaim it.
        drop(unsafe { Box::from_raw(data) });
    }
    result
}

/// Appends a themed *or* plain leaf item to a raw `HMENU`, depending on
/// `colors` — for `y-pop`'s `on_init_menu_popup`, populating a `browse`
/// submenu discovered at menu-open time (see `core::menu`'s module doc).
pub fn append_item(menu: HMENU, id: u32, text: &str, colors: Option<MenuColors>) -> Result<()> {
    match colors {
        Some(colors) => append_owner_draw(menu, id as usize, false, false, text, ItemKind::Normal, colors),
        None => {
            let text = HSTRING::from(text);
            unsafe { AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(text.as_ptr())) }
        }
    }
}

/// The disabled-placeholder counterpart to [`append_item`] — an empty
/// browsed folder's `"(empty)"` row.
pub fn append_disabled_item(menu: HMENU, text: &str, colors: Option<MenuColors>) -> Result<()> {
    match colors {
        Some(colors) => append_owner_draw(menu, 0, true, false, text, ItemKind::Normal, colors),
        None => {
            let text = HSTRING::from(text);
            unsafe {
                AppendMenuW(
                    menu,
                    MF_STRING | MF_DISABLED | MF_GRAYED,
                    0,
                    PCWSTR(text.as_ptr()),
                )
            }
        }
    }
}

/// A themed or plain submenu, attached to a raw `HMENU` — the
/// [`append_item`] counterpart for a discovered subfolder.
pub fn append_submenu(menu: HMENU, text: &str, colors: Option<MenuColors>) -> Result<HMENU> {
    let submenu = unsafe { CreatePopupMenu()? };
    match colors {
        Some(colors) => append_owner_draw(
            menu,
            submenu.0 as usize,
            false,
            true,
            text,
            ItemKind::Normal,
            colors,
        )?,
        None => {
            let text = HSTRING::from(text);
            unsafe { AppendMenuW(menu, MF_POPUP, submenu.0 as usize, PCWSTR(text.as_ptr()))? };
        }
    }
    Ok(submenu)
}

/// Reclaims every owner-drawn item's boxed [`ItemData`], recursing through
/// submenus — `DestroyMenu` frees the `HMENU`s themselves but knows nothing
/// about the heap data `dwItemData` points at, entirely this process's own
/// responsibility (see the module doc). Walking the live tree right before
/// destroying it is what stays correct regardless of how deep or how it got
/// built (`PopupMenu::add_submenu`, or a raw `HMENU` from
/// [`append_submenu`] populated later by `WM_INITMENUPOPUP`).
unsafe fn free_owner_draw_data(menu: HMENU) {
    let count = item_count(menu);
    for position in 0..count {
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_DATA | MIIM_SUBMENU;
        if unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }.is_err() {
            continue;
        }
        if info.dwItemData != 0 {
            drop(unsafe { Box::from_raw(info.dwItemData as *mut ItemData) });
        }
        if !info.hSubMenu.is_invalid() {
            unsafe { free_owner_draw_data(info.hSubMenu) };
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
    // Whether this item is owner-drawn decides where the bitmap goes —
    // `MIIM_BITMAP`'s `hbmpItem` is never painted automatically once
    // `MFT_OWNERDRAW` is set (see the module doc), so a themed item keeps
    // its icon on its own `ItemData` instead, read back by
    // `handle_draw_item`. One extra `GetMenuItemInfoW` round trip to find
    // out which — negligible next to the icon extraction this always
    // follows, and it keeps every existing caller (native or themed) using
    // the exact same `set_item_icon`/`set_item_icon_by_position` call.
    let mut probe: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    probe.cbSize = size_of::<MENUITEMINFOW>() as u32;
    probe.fMask = MIIM_FTYPE | MIIM_DATA;
    unsafe { GetMenuItemInfoW(menu, item, by_position, &mut probe) }?;

    if probe.fType.contains(MFT_OWNERDRAW) && probe.dwItemData != 0 {
        let data = unsafe { &mut *(probe.dwItemData as *mut ItemData) };
        data.bitmap = Some(bitmap);
        return Ok(());
    }

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

// ---------------------------------------------------------------------
// Owner-draw painting — WM_MEASUREITEM / WM_DRAWITEM.
// ---------------------------------------------------------------------

/// Handles `WM_MEASUREITEM`. Call from the owner window's message handler
/// with the message's `lParam` (cast, as Windows sends it, to
/// `*mut MEASUREITEMSTRUCT` via `LPARAM`); returns `true` if this was an
/// owner-drawn *menu* item this module recognised (Windows also sends
/// `WM_MEASUREITEM` for owner-draw list boxes/combo boxes/buttons — the
/// `CtlType` check is what rules those out), `false` otherwise, in which
/// case the caller should fall through to `DefWindowProc`.
pub fn handle_measure_item(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let item = unsafe { &mut *(lparam.0 as *mut MEASUREITEMSTRUCT) };
    if item.CtlType != ODT_MENU || item.itemData == 0 {
        return false;
    }
    let data = unsafe { &*(item.itemData as *const ItemData) };

    let hdc = unsafe { GetDC(None) };
    let mut size = SIZE::default();
    // An empty string measures as zero width, which would starve an
    // untitled separator of the padding it still needs on either side.
    let text: Vec<u16> = if data.text.is_empty() {
        vec![b' ' as u16]
    } else {
        data.text.clone()
    };
    let _ = unsafe { GetTextExtentPoint32W(hdc, &text, &mut size) };
    unsafe { ReleaseDC(None, hdc) };

    // Reserved for every `Normal` row alike, icon or not — see `ICON_SIZE`'s
    // own doc comment for why a fixed reservation, not the real bitmap's
    // width, is what keeps every row's text aligned in one column.
    let icon_width = match data.kind {
        ItemKind::Normal => ICON_SIZE + ICON_GAP,
        ItemKind::Separator => 0,
    };
    let arrow_width = if data.submenu { ARROW_WIDTH + TEXT_PAD } else { 0 };

    item.itemWidth = (size.cx + TEXT_PAD * 2 + icon_width + arrow_width) as u32;
    item.itemHeight = match &data.kind {
        ItemKind::Separator if data.text.is_empty() => SEPARATOR_HEIGHT as u32,
        _ => (size.cy + 8).max(MIN_ITEM_HEIGHT) as u32,
    };
    true
}

/// Handles `WM_DRAWITEM`. Call from the owner window's message handler with
/// the message's `lParam` cast to `*const DRAWITEMSTRUCT`, same shape and
/// same `CtlType` filtering as [`handle_measure_item`].
pub fn handle_draw_item(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
    if item.CtlType != ODT_MENU || item.itemData == 0 {
        return false;
    }
    let data = unsafe { &*(item.itemData as *const ItemData) };

    let hdc = item.hDC;
    let rect = item.rcItem;
    let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
    let disabled = item.itemState.0
        & (windows::Win32::UI::Controls::ODS_DISABLED.0 | windows::Win32::UI::Controls::ODS_GRAYED.0)
        != 0;

    let (back, text_color) = match (disabled, selected) {
        (true, _) => (data.colors.back, data.colors.disabled_text),
        (false, true) => (data.colors.hover_back, data.colors.hover_text),
        (false, false) => (data.colors.back, data.colors.text),
    };

    unsafe {
        let brush = CreateSolidBrush(back.colorref());
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }

    match data.kind {
        ItemKind::Separator => draw_separator(hdc, rect, data, text_color),
        ItemKind::Normal => draw_normal_item(hdc, rect, data, text_color),
    }

    if selected && !disabled {
        unsafe {
            let brush = CreateSolidBrush(data.colors.border.colorref());
            FrameRect(hdc, &rect, brush);
            let _ = DeleteObject(brush.into());
        }
    }

    true
}

/// A leaf or submenu row. No arrow drawn here even for a submenu — see
/// [`ItemKind::Normal`]'s doc comment for why that would only double the
/// one Windows already draws in its own gutter.
fn draw_normal_item(hdc: HDC, rect: RECT, data: &ItemData, text_color: Color) {
    let mut text_rect = rect;
    text_rect.left += TEXT_PAD;
    text_rect.right -= TEXT_PAD;

    // Drawn flush-left in the icon column when this item has one — but the
    // column itself, and so `text_rect`'s advance past it, is reserved
    // whether or not it does (see `ICON_SIZE`'s doc comment): an item with
    // no icon must still start its text at the same x as one that has one.
    if let Some((w, h)) = data.bitmap.and_then(bitmap_size)
        && let Some(bitmap) = data.bitmap
    {
        let y = rect.top + ((rect.bottom - rect.top) - h) / 2;
        blit_icon(hdc, text_rect.left, y, w, h, bitmap);
    }
    text_rect.left += ICON_SIZE + ICON_GAP;

    if data.submenu {
        let column_left = (rect.right - TEXT_PAD - ARROW_WIDTH).max(text_rect.left);
        text_rect.right = column_left;

        // A fixed-size square glyph, vertically centred in the row — same
        // placement convention as `blit_icon`'s icon, not stretched across
        // the row's full (text-baseline-driven, not square) height the way
        // the column reserved for it is.
        let arrow_top = rect.top + ((rect.bottom - rect.top) - ARROW_WIDTH) / 2;
        let arrow_rect = RECT {
            left: column_left,
            top: arrow_top,
            right: column_left + ARROW_WIDTH,
            bottom: arrow_top + ARROW_WIDTH,
        };
        draw_arrow(hdc, arrow_rect, text_color);

        // Windows still draws its own arrow for this `MF_POPUP` item after
        // `WM_DRAWITEM` returns, in a fixed system colour that goes
        // invisible against a dark theme — see `ItemKind::Normal`'s doc
        // comment. Excluding the whole reserved column (not just the
        // smaller square glyph drawn above) from the clip region is
        // PortAL's own fix for exactly this (`menu.cpp::OnDrawItem`,
        // "in order to not redraw the system arrow" is their own comment
        // on the equivalent call), not invented here.
        unsafe {
            let _ = ExcludeClipRect(hdc, column_left, rect.top, rect.right - TEXT_PAD, rect.bottom);
        }
    }

    draw_text(hdc, data.text.clone(), text_rect, text_color, DT_LEFT);
}

/// PortAL's own separator look (`menu.cpp::OnDrawItem`'s `MENU_SEP`/
/// `MENU_TITLE` branches, confirmed by reading them): an `EDGE_ETCHED`
/// groove — the same two-tone line a plain, non-owner-draw `MF_SEPARATOR`
/// gets from Windows for free — spanning the row when untitled, or split
/// either side of the centred title when not.
fn draw_separator(hdc: HDC, rect: RECT, data: &ItemData, text_color: Color) {
    if data.text.is_empty() {
        let mut bar = RECT {
            left: rect.left + TEXT_PAD,
            right: rect.right - TEXT_PAD,
            top: (rect.top + rect.bottom) / 2,
            bottom: (rect.top + rect.bottom) / 2,
        };
        unsafe {
            let _ = DrawEdge(hdc, &mut bar, EDGE_ETCHED, BF_BOTTOM);
        }
        return;
    }

    let mut text = data.text.clone();
    let mut size = SIZE::default();
    let _ = unsafe { GetTextExtentPoint32W(hdc, &text, &mut size) };

    let mid_x = (rect.left + rect.right) / 2;
    let mut text_rect = rect;
    text_rect.left = mid_x - size.cx / 2;
    text_rect.right = text_rect.left + size.cx;

    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, text_color.colorref());
        DrawTextW(hdc, &mut text, &mut text_rect, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }

    let mid_y = (rect.top + rect.bottom) / 2;
    let mut left_bar = RECT {
        left: rect.left + TEXT_PAD,
        right: (text_rect.left - ICON_GAP).max(rect.left + TEXT_PAD),
        top: mid_y,
        bottom: mid_y,
    };
    let mut right_bar = RECT {
        left: (text_rect.right + ICON_GAP).min(rect.right - TEXT_PAD),
        right: rect.right - TEXT_PAD,
        top: mid_y,
        bottom: mid_y,
    };
    unsafe {
        let _ = DrawEdge(hdc, &mut left_bar, EDGE_ETCHED, BF_BOTTOM);
        let _ = DrawEdge(hdc, &mut right_bar, EDGE_ETCHED, BF_BOTTOM);
    }
}

fn draw_text(
    hdc: HDC,
    mut text: Vec<u16>,
    mut rect: RECT,
    color: Color,
    align: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color.colorref());
        DrawTextW(
            hdc,
            &mut text,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }
}

/// The `(width, height)` a bitmap was created at, via `GetObjectW` — every
/// icon this module draws came from [`crate::icon::Icon::to_bitmap`], which
/// always makes a square bitmap, but reading its real size back rather than
/// assuming one keeps this module decoupled from that size choice.
fn bitmap_size(bitmap: HBITMAP) -> Option<(i32, i32)> {
    let mut info: BITMAP = unsafe { std::mem::zeroed() };
    let written = unsafe {
        GetObjectW(
            HGDIOBJ::from(bitmap),
            size_of::<BITMAP>() as i32,
            Some((&raw mut info).cast()),
        )
    };
    (written != 0).then_some((info.bmWidth, info.bmHeight))
}

/// Blits a premultiplied-alpha icon bitmap (see [`crate::icon::Icon::
/// to_bitmap`]) onto `hdc` at `(x, y)` with per-pixel alpha, via a
/// throwaway memory DC — `AlphaBlend` needs the source selected into a DC
/// of its own, there is no "blend straight from a bitmap handle" call.
fn blit_icon(hdc: HDC, x: i32, y: i32, w: i32, h: i32, bitmap: HBITMAP) {
    unsafe {
        let mem_dc = CreateCompatibleDC(Some(hdc));
        if mem_dc.is_invalid() {
            return;
        }
        let previous = SelectObject(mem_dc, bitmap.into());
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = AlphaBlend(hdc, x, y, w, h, mem_dc, 0, 0, w, h, blend);
        SelectObject(mem_dc, previous);
        let _ = DeleteDC(mem_dc);
    }
}

/// Draws a right-pointing submenu arrow into `rect` (expected square, see
/// [`draw_normal_item`]'s call site), recoloured to `color` — the OS's own
/// arrow shape (`DFC_MENU`/`DFCS_MENUARROW`, the same primitive Windows
/// itself uses for a native, non-owner-draw `MF_POPUP` item's arrow) rather
/// than a font glyph, matching PortAL's own choice
/// (`menu.cpp::OnDrawItem`'s `DrawFrameControl(DFC_MENU, DFCS_MENUARROW)`
/// call, confirmed by reading it — not a font-rendered `▶`, which an
/// earlier version of this function used and which came out visibly
/// smaller/thinner than PortAL's arrow in a side-by-side screenshot
/// comparison, since a glyph's rendered size follows the menu font's
/// metrics rather than a fixed box).
///
/// `DrawFrameControl` itself only ever paints in a fixed system colour —
/// there is no colour parameter — so getting a themed arrow out of it needs
/// the same three-`BitBlt` colour-substitution trick PortAL's own
/// `OnDrawItem` uses: draw the glyph once (black arrow on white) into
/// `arrow_dc`, fill a same-size `fill_dc` solid `color`, then
/// XOR/AND/XOR the destination against `fill_dc`/`arrow_dc`/`fill_dc` in
/// that order. Where the glyph is white (background), the two XORs cancel
/// out and the destination pixel is left exactly as it was (so the row's
/// own fill still shows through around the arrow); where the glyph is
/// black (the arrow strokes), the AND zeroes the pixel and the final XOR
/// leaves it at exactly `color`.
fn draw_arrow(hdc: HDC, rect: RECT, color: Color) {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 {
        return;
    }

    unsafe {
        let arrow_dc = CreateCompatibleDC(Some(hdc));
        let fill_dc = CreateCompatibleDC(Some(hdc));
        if arrow_dc.is_invalid() || fill_dc.is_invalid() {
            let _ = DeleteDC(arrow_dc);
            let _ = DeleteDC(fill_dc);
            return;
        }

        let arrow_bitmap = CreateCompatibleBitmap(hdc, w, h);
        let fill_bitmap = CreateCompatibleBitmap(hdc, w, h);
        let old_arrow = SelectObject(arrow_dc, arrow_bitmap.into());
        let old_fill = SelectObject(fill_dc, fill_bitmap.into());

        let mut glyph_rect = RECT { left: 0, top: 0, right: w, bottom: h };
        let _ = DrawFrameControl(arrow_dc, &mut glyph_rect, DFC_MENU, DFCS_MENUARROW);

        let brush = CreateSolidBrush(color.colorref());
        FillRect(fill_dc, &glyph_rect, brush);
        let _ = DeleteObject(brush.into());

        let _ = BitBlt(hdc, rect.left, rect.top, w, h, Some(fill_dc), 0, 0, SRCINVERT);
        let _ = BitBlt(hdc, rect.left, rect.top, w, h, Some(arrow_dc), 0, 0, SRCAND);
        let _ = BitBlt(hdc, rect.left, rect.top, w, h, Some(fill_dc), 0, 0, SRCINVERT);

        SelectObject(arrow_dc, old_arrow);
        SelectObject(fill_dc, old_fill);
        let _ = DeleteObject(arrow_bitmap.into());
        let _ = DeleteObject(fill_bitmap.into());
        let _ = DeleteDC(arrow_dc);
        let _ = DeleteDC(fill_dc);
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Gdi::{CreateBitmap, GetDC, ReleaseDC};
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemInfoW, MIIM_BITMAP, MIIM_DATA, MIIM_FTYPE};

    use super::*;

    fn test_colors() -> MenuColors {
        MenuColors {
            back: Color::rgb(32, 32, 32),
            text: Color::rgb(255, 255, 255),
            hover_back: Color::rgb(0, 120, 215),
            hover_text: Color::rgb(255, 255, 255),
            disabled_text: Color::rgb(120, 120, 120),
            border: Color::rgb(0, 120, 215),
        }
    }

    /// `dwItemData` for the item at `position`, via `MIIM_DATA` — the same
    /// lookup `set_menu_item_bitmap` itself does, used here to build a
    /// synthetic `MEASUREITEMSTRUCT`/`DRAWITEMSTRUCT` without a real
    /// `TrackPopupMenu` message loop to source one from.
    fn item_data(menu: HMENU, position: u32) -> usize {
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_DATA;
        unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }.expect("GetMenuItemInfoW");
        info.dwItemData
    }

    fn measured_width(menu: HMENU, position: u32) -> u32 {
        let mut measure = MEASUREITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemWidth: 0,
            itemHeight: 0,
            itemData: item_data(menu, position),
        };
        let handled = handle_measure_item(LPARAM(&raw mut measure as isize));
        assert!(handled, "a themed item's own WM_MEASUREITEM must be recognised");
        measure.itemWidth
    }

    #[test]
    fn a_themed_menus_items_are_owner_drawn_a_native_menus_are_not() {
        let themed = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        themed.add_item(1, "Item").expect("add_item");
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_FTYPE | MIIM_DATA;
        unsafe { GetMenuItemInfoW(themed.handle(), 1, false, &mut info) }.expect("GetMenuItemInfoW");
        assert!(info.fType.contains(MFT_OWNERDRAW));
        assert_ne!(info.dwItemData, 0, "text/kind/colours live behind dwItemData");

        let native = PopupMenu::new().expect("CreatePopupMenu");
        native.add_item(1, "Item").expect("add_item");
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_FTYPE | MIIM_DATA;
        unsafe { GetMenuItemInfoW(native.handle(), 1, false, &mut info) }.expect("GetMenuItemInfoW");
        assert!(!info.fType.contains(MFT_OWNERDRAW));
        assert_eq!(info.dwItemData, 0);
    }

    #[test]
    fn an_icon_on_a_themed_item_does_not_touch_hbmpitem_or_change_its_measured_width() {
        let menu = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        menu.add_item(1, "Item").expect("add_item");
        let before = measured_width(menu.handle(), 0);

        let bitmap = unsafe { CreateBitmap(16, 16, 1, 1, None) };
        menu.set_item_icon(1, bitmap).expect("set_item_icon");

        // `hbmpItem` must stay unset: an owner-drawn item paints its own
        // icon in `handle_draw_item` — see `set_menu_item_bitmap`'s doc
        // comment for why Windows is never asked to draw one itself here.
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_BITMAP;
        unsafe { GetMenuItemInfoW(menu.handle(), 1, false, &mut info) }.expect("GetMenuItemInfoW");
        assert!(info.hbmpItem.is_invalid());

        // The icon column is reserved for every `Normal` row alike, icon or
        // not (see `ICON_SIZE`'s doc comment) — adding one must not change
        // the measured width, which is exactly what keeps a mix of icon and
        // no-icon rows lining their text up in one column.
        let after = measured_width(menu.handle(), 0);
        assert_eq!(after, before, "the icon column is reserved regardless of whether this item has an icon");

        let _ = unsafe { DeleteObject(bitmap.into()) };
    }

    #[test]
    fn a_titled_separator_measures_wider_than_an_untitled_one_on_a_themed_menu() {
        let untitled = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        untitled.add_separator().expect("add_separator");
        let untitled_width = measured_width(untitled.handle(), 0);

        let titled = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        titled
            .add_titled_separator("A rather long section title")
            .expect("add_titled_separator");
        let titled_width = measured_width(titled.handle(), 0);

        assert!(titled_width > untitled_width);
    }

    #[test]
    fn a_submenu_item_reserves_extra_width_for_its_own_arrow() {
        let mut menu = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        menu.add_item(1, "Leaf").expect("add_item");
        let leaf_width = measured_width(menu.handle(), 0);

        let submenu = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        menu.add_submenu("Leaf", submenu).expect("add_submenu");
        let submenu_width = measured_width(menu.handle(), 1);

        // Same text either way — the only possible difference is the
        // reserved arrow column (see `ItemKind::Normal`'s doc comment for
        // why one is reserved at all: Windows' own arrow is drawn in a
        // fixed system colour and must be clipped out, not relied on).
        assert!(submenu_width > leaf_width);
    }

    #[test]
    fn handle_measure_and_draw_item_ignore_messages_that_are_not_theirs() {
        // Zero itemData: a themed item always has a non-zero one (see the
        // owner-drawn test above), so this is indistinguishable from "not
        // one of ours" and must be declined, not dereferenced.
        let mut measure = MEASUREITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemWidth: 0,
            itemHeight: 0,
            itemData: 0,
        };
        assert!(!handle_measure_item(LPARAM(&raw mut measure as isize)));
        assert!(!handle_measure_item(LPARAM(0)));

        let draw = DRAWITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemAction: unsafe { std::mem::zeroed() },
            itemState: unsafe { std::mem::zeroed() },
            hwndItem: HWND::default(),
            hDC: HDC::default(),
            rcItem: RECT::default(),
            itemData: 0,
        };
        assert!(!handle_draw_item(LPARAM(&raw const draw as isize)));
        assert!(!handle_draw_item(LPARAM(0)));
    }

    #[test]
    fn a_themed_leaf_and_submenu_item_both_draw_without_a_live_popup() {
        let mut menu = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        menu.add_item(1, "Leaf").expect("add_item");
        let submenu = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        menu.add_submenu("Group", submenu).expect("add_submenu");
        menu.add_separator().expect("add_separator");

        let hdc = unsafe { GetDC(None) };
        for position in 0..menu.item_count() {
            let mut draw = DRAWITEMSTRUCT {
                CtlType: ODT_MENU,
                CtlID: 0,
                itemID: 0,
                itemAction: unsafe { std::mem::zeroed() },
                itemState: unsafe { std::mem::zeroed() },
                hwndItem: HWND(menu.handle().0 as *mut _),
                hDC: hdc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 120,
                    bottom: 22,
                },
                itemData: item_data(menu.handle(), position),
            };
            assert!(handle_draw_item(LPARAM(&raw mut draw as isize)));
        }
        unsafe { ReleaseDC(None, hdc) };
    }

    /// Nested themed submenus, several owner-drawn items each — dropping
    /// this must reclaim every `ItemData` (see `free_owner_draw_data`)
    /// without crashing or double-freeing, whether an item was added
    /// through `PopupMenu` (the `Group`) or straight onto a raw `HMENU`
    /// (the browsed-folder shape `append_item`/`append_submenu` exist for).
    #[test]
    fn dropping_a_themed_menu_with_nested_submenus_does_not_crash() {
        let mut root = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        root.add_item(1, "Leaf").expect("add_item");
        root.add_titled_separator("Section").expect("add_titled_separator");

        let inner = PopupMenu::themed(test_colors()).expect("CreatePopupMenu");
        inner.add_item(2, "Nested leaf").expect("add_item");

        let raw_submenu =
            append_submenu(inner.handle(), "Raw", Some(test_colors())).expect("append_submenu");
        append_item(raw_submenu, 3, "Raw leaf", Some(test_colors())).expect("append_item");
        append_disabled_item(raw_submenu, "(empty)", Some(test_colors())).expect("append_disabled_item");

        root.add_submenu("Group", inner).expect("add_submenu");
        drop(root);
    }
}
