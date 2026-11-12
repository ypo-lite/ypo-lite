//! y-coustic Config: a WinUI 3 settings app for `y-coustic.xml`, built on
//! `windows-reactor` and the `ui-kit` toolkit (**read `ui-kit/docs/
//! ui-toolkit.md` before adding or changing a screen**). The plan, and what is
//! built, is `docs/design/plan.md`.
//!
//! Module map:
//! - [`edit`]: what the simple screens change in a `Settings`, as data and one
//!   function, tested without a window.
//! - [`simple`]: the General, Tray icon and Mixer screens.
//! - [`keys_view`], [`key_edit`]: the Keys screen, and the dialog that chooses a key.
//! - [`mouse_view`]: the Mouse screen.
//! - [`hud_view`]: the HUD screen and its preview.
//! - [`colours`], [`colours_view`]: the Colours screen and what it edits.
//! - [`prefs`]: the screen that was open, in `y-coustic-config.ini` next to
//!   the XML, never in it.
//!
//! State lives in [`ConfigApp`]: the loaded `Settings` plus page-local UI
//! state. Every screen is a function of that state and a `Viewport`, and every
//! [`Msg`] is settled in [`ConfigApp::update`].
//!
//! **Nothing is lost silently.** `Settings` keeps comments, unknown attributes
//! and elements y-coustic cannot use (as comments), so a save rewrites the
//! layout but not the content. What y-coustic would warn about is listed in a
//! banner above every screen.
//!
//! **Undo.** One level: before a destructive edit the whole `Settings` is cloned
//! into [`ConfigApp::undo`], and the bar offers it back.

#![windows_subsystem = "windows"]

mod colours;
mod colours_view;
mod edit;
mod key_edit;
mod hud_view;
mod keys_view;
mod mouse_view;
mod prefs;
mod simple;

use ui::shell;
use ypo_ui as ui;

use std::path::PathBuf;

use windows_reactor::*;
use y_core::single_instance;
use y_coustic::config::{self, LoadError, Settings};
use y_coustic::ipc;

use prefs::Prefs;
use ui::prelude::*;

fn main() {
    // One editor at a time: two windows over one file would overwrite each
    // other's saves. A second launch (the tray's *Settings…* clicked again)
    // brings the open one forward instead. Held until the process exits.
    let _instance = match single_instance::SingleInstance::acquire(ipc::CONFIG_INSTANCE) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            single_instance::bring_to_front(ipc::CONFIG_TITLE);
            return;
        }
        // Couldn't even create the mutex: open anyway. Losing the one-window
        // guarantee beats not opening at all.
        Err(_) => None,
    };
    App::run_component::<ConfigApp>(()).unwrap();
}

/// What the page shows: one screen per section of the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Screen {
    #[default]
    General,
    Keys,
    Mouse,
    TrayIcon,
    Mixer,
    Hud,
    Colours,
}

impl Screen {
    /// The navigation pane's order.
    const ALL: [Screen; 7] = [
        Screen::General,
        Screen::Keys,
        Screen::Mouse,
        Screen::TrayIcon,
        Screen::Mixer,
        Screen::Hud,
        Screen::Colours,
    ];

    /// `NavigationViewItem`'s tag, round-tripped by `on_selected_tag_changed`,
    /// and what the preferences file says.
    pub fn tag(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Keys => "keys",
            Self::Mouse => "mouse",
            Self::TrayIcon => "tray",
            Self::Mixer => "mixer",
            Self::Hud => "hud",
            Self::Colours => "colours",
        }
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|screen| screen.tag() == tag.trim())
    }

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Keys => "Keys",
            Self::Mouse => "Mouse",
            Self::TrayIcon => "Tray icon",
            Self::Mixer => "Mixer",
            Self::Hud => "HUD",
            Self::Colours => "Colours",
        }
    }

    fn symbol(self) -> Symbol {
        match self {
            Self::General => Symbol::Setting,
            Self::Keys => Symbol::Keyboard,
            Self::Mouse => Symbol::TouchPointer,
            Self::TrayIcon => Symbol::Pin,
            Self::Mixer => Symbol::Audio,
            Self::Hud => Symbol::View,
            Self::Colours => Symbol::FontColor,
        }
    }
}

