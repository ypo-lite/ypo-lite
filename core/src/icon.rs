//! Loading an icon from a file, synchronously, with a small cache.
//!
//! Two shapes, matching what PortAL's own `iconManager.cpp` found (see
//! `y-pop`'s `docs/portal-analysis.md`) and confirmed by reading it:
//!
//! * `.exe`/`.dll`/`.ico`/`.lnk` carry their own icon (optionally one of
//!   several, by index — the `,6` in PortAL's `ico="...,6"`) and are
//!   extracted directly with `ExtractIconExW` — except a `.lnk` without an
//!   index, whose icon lives in the link itself and which only the shell
//!   (`SHGetFileInfoW` on the real file) resolves; it is still cached per
//!   file, not per extension. The index is passed to it
//!   exactly as given, deliberately including a negative one: Windows
//!   itself treats a negative `nIconIndex` as a *resource id* rather than a
//!   position — the same `"shell32.dll,-41"` convention a `.lnk`'s icon
//!   location string uses, chosen precisely because a resource id survives
//!   a Windows version where the DLL's icons got reordered and a positional
//!   index would not. Nothing here special-cases the sign; forwarding it
//!   unmodified is what makes that convention work at all.
//! * Everything else gets the shell's generic icon for its file type —
//!   `SHGetFileInfoW` returns the identical `HICON` for every file sharing
//!   an extension (every `.txt`, every `.pdf`, …) regardless of which one
//!   was actually asked about, and the same generic "closed folder" icon
//!   for every directory, name aside. [`IconCache`] caches on exactly that
//!   basis — one shell lookup per *extension* (or one for every directory,
//!   full stop), not one per file — which is where the real saving is:
//!   `SHGetFileInfoW` talks to the shell, `ExtractIconExW` reads the file
//!   directly, and neither is free to repeat for every one of what could be
//!   hundreds of listed files in a browsed folder.
//!
//! # Sizes
//!
//! [`IconCache::new`] serves the small icons a menu draws. A cache made with
//! [`IconCache::with_size`] serves icons meant to be drawn that large, from
//! the best source for the size (a cache holds one size, so its keys do not
//! carry it):
//!
//! * up to 16 px: the small slot (`SM_CXSMICON`);
//! * up to 32 px: the large slot, which `ExtractIconExW` and
//!   `SHGetFileInfoW` give at exactly the system's large icon size, so
//!   drawing it at 32 is 1:1 and at 20 is a downscale of a good source
//!   rather than an upscale of a small one;
//! * above 32 px: the shell's own image factory (`IShellItemImageFactory`,
//!   what Store logos already go through), which returns the file's icon at
//!   the size asked — the 48 px and 256 px frames an executable carries —
//!   where the large slot would stop at 32. It needs the file to exist and
//!   ignores an icon index, so those cases fall back to the large slot;
//!   stock icons and the generic folder icon stop at 32 for the same
//!   reason (no file to hand the factory).
//!
//! Chosen by reasoning about the sources, not by comparing renders: it
//! wants a look on the machine (a 👁 in `y-pop`'s `docs/search-plan.md`).
//!
//! Synchronous only — a slow shell lookup (a file on a network share or a
//! removable disk) blocks the caller here. An async, cached-and-refreshed
//! version is later work (see `y-pop`'s `docs/y-pop-plan.md`), not this
//! module's problem to solve on its own.
//!
//! `SHGFI_USEFILEATTRIBUTES` — the flag that lets `SHGetFileInfoW` answer
//! "what icon would a file like this get" without the path needing to name
//! a real file — is reliable for a directory (confirmed: this module uses
//! it for [`IconCache::for_directory`], which never touches a real path at
//! all) but not for a regular file: on this development machine it reports
//! success while still leaving `hIcon` invalid, a known quirk rather than a
//! logic bug (found by this module's own test suite, not assumed). So a
//! generic (shared-by-extension) file lookup requires `path` to name a file
//! that actually exists — the first request for a given extension has to be
//! real, same as PortAL's own `_loadIcon`; every later request for that
//! extension is served from the cache regardless of whether *that*
//! particular file exists, since only the first one is ever queried.

use std::collections::HashMap;
use std::ffi::c_void;

