//! The `<search>` element as plain functions on [`Settings`], no UI: its
//! attributes, its sections and its actions. Tested without a window, like
//! [`crate::roots`] and [`crate::tree`].
//!
//! **What y-pop will do, not what the file says.** [`sections`] and
//! [`actions`] answer with the list y-pop ends up with, defaults and implied
//! entries included, flagged `written: false`. The rules they mirror are the
//! reader's (`y_pop::config::reader::read_search`):
//!
//! * no section written (only `<actions>`, comments, or nothing): the three
//!   defaults, `<recent/>`, `<menus/>`, `<start-menu/>`;
//! * any section or search-only item written: exactly that, plus `<menus/>`
//!   if it is missing (after `<recent>`, else first);
//! * no `<actions>`: the default actions; `<actions>` written: exactly those,
//!   plus `open` first if missing.
//!
//! So the first edit to a list that was only implied **writes it out first**
//! (`customize`): otherwise "add a section" would silently switch the other
//! defaults off, and "reorder an action" would drop the others.
//!
//! Not every row of that list is alike to y-pop: see [`browse_rows`].
//!
//! An empty `<search/>` is not nothing: it turns search on (right-click on a
//! tray icon offers it). So `<search>` is created by the first real edit and
//! removed only by [`turn_off`], never because it became empty.

use y_pop::model::{ActionId, ActionsConfig, same_menu};
use y_pop::search::keys::Chord;
use y_pop::settings::{
    ActionEntry, ActionItem, ItemId, ItemKind, MenusMode, MonitorMode, ResultsMode, SearchEntry,
    SearchSection, SearchSettings, SectionKind, Settings,
};

/// Sets `slot` to `new`; whether that changed it.
fn put<T: PartialEq>(slot: &mut T, new: T) -> bool {
    if *slot == new {
        return false;
    }
    *slot = new;
    true
}

fn absent_when_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

// ---- the element -----------------------------------------------------

/// Whether the file has a `<search>`.
pub fn is_on(settings: &Settings) -> bool {
    settings.search.is_some()
}

/// Removes `<search>` with everything in it, and hands it back (for Undo).
pub fn turn_off(settings: &mut Settings) -> Option<SearchSettings> {
    settings.search.take()
}

pub fn ensure(settings: &mut Settings) -> &mut SearchSettings {
    settings.search.get_or_insert_with(SearchSettings::default)
}

/// Runs an attribute edit, creating `<search>` only if it changed something.
fn edit(settings: &mut Settings, change: impl FnOnce(&mut SearchSettings) -> bool) -> bool {
    let existed = settings.search.is_some();
    let changed = change(ensure(settings));
    if !changed && !existed {
        settings.search = None;
    }
    changed
}

// ---- the attributes --------------------------------------------------

/// `Alt+Space`, raw; empty removes the attribute, and `override` with it:
/// there is nothing left to override for.
pub fn set_hotkey(settings: &mut Settings, text: &str) -> bool {
    edit(settings, |search| {
        let new = absent_when_empty(text);
        let can_override = new.is_some() && search.can_override;
        let dropped = put(&mut search.can_override, can_override);
        put(&mut search.hotkey, new) | dropped
    })
}

pub fn set_override(settings: &mut Settings, on: bool) -> bool {
    edit(settings, |search| put(&mut search.can_override, on))
}

pub fn set_collapsed(settings: &mut Settings, value: Option<bool>) -> bool {
    edit(settings, |search| put(&mut search.collapsed, value))
}

pub fn set_monitor(settings: &mut Settings, value: Option<MonitorMode>) -> bool {
    edit(settings, |search| put(&mut search.monitor, value))
}

pub fn set_results(settings: &mut Settings, value: Option<ResultsMode>) -> bool {
    edit(settings, |search| put(&mut search.results, value))
}

pub fn set_row_actions(settings: &mut Settings, value: Option<bool>) -> bool {
    edit(settings, |search| put(&mut search.row_actions, value))
}

pub fn set_history(settings: &mut Settings, value: Option<bool>) -> bool {
    edit(settings, |search| put(&mut search.history, value))
}

// ---- the sections ----------------------------------------------------

/// Which entry of the sections list. At most one of each kind but plugins,
/// which are told apart by their order among plugins.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionKey {
    Recent,
    Menus,
    StartMenu,
    /// The n-th `<plugin>`.
    Plugin(usize),
    /// A search-only group or folder.
    Item(ItemId),
}

