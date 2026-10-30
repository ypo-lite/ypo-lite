//! y-pop Config — a WinUI 3 settings app for `y-pop.xml`, built on
//! `windows-reactor` and the `ui-kit` toolkit (**read `ui-kit/docs/
//! ui-toolkit.md` before adding or changing a screen**).
//!
//! Module map:
//! - [`roots`] — the tray menus (the items under `<y-pop>`) as plain
//!   functions on `Settings`: add, delete, find, label. No UI, tested.
//! - [`prefs`] — "show comments" and the last screen, in `y-pop-config.ini`
//!   next to `y-pop.xml`, never in it.
//! - [`skin`], [`skin_view`], [`skin_preview`] — the Skin screen: the edits as
//!   functions on `Settings`, the view, and the pictures of a menu.
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

mod app_form;
mod app_icons;
mod from_file;
mod general;
mod icon_picker;
mod inspector;
mod menu_edit;
mod menu_view;
mod prefs;
mod roots;
mod key_edit;
mod search_edit;
mod search_only;
mod search_view;
mod skin;
mod skin_preview;
mod skin_view;
mod start_menu;
mod target_check;
mod tree;
mod variables;
use ui::shell;
use ypo_ui as ui;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use windows_reactor::*;
use y_core::single_instance;
use y_pop::builtins::Builtins;
use y_pop::config::{self, LoadError};
use y_pop::ipc;
use y_pop::settings::{ItemId, PopupStyle, Renderer, Settings, VarEntry};

use menu_edit::MenuMsg;
use menu_view::MenuUi;
use prefs::{LastScreen, Prefs};
use ui::prelude::*;
use variables::VariablesUi;

fn main() {
    // One editor at a time: two windows over one file would overwrite each
    // other's saves. A second launch (the tray's `<settings/>` clicked again)
    // brings the open one forward instead. Held until the process exits.
    let _instance = match single_instance::SingleInstance::acquire(ipc::CONFIG_INSTANCE) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            // What this launch was asked to do goes to the open window.
            if let Some(request) = ipc::Request::from_args(std::env::args().skip(1)) {
                let _ = ipc::post_request(&request);
            }
            single_instance::bring_to_front(ipc::CONFIG_TITLE);
            return;
        }
        // Couldn't even create the mutex: open anyway — losing the
        // one-window guarantee beats not opening at all.
        Err(_) => None,
    };
    // A request left for an earlier window is not for this one.
    if _instance.is_some() {
        let _ = std::fs::remove_file(ipc::request_path());
    }
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
    /// y-pop's builtin variables, resolved lazily and cached; behind a
    /// `RefCell` because a view only has `&self` and resolving fills the cache.
    builtins: RefCell<Builtins>,
    variables_ui: VariablesUi,
    /// What is selected in each tray menu, by the menu's id. A menu with no
    /// entry (or a stale one) has its own item selected.
    selection: HashMap<ItemId, ItemId>,
    /// Groups folded in the outline — how the outline looks, not the file.
    collapsed: HashSet<ItemId>,
    /// The row of less common things to add is open.
    more_open: bool,
    /// The app picker is open under the toolbar.
    app_picker: bool,
    /// The Start menu dialog (the picker's "Start menu…") is open.
    start_open: bool,
    /// The Start menu source: what was found, the search, the ticks.
    start_menu: start_menu::StartMenu,
    start_entries: Vec<start_menu::StartEntry>,
    /// The programs running when the "Running apps" list last opened.
    running_entries: Vec<start_menu::StartEntry>,
    /// How the item selected after a drop was made, for the strip at the top
    /// of its inspector. Lives until another item is selected; never saved.
    origin: Option<(ItemId, from_file::Origin)>,
    /// What a file being dragged is over, for the outline to highlight.
    drag_over: Option<menu_edit::DropTarget>,
    /// The inspector shows the XML an item is written as.
    show_xml: bool,
    /// The dialog that chooses a key, when one is open.
    key_edit: key_edit::KeyEditUi,
    /// The tray menus whose name can't be written in `<menus include>` yet, with
    /// the one that is there: mid-rename, or holding a `;`. Kept for the session.
    include_names: HashMap<ItemId, String>,
    /// What an app's target was before its mode changed, so switching back doesn't
    /// lose what was typed: `(exe, app-id)`. Kept for the session only.
    target_stash: HashMap<ItemId, (Option<String>, Option<String>)>,
    /// What "Run now" last did, for the status bar: the text and whether it went well.
    run_status: Option<(String, bool)>,
    /// The icons of the items, loaded in the background (`app_icons`).
    icons: app_icons::IconStore,
    /// The icon picker modal, when one is open.
    icon_picker: icon_picker::IconPickerUi,
    /// The Skin screen: the colour being edited, and its pictures.
    skin_ui: skin::SkinUi,
    /// The Search screen: the section whose fields are open.
    search_ui: search_view::SearchUi,
    /// Renders what is copied while the window is alive — see
    /// `y_core::shell::ClipboardFlusher`. Held only for its `Drop`.
    _clipboard: Option<y_core::shell::ClipboardFlusher>,
    /// Whether the notes box on General has been mounted and may be given its
    /// text. A WinUI `TextBox` keeps only the first line of a multi-line text
    /// it is given while it is still single-line, and `windows-reactor` sets
    /// `Text` before `AcceptsReturn` when it creates one — so the box is
    /// created empty (already multi-line), and the text arrives one render
    /// later, through [`Msg::NotesReady`]. Reset whenever the screen changes,
    /// since leaving General discards the box. Until then an edit the box
    /// reports is ignored: it would be the truncated text, not the user's.
    notes_ready: bool,
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
    Search,
    Menu(ItemId),
    /// A group or folder that only the search knows, edited like a tray menu.
    SearchOnly(ItemId),
}

