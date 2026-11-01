//! The Search screen: how the search opens, what its empty box shows, how
//! results look, what else it searches, and what a result can do.
//!
//! Every edit is a [`SearchMsg`] settled by [`apply`], a plain function on
//! `Settings` that calls [`crate::search_edit`] — tested without a window.
//! The view only maps `Settings` and [`SearchUi`] to controls.
//!
//! **The page follows what y-pop does, not the shape of the file.** Children of
//! `<search>` are one list in the XML and three things to y-pop: Recent is
//! always on top, the tray menus and groups are the BROWSE rows, in order, and
//! folders, plugins and the Start menu are never rows. Only BROWSE can be
//! reordered here, because only there would it show.
//!
//! **What is shown is what y-pop will use**, defaults included, and the first
//! edit of a list writes it out whole, so changing one thing never switches
//! the others off (see `search_edit`'s module doc). Whether something is
//! written is the file's business and is not said here.
//!
//! Keys are chosen in [`crate::key_edit`]'s dialog, never typed.

use y_pop::model::ActionId;
use y_pop::settings::{ItemId, MenusMode, MonitorMode, ResultsMode, Settings};

use crate::key_edit::{KeyMsg, KeyTarget, hotkey_labels, hotkey_warning, shortcut_labels};
use crate::search_edit::{self as se, SectionAdd, SectionKey};
use crate::tree::NewKind;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg, roots};
use windows_reactor::*;

/// What the screen remembers between renders.
#[derive(Clone, Default, Debug)]
pub struct SearchUi {
    /// The section whose fields are open under its row.
    pub selected: Option<SectionKey>,
    /// A search-only group or folder to open next; taken by `edit_search`.
    pub goto: Option<ItemId>,
    /// "Remove all of it?" is waiting for an answer.
    pub confirming_off: bool,
}

#[derive(Clone, Debug)]
pub enum SearchMsg {
    /// Writes an empty `<search/>`: search is on, from the tray icons' right
    /// click menu.
    TurnOn,
    /// Asks first if that would lose work; if not, turns it off.
    AskTurnOff,
    KeepOn,
    /// Removes `<search>` and all in it (undoable).
    TurnOff,

    RemoveHotkey,
    Override(bool),
    Collapsed(bool),
    /// 0 the cursor's, 1 the focused app's, 2 the main one.
    Monitor(usize),
    /// 0 sections, 1 flat.
    Results(usize),
    RowActions(bool),
    History(bool),

    /// Opens a section's fields, or closes them if open.
    Select(SectionKey),
    Recent(bool),
    StartMenu(bool),
    Remove(SectionKey),
    Move(SectionKey, bool),
    ResetSections,
    /// Adds a search-only group, or a browsed folder, and opens its page.
    AddGroup,
    AddFolder,
    /// Opens the page of a search-only group or folder.
    Open(ItemId),
    RecentCount(Option<f64>),
    /// 0 columns, 1 a-z, 2 list.
    Browse(usize),
    /// Counts the tray menu of this name, or stops counting it.
    Include(String),
    StartExclude(String),

    AddAction(ActionId),
    RemoveAction(usize),
    MoveAction(usize, bool),
    ClearActionKey(usize),
    ActionButton(usize, bool),
    ResetMenuKey,
    QuickPick(bool),
    ResetActions,
}

impl SearchMsg {
    /// What Undo should offer back, for the edits that remove something.
    pub fn undo_label(&self) -> Option<&'static str> {
        match self {
            Self::TurnOff | Self::AskTurnOff => Some("Turned search off"),
            Self::Remove(_) | Self::Recent(false) | Self::StartMenu(false) => {
                Some("Removed a section")
            }
            Self::ResetSections => Some("Reset the sections"),
            Self::RemoveAction(_) => Some("Removed an action"),
            Self::ResetActions => Some("Reset the actions"),
            _ => None,
        }
    }
}

/// The names of the tray menus that can be listed in `include`.
pub fn menu_names(settings: &Settings) -> Vec<String> {
    roots::menus(settings)
        .filter_map(|item| item.name.clone())
        .filter(|name| !name.is_empty())
        .collect()
}

/// The tray menus the section counts, by name.
pub fn counted_menus(settings: &Settings) -> Vec<String> {
    let all = menu_names(settings);
    let listed = se::include_names(settings);
    if listed.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|menu| {
            listed
                .iter()
                .any(|name| y_pop::model::same_menu(name, menu))
        })
        .collect()
}

