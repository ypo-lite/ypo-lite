//! Y-Coustic - volume control by hotkey, mouse and overlay.
//!
//! Phase 1: hotkeys step and mute the master volume. The HUD (1.6) and the
//! rest of the tray menu (1.7) follow, see `docs/design/implementation-plan.md`.

#![windows_subsystem = "windows"]

mod audio;
mod hud;
mod mixer;
mod mouse;
mod panel;

use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, IDI_ERROR, KillTimer, LoadIconW, SetTimer, WM_HOTKEY, WM_TIMER,
};

use y_core::app::TrayApp;
use y_core::hotkey::{Fallback, Hotkey, HotkeyManager};
use y_core::ids::{IDH_FIRST, IDM_FIRST, TRAY_ICON_ID, WM_TRAY_CALLBACK};
use y_core::memory;
use y_core::monitor;
use y_core::prelude::*;
use y_core::resources::{IconSize, app_icon_or_stock, load_icon, product_version};
use y_core::shell;
use y_core::startup;
use y_core::tray::{Notification, Notify, TrayEvent};
use y_core::window;

use y_coustic::appearance::{self, Appearance, Inputs};
use y_coustic::config::{self, Binding, LoadError, Settings, TrayAction, WatchMode};
use y_coustic::input;
use y_coustic::ipc;
use y_coustic::model::{Action, Change, Kind, Target};

use crate::audio::watch::{WM_DEFAULT_DEVICE_CHANGED, WM_ENDPOINT_CHANGED, Watchers};
use crate::audio::{Flow, Microphones};
use crate::hud::{Hud, Reading, Request as HudRequest, WM_TRAY_WHEEL, WheelHook, take_tray_wheel};
use crate::mixer::RowId;
use crate::mouse::{MouseHook, WM_MOUSE_ACTION, WM_MOUSE_WHEEL};
use crate::panel::{OpenWith, Panel, Request};

const APP: AppInfo = AppInfo::new("Y-Coustic", ipc::CLASS).single_instance();

const IDM_RELOAD: u32 = IDM_FIRST;
const IDM_OPEN_CONFIG: u32 = IDM_FIRST + 1;
const IDM_STARTUP: u32 = IDM_FIRST + 2;
const IDM_ABOUT: u32 = IDM_FIRST + 3;
const IDM_EXIT: u32 = IDM_FIRST + 4;
const IDM_REPLACE_VOLUME: u32 = IDM_FIRST + 5;
const IDM_SETTINGS: u32 = IDM_FIRST + 6;

/// The `Run` value name of "Start with Windows".
const STARTUP_NAME: &str = ipc::STARTUP_NAME;

/// The resource id of the tray icon shown while the microphone is muted.
const IDI_MIC_MUTED: u16 = 102;

/// How many warnings a balloon shows before pointing at the file.
const MAX_REPORTED: usize = 6;

/// `wParam` of the `WM_TIMER` that shows the HUD once the pointer has rested on
/// the tray icon for `hover-delay`.
const HOVER_TIMER_ID: usize = 0xC0_0003;

/// One click on the tray icon can reach us twice (`NIN_SELECT` and
/// `WM_LBUTTONUP`, which the shell sends for an icon in the overflow flyout
/// whatever version was negotiated). Two toggles in a row would open the panel
/// and close it again as the button comes up, so a click this soon after the
/// last is the same click.
const TRAY_DEBOUNCE: Duration = Duration::from_millis(500);

/// With a `double-click` configured, two click messages this close are one
/// physical click delivered twice (see [`TRAY_DEBOUNCE`]), not a double click.
const SAME_CLICK: Duration = Duration::from_millis(100);

/// `wParam` of the `WM_TIMER` that ends the wait for a second click: the single
/// click it was holding back then acts.
const CLICK_TIMER_ID: usize = 0xC0_0004;

