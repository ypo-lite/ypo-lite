//! RAII wrapper around a Win32 popup menu, plain or owner-drawn (themed).
//!
//! # Owner-draw theming
//!
//! [`PopupMenu::new`] behaves exactly as before: plain `MF_STRING` items,
//! native colours, `hbmpItem` for icons — the shape every tray/context menu
//! in the workspace still wants. [`PopupMenu::themed`] instead builds every
//! item `MFT_OWNERDRAW`, which hands *all* painting to this process via
//! `WM_MEASUREITEM`/`WM_DRAWITEM` — [`paint::handle_measure_item`]/
//! [`paint::handle_draw_item`], which the owner window's message loop must
//! forward both messages to (see their own doc comments for the exact call
//! shape, and [`paint`]'s own module doc for how the two nested rects it
//! draws relate to [`MenuDesign`]'s fields). `y-pop`'s themed native backend
//! (`docs/y-pop-plan.md`'s roadmap step 5) is the motivating, and so far
//! only, caller.
//!
//! Once `MFT_OWNERDRAW` is set on an item, two things Windows otherwise does
//! for free stop happening, which is why this module (and its [`paint`]
//! submodule) look the way they do:
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
//!
//! # Module layout
//!
//! This file owns the `PopupMenu` RAII wrapper, its `MenuDesign`/[`ItemData`]
//! shapes, and everything that constructs or tears down an `HMENU`/
//! `ItemData` pair. Actually *painting* one — the `WM_MEASUREITEM`/
//! `WM_DRAWITEM` handlers and the GDI calls under them — lives in
//! [`paint`], a separate file: building a menu and rendering one are
//! different enough concerns (construction/lifetime vs. per-pixel drawing)
//! that keeping them apart is worth the small cost of [`ItemData`]/
//! [`ItemKind`] needing to be visible to both — plain private items still
//! reach `paint` for free, since it's a child module of this one.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, HBITMAP};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuInfo, GetMenuItemCount, GetMenuItemInfoW,
    HMENU, MENU_ITEM_FLAGS, MENUINFO, MENUITEMINFOW, MF_CHECKED, MF_DISABLED, MF_GRAYED,
    MF_OWNERDRAW, MF_POPUP, MF_SEPARATOR, MF_STRING, MFT_OWNERDRAW, MIIM_BITMAP, MIIM_DATA,
    MIIM_FTYPE, MIIM_SUBMENU, MIM_BACKGROUND, PostMessageW, SetForegroundWindow, SetMenuInfo,
    SetMenuItemInfoW, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR, Result};

use crate::overlay::Color;

pub mod paint;
pub use paint::{ICON_SIZE, handle_draw_item, handle_measure_item};

pub struct PopupMenu {
    handle: HMENU,
    /// Submenus attached with [`PopupMenu::add_submenu`]. Windows destroys
    /// them with the parent, so they must not be destroyed twice — keeping
    /// them here (rather than as live `PopupMenu`s) is what prevents that.
    owned: Vec<HMENU>,
    /// `Some` for a menu built with [`PopupMenu::themed`] — every item this
    /// menu (and any submenu it attaches) appends from here on is
    /// owner-drawn with this design. `None` is the plain, native-rendered
    /// path every other caller in the workspace still uses.
    design: Option<MenuDesign>,
}