/// One line of the sections list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub key: SectionKey,
    /// In the file. `false`: y-pop supplies it (a default, or the `<menus/>`
    /// it adds when it is missing).
    pub written: bool,
    /// In the file, but y-pop warns and drops it: an `<app>` that is no
    /// browsed folder, a `<plugin>` without a name or a `src`. All that can
    /// be done to one is remove it.
    pub ignored: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sections {
    pub rows: Vec<Row>,
    /// Nothing is written: this is the default list.
    pub defaults: bool,
}

/// Whether a child of `<search>` is part of the sections list (an item or a
/// section, but not `<actions>`).
fn key_of(entry: &SearchEntry, plugins: &mut usize) -> Option<SectionKey> {
    match entry {
        SearchEntry::Comment(_) => None,
        SearchEntry::Item(item) => Some(SectionKey::Item(item.id)),
        SearchEntry::Section(section) => match section.kind {
            SectionKind::Recent { .. } => Some(SectionKey::Recent),
            SectionKind::Menus { .. } => Some(SectionKey::Menus),
            SectionKind::StartMenu { .. } => Some(SectionKey::StartMenu),
            SectionKind::Plugin => {
                *plugins += 1;
                Some(SectionKey::Plugin(*plugins - 1))
            }
            SectionKind::Actions { .. } => None,
        },
    }
}

/// The sections as y-pop will list them. A second `<recent>`, `<menus>` or
/// `<start-menu>` is ignored by y-pop, so it is not a row.
pub fn sections(settings: &Settings) -> Sections {
    let written: Vec<SectionKey> = settings.search.as_ref().map_or_else(Vec::new, |search| {
        let mut plugins = 0;
        let mut seen = Vec::new();
        for entry in &search.children {
            if let Some(key) = key_of(entry, &mut plugins)
                && !seen.contains(&key)
            {
                seen.push(key);
            }
        }
        seen
    });

    let row = |key, written| Row {
        key,
        written,
        ignored: is_ignored(settings, key),
    };
    if written.is_empty() {
        return Sections {
            rows: vec![
                row(SectionKey::Recent, false),
                row(SectionKey::Menus, false),
                row(SectionKey::StartMenu, false),
            ],
            defaults: true,
        };
    }
    let mut rows: Vec<Row> = written.iter().map(|&key| row(key, true)).collect();
    if !written.contains(&SectionKey::Menus) {
        let at = rows
            .iter()
            .position(|row| row.key == SectionKey::Recent)
            .map_or(0, |recent| recent + 1);
        rows.insert(at, row(SectionKey::Menus, false));
    }
    Sections {
        rows,
        defaults: false,
    }
}

/// Whether y-pop's reader drops this row of the file.
fn is_ignored(settings: &Settings, key: SectionKey) -> bool {
    match key {
        SectionKey::Item(id) => crate::search_only::find(settings, id)
            .is_some_and(|item| matches!(&item.kind, ItemKind::App(app) if !app.browse)),
        SectionKey::Plugin(nth) => plugin(settings, nth).is_some_and(|attrs| {
            ["name", "src"]
                .iter()
                .any(|name| attrs.iter().all(|(attr, _)| attr != name))
        }),
        _ => false,
    }
}

/// The attributes of the n-th `<plugin>`.
pub fn plugin(settings: &Settings, nth: usize) -> Option<&[(String, String)]> {
    settings
        .search
        .as_ref()?
        .children
        .iter()
        .filter_map(|entry| match entry {
            SearchEntry::Section(SearchSection {
                kind: SectionKind::Plugin,
                extra_attrs,
                ..
            }) => Some(extra_attrs.as_slice()),
            _ => None,
        })
        .nth(nth)
}

/// Whether y-pop shows this row under BROWSE, where order means something:
/// the tray menus and the search-only groups. Recent is always above them,
/// and folders, plugins and the Start menu are never rows.
fn in_browse(settings: &Settings, key: SectionKey) -> bool {
    match key {
        SectionKey::Menus => true,
        SectionKey::Item(id) => crate::search_only::find(settings, id)
            .is_some_and(|item| matches!(item.kind, ItemKind::Group)),
        _ => false,
    }
}

/// The rows of y-pop's BROWSE list, in order.
pub fn browse_rows(settings: &Settings) -> Vec<Row> {
    sections(settings)
        .rows
        .into_iter()
        .filter(|row| in_browse(settings, row.key))
        .collect()
}

/// What is searched without being a row: folders and plugins.
pub fn other_rows(settings: &Settings) -> Vec<Row> {
    sections(settings)
        .rows
        .into_iter()
        .filter(|row| matches!(row.key, SectionKey::Item(_) | SectionKey::Plugin(_)))
        .filter(|row| !in_browse(settings, row.key))
        .collect()
}