/// What turning search off would lose that isn't a matter of a few clicks, in
/// words; `None` if nothing is.
pub fn lost_by_turning_off(settings: &Settings) -> Option<String> {
    let items = crate::search_only::roots(settings).count();
    let items = match items {
        0 => None,
        1 => Some("a search-only group or folder".to_owned()),
        n => Some(format!("{n} search-only groups and folders")),
    };
    let actions = se::actions(settings)
        .written
        .then(|| "your actions".to_owned());
    match (items, actions) {
        (Some(items), Some(actions)) => Some(format!("{items} and {actions}")),
        (items, actions) => items.or(actions),
    }
}

/// Settles `message`; whether the settings changed.
pub fn apply(settings: &mut Settings, ui: &mut SearchUi, message: SearchMsg) -> bool {
    match message {
        SearchMsg::TurnOn => {
            let was = se::is_on(settings);
            se::ensure(settings);
            !was
        }
        SearchMsg::AskTurnOff if lost_by_turning_off(settings).is_some() => {
            ui.confirming_off = true;
            false
        }
        SearchMsg::KeepOn => {
            ui.confirming_off = false;
            false
        }
        SearchMsg::AskTurnOff | SearchMsg::TurnOff => {
            *ui = SearchUi::default();
            se::turn_off(settings).is_some()
        }

        SearchMsg::RemoveHotkey => se::set_hotkey(settings, ""),
        SearchMsg::Override(on) => se::set_override(settings, on),
        SearchMsg::Collapsed(on) => flag(settings, on, false, |s| s.collapsed, se::set_collapsed),
        SearchMsg::RowActions(on) => {
            flag(settings, on, true, |s| s.row_actions, se::set_row_actions)
        }
        SearchMsg::History(on) => flag(settings, on, true, |s| s.history, se::set_history),
        SearchMsg::Monitor(choice) => {
            let mode = match choice {
                1 => Some(MonitorMode::Focus),
                2 => Some(MonitorMode::Primary),
                _ => None,
            };
            let current = settings.search.as_ref().and_then(|s| s.monitor);
            // `cursor` written out and not written are the same choice.
            let same = current == mode || (mode.is_none() && current == Some(MonitorMode::Cursor));
            !same && se::set_monitor(settings, mode)
        }
        SearchMsg::Results(choice) => {
            let flat = choice == 1;
            let current = settings.search.as_ref().and_then(|s| s.results);
            if (current == Some(ResultsMode::Flat)) == flat {
                return false;
            }
            se::set_results(settings, flat.then_some(ResultsMode::Flat))
        }

        SearchMsg::Select(key) => {
            ui.selected = (ui.selected != Some(key)).then_some(key);
            false
        }
        SearchMsg::Recent(on) => {
            section_switch(settings, SectionKey::Recent, SectionAdd::Recent, on)
        }
        SearchMsg::StartMenu(on) => {
            section_switch(settings, SectionKey::StartMenu, SectionAdd::StartMenu, on)
        }
        SearchMsg::Remove(key) => {
            if ui.selected == Some(key) {
                ui.selected = None;
            }
            se::remove_section(settings, key).is_some()
        }
        SearchMsg::Move(key, up) => se::move_section(settings, key, up),
        SearchMsg::ResetSections => {
            ui.selected = None;
            se::reset_sections(settings)
        }
        SearchMsg::AddGroup => add_only(settings, ui, NewKind::Group),
        SearchMsg::AddFolder => add_only(settings, ui, NewKind::Folder),
        SearchMsg::Open(id) => {
            ui.goto = Some(id);
            false
        }
        SearchMsg::RecentCount(count) => {
            let count = count.map(|count| count.max(0.0) as u32);
            if count == Some(se::recent_count(settings)) {
                return false;
            }
            se::set_recent_count(settings, count)
        }
        SearchMsg::Browse(choice) => {
            let mode = match choice {
                1 => Some(MenusMode::AZ),
                2 => Some(MenusMode::List),
                _ => None,
            };
            // `columns` written out and not written are the same choice.
            let same = se::menus_browse(settings) == mode
                || (mode.is_none() && se::menus_browse(settings) == Some(MenusMode::Columns));
            !same && se::set_menus_browse(settings, mode)
        }
        SearchMsg::Include(name) => {
            // A menu with a `;` can't be listed: "every menu" is the only way
            // to count it, so it isn't part of what is chosen among.
            let usable = |name: &String| !name.contains(';');
            let all: Vec<String> = menu_names(settings).into_iter().filter(usable).collect();
            let mut counted: Vec<String> =
                counted_menus(settings).into_iter().filter(usable).collect();
            match counted.iter().position(|menu| *menu == name) {
                // The last one stays: no `include` means every menu.
                Some(_) if counted.len() == 1 => return false,
                Some(at) => {
                    counted.remove(at);
                }
                None => counted.push(name),
            }
            se::set_include(settings, &counted, &all).unwrap_or(false)
        }
        SearchMsg::StartExclude(text) => se::set_start_exclude(settings, &text),

        SearchMsg::AddAction(id) => se::add_action(settings, id),
        SearchMsg::RemoveAction(index) => se::remove_action(settings, index),
        SearchMsg::MoveAction(index, up) => se::move_action(settings, index, up),
        SearchMsg::ClearActionKey(index) => se::set_action_key(settings, index, ""),
        SearchMsg::ActionButton(index, on) => se::set_action_button(settings, index, on),
        SearchMsg::ResetMenuKey => se::set_menu_key(settings, ""),
        SearchMsg::QuickPick(on) => se::set_quick_pick(settings, on),
        SearchMsg::ResetActions => se::reset_actions(settings),
    }
}

