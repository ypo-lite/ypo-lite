//! One function per screen: state and a [`Viewport`] in, a view out.
//!
//! Phase 3 only: General, Skin and Variables say what they will hold, and a
//! tray menu shows its entries as a plain list with the one action the shell
//! already supports (delete). The real screens replace these one by one.

use y_pop::settings::{Entry, Item, ItemId, ItemKind, Settings, VarEntry};

use crate::roots;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;

pub fn general(settings: &Settings, path: &std::path::Path) -> View {
    let menus = roots::menus(settings).count();
    let items = settings.items().count();
    page(
        "General",
        "How y-pop draws its menus and launches what you pick.",
        Stack::vertical(space::M)
            .child("file", text::body(format!("{}", path.display())))
            .child("count", text::caption(format!("{menus} tray menus · {items} items")))
            .child("later", note("Menu style, shell-execute and show-shortcuts come next.")),
    )
}

pub fn skin(_settings: &Settings) -> View {
    page(
        "Skin",
        "The theme of the popup menus, and what you override on top of it.",
        note("Theme cards, overrides and a live preview come next."),
    )
}

pub fn variables(settings: &Settings) -> View {
    let count = settings.variables.iter().filter(|entry| matches!(entry, VarEntry::Var(_))).count();
    page(
        "Variables",
        "Names usable as %name% in any path below. A variable only sees the ones above it.",
        Stack::vertical(space::M)
            .child("count", text::body(format!("{count} variables")))
            .child("later", note("The editable table comes next.")),
    )
}

/// A tray menu: for now its entries as a list, and Delete.
pub fn menu(
    settings: &Settings,
    id: ItemId,
    confirming_delete: bool,
    context: &ViewContext<ConfigApp>,
) -> View {
    let Some(item) = roots::find(settings, id) else {
        return page("Tray menu", "This tray menu is no longer in the file.", View::empty());
    };

    let kind = match item.kind {
        ItemKind::Group => "A tray icon that opens this menu",
        ItemKind::App(_) => "A tray icon that launches this app when clicked",
        _ => "A tray icon",
    };
    let hotkeys: Vec<String> = item.hotkeys.iter().map(|hotkey| hotkey.key.clone()).collect();
    let hotkey_line = if hotkeys.is_empty() { "No hotkey".to_owned() } else { hotkeys.join("  ·  ") };

    let entries: Vec<(String, View)> = item
        .children
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let line = match entry {
                Entry::Item(child) => format!("{}  —  {}", child.kind.element(), roots::label(child)),
                Entry::Comment(comment) => format!("comment  —  {}", first_line(&comment.text)),
            };
            (format!("entry-{index}"), text::body(line).into())
        })
        .collect();

    let delete: View = if confirming_delete {
        Stack::horizontal(space::S)
            .child("ask", text::body("Remove this tray icon and everything in it?"))
            .child("yes", button("Delete", true, true, context.callback(move |()| Msg::DeleteConfirmed(id))))
            .child("no", button("Keep it", false, true, context.callback(|()| Msg::DeleteCancelled)))
            .into()
    } else {
        button("Delete this tray menu", false, true, context.callback(move |()| Msg::DeleteAsked(id)))
    };

    page(
        &roots::label(item),
        kind,
        Stack::vertical(space::L)
            .child("hotkey", text::caption(hotkey_line))
            .child("entries", section(format!("Entries — {}", entries.len()), Stack::vertical(space::XS).children(entries)))
            .child("later", note("The tree and the inspector come next."))
            .child("delete", delete),
    )
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// Used by the nav: the Item a root screen is about, for its icon.
pub fn menu_symbol(item: &Item) -> Symbol {
    match item.kind {
        ItemKind::Group => Symbol::Folder,
        _ => Symbol::AllApps,
    }
}
