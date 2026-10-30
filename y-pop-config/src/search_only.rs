//! The search-only groups and folders: the items that are children of
//! `<search>` and in no tray menu. Plain functions on [`Settings`], no UI.
//!
//! Everything *inside* one is edited by [`crate::tree`], by id, like a tray
//! menu. What is here is the top level of that list, which `tree` refuses
//! (its siblings are sections and comments of another type): adding,
//! removing and finding them. A group moves as a row of the BROWSE list,
//! [`crate::search_edit::move_section`]; a folder is no row, so it stays put.
//!
//! Writing one turns y-pop's default sections off (any child of `<search>`
//! does), so adding first writes them out — see [`search_edit::customize`].


use y_pop::settings::{Item, ItemId, SearchEntry, Settings};

use crate::search_edit;
use crate::tree::{self, NewKind};

/// The top-level search-only items, in file order.
pub fn roots(settings: &Settings) -> impl Iterator<Item = &Item> {
    settings
        .search
        .iter()
        .flat_map(|search| search.children.iter())
        .filter_map(|entry| match entry {
            SearchEntry::Item(item) => Some(item),
            _ => None,
        })
}

pub fn find(settings: &Settings, id: ItemId) -> Option<&Item> {
    roots(settings).find(|item| item.id == id)
}

/// Where the item sits among the top-level search-only items, for
/// remembering the open screen across runs and for `--select`.
pub fn position(settings: &Settings, id: ItemId) -> Option<usize> {
    roots(settings).position(|item| item.id == id)
}

pub fn at_position(settings: &Settings, position: usize) -> Option<ItemId> {
    roots(settings).nth(position).map(|item| item.id)
}

/// Appends a new, empty group — or a browsed folder, which is an app with
/// `browse="true"`: a plain app is not allowed here — and returns its id.
pub fn add(settings: &mut Settings, kind: NewKind) -> Option<ItemId> {
    if !matches!(kind, NewKind::Group | NewKind::Folder) {
        return None;
    }
    // After the defaults are written: the first child turns them off.
    search_edit::customize(settings);
    let mut item = tree::new_item(settings, kind);
    // Set apart from the section before it, like a new paragraph.
    item.blank_before = true;
    let id = item.id;
    search_edit::ensure(settings)
        .children
        .push(SearchEntry::Item(item));
    Some(id)
}

/// Removes a top-level search-only item with everything in it and the
/// comments attached to it, and hands it back. `None` for any other id.
pub fn delete(settings: &mut Settings, id: ItemId) -> Option<Item> {
    find(settings, id)?;
    match search_edit::remove_section(settings, search_edit::SectionKey::Item(id))? {
        SearchEntry::Item(item) => Some(item),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