/// A section that is simply there or not. A switch reports its value when it
/// first shows, so one that is already so does nothing.
fn section_switch(settings: &mut Settings, key: SectionKey, add: SectionAdd, on: bool) -> bool {
    if se::sections(settings).rows.iter().any(|row| row.key == key) == on {
        return false;
    }
    if on {
        se::add_section(settings, add)
    } else {
        se::remove_section(settings, key).is_some()
    }
}

fn add_only(settings: &mut Settings, ui: &mut SearchUi, kind: NewKind) -> bool {
    let added = crate::search_only::add(settings, kind);
    ui.goto = added;
    added.is_some()
}

/// A true/false attribute: a value equal to the default is not written.
fn flag(
    settings: &mut Settings,
    on: bool,
    default: bool,
    get: fn(&y_pop::settings::SearchSettings) -> Option<bool>,
    set: fn(&mut Settings, Option<bool>) -> bool,
) -> bool {
    let current = settings.search.as_ref().and_then(get).unwrap_or(default);
    current != on && set(settings, (on != default).then_some(on))
}

// ---- words -------------------------------------------------------------

/// What a row is called.
pub fn section_title(settings: &Settings, key: SectionKey) -> String {
    match key {
        SectionKey::Recent => "Recent".to_owned(),
        SectionKey::Menus => "Tray menus".to_owned(),
        SectionKey::StartMenu => "Start menu".to_owned(),
        SectionKey::Plugin(n) => {
            plugin_attr(settings, n, "name").unwrap_or_else(|| "Plugin".to_owned())
        }
        SectionKey::Item(id) => crate::tree::item(settings, id)
            .map(roots::label)
            .unwrap_or_default(),
    }
}

fn plugin_attr(settings: &Settings, nth: usize, attr: &str) -> Option<String> {
    se::plugin(settings, nth)?
        .iter()
        .find(|(name, _)| name == attr)
        .map(|(_, value)| value.clone())
}

/// One line saying what a row of the BROWSE list or of "Also searched" does.
pub fn section_summary(settings: &Settings, key: SectionKey) -> String {
    match key {
        SectionKey::Recent | SectionKey::StartMenu => String::new(),
        SectionKey::Menus => {
            let how = match se::menus_browse(settings) {
                None | Some(MenusMode::Columns) => "columns",
                Some(MenusMode::AZ) => "A–Z",
                Some(MenusMode::List) => "list",
            };
            let all = menu_names(settings).len();
            let counted = counted_menus(settings).len();
            if counted == all {
                format!("Every tray menu, as {how}")
            } else {
                format!("{counted} of {all} tray menus, as {how}")
            }
        }
        SectionKey::Plugin(n) => match plugin_attr(settings, n, "src") {
            Some(src) => format!("Plugin {src}: kept, but not run yet"),
            None => "A plugin needs a name and a src: y-pop ignores this one".to_owned(),
        },
        SectionKey::Item(id) => match crate::tree::item(settings, id).map(|item| &item.kind) {
            Some(y_pop::settings::ItemKind::App(app)) if app.browse => {
                "A folder: its files are found with /".to_owned()
            }
            Some(y_pop::settings::ItemKind::App(_)) => {
                "Not a browsed folder: y-pop ignores it here".to_owned()
            }
            Some(_) => match crate::tree::count_inside(settings, id) {
                0 => "Empty: y-pop lists it once it holds something".to_owned(),
                inside => format!("A group of {inside}"),
            },
            None => String::new(),
        },
    }
}

