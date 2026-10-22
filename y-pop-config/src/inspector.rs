//! The inspector: everything about the selected item, and the buttons that
//! move, copy and delete it.
//!
//! Edits apply as they are typed — the outline row changes with the name box
//! — and every message carries the item's id, so a late event from a box that
//! was showing another item can't land on the wrong one.
//!
//! An app gets a basic form here (name, target, tooltip, icon); its full set
//! of launch options arrives in the next step. Comments
//! are shown in the outline and not yet editable.

use y_pop::settings::{Item, ItemId, ItemKind, Settings};

use crate::menu_edit::{FlagField, MenuMsg, TextField, canonical_hotkey, split_hotkey};
use crate::menu_view::{HotkeyUi, MenuUi, symbol_of};
use crate::tree::{self, MoveDir};
use crate::ui::key_picker::{KeyPickerEvents, combo_labels, key_picker};
use crate::ui::editor::modal;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg, roots};
use windows_reactor::*;

/// Narrowest card content that keeps the identity and the move bar side by
/// side; narrower, the bar goes under the name.
const HEADER_SIDE_BY_SIDE_MIN: f64 = 420.0;

pub fn inspector_view(
    settings: &Settings,
    ui: &MenuUi<'_>,
    id: ItemId,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let Some(item) = tree::item(settings, id) else {
        return card().content(text::body("Nothing is selected."));
    };
    let location = tree::locate(settings, id);
    let is_root = location.is_some_and(|location| location.parent.is_none());
    let inner = viewport.inset(CARD_INSET);

    let mut body = Stack::vertical(space::L)
        .child("header", header(settings, ui, item, is_root, inner, context))
        .maybe("confirm", (is_root && ui.confirming_delete).then(|| confirm_delete(id, context)));

    // Moving to another group isn't a thing for a tray menu or a column break.
    if !is_root && item.kind != ItemKind::Break {
        let paths: Vec<String> = tree::groups(settings, Some(id)).into_iter().map(|(path, _)| path).collect();
        // Keyed by where the item is: after a move the box must start over
        // with nothing chosen, not keep showing the group it was just sent to.
        let place = location.map_or(0, |l| l.index + 1000 * l.parent.map_or(0, |p| p.get() as usize));
        let moving: View = combo(
            paths,
            None,
            context.callback(move |choice: Option<usize>| match choice {
                Some(group) => Msg::Menu(MenuMsg::MoveTo { id, group }),
                None => Msg::Ignored,
            }),
        )
        .placeholder_text("Move to… (any group, in any tray menu)")
        .into();
        body = body.child(format!("moveto-{}-{place}", id.get()), moving);
    }

    // One key per item: switching to another starts every box fresh.
    body = body.child(format!("fields-{}", id.get()), fields(settings, item, is_root, inner, context));
    body = body.child("xml", xml_preview(item, ui, context));

    card().content(body)
}

// ---- header ----------------------------------------------------------

fn kind_label(item: &Item) -> &'static str {
    match &item.kind {
        ItemKind::Group => "Group",
        ItemKind::App(app) if app.browse => "Folder (browse)",
        ItemKind::App(app) if app.app_id.is_some() => "Store app",
        ItemKind::App(_) => "App",
        ItemKind::Separator if item.name.as_deref().is_some_and(|name| !name.is_empty()) => "Titled separator",
        ItemKind::Separator => "Separator",
        ItemKind::Break => "Column break",
        ItemKind::About => "About",
        ItemKind::Quit => "Quit",
        ItemKind::Settings => "Settings",
    }
}

fn header(
    settings: &Settings,
    ui: &MenuUi<'_>,
    item: &Item,
    is_root: bool,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let place = if is_root {
        "tray icon".to_owned()
    } else {
        match tree::locate(settings, item.id).and_then(|l| l.parent).and_then(|p| tree::item(settings, p)) {
            Some(parent) => format!("in {}", roots::label(parent)),
            None => String::new(),
        }
    };
    let identity: View = Stack::horizontal(space::M)
        .child("icon", SymbolIcon::new().symbol(symbol_of(&item.kind)))
        .child(
            "who",
            Stack::vertical(space::XXS)
                .child("name", text::strong(roots::label(item)))
                .child("kind", text::caption(format!("{} · {place}", kind_label(item)))),
        )
        .into();

    let bar = move_bar(settings, ui, item.id, is_root, context);

    if viewport.fits(HEADER_SIDE_BY_SIDE_MIN) {
        Grid::new().columns([GridLength::STAR, GridLength::Auto]).column_spacing(space::M).keyed_children([
            ("identity", cell(identity, 0, 0)),
            ("bar", cell(bar, 0, 1)),
        ])
    } else {
        Stack::vertical(space::S).child("identity", identity).child("bar", bar).into()
    }
}