struct YCoustic {
    tray: Option<TrayIcon>,
    /// Dropped and rebuilt on every load, which unregisters what it held.
    hotkeys: Option<HotkeyManager>,
    /// Learns of changes made outside y-coustic; rebuilt on every load.
    watchers: Option<Watchers>,
    settings: Option<Settings>,
    /// What is registered, in registration order: a hotkey id is an index here
    /// plus `IDH_FIRST`. The expansion of the configuration's keys and layers.
    bindings: Vec<Binding>,
    hud: Option<Hud>,
    /// The mouse hook, installed only while `<mouse>` has bindings.
    mouse: Option<MouseHook>,
    /// When the tray icon last toggled the panel: see [`TRAY_DEBOUNCE`].
    last_tray_click: Option<Instant>,
    /// A single click is waiting for a possible second one (`double-click` set).
    click_pending: bool,
    /// The wheel hook scoped to the tray icon, while the pointer is on it.
    tray_wheel: Option<WheelHook>,
    /// The pending click closed the mixer by taking its focus: it must not
    /// reopen it when its turn comes, a double-click time later.
    click_closed_mixer: bool,
    /// When a double click last acted, so its trailing messages do not act again.
    last_double_click: Option<Instant>,
    /// The panel; created with the window, open only on demand.
    panel: Option<Panel>,
    /// What the tray icon currently shows.
    appearance: Appearance,
    /// [`ipc::reload_message`]'s id, looked up once: the settings app sends it
    /// after a save.
    reload_message: u32,
}

impl YCoustic {
    fn new() -> Self {
        Self {
            tray: None,
            hotkeys: None,
            watchers: None,
            settings: None,
            bindings: Vec::new(),
            hud: None,
            mouse: None,
            panel: None,
            last_tray_click: None,
            click_pending: false,
            tray_wheel: None,
            click_closed_mixer: false,
            last_double_click: None,
            appearance: Appearance::Normal,
            reload_message: ipc::reload_message(),
        }
    }

    /// Reads the configuration and registers its hotkeys, replacing what was
    /// registered before. A missing file is not an error: the shipped default
    /// applies, so a fresh install works with nothing to copy.
    fn load(&mut self, hwnd: HWND) {
        // First: the old registrations must go or the new ones collide.
        // The panel was opened on the old settings.
        if let Some(panel) = self.panel.as_mut() {
            panel.close();
        }
        self.hotkeys = None;
        self.mouse = None;
        self.watchers = None;
        self.settings = None;
        self.bindings.clear();

        let settings = match Settings::load() {
            Ok(settings) => settings,
            Err(LoadError::Missing(path)) => Settings::parse(path, config::DEFAULT_XML)
                .expect("the shipped default configuration is well-formed"),
            Err(error) => {
                self.notify("Configuration", &error.to_string(), Notify::Error);
                return;
            }
        };

        let mut manager = HotkeyManager::new(hwnd);
        let mut refused = Vec::new();
        let bindings = settings.bindings();
        for (index, binding) in bindings.iter().enumerate() {
            let hotkey = repeat_for(binding.action, binding.hotkey);
            // A media key goes through the keyboard hook only: it is the one
            // route that behaves the same on every keyboard, and it swallows
            // the key before Windows shows its flyout. With no media binding
            // there is no hook at all.
            let fallback = if binding.hook_only {
                Fallback::HookOnly
            } else {
                Fallback::None
            };
            if manager
                .register(IDH_FIRST + index as u32, hotkey, fallback)
                .is_err()
            {
                refused.push(binding.hotkey.to_string());
            }
        }

        self.report(&settings, &refused);
        if !settings.mouse.is_empty() {
            match MouseHook::install(hwnd, &settings.mouse) {
                Ok(hook) => self.mouse = Some(hook),
                Err(error) => {
                    let message = format!("The mouse bindings are off: {error}");
                    self.notify("Mouse", &message, Notify::Error);
                }
            }
        }
        // With the hover on, the HUD is the icon's tooltip and the shell's own
        // is off (it would otherwise open on top of it, and the shell only
        // reports the pointer entering and leaving an icon without one).
        if let Some(tray) = self.tray.as_mut() {
            let hover = settings.tray.hover_delay > 0 && settings.hud.interactive;
            let _ = tray.set_standard_tooltip(!hover);
        }
        self.watchers = Self::watch(hwnd, &settings);
        self.bindings = bindings;
        self.hotkeys = Some(manager);
        self.settings = Some(settings);
        self.refresh_tray_icon();

        // Loading allocates a document and a tree, all dropped by now.
        let _ = memory::trim_working_set();
    }

    /// The watchers the configuration asks for: the microphone unless `off`,
    /// the master only with `hud` (a master `tray` has no icon variant to keep
    /// true, so nothing is resident for it).
    fn watch(hwnd: HWND, settings: &Settings) -> Option<Watchers> {
        let mut flows = Vec::new();
        if settings.watch.master == WatchMode::Hud {
            flows.push(Flow::Render);
        }
        if settings.watch.microphone != WatchMode::Off {
            flows.push(Flow::Capture);
        }
        (!flows.is_empty()).then(|| Watchers::new(hwnd, &flows))
    }