/// The label of an action row: its name, or the id as written if unknown.
fn action_title(row: &se::ActionRow) -> String {
    match ActionId::parse(&row.id) {
        Some(id) if !row.ignored => id.label().to_owned(),
        Some(id) => format!("{} (listed twice, ignored)", id.label()),
        None => format!("{} (unknown, ignored)", row.id),
    }
}

// ---- the view ----------------------------------------------------------

const BROWSE_COLUMNS: [Column; 3] = [
    Column::flex("Row", 130.0),
    Column::flex("Shows", 200.0),
    Column::actions(FOUR_ICON_BUTTONS),
];

const OTHER_COLUMNS: [Column; 3] = [
    Column::flex("Name", 130.0),
    Column::flex("What", 200.0),
    Column::actions(TWO_ICON_BUTTONS),
];

const ACTION_COLUMNS: [Column; 4] = [
    Column::flex("Action", 130.0),
    // Room for `Ctrl + Shift + Enter` on its button, and the ✕.
    Column::flex("Shortcut", 230.0),
    Column::fixed("Button", 64.0),
    Column::actions(THREE_ICON_BUTTONS),
];

pub fn search_view(
    settings: &Settings,
    ui: &SearchUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let lede = "The search window: opened by a hotkey or from the right-click menu of a tray icon, it finds \
                your tray menu entries, recent files and more.";
    if !se::is_on(settings) {
        return page(
            "Search",
            lede,
            Stack::vertical(space::L)
                .child(
                    "off",
                    note("Search is off. Turn it on to give it a hotkey and choose what it lists."),
                )
                .child(
                    "on",
                    button(
                        "Turn search on",
                        true,
                        true,
                        context.callback(|()| Msg::Search(SearchMsg::TurnOn)),
                    ),
                ),
        );
    }

    page(
        "Search",
        lede,
        Stack::vertical(space::XL)
            .child("opening", opening(settings, viewport, context))
            .child("empty-box", empty_box(settings, ui, viewport, context))
            .child("results", results(settings, viewport, context))
            .child("also", also_searched(settings, viewport, context))
            .child("actions", actions_card(settings, viewport, context))
            .child("off", turn_off(settings, ui, context)),
    )
}

fn search_callback<T: 'static>(
    context: &ViewContext<ConfigApp>,
    make: impl Fn(T) -> SearchMsg + 'static,
) -> Callback<T> {
    context.callback(move |value| Msg::Search(make(value)))
}

fn key_callback(context: &ViewContext<ConfigApp>, target: KeyTarget) -> Callback<()> {
    context.callback(move |()| Msg::Key(KeyMsg::Open(target)))
}

/// A key as keycaps; text that isn't one as itself, so nothing is hidden.
fn shown_key(labels: &[String], text: &str, none: &str) -> View {
    if !labels.is_empty() {
        keycaps(labels, false)
    } else if text.trim().is_empty() {
        text::caption(none.to_owned()).into()
    } else {
        text::caption(text.to_owned()).into()
    }
}

fn hotkey_control(settings: &Settings, context: &ViewContext<ConfigApp>) -> View {
    let search = settings.search.clone().unwrap_or_default();
    let hotkey = search.hotkey.clone().unwrap_or_default();
    let set = !hotkey.trim().is_empty();
    let labels = hotkey_labels(&hotkey);

    let line = key_field(
        shown_key(&labels, &hotkey, "Choose a key…"),
        "Choose the key combination",
        key_callback(context, KeyTarget::SearchHotkey),
        Some((
            "Remove the hotkey",
            set,
            search_callback(context, |()| SearchMsg::RemoveHotkey),
        )),
    );

    let mut control = Stack::vertical(space::XS).child("line", line);
    if set && labels.is_empty() {
        control = control.child(
            "invalid",
            text::caption("Not a key combination: y-pop ignores it."),
        );
    }
    if let Some(warning) = hotkey_warning(settings, &hotkey, KeyTarget::SearchHotkey) {
        control = control.child("warning", text::caption(warning));
    }
    if set {
        control = control.child(
            "override",
            labeled_switch(
                "Take it even if another program holds it",
                search.can_override,
                search_callback(context, SearchMsg::Override),
            ),
        );
    }
    control.into()
}