fn new_section(kind: SectionKind) -> SearchEntry {
    SearchEntry::Section(SearchSection {
        lead: Vec::new(),
        blank_before: false,
        kind,
        extra_attrs: Vec::new(),
    })
}

fn default_kind(key: SectionKey) -> Option<SectionKind> {
    match key {
        SectionKey::Recent => Some(SectionKind::Recent { count: None }),
        SectionKey::Menus => Some(SectionKind::Menus {
            browse: None,
            include: None,
        }),
        SectionKey::StartMenu => Some(SectionKind::StartMenu { exclude: None }),
        SectionKey::Plugin(_) | SectionKey::Item(_) => None,
    }
}

/// Writes the sections y-pop only implies, so that an edit to the list
/// changes the list and nothing else. Creates `<search>` if need be. Returns
/// whether anything was written.
pub fn customize(settings: &mut Settings) -> bool {
    let plan = sections(settings);
    if plan.rows.iter().all(|row| row.written) {
        return false;
    }
    let search = ensure(settings);
    let mut plugins = 0;
    let first = search
        .children
        .iter()
        .position(|entry| key_of(entry, &mut plugins).is_some());

    if plan.defaults {
        for (offset, row) in plan.rows.iter().enumerate() {
            if let Some(kind) = default_kind(row.key) {
                search.children.insert(offset, new_section(kind));
            }
        }
        return true;
    }
    // Only `<menus/>` can be implied here: just after `<recent>`, else first.
    let recent = search.children.iter().position(|entry| {
        matches!(
            entry,
            SearchEntry::Section(SearchSection {
                kind: SectionKind::Recent { .. },
                ..
            })
        )
    });
    let at = recent.map_or(first.unwrap_or(0), |recent| recent + 1);
    search.children.insert(
        at,
        new_section(default_kind(SectionKey::Menus).expect("menus")),
    );
    true
}

/// What can be added to the list: a plugin is never made here, and `<menus/>`
/// is always there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionAdd {
    Recent,
    StartMenu,
}

impl SectionAdd {
    fn key(self) -> SectionKey {
        match self {
            Self::Recent => SectionKey::Recent,
            Self::StartMenu => SectionKey::StartMenu,
        }
    }
}

/// Which sections the list does not have yet.
pub fn addable(settings: &Settings) -> Vec<SectionAdd> {
    let rows = sections(settings).rows;
    [SectionAdd::Recent, SectionAdd::StartMenu]
        .into_iter()
        .filter(|add| rows.iter().all(|row| row.key != add.key()))
        .collect()
}

/// Adds a section: `<recent/>` before the others, `<start-menu/>` after them.
pub fn add_section(settings: &mut Settings, add: SectionAdd) -> bool {
    if !addable(settings).contains(&add) {
        return false;
    }
    customize(settings);
    let search = ensure(settings);
    let mut plugins = 0;
    let listed: Vec<usize> = (0..search.children.len())
        .filter(|&i| key_of(&search.children[i], &mut plugins).is_some())
        .collect();
    let (at, kind) = match add {
        SectionAdd::Recent => (
            listed.first().copied().unwrap_or(0),
            SectionKind::Recent { count: None },
        ),
        SectionAdd::StartMenu => (
            listed.last().map_or(search.children.len(), |last| last + 1),
            SectionKind::StartMenu { exclude: None },
        ),
    };
    search.children.insert(at, new_section(kind));
    true
}

/// `<menus/>` stays: y-pop would put it back, and a row that comes back after
/// "Remove" is a button that lies.
pub fn can_remove(key: SectionKey) -> bool {
    key != SectionKey::Menus
}

/// Index among `<search>`'s children of the entry `key` names.
fn index_of(search: &SearchSettings, key: SectionKey) -> Option<usize> {
    let mut plugins = 0;
    (0..search.children.len()).find(|&i| key_of(&search.children[i], &mut plugins) == Some(key))
}

/// Removes a section, or a search-only item with all that is in it, and hands
/// the entry back, with any second `<recent>` y-pop ignored, which would
/// otherwise take its place. `<menus/>` stays, so the list never empties:
/// [`reset_sections`] is the way back to the defaults.
pub fn remove_section(settings: &mut Settings, key: SectionKey) -> Option<SearchEntry> {
    if !can_remove(key) || sections(settings).rows.iter().all(|row| row.key != key) {
        return None;
    }
    customize(settings);
    let search = settings.search.as_mut()?;
    let removed = search.children.remove(index_of(search, key)?);
    // Plugins are numbered, so the next one would answer to the same key.
    if !matches!(key, SectionKey::Plugin(_)) {
        while let Some(again) = index_of(search, key) {
            search.children.remove(again);
        }
    }
    Some(removed)
}