    /// Tells the user what the configuration got wrong, once, in one balloon.
    fn report(&mut self, settings: &Settings, refused: &[String]) {
        let mut lines = Vec::new();
        if !refused.is_empty() {
            lines.push(format!(
                "Already taken by another application: {}",
                refused.join(", ")
            ));
        }
        lines.extend(settings.warnings.iter().take(MAX_REPORTED).cloned());
        if settings.warnings.len() > MAX_REPORTED {
            lines.push(format!(
                "...and {} more, see {}",
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
            let _ = tray.notify(&Notification::new(title, message).level(level));
        }
    }

    /// A hotkey fired: find what it was bound to and do it.
    fn on_hotkey(&mut self, id: u32) {
        // A reload may have shortened the list between the post and now.
        let Some(binding) = id
            .checked_sub(IDH_FIRST)
            .and_then(|index| self.bindings.get(index as usize))
            .copied()
        else {
            return;
        };
        self.run_binding(binding.target, binding.action);
        let _ = memory::trim_working_set();
    }

    /// What a binding does, from a key or a mouse button: the panel's toggle,
    /// the selected row's step or mute while the panel is open (the volume keys
    /// and layers then act on what the panel shows), else the binding's own
    /// target.
    fn run_binding(&mut self, target: Target, action: Action) {
        if action == Action::Mixer {
            self.toggle_panel(false);
            return;
        }
        if matches!(action, Action::VolumeUp | Action::VolumeDown | Action::Mute)
            && let Some(row) = self
                .panel
                .as_ref()
                .and_then(|panel| panel.selected())
                .cloned()
        {
            self.carry_out(vec![Request::Change {
                row,
                change: Change::Action(action),
            }]);
            return;
        }
        self.run(target, action);
    }

    /// Applies `action` to `target` and shows the outcome on the HUD: what the
    /// device reports afterwards, or that there is no such device. A hotkey
    /// that finds nothing still consumed its key, as any bound combination
    /// does.
    fn run(&mut self, target: Target, action: Action) {
        self.run_change(target, Change::Action(action));
    }

    /// [`run`](Self::run) for anything a binding can change, a wheel's
    /// notches included. False only when the target is an application with no
    /// audio and `change` is a wheel's: the caller then gives the wheel back
    /// to whoever was under the pointer, and nothing is shown.
    fn run_change(&mut self, target: Target, change: Change) -> bool {
        let Some(settings) = self.settings.as_ref() else {
            return true;
        };
        let step = settings.step as f32;
        let kind = target.kind();
        let reading = match target {
            Target::Master => Some(
                mixer::apply_to_master(change, step)
                    .map_or(Reading::missing(kind), |state| Reading::new(kind, state)),
            ),
            Target::Mic => Some(
                mixer::apply_to_microphone(change, step)
                    .map_or(Reading::missing(kind), |state| Reading::new(kind, state)),
            ),
            Target::Focused | Target::Pointed => mixer::apply_to_application(target, change, step)
                .map(|(row, app, state)| Reading::application(row, app, state)),
        };
        let Some(reading) = reading else {
            // No audio under that window. A key or a click says so; a wheel
            // that did nothing has no right to flash a bar on every notch.
            if matches!(change, Change::Scroll(_)) {
                return false;
            }
            if let Some(hud) = self.hud.as_mut() {
                hud.show(Reading::missing(Kind::App), &settings.hud);
            }
            return true;
        };
        if let Some(hud) = self.hud.as_mut() {
            hud.show(reading, &settings.hud);
        }
        if target == Target::Mic {
            self.refresh_tray_icon();
        }
        true
    }

    /// A wheel binding has delta waiting: the sum of the notches since the
    /// last turn through the message loop, as one proportional change.
    fn on_mouse_wheel(&mut self, index: usize) {
        let delta = mouse::take_wheel(index);
        let Some(binding) = mouse::binding(index) else {
            return;
        };
        if delta != 0 {
            let notches = input::notches(delta, binding.invert);
            if !self.run_change(binding.target, Change::Scroll(notches)) {
                mouse::reinject(binding.input, delta);
            }
        }
        let _ = memory::trim_working_set();
    }

    /// A button binding fired, on the release.
    fn on_mouse_action(&mut self, index: usize) {
        if let Some(binding) = mouse::binding(index)
            && let Some(action) = binding.action
        {
            self.run_binding(binding.target, action);
        }
        let _ = memory::trim_working_set();
    }

    /// A watched endpoint changed from outside (never from our own writes: the
    /// callback filters those). Does what `<watch>` says for that flow.
    fn on_endpoint_changed(&mut self, flow: Flow) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let (target, mode) = match flow {
            Flow::Render => (Target::Master, settings.watch.master),
            Flow::Capture => (Target::Mic, settings.watch.microphone),
        };
        match mode {
            WatchMode::Off => {}
            WatchMode::Tray => {
                if target == Target::Mic {
                    self.refresh_tray_icon();
                }
            }
            // Reads the device back and shows it; `Panel` changes nothing.
            WatchMode::Hud => self.run(target, Action::Mixer),
        }
        let _ = memory::trim_working_set();
    }

    /// The default device of `flow` changed: the watcher moves onto the new
    /// one, and the tray icon follows the new microphone's state. No HUD: a
    /// headset plugged in is not a volume change.
    fn on_default_device_changed(&mut self, flow: Flow) {
        if let Some(watchers) = self.watchers.as_mut() {
            watchers.rewatch(flow);
        }
        if flow == Flow::Capture {
            self.refresh_tray_icon();
        }
    }

    /// Puts on the tray icon what the `<tray icon>` rule decides from the
    /// devices (`appearance::decide`). With the default `fixed` rule nothing is
    /// read and the icon is never touched. Touches the icon only when the
    /// appearance changed. Outside changes (the Sound panel, a laptop key)
    /// arrive through the watchers.
    fn refresh_tray_icon(&mut self) {
        let Some(rule) = self.settings.as_ref().map(|settings| settings.tray.icon) else {
            return;
        };
        let inputs = Inputs {
            microphone: if rule.reads_microphone() {
                Microphones::resolve()
                    .and_then(|microphones| microphones.state())
                    .ok()
            } else {
                None
            },
        };
        let appearance = appearance::decide(rule, &inputs);
        if appearance == self.appearance {
            return;
        }
        self.appearance = appearance;

        let (icon, tooltip) = match appearance {
            Appearance::Normal => (app_icon_or_stock(IconSize::Small), APP.name),
            Appearance::MicMuted => (muted_icon(), "Y-Coustic: microphone muted"),
        };
        if let Some(tray) = self.tray.as_mut() {
            let _ = tray.set_icon(icon);
            let _ = tray.set_tooltip(tooltip);
        }
    }

    fn show_context_menu(&mut self, hwnd: HWND, position: POINT) {
        let Ok(mut menu) = PopupMenu::new() else {
            return;
        };
        let _ = menu.add_disabled_item(APP.name);
        let _ = menu.add_disabled_item(&format!("{} hotkeys", self.bindings.len()));
        let _ = menu.add_separator();

        // The same submenu as in every Ypo-Lite app.
        if let Ok(configuration) = PopupMenu::new() {
            let _ = configuration.add_item(IDM_SETTINGS, "Settings…");
            let _ = configuration.add_item(IDM_OPEN_CONFIG, &format!("Edit {}", config::FILE_NAME));
            let _ = configuration.add_item(IDM_RELOAD, "Reload configuration");
            let _ = configuration.add_separator();
            // Only when the configuration has volume keys to take over.
            if let Some(settings) = self.settings.as_ref()
                && settings.has_media_group()
            {
                let _ = configuration.add_check_item(
                    IDM_REPLACE_VOLUME,
                    "Replace the Windows volume indicator",
                    settings.replaces_windows_volume(),
                );
            }
            let _ = configuration.add_check_item(
                IDM_STARTUP,
                "Start with Windows",
                startup::is_enabled(STARTUP_NAME),
            );
            let _ = menu.add_submenu("Configuration", configuration);
        }
        let _ = menu.add_separator();
        let _ = menu.add_item(IDM_ABOUT, "About");
        let _ = menu.add_item(IDM_EXIT, "Exit");

        match menu.track(hwnd, position) {
            Some(IDM_RELOAD) => self.load(hwnd),
            Some(IDM_SETTINGS) => self.open_settings(),
            Some(IDM_OPEN_CONFIG) => self.open_config(),
            Some(IDM_REPLACE_VOLUME) => self.toggle_replace_volume(hwnd),
            Some(IDM_STARTUP) => self.toggle_startup(),
            Some(IDM_ABOUT) => self.show_about(),
            Some(IDM_EXIT) => window::quit(0),
            _ => {}
        }
        let _ = memory::trim_working_set();
    }

    /// Flips `plain` on the media keys between `windows` and `replace`, saves
    /// the configuration and registers again. The file is regenerated, so its
    /// comments are lost, as for any save by a program (see the configuration
    /// reference).
    fn toggle_replace_volume(&mut self, hwnd: HWND) {
        let Some(mut settings) = self.settings.clone() else {
            return;
        };
        settings.set_replace_windows_volume(!settings.replaces_windows_volume());

        if let Err(error) = settings.save() {
            let message = format!("Could not save {}: {error}", settings.path.display());
            self.notify("Configuration", &message, Notify::Error);
            return;
        }
        // Through the file, so what is registered is what is saved.
        self.load(hwnd);
    }

    /// Opens the configuration file, creating it from the shipped default
    /// first when there is none: the file a user wants to edit is the one
    /// that already says what the app is doing.
    /// `y-coustic-config.exe`, expected next to this executable: the two ship
    /// together. `None` when it is not there.
    fn settings_app(&self) -> Option<std::path::PathBuf> {
        std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name(ipc::CONFIG_EXE))
            .filter(|app| app.is_file())
    }

