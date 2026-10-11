//! The menu tree as plain functions on [`Settings`], no UI: where an item
//! sits, how it moves, what a new item is inserted after, and what the
//! outline shows. Everything a button in the inspector does is one of these,
//! so every rule is tested without a window.
//!
//! Items are addressed by [`ItemId`], never by position. A *place* is a
//! [`Location`]: the group whose children hold it (`None` for the top-level
//! list, where the tray menus are) and an index among that list's entries —
//! comments included, since they are entries too.
//!
//! **Moving steps over items, not over comments.** "Up" puts an item where
//! the previous *item* of its list was; comments between the two stay where
//! they are. That way one click is one visible step whether comments are
//! shown or hidden, and a comment attached to an item (its `lead`) is part
//! of the item and travels with it.

use std::collections::HashSet;

use y_pop::settings::{AppFields, Entry, Item, ItemId, ItemKind, Settings};

/// Where an entry sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location {
    /// The group holding it; `None` for the top-level list.
    pub parent: Option<ItemId>,
    /// Among the entries of that list, comments counted.
    pub index: usize,
}

// ---- finding ---------------------------------------------------------

pub fn item(settings: &Settings, id: ItemId) -> Option<&Item> {
    fn find(entries: &[Entry], id: ItemId) -> Option<&Item> {
        entries.iter().find_map(|entry| match entry {
            Entry::Item(item) if item.id == id => Some(item),
            Entry::Item(item) => find(&item.children, id),
            Entry::Comment(_) => None,
        })
    }
    find(&settings.roots, id)
}

/// Indices to follow from the top-level list down to `id`.
fn path_to(entries: &[Entry], id: ItemId) -> Option<Vec<usize>> {
    for (index, entry) in entries.iter().enumerate() {
        let Entry::Item(item) = entry else { continue };
        if item.id == id {
            return Some(vec![index]);
        }
        if let Some(mut rest) = path_to(&item.children, id) {
            rest.insert(0, index);
            return Some(rest);
        }
    }
    None
}

pub fn item_mut(settings: &mut Settings, id: ItemId) -> Option<&mut Item> {
    let path = path_to(&settings.roots, id)?;
    let mut entries = &mut settings.roots;
    for (step, &index) in path.iter().enumerate() {
        let Entry::Item(item) = &mut entries[index] else {
            return None;
        };
        if step + 1 == path.len() {
            return Some(item);
        }
        entries = &mut item.children;
    }
    None
}

pub fn locate(settings: &Settings, id: ItemId) -> Option<Location> {
    let path = path_to(&settings.roots, id)?;
    let index = *path.last()?;
    let parent = if path.len() == 1 {
        None
    } else {
        // The group is the item one step up the path.
        let mut entries = &settings.roots;
        let mut parent = None;
        for &step in &path[..path.len() - 1] {
            let Entry::Item(item) = &entries[step] else {
                return None;
            };
            parent = Some(item.id);
            entries = &item.children;
        }
        parent
    };
    Some(Location { parent, index })
}

/// The list `parent` names: its children, or the top-level list.
pub fn entries(settings: &Settings, parent: Option<ItemId>) -> Option<&Vec<Entry>> {
    match parent {
        None => Some(&settings.roots),
        Some(id) => item(settings, id).map(|item| &item.children),
    }
}

pub fn entries_mut(settings: &mut Settings, parent: Option<ItemId>) -> Option<&mut Vec<Entry>> {
    match parent {
        None => Some(&mut settings.roots),
        Some(id) => item_mut(settings, id).map(|item| &mut item.children),
    }
}

/// Whether `candidate` is `ancestor` itself or somewhere inside it.
pub fn is_inside(settings: &Settings, ancestor: ItemId, candidate: ItemId) -> bool {
    ancestor == candidate
        || item(settings, ancestor).is_some_and(|item| {
            fn contains(entries: &[Entry], id: ItemId) -> bool {
                entries.iter().any(|entry| match entry {
                    Entry::Item(item) => item.id == id || contains(&item.children, id),
                    Entry::Comment(_) => false,
                })
            }
            contains(&item.children, candidate)
        })
}

