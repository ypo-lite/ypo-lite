//! Access to the resources embedded by `build.rs` from an application's `.rc`.
//!
//! The ids here must match the `#define`s in the resource script; see
//! [`crate::build`].

use std::ffi::c_void;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, LoadIconW,
    LoadImageW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

/// Icon of the application. The lowest-numbered icon resource is the one
/// Explorer shows for the executable, so keep it first in the `.rc`.
pub const IDI_APP_ICON: u16 = 101;

/// Which of a multi-resolution icon's images to pick.
#[derive(Clone, Copy)]
pub enum IconSize {
    /// Notification area, window caption, `WNDCLASSEXW::hIconSm` (usually 16px).
    Small,
    /// Alt+Tab, `WNDCLASSEXW::hIcon` (usually 32px).
    Large,
}

impl IconSize {
    /// The size in physical pixels, from the system metrics.
    pub fn pixels(self) -> (i32, i32) {
        let (cx, cy) = match self {
            IconSize::Small => (SM_CXSMICON, SM_CYSMICON),
            IconSize::Large => (SM_CXICON, SM_CYICON),
        };
        unsafe { (GetSystemMetrics(cx), GetSystemMetrics(cy)) }
    }
}

/// Loads [`IDI_APP_ICON`] from the executable's own resources.
pub fn load_app_icon(size: IconSize) -> Result<HICON> {
    load_icon(IDI_APP_ICON, size)
}

/// Loads an icon resource from the executable.
///
/// `LR_SHARED` hands back a cached handle owned by the system: it must *not*
/// be destroyed, and repeated calls are cheap. This is the documented way to
/// load a resource icon, as opposed to icons created at runtime.
///
/// The size is taken from the system metrics so the right image is picked out
/// of the multi-resolution `.ico`, instead of letting the shell rescale a
/// 32px image down to 16px.
pub fn load_icon(id: u16, size: IconSize) -> Result<HICON> {
    let (cx, cy) = size.pixels();
    let handle = unsafe {
        LoadImageW(
            Some(module_handle()?.into()),
            resource_id(id),
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR | LR_SHARED,
        )?
    };

    Ok(HICON(handle.0))
}

/// The application icon, falling back to the generic system one.
///
/// Useful while an application has no `.rc` of its own yet: an icon is
/// cosmetic, and failing to start over one would be absurd.
pub fn app_icon_or_stock(size: IconSize) -> HICON {
    load_app_icon(size)
        .or_else(|_| unsafe { LoadIconW(None, IDI_APPLICATION) })
        .unwrap_or_default()
}

/// The product version in the executable's own `VERSIONINFO`, as
/// `major.minor.patch` (a fourth, build, number appears only when it is not
/// zero) — `"0.1.0"`.
///
/// `build.rs` keeps that resource in sync with `Cargo.toml` (see
/// [`crate::build`]), so this is the crate version of an application that
/// embeds resources. `None` for one that does not, or if the resource cannot
/// be read; fall back to `env!("CARGO_PKG_VERSION")` of the application.
pub fn product_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let path = HSTRING::from(exe.as_os_str());

    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, block.as_mut_ptr().cast()) }.ok()?;

    // The root of the block, `\`, is the language-neutral fixed record.
    let mut fixed: *mut c_void = std::ptr::null_mut();
    let mut length = 0u32;
    let found = unsafe { VerQueryValueW(block.as_ptr().cast(), w!("\\"), &mut fixed, &mut length) };
    if !found.as_bool() || fixed.is_null() || (length as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }

    let info = unsafe { &*fixed.cast::<VS_FIXEDFILEINFO>() };
    Some(format_version(
        info.dwProductVersionMS,
        info.dwProductVersionLS,
    ))
}

/// `VS_FIXEDFILEINFO`'s two 32-bit halves of a version, as text.
fn format_version(most: u32, least: u32) -> String {
    let (major, minor) = (most >> 16, most & 0xFFFF);
    let (patch, build) = (least >> 16, least & 0xFFFF);
    if build == 0 {
        format!("{major}.{minor}.{patch}")
    } else {
        format!("{major}.{minor}.{patch}.{build}")
    }
}

/// Handle of the running executable, which is where the resources live.
fn module_handle() -> Result<HMODULE> {
    unsafe { GetModuleHandleW(None) }
}

/// Rust equivalent of the `MAKEINTRESOURCE` macro: a numeric id is passed to
/// the Win32 API as a pointer whose value is the id itself.
fn resource_id(id: u16) -> PCWSTR {
    PCWSTR(id as usize as *const u16)
}

/// Copies `text` as UTF-16 into a fixed-size Win32 buffer, truncating if
/// needed and always leaving a terminating NUL.
pub(crate) fn copy_to_buffer(text: &str, buffer: &mut [u16]) {
    let max = buffer.len() - 1;
    let mut written = 0;
    for unit in text.encode_utf16().take(max) {
        buffer[written] = unit;
        written += 1;
    }
    buffer[written..].fill(0);
}

/// Like [`copy_to_buffer`], but a text that does not fit ends in `…` instead
/// of being cut mid-sentence. Never splits a surrogate pair.
pub(crate) fn copy_to_buffer_ellipsis(text: &str, buffer: &mut [u16]) {
    let max = buffer.len() - 1;
    if text.encode_utf16().count() <= max {
        return copy_to_buffer(text, buffer);
    }

    // One unit is kept back for the ellipsis itself.
    let mut written = 0;
    for character in text.chars() {
        let length = character.len_utf16();
        if written + length > max - 1 {
            break;
        }
        character.encode_utf16(&mut buffer[written..]);
        written += length;
    }
    buffer[written] = '…' as u16;
    buffer[written + 1..].fill(0);
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn formats_the_fixed_version_halves() {
        // 0,1,0,0 — what `y-snap.rc` declares.
        assert_eq!(format_version(0x0000_0001, 0x0000_0000), "0.1.0");
        assert_eq!(format_version(0x0001_0002, 0x0003_0000), "1.2.3");
        assert_eq!(format_version(0x0001_0002, 0x0003_0004), "1.2.3.4");
    }

    #[test]
    fn a_test_executable_has_no_version_resource() {
        // The test harness embeds none; the answer is `None`, not a panic.
        assert!(product_version().is_none());
    }
}

#[cfg(test)]
mod ellipsis_tests {
    use super::*;

    fn text_of(buffer: &[u16]) -> String {
        let end = buffer.iter().position(|&unit| unit == 0).unwrap();
        String::from_utf16(&buffer[..end]).unwrap()
    }

    #[test]
    fn short_text_is_copied_whole() {
        let mut buffer = [0xFFFF; 8];
        copy_to_buffer_ellipsis("abc", &mut buffer);
        assert_eq!(text_of(&buffer), "abc");
    }

    #[test]
    fn text_that_just_fits_is_not_ellipsized() {
        let mut buffer = [0; 4];
        copy_to_buffer_ellipsis("abc", &mut buffer);
        assert_eq!(text_of(&buffer), "abc");
    }

    #[test]
    fn long_text_ends_in_an_ellipsis_and_a_nul() {
        let mut buffer = [0xFFFF; 5];
        copy_to_buffer_ellipsis("abcdefgh", &mut buffer);
        assert_eq!(text_of(&buffer), "abc…");
    }

    #[test]
    fn a_surrogate_pair_is_never_split() {
        // 😀 is two UTF-16 units; "ab😀" does not leave room for it plus "…".
        let mut buffer = [0xFFFF; 5];
        copy_to_buffer_ellipsis("ab😀cd", &mut buffer);
        assert_eq!(text_of(&buffer), "ab…");
    }
}