impl Screen {
    /// The top-level item whose tree the screen edits: a tray menu or a
    /// search-only group or folder.
    fn menu_root(self) -> Option<ItemId> {
        match self {
            Self::Menu(id) | Self::SearchOnly(id) => Some(id),
            _ => None,
        }
    }
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
            Self::Screen(Screen::Search) => "search".to_owned(),
            Self::Screen(Screen::Menu(id)) => format!("menu:{}", id.get()),
            Self::Screen(Screen::SearchOnly(id)) => format!("only:{}", id.get()),
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
            "search" => Some(Self::Screen(Screen::Search)),
            "new" => Some(Self::NewMenu),
            other if other.starts_with("only:") => {
                let id = search_only::roots(settings)
                    .map(|item| item.id)
                    .find(|id| format!("only:{}", id.get()) == other)?;
                Some(Self::Screen(Screen::SearchOnly(id)))
            }
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

    // General
    RendererChosen(usize),
    PopupStyleChosen(usize),
    ShellExecuteToggled(bool),
    ShowShortcutsToggled(bool),
    OpenConfigFolder,
    EditConfigXml,
    ShowCommentsToggled(bool),
    NotesEdited(String),
    NotesReady,

    // Tray menus
    Menu(MenuMsg),
    /// A control reported "nothing chosen": not an edit.
    Ignored,

    /// Another launch asked this window to do something (`--select`,
    /// `--new-app`), see [`ConfigApp::watch_requests`].
    Requested(ipc::Request),

    /// An item's icon came back from the thread pool (PNG bytes, or none found).
    IconLoaded(app_icons::IconSpec, Option<Vec<u8>>),
    /// A Store logo that failed has waited long enough: ask for it again.
    IconRetry,
    /// The Start menu shortcuts were found and read on the thread pool.
    StartMenuScanned(Vec<start_menu::StartEntry>),
    /// The running programs were listed on the thread pool.
    RunningScanned(Vec<start_menu::StartEntry>),
    /// The icon picker.
    Icon(icon_picker::IconMsg),
    /// The dialog that chooses a key.
    Key(key_edit::KeyMsg),

    // Skin
    Skin(skin::SkinMsg),

    // Search
    Search(search_view::SearchMsg),

    // Variables
    VarNameChanged(usize, String),
    VarValueChanged(usize, String),
    VarAdded,
    VarMoved {
        index: usize,
        up: bool,
    },
    VarDeleteAsked(usize),
    VarDeleteConfirmed(usize),
    VarDeleteCancelled,
}

/// Reads `path`, with y-pop's own builtins so the warnings are y-pop's.
/// The folder this program is in — what `%pop%` means.
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

fn load(path: &Path) -> (Result<Settings, LoadError>, bool) {
    let mut builtins = Builtins::new(&exe_dir(), path);

    match Settings::load_from(path.to_owned(), &mut builtins) {
        Err(LoadError::Missing(_)) => (Ok(Settings::empty()), true),
        loaded => (loaded, false),
    }
}

impl Component for ConfigApp {
    type Input = ();
    type Message = Msg;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let path = config::path();
        let prefs = Prefs::load(&prefs::path_for(&path));
        let (settings, missing) = load(&path);
        let builtins = Builtins::new(&exe_dir(), &path);

        let screen = match (&settings, prefs.last_screen) {
            (Ok(settings), LastScreen::Menu(position)) => {
                roots::at_position(settings, position).map_or(Screen::General, Screen::Menu)
            }
            (_, LastScreen::Skin) => Screen::Skin,
            (_, LastScreen::Variables) => Screen::Variables,
            (_, LastScreen::Search) => Screen::Search,
            (Ok(settings), LastScreen::SearchOnly(position)) => {
                search_only::at_position(settings, position)
                    .map_or(Screen::Search, Screen::SearchOnly)
            }
            _ => Screen::General,
        };