/// How many items are inside `id`, at any depth.
pub fn count_inside(settings: &Settings, id: ItemId) -> usize {
    fn count(entries: &[Entry]) -> usize {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Item(item) => 1 + count(&item.children),
                Entry::Comment(_) => 0,
            })
            .sum()
    }
    item(settings, id).map_or(0, |item| count(&item.children))
}

/// Comments in `id`'s subtree: attached and free, at any depth, and its own.
pub fn count_comments(settings: &Settings, id: ItemId) -> usize {
    fn count(entries: &[Entry]) -> usize {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Item(item) => item.lead.len() + count(&item.children),
                Entry::Comment(_) => 1,
            })
            .sum()
    }
    item(settings, id).map_or(0, |item| item.lead.len() + count(&item.children))
}

fn is_group(entry: &Entry) -> bool {
    matches!(entry, Entry::Item(item) if item.kind == ItemKind::Group)
}

fn is_item(entry: &Entry) -> bool {
    matches!(entry, Entry::Item(_))
}

// ---- moving ----------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveDir {
    Up,
    Down,
    /// Becomes the entry after its group.
    Out,
    /// Becomes the last entry of the nearest group above it.
    Into,
}

/// The index of the previous (`up`) or next item in `list`, comments skipped.
fn neighbour(list: &[Entry], index: usize, up: bool) -> Option<usize> {
    if up {
        (0..index).rev().find(|&i| is_item(&list[i]))
    } else {
        (index + 1..list.len()).find(|&i| is_item(&list[i]))
    }
}

/// The group an item would go into: the nearest group above it in its list.
pub fn group_above(settings: &Settings, id: ItemId) -> Option<ItemId> {
    let location = locate(settings, id)?;
    let list = entries(settings, location.parent)?;
    (0..location.index).rev().find(|&i| is_group(&list[i])).and_then(|i| match &list[i] {
        Entry::Item(group) => Some(group.id),
        Entry::Comment(_) => None,
    })
}

/// Whether `dir` does anything for `id` inside the menu whose own item is
/// `root`. A tray menu itself can only change place among the others.
pub fn can_move(settings: &Settings, id: ItemId, dir: MoveDir, root: ItemId) -> bool {
    let Some(location) = locate(settings, id) else {
        return false;
    };
    let Some(list) = entries(settings, location.parent) else {
        return false;
    };
    match dir {
        MoveDir::Up => neighbour(list, location.index, true).is_some(),
        MoveDir::Down => neighbour(list, location.index, false).is_some(),
        MoveDir::Out => location.parent.is_some_and(|parent| parent != root),
        MoveDir::Into => location.parent.is_some() && group_above(settings, id).is_some(),
    }
}

/// Moves `id`; `false`, with nothing changed, when it can't go that way.
pub fn move_item(settings: &mut Settings, id: ItemId, dir: MoveDir, root: ItemId) -> bool {
    if !can_move(settings, id, dir, root) {
        return false;
    }
    let Some(location) = locate(settings, id) else {
        return false;
    };

    match dir {
        MoveDir::Up | MoveDir::Down => {
            let Some(list) = entries_mut(settings, location.parent) else {
                return false;
            };
            let Some(other) = neighbour(list, location.index, dir == MoveDir::Up) else {
                return false;
            };
            let entry = list.remove(location.index);
            // Up: it takes the neighbour's place. Down: the neighbour slid up
            // by one when `entry` left, so `other` is just after it. Either way
            // the same index.
            list.insert(other, entry);
            true
        }
        MoveDir::Out => {
            let Some(parent) = location.parent else {
                return false;
            };
            let Some(group) = locate(settings, parent) else {
                return false;
            };
            let Some(entry) = take(settings, id) else {
                return false;
            };
            // Where the group is has not changed: `id` was inside it.
            put(settings, group.parent, group.index + 1, entry)
        }
        MoveDir::Into => {
            let Some(target) = group_above(settings, id) else {
                return false;
            };
            move_to_group(settings, id, target)
        }
    }
}

