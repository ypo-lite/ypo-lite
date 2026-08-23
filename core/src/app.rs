//! Start-up: single instance, message window, message loop, error reporting.
//!
//! Every Ypo-Lite application has the same `main`:
//!
//! ```no_run
//! # use y_core::prelude::*;
//! # struct App;
//! # impl WindowHandler for App {
//! #     fn handle_message(&mut self, _: HWND, _: u32, _: WPARAM, _: LPARAM) -> Option<LRESULT> { None }
//! # }
//! const APP: AppInfo = AppInfo::new("Y-Pop", "YpoLite.YPop").single_instance();
//!
//! fn main() -> ! {
//!     y_core::app::start(&APP, || Box::new(App))
//! }
//! ```

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, MB_ICONERROR, MB_OK, MessageBoxW, WM_CLOSE, WM_DESTROY,
};
use windows::core::{HSTRING, PCWSTR, Result};

use crate::ids::WM_ACTIVATE_INSTANCE;
use crate::memory;
use crate::resources::{IconSize, app_icon_or_stock};
use crate::single_instance::{self, SingleInstance};
use crate::window::{self, WindowHandler, WindowIcons, WindowSpec};

/// Identity of an application: what start-up needs to know about it.
#[derive(Clone, Copy)]
pub struct AppInfo<'a> {
    /// Display name — window title, tray tooltip, error box caption.
    pub name: &'a str,
    /// Window class of the main (hidden) window. Must be unique in the
    /// process and stable across versions: `"YpoLite.YSnap"`.
    pub class: &'a str,
    /// Name of the mutex guarding the single instance, or `None` to allow
    /// several. [`AppInfo::single_instance`] derives it from `class`.
    pub mutex: Option<&'a str>,
}

impl<'a> AppInfo<'a> {
    pub const fn new(name: &'a str, class: &'a str) -> Self {
        Self {
            name,
            class,
            mutex: None,
        }
    }

    /// Refuses to start a second copy; the first one is told to show itself
    /// (see [`Outcome::AlreadyRunning`] and [`crate::ids::WM_ACTIVATE_INSTANCE`]).
    pub const fn single_instance(mut self) -> Self {
        self.mutex = Some(self.class);
        self
    }
}

/// How [`run`] ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The message loop ran and exited normally.
    Ran,
    /// Another instance was already running; it was asked to come forward and
    /// this process did nothing.
    AlreadyRunning,
}

/// The whole of `main`: runs the application, reports a start-up failure in a
/// message box, and exits the process.
///
/// Without a console, a message box is the only way to report a failure that
/// happens before there is any UI.
pub fn start(info: &AppInfo<'_>, handler: impl FnOnce() -> Box<dyn WindowHandler>) -> ! {
    match run(info, handler) {
        Ok(_) => std::process::exit(0),
        Err(error) => {
            let message = HSTRING::from(format!("{error}"));
            let caption = HSTRING::from(info.name);
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(message.as_ptr()),
                    PCWSTR(caption.as_ptr()),
                    MB_OK | MB_ICONERROR,
                );
            }
            std::process::exit(1);
        }
    }
}

/// Start-up without the exit: takes the single instance, creates the hidden
/// message window, trims the working set and pumps messages until the
/// application quits.
///
/// The handler is built *after* the window class exists but is passed into
/// `CreateWindowExW`, so `WM_CREATE` is the first place with a valid `HWND` —
/// build tray icons and hotkeys there, not in the closure.
pub fn run(
    info: &AppInfo<'_>,
    handler: impl FnOnce() -> Box<dyn WindowHandler>,
) -> Result<Outcome> {
    // Held for the lifetime of the process: dropping it would let a second
    // instance in while this one is still running.
    let _instance = match info.mutex {
        Some(name) => match SingleInstance::acquire(name)? {
            Some(guard) => Some(guard),
            None => {
                single_instance::notify_running_instance(activation_message(info));
                return Ok(Outcome::AlreadyRunning);
            }
        },
        None => None,
    };

    let icons = WindowIcons {
        large: Some(app_icon_or_stock(IconSize::Large)),
        small: Some(app_icon_or_stock(IconSize::Small)),
    };
    let spec = WindowSpec::hidden(info.class, info.name).with_icons(icons);

    window::create(&spec, handler())?;

    // Start-up is over and the process is about to sit idle: the pages touched
    // by CRT init and resource loading will not be read again. See `memory.rs`
    // for why this is done exactly once and never on a timer.
    let _ = memory::trim_working_set();

    window::run_message_loop()?;
    Ok(Outcome::Ran)
}

