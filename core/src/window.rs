//! Window classes, the window procedure, and the message loop.
//!
//! The tricky part of any Win32 application in Rust is routing the C callback
//! (`wndproc`) back to a Rust value. The pattern used here is the canonical
//! one:
//!
//! 1. the handler is boxed and passed through `CreateWindowExW`'s `lpParam`,
//! 2. `WM_NCCREATE` stores that pointer in the window's `GWLP_USERDATA` slot,
//! 3. every later message borrows the handler back from that slot,
//! 4. `WM_NCDESTROY` (the very last message a window receives) drops the box.
//!
//! Nothing in the applications needs `unsafe`, which is exactly the point.

use std::ffi::c_void;

use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, HICON, IDC_ARROW, LoadCursorW, MSG,
    PostQuitMessage, RegisterClassExW, SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_NCCREATE, WM_NCDESTROY, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{Error, HRESULT, HSTRING, PCWSTR, Result};

/// Implemented by whatever owns the state behind a window.
///
/// Return `Some(result)` for messages you handled, `None` to let
/// `DefWindowProcW` do its job.
pub trait WindowHandler {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT>;
}

/// Icons registered with the window class: shown in Alt+Tab (`large`) and in
/// the window caption or task switcher list (`small`). `None` falls back to
/// the system default.
#[derive(Clone, Copy, Default)]
pub struct WindowIcons {
    pub large: Option<HICON>,
    pub small: Option<HICON>,
}

/// Everything [`create`] needs to know. Build one with [`WindowSpec::hidden`]
/// or [`WindowSpec::new`] and adjust the fields that matter.
#[derive(Clone, Copy)]
pub struct WindowSpec<'a> {
    /// Window class name. Must be unique inside the process — prefix it with
    /// the application, e.g. `"YpoLite.YSnap.Main"`.
    pub class: &'a str,
    pub title: &'a str,
    pub style: WINDOW_STYLE,
    pub ex_style: WINDOW_EX_STYLE,
    /// Screen position and size, in physical pixels.
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub icons: WindowIcons,
}

impl<'a> WindowSpec<'a> {
    /// A plain top-level window, not shown until `ShowWindow`.
    pub const fn new(class: &'a str, title: &'a str) -> Self {
        Self {
            class,
            title,
            style: WS_OVERLAPPEDWINDOW,
            ex_style: WINDOW_EX_STYLE(0),
            position: (0, 0),
            size: (0, 0),
            icons: WindowIcons {
                large: None,
                small: None,
            },
        }
    }

    /// The message window of a tray application: created, never shown.
    ///
    /// Note that this is a plain top-level window rather than a message-only
    /// window (`HWND_MESSAGE`). Message-only windows do not receive broadcast
    /// messages, and the `TaskbarCreated` broadcast is what tells us to
    /// restore the tray icon after Explorer restarts.
    pub const fn hidden(class: &'a str, title: &'a str) -> Self {
        Self::new(class, title)
    }

    pub const fn with_icons(mut self, icons: WindowIcons) -> Self {
        self.icons = icons;
        self
    }

    pub const fn with_style(mut self, style: WINDOW_STYLE, ex_style: WINDOW_EX_STYLE) -> Self {
        self.style = style;
        self.ex_style = ex_style;
        self
    }

    pub const fn with_bounds(mut self, position: (i32, i32), size: (i32, i32)) -> Self {
        self.position = position;
        self.size = size;
        self
    }
}

/// Creates a window and hands ownership of `handler` to it; the handler is
/// dropped when the window is destroyed.
pub fn create(spec: &WindowSpec<'_>, handler: Box<dyn WindowHandler>) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let class = HSTRING::from(spec.class);

    let window_class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        hIcon: spec.icons.large.unwrap_or_default(),
        hIconSm: spec.icons.small.unwrap_or_default(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        hbrBackground: HBRUSH::default(),
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };

    // A zero return means failure; ignore "already registered" so a class can
    // back more than one window.
    if unsafe { RegisterClassExW(&window_class) } == 0 {
        let error = Error::from_thread();
        if error.code() != HRESULT::from_win32(ERROR_CLASS_ALREADY_EXISTS.0) {
            return Err(error);
        }
    }

    // `Box<dyn Trait>` is a fat pointer, so box it once more to get something
    // that fits in the single pointer-sized `GWLP_USERDATA` slot.
    let handler = Box::into_raw(Box::new(handler));

    let hwnd = unsafe {
        CreateWindowExW(
            spec.ex_style,
            PCWSTR(class.as_ptr()),
            &HSTRING::from(spec.title),
            spec.style,
            spec.position.0,
            spec.position.1,
            spec.size.0,
            spec.size.1,
            None,
            None,
            Some(instance.into()),
            Some(handler as *const c_void),
        )
    };

    match hwnd {
        Ok(hwnd) => Ok(hwnd),
        Err(error) => {
            // The window never came to life, so `WM_NCDESTROY` will not run:
            // reclaim the handler here to avoid leaking it.
            drop(unsafe { Box::from_raw(handler) });
            Err(error)
        }
    }
}

/// Standard message pump. Returns once a window calls [`quit`].
pub fn run_message_loop() -> Result<()> {
    let mut msg = MSG::default();
    loop {
        match unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 {
            0 => return Ok(()),                     // WM_QUIT
            -1 => return Err(Error::from_thread()), // invalid window / bad params
            _ => unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
    }
}

/// Convenience for handlers: end the message loop.
pub fn quit(exit_code: i32) {
    unsafe { PostQuitMessage(exit_code) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = lparam.0 as *const CREATESTRUCTW;
            let handler = (*create).lpCreateParams as isize;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, handler);
        }

        let handler = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Box<dyn WindowHandler>;

        // Messages sent before WM_NCCREATE (WM_GETMINMAXINFO) have no handler yet.
        let result = if handler.is_null() {
            None
        } else {
            (*handler).handle_message(hwnd, msg, wparam, lparam)
        };

        // WM_NCDESTROY is the last message the window ever sees.
        if msg == WM_NCDESTROY && !handler.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(handler));
        }

        result.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wparam, lparam))
    }
}