/// Whether the three sections differ from y-pop's defaults.
pub fn sections_customised(settings: &Settings) -> bool {
    use SectionKey::{Menus, Recent, StartMenu};
    let list = sections(settings);
    if list.defaults {
        return false;
    }
    let order: Vec<SectionKey> = list
        .rows
        .iter()
        .map(|row| row.key)
        .filter(|key| matches!(key, Recent | Menus | StartMenu))
        .collect();
    let plain = |key| match kind_ref(settings, key) {
        Some(SectionKind::Recent { count }) => count.is_none(),
        Some(SectionKind::Menus { browse, include }) => browse.is_none() && include.is_none(),
        Some(SectionKind::StartMenu { exclude }) => exclude.is_none(),
        _ => true,
    };
    order != [Recent, Menus, StartMenu] || ![Recent, Menus, StartMenu].into_iter().all(plain)
}

/// Back to y-pop's three default sections, as they come. Search-only items
/// and plugins stay, and as they turn the defaults off, the three are then
/// written out before them.
pub fn reset_sections(settings: &mut Settings) -> bool {
    if !sections_customised(settings) {
        return false;
    }
    let Some(search) = settings.search.as_mut() else {
        return false;
    };
    search.children.retain(|entry| {
        !matches!(
            entry,
            SearchEntry::Section(SearchSection {
                kind: SectionKind::Recent { .. }
                    | SectionKind::Menus { .. }
                    | SectionKind::StartMenu { .. },
                ..
            })
        )
    });
    if !sections(settings).defaults {
        let defaults = [SectionKey::Recent, SectionKey::Menus, SectionKey::StartMenu];
        let search = ensure(settings);
        for (at, key) in defaults.into_iter().enumerate() {
            if let Some(kind) = default_kind(key) {
                search.children.insert(at, new_section(kind));
            }
        }
    }
    true
}

/// Moves a row of the BROWSE list one place up or down among the others.
/// Everything else in `<search>` stays where it is.
pub fn move_section(settings: &mut Settings, key: SectionKey, up: bool) -> bool {
    let rows = browse_rows(settings);
    let Some(position) = rows.iter().position(|row| row.key == key) else {
        return false;
    };
    let other = if up {
        position.checked_sub(1)
    } else {
        Some(position + 1).filter(|&next| next < rows.len())
    };
    let Some(other) = other.map(|other| rows[other].key) else {
        return false;
    };
    customize(settings);
    let Some(search) = settings.search.as_mut() else {
        return false;
    };
    let (Some(here), Some(other)) = (index_of(search, key), index_of(search, other)) else {
        return false;
    };
    // Up: it takes the neighbour's place. Down: the neighbour slid up by one
    // when it left, so `other` is just after it. The same index either way.
    let entry = search.children.remove(here);
    search.children.insert(other, entry);
    true
}

fn kind_ref(settings: &Settings, key: SectionKey) -> Option<&SectionKind> {
    let search = settings.search.as_ref()?;
    match search.children.get(index_of(search, key)?)? {
        SearchEntry::Section(section) => Some(&section.kind),
        _ => None,
    }
}

/// The section's kind for editing, written out first when it is only implied.
fn kind_mut(settings: &mut Settings, key: SectionKey) -> Option<&mut SectionKind> {
    customize(settings);
    let search = settings.search.as_mut()?;
    let index = index_of(search, key)?;
    match search.children.get_mut(index)? {
        SearchEntry::Section(section) => Some(&mut section.kind),
        _ => None,
    }
}

// ---- Recent, Start menu ----------------------------------------------

/// How many recent items the section lists.
pub fn recent_count(settings: &Settings) -> u32 {
    match kind_ref(settings, SectionKey::Recent) {
        Some(SectionKind::Recent { count: Some(count) }) => *count,
        _ => y_pop::model::Section::DEFAULT_RECENT,
    }
}

/// `None` is "not written": the default.
pub fn set_recent_count(settings: &mut Settings, count: Option<u32>) -> bool {
    let current = match kind_ref(settings, SectionKey::Recent) {
        Some(SectionKind::Recent { count }) => *count,
        _ => None,
    };
    if current == count {
        return false;
    }
    match kind_mut(settings, SectionKey::Recent) {
        Some(SectionKind::Recent { count: slot }) => put(slot, count),
        _ => false,
    }
}

