//! Launching an `App` node.
//!
//! Three routes, chosen the same way PortAL's `launch.cpp` chose them:
//! `CreateProcessW` by default, `ShellExecuteExW` when `shell`/`elevate`
//! asks for it (elevation only works through the shell — `"runas"` as the
//! verb — never through `CreateProcessW` directly), and
//! `IApplicationActivationManager::ActivateApplication` for a Store/RT
//! `app_id` instead of an `exe`.
//!
//! A launch the user asked for is also **told to Windows** ([`Usage::Logged`]),
//! so it lands in the history search reads ([`crate::search::history`]) and in
//! the Start menu's own lists. A program gets there by
//! `SEE_MASK_FLAG_LOG_USAGE`, which only `ShellExecuteExW` has, so a logged
//! launch takes the shell route, and a Store app its `shell:AppsFolder` address
//! rather than the activation manager. A file gets there by
//! `y_core::recent::add_file`. An explicit `shell="false"` means the app wants
//! the direct route: it is respected, and so it is not recorded.
//!
//! `%clipboard%`/`%clipboards%` are expanded here, and only here — see
//! `y_pop::variables`'s module doc for why `y_pop::config` deliberately
//! leaves them untouched: the clipboard's contents at config-load time are
//! not what the user wants pasted when they actually pick the item, minutes
//! or hours later.

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HGLOBAL, HWND};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoCreateInstance, CoInitialize, CoUninitialize,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    STARTUPINFOW,
};
use windows::Win32::UI::Shell::{
    AO_NONE, ApplicationActivationManager, DragQueryFileW, HDROP, IApplicationActivationManager,
    SEE_MASK_FLAG_LOG_USAGE, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, PWSTR, Result};

use crate::model::App;
use crate::variables::{self, Variable};

/// Whether a launch goes into Windows's history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Usage {
    /// Not something the user asked for just now (`start`/`exit` events),
    /// `history="false"`, or asked not to be recorded this once.
    Silent,
    /// Recorded, unless the app says `shell="false"`.
    Logged,
    /// Recorded whatever the app says, through the shell: the user asked for
    /// it, this once.
    Forced,
}

/// Extensions of what Windows does not list among recent files: programs, and
/// shortcuts, whose target it would have to be told about instead.
const NOT_DOCUMENTS: [&str; 6] = ["exe", "com", "bat", "cmd", "msc", "lnk"];

pub fn launch(hwnd: HWND, app: &App, usage: Usage) -> Result<()> {
    let logged = logs(app, usage);
    if let Some(app_id) = &app.app_id {
        if logged {
            return open_item(hwnd, &format!(r"shell:AppsFolder\{app_id}"), false, true);
        }
        return launch_packaged(app_id);
    }

    let exe = expand_lazy(app.exe.as_deref().unwrap_or_default());
    let param = app.param.as_deref().map(expand_lazy);
    let path = app.path.as_deref().map(expand_lazy);

    if app.shell == Some(true) || app.elevate || logged {
        shell_execute(
            hwnd,
            &exe,
            param.as_deref(),
            path.as_deref(),
            app.elevate,
            logged,
        )?;
    } else {
        create_process(&exe, param.as_deref(), path.as_deref())?;
    }
    if logged && is_document(&exe) {
        y_core::recent::add_file(Path::new(&exe));
    }
    Ok(())
}

/// Whether launching `app` this way tells Windows.
fn logs(app: &App, usage: Usage) -> bool {
    match usage {
        Usage::Silent => false,
        Usage::Logged => app.shell != Some(false),
        Usage::Forced => true,
    }
}

/// Opens `target` (a path, or an address such as `shell:AppsFolder\…`) through
/// the shell, telling Windows if `logged`. For the items of the history, which
/// are no [`App`].
pub fn open_item(hwnd: HWND, target: &str, elevate: bool, logged: bool) -> Result<()> {
    shell_execute(hwnd, target, None, None, elevate, logged)
}

/// Whether `file` is something Windows lists among recent files: not a
/// program, and not an address (`https://…`, `mailto:…`), which is no file.
fn is_document(file: &str) -> bool {
    let address = file
        .split_once(':')
        .is_some_and(|(scheme, _)| scheme.len() > 1);
    !address
        && !Path::new(file).extension().is_some_and(|extension| {
            NOT_DOCUMENTS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// Expands `%clipboard%`/`%clipboards%` — the one pair of tokens
/// `y_pop::config` deliberately leaves untouched at load time; everything
/// else in `text` was already resolved when the tree was parsed. Reads the
/// clipboard only when a launch actually asks for it, not on every launch.
fn expand_lazy(text: &str) -> String {
    if !text.contains("%clipboard") {
        return text.to_owned();
    }
    let variables = [
        Variable {
            name: "clipboard".to_owned(),
            value: clipboard_text(false),
        },
        Variable {
            name: "clipboards".to_owned(),
            value: clipboard_text(true),
        },
    ];
    // Nothing else is left to resolve at launch time — everything but
    // %clipboard%/%clipboards% was already expanded when the tree was
    // parsed — so there is no builtins fallback to plug in here.
    variables::expand(text, &variables, &mut Vec::new())
}

/// The clipboard's text, or the path(s) of the file(s) on it — PortAL's own
/// behaviour (see `docs/portal-analysis.md`): a single path when
/// `all_files` is false (only the first, when several are copied), every
/// path space-separated when it is true. Empty on any failure — a launch
/// that can't read the clipboard should still happen, just without that
/// argument, rather than not happen at all.
fn clipboard_text(all_files: bool) -> String {
    if unsafe { OpenClipboard(None) }.is_err() {
        return String::new();
    }
    let text = read_clipboard(all_files).unwrap_or_default();
    let _ = unsafe { CloseClipboard() };
    text
}

fn read_clipboard(all_files: bool) -> Option<String> {
    if unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32) }.is_ok() {
        let handle = unsafe { GetClipboardData(CF_HDROP.0 as u32) }.ok()?;
        let drop = HDROP(handle.0);
        let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
        if count == 0 {
            return None;
        }

        let take = if all_files { count } else { 1 };
        let mut files = Vec::with_capacity(take as usize);
        for index in 0..take {
            let mut buffer = [0u16; 260]; // MAX_PATH
            let len = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
            files.push(String::from_utf16_lossy(&buffer[..len as usize]));
        }
        return Some(files.join(" "));
    }

    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32) }.is_ok() {
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
        let memory = HGLOBAL(handle.0);
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            return None;
        }
        let text = unsafe { PCWSTR(pointer.cast()).to_string() }.ok();
        let _ = unsafe { GlobalUnlock(memory) };
        return text;
    }

    None
}