/// Moves `id` to the end of `group`, which may be in another tray menu.
/// Refused for a group inside `id` (or `id` itself).
pub fn move_to_group(settings: &mut Settings, id: ItemId, group: ItemId) -> bool {
    let is_a_group = item(settings, group).is_some_and(|item| item.kind == ItemKind::Group);
    if !is_a_group || is_inside(settings, id, group) {
        return false;
    }
    let Some(entry) = take(settings, id) else {
        return false;
    };
    let end = item(settings, group).map_or(0, |group| group.children.len());
    put(settings, Some(group), end, entry)
}

/// Removes the entry of `id` from wherever it is and hands it over.
fn take(settings: &mut Settings, id: ItemId) -> Option<Entry> {
    let location = locate(settings, id)?;
    let list = entries_mut(settings, location.parent)?;
    Some(list.remove(location.index))
}

/// Puts `entry` at `index` of `parent`'s list (clamped to its end).
fn put(settings: &mut Settings, parent: Option<ItemId>, index: usize, entry: Entry) -> bool {
    let Some(list) = entries_mut(settings, parent) else {
        return false;
    };
    list.insert(index.min(list.len()), entry);
    true
}

/// Every group in every tray menu, as `("y-pop › Explorers", id)`, for a
/// "Move to…" list. Groups inside `excluding` (and it) are left out: an item
/// can't move into itself.
pub fn groups(settings: &Settings, excluding: Option<ItemId>) -> Vec<(String, ItemId)> {
    fn walk(entries: &[Entry], path: &str, excluding: Option<ItemId>, out: &mut Vec<(String, ItemId)>) {
        for entry in entries {
            let Entry::Item(item) = entry else { continue };
            if item.kind != ItemKind::Group || Some(item.id) == excluding {
                continue;
            }
            let name = crate::roots::label(item);
            let here = if path.is_empty() { name } else { format!("{path} › {name}") };
            out.push((here.clone(), item.id));
            walk(&item.children, &here, excluding, out);
        }
    }
    let mut out = Vec::new();
    walk(&settings.roots, "", excluding, &mut out);
    out
}

/// The tray menu (top-level item) `id` belongs to — itself, for one.
pub fn root_of(settings: &Settings, id: ItemId) -> Option<ItemId> {
    let first = *path_to(&settings.roots, id)?.first()?;
    match settings.roots.get(first)? {
        Entry::Item(root) => Some(root.id),
        Entry::Comment(_) => None,
    }
}

/// Wraps the top-level item `id` in a new group of the same name, so an app
/// that launched on a tray click becomes a menu that can hold more. The group
/// takes its place, its attached comments and the empty line before it; the
/// app keeps its own hotkeys. Returns the group's id.
pub fn wrap_in_group(settings: &mut Settings, id: ItemId) -> Option<ItemId> {
    let location = locate(settings, id)?;
    if location.parent.is_some() {
        return None;
    }
    let mut group = settings.new_item(ItemKind::Group);
    let group_id = group.id;
    let Entry::Item(mut inner) = std::mem::replace(&mut settings.roots[location.index], Entry::Comment(Default::default())) else {
        return None;
    };
    group.name = inner.name.clone();
    group.lead = std::mem::take(&mut inner.lead);
    group.blank_before = std::mem::take(&mut inner.blank_before);
    group.children = vec![Entry::Item(inner)];
    settings.roots[location.index] = Entry::Item(group);
    Some(group_id)
}

// ---- copying and removing --------------------------------------------

/// A copy of `source` with new ids all the way down: its attached comments
/// are left out, and so are hotkeys — a combination belongs to one entry —
/// and its name says it is a copy.
fn copy_of(settings: &mut Settings, source: &Item) -> Item {
    let mut copy = source.clone();
    copy.id = settings.new_id();
    copy.lead.clear();
    copy.hotkeys.clear();
    copy.blank_before = false;
    if let Some(name) = &mut copy.name {
        name.push_str(" (copy)");
    }
    copy.children = source
        .children
        .iter()
        .map(|entry| match entry {
            Entry::Item(child) => {
                let mut inner = copy_of(settings, child);
                // Only the item the user asked for says "(copy)".
                inner.name = child.name.clone();
                Entry::Item(inner)
            }
            Entry::Comment(comment) => Entry::Comment(comment.clone()),
        })
        .collect();
    copy
}