        let mut app = Self {
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
            builtins: RefCell::new(builtins),
            variables_ui: VariablesUi::default(),
            selection: HashMap::new(),
            collapsed: HashSet::new(),
            more_open: false,
            app_picker: false,
            start_open: false,
            start_menu: start_menu::StartMenu::default(),
            start_entries: Vec::new(),
            running_entries: Vec::new(),
            origin: None,
            drag_over: None,
            show_xml: false,
            key_edit: key_edit::KeyEditUi::default(),
            include_names: HashMap::new(),
            target_stash: HashMap::new(),
            run_status: None,
            icons: app_icons::IconStore::default(),
            icon_picker: icon_picker::IconPickerUi::default(),
            skin_ui: skin::SkinUi::default(),
            search_ui: search_view::SearchUi::default(),
            _clipboard: y_core::shell::ClipboardFlusher::start(),
            notes_ready: false,
            window_width: shell::INITIAL_WINDOW.0,
        };
        // Whatever started this launch, and what a later one will ask.
        if let Some(request) = ipc::Request::from_args(std::env::args().skip(1)) {
            app.apply_request(request);
        }
        app.watch_requests(context);
        app.request_icons(context);
        app
    }

    fn update(&mut self, message: Msg, context: &ComponentContext<Self>) {
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
                    self.variables_ui = VariablesUi::default();
                    self.key_edit = key_edit::KeyEditUi::default();
                    self.include_names.clear();
                    self.skin_ui.editing = None;
                    self.search_ui = search_view::SearchUi::default();
                    self.leave_missing_menu();
                }
            }

            Msg::Menu(message) => self.edit_menu(message),
            Msg::Key(message) => self.edit_key(message),
            Msg::Ignored => {}
            Msg::Requested(request) => {
                self.apply_request(request);
                self.watch_requests(context);
            }
            Msg::IconLoaded(spec, png) => {
                self.icons.finish(spec, png);
                self.schedule_icon_retry(context);
            }
            Msg::IconRetry => {
                self.icons.retry_due();
                self.schedule_icon_retry(context);
            }
            Msg::StartMenuScanned(entries) => {
                self.start_entries = entries;
                self.start_menu.state = start_menu::Load::Done;
            }
            Msg::RunningScanned(entries) => {
                self.running_entries = entries;
                self.start_menu.running = start_menu::Load::Done;
            }
            Msg::Icon(message) => self.edit_icon(message),
            Msg::Skin(message) => self.edit_skin(message),
            Msg::Search(message) => self.edit_search(message),

            other => self.edit(other),
        }
        self.request_start_menu(context);
        self.request_running(context);
        self.request_icons(context);
        self.schedule_icon_retry(context);
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
            // Nothing to edit if the file can't be read — say why, plainly,
            // and keep Reload within reach for when it's been fixed.
            Err(error) => {
                let screen = page("Couldn't load y-pop.xml", &error.to_string(), View::empty());
                let bar = self.bar("y-pop.xml couldn't be loaded", false, None, context);
                return Grid::new()
                    .rows([GridLength::STAR, GridLength::Auto])
                    .keyed_children([
                        ("page", cell(shell::page_frame(screen, layout), 0, 0)),
                        ("bar", bar),
                    ]);
            }
        };

        // The notes box is created empty and filled one render later.
        if self.screen == Screen::General && !self.notes_ready {
            let _ = context.callback(|()| Msg::NotesReady).call(());
        }
        let viewport = layout.content;
        let screen = match self.screen {
            Screen::General => {
                general::general_view(settings, &self.path, self.notes_ready, viewport, context)
            }
            Screen::Skin => skin_view::skin_view(settings, &self.skin_ui, viewport, context),
            Screen::Search => {
                search_view::search_view(settings, &self.search_ui, viewport, context)
            }
            Screen::Variables => variables::variables_view(
                settings,
                &mut *self.builtins.borrow_mut(),
                self.prefs.show_comments,
                &self.variables_ui,
                viewport,
                context,
            ),
            Screen::Menu(id) | Screen::SearchOnly(id) => menu_view::menu_view(
                settings,
                &MenuUi {
                    root: id,
                    selected: self.selected(id),
                    collapsed: &self.collapsed,
                    more_open: self.more_open,
                    app_picker: self.app_picker,
                    drag_over: self.drag_over,
                    origin: self
                        .origin
                        .as_ref()
                        .filter(|(item, _)| *item == self.selected(id))
                        .map(|(_, origin)| origin),
                    show_xml: self.show_xml,
                    show_comments: self.prefs.show_comments,
                    confirming_delete: self.confirming_delete == Some(id),
                    builtins: &self.builtins,
                    icons: &self.icons,
                },
                viewport,
                context,
            ),
        };

        // Above every screen, not just one: what y-pop would warn about can
        // be anywhere in the file.
        let banner = (!settings.warnings.is_empty() && !self.warnings_dismissed).then(|| {
            warnings_banner(
                &settings.warnings,
                context.callback(|()| Msg::WarningsDismissed),
            )
        });
        // A tray menu shown as two panes scrolls them itself: the page stays
        // put, and gives the screen the window's height.
        let pinned =
            self.screen
            .menu_root()
            .is_some_and(|id| menu_view::is_pinned(settings, id, viewport));
        let page: View = if pinned {
            let screen = Grid::new()
                .rows([GridLength::Auto, GridLength::STAR])
                .row_spacing(space::L)
                .keyed_children([
                    ("warnings", cell(banner.unwrap_or_else(View::empty), 0, 0)),
                    ("screen", cell(screen, 1, 0)),
                ]);
            shell::page_frame_pinned(screen, layout)
        } else {
            let screen: View = Stack::vertical(space::L)
                .maybe("warnings", banner)
                .child("screen", screen)
                .into();
            shell::page_frame(screen, layout)
        };

        let mut slots = vec![
            SlotView::collection(NavigationViewSlot::MenuItems, self.nav_items(settings)),
            SlotView::new(NavigationViewSlot::Content, page),
        ];
        // The file path only fits a labelled pane; on the icon rail it's in
        // the status tooltip instead.
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

        let nav = shell::pane_mode(NavigationView::new().pane_title("y-pop"), layout)
            .on_selected_tag_changed(context.callback(Msg::Navigated))
            .grid_row(0)
            .slots(slots);

        let (status, healthy) = self.status(settings);
        let bar = self.bar(&status, healthy, Some(settings), context);

        // Nav takes the height the bar doesn't (`Star` over `Auto`).
        let app: View = Grid::new()
            .rows([GridLength::STAR, GridLength::Auto])
            .keyed_children([("nav", nav), ("bar", bar)]);
        // A picker covers the whole window, bar included.
        let window = Viewport::new(self.window_width);
        let modal = key_edit::key_modal(settings, &self.key_edit, window, context)
            .or_else(|| {
                icon_picker::icon_modal(
                    settings,
                    &self.icon_picker,
                    &mut *self.builtins.borrow_mut(),
                    &self.icons,
                    window,
                    context,
                )
            })
            .or_else(|| match self.screen.menu_root() {
                Some(root) if self.start_open => Some(menu_view::start_modal(
                    settings,
                    &menu_view::StartUi {
                        root,
                        start: &self.start_menu,
                        entries: self.source_entries(),
                        builtins: &self.builtins,
                        icons: &self.icons,
                    },
                    window,
                    context,
                )),
                _ => None,
            });
        // Always the same wrapper, the app always its first child: if the root
        // changed shape when a picker opened, the reactor would rebuild the
        // whole app under it and every scroll position would be lost.
        let mut layers = vec![("app", app)];
        layers.extend(modal.map(|modal| ("modal", modal)));
        Grid::new().keyed_children(layers)
    }
}