/// `;`-separated patterns, raw.
pub fn start_exclude(settings: &Settings) -> String {
    match kind_ref(settings, SectionKey::StartMenu) {
        Some(SectionKind::StartMenu {
            exclude: Some(text),
        }) => text.clone(),
        _ => String::new(),
    }
}

pub fn set_start_exclude(settings: &mut Settings, text: &str) -> bool {
    let new = absent_when_empty(text);
    if new.as_deref().unwrap_or_default() == start_exclude(settings) {
        return false;
    }
    match kind_mut(settings, SectionKey::StartMenu) {
        Some(SectionKind::StartMenu { exclude }) => put(exclude, new),
        _ => false,
    }
}

// ---- the tray menus section ------------------------------------------

/// `None`: not written, which is `columns`.
pub fn menus_browse(settings: &Settings) -> Option<MenusMode> {
    match kind_ref(settings, SectionKey::Menus) {
        Some(SectionKind::Menus { browse, .. }) => *browse,
        _ => None,
    }
}

pub fn set_menus_browse(settings: &mut Settings, value: Option<MenusMode>) -> bool {
    if menus_browse(settings) == value {
        return false;
    }
    match kind_mut(settings, SectionKey::Menus) {
        Some(SectionKind::Menus { browse, .. }) => put(browse, value),
        _ => false,
    }
}

/// The names in `include`, trimmed, without the empty ones. Empty means
/// every tray menu counts.
pub fn include_names(settings: &Settings) -> Vec<String> {
    match kind_ref(settings, SectionKey::Menus) {
        Some(SectionKind::Menus {
            include: Some(raw), ..
        }) => split_names(raw),
        _ => Vec::new(),
    }
}

fn split_names(raw: &str) -> Vec<String> {
    raw.split(';')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A tray menu whose name has a `;` can't be written in `include`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SemicolonInName(pub String);

/// Counts exactly `selected` of the tray menus `all` (their names, in menu
/// order). Selecting every one, or none, writes no `include` at all: that is
/// how y-pop says "every tray menu", and it keeps counting menus added later.
pub fn set_include(
    settings: &mut Settings,
    selected: &[String],
    all: &[String],
) -> Result<bool, SemicolonInName> {
    if let Some(name) = selected.iter().find(|name| name.contains(';')) {
        return Err(SemicolonInName(name.clone()));
    }
    let every = all
        .iter()
        .all(|menu| selected.iter().any(|name| same_menu(name, menu)));
    let new = if selected.is_empty() || every {
        None
    } else {
        // In menu order, so the file doesn't depend on the order of the ticks.
        let ordered: Vec<&str> = all
            .iter()
            .filter(|menu| selected.iter().any(|name| same_menu(name, menu)))
            .map(String::as_str)
            .collect();
        Some(ordered.join(";"))
    };

    let current = match kind_ref(settings, SectionKey::Menus) {
        Some(SectionKind::Menus { include, .. }) => include.clone(),
        _ => None,
    };
    if current == new {
        return Ok(false);
    }
    Ok(match kind_mut(settings, SectionKey::Menus) {
        Some(SectionKind::Menus { include, .. }) => put(include, new),
        _ => false,
    })
}

/// Names in `include` that match no tray menu — what y-pop warns about at
/// start-up. If **none** of them match, y-pop leaves every tray menu out.
pub fn unknown_includes(settings: &Settings) -> Vec<String> {
    let menus: Vec<&str> = crate::roots::menus(settings)
        .filter_map(|item| item.name.as_deref())
        .collect();
    include_names(settings)
        .into_iter()
        .filter(|name| !menus.iter().any(|menu| same_menu(name, menu)))
        .collect()
}

/// What became of a tray menu's entry in `include` when the menu was renamed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rename {
    /// The menu is not listed, or nothing lists any.
    NotListed,
    Renamed,
    /// The name can't be written there yet (empty, or with a `;`): the entry
    /// keeps the old name. The caller remembers it for the next edit, so
    /// retyping a name doesn't drop the menu from the search.
    Waiting,
}

/// The `include` of the first `<menus>`, the one y-pop reads.
fn include_mut(settings: &mut Settings) -> Option<&mut Option<String>> {
    settings
        .search
        .as_mut()?
        .children
        .iter_mut()
        .find_map(|entry| match entry {
            SearchEntry::Section(SearchSection {
                kind: SectionKind::Menus { include, .. },
                ..
            }) => Some(include),
            _ => None,
        })
}

