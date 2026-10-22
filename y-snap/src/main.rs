//! Y-Snap — window management by hotkey, mouse gesture and overlay.
//!
//! The Rust rewrite of HulK. This is the message pump and the dispatch: it
//! reads `y-snap.xml`, registers what it says, and turns `WM_HOTKEY` into an
//! [`Action`] on a window. The actions themselves live in [`window`], the
//! choice of window in [`target`].
//!
//! Windows hidden by `iconize`/`traynize` are kept in [`hidden`]; this file
//! owns the tray icon, the menus that list them, and the *About* balloon
//! ([`about`]).

#![windows_subsystem = "windows"]

mod about;
mod actions;
mod hidden;
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
    WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL, WM_CLOSE, WM_HOTKEY, WM_TIMER},
};
use windows::core::HSTRING;

use y_core::{
    app::TrayApp,
    hotkey::{Fallback, HotkeyManager},
    icon::Bitmap,
    ids::{IDH_FIRST, IDM_FIRST, TRAY_ICON_ID, WM_TRAY_CALLBACK},
    memory,
    menu::ICON_SIZE,
    monitor,
    prelude::*,
    resources::{IconSize, app_icon_or_stock, product_version},
    tray::{Notification, Notify, TrayEvent},
};

use crate::action::Action;
use crate::actions::Status;
use crate::config::Settings;
use crate::hidden::{Hidden, Mode};

const APP: AppInfo = AppInfo::new("Y-Snap", ipc::CLASS).single_instance();

const IDM_RELOAD: u32 = IDM_FIRST;
const IDM_OPEN_CONFIG: u32 = IDM_FIRST + 1;
const IDM_EXIT: u32 = IDM_FIRST + 2;
const IDM_SETTINGS: u32 = IDM_FIRST + 3;
const IDM_ABOUT: u32 = IDM_FIRST + 4;
const IDM_RESTORE_ALL: u32 = IDM_FIRST + 5;
// The menu of a hidden window's own icon. The commands of the traynized
// entries start at `hidden::IDM_ENTRY_FIRST`, well clear of these.
const IDM_ENTRY_OPEN: u32 = IDM_FIRST + 6;
const IDM_ENTRY_CLOSE: u32 = IDM_FIRST + 7;

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
    /// The windows `iconize` and `traynize` took off the screen.
    hidden: Hidden,
    /// y-snap's own window: where the tray callbacks of hidden windows and
    /// the purge timer are addressed. Set in `on_create`.
    owner: HWND,
    /// What the last balloon was for, so that a click on it can do something.
    /// Only one balloon is live at a time.
    balloon: Balloon,
}

/// What a balloon invited the user to do by clicking it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Balloon {
    /// Nothing: a plain message.
    Plain,
    /// *About*: a click opens the settings app.
    About,
}