#[cfg(test)]
impl ConfigApp {
    /// What pressing Undo does, without a `ComponentContext`.
    fn update_undo_for_test(&mut self) {
        if let Some(undo) = self.undo.take() {
            self.settings = Ok(undo.settings);
            self.dirty = true;
            self.variables_ui = VariablesUi::default();
        }
    }
}

impl ConfigApp {
    /// The first time the Start menu dialog opens, reads the Start menu on the
    /// thread pool (kept for the session: it rarely changes under us).
    fn request_start_menu(&mut self, context: &ComponentContext<Self>) {
        if !self.start_open || self.start_menu.state != start_menu::Load::NotStarted {
            return;
        }
        self.start_menu.state = start_menu::Load::Loading;
        context
            .spawn_background(|_| Msg::StartMenuScanned(start_menu::scan(&start_menu::folders())));
    }

    /// Each time the running programs list opens, lists them on the thread
    /// pool: unlike the Start menu, what is running changes under us.
    fn request_running(&mut self, context: &ComponentContext<Self>) {
        if !self.start_open
            || self.start_menu.source != start_menu::Source::Running
            || self.start_menu.running != start_menu::Load::NotStarted
        {
            return;
        }
        self.start_menu.running = start_menu::Load::Loading;
        context.spawn_background(|_| Msg::RunningScanned(start_menu::scan_running()));
    }

    /// The entries of the list the Start menu dialog shows.
    fn source_entries(&self) -> &[start_menu::StartEntry] {
        match self.start_menu.source {
            start_menu::Source::Running => &self.running_entries,
            start_menu::Source::Shortcuts | start_menu::Source::Store => &self.start_entries,
        }
    }

    /// Starts loading, on the thread pool, every icon the tree shows that has
    /// not been asked for yet. Called after every message: an edit to a
    /// target or an `icon=` can make a new one necessary.
    fn request_icons(&mut self, context: &ComponentContext<Self>) {
        let Ok(settings) = &self.settings else { return };
        let wanted: Vec<app_icons::IconSpec> = {
            let mut builtins = self.builtins.borrow_mut();
            let mut wanted: Vec<_> = settings
                .all_items()
                .filter_map(|item| app_icons::spec_for(settings, item, &mut *builtins))
                .collect();
            wanted.extend(self.icon_picker.visible());
            if self.start_open && self.start_menu.ready() {
                wanted.extend(
                    self.start_menu
                        .listed(self.source_entries())
                        .0
                        .into_iter()
                        .map(|entry| entry.icon_spec()),
                );
            }
            wanted
        };
        for spec in wanted {
            if self.icons.begin(&spec) {
                context.spawn_background(move |_| {
                    let png = app_icons::load_png(&spec);
                    Msg::IconLoaded(spec, png)
                });
            }
        }
    }

    /// Wakes the app when the earliest failed Store logo is due for another
    /// try. One sleeping task at most, so the pool is never tied up by retries.
    fn schedule_icon_retry(&mut self, context: &ComponentContext<Self>) {
        if let Some(delay) = self.icons.next_wake() {
            context.spawn_background(move |_| {
                std::thread::sleep(delay);
                Msg::IconRetry
            });
        }
    }