/// A tray menu was renamed: the same entry of `include` follows it, so the
/// menu doesn't drop out of the search.
pub fn rename_in_include(settings: &mut Settings, old: &str, new: &str) -> Rename {
    let new = new.trim();
    let Some(raw) = include_mut(settings).and_then(|include| include.as_mut()) else {
        return Rename::NotListed;
    };
    let names = split_names(raw);
    if !names.iter().any(|name| same_menu(name, old)) {
        return Rename::NotListed;
    }
    if new.is_empty() || new.contains(';') {
        return Rename::Waiting;
    }
    let renamed: Vec<String> = names
        .into_iter()
        .map(|name| {
            if same_menu(&name, old) {
                new.to_owned()
            } else {
                name
            }
        })
        .collect();
    *raw = renamed.join(";");
    Rename::Renamed
}

/// A tray menu called `name` was deleted: nothing lists it any more, unless
/// another menu has the same name. An `include` left with no name goes, which
/// is "every tray menu": listing only menus that don't exist would leave
/// every one out of the search.
pub fn drop_from_include(settings: &mut Settings, name: &str) -> bool {
    let same_name = |menu: &y_pop::settings::Item| {
        menu.name
            .as_deref()
            .is_some_and(|other| same_menu(other, name))
    };
    if crate::roots::menus(settings).any(same_name) {
        return false;
    }
    let Some(include) = include_mut(settings) else {
        return false;
    };
    let Some(raw) = include.as_ref() else {
        return false;
    };
    let names = split_names(raw);
    let kept: Vec<String> = names
        .iter()
        .filter(|listed| !same_menu(listed, name))
        .cloned()
        .collect();
    if kept.len() == names.len() {
        return false;
    }
    *include = (!kept.is_empty()).then(|| kept.join(";"));
    true
}

/// Whether a tray menu of this name can't be listed in the `include` there is.
pub fn name_breaks_include(settings: &Settings, name: &str) -> bool {
    name.contains(';') && !include_names(settings).is_empty()
}

// ---- the actions -----------------------------------------------------

/// One action as the editor shows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActionRow {
    /// The `do` name, as written.
    pub id: String,
    /// The shortcut, as written.
    pub key: Option<String>,
    pub button: bool,
    /// Not in the file: y-pop adds `open` when it is missing.
    pub implied: bool,
    /// In the file, but y-pop drops it: an unknown action, or one listed
    /// twice. Nothing of it applies, and all that can be done is remove it.
    pub ignored: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActionsView {
    /// An `<actions>` is in the file. If not, the rows are y-pop's defaults.
    pub written: bool,
    /// The action menu's key, as written or the default.
    pub menu_key: String,
    pub quick_pick: bool,
    pub rows: Vec<ActionRow>,
}

fn actions_ref(settings: &Settings) -> Option<(&Option<String>, &Option<bool>, &[ActionEntry])> {
    settings
        .search
        .as_ref()?
        .children
        .iter()
        .find_map(|entry| match entry {
            SearchEntry::Section(SearchSection {
                kind:
                    SectionKind::Actions {
                        menu_key,
                        quick_pick,
                        entries,
                    },
                ..
            }) => Some((menu_key, quick_pick, entries.as_slice())),
            _ => None,
        })
}

fn default_menu_key() -> String {
    ActionsConfig::default().menu_key.to_string()
}

/// The actions y-pop will offer.
pub fn actions(settings: &Settings) -> ActionsView {
    let Some((menu_key, quick_pick, entries)) = actions_ref(settings) else {
        let defaults = ActionsConfig::default();
        return ActionsView {
            written: false,
            menu_key: default_menu_key(),
            quick_pick: defaults.quick_pick,
            rows: defaults
                .actions
                .iter()
                .map(|binding| ActionRow {
                    id: binding.id.name().to_owned(),
                    key: binding.key.map(|key| key.to_string()),
                    button: binding.button,
                    implied: false,
                    ignored: false,
                })
                .collect(),
        };
    };
    let mut seen: Vec<&str> = Vec::new();
    let mut rows: Vec<ActionRow> = entries
        .iter()
        .filter_map(|entry| match entry {
            ActionEntry::Action(item) => {
                let id = item.id.as_deref().unwrap_or_default();
                let ignored = ActionId::parse(id).is_none() || seen.contains(&id);
                seen.push(id);
                Some(ActionRow {
                    id: id.to_owned(),
                    key: item.key.clone(),
                    button: item.button.unwrap_or(false),
                    implied: false,
                    ignored,
                })
            }
            ActionEntry::Comment(_) => None,
        })
        .collect();
    let menu_key = menu_key.clone().unwrap_or_else(default_menu_key);
    if !rows.iter().any(|row| row.id == ActionId::Open.name()) {
        let enter = Chord::parse("Enter").expect("a valid shortcut");
        let taken = holds(&menu_key, enter)
            || rows
                .iter()
                .filter(|row| !row.ignored)
                .any(|row| row.key.as_deref().is_some_and(|key| holds(key, enter)));
        rows.insert(
            0,
            ActionRow {
                id: ActionId::Open.name().to_owned(),
                key: (!taken).then(|| enter.to_string()),
                button: false,
                implied: true,
                ignored: false,
            },
        );
    }
    ActionsView {
        written: true,
        menu_key,
        quick_pick: quick_pick.unwrap_or(true),
        rows,
    }
}

