//! y-pop Config — a WinUI 3 settings app for `y-pop.xml`, built on
//! `windows-reactor` and the `ui-kit` toolkit (**read `ui-kit/docs/
//! ui-toolkit.md` before adding or changing a screen**).
//!
//! Module map:
//! - [`roots`] — the tray menus (the items under `<y-pop>`) as plain
//!   functions on `Settings`: add, delete, find, label. No UI, tested.
//! - [`prefs`] — "show comments" and the last screen, in `y-pop-config.ini`
//!   next to `y-pop.xml`, never in it.
//! - [`screens`] — one function per screen.
//!
//! State lives in [`ConfigApp`]: the loaded `Settings` plus page-local UI
//! state. Every screen is a function of that state and a `Viewport`, and
//! every [`Msg`] is settled in [`ConfigApp::update`].
//!
//! **Nothing is lost silently.** `Settings` keeps comments, unknown
//! attributes and elements y-pop can't load (as disabled-item comments), so
//! a save rewrites the layout but not the content. What y-pop would warn
//! about is listed in a banner above every screen.
//!
//! **Undo.** One level: before a destructive edit the whole `Settings` is
//! cloned into [`ConfigApp::undo`], and the status bar offers it back.

#![windows_subsystem = "windows"]

mod prefs;
mod roots;
mod screens;
use ypo_ui as ui;
use ui::shell;

use std::path::{Path, PathBuf};

use windows_reactor::*;
use y_core::single_instance;
use y_pop::builtins::Builtins;
use y_pop::config::{self, LoadError};
use y_pop::ipc;
use y_pop::settings::{ItemId, Settings};

use prefs::{LastScreen, Prefs};
use ui::prelude::*;

fn main() {
    // One editor at a time: two windows over one file would overwrite each
    // other's saves. A second launch (the tray's `<settings/>` clicked again)
    // brings the open one forward instead. Held until the process exits.
    let _instance = match single_instance::SingleInstance::acquire(ipc::CONFIG_INSTANCE) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            single_instance::bring_to_front(ipc::CONFIG_TITLE);
            return;
        }
        // Couldn't even create the mutex: open anyway — losing the
        // one-window guarantee beats not opening at all.
        Err(_) => None,
    };
    App::run_component::<ConfigApp>(()).unwrap();
}

struct ConfigApp {
    /// `Err` only for a file that can't be read at all or isn't XML; a
    /// missing file is an empty `Settings` that Save will create.
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
    /// After the last successful save: whether a running y-pop was asked to
    /// reload it (`Some(true)`), or none was running (`Some(false)`).
    applied: Option<bool>,
    /// Reload was pressed with unsaved changes: the bar asks first.
    confirming_reload: bool,
    /// A tray menu is waiting for "Delete" to be confirmed.
    confirming_delete: Option<ItemId>,
    undo: Option<Undo>,
    /// The window's client width in DIPs, from `on_window_size`.
    window_width: f64,
}

/// The state before the last destructive edit, and what to call it.
struct Undo {
    label: String,
    settings: Settings,
}

/// What the page shows. A tray menu is addressed by id, never by index, so
/// reordering the menus doesn't change which screen is open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    General,
    Skin,
    Variables,
    Menu(ItemId),
}

/// What a navigation item stands for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Screen(Screen),
    /// "+ New tray menu": not a screen, an action that ends on one.
    NewMenu,
}

impl Target {
    /// `NavigationViewItem`'s tag, round-tripped by `on_selected_tag_changed`.
    fn tag(self) -> String {
        match self {
            Self::Screen(Screen::General) => "general".to_owned(),
            Self::Screen(Screen::Skin) => "skin".to_owned(),
            Self::Screen(Screen::Variables) => "variables".to_owned(),
            Self::Screen(Screen::Menu(id)) => format!("menu:{}", id.get()),
            Self::NewMenu => "new".to_owned(),
        }
    }

    /// The target a tag names, looking menus up in `settings` — a tag of a
    /// menu that no longer exists is `None`.
    fn from_tag(tag: &str, settings: &Settings) -> Option<Self> {
        match tag {
            "general" => Some(Self::Screen(Screen::General)),
            "skin" => Some(Self::Screen(Screen::Skin)),
            "variables" => Some(Self::Screen(Screen::Variables)),
            "new" => Some(Self::NewMenu),
            other => {
                let id = roots::menus(settings)
                    .map(|item| item.id)
                    .find(|id| format!("menu:{}", id.get()) == other)?;
                Some(Self::Screen(Screen::Menu(id)))
            }
        }
    }
}