impl YSnap {
    fn new() -> Self {
        Self {
            hidden: Hidden::new(),
            owner: HWND::default(),
            balloon: Balloon::Plain,
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
        self.balloon = Balloon::Plain;
        if let Some(tray) = self.tray.as_mut() {
            let _ = tray.notify(&Notification::new(title, message).level(level));
        }
    }

    /// The *About* balloon: version, what is loaded, what it costs. Clicking
    /// it opens the settings app.
    fn show_about(&mut self) {
        let version = product_version().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        let title = format!("{} {version}", APP.name);

        let config_path = self
            .settings
            .as_ref()
            .map_or_else(config::path, |settings| settings.path.clone());
        let body = about::body(&about::Facts {
            loaded: self
                .settings
                .as_ref()
                .map(|settings| (settings.hotkeys.len(), settings.overlay.items.len())),
            config: &config_path,
            working_set: memory::working_set_bytes(),
            settings_app: self.settings_app().is_some(),
        });

        self.balloon = Balloon::About;
        if let Some(tray) = self.tray.as_mut() {
            let icon = app_icon_or_stock(IconSize::Large);
            let about = Notification::new(&title, &body)
                .level(Notify::User(icon))
                .large_icon();
            let _ = tray.notify(&about);
        }
    }

    /// Hides `target` behind an icon of its own or in the menu, and says why
    /// when it cannot.
    fn hide(&mut self, target: Option<HWND>, mode: Mode) {
        let Some(target) = target else { return };

        if let Err(refusal) = self.hidden.hide(self.owner, target, mode)
            && let Some(message) = refusal.message()
        {
            self.notify(APP.name, &message, Notify::Warning);
        }
    }

    /// Leaves through the window's normal shutdown (`WM_CLOSE` →
    /// `on_destroy`), so that hidden windows are brought back first.
    /// Quitting the message loop directly would skip `on_destroy` and strand
    /// them.
    fn quit(&self, hwnd: HWND) {
        let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
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
            Action::Quit => self.quit(hwnd),
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
        // The actions that act on the list of hidden windows, not on a window
        // alone, so `actions::dispatch` cannot do them.
        match action {
            Action::Iconize => return self.hide(target, Mode::Iconized),
            Action::Traynize => return self.hide(target, Mode::Traynized),
            Action::Uniconize => return self.hidden.restore_last(),
            Action::UniconizeAll => return self.hidden.restore_all(),
            _ => {}
        }

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
        // Declared before the menu so it is dropped after it: Windows does
        // not free an item's bitmap, and the menu reads it until it is gone.
        let mut icons: Vec<Bitmap> = Vec::new();
        let Ok(menu) = PopupMenu::new() else { return };

        let _ = menu.add_disabled_item(APP.name);
        let _ = menu.add_separator();

        self.add_hidden_section(&menu, &mut icons);

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
        let _ = menu.add_item(IDM_ABOUT, "About");
        let _ = menu.add_item(IDM_EXIT, "Exit");

        match menu.track(hwnd, position) {
            Some(IDM_SETTINGS) => self.open_settings(),
            Some(IDM_RELOAD) => self.load(hwnd),
            Some(IDM_OPEN_CONFIG) => self.open_config(),
            Some(IDM_ABOUT) => self.show_about(),
            Some(IDM_RESTORE_ALL) => self.hidden.restore_all(),
            Some(IDM_EXIT) => self.quit(hwnd),
            Some(id) => {
                if let Some(slot) = hidden::slot_of_menu_id(id) {
                    self.hidden.restore(slot);
                }
            }
            None => {}
        }

        let _ = memory::trim_working_set();
    }

    /// The traynized windows at the top of the menu, one item each with the
    /// window's own icon, and a way to bring everything back. Nothing at all
    /// when nothing is hidden.
    ///
    /// `icons` receives the item bitmaps, which the caller must keep alive
    /// until the menu is closed.
    fn add_hidden_section(&self, menu: &PopupMenu, icons: &mut Vec<Bitmap>) {
        let mut any = false;

        for entry in self.hidden.traynized() {
            if !any {
                let _ = menu.add_titled_separator("Hidden windows");
                any = true;
            }

            let id = hidden::menu_id(entry.slot);
            let _ = menu.add_item(id, &menu_label(&entry.title, &entry.app));

            if let Some(bitmap) = entry
                .icon
                .as_ref()
                .and_then(|icon| icon.to_bitmap(ICON_SIZE))
            {
                let _ = menu.set_item_icon(id, bitmap.handle());
                icons.push(bitmap);
            }
        }

        // Iconized windows are not listed (their icon is the way back), but
        // "restore all" is worth offering whatever is hidden.
        if !self.hidden.is_empty() {
            let _ = menu.add_item(IDM_RESTORE_ALL, "Restore all hidden windows");
            let _ = menu.add_separator();
        }
    }

    /// The right-click menu of an iconized window's own icon: the window's
    /// application as a heading, then *Open* and *Close*.
    fn show_entry_menu(&mut self, hwnd: HWND, slot: usize, position: POINT) {
        let Some(entry) = self.hidden.get(slot) else {
            return;
        };
        let window = entry.window;
        let heading = if entry.app.is_empty() {
            entry.title.clone()
        } else {
            entry.app.clone()
        };

        let Ok(menu) = PopupMenu::new() else { return };
        let _ = menu.add_titled_separator(&heading.replace('&', "&&"));
        let _ = menu.add_item(IDM_ENTRY_OPEN, "Open");
        let _ = menu.add_item(IDM_ENTRY_CLOSE, "Close");

        let chosen = menu.track(hwnd, position);

        // The menu runs its own message loop, and the window may have gone
        // (or the slot been reused) while it was open: act only on the same
        // window.
        if self.hidden.get(slot).map(|entry| entry.window) != Some(window) {
            return;
        }
        match chosen {
            Some(IDM_ENTRY_OPEN) => {
                self.hidden.restore(slot);
            }
            Some(IDM_ENTRY_CLOSE) => self.hidden.close(slot),
            _ => {}
        }
    }

    /// `y-snap-config.exe`, expected next to this executable — the two ship
    /// together. `None` when it is not there.
    fn settings_app(&self) -> Option<std::path::PathBuf> {
        std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name(ipc::CONFIG_EXE))
            .filter(|app| app.is_file())
    }

    /// Opens the settings app. Without it, falls back to the
    /// file itself, saying why, so "Settings…" never silently does nothing.
    /// The settings app is single-instance: a second click brings the open
    /// window forward rather than opening another.
    fn open_settings(&mut self) {
        let Some(app) = self.settings_app() else {
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

/// What a traynized window's menu item says: its title, cut to a reasonable
/// width, or the application's name for a window with none. `&` is doubled,
/// because a menu reads it as a mnemonic marker and a title such as
/// "Q&A — Notes" would lose a letter.
fn menu_label(title: &str, app: &str) -> String {
    let text = if title.is_empty() { app } else { title };
    hidden::shorten(text, hidden::MENU_TITLE_MAX).replace('&', "&&")
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
        self.owner = hwnd;
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
        // The icons of iconized windows went with the old taskbar too.
        self.hidden.restore_tray_icons();
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
                let id = TrayEvent::icon_id(lparam);

                if id == TRAY_ICON_ID {
                    match event {
                        TrayEvent::ContextMenu => self.show_menu(hwnd, position),
                        // The conventional default action of a tray icon.
                        TrayEvent::DoubleClick => self.open_settings(),
                        TrayEvent::BalloonClicked if self.balloon == Balloon::About => {
                            self.balloon = Balloon::Plain;
                            self.open_settings();
                        }
                        _ => {}
                    }
                } else if let Some(slot) = hidden::slot_of_tray_id(id) {
                    // The icon of a hidden window. A double click arrives
                    // after the first click already brought the window back
                    // and removed the icon, so it finds nothing: harmless.
                    match event {
                        TrayEvent::Select | TrayEvent::DoubleClick => {
                            self.hidden.restore(slot);
                        }
                        TrayEvent::ContextMenu => self.show_entry_menu(hwnd, slot, position),
                        _ => {}
                    }
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

            WM_TIMER if wparam.0 == hidden::PURGE_TIMER_ID => {
                self.hidden.purge();
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
        // First, while the message loop still runs: a hidden window has no
        // other way back once y-snap is gone.
        self.hidden.restore_all();

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