/// Whether the shortcut written `text` is `chord`.
fn holds(text: &str, chord: Chord) -> bool {
    Chord::parse(text).is_ok_and(|parsed| parsed == chord)
}

/// Writes the actions y-pop only implies — the defaults, or the missing
/// `open` — so that an edit changes one action and not the list. Returns
/// whether anything was written.
pub fn customize_actions(settings: &mut Settings) -> bool {
    let view = actions(settings);
    if view.written && view.rows.iter().all(|row| !row.implied) {
        return false;
    }
    let action = |row: &ActionRow| {
        ActionEntry::Action(ActionItem {
            id: Some(row.id.clone()),
            key: row.key.clone(),
            button: row.button.then_some(true),
            extra_attrs: Vec::new(),
        })
    };

    let search = ensure(settings);
    for child in &mut search.children {
        if let SearchEntry::Section(SearchSection {
            kind: SectionKind::Actions { entries, .. },
            ..
        }) = child
        {
            // Only `open` can be implied here, and it goes first.
            if let Some(open) = view.rows.first().filter(|row| row.implied) {
                entries.insert(0, action(open));
            }
            return true;
        }
    }
    search.children.push(new_section(SectionKind::Actions {
        menu_key: None,
        quick_pick: None,
        entries: view.rows.iter().map(action).collect(),
    }));
    true
}

type ActionParts<'a> = (
    &'a mut Option<String>,
    &'a mut Option<bool>,
    &'a mut Vec<ActionEntry>,
);

fn actions_mut(settings: &mut Settings) -> Option<ActionParts<'_>> {
    customize_actions(settings);
    settings
        .search
        .as_mut()?
        .children
        .iter_mut()
        .find_map(|entry| match entry {
            SearchEntry::Section(SearchSection {
                kind:
                    SectionKind::Actions {
                        menu_key,
                        quick_pick,
                        entries,
                    },
                ..
            }) => Some((menu_key, quick_pick, entries)),
            _ => None,
        })
}

/// The n-th action (comments among them are not counted).
fn action_at(entries: &mut [ActionEntry], index: usize) -> Option<&mut ActionItem> {
    entries
        .iter_mut()
        .filter_map(|entry| match entry {
            ActionEntry::Action(item) => Some(item),
            ActionEntry::Comment(_) => None,
        })
        .nth(index)
}

/// Index among `entries` (comments counted) of the n-th action.
fn entry_index(entries: &[ActionEntry], index: usize) -> Option<usize> {
    (0..entries.len())
        .filter(|&i| matches!(entries[i], ActionEntry::Action(_)))
        .nth(index)
}

/// Adds an action that the list does not have yet, without a key.
pub fn add_action(settings: &mut Settings, id: ActionId) -> bool {
    if actions(settings).rows.iter().any(|row| row.id == id.name()) {
        return false;
    }
    let Some((_, _, entries)) = actions_mut(settings) else {
        return false;
    };
    entries.push(ActionEntry::Action(ActionItem {
        id: Some(id.name().to_owned()),
        ..ActionItem::default()
    }));
    true
}

/// The actions not in the list yet.
pub fn addable_actions(settings: &Settings) -> Vec<ActionId> {
    let rows = actions(settings).rows;
    ActionId::ALL
        .into_iter()
        .filter(|id| rows.iter().all(|row| row.id != id.name()))
        .collect()
}

/// `open` can't go: Enter would do nothing.
pub fn remove_action(settings: &mut Settings, index: usize) -> bool {
    let view = actions(settings);
    if view
        .rows
        .get(index)
        .is_none_or(|row| row.id == ActionId::Open.name())
    {
        return false;
    }
    let Some((_, _, entries)) = actions_mut(settings) else {
        return false;
    };
    match entry_index(entries, index) {
        Some(at) => {
            entries.remove(at);
            true
        }
        None => false,
    }
}