struct ConfigApp {
    /// `Err` only for a file that can't be read at all or isn't XML; a
    /// missing file is the shipped default, which Save will create.
    settings: Result<Settings, LoadError>,
    path: PathBuf,
    /// The file didn't exist when it was loaded.
    missing: bool,
    prefs: Prefs,
    screen: Screen,
    /// Whether anything has changed since the last load or save.
    dirty: bool,
    save_error: Option<String>,
    /// Whether the user closed the load-warnings banner. Reset by Reload.
    warnings_dismissed: bool,
    /// After the last successful save: whether a running y-coustic was asked to
    /// reload it (`Some(true)`), or none was running (`Some(false)`).
    applied: Option<bool>,
    /// Reload was pressed with unsaved changes: the bar asks first.
    confirming_reload: bool,
    undo: Option<Undo>,
    /// The dialog that chooses a key, when one is open.
    key_edit: key_edit::KeyEditUi,
    /// The HUD screen's preview: the theme it shows and the pictures drawn.
    hud_ui: hud_view::HudUi,
    /// The Colours screen's open picker.
    colours_ui: colours::ColoursUi,
    /// Renders what is copied while the window is alive: see
    /// `y_core::shell::ClipboardFlusher`. Held only for its `Drop`.
    _clipboard: Option<y_core::shell::ClipboardFlusher>,
    /// The window's client width in DIPs, from `on_window_size`.
    window_width: f64,
}

/// The state before the last destructive edit, and what to call it.
struct Undo {
    label: String,
    settings: Settings,
}

enum Msg {
    Navigated(Option<String>),
    WindowResized(WindowSize),
    WarningsDismissed,
    Save,
    Reload,
    ReloadConfirmed,
    ReloadCancelled,
    Undo,
    StartupToggled(bool),
    /// The dialog that chooses a key.
    Key(key_edit::KeyMsg),
    /// The HUD's font, from its text box.
    HudFont(String),
    /// The HUD preview in the dark theme, or the light one.
    HudDark(bool),
    /// The Colours screen: a swatch, the picker, a reset.
    Colour(colours::ColourMsg),
    OpenConfigFolder,
    EditConfigXml,
    /// A screen changed a value.
    Edit(edit::Edit),
    /// A control reported "nothing chosen": not an edit.
    Ignored,
}

/// Reads `path`. A missing file is not an error: the editor starts from the
/// shipped default, so the first Save writes a file with its explanations, as
/// y-coustic's own *Open configuration* does.
fn load(path: &std::path::Path) -> (Result<Settings, LoadError>, bool) {
    match Settings::load_from(path.to_owned()) {
        Err(LoadError::Missing(_)) => (
            Settings::parse(path.to_owned(), config::DEFAULT_XML),
            true,
        ),
        loaded => (loaded, false),
    }
}

impl Component for ConfigApp {
    type Input = ();
    type Message = Msg;

    fn create(_input: &(), _context: &ComponentContext<Self>) -> Self {
        let path = config::path();
        let prefs = Prefs::load(&prefs::path_for(&path));
        let (settings, missing) = load(&path);
        Self {
            settings,
            path,
            missing,
            screen: prefs.last_screen,
            prefs,
            dirty: false,
            save_error: None,
            warnings_dismissed: false,
            applied: None,
            confirming_reload: false,
            undo: None,
            key_edit: key_edit::KeyEditUi::default(),
            hud_ui: hud_view::HudUi::default(),
            colours_ui: colours::ColoursUi::default(),
            _clipboard: y_core::shell::ClipboardFlusher::start(),
            window_width: shell::INITIAL_WINDOW.0,
        }
    }

