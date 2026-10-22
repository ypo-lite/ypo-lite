//! The common "Open" file dialog (`IFileOpenDialog`), as one blocking call.
//!
//! ```ignore
//! let picked = open_files(Some(hwnd), &OpenOptions {
//!     title: Some("Choose a program"),
//!     filters: &[("Programs", "*.exe;*.lnk"), ("All files", "*.*")],
//!     multiple: true,
//!     ..Default::default()
//! })?;
//! ```
//!
//! Cancelling is not an error: it returns an empty list. The call runs a
//! modal loop on the calling thread, so it belongs on the UI thread, which
//! must already be in a COM apartment (a WinUI or Win32 GUI thread is); if
//! it is not, one is entered here.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
    IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, PCWSTR, Result};

/// What to ask for. All optional.
#[derive(Default)]
pub struct OpenOptions<'a> {
    pub title: Option<&'a str>,
    /// `(label, pattern)` pairs, e.g. `("Programs", "*.exe;*.lnk")`. The
    /// first is the one selected. Empty: the dialog's own "All files".
    pub filters: &'a [(&'a str, &'a str)],
    /// The folder it opens in, if it exists.
    pub start_folder: Option<&'a Path>,
    pub multiple: bool,
    /// Choose folders instead of files (`filters` do not apply).
    pub pick_folders: bool,
}

/// Shows the dialog and returns the chosen paths, in the order the dialog
/// lists them; empty when cancelled.
pub fn open_files(owner: Option<HWND>, options: &OpenOptions<'_>) -> Result<Vec<PathBuf>> {
    // S_FALSE ("already initialised") and RPC_E_CHANGED_MODE ("by someone
    // else, differently") both mean there is an apartment to use.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };

    let mut flags = FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST;
    if options.pick_folders {
        flags = (flags & !FOS_FILEMUSTEXIST) | FOS_PICKFOLDERS;
    }
    if options.multiple {
        flags |= FOS_ALLOWMULTISELECT;
    }
    unsafe { dialog.SetOptions(flags)? };

    if let Some(title) = options.title {
        unsafe { dialog.SetTitle(&HSTRING::from(title))? };
    }

    // The strings must outlive `Show`; the spec array only borrows them.
    let names: Vec<HSTRING> = options.filters.iter().map(|(name, _)| HSTRING::from(*name)).collect();
    let patterns: Vec<HSTRING> = options.filters.iter().map(|(_, pattern)| HSTRING::from(*pattern)).collect();
    if !names.is_empty() && !options.pick_folders {
        let specs: Vec<COMDLG_FILTERSPEC> = names
            .iter()
            .zip(&patterns)
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(pattern.as_ptr()),
            })
            .collect();
        unsafe {
            dialog.SetFileTypes(&specs)?;
            dialog.SetFileTypeIndex(1)?;
        }
    }

    if let Some(folder) = options.start_folder.filter(|folder| folder.is_dir()) {
        let item: Result<IShellItem> =
            unsafe { SHCreateItemFromParsingName(&HSTRING::from(folder.as_os_str()), None) };
        if let Ok(item) = item {
            unsafe { dialog.SetFolder(&item)? };
        }
    }

    match unsafe { dialog.Show(owner) } {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    }

    let results = unsafe { dialog.GetResults()? };
    let count = unsafe { results.GetCount()? };
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let item = unsafe { results.GetItemAt(index)? };
        let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        paths.push(PathBuf::from(unsafe { name.to_string() }.unwrap_or_default()));
        unsafe { CoTaskMemFree(Some(name.0.cast())) };
    }
    Ok(paths)
}