use windows::Win32::Foundation::{ERROR_TIMEOUT, GetLastError, HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW,
    HBITMAP, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{
    ExtractIconExW, IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SHFILEINFOW,
    SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES, SHGSI_ICON,
    SHGSI_LARGEICON, SHGSI_SMALLICON, SHGetFileInfoW, SHGetStockIconInfo, SHSTOCKICONID,
    SHSTOCKICONINFO, SIID_APPLICATION, SIID_AUDIOFILES, SIID_DELETE, SIID_DESKTOPPC,
    SIID_DRIVEFIXED, SIID_DRIVENET, SIID_ERROR, SIID_FIND, SIID_FOLDEROPEN, SIID_HELP,
    SIID_IMAGEFILES, SIID_INFO, SIID_KEY, SIID_LINK, SIID_LOCK, SIID_MYNETWORK, SIID_PRINTER,
    SIID_RECYCLER, SIID_SERVER, SIID_SETTINGS, SIID_SHARE, SIID_SHIELD, SIID_USERS,
    SIID_VIDEOFILES, SIID_WARNING, SIID_WORLD, SIID_ZIPFILE, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateIconIndirect, DI_MASK, DI_NORMAL, DestroyIcon, DrawIconEx, GCLP_HICONSM,
    GetClassLongPtrW, HICON, ICONINFO, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};
use windows::core::{HSTRING, Interface};

/// An icon this process extracted — [`Drop`] destroys the underlying
/// `HICON`, the same RAII shape as [`crate::tray::TrayIcon`].
pub struct Icon(HICON);

impl Icon {
    pub fn handle(&self) -> HICON {
        self.0
    }

    /// A second, independently-owned handle to the same image. [`IconCache`]
    /// keeps one master `Icon` per cache entry and hands out a duplicate to
    /// every caller, so each one can drop its own copy — a menu item
    /// removed, a tray icon replaced — without disturbing the cache's own
    /// copy or any other caller's. `None` if Windows can't duplicate it,
    /// which should not happen for a handle this process itself created,
    /// but is not worth a panic if it ever does.
    fn duplicate(&self) -> Option<Icon> {
        unsafe { CopyIcon(self.0) }.ok().map(Icon)
    }

    /// A private copy of the small icon of another application's `window`,
    /// or `None` if it has none to give.
    ///
    /// Asks the window (`WM_GETICON`), then falls back to its class. The ask
    /// is bounded to 250 ms and gives up on a hung window, so a stuck
    /// application cannot freeze the caller's message loop. The handle the
    /// window hands back belongs to *it* and can vanish when it does, so what
    /// is returned is a copy, owned — and destroyed — here.
    pub fn of_window(window: HWND) -> Option<Icon> {
        // ICON_SMALL2 (the size the shell uses for its own lists), ICON_SMALL,
        // then ICON_BIG, which the system scales when drawn.
        const WHICH: [usize; 3] = [2, 0, 1];

        let mut found = 0usize;
        for which in WHICH {
            let mut reply = 0usize;
            let sent = unsafe {
                SendMessageTimeoutW(
                    window,
                    WM_GETICON,
                    WPARAM(which),
                    LPARAM(0),
                    SMTO_ABORTIFHUNG,
                    250,
                    Some(&mut reply),
                )
            };
            if sent.0 == 0 {
                // Timed out or hung: asking again would only wait again.
                if unsafe { GetLastError() } == ERROR_TIMEOUT {
                    break;
                }
                continue;
            }
            if reply != 0 {
                found = reply;
                break;
            }
        }
        if found == 0 {
            found = unsafe { GetClassLongPtrW(window, GCLP_HICONSM) } as usize;
        }
        if found == 0 {
            return None;
        }

        unsafe { CopyIcon(HICON(found as *mut c_void)) }
            .ok()
            .map(Icon)
    }

    /// Renders this icon into a `size`×`size` 32-bit ARGB bitmap suitable
    /// for a menu item's `hbmpItem` (`MIIM_BITMAP`, see
    /// [`crate::menu::PopupMenu::set_item_icon`]) — a menu wants a bitmap,
    /// not an icon, and per-pixel alpha is what lets Windows blend it
    /// properly against a themed or highlighted row instead of showing a
    /// hard-edged square. `DrawIconEx` is what actually draws the icon
    /// (mask and image both, `DI_NORMAL`) into a freshly created DIB
    /// section; the section starts as whatever memory `CreateDIBSection`
    /// handed back, so it is explicitly zeroed first — otherwise the
    /// icon's own transparent pixels would show that leftover memory
    /// through instead of coming out fully transparent.
    pub fn to_bitmap(&self, size: i32) -> Option<Bitmap> {
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: size, // positive: bottom-up: the conventional, most widely supported orientation
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let info = BITMAPINFO {
            bmiHeader: header,
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
        if bits.is_null() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            return None;
        }
        unsafe {
            std::ptr::write_bytes(bits.cast::<u8>(), 0, (size as usize) * (size as usize) * 4)
        };

        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            return None;
        }
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        let drawn = unsafe { DrawIconEx(dc, 0, 0, self.0, size, size, 0, None, DI_NORMAL) };
        unsafe { SelectObject(dc, previous) };
        let _ = unsafe { DeleteDC(dc) };

        if drawn.is_err() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            return None;
        }

        // `DrawIconEx` leaves the DIB's alpha straight (unmultiplied) — fine
        // for `hbmpItem` on an unthemed menu, where Windows' own bitmap
        // blitter reads it correctly either way, but wrong input for
        // `AlphaBlend`'s `AC_SRC_ALPHA` mode, which an owner-drawn themed
        // menu uses to paint this icon itself (`crate::menu::handle_draw_
        // item`) and assumes premultiplied channels — straight alpha there
        // shows up as a bright fringe around every icon. Premultiplying once
        // here, in place, keeps both consumers correct without either one
        // needing to know which convention the other expects.
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size as usize) * (size as usize) * 4)
        };

        // A classic icon (PuTTY's, Pageant's: 4/8-bit colour plus an AND
        // mask) has no alpha channel, so GDI draws its colours into the DIB
        // and leaves every alpha byte at 0 — premultiplied below, that is a
        // fully invisible icon. Rebuild the alpha from the icon's mask.
        if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0)
            && let Some(mask) = self.mask(size)
        {
            for (pixel, covered) in pixels.chunks_exact_mut(4).zip(mask) {
                pixel[3] = if covered { 255 } else { 0 };
            }
        }

        for pixel in pixels.chunks_exact_mut(4) {
            let a = pixel[3] as u32;
            pixel[0] = ((pixel[0] as u32 * a) / 255) as u8; // B
            pixel[1] = ((pixel[1] as u32 * a) / 255) as u8; // G
            pixel[2] = ((pixel[2] as u32 * a) / 255) as u8; // R
        }

        Some(Bitmap(bitmap))
    }

    /// This icon drawn at `size`×`size` as `0xAARRGGBB` words, premultiplied
    /// and top row first — the layout of `overlay::Canvas`'s pixels, so
    /// `Canvas::blit` can composite it. Goes through [`Icon::to_bitmap`], so
    /// it draws and handles masks the same way; `None` if that fails.
    pub fn to_pixels(&self, size: i32) -> Option<Vec<u32>> {
        let bgra = self.to_bitmap(size)?.pixels(size)?;
        let row = size as usize * 4;
        // The bitmap is bottom-up.
        let mut pixels = Vec::with_capacity(bgra.len() / 4);
        for line in bgra.chunks_exact(row).rev() {
            pixels.extend(
                line.chunks_exact(4)
                    .map(|bgra| u32::from_le_bytes([bgra[0], bgra[1], bgra[2], bgra[3]])),
            );
        }
        Some(pixels)
    }

    /// This icon as PNG bytes, `size`×`size` with straight alpha — what a UI
    /// toolkit that loads images from encoded data wants (`windows-reactor`'s
    /// `Image::source_data`). Goes through [`Icon::to_bitmap`], so it draws
    /// and handles masks the same way; `None` if that fails.
    pub fn to_png(&self, size: i32) -> Option<Vec<u8>> {
        let bgra = self.to_bitmap(size)?.pixels(size)?;
        let row = size as usize * 4;
        let mut rgba = Vec::with_capacity(bgra.len());
        // The bitmap is bottom-up and premultiplied; a PNG is top-down and straight.
        for line in bgra.chunks_exact(row).rev() {
            for pixel in line.chunks_exact(4) {
                let alpha = pixel[3] as u32;
                let straight = |channel: u8| {
                    (channel as u32 * 255)
                        .checked_div(alpha)
                        .map_or(0, |value| value.min(255) as u8)
                };
                rgba.extend_from_slice(&[
                    straight(pixel[2]),
                    straight(pixel[1]),
                    straight(pixel[0]),
                    pixel[3],
                ]);
            }
        }
        let png = crate::png::encode_rgba(size as u32, size as u32, &rgba);
        (!png.is_empty()).then_some(png)
    }

    /// Which pixels of this icon are opaque, per its AND mask (`DI_MASK`
    /// draws black where the icon covers the pixel, white where it lets the
    /// background through). Row order is the DIB's own (bottom-up), the same
    /// as [`Icon::to_bitmap`]'s, so the two line up pixel for pixel. `None`
    /// if the scratch bitmap cannot be made.
    fn mask(&self, size: i32) -> Option<Vec<bool>> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
        if bits.is_null() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            return None;
        }
        let len = (size as usize) * (size as usize) * 4;
        // White first: anything the mask does not paint black is transparent.
        unsafe { std::ptr::write_bytes(bits.cast::<u8>(), 0xFF, len) };

        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            let _ = unsafe { DeleteObject(bitmap.into()) };
            return None;
        }
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        let drawn = unsafe { DrawIconEx(dc, 0, 0, self.0, size, size, 0, None, DI_MASK) };
        unsafe { SelectObject(dc, previous) };
        let _ = unsafe { DeleteDC(dc) };

        let mask = drawn.is_ok().then(|| {
            let pixels = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), len) };
            pixels.chunks_exact(4).map(|pixel| pixel[0] == 0).collect()
        });
        let _ = unsafe { DeleteObject(bitmap.into()) };
        mask
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