/// The design an owner-drawn [`PopupMenu`] paints itself with — replaces the
/// original flat `MenuDesign` (a base and hover colour pair, a dimmed
/// disabled text, a fixed 1px hover outline) with PortAL's own
/// `PortalMenuDesign` (`menu.h`) grouping: a border with its own size/round/
/// colour, a fill that's either solid or a gradient, and a per-row
/// [`margin`](Self::margin) PortAL never had at all. See
/// `y-pop/docs/y-pop-plan.md`'s "Theme system v2" section for the full
/// design rationale and the exact [`paint::handle_draw_item`] algorithm this
/// implements. Reuses [`crate::overlay::Color`] rather than a new colour
/// type — the same reasoning `y-snap`'s own `Palette` already follows for
/// its overlay.
#[derive(Clone, Copy)]
pub struct MenuDesign {
    /// An idle row's own colours.
    pub base: ColorPair,
    /// A hovered/selected row's colours — also what [`Fill::Solid`] paints
    /// and what a [`Fill::Gradient`]'s `start`/`end` sit on top of, both
    /// only ever applied to the selected row (see the module's "Theme
    /// system v2" doc: PortAL's own `OnDrawItem` gates border/round/
    /// gradient on `ODS_SELECTED` the same way, confirmed by reading it).
    pub selected: ColorPair,
    pub disabled_text: Color,
    pub border: Border,
    pub fill: Fill,
    /// Gap **between** rows — CSS would call this `margin` (space outside
    /// an element, between it and its neighbours); named `padding` here on
    /// the user's explicit correction, so the two names in this struct
    /// match the vocabulary this codebase actually uses, not CSS's. Inset,
    /// in pixels, applied to every row's own cell on all four sides before
    /// anything else is drawn — not a PortAL feature (its own `border_size`
    /// doubles as the only inset it has); combined with [`Border::round`]
    /// this is what makes a hovered row read as a floating rounded pill
    /// (Windows-11-flyout style) rather than a rounded rect butted flush
    /// against its neighbours. Applies uniformly to idle and selected rows
    /// alike, so the row grid stays evenly spaced; only the selected row's
    /// cell additionally gets `border`/`fill` treatment.
    pub padding: u32,
    /// Gap **within** a row, between its border (or its cell edge, if
    /// `border.size == 0`) and its own icon/text/arrow content — CSS would
    /// call this `padding`; named `margin` here for the same reason
    /// [`padding`](Self::padding) isn't called `margin`. Reserved for every
    /// row alike (selected or not — see [`paint::draw_normal_item`]'s own
    /// doc comment for why content must not shift position when a row's
    /// hover state toggles), on top of [`paint`]'s fixed `TEXT_PAD`
    /// baseline inset, not in place of it — `0` (this field's meaning when
    /// unset) keeps that baseline exactly as it was before this field
    /// existed.
    pub margin: u32,
}

/// An idle or selected row's text/background colour pair — was `MenuDesign`'
/// flat `back`/`text` and `hover_back`/`hover_text` fields, grouped.
#[derive(Clone, Copy)]
pub struct ColorPair {
    pub text: Color,
    pub back: Color,
}

/// A selected row's border — `size`/`round` both `0` (this type's
/// [`Default`]) reproduces the old fixed 1px `FrameRect` outline's
/// *absence* of rounding, not its presence: see [`MenuDesign`]'s "Theme
/// system v2" doc reference for why the shipped `y-pop` themes now set
/// `size = 0` (a filled pill, no stroke) rather than keeping that outline.
#[derive(Clone, Copy)]
pub struct Border {
    /// `0` = no border ring at all — the selected row's own [`Fill`] then
    /// paints its full (optionally rounded) paint rect directly.
    pub size: u32,
    /// `0` = square corners. Passed straight through to `RoundRect`'s own
    /// `width`/`height` ellipse parameters (PortAL's own
    /// `menu.cpp::OnDrawItem` call passes `border_round` for both, not a
    /// separately computed radius — carried over as-is).
    pub round: u32,
    pub color: Color,
}

/// What a selected row's inner rect (inside the border, if any) is filled
/// with.
#[derive(Clone, Copy)]
pub enum Fill {
    /// [`MenuDesign::selected`]'s own `back`, flat — every theme's default.
    Solid,
    Gradient(Gradient),
}

