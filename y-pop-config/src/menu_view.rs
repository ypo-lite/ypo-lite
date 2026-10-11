//! The tray menu screen: the outline of one menu on the left, the inspector
//! of what is selected on the right (under it when the window is narrow).
//!
//! The outline is made of rows rather than WinUI's `TreeView`, which has no
//! icon, no hint column, and no way to draw a separator or a column break as
//! anything but text. A row is a `Grid`: the expand button, the row itself (a
//! `Button`, so it takes focus and Enter like any other), and a trailing slot.
//! The selected row is the Accent one. A row never nests another button, which
//! WinUI doesn't allow.

use std::collections::HashSet;

use y_pop::settings::{Item, ItemId, ItemKind, Settings};

use crate::menu_edit::MenuMsg;
use crate::tree::{self, NewKind, TreeRowKind};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg, roots};
use windows_reactor::*;

/// What the screen needs besides the settings.
pub struct MenuUi<'a> {
    pub root: ItemId,
    pub selected: ItemId,
    pub collapsed: &'a HashSet<ItemId>,
    pub more_open: bool,
    pub show_xml: bool,
    pub show_comments: bool,
    /// "Delete this tray menu?" is waiting for an answer.
    pub confirming_delete: bool,
}

/// Each level of nesting indents a row by this much.
const INDENT: f64 = space::L;
/// The room the expand button takes at the start of a row, empty or not.
const EXPAND_SLOT: f64 = size::ICON_BUTTON;
/// Narrowest the outline and the inspector are useful at, side by side.
const TREE_MIN: f64 = 300.0;
const INSPECTOR_MIN: f64 = 320.0;

pub fn menu_view(settings: &Settings, ui: &MenuUi<'_>, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let Some(root) = tree::item(settings, ui.root) else {
        return page("Tray menu", "This tray menu is no longer in the file.", View::empty());
    };

    let hotkeys: Vec<&str> = root.hotkeys.iter().map(|hotkey| hotkey.key.as_str()).collect();
    let opened_by = if hotkeys.is_empty() { String::new() } else { format!(", also opened with {}", hotkeys.join(", ")) };

    // An app (or anything that isn't a group) has no tree: its inspector is
    // the whole screen.
    if root.kind != ItemKind::Group {
        let lede = format!("A tray icon{opened_by}. It has no menu: turn it into one to hold more items.");
        return page(
            &roots::label(root),
            &lede,
            crate::inspector::inspector_view(settings, ui, root.id, viewport, context),
        );
    }

    let lede = format!(
        "Its own tray icon{opened_by}. Select a row to edit it. New items go where the marker is: after the \
         selected row, or inside a selected open group."
    );

    let split = Split::new(viewport).min_widths(TREE_MIN, INSPECTOR_MIN).weights(1.0, 1.0);
    let outline = outline_pane(settings, ui, split.primary(), context);
    let inspector = crate::inspector::inspector_view(settings, ui, ui.selected, split.secondary(), context);

    page(&roots::label(root), &lede, split.build(outline, inspector))
}

fn outline_pane(settings: &Settings, ui: &MenuUi<'_>, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    // What the toolbar gets: the card's own width, inside its padding.
    let inner = viewport.inset(CARD_INSET);
    let at = tree::insertion_point(settings, ui.root, Some(ui.selected), ui.collapsed);
    let rows = tree::rows(settings, ui.root, ui.collapsed, ui.show_comments, at);

    let mut outline = Stack::vertical(space::XXS);
    let mut other = 0;
    for row in &rows {
        let indent = row.depth as f64 * INDENT;
        match &row.kind {
            TreeRowKind::Item { id, hidden_comments } => {
                if let Some(item) = tree::item(settings, *id) {
                    let view = item_row(settings, item, row.depth, *hidden_comments, ui, context);
                    outline = outline.child(format!("i{}", id.get()), view);
                }
            }
            TreeRowKind::Comment { first_line, lines, .. } => {
                other += 1;
                outline = outline.child(format!("c{other}"), comment_row(first_line, *lines, indent));
            }
            TreeRowKind::Marker => {
                other += 1;
                outline = outline.child(format!("m{other}"), marker_row(indent));
            }
            TreeRowKind::HiddenAtEnd(count) => {
                other += 1;
                outline = outline.child(format!("h{other}"), hidden_row(*count, indent, context));
            }
        }
    }

    let comments = tree::count_comments(settings, ui.root);
    let toolbar = Stack::vertical(space::S)
        .child("add", add_row(inner, context))
        .maybe("more", ui.more_open.then(|| more_row(inner, context)))
        .child(
            "comments",
            labeled_switch(
                &format!("Show comments ({comments})"),
                ui.show_comments,
                context.callback(Msg::ShowCommentsToggled),
            ),
        );

    card().content(Stack::vertical(space::M).child("toolbar", toolbar).child("rows", outline))
}

