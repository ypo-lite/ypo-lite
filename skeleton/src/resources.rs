//! Access to the resources embedded by `build.rs` from `rsc/win-rust-min.rc`.
//!
//! The identifiers here must match the `#define`s in the resource script.

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, LoadImageW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON,
};
use windows::core::{PCWSTR, Result};

/// Icon of the application. Must match `IDI_APP_ICON` in the resource script.
pub const IDI_APP_ICON: u16 = 101;

/// Which of the icon's embedded sizes to pick.
#[derive(Clone, Copy)]
pub enum IconSize {
    /// Notification area, window caption, `WNDCLASSEXW::hIconSm` (usually 16px).
    Small,
    /// Alt+Tab, `WNDCLASSEXW::hIcon` (usually 32px).
    Large,
}

/// Loads an icon from the executable's own resources.
///
/// `LR_SHARED` hands back a cached handle owned by the system: it must *not*
/// be destroyed, and repeated calls are cheap. This is the documented way to
/// load a resource icon, as opposed to icons created at runtime.
///
/// The size is taken from the system metrics so the right image is picked out
/// of the multi-resolution `.ico`, instead of letting the shell rescale a
/// 32px image down to 16px.
pub fn load_app_icon(size: IconSize) -> Result<HICON> {
    let (cx, cy) = match size {
        IconSize::Small => (SM_CXSMICON, SM_CYSMICON),
        IconSize::Large => (SM_CXICON, SM_CYICON),
    };

    let handle = unsafe {
        LoadImageW(
            Some(module_handle()?.into()),
            resource_id(IDI_APP_ICON),
            IMAGE_ICON,
            GetSystemMetrics(cx),
            GetSystemMetrics(cy),
            LR_DEFAULTCOLOR | LR_SHARED,
        )?
    };

    Ok(HICON(handle.0))
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