    /// Waits, on the thread pool, for a request left by a second launch
    /// (`ipc::post_request`) and returns it as a [`Msg::Requested`]; asked
    /// again after each one. A file polled twice a second: nothing here owns
    /// the window procedure, and this costs one `stat` while the window is open.
    fn watch_requests(&self, context: &ComponentContext<Self>) {
        context.spawn_background(|_| {
            loop {
                if let Some(request) = ipc::take_request() {
                    return Msg::Requested(request);
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
    }

    /// Changes screen and remembers it for the next run.
    fn go(&mut self, screen: Screen) {
        if screen != self.screen {
            self.notes_ready = false;
        }
        self.screen = screen;
        self.confirming_delete = None;
        self.key_edit = key_edit::KeyEditUi::default();
        self.include_names.clear();
        self.skin_ui.editing = None;
        self.search_ui = search_view::SearchUi::default();
        self.run_status = None;
        self.remember_screen();
    }

    /// After the settings were swapped for another version (Undo): a menu that
    /// is not in them is a screen that is not there.
    fn leave_missing_menu(&mut self) {
        if let (Ok(settings), Some(id)) = (&self.settings, self.screen.menu_root())
            && tree::item(settings, id).is_none()
        {
            self.go(Screen::General);
        }
    }

    fn remember_screen(&mut self) {
        let Ok(settings) = &self.settings else { return };
        self.prefs.last_screen = match self.screen {
            Screen::General => LastScreen::General,
            Screen::Skin => LastScreen::Skin,
            Screen::Variables => LastScreen::Variables,
            Screen::Search => LastScreen::Search,
            Screen::Menu(id) => {
                roots::position(settings, id).map_or(LastScreen::General, LastScreen::Menu)
            }
            Screen::SearchOnly(id) => search_only::position(settings, id)
                .map_or(LastScreen::Search, LastScreen::SearchOnly),
        };
        self.prefs.save(&prefs::path_for(&self.path));
    }

    /// Settles every message [`Component::update`] doesn't: the ones that
    /// edit the settings, and the few buttons that act on the file.
    fn edit(&mut self, message: Msg) {
        // About the app and the file on disk, not about the settings.
        match message {
            Msg::ShowCommentsToggled(on) => {
                self.prefs.show_comments = on;
                self.prefs.save(&prefs::path_for(&self.path));
                return;
            }
            Msg::OpenConfigFolder => {
                if let Some(folder) = self.path.parent().filter(|folder| folder.is_dir()) {
                    y_core::shell::open(folder);
                }
                return;
            }
            Msg::EditConfigXml => {
                if self.path.is_file() {
                    y_core::shell::open(&self.path);
                }
                return;
            }
            Msg::VarDeleteCancelled => {
                self.variables_ui.confirming_delete = None;
                return;
            }
            Msg::NotesReady => {
                self.notes_ready = true;
                return;
            }
            Msg::NotesEdited(_) if !self.notes_ready => return,
            _ => {}
        }

        let Ok(settings) = &mut self.settings else {
            return;
        };
        let mut changed = false;

        match message {
            Msg::RendererChosen(choice) => {
                let renderer = if choice == 0 {
                    Renderer::Native
                } else {
                    Renderer::Overlay
                };
                if settings.globals.popup.unwrap_or(Renderer::Native) != renderer {
                    settings.globals.popup = Some(renderer);
                    changed = true;
                }
            }
            // Only the list exists; the other two are disabled choices.
            Msg::PopupStyleChosen(0)
                if settings.globals.popup_style.unwrap_or(PopupStyle::List) != PopupStyle::List =>
            {
                settings.globals.popup_style = Some(PopupStyle::List);
                changed = true;
            }
            Msg::ShellExecuteToggled(on)
                if settings.globals.shell_execute.unwrap_or(false) != on =>
            {
                settings.globals.shell_execute = Some(on);
                changed = true;
            }
            Msg::ShowShortcutsToggled(on)
                if settings.globals.show_shortcuts.unwrap_or(true) != on =>
            {
                settings.globals.show_shortcuts = Some(on);
                changed = true;
            }

            Msg::NotesEdited(text) => {
                // Several notes becoming one is worth an Undo; typing in a
                // single one is not.
                let merging = settings.file_notes_count() > 1;
                let before = merging.then(|| settings.clone());
                if settings.set_file_notes_text(&text) {
                    if let Some(before) = before {
                        self.undo = Some(Undo {
                            label: "Merged the file notes".to_owned(),
                            settings: before,
                        });
                    }
                    changed = true;
                }
            }

            Msg::VarNameChanged(index, name) => {
                let unchanged = matches!(settings.variables.get(index), Some(VarEntry::Var(var)) if var.name == name);
                if !unchanged {
                    match settings.rename_variable(index, &name) {
                        Ok(_) => {
                            self.variables_ui.name_error = None;
                            changed = true;
                        }
                        Err(error) => self.variables_ui.name_error = Some((index, error)),
                    }
                }
            }
            Msg::VarValueChanged(index, value) => {
                if let Some(VarEntry::Var(var)) = settings.variables.get_mut(index)
                    && var.value != value
                {
                    var.value = value;
                    changed = true;
                }
            }
            Msg::VarAdded => {
                settings.add_variable();
                changed = true;
            }
            Msg::VarMoved { index, up } => {
                // Indices shift: whatever pointed at one is stale.
                self.variables_ui = VariablesUi::default();
                changed = settings.move_variable_entry(index, up);
            }
            Msg::VarDeleteAsked(index) => {
                if settings.variable_users(index).is_empty() {
                    changed =
                        delete_variable(settings, index, &mut self.undo, &mut self.variables_ui);
                } else {
                    self.variables_ui.confirming_delete = Some(index);
                }
            }
            Msg::VarDeleteConfirmed(index) => {
                changed = delete_variable(settings, index, &mut self.undo, &mut self.variables_ui);
            }

            // Settled by `update`, or above.
            _ => {}
        }

        if changed {
            self.dirty = true;
        }
    }

    fn add_menu(&mut self) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        let id = roots::add_menu(settings);
        self.dirty = true;
        self.go(Screen::Menu(id));
    }

    fn delete_menu(&mut self, id: ItemId) {
        self.confirming_delete = None;
        self.key_edit = key_edit::KeyEditUi::default();
        self.run_status = None;
        let Ok(settings) = &mut self.settings else {
            return;
        };

        let before = settings.clone();
        let listed = self.include_names.remove(&id);
        let removed = if tree::is_search_only(settings, id) {
            search_only::delete(settings, id)
        } else {
            roots::delete(settings, id)
        };
        let Some(removed) = removed else {
            return;
        };
        // In the same Undo step: y-pop leaves every tray menu out when
        // `<menus include>` lists only menus that don't exist.
        if let Some(name) = listed.or_else(|| removed.name.clone()) {
            search_edit::drop_from_include(settings, &name);
        }
        self.undo = Some(Undo {
            label: format!("Deleted {}", roots::label(&removed)),
            settings: before,
        });
        self.dirty = true;
        if self.screen.menu_root() == Some(id) {
            self.go(Screen::General);
        }
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
                // y-pop has no file watcher: tell it, so the change applies
                // now rather than at its next start.
                self.applied = Some(ipc::request_reload());
            }
            Err(error) => self.save_error = Some(error.to_string()),
        }
    }

    fn reload(&mut self) {
        let (settings, missing) = load(&self.path);
        // A failed load had no notes box; the next one is a fresh mount.
        self.notes_ready = false;
        self.settings = settings;
        self.missing = missing;
        self.dirty = false;
        self.save_error = None;
        self.warnings_dismissed = false;
        self.applied = None;
        self.confirming_reload = false;
        self.confirming_delete = None;
        self.key_edit = key_edit::KeyEditUi::default();
        self.skin_ui.editing = None;
        self.search_ui = search_view::SearchUi::default();
        self.run_status = None;
        self.undo = None;
        // A menu that is gone from the file is a screen that is gone.
        if let (Ok(settings), Some(id)) = (&self.settings, self.screen.menu_root())
            && tree::item(settings, id).is_none()
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
            NavigationViewItem::new()
                .tag(target.tag())
                .is_selected(selected)
                .slots([
                    SlotView::new(
                        NavigationViewItemSlot::Icon,
                        SymbolIcon::new().symbol(symbol),
                    ),
                    SlotView::new(NavigationViewItemSlot::Content, content),
                ])
        };
        let group =
            |tag: &str, title: &str, symbol: Symbol, children: Vec<(String, View)>| -> View {
                NavigationViewItem::new()
                    .tag(tag)
                    .selects_on_invoked(false)
                    .is_expanded(true)
                    .slots([
                        SlotView::new(
                            NavigationViewItemSlot::Icon,
                            SymbolIcon::new().symbol(symbol),
                        ),
                        SlotView::new(NavigationViewItemSlot::Content, title),
                        SlotView::collection(NavigationViewItemSlot::MenuItems, children),
                    ])
            };

        let general = [
            (Screen::General, Symbol::Setting, "General"),
            (Screen::Skin, Symbol::FontColor, "Skin"),
            (Screen::Variables, Symbol::Tag, "Variables"),
            (Screen::Search, Symbol::Find, "Search"),
        ];
        let general: Vec<(String, View)> = general
            .into_iter()
            .map(|(screen, symbol, title)| {
                let target = Target::Screen(screen);
                (
                    target.tag(),
                    entry(target, symbol, text::body(title).into()),
                )
            })
            .collect();

        let menus: Vec<(String, View)> = roots::menus(settings)
            .map(|item| {
                let target = Target::Screen(Screen::Menu(item.id));
                let mut label =
                    Stack::horizontal(space::S).child("name", text::body(roots::label(item)));
                if let Some(key) = roots::first_hotkey(item) {
                    label = label.child("hotkey", text::caption(key).opacity(emphasis::SUBTLE));
                }
                (
                    target.tag(),
                    entry(target, menu_view::symbol_of(&item.kind), label.into()),
                )
            })
            .collect();

        let only: Vec<(String, View)> = search_only::roots(settings)
            .map(|item| {
                let target = Target::Screen(Screen::SearchOnly(item.id));
                (
                    target.tag(),
                    entry(
                        target,
                        menu_view::symbol_of(&item.kind),
                        text::body(roots::label(item)).into(),
                    ),
                )
            })
            .collect();

        let new_menu: View = NavigationViewItem::new().tag(Target::NewMenu.tag()).slots([
            SlotView::new(
                NavigationViewItemSlot::Icon,
                SymbolIcon::new().symbol(Symbol::Add),
            ),
            SlotView::new(NavigationViewItemSlot::Content, "New tray menu"),
        ]);

        let mut items = vec![
            (
                "settings".to_owned(),
                group("group-settings", "Settings", Symbol::Setting, general),
            ),
            (
                "menus".to_owned(),
                group("group-menus", "Tray menus", Symbol::Home, menus),
            ),
            ("new".to_owned(), new_menu),
        ];
        // Under the tray menus and their "New": they are not tray icons.
        if !only.is_empty() {
            items.push((
                "only".to_owned(),
                group("group-only", "Only in search", Symbol::Find, only),
            ));
        }
        items
    }