fn opening(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let search = settings.search.clone().unwrap_or_default();
    let cards = Stack::vertical(space::M)
        .child(
            "hotkey",
            setting(
                "Hotkey",
                "Opens the search from anywhere.",
                hotkey_control(settings, context),
                viewport,
            ),
        )
        .child(
            "monitor",
            setting(
                "Monitor",
                "Where the search appears. With Focused app, on the desktop or from a tray icon it follows the cursor.",
                segmented(
                    &[
                        Choice::new("Cursor"),
                        Choice::new("Focused app"),
                        Choice::new("Main monitor"),
                    ],
                    Some(match search.monitor {
                        Some(MonitorMode::Focus) => 1,
                        Some(MonitorMode::Primary) => 2,
                        Some(MonitorMode::Cursor) | None => 0,
                    }),
                    search_callback(context, SearchMsg::Monitor),
                ),
                viewport,
            ),
        )
        .child(
            "collapsed",
            setting(
                "Open collapsed",
                "Open as the input row alone, with a button to expand the list.",
                switch(
                    search.collapsed.unwrap_or(false),
                    search_callback(context, SearchMsg::Collapsed),
                ),
                viewport,
            ),
        );
    section("Opening", cards)
}

fn empty_box(
    settings: &Settings,
    ui: &SearchUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let rows = se::browse_rows(settings);
    let table = Table::new(&BROWSE_COLUMNS);
    let detail_viewport = table.detail_viewport(viewport).inset(CARD_INSET);
    let count = rows.len();

    let rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let key = row.key;
            let moves = move_buttons(
                index,
                count,
                search_callback(context, move |()| SearchMsg::Move(key, true)),
                search_callback(context, move |()| SearchMsg::Move(key, false)),
            );
            let mut actions = Stack::horizontal(space::XS).child("move", moves);
            match key {
                SectionKey::Item(id) => {
                    actions = actions
                        .child(
                            "edit",
                            icon_button(
                                Symbol::Edit,
                                "Open this group",
                                search_callback(context, move |()| SearchMsg::Open(id)),
                            ),
                        )
                        .child(
                            "remove",
                            icon_button(
                                Symbol::Delete,
                                "Remove this group and what is in it",
                                search_callback(context, move |()| SearchMsg::Remove(key)),
                            ),
                        );
                }
                _ => {
                    actions = actions.child(
                        "edit",
                        icon_button(
                            Symbol::Edit,
                            "Choose how it opens and which tray menus count",
                            search_callback(context, move |()| SearchMsg::Select(key)),
                        ),
                    );
                }
            }
            let detail = (ui.selected == Some(key)).then(|| {
                panel(
                    section_title(settings, key),
                    None,
                    menus_fields(settings, detail_viewport, context),
                )
            });
            let shows: View = text::single_line(section_summary(settings, key))
                .opacity(emphasis::MUTED)
                .into();
            Row::new(
                format!("{key:?}"),
                vec![
                    text::body(section_title(settings, key)).into(),
                    shows,
                    actions.into(),
                ],
            )
            .detail(detail)
        })
        .collect();

    let mut foot = vec![(
        "Add group",
        add_button(
            "Add group",
            search_callback(context, |()| SearchMsg::AddGroup),
        ),
    )];
    if se::sections_customised(settings) {
        foot.push((
            "Reset",
            button(
                "Back to the default sections",
                false,
                true,
                search_callback(context, |()| SearchMsg::ResetSections),
            ),
        ));
    }

    let body = Stack::vertical(space::M)
        .child("recent", recent_setting(settings, viewport, context))
        .child(
            "browse",
            field("Browse list", table.rows(rows).build(viewport)),
        )
        .child(
            "foot",
            AutoGrid::new(220.0)
                .max_columns(2)
                .items(foot)
                .build(viewport),
        );
    section("Empty box", body)
}

/// A label and its control on one line, for a setting whose switch is beside
/// them: on the switch's line, a card stays as low as the others.
fn beside(label: &str, control: impl Into<View>) -> View {
    Stack::horizontal(space::S)
        .child(
            "label",
            text::caption(label).vertical_alignment(VerticalAlignment::Center),
        )
        .child("control", control)
        .into()
}

