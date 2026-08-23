//! Y-Term — placeholder wiring.
//!
//! The application does nothing yet: it exists to show the shape every
//! Ypo-Lite application takes on top of `y-core`, and to keep that shape
//! compiling. Replace the body of [`YTerm`] with the real thing.

#![windows_subsystem = "windows"]

use y_core::app::TrayApp;
use y_core::ids::{IDM_FIRST, TRAY_ICON_ID, WM_TRAY_CALLBACK};
use y_core::monitor;
use y_core::prelude::*;
use y_core::resources::{IconSize, app_icon_or_stock};
use y_core::tray::TrayEvent;
use y_core::window;

const APP: AppInfo = AppInfo::new("Y-Term", "YpoLite.YTerm").single_instance();

const IDM_EXIT: u32 = IDM_FIRST;

struct YTerm {
    tray: Option<TrayIcon>,
}

impl YTerm {
    fn new() -> Self {
        Self { tray: None }
    }

    fn show_context_menu(&mut self, hwnd: HWND, position: POINT) {
        let Ok(menu) = PopupMenu::new() else { return };
        let _ = menu.add_disabled_item(APP.name);
        let _ = menu.add_separator();
        let _ = menu.add_item(IDM_EXIT, "Exit");

        if menu.track(hwnd, position) == Some(IDM_EXIT) {
            window::quit(0);
        }
    }
}

impl TrayApp for YTerm {
    fn on_create(&mut self, hwnd: HWND) -> Result<()> {
        self.tray = Some(TrayIcon::new(
            hwnd,
            TRAY_ICON_ID,
            WM_TRAY_CALLBACK,
            app_icon_or_stock(IconSize::Small),
            APP.name,
        )?);
        Ok(())
    }

    fn on_taskbar_created(&mut self) {
        if let Some(tray) = self.tray.as_mut() {
            let _ = tray.restore();
        }
    }

    fn on_activate(&mut self, hwnd: HWND) {
        // A second instance started: show what the user came for. For now,
        // the menu, where the cursor is.
        if let Ok(position) = monitor::cursor_position() {
            self.show_context_menu(hwnd, position);
        }
    }

    fn on_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if msg == WM_TRAY_CALLBACK {
            let (event, position) = TrayEvent::decode(wparam, lparam);
            if event == TrayEvent::ContextMenu {
                self.show_context_menu(hwnd, position);
            }
            return Some(LRESULT(0));
        }
        None
    }

    fn on_destroy(&mut self) {
        self.tray = None; // removes the icon from the notification area
    }
}

fn main() -> ! {
    y_core::app::start(&APP, || YTerm::new().into_handler(&APP))
}
