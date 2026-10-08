//! Y-Snap — window management by hotkey, mouse gesture and overlay.
//!
//! The Rust rewrite of HulK. This is the message pump and the dispatch: it
//! reads `y-snap.xml`, registers what it says, and turns `WM_HOTKEY` into an
//! [`Action`] on a window. The actions themselves live in [`window`], the
//! choice of window in [`target`].
//!
//! Not here yet: hiding windows behind a notification icon (iconize/traynize).

#![windows_subsystem = "windows"]

mod actions;
mod min_overlay;
mod mouse;
mod overlay;
mod target;
mod window;

// `action` and `config` live in this package's library target (`src/lib.rs`)
// instead, so `y-snap-config` can read and write `Settings` too — see there.
use y_snap::action;
use y_snap::config;
use y_snap::ipc;

use windows::Win32::UI::{
    Shell::ShellExecuteW,
    WindowsAndMessaging::{SW_SHOWNORMAL, WM_HOTKEY, WM_TIMER},
};
use windows::core::HSTRING;

use y_core::{
    app::TrayApp,
    hotkey::{Fallback, HotkeyManager},
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

const APP: AppInfo = AppInfo::new("Y-Snap", ipc::CLASS).single_instance();

const IDM_RELOAD: u32 = IDM_FIRST;
const IDM_OPEN_CONFIG: u32 = IDM_FIRST + 1;
const IDM_EXIT: u32 = IDM_FIRST + 2;
const IDM_SETTINGS: u32 = IDM_FIRST + 3;

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
    /// The keyboard-driven action overlay. Same reason: its grid is laid
    /// out from `settings.overlay` once, at construction.
    action_overlay: Option<overlay::ActionOverlay>,
    settings: Option<Settings>,
    /// Why there is no configuration, when there is none. Shown in the menu,
    /// because a balloon at start-up is gone by the time the user looks.
    problem: Option<String>,
    /// [`ipc::reload_message`]'s id, looked up once: `y-snap-config` sends
    /// it after saving, so a change applies without the tray menu.
    reload_message: u32,
}

impl YSnap {
    fn new() -> Self {
        Self {
            tray: None,
            hotkeys: None,
            mouse_hook: None,
            min_overlay: None,
            action_overlay: None,
            settings: None,
            problem: None,
            reload_message: ipc::reload_message(),
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
        self.action_overlay = None;
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

        self.mouse_hook =
            match mouse::MouseHook::install(hwnd, settings.mouse, &settings.min_button) {
                Ok(hook) => Some(hook),
                Err(error) => {
                    let message = format!("Could not install the mouse hook: {error}");
                    self.notify(APP.name, &message, Notify::Error);
                    None
                }
            };
        self.min_overlay = Some(min_overlay::MinOverlay::new(
            hwnd,
            settings.min_button.items.clone(),
        ));
        self.action_overlay = Some(overlay::ActionOverlay::new(hwnd, settings.overlay.clone()));

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

            // Needs the owner window and the overlay's own state, not just
            // a target — same reason Quit/Reload are handled here rather
            // than in `actions::dispatch`.
            Action::Overlay => {
                let Some(window) = target::resolve(binding.target) else {
                    return;
                };
                if let Some(action_overlay) = self.action_overlay.as_mut() {
                    let _ = action_overlay.show(window);
                }
            }

            action => {
                let window = action
                    .needs_window()
                    .then(|| target::resolve(binding.target))
                    .flatten();
                self.perform(action, window, binding.desktop);
            }
        }
    }