    /// The status line, and whether it's good news.
    fn status(&self, settings: &Settings) -> (String, bool) {
        match &self.save_error {
            Some(error) => (format!("Save failed: {error}"), false),
            None if self.run_status.is_some() => self.run_status.clone().unwrap_or_default(),
            None if self.dirty => ("Unsaved changes".to_owned(), true),
            None => match self.applied {
                Some(true) => ("Saved and applied — y-pop is reloading it".to_owned(), true),
                Some(false) => (
                    "Saved — y-pop isn't running; it'll use these settings when it starts"
                        .to_owned(),
                    true,
                ),
                None if self.missing => ("No y-pop.xml yet — Save creates it".to_owned(), true),
                None => (summary(settings), true),
            },
        }
    }

    /// The "show comments" toggle, on the screens that have comments to show.
    fn bar_tools(&self, settings: &Settings, context: &ViewContext<Self>) -> Option<View> {
        let count = match self.screen {
            Screen::Menu(id) | Screen::SearchOnly(id) => tree::count_comments(settings, id),
            Screen::Variables => variables::count_comments(settings),
            _ => return None,
        };
        let on = self.prefs.show_comments;
        let tooltip = format!("{} comments ({count})", if on { "Hide" } else { "Show" });
        Some(
            ToggleButton::new()
                .is_checked(on)
                .min_width(0.0)
                .on_is_checked_changed(context.callback(Msg::ShowCommentsToggled))
                .content(SymbolIcon::new().symbol(Symbol::Comment))
                .tooltip(tooltip),
        )
    }

