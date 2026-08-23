//! Y-Snap — window management by hotkey, mouse gesture and overlay.
//!
//! The Rust rewrite of HulK. This is the message pump and the dispatch: it
//! reads `y-snap.xml`, registers what it says, and turns `WM_HOTKEY` into an
//! [`Action`] on a window. The actions themselves live in [`window`], the
//! choice of window in [`target`].
//!
//! Not here yet, in the order they are worth doing: the action overlay,
//! hiding windows behind a notification icon, and the virtual desktop
//! actions.

#![windows_subsystem = "windows"]

mod action;
mod actions;
mod config;
mod min_overlay;
mod mouse;
mod target;
mod window;

use windows::Win32::UI::{
    Input::KeyboardAndMouse::HOT_KEY_MODIFIERS,
    Shell::ShellExecuteW,
    WindowsAndMessaging::{SW_SHOWNORMAL, WM_HOTKEY},
};
use windows::core::HSTRING;

use y_core::{
    app::TrayApp,
    hotkey::{Fallback, HotkeyManager, absorb_modifier_tap},
    ids::{IDH_FIRST, IDM_FIRST, TRAY_ICON_ID, WM_TRAY_CALLBACK},
    memory, monitor,
    prelude::*,
    resources::{IconSize, app_icon_or_stock},
    tray::{Notify, TrayEvent},
    window as message_loop,
};

use crate::action::Action;
use crate::actions::Status;
use crate::config::Settings;

const APP: AppInfo = AppInfo::new("Y-Snap", "YpoLite.YSnap").single_instance();

const IDM_RELOAD: u32 = IDM_FIRST;
const IDM_OPEN_CONFIG: u32 = IDM_FIRST + 1;
const IDM_EXIT: u32 = IDM_FIRST + 2;

/// How many warnings a balloon shows before giving up and pointing at the
/// file. A notification the user cannot read is a notification wasted.
const MAX_REPORTED: usize = 6;

struct YSnap {
    tray: Option<TrayIcon>,
    /// Dropped and rebuilt on every reload, which unregisters everything the
    /// previous configuration had claimed.
    hotkeys: Option<HotkeyManager>,
    /// Same reason: uninstalling and reinstalling is how a mouse gesture
    /// change in the configuration takes effect.
    mouse_hook: Option<mouse::MouseHook>,
    /// The minimize-button popup. Rebuilt on every reload, since its tile
    /// list comes from the configuration too.
    min_overlay: Option<min_overlay::MinOverlay>,
    settings: Option<Settings>,
    /// Why there is no configuration, when there is none. Shown in the menu,
    /// because a balloon at start-up is gone by the time the user looks.
    problem: Option<String>,
}

impl YSnap {
    fn new() -> Self {
        Self {
            tray: None,
            hotkeys: None,
            mouse_hook: None,
            min_overlay: None,
            settings: None,
            problem: None,
        }
    }

    /// Reads the configuration and registers what it asks for, replacing
    /// whatever was registered before.
    fn load(&mut self, hwnd: HWND) {
        // Before anything else: the old registrations have to go, or the new
        // ones will collide with them and every hotkey will look taken.
        self.hotkeys = None;
        self.mouse_hook = None;
        self.min_overlay = None;
        self.settings = None;
        self.problem = None;

        let settings = match Settings::load() {
            Ok(settings) => settings,
            Err(error) => {
                let message = error.to_string();
                self.notify("Configuration", &message, Notify::Error);
                self.problem = Some(message);
                return;
            }
        };

        let mut manager = HotkeyManager::new(hwnd);
        let mut refused = Vec::new();

        for (index, binding) in settings.hotkeys.iter().enumerate() {
            let fallback = if binding.steal {
                Fallback::LowLevelHook
            } else {
                Fallback::None
            };

            if manager
                .register(IDH_FIRST + index as u32, binding.hotkey, fallback)
                .is_err()
            {
                refused.push(binding.hotkey.to_string());
            }
        }

        self.mouse_hook = match mouse::MouseHook::install(hwnd, settings.mouse) {
            Ok(hook) => Some(hook),
            Err(error) => {
                let message = format!("Could not install the mouse hook: {error}");
                self.notify(APP.name, &message, Notify::Error);
                None
            }
        };
        self.min_overlay = Some(min_overlay::MinOverlay::new(
            hwnd,
            settings.min_button.clone(),
        ));

        self.report(&settings, &refused);
        self.hotkeys = Some(manager);
        self.settings = Some(settings);

        // Loading allocates a document, a tree and a lot of small strings,
        // all dropped by now. Hand the pages back rather than sit on them.
        let _ = memory::trim_working_set();
    }