    /// Runs `action` on `target` and tells the user when it is not written
    /// yet — the one thing a hotkey and a minimize-button tile both need.
    /// `desktop` is the 1-based `desktop` attribute, `0` for a caller with
    /// no such attribute to give (only [`Action::DesktopMove`] reads it).
    fn perform(&mut self, action: Action, target: Option<HWND>, desktop: u32) {
        match actions::dispatch(action, target, desktop) {
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

    /// The window under a window's minimize button changed — hide whatever
    /// the popup was showing (for the button just left, or for none). It
    /// comes back, for whichever button is hovered now, only once
    /// [`Self::on_min_hover_dwell`] fires — see [`mouse::hover`]'s doc
    /// comment for why showing does not happen here.
    fn on_min_hover(&mut self) {
        if let Some(min_overlay) = self.min_overlay.as_mut() {
            min_overlay.hide();
        }
    }

    /// The pointer sat still over a minimize button for the configured
    /// delay — `target` is whichever window that button belongs to, read
    /// fresh by [`mouse::take_min_hover_target`] at the moment the dwell
    /// timer fired; invalid if the button was left in between.
    fn on_min_hover_dwell(&mut self, target: HWND) {
        if target.is_invalid() {
            return;
        }
        let Some(min_overlay) = self.min_overlay.as_mut() else {
            return;
        };

        // One bounded, one-time check against the real window — see
        // `mouse::confirm_min_button`'s own doc comment for why this is the
        // one place it is safe to ask, and why an unreadable cursor position
        // is treated the same as a rejection rather than shown anyway.
        let confirmed = monitor::cursor_position()
            .is_ok_and(|position| mouse::confirm_min_button(target, position));
        if !confirmed {
            return;
        }

        if let Some(settings) = &self.settings {
            let _ = min_overlay.show(target, &settings.overlay);
        }
    }

    /// A plain right-click landed on a minimize button — fire the
    /// configured default action directly, no popup involved.
    ///
    /// The hook only swallowed the click on the fast geometric guess (see
    /// `mouse::hover::on_right_down`/`on_right_up`) — there was nothing to
    /// synchronise there beyond that, so the one bounded, "could be wrong"
    /// check belongs entirely here, off the hook, the same as
    /// [`Self::on_min_hover_dwell`]'s.
    fn on_min_rightclick(&mut self, target: HWND) {
        if let Some(min_overlay) = self.min_overlay.as_mut() {
            min_overlay.hide();
        }

        let confirmed = monitor::cursor_position()
            .is_ok_and(|position| mouse::confirm_min_button(target, position));
        if !confirmed {
            return;
        }

        let Some(action) = self
            .settings
            .as_ref()
            .and_then(|settings| settings.min_button.right_click_action)
        else {
            return;
        };
        // `<minbutton>` tiles have no `desktop` attribute of their own —
        // see `config::QuickAction` — so `desktop-move` is a no-op from here.
        self.perform(action, Some(target), 0);
    }

    /// A tile of the minimize-button popup was clicked.
    fn on_min_action(&mut self, index: usize) {
        let Some(min_overlay) = self.min_overlay.as_mut() else {
            return;
        };

        let action = min_overlay.action_at(index);
        min_overlay.hide();

        if let Some((action, target)) = action {
            self.perform(action, Some(target), 0);
        }
    }

    /// A tile of the action overlay was activated, by click or by key.
    fn on_overlay_action(&mut self, index: usize) {
        let Some(action_overlay) = self.action_overlay.as_mut() else {
            return;
        };

        let action = action_overlay.action_at(index);
        action_overlay.close();

        if let Some((action, target, desktop)) = action {
            self.perform(action, Some(target), desktop);
        }

        // The overlay stays alive once created (`close` only hides it, for
        // instant reuse next time), so its layered-window bitmap and tile
        // state are still fully committed here, just idle — exactly the
        // burst-is-over case `y_core::memory`'s own doc comment calls out,
        // not the hot-loop one it warns against: the overlay now sits
        // closed for as long as the user is doing something else, almost
        // always far longer than the page fault it costs to bring back.
        let _ = memory::trim_working_set();
    }

    /// `Escape`, or a stale activation the overlay already closed itself
    /// for — [`overlay::ActionOverlay::close`] is safe to call either way.
    fn on_overlay_close(&mut self) {
        if let Some(action_overlay) = self.action_overlay.as_mut() {
            action_overlay.close();
        }
        // See `on_overlay_action`'s own comment on this call.
        let _ = memory::trim_working_set();
    }

    /// The pointer moved onto, off, or between tiles of the action overlay.
    /// `usize::MAX` (see [`overlay::WM_OVERLAY_HOVER`]) means off every tile.
    fn on_overlay_hover(&mut self, index: usize) {
        if let Some(action_overlay) = self.action_overlay.as_mut() {
            let index = (index != usize::MAX).then_some(index);
            let _ = action_overlay.set_hover(index);
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

        let _ = menu.add_item(IDM_SETTINGS, "Settings…");
        let _ = menu.add_item(IDM_OPEN_CONFIG, "Edit y-snap.xml");
        let _ = menu.add_item(IDM_RELOAD, "Reload configuration");
        let _ = menu.add_separator();
        let _ = menu.add_item(IDM_EXIT, "Exit");

        match menu.track(hwnd, position) {
            Some(IDM_SETTINGS) => self.open_settings(),
            Some(IDM_RELOAD) => self.load(hwnd),
            Some(IDM_OPEN_CONFIG) => self.open_config(),
            Some(IDM_EXIT) => message_loop::quit(0),
            _ => {}
        }

        let _ = memory::trim_working_set();
    }

    /// Opens the settings app (`y-snap-config.exe`, expected next to this
    /// executable — the two ship together). Without it, falls back to the
    /// file itself, saying why, so "Settings…" never silently does nothing.
    /// The settings app is single-instance: a second click brings the open
    /// window forward rather than opening another.
    fn open_settings(&mut self) {
        let app = std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name(ipc::CONFIG_EXE));
        let Some(app) = app.filter(|app| app.is_file()) else {
            let message = format!(
                "{} isn't installed next to y-snap — opening the file instead.",
                ipc::CONFIG_EXE
            );
            self.notify("Settings", &message, Notify::Info);
            self.open_config();
            return;
        };
        shell_open(&app);
    }

    /// Opens `y-snap.xml` in whatever edits `.xml` files.
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

        shell_open(&path);
    }
}

/// `ShellExecute`'s "open" on `path` — runs an executable, or opens a file
/// with its associated application.
fn shell_open(path: &std::path::Path) {
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
                match event {
                    TrayEvent::ContextMenu => self.show_menu(hwnd, position),
                    // The conventional default action of a tray icon.
                    TrayEvent::DoubleClick => self.open_settings(),
                    _ => {}
                }
                Some(LRESULT(0))
            }