    fn update(&mut self, message: Msg, _context: &ComponentContext<Self>) {
        match message {
            Msg::WindowResized(size) => self.window_width = size.width,
            Msg::WarningsDismissed => self.warnings_dismissed = true,
            Msg::Navigated(tag) => {
                if let Some(screen) = tag.as_deref().and_then(Screen::from_tag) {
                    self.go(screen);
                }
            }

            Msg::Reload if self.dirty => self.confirming_reload = true,
            Msg::Reload | Msg::ReloadConfirmed => self.reload(),
            Msg::ReloadCancelled => self.confirming_reload = false,

            Msg::Save => self.save(),

            Msg::Edit(edit) => self.apply_edit(edit),
            Msg::Key(message) => self.edit_key(message),
            Msg::HudFont(text) => {
                if let Ok(settings) = &mut self.settings
                    && edit::set_font(settings, &text)
                {
                    self.dirty = true;
                    self.applied = None;
                }
            }
            Msg::HudDark(on) => self.hud_ui.dark = on,
            Msg::Colour(message) => self.edit_colour(message),
            Msg::Ignored => {}
            Msg::OpenConfigFolder => {
                if let Some(folder) = self.path.parent().filter(|folder| folder.is_dir()) {
                    y_core::shell::open(folder);
                }
            }
            Msg::EditConfigXml => {
                if self.path.is_file() {
                    y_core::shell::open(&self.path);
                }
            }

            Msg::Undo => {
                if let Some(undo) = self.undo.take() {
                    self.settings = Ok(undo.settings);
                    self.dirty = true;
                    self.key_edit = key_edit::KeyEditUi::default();
                    self.colours_ui = colours::ColoursUi::default();
                }
            }

            // Takes effect at once and is not part of y-coustic.xml; the bar
            // reads the registry again on the redraw, so a failure leaves the
            // switch where it was.
            Msg::StartupToggled(on) => {
                if let Err(error) = y_core::startup::set_enabled(ipc::STARTUP_NAME, on) {
                    y_core::log::line(format_args!("start with Windows: {error}"));
                }
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title(ipc::CONFIG_TITLE);
        let (width, height) = shell::INITIAL_WINDOW;
        let (min_width, min_height) = shell::MIN_WINDOW;
        context.window_visuals(
            WindowVisuals::new()
                .backdrop(WindowBackdrop::Mica)
                .client_size(width, height)
                .constraints(WindowConstraints {
                    min_width: Some(min_width),
                    min_height: Some(min_height),
                    ..Default::default()
                }),
        );
        context.on_window_size(context.callback(Msg::WindowResized));

        let layout = shell::layout(self.window_width);
        let settings = match &self.settings {
            Ok(settings) => settings,
            // Nothing to edit if the file can't be read: say why, plainly,
            // and keep Reload within reach for when it's been fixed.
            Err(error) => {
                let screen = page("Couldn't load y-coustic.xml", &error.to_string(), View::empty());
                let bar = self.bar("y-coustic.xml couldn't be loaded", false, None, context);
                let app: View = Grid::new()
                    .rows([GridLength::STAR, GridLength::Auto])
                    .keyed_children([
                        ("page", cell(shell::page_frame(screen, layout), 0, 0)),
                        ("bar", bar),
                    ]);
                return shell::window_frame(ipc::CONFIG_TITLE, app);
            }
        };

        let screen = self.screen_view(settings, layout.content, context);

        // Above every screen, not just one: what y-coustic would warn about can
        // be anywhere in the file.
        let banner = (!settings.warnings.is_empty() && !self.warnings_dismissed).then(|| {
            warnings_banner(
                &settings.warnings,
                context.callback(|()| Msg::WarningsDismissed),
            )
        });
        let screen: View = Stack::vertical(space::L)
            .maybe("warnings", banner)
            .child("screen", screen)
            .into();
        let page = shell::page_frame(screen, layout);

        let mut slots = vec![
            SlotView::collection(NavigationViewSlot::MenuItems, self.nav_items()),
            SlotView::new(NavigationViewSlot::Content, page),
        ];
        // The file path only fits a labelled pane; on the icon rail it's in the
        // status tooltip instead.
        if layout.pane_expanded {
            let footer = Stack::vertical(space::XXS)
                .child("caption", text::caption("Loaded from"))
                .child(
                    "path",
                    text::caption(self.path.display().to_string()).opacity(emphasis::SUBTLE),
                );
            slots.push(SlotView::new(
                NavigationViewSlot::PaneFooter,
                Border::new()
                    .padding(Thickness::uniform(space::M))
                    .content(footer),
            ));
        }

        let nav = shell::pane_mode(NavigationView::new(), layout)
            .on_selected_tag_changed(context.callback(Msg::Navigated))
            .grid_row(0)
            .slots(slots);

        let (status, healthy) = self.status(settings);
        let bar = self.bar(&status, healthy, Some(settings), context);

        // Nav takes the height the bar doesn't (`Star` over `Auto`).
        let app: View = Grid::new()
            .rows([GridLength::STAR, GridLength::Auto])
            .keyed_children([("nav", nav), ("bar", bar)]);
        // Always the same wrapper, the app always its first child: if the root
        // changed shape when a picker opened (later phases), the reactor would
        // rebuild the whole app under it and every scroll position would be lost.
        let window = Viewport::new(self.window_width);
        let mut layers = vec![("app", app)];
        layers.extend(
            key_edit::key_modal(settings, &self.key_edit, window, context)
                .map(|modal| ("modal", modal)),
        );
        let layers: View = Grid::new().keyed_children(layers);
        shell::window_frame(ipc::CONFIG_TITLE, layers)
    }
}

impl ConfigApp {
    /// The screen's page. The real screens arrive phase by phase; until then
    /// each says what it will edit.
    fn screen_view(
        &self,
        settings: &Settings,
        viewport: Viewport,
        context: &ViewContext<Self>,
    ) -> View {
        match self.screen {
            Screen::General => simple::general_view(settings, &self.path, viewport, context),
            Screen::TrayIcon => simple::tray_view(settings, viewport, context),
            Screen::Keys => keys_view::keys_view(settings, viewport, context),
            Screen::Mouse => mouse_view::mouse_view(settings, viewport, context),
            Screen::Hud => hud_view::hud_view(settings, &self.hud_ui, viewport, context),
            Screen::Colours => {
                colours_view::colours_view(settings, &self.colours_ui, &self.hud_ui, viewport, context)
            }
            Screen::Mixer => simple::mixer_view(settings, viewport, context),
        }
    }

    /// Writes an edit from a screen; only a value that changed makes the file dirty.
    fn apply_edit(&mut self, edit: edit::Edit) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        // What an edit removes is worth an Undo.
        let before = edit
            .undo_label(settings)
            .map(|label| (label, settings.clone()));
        if edit::apply(settings, edit) {
            self.dirty = true;
            self.applied = None;
            if let Some((label, before)) = before {
                self.undo = Some(Undo {
                    label: label.to_owned(),
                    settings: before,
                });
            }
        }
    }

    /// Settles a message of the key dialog; only its OK changes the settings.
    fn edit_key(&mut self, message: key_edit::KeyMsg) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        if key_edit::apply(settings, &mut self.key_edit, message) {
            self.dirty = true;
            self.applied = None;
        }
    }