    /// The bar along the bottom: status, an inline question or an Undo
    /// beside the buttons, Reload, Save.
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
                save_tooltip: "Rewrites y-pop.xml from these settings. Comments, unknown attributes and entries y-pop \
                               can't load are kept; only the layout is regenerated. The first save of a session keeps \
                               the old file as y-pop.xml.bak.",
                extra,
                tools: settings.and_then(|settings| self.bar_tools(settings, context)),
            },
            context.callback(|()| Msg::Reload),
            context.callback(|()| Msg::Save),
        )
    }
}

/// Removes the variable at `index`, keeping what it was for Undo. `false` if
/// there is no variable there.
fn delete_variable(
    settings: &mut Settings,
    index: usize,
    undo: &mut Option<Undo>,
    ui: &mut VariablesUi,
) -> bool {
    let Some(VarEntry::Var(var)) = settings.variables.get(index) else {
        return false;
    };
    *undo = Some(Undo {
        label: format!("Deleted variable {}", var.name),
        settings: settings.clone(),
    });
    settings.variables.remove(index);
    *ui = VariablesUi::default();
    true
}

/// "3 tray menus · 21 items" — the status line when nothing's pending.
fn summary(settings: &Settings) -> String {
    let menus = roots::menus(settings).count();
    format!(
        "{menus} tray menu{} · {} items",
        if menus == 1 { "" } else { "s" },
        settings.all_items().count()
    )
}

/// The load-warnings banner in y-pop's words: unlike y-snap's, a save keeps
/// what the warnings describe (as comments, or as the attributes written
/// back), so it says what is *wrong*, not what will be lost.
fn warnings_banner(warnings: &[String], on_close: Callback<()>) -> View {
    let count = warnings.len();
    shell::warnings_banner(
        format!(
            "y-pop would report {count} problem{} in y-pop.xml",
            if count == 1 { "" } else { "s" }
        ),
        "Entries y-pop can't load are kept in the file as comments, and unknown attributes are written back, so \
         saving loses nothing — but those parts do nothing until they're fixed.",
        warnings,
        on_close,
    )
}

/// An app over `settings`, as `create` would make it but with no window and a
/// file path under the temp folder (nothing in the tests saves).
#[cfg(test)]
fn test_app(settings: Settings) -> ConfigApp {
    ConfigApp {
        settings: Ok(settings),
        path: std::env::temp_dir()
            .join("y-pop-config-tests")
            .join("y-pop.xml"),
        missing: false,
        prefs: Prefs::default(),
        screen: Screen::General,
        dirty: false,
        save_error: None,
        warnings_dismissed: false,
        applied: None,
        confirming_reload: false,
        confirming_delete: None,
        undo: None,
        builtins: RefCell::new(Builtins::new(Path::new(""), Path::new(""))),
        variables_ui: VariablesUi::default(),
        selection: HashMap::new(),
        collapsed: HashSet::new(),
        more_open: false,
        app_picker: false,
        start_open: false,
        start_menu: start_menu::StartMenu::default(),
        start_entries: Vec::new(),
        running_entries: Vec::new(),
        origin: None,
        drag_over: None,
        show_xml: false,
        key_edit: key_edit::KeyEditUi::default(),
        include_names: HashMap::new(),
        target_stash: HashMap::new(),
        run_status: None,
        icons: app_icons::IconStore::default(),
        icon_picker: icon_picker::IconPickerUi::default(),
        skin_ui: skin::SkinUi::default(),
        search_ui: search_view::SearchUi::default(),
        _clipboard: None,
        notes_ready: true,
        window_width: 1000.0,
    }
}