/// ↑ ↓ and, inside a menu, out of the group / into the one above; then copy
/// and delete. A button that can't act is disabled and says why.
fn move_bar(settings: &Settings, ui: &MenuUi<'_>, id: ItemId, is_root: bool, context: &ViewContext<ConfigApp>) -> View {
    let can = |dir| tree::can_move(settings, id, dir, ui.root);
    let go = move |dir| context.callback(move |()| Msg::Menu(MenuMsg::Move { id, dir }));

    let (up, down) = if is_root {
        ("Move this tray icon earlier", "Move this tray icon later")
    } else {
        ("Move up", "Move down")
    };

    let mut bar = Stack::horizontal(space::XS)
        .child("up", arrow_button(true, up, can(MoveDir::Up), go(MoveDir::Up)))
        .child("down", arrow_button(false, down, can(MoveDir::Down), go(MoveDir::Down)));

    if !is_root {
        let into = tree::group_above(settings, id).and_then(|group| tree::item(settings, group));
        let into_tip = match into {
            Some(group) => format!("Into the group above: end of {}", roots::label(group)),
            None => "There is no group above to move into".to_owned(),
        };
        bar = bar
            .child(
                "out",
                icon_button_enabled(
                    Symbol::Back,
                    if can(MoveDir::Out) { "Out of its group: right after it" } else { "Already at the top of this menu" },
                    can(MoveDir::Out),
                    go(MoveDir::Out),
                ),
            )
            .child("into", icon_button_enabled(Symbol::Forward, &into_tip, can(MoveDir::Into), go(MoveDir::Into)))
            .child(
                "copy",
                icon_button(
                    Symbol::Copy,
                    "Duplicate (without its hotkeys: a combination belongs to one entry)",
                    context.callback(move |()| Msg::Menu(MenuMsg::Duplicate(id))),
                ),
            );
    }

    let delete: View = if is_root {
        icon_button(Symbol::Delete, "Delete this tray menu", context.callback(move |()| Msg::DeleteAsked(id)))
    } else {
        icon_button(Symbol::Delete, "Delete", context.callback(move |()| Msg::Menu(MenuMsg::Delete(id))))
    };
    bar.child("delete", delete).into()
}

/// Deleting a tray menu removes a tray icon, so it asks.
fn confirm_delete(id: ItemId, context: &ViewContext<ConfigApp>) -> View {
    Stack::vertical(space::S)
        .child("ask", text::body("Remove this tray icon and everything in it?"))
        .child(
            "buttons",
            Stack::horizontal(space::S)
                .child("yes", button("Delete", true, true, context.callback(move |()| Msg::DeleteConfirmed(id))))
                .child("no", button("Keep it", false, true, context.callback(|()| Msg::DeleteCancelled))),
        )
        .into()
}

// ---- fields ----------------------------------------------------------