/// Recent is on top of the box or isn't there. History feeds it, so without
/// history it is greyed out, whatever the file says.
fn recent_setting(
    settings: &Settings,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let history = settings
        .search
        .as_ref()
        .and_then(|s| s.history)
        .unwrap_or(true);
    let on = se::sections(settings)
        .rows
        .iter()
        .any(|row| row.key == SectionKey::Recent);

    let mut control = Stack::horizontal(space::M);
    if on && history {
        let count = se::recent_count(settings);
        // The file may hold more than the box usually offers: a box coerces a
        // value over its maximum, which would be an edit nobody asked for.
        let maximum = f64::from(count.max(RECENT_MAX));
        control = control.child(
            "count",
            beside(
                "How many",
                number_box(
                    Some(f64::from(count)),
                    0.0,
                    maximum,
                    search_callback(context, SearchMsg::RecentCount),
                )
                .width(size::INLINE_NUMBER),
            ),
        );
    }
    let control = control.child(
        "switch",
        switch_enabled(on, history, search_callback(context, SearchMsg::Recent)),
    );
    let what = if history {
        "The last things you opened, above the rest. At 0, only a link to all of them."
    } else {
        "Needs Windows history, which is off under Results."
    };
    setting("Recent", what, control, viewport)
}

const RECENT_MAX: u32 = 100;

fn results(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let search = settings.search.clone().unwrap_or_default();
    let cards = Stack::vertical(space::M)
        .child(
            "layout",
            setting(
                "Results",
                "Sections: the best hit, then groups of at most five. Flat: one list in score order.",
                segmented(
                    &[Choice::new("Sections"), Choice::new("Flat")],
                    Some(usize::from(search.results == Some(ResultsMode::Flat))),
                    search_callback(context, SearchMsg::Results),
                ),
                viewport,
            ),
        )
        .child(
            "row-actions",
            setting(
                "Buttons on the selected result",
                "Show the actions marked as buttons at the end of the selected row. Their keys work either way.",
                switch(
                    search.row_actions.unwrap_or(true),
                    search_callback(context, SearchMsg::RowActions),
                ),
                viewport,
            ),
        )
        .child(
            "history",
            setting(
                "Use Windows history",
                "Remember what you launch, list it as Recent and let it rank results. Off: none of that.",
                switch(
                    search.history.unwrap_or(true),
                    search_callback(context, SearchMsg::History),
                ),
                viewport,
            ),
        );
    section("Results", cards)
}

/// What is searched without being a row of the box.
fn also_searched(
    settings: &Settings,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let rows: Vec<Row> = se::other_rows(settings)
        .iter()
        .map(|row| {
            let key = row.key;
            let mut actions = Stack::horizontal(space::XS);
            if let (SectionKey::Item(id), false) = (key, row.ignored) {
                actions = actions.child(
                    "edit",
                    icon_button(
                        Symbol::Edit,
                        "Open this folder",
                        search_callback(context, move |()| SearchMsg::Open(id)),
                    ),
                );
            }
            actions = actions.child(
                "remove",
                icon_button(
                    Symbol::Delete,
                    "Remove it",
                    search_callback(context, move |()| SearchMsg::Remove(key)),
                ),
            );
            let what: View = text::single_line(section_summary(settings, key))
                .opacity(emphasis::MUTED)
                .into();
            Row::new(
                format!("{key:?}"),
                vec![
                    text::body(section_title(settings, key)).into(),
                    what,
                    actions.into(),
                ],
            )
        })
        .collect();
    let table = Table::new(&OTHER_COLUMNS)
        .empty_message("No folders here: your tray menus' are searched anyway.");

    let start_on = se::sections(settings)
        .rows
        .iter()
        .any(|row| row.key == SectionKey::StartMenu);
    let mut start = Stack::horizontal(space::M);
    if start_on {
        start = start.child(
            "exclude",
            beside(
                "Leave out",
                text_box(
                    se::start_exclude(settings),
                    "*uninstall*;*readme*",
                    search_callback(context, SearchMsg::StartExclude),
                )
                .width(size::INLINE_TEXT),
            ),
        );
    }
    let start = start.child(
        "switch",
        switch(start_on, search_callback(context, SearchMsg::StartMenu)),
    );

    let body = Stack::vertical(space::M)
        .child("table", table.rows(rows).build(viewport))
        .child(
            "add",
            AutoGrid::new(150.0)
                .max_columns(4)
                .keep_columns()
                .items([(
                    "Add folder",
                    add_button(
                        "Add folder",
                        search_callback(context, |()| SearchMsg::AddFolder),
                    ),
                )])
                .build(viewport),
        )
        .child(
            "start",
            setting(
                "Start menu",
                "Not used yet: y-pop doesn't list the Start menu. Patterns are separated by ;.",
                start,
                viewport,
            ),
        );
    section("Also searched", body)
}