/// Duplicates `id` right after itself and returns the copy's id. A tray menu
/// is not duplicated: two tray icons with one menu is not a thing to ask for.
pub fn duplicate(settings: &mut Settings, id: ItemId) -> Option<ItemId> {
    let location = locate(settings, id)?;
    location.parent?;
    let source = item(settings, id)?.clone();
    let copy = copy_of(settings, &source);
    let copy_id = copy.id;
    put(settings, location.parent, location.index + 1, Entry::Item(copy)).then_some(copy_id)
}

/// Removes `id` (with everything inside it and the comments attached to it)
/// and returns it, plus what to select instead: the item before it in its
/// list, else the one after, else the group it was in.
pub fn delete(settings: &mut Settings, id: ItemId) -> Option<(Item, Option<ItemId>)> {
    let location = locate(settings, id)?;
    let list = entries(settings, location.parent)?;
    let id_at = |i: usize| match &list[i] {
        Entry::Item(item) => Some(item.id),
        Entry::Comment(_) => None,
    };
    let next = neighbour(list, location.index, true)
        .and_then(id_at)
        .or_else(|| neighbour(list, location.index, false).and_then(id_at))
        .or(location.parent);

    match take(settings, id)? {
        Entry::Item(item) => Some((item, next)),
        Entry::Comment(_) => None,
    }
}

// ---- inserting -------------------------------------------------------

/// Where the next new item goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Insertion {
    pub parent: ItemId,
    pub index: usize,
}

/// The insertion point of the menu whose own item is `root`, given what is
/// selected: inside it at the end when the selection is the menu itself or an
/// open group, else right after the selected item.
pub fn insertion_point(
    settings: &Settings,
    root: ItemId,
    selected: Option<ItemId>,
    collapsed: &HashSet<ItemId>,
) -> Option<Insertion> {
    let end_of = |group: ItemId| {
        item(settings, group).map(|item| Insertion { parent: group, index: item.children.len() })
    };

    let selected = selected.filter(|&id| is_inside(settings, root, id)).unwrap_or(root);
    if selected == root {
        return end_of(root);
    }
    let chosen = item(settings, selected)?;
    if chosen.kind == ItemKind::Group && !collapsed.contains(&selected) {
        return end_of(selected);
    }
    let location = locate(settings, selected)?;
    Some(Insertion { parent: location.parent?, index: location.index + 1 })
}

/// Inserts `items` in order at `at` and returns the last one's id.
pub fn insert(settings: &mut Settings, at: Insertion, items: Vec<Item>) -> Option<ItemId> {
    let last = items.last()?.id;
    let list = entries_mut(settings, Some(at.parent))?;
    let start = at.index.min(list.len());
    for (offset, item) in items.into_iter().enumerate() {
        list.insert(start + offset, Entry::Item(item));
    }
    Some(last)
}

/// What the toolbar can add.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewKind {
    App,
    Group,
    Separator,
    /// A separator with a title: a section header.
    Titled,
    /// An app that lists a folder as a submenu.
    Folder,
    Break,
    About,
    Quit,
    Settings,
}

/// A new, empty item of `kind` with a fresh id and a placeholder a person
/// would replace.
pub fn new_item(settings: &mut Settings, kind: NewKind) -> Item {
    let (item_kind, name) = match kind {
        NewKind::App => (ItemKind::App(Box::new(AppFields { exe: Some(String::new()), ..AppFields::default() })), "New app"),
        NewKind::Folder => (
            ItemKind::App(Box::new(AppFields { exe: Some(String::new()), browse: true, ..AppFields::default() })),
            "New folder",
        ),
        NewKind::Group => (ItemKind::Group, "New group"),
        NewKind::Separator => (ItemKind::Separator, ""),
        NewKind::Titled => (ItemKind::Separator, "Section"),
        NewKind::Break => (ItemKind::Break, ""),
        NewKind::About => (ItemKind::About, ""),
        NewKind::Quit => (ItemKind::Quit, ""),
        NewKind::Settings => (ItemKind::Settings, ""),
    };
    let mut item = settings.new_item(item_kind);
    // Leaves without a title keep the default one y-pop gives them.
    if !name.is_empty() {
        item.name = Some(name.to_owned());
    }
    item
}