#[derive(Clone, Copy)]
pub struct Gradient {
    pub direction: GradientDirection,
    pub start: Color,
    pub end: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientDirection {
    Horizontal,
    Vertical,
}

/// What one owner-drawn item paints itself as — boxed and handed to Windows
/// as `dwItemData` (see the module doc's "Owner-draw theming" section for
/// why that, and not `dwTypeData`, is where an owner-drawn item's text has
/// to live). Private to `menu` (this file and [`paint`] both reach it for
/// free, since `paint` is a child module).
struct ItemData {
    text: Vec<u16>,
    kind: ItemKind,
    design: MenuDesign,
    /// Set later, once an icon is resolved — see [`set_item_icon`]'s doc
    /// comment for why this, and not `hbmpItem`, is where a themed item's
    /// icon ends up.
    bitmap: Option<HBITMAP>,
    /// Whether this item opens a submenu — see [`ItemKind::Normal`]'s doc
    /// comment for why [`paint::draw_normal_item`] needs to know this even
    /// though `kind` itself doesn't distinguish the two.
    submenu: bool,
    /// The right-aligned hint text drawn in [`MenuDesign::disabled_text`]
    /// after the label — PortAL's `sctext` (`menu.cpp`), a formatted hotkey
    /// combination such as `"Win+Y"`, never anything else. `None` for every
    /// item with nothing to show (no hotkey, or `show-shortcuts="false"`),
    /// which is also what an empty string would mean, so callers filter one
    /// out before it gets here rather than this module treating the two
    /// differently.
    shortcut: Option<Vec<u16>>,
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
    /// row is hovered into a lighter highlight. [`paint::draw_normal_item`]
    /// does the same clip-exclusion PortAL does, for the same reason.
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

impl PopupMenu {
    pub fn new() -> Result<Self> {
        let handle = unsafe { CreatePopupMenu()? };
        Ok(Self {
            handle,
            owned: Vec::new(),
            design: None,
        })
    }

    /// Like [`PopupMenu::new`], but every item this menu (and every submenu
    /// it goes on to attach) appends is owner-drawn with `colors` — see the
    /// module doc's "Owner-draw theming" section. Also paints the whole
    /// popup's own background with `design.base.back` (see
    /// [`set_menu_background`]'s doc comment) — required, not cosmetic,
    /// once [`MenuDesign::margin`] leaves a gap around every row that this
    /// row-level painting alone never covers.
    pub fn themed(design: MenuDesign) -> Result<Self> {
        let handle = unsafe { CreatePopupMenu()? };
        set_menu_background(handle, design.base.back);
        Ok(Self {
            handle,
            owned: Vec::new(),
            design: Some(design),
        })
    }

    /// [`PopupMenu::new`] or [`PopupMenu::themed`] depending on whether a
    /// palette is in play — the shape every submenu-constructing call site
    /// in `y-pop/src/menu.rs` actually wants: build every level of the tree
    /// the same way, themed or not, without an `if` at each one.
    pub fn new_maybe_themed(design: Option<MenuDesign>) -> Result<Self> {
        match design {
            Some(design) => Self::themed(design),
            None => Self::new(),
        }
    }

    pub fn add_item(&self, id: u32, text: &str) -> Result<()> {
        self.add_item_with_shortcut(id, text, None)
    }