    /// Opens the settings app, which is single-instance: a second click brings the
    /// open window forward. Without it, falls back to the file itself and says
    /// why, so *Settings…* never silently does nothing.
    fn open_settings(&mut self) {
        let Some(app) = self.settings_app() else {
            let message = format!(
                "{} is not next to y-coustic: opening the file instead.",
                ipc::CONFIG_EXE
            );
            self.notify("Settings", &message, Notify::Info);
            self.open_config();
            return;
        };
        shell::open(&app);
    }

    fn open_config(&mut self) {
        let path = self
            .settings
            .as_ref()
            .map_or_else(config::path, |settings| settings.path.clone());

        if !path.is_file() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, config::DEFAULT_XML));
            if let Err(error) = created {
                let message = format!("Cannot create {}: {error}", path.display());
                self.notify("Configuration", &message, Notify::Error);
                return;
            }
        }
        shell::open(&path);
    }

    /// Flips the `Run` registration, telling the user when it fails.
    fn toggle_startup(&mut self) {
        let enable = !startup::is_enabled(STARTUP_NAME);
        if let Err(error) = startup::set_enabled(STARTUP_NAME, enable) {
            let message = format!("Could not change Start with Windows: {error}");
            self.notify(APP.name, &message, Notify::Error);
        }
    }

    /// The *About* balloon: version, what is loaded, what it costs.
    fn show_about(&mut self) {
        let version = product_version().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
        let title = format!("{} {version}", APP.name);
        let path = self
            .settings
            .as_ref()
            .map_or_else(config::path, |settings| settings.path.clone());
        let memory = memory::working_set_bytes().map_or_else(
            || "unknown".to_owned(),
            |bytes| format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)),
        );
        let body = format!(
            "Volume control by hotkey, mouse and overlay
Memory: {memory}
Config: {}",
            path.display()
        );
        if let Some(tray) = self.tray.as_mut() {
            let icon = app_icon_or_stock(IconSize::Large);
            let about = Notification::new(&title, &body)
                .level(Notify::User(icon))
                .large_icon();
            let _ = tray.notify(&about);
        }
    }

    /// The pointer used the HUD: the wheel over it, a click or a drag on its bar,
    /// a click on its glyph or anywhere else. What it asks for is done here (the
    /// HUD touches no audio) and what came of it is drawn in place.
    fn on_hud_message(&mut self, msg: u32, lparam: LPARAM) {
        let Some(request) = self
            .hud
            .as_mut()
            .and_then(|hud| hud.on_message(msg, lparam))
        else {
            return;
        };
        match request {
            HudRequest::OpenPanel => self.toggle_panel(false),
            HudRequest::Change { row, change } => {
                let Some(step) = self.settings.as_ref().map(|settings| settings.step as f32) else {
                    return;
                };
                if let Ok(state) = mixer::apply_row(&row, change, step) {
                    if let Some(hud) = self.hud.as_mut() {
                        hud.redraw(state);
                    }
                    if row == RowId::Mic {
                        self.refresh_tray_icon();
                    }
                }
            }
        }
    }

    /// The pointer came onto the tray icon: after `hover-delay` the HUD shows,
    /// against the icon. A one-shot timer that exists only while the pointer is
    /// on the icon. Not while the panel is open: it already says everything.
    fn on_tray_popup_open(&mut self, hwnd: HWND) {
        // The wheel over the icon, from the moment the pointer is on it: the
        // hook takes the wheel inside the icon's rectangle, kept from now.
        let wheel = self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.tray.wheel && settings.tray.hover_delay > 0);
        if wheel && let Some(rect) = self.tray.as_ref().and_then(|tray| tray.rect()) {
            self.tray_wheel = WheelHook::install_tray(hwnd, rect).ok();
        }

        let Some(delay) = self
            .settings
            .as_ref()
            // A HUD that cannot be used is not worth resting the pointer for.
            .filter(|settings| settings.hud.interactive)
            .map(|settings| settings.tray.hover_delay)
            .filter(|&delay| delay > 0)
        else {
            return;
        };
        if self.panel.as_ref().is_some_and(Panel::is_open) {
            return;
        }
        // Setting a timer on the same id replaces it.
        unsafe { SetTimer(Some(hwnd), HOVER_TIMER_ID, delay, None) };
    }

    /// The pointer left the icon before the delay: no HUD.
    fn on_tray_popup_close(&mut self, hwnd: HWND) {
        self.tray_wheel = None;
        let _ = unsafe { KillTimer(Some(hwnd), HOVER_TIMER_ID) };
        // A HUD shown for the pointer on the icon stayed while it was there; it
        // now goes after its usual timeout.
        if let Some(hud) = self.hud.as_mut() {
            hud.release();
        }
    }

    /// A left click on the icon. Without a `double-click` it acts at once, once
    /// per physical click; with one it waits the system's double-click time for
    /// a second click, and a second click acts as the double click.
    fn on_tray_select(&mut self, hwnd: HWND) {
        let Some(tray) = self.settings.as_ref().map(|settings| settings.tray) else {
            return;
        };
        let now = Instant::now();
        let recently = |at: Option<Instant>, within: Duration| {
            at.is_some_and(|at| now.duration_since(at) < within)
        };

        if tray.double_click == TrayAction::None {
            let repeat = recently(self.last_tray_click, TRAY_DEBOUNCE);
            self.last_tray_click = Some(now);
            if !repeat {
                let closed = self.mixer_just_closed();
                self.run_tray_action(hwnd, tray.click, closed);
            }
            return;
        }

        // The one click arriving as two messages, or the tail of a double click
        // that has already acted.
        let double_time = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
        if recently(self.last_tray_click, SAME_CLICK)
            || recently(self.last_double_click, double_time)
        {
            return;
        }
        self.last_tray_click = Some(now);

        if self.click_pending {
            self.click_pending = false;
            let _ = unsafe { KillTimer(Some(hwnd), CLICK_TIMER_ID) };
            self.last_double_click = Some(now);
            self.run_tray_action(hwnd, tray.double_click, false);
        } else {
            self.click_pending = true;
            self.click_closed_mixer = self.mixer_just_closed();
            unsafe {
                SetTimer(
                    Some(hwnd),
                    CLICK_TIMER_ID,
                    double_time.as_millis() as u32,
                    None,
                )
            };
        }
    }

    /// `WM_LBUTTONDBLCLK`: the shell's own word for a double click. Usually
    /// already acted on through the second `Select`.
    fn on_tray_double_click(&mut self, hwnd: HWND) {
        let Some(tray) = self.settings.as_ref().map(|settings| settings.tray) else {
            return;
        };
        if tray.double_click == TrayAction::None {
            return;
        }
        let double_time = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
        if self
            .last_double_click
            .is_some_and(|at| at.elapsed() < double_time)
        {
            return;
        }
        self.click_pending = false;
        let _ = unsafe { KillTimer(Some(hwnd), CLICK_TIMER_ID) };
        self.last_double_click = Some(Instant::now());
        self.run_tray_action(hwnd, tray.double_click, false);
    }

    /// The wait for a second click is over: the single click acts.
    fn on_click_timer(&mut self, hwnd: HWND) {
        let _ = unsafe { KillTimer(Some(hwnd), CLICK_TIMER_ID) };
        if !std::mem::take(&mut self.click_pending) {
            return;
        }
        let Some(action) = self.settings.as_ref().map(|settings| settings.tray.click) else {
            return;
        };
        let closed = self.click_closed_mixer;
        self.run_tray_action(hwnd, action, closed);
    }

    /// Whether the mixer closed a moment ago because this click took its focus:
    /// the click then only closes it, and must not open it again.
    fn mixer_just_closed(&self) -> bool {
        self.panel.as_ref().is_some_and(Panel::just_lost_focus)
    }

    fn run_tray_action(&mut self, hwnd: HWND, action: TrayAction, mixer_just_closed: bool) {
        match action {
            TrayAction::None => {}
            TrayAction::Mixer => {
                if !mixer_just_closed {
                    self.toggle_panel(true);
                }
            }
            TrayAction::Hud => self.on_hover_timer(hwnd),
            TrayAction::Mute => self.run(Target::Master, Action::Mute),
            TrayAction::MuteMic => self.run(Target::Mic, Action::Mute),
            TrayAction::VolumeUp => self.run(Target::Master, Action::VolumeUp),
            TrayAction::VolumeDown => self.run(Target::Master, Action::VolumeDown),
        }
    }

    /// The delay has passed with the pointer still on the icon: shows where the
    /// master volume stands, against the icon, changing nothing.
    fn on_hover_timer(&mut self, hwnd: HWND) {
        let _ = unsafe { KillTimer(Some(hwnd), HOVER_TIMER_ID) };
        self.show_master_at_icon(Change::Action(Action::Mixer));
    }

    /// The wheel over the tray icon has delta waiting: steps the master and
    /// shows it against the icon, held while the pointer stays.
    fn on_tray_wheel(&mut self) {
        let delta = take_tray_wheel();
        if delta != 0 {
            self.show_master_at_icon(Change::Scroll(input::notches(delta, false)));
        }
    }

    /// Applies `change` to the master (`Action::Mixer` changes nothing) and
    /// shows where it stands against the tray icon.
    fn show_master_at_icon(&mut self, change: Change) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let kind = Kind::Master;
        let reading = mixer::apply_to_master(change, settings.step as f32)
            .map_or(Reading::missing(kind), |state| Reading::new(kind, state));
        let anchor = self.tray.as_ref().and_then(|tray| tray.rect());
        if let Some(hud) = self.hud.as_mut() {
            match anchor {
                Some(rect) => hud.show_against(reading, &settings.hud, rect),
                // The shell does not say where an icon in the overflow is: the
                // HUD goes where the settings put it.
                None => hud.show(reading, &settings.hud),
            }
        }
        let _ = memory::trim_working_set();
    }

    /// Opens the panel, or closes it when it is open. What it selects first is
    /// the row of the application in front, else the one under the pointer,
    /// else the master: read **before** the panel takes the focus, which would
    /// change the answer.
    fn toggle_panel(&mut self, from_tray: bool) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        if panel.is_open() {
            panel.close();
            let _ = memory::trim_working_set();
            return;
        }

        // The tray click that has just taken the focus from the panel is the
        // click that closes it.
        if from_tray && panel.just_lost_focus() {
            return;
        }

        let select =
            mixer::row_of_window(Target::Focused).or_else(|| mixer::row_of_window(Target::Pointed));
        let with = OpenWith {
            config: &settings.mixer,
            tray: self.tray.as_ref().and_then(|tray| tray.rect()),
            hud_position: settings.hud.position,
            look: &settings.hud,
            rows: mixer::read_rows(settings.mixer.system_sounds),
            select,
        };
        // The panel says it all: a bar held for the pointer on the icon goes.
        if let Some(hud) = self.hud.as_mut() {
            hud.hide();
        }
        if panel.open(with).is_err() {
            self.notify("Mixer", "The mixer could not be opened", Notify::Error);
        }
    }

    /// Re-reads the rows into the open panel.
    fn refresh_panel(&mut self) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let rows = mixer::read_rows(settings.mixer.system_sounds);
        if let Some(panel) = self.panel.as_mut() {
            panel.update(rows);
        }
    }

    /// Does what the panel asked for, then shows it the result.
    fn carry_out(&mut self, requests: Vec<Request>) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let step = settings.step as f32;
        let mut microphone = false;
        for Request::Change { row, change } in &requests {
            microphone |= *row == RowId::Mic;
            // A row that has gone away since the last refresh just fails.
            let _ = mixer::apply_row(row, *change, step);
        }
        if !requests.is_empty() {
            self.refresh_panel();
        }
        if microphone {
            self.refresh_tray_icon();
        }
    }

    fn on_panel_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) {
        let Some(step) = self.settings.as_ref().map(|settings| settings.step as f32) else {
            return;
        };
        let Some(panel) = self.panel.as_mut() else {
            return;
        };
        let requests = panel.on_message(msg, wparam, lparam, step);
        let closed = !panel.is_open();
        self.carry_out(requests);
        if closed {
            // Closing drops the window and the rows: back to nothing resident.
            let _ = memory::trim_working_set();
        }
    }

    /// The panel's second: the rows are read again, unless a bar is being
    /// dragged, which they would move under the pointer.
    fn on_panel_timer(&mut self) {
        if self
            .panel
            .as_ref()
            .is_some_and(|panel| !panel.is_dragging())
        {
            self.refresh_panel();
        }
    }
}