fn fields(settings: &Settings, item: &Item, is_root: bool, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let id = item.id;
    let text_of = move |field: TextField| context.callback(move |text: String| Msg::Menu(MenuMsg::Text(id, field, text)));
    let name = |placeholder: &str| {
        field("Name", text_box(item.name.clone().unwrap_or_default(), placeholder, text_of(TextField::Name)))
    };
    let tooltip = || field("Tooltip", text_box(item.tooltip.clone().unwrap_or_default(), "Shown when the pointer rests on it", text_of(TextField::Tooltip)));
    let icon = || {
        field(
            "Icon",
            text_box(item.icon.clone().unwrap_or_default(), "From its target, or path,index", text_of(TextField::Icon)),
        )
    };
    let grid = |items: Vec<(&'static str, View)>| AutoGrid::new(200.0).max_columns(2).items(items).build(viewport);
    let flag = move |label: &str, field: FlagField, on: bool| {
        labeled_switch(label, on, context.callback(move |on| Msg::Menu(MenuMsg::Flag(id, field, on))))
    };

    match &item.kind {
        ItemKind::Break => note("Everything after this starts a new column in the menu. There is nothing to set: move it to change where the column starts."),

        ItemKind::Separator => Stack::vertical(space::M)
            .child("title", field("Title", text_box(item.name.clone().unwrap_or_default(), "Empty for a plain line", text_of(TextField::Name))))
            .child("hint", text::caption("With a title the line becomes a section header."))
            .into(),

        ItemKind::App(app) => {
            let target = field(
                if app.browse { "Folder" } else { "Program or file" },
                text_box(app.exe.clone().unwrap_or_default(), r"%programfiles%\App\app.exe", text_of(TextField::Target)),
            );
            Stack::vertical(space::L)
                .child("fields", grid(vec![("name", name("Name in the menu")), ("target", target), ("tooltip", tooltip()), ("icon", icon())]))
                .child("default", flag("Default item (shown in bold)", FlagField::Default, item.default))
                .child("hotkeys", hotkeys_section(settings, item, context))
                .child("more", text::caption("Arguments, the working folder and the launch options come with the full app form."))
                .maybe("menu", is_root.then(|| turn_into_menu(id, context)))
                .into()
        }

        ItemKind::Group => {
            let shortcuts_default = if settings.globals.show_shortcuts.unwrap_or(true) { "on" } else { "off" };
            let shortcuts = field(
                "Show shortcuts",
                combo(
                    [format!("Inherit ({shortcuts_default})"), "On".to_owned(), "Off".to_owned()],
                    Some(match item.show_shortcuts {
                        None => 0,
                        Some(true) => 1,
                        Some(false) => 2,
                    }),
                    context.callback(move |choice: Option<usize>| match choice {
                        Some(choice) => Msg::Menu(MenuMsg::ShowShortcuts(id, choice)),
                        None => Msg::Ignored,
                    }),
                ),
            );
            let renderer_default = match settings.globals.popup {
                Some(y_pop::settings::Renderer::Overlay) => "overlay",
                _ => "native",
            };
            let style = field(
                "Menu style",
                combo(
                    [format!("Inherit ({renderer_default})"), "Native".to_owned(), "Overlay".to_owned()],
                    Some(match item.popup {
                        None => 0,
                        Some(y_pop::settings::Renderer::Native) => 1,
                        Some(y_pop::settings::Renderer::Overlay) => 2,
                    }),
                    context.callback(move |choice: Option<usize>| match choice {
                        Some(choice) => Msg::Menu(MenuMsg::PopupStyle(id, choice)),
                        None => Msg::Ignored,
                    }),
                ),
            );

            let mut switches = Stack::vertical(space::S).child("default", flag("Default item (shown in bold)", FlagField::Default, item.default));
            if is_root {
                switches = switches.child("hover", flag("Open when the pointer rests on its tray icon", FlagField::OpenOnHover, item.auto_open));
            }
            Stack::vertical(space::L)
                .child("fields", grid(vec![("name", name("Name in the menu")), ("tooltip", tooltip()), ("icon", icon()), ("shortcuts", shortcuts), ("style", style)]))
                .child("switches", switches)
                .child("hotkeys", hotkeys_section(settings, item, context))
                .into()
        }

        // About, Quit, Settings: a name (y-pop has its own word for each) and an icon.
        _ => {
            let placeholder = match item.kind {
                ItemKind::About => "About",
                ItemKind::Quit => "Quit",
                _ => "Settings…",
            };
            let mut form = Stack::vertical(space::L)
                .child("fields", grid(vec![("name", name(placeholder)), ("icon", icon())]))
                .child("default", flag("Default item (shown in bold)", FlagField::Default, item.default));
            if is_root {
                form = form.child("hotkeys", hotkeys_section(settings, item, context)).child("menu", turn_into_menu(id, context));
            }
            form.into()
        }
    }
}

/// A tray icon that launches something can become a menu that holds it.
fn turn_into_menu(id: ItemId, context: &ViewContext<ConfigApp>) -> View {
    Stack::vertical(space::S)
        .child("explanation", text::caption("This tray icon launches it on a click. As a menu it can hold more entries; this one becomes the first."))
        .child("button", button("Turn into a menu", false, true, context.callback(move |()| Msg::Menu(MenuMsg::TurnIntoMenu(id)))))
        .into()
}

/// The item's hotkeys: each as keycaps with Edit (opens the shared key picker
/// in a dialog), an Override switch and Remove; then "Add hotkey". A
/// combination used by more than one hotkey in the file is flagged — the
/// second one to register would silently lose.
fn hotkeys_section(
    settings: &Settings,
    item: &Item,
    context: &ViewContext<ConfigApp>,
) -> View {
    let id = item.id;
    let mut rows = Stack::vertical(space::M);
    if item.hotkeys.is_empty() {
        rows = rows.child("none", text::caption("None"));
    }

    for (index, entry) in item.hotkeys.iter().enumerate() {
        let (modifiers, key) = split_hotkey(&entry.key);
        let labels = combo_labels(modifiers, key);

        let shown: View = if labels.is_empty() { text::caption("No key yet").into() } else { keycaps(&labels, false) };
        let line = Stack::horizontal(space::S)
            .child("keys", shown)
            .child("edit", button("Edit", false, true, context.callback(move |()| Msg::Menu(MenuMsg::HotkeyEdit(id, index)))))
            .child(
                "remove",
                icon_button(Symbol::Delete, "Remove this hotkey", context.callback(move |()| Msg::Menu(MenuMsg::HotkeyRemove(id, index)))),
            );

        let mut row = Stack::vertical(space::S).child("line", line).child(
            "override",
            labeled_switch(
                "Override: take the combination even if another program holds it",
                entry.can_override,
                context.callback(move |on| Msg::Menu(MenuMsg::HotkeyOverride(id, index, on))),
            ),
        );
        if let Some(other) = shared_with(settings, item, &entry.key) {
            row = row.child("conflict", text::caption(format!("Also used by {other}: only one of them will work.")));
        }
        rows = rows.child(format!("hotkey{index}"), row);
    }

    rows = rows.child("add", add_button("Add hotkey", context.callback(move |()| Msg::Menu(MenuMsg::HotkeyAdd(id)))));
    field("Hotkeys", rows)
}

/// The key picker in a modal with OK / Cancel, over the whole page, while a
/// hotkey is being edited. The edit commits live, so the keycaps behind it
/// follow; Cancel restores the old text.
pub fn hotkey_modal(
    settings: &Settings,
    hotkeys: &HotkeyUi,
    window: Viewport,
    context: &ViewContext<ConfigApp>,
) -> Option<View> {
    let (id, index) = hotkeys.open?;
    let item = tree::item(settings, id)?;
    let entry = item.hotkeys.get(index)?;
    let (modifiers, key) = split_hotkey(&entry.key);
    let events = KeyPickerEvents {
        on_key: context.callback(move |key| Msg::Menu(MenuMsg::HotkeyKey(id, index, key))),
        on_modifier: context.callback(move |modifier| Msg::Menu(MenuMsg::HotkeyModifier(id, index, modifier))),
        on_category: context.callback(|category| Msg::Menu(MenuMsg::HotkeyCategory(category))),
        on_search_changed: context.callback(|text| Msg::Menu(MenuMsg::HotkeySearch(text))),
        on_search_chosen: context.callback(move |text| Msg::Menu(MenuMsg::HotkeySearchChosen(id, index, text))),
    };
    Some(modal(
        window,
        format!("Hotkey for {}", roots::label(item)),
        |viewport| key_picker(key, Some(modifiers), &hotkeys.picker, viewport, &events),
        // A combination needs a key: OK waits for one.
        key.is_some(),
        context.callback(|()| Msg::Menu(MenuMsg::HotkeyClosed { accepted: false })),
        context.callback(|()| Msg::Menu(MenuMsg::HotkeyClosed { accepted: true })),
    ))
}

/// The label of another item that has the same combination as `raw`, if any.
fn shared_with(settings: &Settings, item: &Item, raw: &str) -> Option<String> {
    let canonical = canonical_hotkey(raw)?;
    let mut seen_here = 0;
    for other in settings.items() {
        for entry in &other.hotkeys {
            if canonical_hotkey(&entry.key).as_deref() != Some(canonical.as_str()) {
                continue;
            }
            if other.id != item.id {
                return Some(roots::label(other));
            }
            seen_here += 1;
        }
    }
    (seen_here > 1).then(|| "another hotkey of this item".to_owned())
}

// ---- the XML preview -------------------------------------------------

/// What this item is in `y-pop.xml`: written by the same code as a save.
fn xml_preview(item: &Item, ui: &MenuUi<'_>, context: &ViewContext<ConfigApp>) -> View {
    let toggle = Button::new()
        .horizontal_alignment(HorizontalAlignment::Left)
        .on_click(context.callback(|()| Msg::Menu(MenuMsg::XmlToggled)))
        .content(if ui.show_xml { "Hide what is written to y-pop.xml" } else { "Show what is written to y-pop.xml" });

    let mut preview = Stack::vertical(space::S).child("toggle", toggle);
    if ui.show_xml {
        preview = preview.child(
            "xml",
            Border::new()
                .padding(Thickness::uniform(space::M))
                .corner_radius(CornerRadius::uniform(radius::CONTROL))
                .background(ThemeBrush::CardBackground)
                .content(text::caption(item.to_xml())),
        );
    }
    preview.into()
}