#[cfg(test)]
mod menu_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use y_pop::variables::Variable;

    fn app(xml: &str) -> ConfigApp {
        test_app(Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid"))
    }

    fn settings(app: &ConfigApp) -> &Settings {
        app.settings.as_ref().expect("loaded")
    }

    const VARIABLES: &str = r#"<y-pop><variables>
        <var name="a" value="1"/>
        <var name="b" value="%a%2"/>
    </variables><app name="X" exe="%a%x.exe"/></y-pop>"#;

    #[test]
    fn a_toggle_that_changes_nothing_is_not_an_edit() {
        let mut app = app("<y-pop><quit/></y-pop>");
        // The defaults: off, and on.
        app.edit(Msg::ShellExecuteToggled(false));
        app.edit(Msg::ShowShortcutsToggled(true));
        app.edit(Msg::RendererChosen(0));
        app.edit(Msg::PopupStyleChosen(0));
        assert!(!app.dirty);
        assert_eq!(settings(&app).globals, Default::default());

        app.edit(Msg::ShellExecuteToggled(true));
        assert!(app.dirty);
        assert_eq!(settings(&app).globals.shell_execute, Some(true));
    }

    #[test]
    fn choosing_the_overlay_is_written_and_the_unbuilt_layouts_are_not_choosable() {
        let mut app = app("<y-pop><quit/></y-pop>");
        app.edit(Msg::RendererChosen(1));
        app.edit(Msg::PopupStyleChosen(1));
        app.edit(Msg::PopupStyleChosen(2));
        assert_eq!(settings(&app).globals.popup, Some(Renderer::Overlay));
        assert_eq!(settings(&app).globals.popup_style, None);
    }

    #[test]
    fn what_a_just_created_notes_box_reports_is_not_the_users_edit() {
        let mut app = app("<!-- one --><y-pop><!-- two --><variables/><quit/></y-pop>");
        app.notes_ready = false;

        // The truncated first line a single-line `TextBox` keeps.
        app.edit(Msg::NotesEdited("one".into()));
        assert!(!app.dirty);
        assert_eq!(
            settings(&app).file_notes_count(),
            2,
            "nothing merged or lost"
        );

        app.edit(Msg::NotesReady);
        assert!(app.notes_ready);
        app.edit(Msg::NotesEdited(
            "one

two!"
                .into(),
        ));
        assert!(app.dirty);
    }

    #[test]
    fn leaving_general_unmounts_the_box_and_returning_primes_it_again() {
        let mut app = app("<y-pop><quit/></y-pop>");
        assert!(app.notes_ready);
        app.go(Screen::Skin);
        app.go(Screen::General);
        assert!(!app.notes_ready);
    }

    #[test]
    fn the_notes_box_edits_one_note_and_merging_several_can_be_undone() {
        let mut app = app("<!-- one --><y-pop><!-- two --><variables/><quit/></y-pop>");

        // What the box shows on first render is not an edit.
        let shown = settings(&app).file_notes_text();
        app.edit(Msg::NotesEdited(shown));
        assert!(!app.dirty && app.undo.is_none());

        app.edit(Msg::NotesEdited(
            "one

two, changed"
                .into(),
        ));
        assert!(app.dirty);
        assert_eq!(settings(&app).file_notes_count(), 1);
        assert_eq!(
            app.undo.as_ref().map(|undo| undo.label.as_str()),
            Some("Merged the file notes")
        );

        app.update_undo_for_test();
        assert_eq!(settings(&app).file_notes_count(), 2);

        // One note: typing is not something to undo.
        app.edit(Msg::NotesEdited("a".into()));
        app.undo = None;
        app.edit(Msg::NotesEdited("ab".into()));
        assert!(app.undo.is_none());
    }

    #[test]
    fn renaming_a_variable_rewrites_its_users_and_a_bad_name_says_why() {
        let mut app = app(VARIABLES);
        app.edit(Msg::VarNameChanged(0, "first".into()));
        assert!(app.dirty);
        assert!(settings(&app).to_xml().contains(r#"value="%first%2""#));
        assert!(settings(&app).to_xml().contains(r#"exe="%first%x.exe""#));

        app.dirty = false;
        app.edit(Msg::VarNameChanged(0, "b".into()));
        assert!(!app.dirty, "a refused name is not an edit");
        assert_eq!(
            app.variables_ui.name_error,
            Some((0, y_pop::settings::NameError::Taken))
        );

        app.edit(Msg::VarNameChanged(0, "second".into()));
        assert_eq!(app.variables_ui.name_error, None);
    }

    #[test]
    fn deleting_a_used_variable_asks_first_and_can_be_undone() {
        let mut app = app(VARIABLES);

        app.edit(Msg::VarDeleteAsked(0));
        assert_eq!(app.variables_ui.confirming_delete, Some(0));
        assert_eq!(settings(&app).variables.len(), 2, "nothing deleted yet");
        app.edit(Msg::VarDeleteCancelled);
        assert_eq!(app.variables_ui.confirming_delete, None);

        app.edit(Msg::VarDeleteAsked(0));
        app.edit(Msg::VarDeleteConfirmed(0));
        assert_eq!(settings(&app).variables.len(), 1);
        assert!(app.dirty);
        assert_eq!(
            app.undo.as_ref().map(|undo| undo.label.as_str()),
            Some("Deleted variable a")
        );

        app.update_undo_for_test();
        assert_eq!(settings(&app).variables.len(), 2);
    }

    #[test]
    fn an_unused_variable_is_deleted_at_once() {
        let mut app = app(VARIABLES);
        app.edit(Msg::VarDeleteAsked(1));
        assert_eq!(settings(&app).variables.len(), 1);
        assert_eq!(app.variables_ui.confirming_delete, None);
    }

    #[test]
    fn variables_are_added_edited_and_moved() {
        let mut app = app(VARIABLES);
        app.edit(Msg::VarAdded);
        app.edit(Msg::VarValueChanged(2, "three".into()));
        app.edit(Msg::VarMoved { index: 2, up: true });

        let names: Vec<String> = settings(&app)
            .variables
            .iter()
            .filter_map(|entry| match entry {
                VarEntry::Var(var) => Some(format!("{}={}", var.name, var.value)),
                VarEntry::Comment(_) => None,
            })
            .collect();
        assert_eq!(names, ["a=1", "variable=three", "b=%a%2"]);
    }

    #[test]
    fn a_tag_names_its_target_and_back() {
        let settings = Settings::load_str(
            r#"<y-pop><group name="A"/></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
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