fn shell_execute(
    hwnd: HWND,
    exe: &str,
    param: Option<&str>,
    path: Option<&str>,
    elevate: bool,
    logged: bool,
) -> Result<()> {
    let exe = HSTRING::from(exe);
    let param = param.map(HSTRING::from);
    let path = path.map(HSTRING::from);
    let verb = elevate.then(|| HSTRING::from("runas"));

    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // Owns the process handle back to us so we can close it — without
        // this flag the shell closes it itself and `info.hProcess` is
        // useless, which would leak a handle if we tried to close it anyway.
        fMask: if logged {
            SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_LOG_USAGE
        } else {
            SEE_MASK_NOCLOSEPROCESS
        },
        hwnd,
        lpVerb: pcwstr(verb.as_ref()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: pcwstr(param.as_ref()),
        lpDirectory: pcwstr(path.as_ref()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&mut info) }?;
    if !info.hProcess.is_invalid() {
        let _ = unsafe { CloseHandle(info.hProcess) };
    }
    Ok(())
}

fn create_process(exe: &str, param: Option<&str>, path: Option<&str>) -> Result<()> {
    let mut command_line = format!("\"{exe}\"");
    if let Some(param) = param {
        command_line.push(' ');
        command_line.push_str(param);
    }
    // CreateProcessW may write into this buffer (it can rewrite argv[0] in
    // place), so it has to be an owned, mutable, nul-terminated buffer, not
    // a borrowed HSTRING.
    let mut command_line: Vec<u16> = command_line.encode_utf16().chain([0]).collect();

    let directory = path.map(HSTRING::from);
    let startup_info = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessW(
            None,
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(CREATE_UNICODE_ENVIRONMENT.0),
            None,
            pcwstr(directory.as_ref()),
            &startup_info,
            &mut process_info,
        )?;
    }

    let _ = unsafe { CloseHandle(process_info.hProcess) };
    let _ = unsafe { CloseHandle(process_info.hThread) };
    Ok(())
}

/// Activates a Store/RT app by its `app_id`. `CoInitialize`/`CoUninitialize`
/// paired within this one call: unlike
/// `y_core::virtual_desktop`'s cached, never-released manager (a long-lived
/// out-of-process proxy, kept deliberately), nothing here is held past this
/// function returning, so there is nothing wrong with releasing the
/// apartment immediately — and every launch pairs its own
/// initialize/uninitialize regardless of how many are already nested on this
/// thread, which is exactly what COM's reference-counted init expects.
fn launch_packaged(app_id: &str) -> Result<()> {
    unsafe { CoInitialize(None).ok()? };
    let result = (|| -> Result<()> {
        let manager = unsafe {
            CoCreateInstance::<_, IApplicationActivationManager>(
                &ApplicationActivationManager,
                None,
                CLSCTX_LOCAL_SERVER,
            )?
        };
        unsafe { manager.ActivateApplication(&HSTRING::from(app_id), &HSTRING::new(), AO_NONE) }?;
        Ok(())
    })();
    unsafe { CoUninitialize() };
    result
}

fn pcwstr(value: Option<&HSTRING>) -> PCWSTR {
    value.map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr()))
}

#[cfg(test)]
mod tests {
    use super::{Usage, is_document, logs};
    use crate::model::{App, EventMask};

    fn app(shell: Option<bool>) -> App {
        App {
            exe: None,
            app_id: None,
            param: None,
            path: None,
            shell,
            elevate: false,
            events: EventMask::default(),
            browse: None,
        }
    }

    #[test]
    fn shell_false_is_only_recorded_when_the_user_insists() {
        assert!(logs(&app(None), Usage::Logged), "no attribute: recorded");
        assert!(logs(&app(Some(true)), Usage::Logged));
        assert!(!logs(&app(Some(false)), Usage::Logged));
        assert!(logs(&app(Some(false)), Usage::Forced));
        assert!(!logs(&app(None), Usage::Silent));
        assert!(!logs(&app(Some(true)), Usage::Silent));
    }

    #[test]
    fn a_file_is_a_document_a_program_or_an_address_is_not() {
        assert!(is_document(r"C:\Docs\report.docx"));
        assert!(is_document(r"C:\Docs"));
        assert!(is_document(r"\\host\share\notes.txt"));
        assert!(!is_document(r"C:\Tools\tool.EXE"));
        assert!(!is_document(r"C:\Start\Word.lnk"));
        assert!(!is_document("notepad.exe"));
        assert!(!is_document("https://example.com/page.html"));
        assert!(!is_document("mailto:someone@example.com"));
    }
}