    /// Tells the user what the configuration got wrong, once, in one balloon.
    fn report(&mut self, settings: &Settings, refused: &[String]) {
        let mut lines = Vec::new();

        if !refused.is_empty() {
            lines.push(format!(
                "Already taken by another application: {}",
                refused.join(", ")
            ));
            // The fix is a configuration change, so name it here rather than
            // leave the user to guess why a key does nothing.
            lines.push("Add steal-from-shell=\"true\" to claim one anyway.".to_owned());
        }

        lines.extend(settings.warnings.iter().take(MAX_REPORTED).cloned());
        if settings.warnings.len() > MAX_REPORTED {
            lines.push(format!(
                "…and {} more, see {}",
                settings.warnings.len() - MAX_REPORTED,
                settings.path.display()
            ));
        }

        if !lines.is_empty() {
            self.notify("Configuration", &lines.join("\n"), Notify::Warning);
        }
    }

    fn notify(&mut self, title: &str, message: &str, level: Notify) {
        if let Some(tray) = self.tray.as_mut() {
            let _ = tray.notify(title, message, level);
        }
    }

    /// A hotkey fired: find what it was bound to and do it.
    fn on_hotkey(&mut self, hwnd: HWND, id: u32) {
        let Some(index) = id.checked_sub(IDH_FIRST) else {
            return;
        };
        let Some(binding) = self
            .settings
            .as_ref()
            .and_then(|settings| settings.hotkeys.get(index as usize))
            .copied()
        else {
            return;
        };

        match binding.action {
            Action::Quit => message_loop::quit(0),
            Action::Reload => self.load(hwnd),

            action => {
                let window = action
                    .needs_window()
                    .then(|| target::resolve(binding.target))
                    .flatten();
                self.perform(action, window);
            }
        }
    }

    /// Runs `action` on `target` and tells the user when it is not written
    /// yet — the one thing a hotkey and a minimize-button tile both need.
    fn perform(&mut self, action: Action, target: Option<HWND>) {
        match actions::dispatch(action, target) {
            // Nothing under the pointer worth acting on. Silence is the
            // right answer: the user pressed a key over the desktop, they do
            // not need to be told.
            Status::Done | Status::NoTarget => {}
            Status::Pending => {
                let message = format!("{action} is not implemented yet");
                self.notify(APP.name, &message, Notify::Info);
            }
        }
    }

    /// The window under a window's minimize button changed — show or hide
    /// the popup for it.
    fn on_min_hover(&mut self, target: HWND) {
        let Some(min_overlay) = self.min_overlay.as_mut() else {
            return;
        };

        if target.is_invalid() {
            min_overlay.hide();
        } else if let Some(settings) = &self.settings {
            let _ = min_overlay.show(target, &settings.overlay);
        }
    }

    /// A tile of the minimize-button popup was clicked.
    fn on_min_action(&mut self, index: usize) {
        let Some(min_overlay) = self.min_overlay.as_mut() else {
            return;
        };

        let action = min_overlay.action_at(index);
        min_overlay.hide();

        if let Some((action, target)) = action {
            self.perform(action, Some(target));
        }
    }