/// A bitmap this process created for a menu item's `hbmpItem` — [`Drop`]
/// destroys the underlying `HBITMAP`. Unlike an [`Icon`]'s `HICON`, which
/// the shell/GDI subsystem tracks on this process's behalf, Windows does
/// *not* free a custom `hbmpItem` when the item or menu holding it is
/// destroyed — for as long as it's referenced, keeping it alive (and
/// eventually dropping it) is entirely this process's own job.
pub struct Bitmap(HBITMAP);

impl Bitmap {
    pub fn handle(&self) -> HBITMAP {
        self.0
    }

    /// A copy of the pixels of a `size`×`size` 32-bit DIB section (what
    /// [`Icon::to_bitmap`] makes): BGRA, bottom row first. `None` for a
    /// bitmap that is not one.
    fn pixels(&self, size: i32) -> Option<Vec<u8>> {
        let mut info = BITMAP::default();
        let got = unsafe {
            GetObjectW(
                self.0.into(),
                size_of::<BITMAP>() as i32,
                Some((&mut info as *mut BITMAP).cast::<c_void>()),
            )
        };
        let len = size as usize * size as usize * 4;
        if got == 0 || info.bmBits.is_null() || info.bmWidth != size || info.bmHeight != size {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(info.bmBits.cast::<u8>(), len) }.to_vec())
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

/// Extensions that carry their own icon rather than sharing the shell's
/// generic one for their type.
fn has_its_own_icon(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "exe" | "dll" | "ico" | "lnk"
    )
}

fn is_shortcut(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("lnk")
}