    /// Like [`PopupMenu::add_item`], with a right-aligned shortcut hint
    /// (`"Win+Y"`, already formatted — see [`y_core::hotkey::Hotkey`]'s own
    /// `Display` impl, which already matches PortAL's `getHotkeyText`
    /// formatting) drawn after the label. `None` (or an empty string) draws
    /// exactly as [`PopupMenu::add_item`] does — a separate method rather
    /// than a plain extra parameter on it, so the many callers with no
    /// shortcut to show (`y-term`, `y-snap`, `skeleton`, this module's own
    /// tests) needed no change. On the native (unthemed) path this is a
    /// `"{text}\t{shortcut}"` composed once, up front — Win32 already
    /// right-aligns anything after a menu item's `\t` as an accelerator
    /// hint for free; only the owner-draw path needs [`paint`] to draw one
    /// itself.
    pub fn add_item_with_shortcut(&self, id: u32, text: &str, shortcut: Option<&str>) -> Result<()> {
        match self.design {
            Some(design) => append_owner_draw(
                self.handle,
                NewItem {
                    id: id as usize,
                    disabled: false,
                    submenu: false,
                    text,
                    shortcut,
                    kind: ItemKind::Normal,
                },
                design,
            ),
            None => self.append(MF_STRING, id as usize, &with_native_shortcut(text, shortcut)),
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
    /// needs its own glyph handling in [`paint::handle_draw_item`] that
    /// isn't worth building ahead of a use for it.
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
        match self.design {
            Some(design) => append_owner_draw(
                self.handle,
                NewItem { id: 0, disabled: true, submenu: false, text, shortcut: None, kind: ItemKind::Normal },
                design,
            ),
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
        match self.design {
            Some(design) => append_owner_draw(
                self.handle,
                NewItem {
                    id: 0,
                    disabled: false,
                    submenu: false,
                    text: title,
                    shortcut: None,
                    kind: ItemKind::Separator,
                },
                design,
            ),
            None if title.is_empty() => self.append(MF_SEPARATOR, 0, ""),
            None => self.append(MF_STRING | MF_DISABLED | MF_GRAYED, 0, title),
        }
    }

    /// Attaches `submenu` under `text`. The submenu's handle is taken over by
    /// this menu and destroyed with it.
    pub fn add_submenu(&mut self, text: &str, submenu: PopupMenu) -> Result<()> {
        self.add_submenu_with_shortcut(text, submenu, None)
    }

    /// Like [`PopupMenu::add_submenu`], with a shortcut hint — for a
    /// `<group>` (or `browse` folder) opened by its own `<hotkey>`, same as
    /// a leaf item's: see [`PopupMenu::add_item_with_shortcut`].
    pub fn add_submenu_with_shortcut(
        &mut self,
        text: &str,
        submenu: PopupMenu,
        shortcut: Option<&str>,
    ) -> Result<()> {
        let handle = submenu.into_raw();
        self.owned.push(handle);
        match self.design {
            Some(design) => append_owner_draw(
                self.handle,
                NewItem {
                    id: handle.0 as usize,
                    disabled: false,
                    submenu: true,
                    text,
                    shortcut,
                    kind: ItemKind::Normal,
                },
                design,
            ),
            None => self.append(MF_POPUP, handle.0 as usize, &with_native_shortcut(text, shortcut)),
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

/// `text`, or `"{text}\t{shortcut}"` if `shortcut` is non-empty — a native
/// (unthemed) menu right-aligns anything after a `\t` in its item text as an
/// accelerator hint on its own, so this is the entire native-path
/// implementation of the shortcut column; see [`PopupMenu::
/// add_item_with_shortcut`].
fn with_native_shortcut(text: &str, shortcut: Option<&str>) -> String {
    match shortcut {
        Some(shortcut) if !shortcut.is_empty() => format!("{text}\t{shortcut}"),
        _ => text.to_owned(),
    }
}

/// Everything [`append_owner_draw`] needs about the item itself, grouped so
/// the function stays under `clippy::too_many_arguments` now that a
/// shortcut hint is one more thing every caller can supply — `menu` and
/// `design` stay their own parameters since every call site already has
/// them lying around separately (a `PopupMenu`'s own handle/design fields,
/// or the raw-`HMENU` free functions' own arguments) rather than a value
/// worth constructing this struct just to carry.
struct NewItem<'a> {
    /// A command id for a leaf item, or the submenu's own `HMENU` (as
    /// `usize`) for one created with `submenu` set — `submenu` only changes
    /// the `AppendMenuW` flags (`MF_POPUP`, and that handle-as-id
    /// convention); it plays no part in how the item paints itself (see
    /// [`ItemKind::Normal`]'s doc comment for why).
    id: usize,
    disabled: bool,
    submenu: bool,
    text: &'a str,
    shortcut: Option<&'a str>,
    kind: ItemKind,
}

/// Appends one owner-drawn item to a raw `HMENU` — the free-function form
/// [`PopupMenu`]'s own themed methods build on, and the one a caller with no
/// `&mut PopupMenu` in hand can use directly (`y-pop`'s
/// `on_init_menu_popup`, populating a submenu from its raw handle — see
/// `core::menu`'s module doc).
fn append_owner_draw(menu: HMENU, item: NewItem, design: MenuDesign) -> Result<()> {
    let NewItem { id, disabled, submenu, text, shortcut, kind } = item;
    let separator = matches!(kind, ItemKind::Separator);

    let data = Box::new(ItemData {
        text: text.encode_utf16().collect(),
        kind,
        design,
        bitmap: None,
        submenu,
        shortcut: shortcut
            .filter(|s| !s.is_empty())
            .map(|s| s.encode_utf16().collect()),
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
pub fn append_item(menu: HMENU, id: u32, text: &str, design: Option<MenuDesign>) -> Result<()> {
    match design {
        Some(design) => append_owner_draw(
            menu,
            NewItem { id: id as usize, disabled: false, submenu: false, text, shortcut: None, kind: ItemKind::Normal },
            design,
        ),
        None => {
            let text = HSTRING::from(text);
            unsafe { AppendMenuW(menu, MF_STRING, id as usize, PCWSTR(text.as_ptr())) }
        }
    }
}

/// The disabled-placeholder counterpart to [`append_item`] — an empty
/// browsed folder's `"(empty)"` row.
pub fn append_disabled_item(menu: HMENU, text: &str, design: Option<MenuDesign>) -> Result<()> {
    match design {
        Some(design) => append_owner_draw(
            menu,
            NewItem { id: 0, disabled: true, submenu: false, text, shortcut: None, kind: ItemKind::Normal },
            design,
        ),
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
pub fn append_submenu(menu: HMENU, text: &str, design: Option<MenuDesign>) -> Result<HMENU> {
    let submenu = unsafe { CreatePopupMenu()? };
    match design {
        Some(design) => {
            // A submenu populated lazily (a `browse` folder's own contents,
            // via `WM_INITMENUPOPUP`) never goes through `PopupMenu::themed`
            // — this is the only place it's created, so the background
            // paint has to happen here too, not just there.
            set_menu_background(submenu, design.base.back);
            append_owner_draw(
                menu,
                NewItem {
                    id: submenu.0 as usize,
                    disabled: false,
                    submenu: true,
                    text,
                    shortcut: None,
                    kind: ItemKind::Normal,
                },
                design,
            )?
        }
        None => {
            let text = HSTRING::from(text);
            unsafe { AppendMenuW(menu, MF_POPUP, submenu.0 as usize, PCWSTR(text.as_ptr()))? };
        }
    }
    Ok(submenu)
}

/// Paints `menu`'s *whole* client background with `color`, via `SetMenuInfo`
/// (`MIIM_BACKGROUND`) — PortAL's own `OnInitMenuPopup` does the same
/// (confirmed by reading it), and it turns out not to have been optional:
/// [`MenuDesign::margin`] leaves a gap around every row that no single
/// row's own `WM_DRAWITEM` fill ever paints, so without this the gap shows
/// through to the OS's raw default menu background instead of the theme.
/// Every themed `HMENU` this module creates calls this itself
/// ([`PopupMenu::themed`], and [`append_submenu`]'s themed branch — the
/// only two places a themed `HMENU` is ever `CreatePopupMenu`'d) — no
/// caller needs to remember to. The brush this allocates is *not* freed by
/// Windows when the menu is (documented `SetMenuInfo` behaviour, and
/// confirmed by PortAL never freeing its own either) — reclaimed instead by
/// [`free_owner_draw_data`], the same recursive live-tree walk that already
/// reclaims every item's [`ItemData`] right before `DestroyMenu`.
fn set_menu_background(menu: HMENU, color: Color) {
    let brush = unsafe { CreateSolidBrush(color.colorref()) };
    let mut info: MENUINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MENUINFO>() as u32;
    info.fMask = MIM_BACKGROUND;
    info.hbrBack = brush;
    let _ = unsafe { SetMenuInfo(menu, &info) };
}

/// Reclaims every owner-drawn item's boxed [`ItemData`], recursing through
/// submenus — `DestroyMenu` frees the `HMENU`s themselves but knows nothing
/// about the heap data `dwItemData` points at, entirely this process's own
/// responsibility (see the module doc). Walking the live tree right before
/// destroying it is what stays correct regardless of how deep or how it got
/// built (`PopupMenu::add_submenu`, or a raw `HMENU` from
/// [`append_submenu`] populated later by `WM_INITMENUPOPUP`). Also reclaims
/// `menu`'s own background brush, if [`set_menu_background`] gave it one —
/// same reasoning, same "not freed by Windows" doc'd behaviour, and the same
/// live-tree-at-drop-time walk is what reaches a lazily-populated `browse`
/// submenu's brush along with everything else it needs to reclaim.
unsafe fn free_owner_draw_data(menu: HMENU) {
    let mut background: MENUINFO = unsafe { std::mem::zeroed() };
    background.cbSize = size_of::<MENUINFO>() as u32;
    background.fMask = MIM_BACKGROUND;
    if unsafe { GetMenuInfo(menu, &mut background) }.is_ok() && !background.hbrBack.is_invalid() {
        let _ = unsafe { DeleteObject(background.hbrBack.into()) };
    }

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
    // [`paint::handle_draw_item`]. One extra `GetMenuItemInfoW` round trip
    // to find out which — negligible next to the icon extraction this
    // always follows, and it keeps every existing caller (native or
    // themed) using the exact same `set_item_icon`/`set_item_icon_by_position`
    // call.
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

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Gdi::CreateBitmap;
    use windows::Win32::UI::WindowsAndMessaging::{MIIM_BITMAP, MIIM_DATA, MIIM_FTYPE};

    use super::*;

    /// Today's exact look (see [`MenuDesign::padding`]/[`MenuDesign::
    /// margin`]'s doc comments): zero-size/zero-round border, no padding,
    /// no margin, solid fill — every existing test below exercises this
    /// shape unless it says otherwise. Also used by `paint`'s own tests —
    /// duplicated there rather than shared, so the two test modules stay
    /// independent of each other.
    fn test_design() -> MenuDesign {
        MenuDesign {
            base: ColorPair {
                text: Color::rgb(255, 255, 255),
                back: Color::rgb(32, 32, 32),
            },
            selected: ColorPair {
                text: Color::rgb(255, 255, 255),
                back: Color::rgb(0, 120, 215),
            },
            disabled_text: Color::rgb(120, 120, 120),
            border: Border {
                size: 0,
                round: 0,
                color: Color::rgb(0, 120, 215),
            },
            fill: Fill::Solid,
            padding: 0,
            margin: 0,
        }
    }

    /// `dwItemData` for the item at `position`, via `MIIM_DATA` — the same
    /// lookup `set_menu_item_bitmap` itself does, used here to feed
    /// [`handle_measure_item`] without a real `TrackPopupMenu` message loop
    /// to source one from. Also used by `paint`'s own tests, for the same
    /// reason `test_design` is duplicated there.
    fn item_data(menu: HMENU, position: u32) -> usize {
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_DATA;
        unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }.expect("GetMenuItemInfoW");
        info.dwItemData
    }

    fn measured_width(menu: HMENU, position: u32) -> u32 {
        let mut measure = windows::Win32::UI::Controls::MEASUREITEMSTRUCT {
            CtlType: windows::Win32::UI::Controls::ODT_MENU,
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
        let themed = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
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

    /// Regression test: `PopupMenu::themed` must paint the *whole* popup's
    /// background, not leave that to individual rows — a themed `HMENU`
    /// that skipped this shows the OS's raw default menu background through
    /// every [`MenuDesign::margin`] gap instead of the theme, once rows
    /// stopped flat-filling their entire `rcItem` (see [`set_menu_background`]'s
    /// doc comment for how this was found: assumed already wired up by
    /// analogy with PortAL's `OnInitMenuPopup`, without checking `y-pop`
    /// itself actually did it — it didn't).
    #[test]
    fn a_themed_menu_paints_its_own_background_a_native_menu_does_not() {
        let themed = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        let mut info: MENUINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUINFO>() as u32;
        info.fMask = MIM_BACKGROUND;
        unsafe { GetMenuInfo(themed.handle(), &mut info) }.expect("GetMenuInfo");
        assert!(!info.hbrBack.is_invalid(), "a themed popup must set its own background brush");

        let native = PopupMenu::new().expect("CreatePopupMenu");
        let mut info: MENUINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUINFO>() as u32;
        info.fMask = MIM_BACKGROUND;
        unsafe { GetMenuInfo(native.handle(), &mut info) }.expect("GetMenuInfo");
        assert!(info.hbrBack.is_invalid(), "a native popup keeps the OS default, untouched");
    }

    /// A submenu created through the lazy-`browse` path ([`append_submenu`],
    /// not `PopupMenu::themed`) needs the same background paint — the other
    /// of the two places a themed `HMENU` is ever created.
    #[test]
    fn a_lazily_appended_themed_submenu_also_paints_its_own_background() {
        let root = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        let submenu = append_submenu(root.handle(), "Sub", Some(test_design())).expect("append_submenu");

        let mut info: MENUINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUINFO>() as u32;
        info.fMask = MIM_BACKGROUND;
        unsafe { GetMenuInfo(submenu, &mut info) }.expect("GetMenuInfo");
        assert!(!info.hbrBack.is_invalid());
    }

    /// A themed item's shortcut hint reserves extra width (`shortcut text
    /// width + 15`, PortAL's own gap — see `paint`'s `SHORTCUT_GAP`) at
    /// measure time, and a native item's `\t`-joined text is what
    /// `add_item_with_shortcut`
    /// composes for it — the two halves this feature's own bug report
    /// turned out to need (see `y-pop`'s docs: "no shortcut-text rendering
    /// in `y-core::menu` today, themed or not").
    #[test]
    fn a_shortcut_hint_widens_a_themed_item_and_tab_joins_a_native_ones_text() {
        let themed = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        themed.add_item(1, "Notepad").expect("add_item");
        let plain_width = measured_width(themed.handle(), 0);

        themed
            .add_item_with_shortcut(2, "Notepad", Some("Win+N"))
            .expect("add_item_with_shortcut");
        let with_shortcut_width = measured_width(themed.handle(), 1);
        assert!(
            with_shortcut_width > plain_width,
            "a shortcut hint must reserve extra row width, got {with_shortcut_width} <= {plain_width}"
        );

        let native = PopupMenu::new().expect("CreatePopupMenu");
        native
            .add_item_with_shortcut(1, "Notepad", Some("Win+N"))
            .expect("add_item_with_shortcut");
        let mut buf = [0u16; 64];
        let len = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetMenuStringW(
                native.handle(),
                1,
                Some(&mut buf),
                windows::Win32::UI::WindowsAndMessaging::MF_BYCOMMAND,
            )
        };
        let text = String::from_utf16_lossy(&buf[..len as usize]);
        assert_eq!(text, "Notepad\tWin+N", "Win32 right-aligns anything after a menu item's own \\t");
    }

    #[test]
    fn an_icon_on_a_themed_item_does_not_touch_hbmpitem_or_change_its_measured_width() {
        let menu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
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

    /// Nested themed submenus, several owner-drawn items each — dropping
    /// this must reclaim every `ItemData` (see `free_owner_draw_data`)
    /// without crashing or double-freeing, whether an item was added
    /// through `PopupMenu` (the `Group`) or straight onto a raw `HMENU`
    /// (the browsed-folder shape `append_item`/`append_submenu` exist for).
    #[test]
    fn dropping_a_themed_menu_with_nested_submenus_does_not_crash() {
        let mut root = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        root.add_item(1, "Leaf").expect("add_item");
        root.add_titled_separator("Section").expect("add_titled_separator");

        let inner = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        inner.add_item(2, "Nested leaf").expect("add_item");

        let raw_submenu =
            append_submenu(inner.handle(), "Raw", Some(test_design())).expect("append_submenu");
        append_item(raw_submenu, 3, "Raw leaf", Some(test_design())).expect("append_item");
        append_disabled_item(raw_submenu, "(empty)", Some(test_design())).expect("append_disabled_item");

        root.add_submenu("Group", inner).expect("add_submenu");
        drop(root);
    }
}