            mouse::WM_MIN_HOVER => {
                self.on_min_hover();
                Some(LRESULT(0))
            }

            mouse::WM_MIN_RIGHTCLICK => {
                self.on_min_rightclick(HWND(wparam.0 as *mut _));
                Some(LRESULT(0))
            }

            mouse::WM_APPLY_GESTURE => {
                mouse::apply_pending_gesture();
                Some(LRESULT(0))
            }

            WM_TIMER if wparam.0 == mouse::REHOOK_TIMER_ID => {
                if let Some(hook) = self.mouse_hook.as_mut() {
                    hook.check_liveness();
                }
                Some(LRESULT(0))
            }

            WM_TIMER if wparam.0 == mouse::MIN_HOVER_TIMER_ID => {
                let target = mouse::take_min_hover_target();
                self.on_min_hover_dwell(target);
                Some(LRESULT(0))
            }

            min_overlay::WM_MIN_ACTION => {
                self.on_min_action(wparam.0);
                Some(LRESULT(0))
            }

            overlay::WM_OVERLAY_ACTION => {
                self.on_overlay_action(wparam.0);
                Some(LRESULT(0))
            }

            overlay::WM_OVERLAY_CLOSE => {
                self.on_overlay_close();
                Some(LRESULT(0))
            }

            overlay::WM_OVERLAY_HOVER => {
                self.on_overlay_hover(wparam.0);
                Some(LRESULT(0))
            }
            // Registered at run time, so a guard rather than a constant pattern;
            // 0 means registration failed, and 0 is also `WM_NULL`.
            msg if msg != 0 && msg == self.reload_message => {
                self.load(hwnd);
                Some(LRESULT(0))
            }

            _ => None,
        }
    }

    fn on_destroy(&mut self) {
        // Order matters: the hotkeys, the mouse hook and the action
        // overlay's own keyboard hook all hold hooks that must come down
        // while the message loop is still running. There is deliberately no
        // `virtual_desktop` teardown call here — see that module's own
        // "Never released" doc section for why releasing its COM interfaces
        // at shutdown is exactly what must not happen.
        self.hotkeys = None;
        self.mouse_hook = None;
        self.min_overlay = None;
        self.action_overlay = None;
        self.tray = None;
    }
}

fn main() -> ! {
    y_core::app::start(&APP, || YSnap::new().into_handler(&APP))
}
