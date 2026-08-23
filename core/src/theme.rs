//! Whether Windows is currently asking applications to render dark.
//!
//! There is no API for this that works everywhere: `ShouldAppsUseDarkMode` is
//! an undocumented ordinal export of `uxtheme.dll`, and the WinRT
//! `UISettings` route pulls in the whole runtime. The registry value below is
//! the one Windows itself writes when the user flips the setting, it is
//! documented by convention rather than by contract, and reading it costs one
//! key open.
//!
//! An overlay is cosmetic, so every failure here means light: an unreadable
//! value is far more likely to mean "an older Windows" than "the user wanted
//! dark".

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
    RegQueryValueExW,
};
use windows::core::{PCWSTR, w};

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

/// Whether applications should use the dark palette.
pub fn is_dark() -> bool {
    // The value is named for the light case and reads 1 for light, 0 for dark.
    read_dword(w!("AppsUseLightTheme")) == Some(0)
}

/// Whether the taskbar and other system surfaces are dark, which the user can
/// set independently of [`is_dark`].
pub fn is_system_dark() -> bool {
    read_dword(w!("SystemUsesLightTheme")) == Some(0)
}

fn read_dword(name: PCWSTR) -> Option<u32> {
    let mut key = HKEY::default();
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PERSONALIZE, None, KEY_READ, &mut key) }
        .ok()
        .ok()?;

    let mut value = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = size_of::<u32>() as u32;

    let status = unsafe {
        RegQueryValueExW(
            key,
            name,
            None,
            Some(&mut kind),
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    let _ = unsafe { RegCloseKey(key) };

    (status.is_ok() && kind == REG_DWORD).then_some(value)
}