// ---- the toolbar -----------------------------------------------------

/// A toolbar button that fills its cell: an icon and a label.
fn tool(symbol: Symbol, label: &str, accent: bool, on_click: Callback<()>) -> View {
    Button::new()
        .style(if accent { ButtonStyle::Accent } else { ButtonStyle::Default })
        .fill()
        .on_click(on_click)
        .content(
            Stack::horizontal(space::S)
                .child("icon", SymbolIcon::new().symbol(symbol))
                .child("label", text::body(label)),
        )
}

fn add_row(viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let add = |kind: NewKind| context.callback(move |()| Msg::Menu(MenuMsg::Add(kind)));
    AutoGrid::new(96.0)
        .max_columns(4)
        .gap(space::S, space::S)
        .items([
            // Phase 8 turns this into the app picker; until then it adds an
            // empty app to fill in.
            ("app", tool(Symbol::AllApps, "App", true, add(NewKind::App))),
            ("group", tool(Symbol::Folder, "Group", false, add(NewKind::Group))),
            ("separator", tool(Symbol::Remove, "Separator", false, add(NewKind::Separator))),
            ("more", tool(Symbol::More, "More", false, context.callback(|()| Msg::Menu(MenuMsg::MoreToggled)))),
        ])
        .build(viewport)
}

fn more_row(viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let add = |kind: NewKind| context.callback(move |()| Msg::Menu(MenuMsg::Add(kind)));
    AutoGrid::new(120.0)
        .max_columns(3)
        .gap(space::S, space::S)
        .items([
            ("titled", tool(Symbol::Tag, "Titled separator", false, add(NewKind::Titled))),
            ("folder", tool(Symbol::Folder, "Folder (browse)", false, add(NewKind::Folder))),
            ("break", tool(Symbol::ViewAll, "Column break", false, add(NewKind::Break))),
            ("about", tool(Symbol::Help, "About", false, add(NewKind::About))),
            ("quit", tool(Symbol::Cancel, "Quit", false, add(NewKind::Quit))),
            ("settings", tool(Symbol::Setting, "Settings", false, add(NewKind::Settings))),
        ])
        .build(viewport)
}

// ---- rows ------------------------------------------------------------

/// An item's row: expand button, the row, and what trails it.
fn item_row(
    settings: &Settings,
    item: &Item,
    depth: usize,
    hidden_comments: usize,
    ui: &MenuUi<'_>,
    context: &ViewContext<ConfigApp>,
) -> View {
    let id = item.id;
    let selected = ui.selected == id;
    let indent = depth as f64 * INDENT;

    // Groups below the menu itself fold; everything else has an empty slot of
    // the same width, so names line up.
    let expand: View = if item.kind == ItemKind::Group && depth > 0 {
        let open = !ui.collapsed.contains(&id);
        Button::new()
            .style(ButtonStyle::Subtle)
            .min_width(0.0)
            .width(EXPAND_SLOT)
            .vertical_alignment(VerticalAlignment::Center)
            .on_click(context.callback(move |()| Msg::Menu(MenuMsg::ToggleCollapsed(id))))
            .content(text::caption(if open { "▾" } else { "▸" }))
            .tooltip(if open { "Collapse" } else { "Expand" })
    } else {
        Border::new().width(EXPAND_SLOT).into()
    };

    let body = row_body(settings, item, ui);
    let row: View = Button::new()
        .style(if selected { ButtonStyle::Accent } else { ButtonStyle::Subtle })
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .on_click(context.callback(move |()| Msg::Menu(MenuMsg::Select(id))))
        .content(body);

    let trailing: View = if hidden_comments > 0 {
        icon_button(
            Symbol::Comment,
            &format!("{hidden_comments} hidden comment{} — click to show", if hidden_comments == 1 { "" } else { "s" }),
            context.callback(|()| Msg::ShowCommentsToggled(true)),
        )
    } else {
        Border::new().width(0.0).into()
    };

    Grid::new()
        .columns([GridLength::Auto, GridLength::STAR, GridLength::Auto])
        .keyed_children([
            ("expand", cell(Border::new().margin(Thickness::new(indent, 0.0, 0.0, 0.0)).content(expand), 0, 0)),
            ("row", cell(row, 0, 1)),
            ("trailing", cell(trailing, 0, 2)),
        ])
}