fn menus_fields(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let browse = match se::menus_browse(settings) {
        None | Some(MenusMode::Columns) => 0,
        Some(MenusMode::AZ) => 1,
        Some(MenusMode::List) => 2,
    };
    let opens = field(
        "Opens as",
        combo(
            ["Columns", "A–Z", "List"],
            Some(browse),
            search_callback(context, |choice: Option<usize>| {
                SearchMsg::Browse(choice.unwrap_or(0))
            }),
        ),
    );

    let counted = counted_menus(settings);
    let usable_counted = counted.iter().filter(|name| !name.contains(';')).count();
    let menus = menu_names(settings)
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let on = counted.contains(&name);
            let usable = !name.contains(';');
            let last = on && usable_counted == 1;
            let shown = roots::mnemonic(&name);
            // A selected look, not a toggle: see the toolkit's trap table.
            let chosen = button(
                &shown,
                on,
                usable && !last,
                search_callback(context, move |()| SearchMsg::Include(name.clone())),
            );
            (index.to_string(), chosen)
        });

    let mut counted_box =
        Stack::vertical(space::S).child("menus", AutoGrid::new(150.0).items(menus).build(viewport));
    let unknown = se::unknown_includes(settings);
    if !unknown.is_empty() {
        counted_box = counted_box.child(
            "unknown",
            text::caption(format!(
                "No tray menu is called {}: y-pop warns about it at start-up.",
                unknown.join(", ")
            )),
        );
    }
    if menu_names(settings).iter().any(|name| name.contains(';')) {
        counted_box = counted_box.child(
            "semicolon",
            text::caption(
                "A menu whose name has a ; can't be chosen: it counts only while all of them do.",
            ),
        );
    }
    counted_box = counted_box.child(
        "help",
        text::caption("The highlighted menus are in this section and in plain search. All of them is the default; the others still work in the tray."),
    );

    Stack::vertical(space::M)
        .child("opens", opens)
        .child("counted", field("Tray menus counted", counted_box))
        .into()
}

fn shortcut_cell(index: usize, row: &se::ActionRow, context: &ViewContext<ConfigApp>) -> View {
    let text = row.key.clone().unwrap_or_default();
    let shown = shown_key(&shortcut_labels(&text), &text, "None");
    if row.ignored {
        return shown;
    }
    key_field(
        shown,
        "Choose a shortcut",
        key_callback(context, KeyTarget::Action(index)),
        Some((
            "No shortcut",
            row.key.is_some(),
            search_callback(context, move |()| SearchMsg::ClearActionKey(index)),
        )),
    )
}

fn menu_key_control(view: &se::ActionsView, context: &ViewContext<ConfigApp>) -> View {
    let default = y_pop::model::ActionsConfig::default().menu_key.to_string();
    key_field(
        shown_key(&shortcut_labels(&view.menu_key), &view.menu_key, "None"),
        "Choose the key",
        key_callback(context, KeyTarget::MenuKey),
        Some((
            "Back to the usual key",
            view.menu_key != default,
            search_callback(context, |()| SearchMsg::ResetMenuKey),
        )),
    )
}

