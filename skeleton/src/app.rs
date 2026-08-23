//! Application state and message handling.
//!
//! This is the only module you normally need to touch when turning the
//! skeleton into a real application: add fields to [`App`], add arms to
//! [`App::handle_message`] and entries to [`App::show_context_menu`].

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_F12,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetCursorPos, RegisterWindowMessageW, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU,
    WM_CREATE, WM_DESTROY, WM_HOTKEY, WM_RBUTTONUP,
};
use windows::core::{Result, w};

use crate::hotkey::{Fallback, Hotkey, HotkeyManager};
use crate::ids::{IDH_SHOW_MENU, IDM_EXIT, TRAY_ICON_ID, WM_TRAY_CALLBACK};
use crate::memory;
use crate::menu::PopupMenu;
use crate::resources::{IconSize, load_app_icon};
use crate::tray::TrayIcon;
use crate::window::{self, WindowHandler, WindowIcons};

/// Window title and tray tooltip.
const APP_NAME: &str = "Win Rust Min";

/// Builds the window, its tray icon, and pumps messages until exit.
pub fn run() -> Result<()> {
    let icons = WindowIcons {
        large: load_app_icon(IconSize::Large).ok(),
        small: load_app_icon(IconSize::Small).ok(),
    };

    window::create(APP_NAME, icons, Box::new(App::new()))?;

    // Start-up is over and the process is about to sit idle: the pages touched
    // by CRT init and resource loading will not be read again. See `memory.rs`
    // for why this is done exactly once and never on a timer.
    let _ = memory::trim_working_set();

    window::run_message_loop()
}

/// Opens the tray menu from the keyboard. Replace with whatever the
/// application needs — or delete the `register` call in `on_create`.
const SHOW_MENU_HOTKEY: Hotkey = Hotkey::new(
    HOT_KEY_MODIFIERS(MOD_WIN.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0),
    VK_F12,
);

struct App {
    tray: Option<TrayIcon>,
    hotkeys: Option<HotkeyManager>,
    /// Broadcast sent by Explorer when the taskbar is (re)created.
    taskbar_created: u32,
}

impl App {
    fn new() -> Self {
        Self {
            tray: None,
            hotkeys: None,
            taskbar_created: 0,
        }
    }

    /// `WM_CREATE`: everything that needs a valid `HWND` happens here.
    fn on_create(&mut self, hwnd: HWND) -> Result<()> {
        self.taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };

        // A small icon: the notification area draws at SM_CXSMICON.
        let icon = load_app_icon(IconSize::Small)?;
        self.tray = Some(TrayIcon::new(
            hwnd,
            TRAY_ICON_ID,
            WM_TRAY_CALLBACK,
            icon,
            APP_NAME,
        )?);

        // A hotkey the user cannot reach is an annoyance, not a failure:
        // losing it must not take the application down with it.
        let mut hotkeys = HotkeyManager::new(hwnd);
        let _ = hotkeys.register(IDH_SHOW_MENU, SHOW_MENU_HOTKEY, Fallback::LowLevelHook);
        self.hotkeys = Some(hotkeys);

        Ok(())
    }

    /// A hotkey fired, whether through `RegisterHotKey` or the override hook.
    fn on_hotkey(&mut self, hwnd: HWND, id: u32) {
        if id == IDH_SHOW_MENU {
            let mut position = POINT::default();
            if unsafe { GetCursorPos(&mut position) }.is_ok() {
                self.show_context_menu(hwnd, position);
            }
        }
    }

    /// Tray callback, version 4 protocol: the cursor position is packed into
    /// `wParam`, the actual event id into the low word of `lParam`.
    fn on_tray_event(&mut self, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
        let event = (lparam.0 as u32) & 0xFFFF;
        let position = POINT {
            x: (wparam.0 & 0xFFFF) as i16 as i32,
            y: ((wparam.0 >> 16) & 0xFFFF) as i16 as i32,
        };

        // WM_CONTEXTMENU covers right click and the keyboard menu key;
        // WM_RBUTTONUP is kept for older shells.
        if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
            self.show_context_menu(hwnd, position);
        }
    }

    fn show_context_menu(&mut self, hwnd: HWND, position: POINT) {
        let Ok(menu) = PopupMenu::new() else { return };
        let _ = menu.add_item(IDM_EXIT, "Exit");

        if let Some(command) = menu.track(hwnd, position) {
            self.on_command(hwnd, command);
        }
    }

    /// Central dispatch for menu (and later, accelerator) commands.
    #[allow(clippy::single_match)] // kept as a match: this is the extension point
    fn on_command(&mut self, hwnd: HWND, command: u32) {
        match command {
            IDM_EXIT => {
                let _ = unsafe { DestroyWindow(hwnd) };
            }
            _ => {}
        }
    }

    /// `WM_DESTROY`: release shell resources before the message loop ends.
    fn on_destroy(&mut self) {
        self.tray = None; // removes the icon from the notification area
        self.hotkeys = None; // unregisters the hotkeys and drops the hook
        window::quit(0);
    }
}

impl WindowHandler for App {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        // Registered messages are not compile-time constants, so they cannot
        // appear in the `match` below.
        if msg != 0 && msg == self.taskbar_created {
            if let Some(tray) = self.tray.as_mut() {
                let _ = tray.restore();
            }
            return Some(LRESULT(0));
        }

        match msg {
            WM_CREATE => match self.on_create(hwnd) {
                Ok(()) => Some(LRESULT(0)),
                Err(_) => Some(LRESULT(-1)), // aborts CreateWindowExW
            },
            WM_TRAY_CALLBACK => {
                self.on_tray_event(hwnd, wparam, lparam);
                Some(LRESULT(0))
            }
            WM_COMMAND => {
                self.on_command(hwnd, (wparam.0 & 0xFFFF) as u32);
                Some(LRESULT(0))
            }
            WM_HOTKEY => {
                self.on_hotkey(hwnd, wparam.0 as u32);
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                let _ = unsafe { DestroyWindow(hwnd) };
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                self.on_destroy();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}
