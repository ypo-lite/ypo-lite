//! Run at logon, through the per-user `Run` key.
//!
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` is the right place for
//! a tray tool: it needs no elevation, it is per-user, and the Startup tab of
//! Task Manager lists it so the user can turn it off where they expect to.
//! (A scheduled task or a service would be neither.)

use windows::Win32::Foundation::{E_FAIL, ERROR_FILE_NOT_FOUND};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{Error, HSTRING, PCWSTR, Result, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Whether `name` is registered to start at logon.
pub fn is_enabled(name: &str) -> bool {
    let Ok(key) = open(KEY_READ) else {
        return false;
    };
    let name = HSTRING::from(name);

    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&mut size),
        )
    };

    status.is_ok()
}

/// Registers this executable to start at logon under `name`.
///
/// The path is quoted: without quotes Windows splits `C:\Program Files\...`
/// at the space and starts `C:\Program.exe`.
pub fn enable(name: &str) -> Result<()> {
    let exe = std::env::current_exe()
        .map_err(|error| Error::new(E_FAIL, format!("cannot locate the executable: {error}")))?;
    let command = HSTRING::from(format!("\"{}\"", exe.display()));
    let name = HSTRING::from(name);

    let key = open(KEY_WRITE)?;

    // REG_SZ is measured in bytes, terminator included.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            command.as_ptr() as *const u8,
            (command.len() + 1) * size_of::<u16>(),
        )
    };

    unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) }.ok()
}

/// Removes the registration. Succeeds if it was not there.
pub fn disable(name: &str) -> Result<()> {
    let key = open(KEY_WRITE)?;
    let name = HSTRING::from(name);

    let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    status.ok()
}

/// [`enable`] or [`disable`], for a settings checkbox.
pub fn set_enabled(name: &str, enabled: bool) -> Result<()> {
    if enabled { enable(name) } else { disable(name) }
}

/// Closes the registry key on drop, including on the early returns above.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Result<Key> {
    let mut key = HKEY::default();
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, access, &mut key) }.ok()?;
    Ok(Key(key))
}