    /// Settles a message of the Colours screen. Clearing a whole set is worth an
    /// Undo; one colour is one click to set again.
    fn edit_colour(&mut self, message: colours::ColourMsg) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        let before = matches!(message, colours::ColourMsg::ResetSet(_)).then(|| settings.clone());
        if colours::apply(settings, &mut self.colours_ui, message, self.hud_ui.dark) {
            self.dirty = true;
            self.applied = None;
            if let Some(before) = before {
                self.undo = Some(Undo {
                    label: "Reset a set of colours".to_owned(),
                    settings: before,
                });
            }
        }
    }

    /// Changes screen and remembers it for the next run.
    fn go(&mut self, screen: Screen) {
        self.key_edit = key_edit::KeyEditUi::default();
        self.colours_ui = colours::ColoursUi::default();
        self.screen = screen;
        self.prefs.last_screen = screen;
        self.prefs.save(&prefs::path_for(&self.path));
    }

    fn save(&mut self) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        match settings.save_to(&self.path) {
            Ok(()) => {
                self.dirty = false;
                self.missing = false;
                self.save_error = None;
                // y-coustic has no file watcher: tell it, so the change applies
                // now rather than at its next start.
                self.applied = Some(ipc::request_reload());
            }
            Err(error) => self.save_error = Some(error.to_string()),
        }
    }

    fn reload(&mut self) {
        let (settings, missing) = load(&self.path);
        self.settings = settings;
        self.missing = missing;
        self.dirty = false;
        self.save_error = None;
        self.warnings_dismissed = false;
        self.applied = None;
        self.confirming_reload = false;
        self.undo = None;
        self.key_edit = key_edit::KeyEditUi::default();
        self.colours_ui = colours::ColoursUi::default();
    }

    /// The navigation pane: one item per screen, in order.
    fn nav_items(&self) -> Vec<(String, View)> {
        Screen::ALL
            .into_iter()
            .map(|screen| {
                let item: View = NavigationViewItem::new()
                    .tag(screen.tag())
                    .is_selected(screen == self.screen)
                    .slots([
                        SlotView::new(
                            NavigationViewItemSlot::Icon,
                            SymbolIcon::new().symbol(screen.symbol()),
                        ),
                        SlotView::new(NavigationViewItemSlot::Content, text::body(screen.title())),
                    ]);
                (screen.tag().to_owned(), item)
            })
            .collect()
    }

    fn status(&self, settings: &Settings) -> (String, bool) {
        match &self.save_error {
            Some(error) => (format!("Save failed: {error}"), false),
            None if self.dirty => ("Unsaved changes".to_owned(), true),
            None => match self.applied {
                Some(true) => ("Saved and applied: y-coustic is reloading it".to_owned(), true),
                Some(false) => (
                    "Saved. y-coustic isn't running; it'll use these settings when it starts"
                        .to_owned(),
                    true,
                ),
                None if self.missing => ("No y-coustic.xml yet: Save creates it".to_owned(), true),
                None => (summary(settings), true),
            },
        }
    }

    /// The bar's small controls: "Start with Windows", which acts on the
    /// registry at once.
    fn bar_tools(&self, context: &ViewContext<Self>) -> View {
        shell::startup_toggle(
            y_core::startup::is_enabled(ipc::STARTUP_NAME),
            context.callback(Msg::StartupToggled),
        )
    }

    /// The bar along the bottom: status, an inline question or an Undo beside
    /// the buttons, Reload, Save.
    fn bar(
        &self,
        status: &str,
        healthy: bool,
        settings: Option<&Settings>,
        context: &ViewContext<Self>,
    ) -> View {
        let extra: Option<View> = if self.confirming_reload {
            Some(
                Stack::horizontal(space::S)
                    .child("ask", text::body("Discard unsaved changes?"))
                    .child(
                        "yes",
                        button(
                            "Discard",
                            true,
                            true,
                            context.callback(|()| Msg::ReloadConfirmed),
                        ),
                    )
                    .child(
                        "no",
                        button(
                            "Keep editing",
                            false,
                            true,
                            context.callback(|()| Msg::ReloadCancelled),
                        ),
                    )
                    .into(),
            )
        } else {
            self.undo.as_ref().map(|undo| {
                Stack::horizontal(space::S)
                    .child("what", text::body(undo.label.clone()))
                    .child(
                        "undo",
                        button("Undo", false, true, context.callback(|()| Msg::Undo)),
                    )
                    .into()
            })
        };

        let detail = format!("{status}\n{}", self.path.display());
        shell::action_bar(
            shell::BarStatus {
                text: status,
                detail: &detail,
                healthy,
                dirty: self.dirty && settings.is_some(),
                save_tooltip: "Rewrites y-coustic.xml from these settings. Comments, unknown attributes and entries \
                               y-coustic can't load are kept; only the layout is regenerated. The first save of a \
                               session keeps the old file as y-coustic.xml.bak.",
                extra,
                tools: Some(self.bar_tools(context)),
            },
            context.callback(|()| Msg::Reload),
            context.callback(|()| Msg::Save),
        )
    }
}