/// `Hotkey::parse` adds `MOD_NOREPEAT`. Holding a volume key should keep
/// stepping, so a step action gets auto-repeat back; mute keeps the flag, or
/// holding it would flap the switch.
fn repeat_for(action: Action, hotkey: Hotkey) -> Hotkey {
    if action.notches().is_some() {
        Hotkey::new(
            HOT_KEY_MODIFIERS(hotkey.modifiers.0 & !MOD_NOREPEAT.0),
            hotkey.key,
        )
    } else {
        hotkey
    }
}

/// The tray icon while the microphone is muted: `rsc/y-coustic-mic-muted.ico`
/// when it is compiled in (resource 102), else a stock icon so nothing waits
/// on the art.
fn muted_icon() -> HICON {
    load_icon(IDI_MIC_MUTED, IconSize::Small).unwrap_or_else(|_| {
        unsafe { LoadIconW(None, IDI_ERROR) }.unwrap_or_else(|_| app_icon_or_stock(IconSize::Small))
    })
}

impl TrayApp for YCoustic {
    fn on_create(&mut self, hwnd: HWND) -> Result<()> {
        audio::init()?;
        self.hud = Some(Hud::new(hwnd));
        self.panel = Some(Panel::new(hwnd));

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
        match msg {
            WM_HOTKEY => {
                self.on_hotkey(wparam.0 as u32);
                Some(LRESULT(0))
            }
            WM_MOUSE_WHEEL => {
                self.on_mouse_wheel(wparam.0);
                Some(LRESULT(0))
            }
            WM_MOUSE_ACTION => {
                self.on_mouse_action(wparam.0);
                Some(LRESULT(0))
            }
            WM_ENDPOINT_CHANGED => {
                if let Some(flow) = Flow::from_wparam(wparam.0) {
                    self.on_endpoint_changed(flow);
                }
                Some(LRESULT(0))
            }
            WM_DEFAULT_DEVICE_CHANGED => {
                if let Some(flow) = Flow::from_wparam(wparam.0) {
                    self.on_default_device_changed(flow);
                }
                Some(LRESULT(0))
            }
            // Registered at run time, so a guard rather than a constant pattern; 0
            // means registration failed, and 0 is also `WM_NULL`.
            msg if msg != 0 && msg == self.reload_message => {
                self.load(hwnd);
                Some(LRESULT(0))
            }
            WM_TRAY_WHEEL => {
                self.on_tray_wheel();
                Some(LRESULT(0))
            }
            _ if hud::is_message(msg) => {
                self.on_hud_message(msg, lparam);
                Some(LRESULT(0))
            }
            _ if Panel::is_message(msg) => {
                self.on_panel_message(msg, wparam, lparam);
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == CLICK_TIMER_ID => {
                self.on_click_timer(hwnd);
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == HOVER_TIMER_ID => {
                self.on_hover_timer(hwnd);
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == panel::TIMER_ID => {
                self.on_panel_timer();
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == hud::FADE_TIMER_ID => {
                if let Some(hud) = self.hud.as_mut() {
                    hud.on_fade_timer();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == hud::TIMER_ID => {
                if let Some(hud) = self.hud.as_mut() {
                    hud.on_timer();
                }
                let _ = memory::trim_working_set();
                Some(LRESULT(0))
            }
            WM_TRAY_CALLBACK => {
                let (event, position) = TrayEvent::decode(wparam, lparam);
                match event {
                    TrayEvent::ContextMenu => self.show_context_menu(hwnd, position),
                    TrayEvent::Select => self.on_tray_select(hwnd),
                    TrayEvent::DoubleClick => self.on_tray_double_click(hwnd),
                    TrayEvent::MiddleClick => {
                        let action = self
                            .settings
                            .as_ref()
                            .map(|settings| settings.tray.middle_click);
                        if let Some(action) = action {
                            self.run_tray_action(hwnd, action, false);
                        }
                    }
                    TrayEvent::PopupOpen => self.on_tray_popup_open(hwnd),
                    TrayEvent::PopupClose => self.on_tray_popup_close(hwnd),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn on_destroy(&mut self) {
        self.watchers = None; // unregisters before the audio objects go
        self.hotkeys = None; // gives the keys back
        self.mouse = None; // unhooks
        self.panel = None;
        self.hud = None;
        self.tray = None; // removes the icon from the notification area
    }
}

fn main() -> ! {
    y_core::app::start(&APP, || YCoustic::new().into_handler(&APP))
}