fn extension_of(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(_, extension)| extension)
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum CacheKey {
    /// A specific file, at a specific icon index — never shared: two
    /// different `.exe`s (or two different icons within the same one) are
    /// not interchangeable the way two `.txt` files are.
    File(String, i32),
    /// Shared by every file with this extension. `""` for an extensionless
    /// file — still one shared bucket, not one entry per such file.
    Extension(String),
    /// Shared by every directory, name aside.
    Directory,
    /// One of the shell's stock icons (see [`StockIcon`]).
    Stock(i32),
    /// The logo of a packaged app, by its AppUserModelID.
    Store(String),
}

/// A shell stock icon — for the menu entries that have no file to take an
/// icon from, and the built-in icons a config can ask for by name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StockIcon {
    /// The blue "i".
    Info,
    /// The red cross.
    Delete,
    /// The cog.
    Settings,
    Application,
    Audio,
    Computer,
    Drive,
    Error,
    Find,
    FolderOpen,
    Help,
    Images,
    Key,
    Link,
    Lock,
    Network,
    NetworkDrive,
    Printer,
    RecycleBin,
    Server,
    Share,
    Shield,
    Users,
    Video,
    Warning,
    World,
    Zip,
}

impl StockIcon {
    fn id(self) -> SHSTOCKICONID {
        match self {
            Self::Info => SIID_INFO,
            Self::Delete => SIID_DELETE,
            Self::Settings => SIID_SETTINGS,
            Self::Application => SIID_APPLICATION,
            Self::Audio => SIID_AUDIOFILES,
            Self::Computer => SIID_DESKTOPPC,
            Self::Drive => SIID_DRIVEFIXED,
            Self::Error => SIID_ERROR,
            Self::Find => SIID_FIND,
            Self::FolderOpen => SIID_FOLDEROPEN,
            Self::Help => SIID_HELP,
            Self::Images => SIID_IMAGEFILES,
            Self::Key => SIID_KEY,
            Self::Link => SIID_LINK,
            Self::Lock => SIID_LOCK,
            Self::Network => SIID_MYNETWORK,
            Self::NetworkDrive => SIID_DRIVENET,
            Self::Printer => SIID_PRINTER,
            Self::RecycleBin => SIID_RECYCLER,
            Self::Server => SIID_SERVER,
            Self::Share => SIID_SHARE,
            Self::Shield => SIID_SHIELD,
            Self::Users => SIID_USERS,
            Self::Video => SIID_VIDEOFILES,
            Self::Warning => SIID_WARNING,
            Self::World => SIID_WORLD,
            Self::Zip => SIID_ZIPFILE,
        }
    }
}

/// Resolves and caches icons — see the module doc for the two shapes and
/// why each is cached the way it is.
///
/// Cheap to create and drop per use, same reasoning as
/// [`crate::known_folders::KnownFolders`]: nothing here needs to, or
/// should, outlive whatever one batch of icons it was built to load — one
/// popup's worth of menu items, say.
#[derive(Default)]
pub struct IconCache {
    cache: HashMap<CacheKey, Option<Icon>>,
    /// The size, in pixels, the icons are meant to be drawn at; `0` is
    /// "small". See the module doc's "Sizes".
    size: i32,
}

/// The largest size the small slot serves, and the large slot after it.
const SMALL_MAX: i32 = 16;
const LARGE_MAX: i32 = 32;

impl IconCache {
    /// A cache of small icons, the size a menu draws.
    pub fn new() -> Self {
        Self::default()
    }

    /// A cache of icons meant to be drawn `size` pixels square — see the
    /// module doc's "Sizes" for where each size comes from. An icon is still
    /// scaled to the exact size when drawn (`Icon::to_bitmap`).
    pub fn with_size(size: i32) -> Self {
        Self {
            size,
            ..Self::default()
        }
    }

    fn large(&self) -> bool {
        self.size > SMALL_MAX
    }

    fn jumbo(&self) -> bool {
        self.size > LARGE_MAX
    }

    /// The icon for the file at `path` — its own, if it's one of the kinds
    /// that carries one (optionally the `index`-th of several; `None`
    /// defaults to the first), otherwise the shell's generic icon for
    /// whatever type it is. `None` if Windows could not produce one.
    pub fn for_file(&mut self, path: &str, index: Option<i32>) -> Option<Icon> {
        // A shortcut keeps its icon in the link, not in a resource
        // `ExtractIconExW` could read: only the shell resolves it (its own
        // `IconLocation`, else the target's). Still one entry per file.
        let through_shell = index.is_none() && extension_of(path).is_some_and(is_shortcut);
        let index = index.unwrap_or(0);
        let key = match extension_of(path) {
            Some(extension) if has_its_own_icon(extension) => {
                CacheKey::File(path.to_ascii_lowercase(), index)
            }
            Some(extension) => CacheKey::Extension(extension.to_ascii_lowercase()),
            None => CacheKey::Extension(String::new()),
        };

        // By reference: `key` still has to move into `.entry(key)` below.
        let owns_icon = matches!(&key, CacheKey::File(..));
        let (large, size) = (self.large(), self.size);
        // The image factory knows nothing of an icon index.
        let from_image = self.jumbo() && index == 0;
        self.cache
            .entry(key)
            .or_insert_with(|| {
                from_image
                    .then(|| shell_image(&HSTRING::from(path), size))
                    .flatten()
                    .or_else(|| {
                        if owns_icon && !through_shell {
                            extract_own_icon(path, index, large)
                        } else {
                            shell_icon(path, false, large)
                        }
                    })
            })
            .as_ref()
            .and_then(Icon::duplicate)
    }