// ---- the outline -----------------------------------------------------

/// One line of the outline.
#[derive(Clone, PartialEq, Debug)]
pub struct TreeRow {
    pub depth: usize,
    pub kind: TreeRowKind,
}

#[derive(Clone, PartialEq, Debug)]
pub enum TreeRowKind {
    Item {
        id: ItemId,
        /// Comments above it that are not shown: attached ones, and free
        /// ones since the previous item.
        hidden_comments: usize,
    },
    Comment {
        first_line: String,
        lines: usize,
        /// Sits right above its item, at its depth, joined to it.
        attached: bool,
    },
    /// "The next item goes here".
    Marker,
    /// Hidden free comments after the last item of a group.
    HiddenAtEnd(usize),
}

/// The outline of the menu whose own item is `root`: the root row, then
/// everything inside it that isn't collapsed (the root never collapses),
/// with the insertion marker where `at` says.
pub fn rows(
    settings: &Settings,
    root: ItemId,
    collapsed: &HashSet<ItemId>,
    show_comments: bool,
    at: Option<Insertion>,
) -> Vec<TreeRow> {
    let mut out = Vec::new();
    if let Some(root_item) = item(settings, root) {
        walk(root_item, 0, collapsed, show_comments, at, &mut out);
    }
    out
}

fn first_line(text: &str) -> String {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or("(empty comment)").trim().to_owned()
}

fn walk(
    item: &Item,
    depth: usize,
    collapsed: &HashSet<ItemId>,
    show_comments: bool,
    at: Option<Insertion>,
    out: &mut Vec<TreeRow>,
) {
    out.push(TreeRow { depth, kind: TreeRowKind::Item { id: item.id, hidden_comments: 0 } });

    let open = item.kind == ItemKind::Group && (depth == 0 || !collapsed.contains(&item.id));
    if !open {
        return;
    }

    let marker_here = |index: usize| at == Some(Insertion { parent: item.id, index });
    // Hidden free comments waiting to be counted on the next item's row.
    let mut pending = 0;

    for (index, entry) in item.children.iter().enumerate() {
        if marker_here(index) {
            out.push(TreeRow { depth: depth + 1, kind: TreeRowKind::Marker });
        }
        match entry {
            Entry::Comment(comment) => {
                if show_comments {
                    out.push(TreeRow {
                        depth: depth + 1,
                        kind: TreeRowKind::Comment {
                            first_line: first_line(&comment.text),
                            lines: comment.text.lines().count().max(1),
                            attached: false,
                        },
                    });
                } else {
                    pending += 1;
                }
            }
            Entry::Item(child) => {
                let mut hidden = std::mem::take(&mut pending);
                if show_comments {
                    for lead in &child.lead {
                        out.push(TreeRow {
                            depth: depth + 1,
                            kind: TreeRowKind::Comment {
                                first_line: first_line(&lead.text),
                                lines: lead.text.lines().count().max(1),
                                attached: true,
                            },
                        });
                    }
                } else {
                    hidden += child.lead.len();
                }
                let before = out.len();
                walk(child, depth + 1, collapsed, show_comments, at, out);
                if let Some(TreeRow { kind: TreeRowKind::Item { hidden_comments, .. }, .. }) = out.get_mut(before) {
                    *hidden_comments = hidden;
                }
            }
        }
    }

    if pending > 0 {
        out.push(TreeRow { depth: depth + 1, kind: TreeRowKind::HiddenAtEnd(pending) });
    }
    if marker_here(item.children.len()) {
        out.push(TreeRow { depth: depth + 1, kind: TreeRowKind::Marker });
    }
}

#[cfg(test)]
mod tests;