/// The broadcast a second instance sends to the running one. Both processes
/// derive it from the same class name, so both get the same id.
pub fn activation_message(info: &AppInfo<'_>) -> u32 {
    single_instance::broadcast_message(&format!("{}.Activate", info.class))
}

/// The message plumbing shared by every tray application: `TaskbarCreated`,
/// the activation broadcast, `WM_CLOSE` and `WM_DESTROY`.
///
/// Implement [`TrayApp`] instead of [`WindowHandler`] and wrap the value in
/// [`TrayApp::into_handler`]; the messages an application actually cares
/// about arrive as methods, and the ones every application handles the same
/// way are handled here.
pub trait TrayApp {
    /// `WM_CREATE`: the first point with a valid `HWND`. Returning an error
    /// aborts window creation and fails start-up.
    fn on_create(&mut self, hwnd: HWND) -> Result<()>;

    /// Explorer restarted and the tray icon must be re-added, or a second
    /// instance asked this one to come forward. Both mean "the shell changed
    /// under us"; they are separated because the responses differ.
    fn on_taskbar_created(&mut self) {}

    /// A second instance started and handed over. Show whatever UI the user
    /// was expecting to see.
    fn on_activate(&mut self, hwnd: HWND) {
        let _ = hwnd;
    }

    /// Anything not covered above. Same contract as
    /// [`WindowHandler::handle_message`].
    fn on_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let _ = (hwnd, msg, wparam, lparam);
        None
    }

    /// `WM_DESTROY`: drop tray icons and hotkeys here. The message loop ends
    /// right after.
    fn on_destroy(&mut self) {}

    /// Wraps `self` for [`run`] / [`start`].
    fn into_handler(self, info: &AppInfo<'_>) -> Box<dyn WindowHandler>
    where
        Self: Sized + 'static,
    {
        Box::new(TrayHandler {
            app: self,
            taskbar_created: 0,
            activate: activation_message(info),
        })
    }
}

struct TrayHandler<T> {
    app: T,
    /// Broadcast sent by Explorer when the taskbar is (re)created.
    taskbar_created: u32,
    /// Broadcast sent by a second instance of this application.
    activate: u32,
}

impl<T: TrayApp> WindowHandler for TrayHandler<T> {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        // Registered messages are not compile-time constants, so they cannot
        // appear in a `match`.
        if msg != 0 && msg == self.taskbar_created {
            self.app.on_taskbar_created();
            return Some(LRESULT(0));
        }
        if (msg != 0 && msg == self.activate) || msg == WM_ACTIVATE_INSTANCE {
            self.app.on_activate(hwnd);
            return Some(LRESULT(0));
        }

        match msg {
            windows::Win32::UI::WindowsAndMessaging::WM_CREATE => {
                self.taskbar_created = single_instance::broadcast_message("TaskbarCreated");
                match self.app.on_create(hwnd) {
                    Ok(()) => Some(LRESULT(0)),
                    Err(_) => Some(LRESULT(-1)), // aborts CreateWindowExW
                }
            }
            WM_CLOSE => {
                let _ = unsafe { DestroyWindow(hwnd) };
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                self.app.on_destroy();
                window::quit(0);
                Some(LRESULT(0))
            }
            _ => self.app.on_message(hwnd, msg, wparam, lparam),
        }
    }
}