    /// A shell stock icon, small or (for a cache above 16 px) large; never
    /// larger. `None` if Windows has none to give.
    pub fn for_stock(&mut self, stock: StockIcon) -> Option<Icon> {
        let which = if self.large() {
            SHGSI_LARGEICON
        } else {
            SHGSI_SMALLICON
        };
        self.cache
            .entry(CacheKey::Stock(stock.id().0))
            .or_insert_with(|| {
                let mut info = SHSTOCKICONINFO {
                    cbSize: size_of::<SHSTOCKICONINFO>() as u32,
                    ..Default::default()
                };
                unsafe { SHGetStockIconInfo(stock.id(), SHGSI_ICON | which, &mut info) }
                    .ok()
                    .filter(|()| !info.hIcon.is_invalid())
                    .map(|()| Icon(info.hIcon))
            })
            .as_ref()
            .and_then(Icon::duplicate)
    }

    /// The logo of the packaged (Store) app `app_id` — its
    /// AppUserModelID, `Publisher.App_8wekyb3d8bbwe!App` — as the shell shows
    /// it. `None` if the app is not installed or has no logo to give.
    pub fn for_store_app(&mut self, app_id: &str) -> Option<Icon> {
        self.cache
            .entry(CacheKey::Store(app_id.to_owned()))
            .or_insert_with(|| store_logo(app_id))
            .as_ref()
            .and_then(Icon::duplicate)
    }

    /// The generic "closed folder" icon — not specific to any one
    /// directory, so takes no path.
    pub fn for_directory(&mut self) -> Option<Icon> {
        let large = self.large();
        self.cache
            .entry(CacheKey::Directory)
            // A *non-empty* placeholder, not `""` — confirmed live, not
            // assumed: an empty `pszPath` with `SHGFI_USEFILEATTRIBUTES` |
            // `FILE_ATTRIBUTE_DIRECTORY` returns a real, valid `HICON` (so
            // the `Option` here doesn't catch it), just the wrong one — the
            // shell namespace's own drive/"This PC" icon, not a folder,
            // apparently because an empty name resolves to the desktop/root
            // namespace rather than "a nominally-named directory." Any
            // non-empty name works identically here (`SHGFI_USEFILEATTRIBUTES`
            // means the shell never touches the filesystem to check it
            // exists — see the module doc), so the literal text is
            // arbitrary; it exists only to not be empty.
            .or_insert_with(|| shell_icon("folder", true, large))
            .as_ref()
            .and_then(Icon::duplicate)
    }
}

/// Pixels asked of the shell for a Store logo; menus and lists scale it down.
const LOGO: i32 = 64;

/// How many times the shell is asked for a logo before giving up.
const LOGO_ATTEMPTS: u32 = 4;

/// The wait before the first retry; it doubles each time (250, 500, 1000 ms),
/// so a shell that is busy is given room rather than hammered.
const LOGO_RETRY_BASE_MS: u64 = 250;

/// The shell's image of the packaged app `app_id` (`shell:AppsFolder\<id>`),
/// turned into an icon so everything that draws an [`Icon`] works on it.
///
/// The shell extracts a packaged app's logo lazily and can answer `E_PENDING`
/// or a failed extraction while it is still resolving the package's resources
/// (seen live: the same app got its logo on one launch and not the next). So a
/// failure is retried a few times, with a fresh item and a short wait, first
/// icon-only and then letting the shell fall back to a thumbnail, before the
/// app is declared logo-less.
fn store_logo(app_id: &str) -> Option<Icon> {
    let name = HSTRING::from(format!(r"{}\{app_id}", crate::apps_folder::FOLDER));
    for attempt in 0..LOGO_ATTEMPTS {
        if attempt > 0 {
            let wait = LOGO_RETRY_BASE_MS << (attempt - 1);
            std::thread::sleep(std::time::Duration::from_millis(wait));
        }
        if let Some(icon) = shell_image(&name, LOGO) {
            return Some(icon);
        }
    }
    None
}

/// The shell's image of the item `name` (a path, or `shell:AppsFolder\…`) at
/// `size` pixels, as an icon: first icon-only, then letting the shell fall
/// back to a thumbnail. One attempt.
fn shell_image(name: &HSTRING, size: i32) -> Option<Icon> {
    // S_FALSE and RPC_E_CHANGED_MODE both mean there is an apartment.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let item = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(name, None) }.ok()?;
    let factory = item.cast::<IShellItemImageFactory>().ok()?;
    for flags in [SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK, SIIGBF_BIGGERSIZEOK] {
        let Ok(image) = (unsafe { factory.GetImage(SIZE { cx: size, cy: size }, flags) }) else {
            continue;
        };
        let icon = icon_from_image(image);
        let _ = unsafe { DeleteObject(image.into()) };
        if icon.is_some() {
            return icon;
        }
    }
    None
}

