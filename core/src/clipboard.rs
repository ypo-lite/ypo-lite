//! The clipboard's text: reading it and replacing it.
//!
//! Only plain Unicode text (`CF_UNICODETEXT`). What a launcher needs for a
//! text box's copy and paste; `y-pop`'s `%clipboard%` also reads file lists,
//! and does so itself.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::core::PCWSTR;

/// The clipboard's text, or `None` if it holds none or cannot be opened
/// (another program has it open; the caller just does nothing).
pub fn text() -> Option<String> {
    unsafe { OpenClipboard(None) }.ok()?;
    let text = read();
    let _ = unsafe { CloseClipboard() };
    text
}

fn read() -> Option<String> {
    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32) }.ok()?;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }.ok()?;
    let memory = HGLOBAL(handle.0);
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        return None;
    }
    let text = unsafe { PCWSTR(pointer.cast()).to_string() }.ok();
    let _ = unsafe { GlobalUnlock(memory) };
    text
}

/// Replaces the clipboard with `text`. `owner` is a window of this process:
/// Windows ties the clipboard's contents to one, and a clipboard emptied with
/// no owner refuses to be filled. Returns whether it worked.
pub fn set_text(owner: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * size_of::<u16>();

    let Ok(memory) = (unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }) else {
        return false;
    };
    let target = unsafe { GlobalLock(memory) };
    if target.is_null() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        return false;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target.cast::<u16>(), wide.len());
        let _ = GlobalUnlock(memory);
    }

    if unsafe { OpenClipboard(Some(owner)) }.is_err() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        return false;
    }
    let stored = unsafe { EmptyClipboard() }.is_ok()
        // On success the clipboard owns the memory; on failure we still do.
        && unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0))) }.is_ok();
    let _ = unsafe { CloseClipboard() };
    if !stored {
        let _ = unsafe { GlobalFree(Some(memory)) };
    }
    stored
}