fn actions_card(settings: &Settings, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let view = se::actions(settings);
    let problems = se::key_problems(settings);
    let count = view.rows.len();
    let table = Table::new(&ACTION_COLUMNS);
    let buttons_shown = settings
        .search
        .as_ref()
        .and_then(|s| s.row_actions)
        .unwrap_or(true);

    let rows: Vec<Row> = view
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let is_open = row.id == ActionId::Open.name() && !row.ignored;
            let button_switch = switch_enabled(
                row.button,
                buttons_shown && !row.ignored,
                search_callback(context, move |on| SearchMsg::ActionButton(index, on)),
            );
            let actions: View = Stack::horizontal(space::XS)
                .child(
                    "move",
                    move_buttons(
                        index,
                        count,
                        search_callback(context, move |()| SearchMsg::MoveAction(index, true)),
                        search_callback(context, move |()| SearchMsg::MoveAction(index, false)),
                    ),
                )
                .child(
                    "remove",
                    icon_button_enabled(
                        Symbol::Delete,
                        if is_open {
                            "Open is always there: y-pop would put it back"
                        } else {
                            "Remove this action"
                        },
                        !is_open,
                        search_callback(context, move |()| SearchMsg::RemoveAction(index)),
                    ),
                )
                .into();
            let problem = problems
                .iter()
                .find(|(at, _)| *at == Some(index))
                .map(|(_, text)| {
                    text::caption(format!("y-pop ignores this shortcut: {text}")).into()
                });
            // An ignored row can repeat an id, so it goes by its place.
            let key = if row.ignored {
                format!("#{index}")
            } else {
                row.id.clone()
            };
            Row::new(
                key,
                vec![
                    text::body(action_title(row)).into(),
                    shortcut_cell(index, row, context),
                    button_switch,
                    actions,
                ],
            )
            .detail(problem)
        })
        .collect();

    let mut menu_key = Stack::vertical(space::XS).child("key", menu_key_control(&view, context));
    if let Some((_, problem)) = problems.iter().find(|(at, _)| at.is_none()) {
        menu_key = menu_key.child(
            "problem",
            text::caption(format!("y-pop uses Alt+Enter instead: {problem}")),
        );
    }
    let settings_grid = AutoGrid::new(220.0)
        .max_columns(2)
        .items([
            (
                "menu-key",
                field("Key that opens the action menu", menu_key),
            ),
            (
                "quick-pick",
                labeled_switch(
                    "Alt+1…9 open the n-th result",
                    view.quick_pick,
                    search_callback(context, SearchMsg::QuickPick),
                ),
            ),
        ])
        .build(viewport);

    let mut body = Stack::vertical(space::M).child("settings", settings_grid);
    if se::enter_is_unused(settings) {
        body = body.child(
            "no-enter",
            note("Nothing is on Enter: pressing it on a result does nothing."),
        );
    }
    if !buttons_shown {
        body = body.child(
            "no-buttons",
            text::caption("Buttons on the selected result are off under Results, so the Button column has no effect."),
        );
    }
    body = body.child("table", table.rows(rows).build(viewport));

    let addable = se::addable_actions(settings);
    if !addable.is_empty() {
        let buttons = addable.into_iter().map(|id| {
            (
                id.name(),
                add_button(
                    &format!("Add {}", id.short_label()),
                    search_callback(context, move |()| SearchMsg::AddAction(id)),
                ),
            )
        });
        body = body.child("add", AutoGrid::new(150.0).items(buttons).build(viewport));
    }
    if view.written {
        body = body.child(
            "reset",
            button(
                "Back to the default actions",
                false,
                true,
                search_callback(context, |()| SearchMsg::ResetActions),
            ),
        );
    }
    section("Actions", body)
}

fn turn_off(settings: &Settings, ui: &SearchUi, context: &ViewContext<ConfigApp>) -> View {
    let ask = button(
        "Turn search off",
        false,
        true,
        search_callback(context, |()| SearchMsg::AskTurnOff),
    );
    let lost = lost_by_turning_off(settings).filter(|_| ui.confirming_off);
    let Some(lost) = lost else {
        return ask;
    };
    Stack::vertical(space::S)
        .child(
            "ask",
            text::body(format!("Turning search off also removes {lost}.")),
        )
        .child(
            "buttons",
            Stack::horizontal(space::S)
                .child(
                    "yes",
                    button(
                        "Turn it off",
                        true,
                        true,
                        search_callback(context, |()| SearchMsg::TurnOff),
                    ),
                )
                .child(
                    "no",
                    button(
                        "Keep it",
                        false,
                        true,
                        search_callback(context, |()| SearchMsg::KeepOn),
                    ),
                ),
        )
        .into()
}

impl ConfigApp {
    /// Settles a message of the Search screen and keeps what Undo needs.
    pub(crate) fn edit_search(&mut self, message: SearchMsg) {
        let Ok(settings) = &mut self.settings else {
            return;
        };
        self.run_status = None;
        let undo = message.undo_label().map(|label| (label, settings.clone()));
        if apply(settings, &mut self.search_ui, message) {
            if let Some((label, before)) = undo {
                self.undo = Some(crate::Undo {
                    label: label.to_owned(),
                    settings: before,
                });
            }
            self.dirty = true;
        }
        if let Some(id) = self.search_ui.goto.take() {
            self.go(crate::Screen::SearchOnly(id));
        }
    }
}

#[cfg(test)]
mod tests;
