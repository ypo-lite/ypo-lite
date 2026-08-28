//! Launching an `App` node.
//!
//! Three routes, chosen the same way PortAL's `launch.cpp` chose them:
//! `CreateProcessW` by default, `ShellExecuteExW` when `shell`/`elevate`
//! asks for it (elevation only works through the shell — `"runas"` as the
//! verb — never through `CreateProcessW` directly), and
//! `IApplicationActivationManager::ActivateApplication` for a Store/RT
//! `app_id` instead of an `exe`.
//!
//! `%clipboard%`/`%clipboards%` are expanded here, and only here — see
//! `y_pop::variables`'s module doc for why `y_pop::config` deliberately
//! leaves them untouched: the clipboard's contents at config-load time are
//! not what the user wants pasted when they actually pick the item, minutes
//! or hours later.

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
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, PWSTR, Result};

use y_pop::model::App;
use y_pop::variables::{self, Variable};

pub fn launch(hwnd: HWND, app: &App) -> Result<()> {
    if let Some(app_id) = &app.app_id {
        return launch_packaged(app_id);
    }

    let exe = expand_lazy(app.exe.as_deref().unwrap_or_default());
    let param = app.param.as_deref().map(expand_lazy);
    let path = app.path.as_deref().map(expand_lazy);

    if app.shell || app.elevate {
        shell_execute(hwnd, &exe, param.as_deref(), path.as_deref(), app.elevate)
    } else {
        create_process(&exe, param.as_deref(), path.as_deref())
    }
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
        fMask: SEE_MASK_NOCLOSEPROCESS,
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
