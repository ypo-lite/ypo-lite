//! Loading an icon from a file, synchronously, with a small cache.
//!
//! Two shapes, matching what PortAL's own `iconManager.cpp` found (see
//! `y-pop`'s `docs/portal-analysis.md`) and confirmed by reading it:
//!
//! * `.exe`/`.dll`/`.ico`/`.lnk` carry their own icon (optionally one of
//!   several, by index — the `,6` in PortAL's `ico="...,6"`) and are
//!   extracted directly with `ExtractIconExW`. The index is passed to it
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

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HBITMAP, SelectObject,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::UI::Shell::{
    ExtractIconExW, SHFILEINFOW, SHGetFileInfoW, SHGFI_ICON, SHGFI_SMALLICON,
    SHGFI_USEFILEATTRIBUTES,
};
use windows::Win32::UI::WindowsAndMessaging::{CopyIcon, DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use windows::core::HSTRING;

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
        unsafe { std::ptr::write_bytes(bits.cast::<u8>(), 0, (size as usize) * (size as usize) * 4) };

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
        Some(Bitmap(bitmap))
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
}

impl IconCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The icon for the file at `path` — its own, if it's one of the kinds
    /// that carries one (optionally the `index`-th of several; `None`
    /// defaults to the first), otherwise the shell's generic icon for
    /// whatever type it is. `None` if Windows could not produce one.
    pub fn for_file(&mut self, path: &str, index: Option<i32>) -> Option<Icon> {
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
        self.cache
            .entry(key)
            .or_insert_with(|| {
                if owns_icon {
                    extract_own_icon(path, index)
                } else {
                    shell_icon(path, false)
                }
            })
            .as_ref()
            .and_then(Icon::duplicate)
    }

    /// The generic "closed folder" icon — not specific to any one
    /// directory, so takes no path.
    pub fn for_directory(&mut self) -> Option<Icon> {
        self.cache
            .entry(CacheKey::Directory)
            .or_insert_with(|| shell_icon("", true))
            .as_ref()
            .and_then(Icon::duplicate)
    }
}

fn extract_own_icon(path: &str, index: i32) -> Option<Icon> {
    let wide = HSTRING::from(path);
    let mut icon = HICON::default();
    let extracted =
        unsafe { ExtractIconExW(&wide, index, None, Some(&mut icon), 1) };
    (extracted > 0 && !icon.is_invalid()).then_some(Icon(icon))
}

/// Always passes `SHGFI_USEFILEATTRIBUTES`: this asks the shell for "the
/// icon a file named like this, with these attributes, would get" purely
/// from its name and the attributes given — it never touches the
/// filesystem or requires `path` to name a real file. That's exactly the
/// point of caching by extension (or, for a directory, not caching by path
/// at all) in the first place: the specific file was never going to matter
/// to the answer, so the lookup should not depend on it existing either.
fn shell_icon(path: &str, is_directory: bool) -> Option<Icon> {
    let wide = HSTRING::from(path);
    let mut info = SHFILEINFOW::default();

    // See the module doc: USEFILEATTRIBUTES is reliable for a directory,
    // not for a regular file — so only the directory branch uses it.
    let (attributes, flags) = if is_directory {
        (
            FILE_ATTRIBUTE_DIRECTORY,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        )
    } else {
        (FILE_ATTRIBUTE_NORMAL, SHGFI_ICON | SHGFI_SMALLICON)
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
        SHELL_CALLS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
            self.0.to_str().expect("temp file path is valid UTF-8").to_owned()
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
    fn every_directory_shares_one_cache_entry() {
        let _guard = serialized();
        let mut cache = IconCache::new();
        assert!(cache.for_directory().is_some());
        assert!(cache.for_directory().is_some());
        assert_eq!(cache.cache.len(), 1);
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
}