/// "3 hotkeys · 2 mouse bindings": the status line when nothing's pending.
fn summary(settings: &Settings) -> String {
    let keys = settings.bindings().len();
    let mouse = settings.mouse.len();
    format!(
        "{keys} hotkey{} · {mouse} mouse binding{}",
        if keys == 1 { "" } else { "s" },
        if mouse == 1 { "" } else { "s" },
    )
}

/// The load-warnings banner in y-coustic's words: a save keeps what the
/// warnings describe (as comments, or as the attributes written back), so it
/// says what is *wrong*, not what will be lost.
fn warnings_banner(warnings: &[String], on_close: Callback<()>) -> View {
    let count = warnings.len();
    shell::warnings_banner(
        format!(
            "y-coustic would report {count} problem{} in y-coustic.xml",
            if count == 1 { "" } else { "s" }
        ),
        "Entries y-coustic can't load are kept in the file as comments, and unknown attributes are written back, so \
         saving loses nothing, but those parts do nothing until they're fixed.",
        warnings,
        on_close,
    )
}

/// An app over `settings`, as `create` would make it but with no window and a
/// file path under the temp folder.
#[cfg(test)]
fn test_app(settings: Settings, path: PathBuf) -> ConfigApp {
    ConfigApp {
        settings: Ok(settings),
        path,
        missing: false,
        prefs: Prefs::default(),
        screen: Screen::General,
        dirty: false,
        save_error: None,
        warnings_dismissed: false,
        applied: None,
        confirming_reload: false,
        undo: None,
        key_edit: key_edit::KeyEditUi::default(),
        hud_ui: hud_view::HudUi::default(),
        colours_ui: colours::ColoursUi::default(),
        _clipboard: None,
        window_width: 1000.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the temp folder, empty.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "y-coustic-config-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn default_settings(path: &std::path::Path) -> Settings {
        Settings::parse(path.to_owned(), config::DEFAULT_XML).expect("the shipped file loads")
    }

    #[test]
    fn every_screen_has_a_tag_that_finds_it_again() {
        for screen in Screen::ALL {
            assert_eq!(Screen::from_tag(screen.tag()), Some(screen));
            assert!(!screen.title().is_empty());
        }
        assert_eq!(Screen::from_tag("nowhere"), None);
        assert_eq!(Screen::default(), Screen::General);
    }

    #[test]
    fn a_missing_file_starts_from_the_shipped_default() {
        let directory = scratch("missing");
        let (settings, missing) = load(&directory.join("y-coustic.xml"));
        assert!(missing);
        let settings = settings.expect("the default loads");
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert!(!settings.bindings().is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn saving_writes_the_file_and_a_second_save_writes_the_same_bytes() {
        let directory = scratch("save");
        let path = directory.join("y-coustic.xml");
        let mut app = test_app(default_settings(&path), path.clone());
        app.missing = true;
        app.dirty = true;

        app.save();
        assert!(!app.dirty && !app.missing);
        assert!(app.save_error.is_none());
        let first = std::fs::read(&path).unwrap();

        app.save();
        assert_eq!(std::fs::read(&path).unwrap(), first);

        // And what was written is what y-coustic reads.
        let again = Settings::load_from(path).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn reload_asks_first_when_there_are_unsaved_changes() {
        let directory = scratch("reload");
        let path = directory.join("y-coustic.xml");
        let mut app = test_app(default_settings(&path), path);

        // Nothing to lose: it reloads at once (the file is missing, so the
        // default comes back).
        app.reload();
        assert!(!app.confirming_reload);
        assert!(app.missing);

        app.dirty = true;
        app.confirming_reload = true;
        app.reload();
        assert!(!app.dirty && !app.confirming_reload);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn undo_gives_back_the_settings_it_kept() {
        let directory = scratch("undo");
        let path = directory.join("y-coustic.xml");
        let before = default_settings(&path);
        let mut app = test_app(before.clone(), path);
        let mut changed = before.clone();
        changed.step = before.step + 1;
        app.settings = Ok(changed);
        app.undo = Some(Undo {
            label: "Changed the step".to_owned(),
            settings: before.clone(),
        });

        if let Some(undo) = app.undo.take() {
            app.settings = Ok(undo.settings);
            app.dirty = true;
        }
        assert_eq!(app.settings.as_ref().unwrap().step, before.step);
        assert!(app.dirty && app.undo.is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_status_says_what_happened() {
        let directory = scratch("status");
        let path = directory.join("y-coustic.xml");
        let settings = default_settings(&path);
        let mut app = test_app(settings.clone(), path);

        assert!(app.status(&settings).0.contains("hotkey"));
        app.dirty = true;
        assert_eq!(app.status(&settings).0, "Unsaved changes");
        app.dirty = false;
        app.applied = Some(false);
        assert!(app.status(&settings).0.contains("isn't running"));
        app.applied = Some(true);
        assert!(app.status(&settings).0.contains("reloading"));
        app.save_error = Some("disk full".to_owned());
        let (text, healthy) = app.status(&settings);
        assert!(text.contains("disk full") && !healthy);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_edit_makes_the_file_dirty_only_when_it_changes_something() {
        let directory = scratch("edit");
        let path = directory.join("y-coustic.xml");
        let settings = default_settings(&path);
        let step = settings.step;
        let mut app = test_app(settings, path);

        // What a control reports when it is first set: the current value.
        app.apply_edit(edit::Edit::Step(step));
        assert!(!app.dirty);

        app.apply_edit(edit::Edit::Step(step + 1));
        assert!(app.dirty);
        assert_eq!(app.settings.as_ref().unwrap().step, step + 1);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_edited_file_is_saved_and_read_back() {
        let directory = scratch("edit-save");
        let path = directory.join("y-coustic.xml");
        let mut app = test_app(default_settings(&path), path.clone());
        app.apply_edit(edit::Edit::Step(11));
        app.apply_edit(edit::Edit::MaxRows(14));
        app.save();
        let saved = Settings::load_from(path).unwrap();
        assert_eq!((saved.step, saved.mixer.max_rows), (11, 14));
        assert!(saved.warnings.is_empty(), "{:?}", saved.warnings);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn deleting_offers_an_undo_that_gives_the_entry_back() {
        let directory = scratch("delete");
        let path = directory.join("y-coustic.xml");
        let settings = default_settings(&path);
        let hotkeys = settings.hotkeys.len();
        let mut app = test_app(settings, path);

        app.apply_edit(edit::Edit::Remove(y_coustic::edit::List::Hotkeys, 0));
        assert_eq!(app.settings.as_ref().unwrap().hotkeys.len(), hotkeys - 1);
        assert!(app.dirty);
        assert_eq!(app.undo.as_ref().map(|undo| undo.label.as_str()), Some("Deleted a hotkey"));

        // An edit with nothing to undo leaves the Undo that is there alone.
        app.apply_edit(edit::Edit::Step(app.settings.as_ref().unwrap().step + 1));
        assert!(app.undo.is_some());

        if let Some(undo) = app.undo.take() {
            app.settings = Ok(undo.settings);
        }
        assert_eq!(app.settings.as_ref().unwrap().hotkeys.len(), hotkeys);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_key_dialog_changes_the_file_only_through_ok() {
        use key_edit::{KeyMsg, KeyTarget};
        let directory = scratch("keydialog");
        let path = directory.join("y-coustic.xml");
        let mut app = test_app(default_settings(&path), path);
        let before = app.settings.as_ref().unwrap().hotkeys[0].hotkey;

        app.edit_key(KeyMsg::Open(KeyTarget::Hotkey(0)));
        app.edit_key(KeyMsg::Key(windows::Win32::UI::Input::KeyboardAndMouse::VK_F9));
        assert!(!app.dirty);
        app.edit_key(KeyMsg::Closed { accepted: false });
        assert!(!app.dirty);
        assert_eq!(app.settings.as_ref().unwrap().hotkeys[0].hotkey, before);

        app.edit_key(KeyMsg::Open(KeyTarget::Hotkey(0)));
        app.edit_key(KeyMsg::Key(windows::Win32::UI::Input::KeyboardAndMouse::VK_F9));
        app.edit_key(KeyMsg::Closed { accepted: true });
        assert!(app.dirty);
        assert_ne!(app.settings.as_ref().unwrap().hotkeys[0].hotkey, before);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn going_to_a_screen_remembers_it() {
        let directory = scratch("go");
        let path = directory.join("y-coustic.xml");
        let mut app = test_app(default_settings(&path), path.clone());
        app.go(Screen::Hud);
        assert_eq!(app.screen, Screen::Hud);
        assert_eq!(Prefs::load(&prefs::path_for(&path)).last_screen, Screen::Hud);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