    fn show_menu(&mut self, hwnd: HWND, position: POINT) {
        let Ok(menu) = PopupMenu::new() else { return };

        let _ = menu.add_disabled_item(APP.name);
        let _ = menu.add_separator();

        match (&self.problem, &self.settings) {
            (Some(problem), _) => {
                let _ = menu.add_disabled_item(problem);
            }
            (None, Some(settings)) => {
                let summary = format!(
                    "{} hotkeys, {} overlay actions",
                    settings.hotkeys.len(),
                    settings.overlay.items.len()
                );
                let _ = menu.add_disabled_item(&summary);
            }
            (None, None) => {}
        }

        let _ = menu.add_item(IDM_RELOAD, "Reload configuration");
        let _ = menu.add_item(IDM_OPEN_CONFIG, "Open configuration");
        let _ = menu.add_separator();
        let _ = menu.add_item(IDM_EXIT, "Exit");

        match menu.track(hwnd, position) {
            Some(IDM_RELOAD) => self.load(hwnd),
            Some(IDM_OPEN_CONFIG) => self.open_config(),
            Some(IDM_EXIT) => message_loop::quit(0),
            _ => {}
        }

        let _ = memory::trim_working_set();
    }

    /// Opens the configuration in whatever the user has associated with
    /// `.xml`, which is the closest thing y-snap has to a settings dialog.
    fn open_config(&mut self) {
        let path = self
            .settings
            .as_ref()
            .map_or_else(config::path, |settings| settings.path.clone());

        if !path.is_file() {
            let message = format!("There is no file at {}", path.display());
            self.notify("Configuration", &message, Notify::Error);
            return;
        }

        let file = HSTRING::from(path.as_os_str());
        unsafe {
            ShellExecuteW(
                None,
                windows::core::w!("open"),
                &file,
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
    }
}

impl TrayApp for YSnap {
    fn on_create(&mut self, hwnd: HWND) -> Result<()> {
        self.tray = Some(TrayIcon::new(
            hwnd,
            TRAY_ICON_ID,
            WM_TRAY_CALLBACK,
            app_icon_or_stock(IconSize::Small),
            APP.name,
        )?);

        // After the tray icon exists, so a configuration error has somewhere
        // to be reported.
        self.load(hwnd);
        Ok(())
    }

    fn on_taskbar_created(&mut self) {
        if let Some(tray) = self.tray.as_mut() {
            let _ = tray.restore();
        }
    }

    fn on_activate(&mut self, hwnd: HWND) {
        // A second copy was started. y-snap has no window to bring forward,
        // so show the menu where the user's pointer is: it is the only thing
        // they could have wanted.
        if let Ok(position) = monitor::cursor_position() {
            self.show_menu(hwnd, position);
        }
    }

    fn on_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_HOTKEY => {
                self.on_hotkey(hwnd, wparam.0 as u32);
                Some(LRESULT(0))
            }

            WM_TRAY_CALLBACK => {
                let (event, position) = TrayEvent::decode(wparam, lparam);
                if event == TrayEvent::ContextMenu {
                    self.show_menu(hwnd, position);
                }
                Some(LRESULT(0))
            }

            mouse::WM_MIN_HOVER => {
                self.on_min_hover(HWND(wparam.0 as *mut _));
                Some(LRESULT(0))
            }

            // Test/debug — see `mouse::WM_ABSORB_MODIFIER`'s doc comment.
            // Delete this arm along with that constant once the comparison
            // it exists for has an answer.
            mouse::WM_ABSORB_MODIFIER => {
                absorb_modifier_tap(HOT_KEY_MODIFIERS(wparam.0 as u32));
                Some(LRESULT(0))
            }

            min_overlay::WM_MIN_ACTION => {
                self.on_min_action(wparam.0);
                Some(LRESULT(0))
            }

            _ => None,
        }
    }

    fn on_destroy(&mut self) {
        // Order matters: the hotkeys and the mouse hook both hold hooks that
        // must come down while the message loop is still running.
        self.hotkeys = None;
        self.mouse_hook = None;
        self.min_overlay = None;
        self.tray = None;
    }
}

fn main() -> ! {
    y_core::app::start(&APP, || YSnap::new().into_handler(&APP))
}