/// An icon with the pixels of `image`, a 32-bit bitmap from the shell.
///
/// The pixels are read top-down and, whether the shell gave them
/// premultiplied (a channel never above alpha is taken as that) or straight,
/// handed to `CreateIconIndirect` straight, as an icon file's are: the same
/// convention every other [`Icon`] here has, so `to_bitmap` treats them alike.
fn icon_from_image(image: HBITMAP) -> Option<Icon> {
    let mut info = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            image.into(),
            size_of::<BITMAP>() as i32,
            Some((&mut info as *mut BITMAP).cast::<c_void>()),
        )
    };
    let (width, height) = (info.bmWidth, info.bmHeight);
    if got == 0 || width <= 0 || height <= 0 {
        return None;
    }

    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let screen = unsafe { GetDC(None) };
    let read = unsafe {
        GetDIBits(
            screen,
            image,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast::<c_void>()),
            &mut header,
            DIB_RGB_COLORS,
        )
    };
    unsafe { ReleaseDC(None, screen) };
    if read == 0 {
        return None;
    }

    unpremultiply_or_opaque(&mut pixels);

    let color =
        unsafe { CreateBitmap(width, height, 1, 32, Some(pixels.as_ptr().cast::<c_void>())) };
    // The mask is ignored by a 32-bit colour bitmap with alpha, but must exist.
    let mask = unsafe { CreateBitmap(width, height, 1, 1, None) };
    let made = unsafe {
        CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        })
    };
    let _ = unsafe { DeleteObject(color.into()) };
    let _ = unsafe { DeleteObject(mask.into()) };
    made.ok().map(Icon)
}

/// BGRA pixels made straight. Premultiplied ones (no channel above alpha) are
/// divided by it; a bitmap with no alpha at all (every byte 0) becomes opaque.
fn unpremultiply_or_opaque(pixels: &mut [u8]) {
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        return;
    }
    let premultiplied = pixels
        .chunks_exact(4)
        .all(|pixel| pixel[..3].iter().all(|&channel| channel <= pixel[3]));
    if !premultiplied {
        return;
    }
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255) / alpha).min(255) as u8;
            }
        }
    }
}

/// How many icons the file at `path` holds (`.exe`, `.dll`, `.ico`…), 0 if
/// it has none or cannot be read. The positions `0..count` are what
/// [`IconCache::for_file`] takes as an index.
pub fn count_in_file(path: &str) -> u32 {
    let wide = HSTRING::from(path);
    // Index -1 with no buffers asks only for the count.
    unsafe { ExtractIconExW(&wide, -1, None, None, 0) }
}
fn extract_own_icon(path: &str, index: i32, large: bool) -> Option<Icon> {
    let wide = HSTRING::from(path);
    let mut icon = HICON::default();
    // The large slot is the first out-parameter, the small one the second.
    let (large_slot, small_slot) = if large {
        (Some(&mut icon as *mut HICON), None)
    } else {
        (None, Some(&mut icon as *mut HICON))
    };
    let extracted = unsafe { ExtractIconExW(&wide, index, large_slot, small_slot, 1) };
    (extracted > 0 && !icon.is_invalid()).then_some(Icon(icon))
}

