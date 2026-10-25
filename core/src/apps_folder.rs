//! The packaged (Store) apps installed for the current user, read from the
//! shell's `shell:AppsFolder`.
//!
//! The folder lists every launchable app, desktop programs included; a
//! packaged app is told by its AppUserModelID, `<PackageFamilyName>!<AppId>`
//! (`Microsoft.WindowsCalculator_8wekyb3d8bbwe!App`). Desktop programs are
//! left to the Start menu scan, where they come with a real shortcut.
//!
//! The id is read as the item's absolute parsing name
//! (`shell:AppsFolder\<AppUserModelID>`), which needs no property store.
//! Enumerating touches the shell and can take a moment: call it off the UI
//! thread.

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};
use windows::core::{HSTRING, PWSTR};

/// What the shell calls the folder, as a parsing name.
pub const FOLDER: &str = "shell:AppsFolder";

/// One packaged app.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StoreApp {
    pub name: String,
    /// `<PackageFamilyName>!<AppId>`: the `app-id` of a menu entry.
    pub app_id: String,
}

/// Every packaged app in the folder, sorted by name. Empty when the folder
/// cannot be read.
pub fn store_apps() -> Vec<StoreApp> {
    // S_FALSE and RPC_E_CHANGED_MODE both mean there is an apartment.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    let mut apps = enumerate().unwrap_or_default();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps.dedup_by(|a, b| a.app_id == b.app_id);
    apps
}

/// The display name the shell gives the packaged app `app_id`, if it is
/// installed.
pub fn display_name_of(app_id: &str) -> Option<String> {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let item: IShellItem =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(format!(r"{FOLDER}\{app_id}")), None) }
            .ok()?;
    display_name(&item, SIGDN_NORMALDISPLAY).filter(|name| !name.is_empty())
}

fn enumerate() -> Option<Vec<StoreApp>> {
    let folder: IShellItem =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(FOLDER), None) }.ok()?;
    let items: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems) }.ok()?;

    let mut apps = Vec::new();
    loop {
        let mut next = [None];
        let mut fetched = 0;
        if unsafe { items.Next(&mut next, Some(&mut fetched)) }.is_err() || fetched == 0 {
            break;
        }
        let Some(item) = next[0].take() else { break };
        let parsing = display_name(&item, SIGDN_DESKTOPABSOLUTEPARSING);
        let name = display_name(&item, SIGDN_NORMALDISPLAY);
        if let (Some(parsing), Some(name)) = (parsing, name)
            && let Some(app_id) = packaged_id(&parsing)
        {
            apps.push(StoreApp {
                name,
                app_id: app_id.to_owned(),
            });
        }
    }
    Some(apps)
}

fn display_name(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    let name: PWSTR = unsafe { item.GetDisplayName(kind) }.ok()?;
    // SAFETY: the shell returned a NUL-terminated string it allocated for us.
    let text = unsafe { name.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    text
}

/// The AppUserModelID in an absolute parsing name, when it is a packaged
/// app's: `<PackageFamilyName>!<AppId>` has both an `_` (before the publisher
/// hash) and a `!`. A desktop program's is a path, a known-folder id or a bare
/// name with neither.
pub fn packaged_id(parsing: &str) -> Option<&str> {
    let id = match parsing.split_once('\\') {
        Some((folder, id)) if folder.eq_ignore_ascii_case(FOLDER) => id,
        _ => parsing,
    };
    let (family, app) = id.split_once('!')?;
    let well_formed = family.contains('_')
        && !app.is_empty()
        && !id.contains(['\\', '/', ':'])
        && !family.starts_with('{');
    well_formed.then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packaged_app_id_is_found_with_or_without_the_folder_prefix() {
        let id = "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App";
        assert_eq!(packaged_id(&format!(r"shell:AppsFolder\{id}")), Some(id));
        assert_eq!(packaged_id(&format!(r"SHELL:APPSFOLDER\{id}")), Some(id));
        assert_eq!(packaged_id(id), Some(id));
    }

    #[test]
    fn desktop_programs_are_not_packaged_apps() {
        for parsing in [
            r"shell:AppsFolder\{6D809377-6AF0-444B-8957-A3773F02200E}\Notepad\notepad.exe",
            r"shell:AppsFolder\Microsoft.Windows.Explorer",
            r"C:\Program Files\App\app.exe",
            r"shell:AppsFolder\Name_x!",
            r"shell:AppsFolder\NoUnderscore!App",
        ] {
            assert_eq!(packaged_id(parsing), None, "{parsing}");
        }
    }

    /// Reads the real folder: only what a machine has, so it asserts the
    /// shape of the answer and not its content.
    #[test]
    fn the_real_folder_lists_well_formed_apps() {
        let apps = store_apps();
        eprintln!("{} packaged apps, e.g. {:?}", apps.len(), apps.first());
        for app in apps {
            assert!(!app.name.is_empty());
            assert_eq!(packaged_id(&app.app_id), Some(app.app_id.as_str()));
        }
    }
}
