//! Access to the resources embedded by `build.rs` from an application's `.rc`.
//!
//! The ids here must match the `#define`s in the resource script; see
//! [`crate::build`].

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, LoadIconW,
    LoadImageW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON,
};
use windows::core::{PCWSTR, Result};

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