/// What is inside an item's row button.
fn row_body(settings: &Settings, item: &Item, ui: &MenuUi<'_>) -> View {
    match &item.kind {
        ItemKind::Separator => {
            // A rule, with its title in front when it has one.
            let rule: View = Border::new()
                .height(size::STROKE)
                .background(ThemeBrush::CardStroke)
                .vertical_alignment(VerticalAlignment::Center)
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .into();
            match item.name.as_deref().filter(|name| !name.is_empty()) {
                Some(title) => Grid::new()
                    .columns([GridLength::Auto, GridLength::STAR])
                    .column_spacing(space::S)
                    .keyed_children([
                        ("title", cell(text::caption(title.to_owned()).vertical_alignment(VerticalAlignment::Center), 0, 0)),
                        ("rule", cell(rule, 0, 1)),
                    ]),
                None => rule,
            }
        }
        ItemKind::Break => text::caption("— column break —").into(),
        kind => {
            let mut parts = Stack::horizontal(space::S)
                .child("icon", SymbolIcon::new().symbol(symbol_of(kind)))
                .child("name", text::single_line(roots::label(item)));

            if let ItemKind::App(app) = kind {
                if app.browse {
                    parts = parts.child("browse", text::caption("browse"));
                }
                if app.app_id.is_some() {
                    parts = parts.child("store", text::caption("store"));
                }
            }
            if item.kind == ItemKind::Group && ui.collapsed.contains(&item.id) {
                parts = parts.child("count", text::caption(tree::count_inside(settings, item.id).to_string()));
            }
            for (index, hotkey) in item.hotkeys.iter().enumerate() {
                parts = parts.child(format!("key{index}"), keycaps(std::slice::from_ref(&hotkey.key), false));
            }
            parts.into()
        }
    }
}

pub fn symbol_of(kind: &ItemKind) -> Symbol {
    match kind {
        ItemKind::Group => Symbol::Folder,
        ItemKind::App(app) if app.browse => Symbol::Folder,
        ItemKind::App(_) => Symbol::AllApps,
        ItemKind::Separator => Symbol::Remove,
        ItemKind::About => Symbol::Help,
        ItemKind::Quit => Symbol::Cancel,
        ItemKind::Settings => Symbol::Setting,
        ItemKind::Break => Symbol::ViewAll,
    }
}

/// A comment, read-only: muted, one line, with how many lines it really has.
fn comment_row(first_line: &str, lines: usize, indent: f64) -> View {
    let mut parts = Stack::horizontal(space::S)
        .child("icon", SymbolIcon::new().symbol(Symbol::Comment).opacity(emphasis::SUBTLE))
        .child("text", text::single_line(first_line.to_owned()).opacity(emphasis::MUTED));
    if lines > 1 {
        parts = parts.child("lines", text::caption(format!("{lines} lines")));
    }
    Border::new()
        .margin(Thickness::new(indent + EXPAND_SLOT, 0.0, 0.0, 0.0))
        .padding(Thickness::xy(space::M, space::XS))
        .content(parts)
}

/// "The next item goes here."
fn marker_row(indent: f64) -> View {
    let bar: View = Border::new()
        .width(space::XL)
        .height(2.0)
        .background(ThemeBrush::Accent)
        .vertical_alignment(VerticalAlignment::Center)
        .into();
    Border::new().margin(Thickness::new(indent + EXPAND_SLOT, 0.0, 0.0, 0.0)).padding(Thickness::xy(space::M, 0.0)).content(
        Stack::horizontal(space::S).child("bar", bar).child("text", text::caption("next item goes here")),
    )
}

/// Comments with no item after them to hang a marker on.
fn hidden_row(count: usize, indent: f64, context: &ViewContext<ConfigApp>) -> View {
    let marker = icon_button(
        Symbol::Comment,
        &format!("{count} hidden comment{} — click to show", if count == 1 { "" } else { "s" }),
        context.callback(|()| Msg::ShowCommentsToggled(true)),
    );
    Border::new()
        .margin(Thickness::new(indent + EXPAND_SLOT, 0.0, 0.0, 0.0))
        .content(Stack::horizontal(space::S).child("marker", marker).child("text", text::caption("hidden at the end")))
}
