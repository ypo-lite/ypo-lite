//! What a file says about itself, for turning a dropped or picked file into
//! something an application can show: where a `.lnk` shortcut points, and the
//! `FileDescription` of an executable.
//!
//! Both are plain reads and neither needs a window. Both can block on a slow
//! (network, removable) path, so call them off the UI thread when the file
//! may live there.

use std::ffi::c_void;
use std::path::Path;

use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface, PCWSTR};

/// Longest path or argument string read from a shortcut. A `.lnk` can hold
/// more than `MAX_PATH`, but the shell's own getters are documented for
/// `INFOTIPSIZE`-sized buffers at most.
const BUFFER: usize = 2048;

/// What a `.lnk` file holds. A field the shortcut leaves out is empty.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ShellLinkInfo {
    /// The file or folder it opens. Empty for a shortcut to something that
    /// is not a file: a Store app, a control-panel item.
    pub target: String,
    pub arguments: String,
    pub working_dir: String,
    /// The `IconLocation` file; empty when the shortcut uses its target's.
    pub icon_file: String,
    pub icon_index: i32,
    pub description: String,
}

/// Reads the `.lnk` at `path` without resolving it (no search for a moved
/// target, no dialog). `None` when it is not a readable shortcut.
pub fn read_shell_link(path: &Path) -> Option<ShellLinkInfo> {
    // S_FALSE and RPC_E_CHANGED_MODE both mean there is an apartment.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let file: IPersistFile = link.cast().ok()?;
    unsafe { file.Load(&HSTRING::from(path.as_os_str()), STGM_READ) }.ok()?;

    let mut info = ShellLinkInfo::default();
    let mut buffer = [0u16; BUFFER];

    // SAFETY (all getters): the buffer is valid for its whole length, and
    // the shell writes a NUL-terminated string into it or leaves it empty.
    if unsafe { link.GetPath(&mut buffer, std::ptr::null_mut(), 0) }.is_ok() {
        info.target = text(&buffer);
    }
    buffer[0] = 0;
    if unsafe { link.GetArguments(&mut buffer) }.is_ok() {
        info.arguments = text(&buffer);
    }
    buffer[0] = 0;
    if unsafe { link.GetWorkingDirectory(&mut buffer) }.is_ok() {
        info.working_dir = text(&buffer);
    }
    buffer[0] = 0;
    if unsafe { link.GetDescription(&mut buffer) }.is_ok() {
        info.description = text(&buffer);
    }
    buffer[0] = 0;
    let mut index = 0;
    if unsafe { link.GetIconLocation(&mut buffer, &mut index) }.is_ok() {
        info.icon_file = text(&buffer);
        info.icon_index = index;
    }
    Some(info)
}

/// The `FileDescription` of an executable or library — "Notepad", "Visual
/// Studio Code" — in the file's own first language. `None` when the file has
/// no version resource or the description is empty.
pub fn file_description(path: &Path) -> Option<String> {
    let path = HSTRING::from(path.as_os_str());
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, block.as_mut_ptr().cast()) }.ok()?;

    // `\VarFileInfo\Translation` lists (language, code page) pairs; the
    // string table is named after the first. 040904B0 (US English, Unicode)
    // is the usual one when the list cannot be read.
    let mut translation = *b"\x09\x04\xB0\x04";
    let mut value: *mut c_void = std::ptr::null_mut();
    let mut length = 0u32;
    let found = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            windows::core::w!("\\VarFileInfo\\Translation"),
            &mut value,
            &mut length,
        )
    };
    if found.as_bool() && length >= 4 && !value.is_null() {
        // SAFETY: `VerQueryValueW` reported at least four readable bytes.
        translation.copy_from_slice(unsafe { std::slice::from_raw_parts(value.cast::<u8>(), 4) });
    }
    let language = u16::from_le_bytes([translation[0], translation[1]]);
    let code_page = u16::from_le_bytes([translation[2], translation[3]]);

    let query = HSTRING::from(format!(
        "\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription"
    ));
    let found = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            PCWSTR(query.as_ptr()),
            &mut value,
            &mut length,
        )
    };
    if !found.as_bool() || value.is_null() || length == 0 {
        return None;
    }
    // SAFETY: for a string value `length` counts UTF-16 units, terminator
    // included, inside `block`, which is still alive.
    let units = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), length as usize) };
    let description = text(units);
    let description = description.trim();
    (!description.is_empty()).then(|| description.to_owned())
}

/// A NUL-terminated UTF-16 buffer as text (up to the first NUL).
fn text(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_stops_at_the_first_nul() {
        let buffer: Vec<u16> = "abc\0def".encode_utf16().collect();
        assert_eq!(text(&buffer), "abc");
        assert_eq!(text(&[]), "");
    }

    #[test]
    fn a_system_program_has_a_description() {
        // Present on every Windows install; the exact wording is localized.
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot");
        let notepad = Path::new(&windir).join("System32").join("notepad.exe");
        let description = file_description(&notepad);
        assert!(description.is_some_and(|text| !text.is_empty()));
    }

    #[test]
    fn something_that_is_not_a_shortcut_is_none() {
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot");
        let notepad = Path::new(&windir).join("System32").join("notepad.exe");
        assert!(read_shell_link(&notepad).is_none());
        assert!(file_description(Path::new(r"C:\no\such\file.exe")).is_none());
    }
}