pub fn move_action(settings: &mut Settings, index: usize, up: bool) -> bool {
    let count = actions(settings).rows.len();
    let target = if up {
        index.checked_sub(1)
    } else {
        Some(index + 1).filter(|&next| next < count)
    };
    let Some(target) = target.filter(|_| index < count) else {
        return false;
    };
    let Some((_, _, entries)) = actions_mut(settings) else {
        return false;
    };
    let (Some(from), Some(to)) = (entry_index(entries, index), entry_index(entries, target)) else {
        return false;
    };
    let entry = entries.remove(from);
    entries.insert(to, entry);
    true
}

pub fn set_action_key(settings: &mut Settings, index: usize, text: &str) -> bool {
    let new = absent_when_empty(text);
    let view = actions(settings);
    if view.rows.get(index).is_none_or(|row| row.key == new) {
        return false;
    }
    let Some((_, _, entries)) = actions_mut(settings) else {
        return false;
    };
    action_at(entries, index).is_some_and(|item| put(&mut item.key, new))
}

pub fn set_action_button(settings: &mut Settings, index: usize, on: bool) -> bool {
    let view = actions(settings);
    if view.rows.get(index).is_none_or(|row| row.button == on) {
        return false;
    }
    let Some((_, _, entries)) = actions_mut(settings) else {
        return false;
    };
    action_at(entries, index).is_some_and(|item| put(&mut item.button, on.then_some(true)))
}

pub fn set_menu_key(settings: &mut Settings, text: &str) -> bool {
    // Empty is the default key, which is not written.
    let text = absent_when_empty(text).unwrap_or_else(default_menu_key);
    if text == actions(settings).menu_key {
        return false;
    }
    let Some((menu_key, _, _)) = actions_mut(settings) else {
        return false;
    };
    put(menu_key, (text != default_menu_key()).then_some(text))
}

pub fn set_quick_pick(settings: &mut Settings, on: bool) -> bool {
    if actions(settings).quick_pick == on {
        return false;
    }
    let Some((_, quick_pick, _)) = actions_mut(settings) else {
        return false;
    };
    put(quick_pick, (!on).then_some(false))
}

/// Back to y-pop's default actions: `<actions>` is removed.
pub fn reset_actions(settings: &mut Settings) -> bool {
    let Some(search) = settings.search.as_mut() else {
        return false;
    };
    let before = search.children.len();
    search.children.retain(|entry| {
        !matches!(
            entry,
            SearchEntry::Section(SearchSection {
                kind: SectionKind::Actions { .. },
                ..
            })
        )
    });
    search.children.len() != before
}

/// Whether Enter does nothing on a result: no action has it, and it doesn't
/// open the menu. A shortcut may be moved off Enter, so this is worth a word,
/// not a refusal.
pub fn enter_is_unused(settings: &Settings) -> bool {
    let enter = Chord::parse("Enter").expect("a valid shortcut");
    let view = actions(settings);
    !holds(&view.menu_key, enter)
        && !view
            .rows
            .iter()
            .filter(|row| !row.ignored)
            .any(|row| row.key.as_deref().is_some_and(|key| holds(key, enter)))
}

/// What y-pop would warn about in the shortcuts, with the row it is on
/// (`None`: the menu key) — the reader's own rules, in its order: the menu
/// key first, then each action, a shortcut already taken being ignored.
pub fn key_problems(settings: &Settings) -> Vec<(Option<usize>, String)> {
    let view = actions(settings);
    let mut problems = Vec::new();
    let mut taken: Vec<Chord> = Vec::new();

    match Chord::parse(&view.menu_key) {
        Ok(chord) => taken.push(chord),
        Err(error) => {
            problems.push((None, format!("{:?} {error}", view.menu_key)));
            taken.push(Chord::parse(&default_menu_key()).expect("a valid default"));
        }
    }
    for (index, row) in view.rows.iter().enumerate() {
        let Some(text) = row.key.as_deref().filter(|_| !row.ignored) else {
            continue;
        };
        match Chord::parse(text) {
            Ok(chord) if taken.contains(&chord) => {
                problems.push((Some(index), format!("{text:?} is already another action's")));
            }
            Ok(chord) => taken.push(chord),
            Err(error) => problems.push((Some(index), format!("{text:?} {error}"))),
        }
    }
    problems
}

#[cfg(test)]
mod tests;