/// Always passes `SHGFI_USEFILEATTRIBUTES`: this asks the shell for "the
/// icon a file named like this, with these attributes, would get" purely
/// from its name and the attributes given — it never touches the
/// filesystem or requires `path` to name a real file. That's exactly the
/// point of caching by extension (or, for a directory, not caching by path
/// at all) in the first place: the specific file was never going to matter
/// to the answer, so the lookup should not depend on it existing either.
fn shell_icon(path: &str, is_directory: bool, large: bool) -> Option<Icon> {
    let wide = HSTRING::from(path);
    let mut info = SHFILEINFOW::default();

    // See the module doc: USEFILEATTRIBUTES is reliable for a directory,
    // not for a regular file — so only the directory branch uses it.
    let size = if large {
        SHGFI_LARGEICON
    } else {
        SHGFI_SMALLICON
    };
    let (attributes, flags) = if is_directory {
        (
            FILE_ATTRIBUTE_DIRECTORY,
            SHGFI_ICON | size | SHGFI_USEFILEATTRIBUTES,
        )
    } else {
        (FILE_ATTRIBUTE_NORMAL, SHGFI_ICON | size)
    };

    let result = unsafe {
        SHGetFileInfoW(
            &wide,
            attributes,
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    (result != 0 && !info.hIcon.is_invalid()).then_some(Icon(info.hIcon))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// `cargo test`'s default parallel runner gives each test its own OS
    /// thread, and the shell/COM machinery behind `SHGetFileInfoW` is not
    /// safe to call concurrently from multiple threads with no per-thread
    /// COM initialization — confirmed directly: this whole suite is flaky
    /// under the default runner and consistently clean under
    /// `--test-threads=1`. Not a production concern (`y-pop`'s message loop
    /// is single-threaded), just a fact about this test suite, so every
    /// test here takes this lock rather than the crate serializing all of
    /// its tests, or the production code gaining a COM-init call it has no
    /// other reason to need. `unwrap_or_else` rather than `unwrap`: one
    /// test panicking while holding the lock must not poison it for every
    /// test after it.
    static SHELL_CALLS: Mutex<()> = Mutex::new(());

    fn serialized() -> std::sync::MutexGuard<'static, ()> {
        SHELL_CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A throwaway file under the system temp folder, removed on drop —
    /// `SHGetFileInfoW` needs a real file for the generic (non-directory)
    /// case, see the module doc.
    struct TempFile(std::path::PathBuf);

    impl TempFile {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(name);
            std::fs::write(&path, b"").expect("create temp file");
            Self(path)
        }

        fn path(&self) -> String {
            self.0
                .to_str()
                .expect("temp file path is valid UTF-8")
                .to_owned()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn extracts_an_icon_from_a_real_exe() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        let notepad = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\notepad.exe";
        let icon = cache.for_file(&notepad, None);
        assert!(icon.is_some(), "notepad.exe should have an icon");
    }

    #[test]
    fn counts_the_icons_in_a_real_file() {
        let dll = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\System32\shell32.dll";
        assert!(count_in_file(&dll) > 100);
        assert_eq!(count_in_file(r"C:\no\such\file.dll"), 0);
    }

    #[test]
    fn converts_to_png_bytes_that_are_not_blank() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        let notepad = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\notepad.exe";
        let icon = cache
            .for_file(&notepad, None)
            .expect("notepad.exe has an icon");
        let png = icon.to_png(32).expect("converts to a PNG");
        assert_eq!(&png[1..4], b"PNG");
        // IHDR says 32×32, and the pixels are not all one transparent value.
        assert_eq!(&png[16..24], &[0, 0, 0, 32, 0, 0, 0, 32]);
        let idat = png
            .windows(4)
            .position(|w| w == b"IDAT")
            .expect("an IDAT chunk");
        let pixels = &png[idat + 4 + 2 + 5..];
        assert!(
            pixels[..32 * 4].iter().any(|&b| b != 0)
                || pixels[32 * 4..2000].iter().any(|&b| b != 0)
        );
    }

    #[test]
    fn converts_to_a_menu_bitmap() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        let notepad = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\notepad.exe";
        let icon = cache.for_file(&notepad, None).expect("notepad.exe icon");

        let bitmap = icon.to_bitmap(16).expect("converts to a 16x16 bitmap");
        assert!(!bitmap.handle().is_invalid());
    }

    #[test]
    fn distinct_indices_of_the_same_file_are_not_conflated() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        let dll = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\System32\shell32.dll";

        let _ = cache.for_file(&dll, Some(0));
        let _ = cache.for_file(&dll, Some(3));
        // A negative index — Windows' own "resource id, not position"
        // convention (see the module doc) — is a third, distinct key of its
        // own, not somehow folded into positional index 3 or 0.
        let _ = cache.for_file(&dll, Some(-41));
        assert_eq!(
            cache.cache.len(),
            3,
            "each (path, index) pair gets its own cache entry"
        );
    }

    #[test]
    fn the_index_actually_selects_which_icon_not_always_the_first() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        // shell32.dll ships hundreds of icons on any Windows install, but
        // certainly not a million — the point is that *some* index fails
        // where index 0 does not, proving the value reaches ExtractIconExW
        // and is not silently ignored in favour of "always extract the
        // first icon."
        let dll = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())
            + r"\System32\shell32.dll";
        assert!(
            cache.for_file(&dll, Some(0)).is_some(),
            "shell32.dll's first icon should extract"
        );
        assert!(
            cache.for_file(&dll, Some(1_000_000)).is_none(),
            "shell32.dll does not have a millionth icon"
        );
    }

    #[test]
    fn the_same_extension_is_cached_and_reused() {
        let _guard = serialized();
        let a = TempFile::new("y-core-icon-test-a.txt");
        let b = TempFile::new("y-core-icon-test-b.txt");

        let mut cache = IconCache::new();
        assert!(cache.for_file(&a.path(), None).is_some());
        assert!(
            cache.for_file(&b.path(), None).is_some(),
            "a different .txt file, served from the same cache entry"
        );
        assert_eq!(cache.cache.len(), 1, "one shared entry for .txt");
    }

    #[test]
    fn exe_dll_ico_lnk_are_not_shared_by_extension() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        let _ = cache.for_file(r"C:\a.exe", None);
        let _ = cache.for_file(r"C:\b.exe", None);
        assert_eq!(cache.cache.len(), 2, "distinct files, distinct entries");
    }

    #[test]
    fn shell_pixels_are_made_straight_or_opaque() {
        // Premultiplied: 50% alpha, channels halved → back to full.
        let mut pixels = [64, 32, 16, 128];
        unpremultiply_or_opaque(&mut pixels);
        assert_eq!(pixels, [127, 63, 31, 128]);
        // Already straight (a channel above alpha): left alone.
        let mut pixels = [200, 10, 10, 100, 0, 0, 0, 255];
        unpremultiply_or_opaque(&mut pixels);
        assert_eq!(pixels, [200, 10, 10, 100, 0, 0, 0, 255]);
        // No alpha channel at all: opaque.
        let mut pixels = [1, 2, 3, 0, 4, 5, 6, 0];
        unpremultiply_or_opaque(&mut pixels);
        assert_eq!(pixels, [1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn a_real_store_app_has_a_logo_and_it_is_cached_per_app() {
        let _guard = serialized();
        let Some(app) = crate::apps_folder::store_apps().into_iter().next() else {
            return; // a machine without packaged apps
        };
        let mut cache = IconCache::new();
        let icon = cache.for_store_app(&app.app_id);
        assert!(icon.is_some(), "{}", app.app_id);
        assert!(icon.unwrap().to_png(16).is_some());
        let _ = cache.for_store_app(&app.app_id);
        assert_eq!(cache.cache.len(), 1);
        assert!(cache.for_store_app("No.Such_app!App").is_none());
    }

    #[test]
    fn every_directory_shares_one_cache_entry() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        assert!(cache.for_directory().is_some());
        assert!(cache.for_directory().is_some());
        assert_eq!(cache.cache.len(), 1);
    }

    #[test]
    fn an_empty_path_resolves_to_a_different_shell_icon_than_a_named_placeholder() {
        let _guard = serialized();
        // Regression guard for the bug `for_directory`'s `shell_icon("",
        // true)` used to have: an empty `pszPath` with
        // `SHGFI_USEFILEATTRIBUTES` | `FILE_ATTRIBUTE_DIRECTORY` returns a
        // *valid* `HICON` — so nothing here fails outright — but the wrong
        // one, the shell namespace's own drive/"This PC" icon rather than a
        // folder, apparently because an empty name resolves to the
        // desktop/root namespace instead of "a nominally-named directory."
        // `iIcon` (the system image list index `SHGetFileInfoW` also fills
        // in) is what proves the two calls answer differently, confirmed
        // here rather than assumed — `for_directory` must never go back to
        // passing `""`.
        let icon_index = |path: &str| {
            let wide = HSTRING::from(path);
            let mut info = SHFILEINFOW::default();
            let result = unsafe {
                SHGetFileInfoW(
                    &wide,
                    FILE_ATTRIBUTE_DIRECTORY,
                    Some(&mut info),
                    size_of::<SHFILEINFOW>() as u32,
                    SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
                )
            };
            assert_ne!(result, 0, "SHGetFileInfoW should succeed for {path:?}");
            let _ = unsafe { DestroyIcon(info.hIcon) };
            info.iIcon
        };

        assert_ne!(
            icon_index(""),
            icon_index("folder"),
            "an empty path must not be used for the directory icon lookup — \
             it resolves to a different (wrong) shell icon than a named \
             placeholder does"
        );
    }

    #[test]
    fn once_primed_a_nonexistent_file_of_the_same_extension_still_gets_an_icon() {
        let _guard = serialized();
        let real = TempFile::new("y-core-icon-test-prime.md");
        let mut cache = IconCache::new();
        assert!(cache.for_file(&real.path(), None).is_some());

        // Only the first .md request had to name a real file — this one is
        // served straight from the cache, never queried at all.
        let icon = cache.for_file(r"Z:\this\path\does\not\exist.md", None);
        assert!(icon.is_some());
    }

    fn notepad() -> String {
        std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned()) + r"\notepad.exe"
    }

    #[test]
    fn to_pixels_is_top_down_premultiplied_and_not_empty() {
        let _guard = serialized();
        let icon = IconCache::new()
            .for_file(&notepad(), None)
            .expect("an icon");
        let pixels = icon.to_pixels(16).expect("pixels");
        assert_eq!(pixels.len(), 16 * 16);

        assert!(pixels.iter().any(|&p| p >> 24 != 0), "something is drawn");
        for &pixel in &pixels {
            let alpha = pixel >> 24;
            for shift in [0, 8, 16] {
                assert!(
                    (pixel >> shift) & 0xFF <= alpha,
                    "premultiplied: {pixel:#010x}"
                );
            }
        }
    }

    #[test]
    fn every_size_tier_yields_an_icon_that_draws_at_its_size() {
        let _guard = serialized();
        for size in [16, 20, 32, 48, 64] {
            let mut cache = IconCache::with_size(size);
            let icon = cache
                .for_file(&notepad(), None)
                .unwrap_or_else(|| panic!("no icon at {size}"));
            let pixels = icon.to_pixels(size).expect("pixels");
            assert_eq!(pixels.len() as i32, size * size);
            assert!(pixels.iter().any(|&p| p >> 24 != 0), "blank at {size}");

            assert!(cache.for_directory().is_some(), "folder at {size}");
            assert!(
                cache.for_stock(StockIcon::Info).is_some(),
                "stock at {size}"
            );
        }
    }

    /// A bigger tier is a different source, not the small icon blown up: the
    /// large slot of an executable is a 32 px frame, so drawing it at 32 has
    /// more distinct pixels than the small one stretched to the same size.
    #[test]
    fn a_bigger_cache_gives_a_richer_icon() {
        let _guard = serialized();
        let distinct = |size: i32| {
            let icon = IconCache::with_size(size)
                .for_file(&notepad(), None)
                .expect("an icon");
            let mut colours = icon.to_pixels(32).expect("pixels");
            colours.sort_unstable();
            colours.dedup();
            colours.len()
        };
        assert!(distinct(32) >= distinct(16));
    }

    #[test]
    fn draw_icon_lands_inside_its_square_and_nowhere_else() {
        use crate::overlay::Canvas;
        let _guard = serialized();
        let icon = IconCache::new()
            .for_file(&notepad(), None)
            .expect("an icon");

        let mut canvas = Canvas::new((40, 40)).unwrap();
        canvas.clear(crate::overlay::Color::TRANSPARENT);
        canvas.draw_icon(&icon, 10, 12, 16);

        let pixels = canvas.pixels_mut().to_vec();
        let mut inside = 0;
        for (index, &pixel) in pixels.iter().enumerate() {
            let (x, y) = ((index % 40) as i32, (index / 40) as i32);
            if (10..26).contains(&x) && (12..28).contains(&y) {
                inside += usize::from(pixel != 0);
            } else {
                assert_eq!(pixel, 0, "drawn outside the square at {x},{y}");
            }
        }
        assert!(inside > 0, "nothing was drawn");

        // And it matches the icon's own pixels where it is opaque.
        let own = icon.to_pixels(16).unwrap();
        let first_opaque = own
            .iter()
            .position(|&p| p >> 24 == 255)
            .expect("opaque pixel");
        let (x, y) = (first_opaque % 16, first_opaque / 16);
        assert_eq!(pixels[(12 + y) * 40 + 10 + x], own[first_opaque]);
    }
}