enum Msg {
    Navigated(Option<String>),
    WindowResized(WindowSize),
    WarningsDismissed,
    Save,
    Reload,
    ReloadConfirmed,
    ReloadCancelled,
    DeleteAsked(ItemId),
    DeleteConfirmed(ItemId),
    DeleteCancelled,
    Undo,
}

/// Reads `path`, with y-pop's own builtins so the warnings are y-pop's.
fn load(path: &Path) -> (Result<Settings, LoadError>, bool) {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let mut builtins = Builtins::new(&exe_dir, path);

    match Settings::load_from(path.to_owned(), &mut builtins) {
        Err(LoadError::Missing(_)) => (Ok(Settings::empty()), true),
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

        let screen = match (&settings, prefs.last_screen) {
            (Ok(settings), LastScreen::Menu(position)) => {
                roots::at_position(settings, position).map_or(Screen::General, Screen::Menu)
            }
            (_, LastScreen::Skin) => Screen::Skin,
            (_, LastScreen::Variables) => Screen::Variables,
            _ => Screen::General,
        };

        Self {
            settings,
            path,
            missing,
            prefs,
            screen,
            dirty: false,
            save_error: None,
            warnings_dismissed: false,
            applied: None,
            confirming_reload: false,
            confirming_delete: None,
            undo: None,
            window_width: shell::INITIAL_WINDOW.0,
        }
    }

    fn update(&mut self, message: Msg, _context: &ComponentContext<Self>) {
        match message {
            Msg::WindowResized(size) => self.window_width = size.width,
            Msg::WarningsDismissed => self.warnings_dismissed = true,
            Msg::Navigated(tag) => {
                let target = match (&self.settings, tag) {
                    (Ok(settings), Some(tag)) => Target::from_tag(&tag, settings),
                    _ => None,
                };
                match target {
                    Some(Target::Screen(screen)) => self.go(screen),
                    Some(Target::NewMenu) => self.add_menu(),
                    None => {}
                }
            }

            Msg::Reload if self.dirty => self.confirming_reload = true,
            Msg::Reload | Msg::ReloadConfirmed => self.reload(),
            Msg::ReloadCancelled => self.confirming_reload = false,

            Msg::Save => self.save(),

            Msg::DeleteAsked(id) => self.confirming_delete = Some(id),
            Msg::DeleteCancelled => self.confirming_delete = None,
            Msg::DeleteConfirmed(id) => self.delete_menu(id),
            Msg::Undo => {
                if let Some(undo) = self.undo.take() {
                    self.settings = Ok(undo.settings);
                    self.dirty = true;
                }
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title(ipc::CONFIG_TITLE);
        let (width, height) = shell::INITIAL_WINDOW;
        let (min_width, min_height) = shell::MIN_WINDOW;
        context.window_visuals(
            WindowVisuals::new().backdrop(WindowBackdrop::Mica).client_size(width, height).constraints(
                WindowConstraints { min_width: Some(min_width), min_height: Some(min_height), ..Default::default() },
            ),
        );
        context.on_window_size(context.callback(Msg::WindowResized));

        let layout = shell::layout(self.window_width);
        let settings = match &self.settings {
            Ok(settings) => settings,
            // Nothing to edit if the file can't be read — say why, plainly,
            // and keep Reload within reach for when it's been fixed.
            Err(error) => {
                let screen = page("Couldn't load y-pop.xml", &error.to_string(), View::empty());
                let bar = self.bar("y-pop.xml couldn't be loaded", false, None, context);
                return Grid::new()
                    .rows([GridLength::STAR, GridLength::Auto])
                    .keyed_children([("page", cell(shell::page_frame(screen, layout), 0, 0)), ("bar", bar)]);
            }
        };

        let screen = match self.screen {
            Screen::General => screens::general(settings, &self.path),
            Screen::Skin => screens::skin(settings),
            Screen::Variables => screens::variables(settings),
            Screen::Menu(id) => screens::menu(settings, id, self.confirming_delete == Some(id), context),
        };

        // Above every screen, not just one: what y-pop would warn about can
        // be anywhere in the file.
        let banner = (!settings.warnings.is_empty() && !self.warnings_dismissed)
            .then(|| warnings_banner(&settings.warnings, context.callback(|()| Msg::WarningsDismissed)));
        let screen: View = Stack::vertical(space::L).maybe("warnings", banner).child("screen", screen).into();

        let mut slots = vec![
            SlotView::collection(NavigationViewSlot::MenuItems, self.nav_items(settings)),
            SlotView::new(NavigationViewSlot::Content, shell::page_frame(screen, layout)),
        ];
        // The file path only fits a labelled pane; on the icon rail it's in
        // the status tooltip instead.
        if layout.pane_expanded {
            let footer = Stack::vertical(space::XXS)
                .child("caption", text::caption("Loaded from"))
                .child("path", text::caption(self.path.display().to_string()).opacity(emphasis::SUBTLE));
            slots.push(SlotView::new(
                NavigationViewSlot::PaneFooter,
                Border::new().padding(Thickness::uniform(space::M)).content(footer),
            ));
        }

        let nav = shell::pane_mode(NavigationView::new().pane_title("y-pop"), layout)
            .on_selected_tag_changed(context.callback(Msg::Navigated))
            .grid_row(0)
            .slots(slots);

        let (status, healthy) = self.status(settings);
        let bar = self.bar(&status, healthy, Some(settings), context);

        // Nav takes the height the bar doesn't (`Star` over `Auto`).
        Grid::new().rows([GridLength::STAR, GridLength::Auto]).keyed_children([("nav", nav), ("bar", bar)])
    }
}

impl ConfigApp {
    /// Changes screen and remembers it for the next run.
    fn go(&mut self, screen: Screen) {
        self.screen = screen;
        self.confirming_delete = None;
        self.remember_screen();
    }

    fn remember_screen(&mut self) {
        let Ok(settings) = &self.settings else { return };
        self.prefs.last_screen = match self.screen {
            Screen::General => LastScreen::General,
            Screen::Skin => LastScreen::Skin,
            Screen::Variables => LastScreen::Variables,
            Screen::Menu(id) => roots::position(settings, id).map_or(LastScreen::General, LastScreen::Menu),
        };
        self.prefs.save(&prefs::path_for(&self.path));
    }

    fn add_menu(&mut self) {
        let Ok(settings) = &mut self.settings else { return };
        let id = roots::add_menu(settings);
        self.dirty = true;
        self.go(Screen::Menu(id));
    }

    fn delete_menu(&mut self, id: ItemId) {
        self.confirming_delete = None;
        let Ok(settings) = &mut self.settings else { return };

        let before = settings.clone();
        let Some(removed) = roots::delete(settings, id) else { return };
        self.undo = Some(Undo { label: format!("Deleted {}", roots::label(&removed)), settings: before });
        self.dirty = true;
        if self.screen == Screen::Menu(id) {
            self.go(Screen::General);
        }
    }

    fn save(&mut self) {
        let Ok(settings) = &mut self.settings else { return };
        match settings.save_to(&self.path) {
            Ok(()) => {
                self.dirty = false;
                self.missing = false;
                self.save_error = None;
                // y-pop has no file watcher: tell it, so the change applies
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
        self.confirming_delete = None;
        self.undo = None;
        // A menu that is gone from the file is a screen that is gone.
        if let (Ok(settings), Screen::Menu(id)) = (&self.settings, self.screen)
            && roots::find(settings, id).is_none()
        {
            self.screen = Screen::General;
        }
    }

    /// The navigation pane: a "Settings" group, a "Tray menus" group, and
    /// the action that adds one. `NavigationView` has no header item in this
    /// version of `windows-reactor`, so the groups are expanded parents that
    /// don't select.
    fn nav_items(&self, settings: &Settings) -> Vec<(String, View)> {
        let current = self.screen;
        let entry = |target: Target, symbol: Symbol, content: View| -> View {
            let selected = target == Target::Screen(current);
            NavigationViewItem::new().tag(target.tag()).is_selected(selected).slots([
                SlotView::new(NavigationViewItemSlot::Icon, SymbolIcon::new().symbol(symbol)),
                SlotView::new(NavigationViewItemSlot::Content, content),
            ])
        };
        let group = |tag: &str, title: &str, symbol: Symbol, children: Vec<(String, View)>| -> View {
            NavigationViewItem::new().tag(tag).selects_on_invoked(false).is_expanded(true).slots([
                SlotView::new(NavigationViewItemSlot::Icon, SymbolIcon::new().symbol(symbol)),
                SlotView::new(NavigationViewItemSlot::Content, title),
                SlotView::collection(NavigationViewItemSlot::MenuItems, children),
            ])
        };

        let general = [
            (Screen::General, Symbol::Setting, "General"),
            (Screen::Skin, Symbol::FontColor, "Skin"),
            (Screen::Variables, Symbol::Tag, "Variables"),
        ];
        let general: Vec<(String, View)> = general
            .into_iter()
            .map(|(screen, symbol, title)| {
                let target = Target::Screen(screen);
                (target.tag(), entry(target, symbol, text::body(title).into()))
            })
            .collect();

        let menus: Vec<(String, View)> = roots::menus(settings)
            .map(|item| {
                let target = Target::Screen(Screen::Menu(item.id));
                let mut label = Stack::horizontal(space::S).child("name", text::body(roots::label(item)));
                if let Some(key) = roots::first_hotkey(item) {
                    label = label.child("hotkey", text::caption(key).opacity(emphasis::SUBTLE));
                }
                (target.tag(), entry(target, screens::menu_symbol(item), label.into()))
            })
            .collect();

        let new_menu: View = NavigationViewItem::new().tag(Target::NewMenu.tag()).slots([
            SlotView::new(NavigationViewItemSlot::Icon, SymbolIcon::new().symbol(Symbol::Add)),
            SlotView::new(NavigationViewItemSlot::Content, "New tray menu"),
        ]);

        vec![
            ("settings".to_owned(), group("group-settings", "Settings", Symbol::Setting, general)),
            ("menus".to_owned(), group("group-menus", "Tray menus", Symbol::Home, menus)),
            ("new".to_owned(), new_menu),
        ]
    }

    /// The status line, and whether it's good news.
    fn status(&self, settings: &Settings) -> (String, bool) {
        match &self.save_error {
            Some(error) => (format!("Save failed: {error}"), false),
            None if self.dirty => ("Unsaved changes".to_owned(), true),
            None => match self.applied {
                Some(true) => ("Saved and applied — y-pop is reloading it".to_owned(), true),
                Some(false) => ("Saved — y-pop isn't running; it'll use these settings when it starts".to_owned(), true),
                None if self.missing => ("No y-pop.xml yet — Save creates it".to_owned(), true),
                None => (summary(settings), true),
            },
        }
    }

    /// The bar along the bottom: status, an inline question or an Undo
    /// beside the buttons, Reload, Save.
    fn bar(&self, status: &str, healthy: bool, settings: Option<&Settings>, context: &ViewContext<Self>) -> View {
        let extra: Option<View> = if self.confirming_reload {
            Some(
                Stack::horizontal(space::S)
                    .child("ask", text::body("Discard unsaved changes?"))
                    .child("yes", button("Discard", true, true, context.callback(|()| Msg::ReloadConfirmed)))
                    .child("no", button("Keep editing", false, true, context.callback(|()| Msg::ReloadCancelled)))
                    .into(),
            )
        } else {
            self.undo.as_ref().map(|undo| {
                Stack::horizontal(space::S)
                    .child("what", text::body(undo.label.clone()))
                    .child("undo", button("Undo", false, true, context.callback(|()| Msg::Undo)))
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
                save_tooltip: "Rewrites y-pop.xml from these settings. Comments, unknown attributes and entries y-pop \
                               can't load are kept; only the layout is regenerated. The first save of a session keeps \
                               the old file as y-pop.xml.bak.",
                extra,
            },
            context.callback(|()| Msg::Reload),
            context.callback(|()| Msg::Save),
        )
    }
}

/// "3 tray menus · 21 items" — the status line when nothing's pending.
fn summary(settings: &Settings) -> String {
    let menus = roots::menus(settings).count();
    format!("{menus} tray menu{} · {} items", if menus == 1 { "" } else { "s" }, settings.items().count())
}

/// The load-warnings banner in y-pop's words: unlike y-snap's, a save keeps
/// what the warnings describe (as comments, or as the attributes written
/// back), so it says what is *wrong*, not what will be lost.
fn warnings_banner(warnings: &[String], on_close: Callback<()>) -> View {
    let count = warnings.len();
    shell::warnings_banner(
        format!("y-pop would report {count} problem{} in y-pop.xml", if count == 1 { "" } else { "s" }),
        "Entries y-pop can't load are kept in the file as comments, and unknown attributes are written back, so \
         saving loses nothing — but those parts do nothing until they're fixed.",
        warnings,
        on_close,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_pop::variables::Variable;

    #[test]
    fn a_tag_names_its_target_and_back() {
        let settings = Settings::load_str(r#"<y-pop><group name="A"/></y-pop>"#, &mut Vec::<Variable>::new())
            .expect("valid");
        let menu = roots::menus(&settings).next().expect("one").id;

        for target in [
            Target::Screen(Screen::General),
            Target::Screen(Screen::Skin),
            Target::Screen(Screen::Variables),
            Target::Screen(Screen::Menu(menu)),
            Target::NewMenu,
        ] {
            assert_eq!(Target::from_tag(&target.tag(), &settings), Some(target));
        }
        assert_eq!(Target::from_tag("group-menus", &settings), None);
        assert_eq!(Target::from_tag("menu:999", &settings), None);
    }
}
