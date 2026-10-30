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

use crate::menu_edit::{DropTarget, MenuMsg};
use crate::start_menu::Source;
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
    /// The app picker is open.
    pub app_picker: bool,
    /// What a dragged file is over.
    pub drag_over: Option<DropTarget>,
    /// How the selected item was made, after a drop.
    pub origin: Option<&'a crate::from_file::Origin>,
    pub show_xml: bool,
    pub show_comments: bool,
    /// Resolves `%builtins%` for the target readout; filled lazily.
    pub builtins: &'a std::cell::RefCell<y_pop::builtins::Builtins>,
    pub icons: &'a crate::app_icons::IconStore,
    /// "Delete this tray menu?" is waiting for an answer.
    pub confirming_delete: bool,
}

/// Each level of nesting indents a row by this much.
const INDENT: f64 = space::M;

/// How far a row at `depth` is indented. The menu itself (depth 0) is the
/// outline's heading and its items start flush left, so only what is inside a
/// group is indented.
fn indent_of(depth: usize) -> f64 {
    depth.saturating_sub(1) as f64 * INDENT
}
/// The room the expand button takes at the start of a row, empty or not.
const EXPAND_SLOT: f64 = size::ICON_BUTTON;
/// Narrowest the outline and the inspector are useful at, side by side.
const TREE_MIN: f64 = 300.0;
const INSPECTOR_MIN: f64 = 320.0;

pub fn menu_view(
    settings: &Settings,
    ui: &MenuUi<'_>,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let Some(root) = tree::item(settings, ui.root) else {
        return page(
            "Tray menu",
            "This tray menu is no longer in the file.",
            View::empty(),
        );
    };

    let hotkeys: Vec<&str> = root
        .hotkeys
        .iter()
        .map(|hotkey| hotkey.key.as_str())
        .collect();
    let opened_by = if hotkeys.is_empty() {
        String::new()
    } else {
        format!(", also opened with {}", hotkeys.join(", "))
    };

    // An app (or anything that isn't a group) has no tree: its inspector is
    // the whole screen.
    if root.kind != ItemKind::Group {
        let lede =
            format!("A tray icon{opened_by}. It has no menu: turn it into one to hold more items.");
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

    let split = split(viewport);
    if split.is_side_by_side() {
        // Pinned: the page doesn't scroll. The outline scrolls its rows under
        // its toolbar, and the inspector scrolls on its own.
        let outline = outline_pane(settings, ui, split.primary(), true, context);
        let inspector = pane_scroll(crate::inspector::inspector_view(
            settings,
            ui,
            ui.selected,
            pane_viewport(split.secondary()),
            context,
        ));
        let header = page(&roots::label(root), &lede, View::empty());
        return Grid::new()
            .rows([GridLength::Auto, GridLength::STAR])
            .row_spacing(space::XL)
            .keyed_children([
                ("header", cell(header, 0, 0)),
                ("panes", cell(split.build(outline, inspector), 1, 0)),
            ]);
    }

    let outline = outline_pane(settings, ui, split.primary(), false, context);
    let inspector =
        crate::inspector::inspector_view(settings, ui, ui.selected, split.secondary(), context);
    page(&roots::label(root), &lede, split.build(outline, inspector))
}

fn split(viewport: Viewport) -> Split {
    Split::new(viewport)
        .min_widths(TREE_MIN, INSPECTOR_MIN)
        .weights(1.0, 1.0)
}

/// Whether the screen shows the outline and the inspector side by side, each
/// scrolling on its own — then the page around it must not scroll
/// (`shell::page_frame_pinned`).
pub fn is_pinned(settings: &Settings, root: ItemId, viewport: Viewport) -> bool {
    tree::item(settings, root).is_some_and(|root| root.kind == ItemKind::Group)
        && split(viewport).is_side_by_side()
}

/// What a pane's scrollbar takes from its width, so it never covers a card.
const SCROLLBAR: f64 = space::M;

fn pane_viewport(viewport: Viewport) -> Viewport {
    Viewport::new(viewport.width - SCROLLBAR)
}

fn pane_scroll(pane: View) -> View {
    scroll_y(
        Border::new()
            .padding(Thickness::new(0.0, 0.0, SCROLLBAR, 0.0))
            .content(pane),
    )
}

/// `pinned`: the pane has the window's height to itself, and scrolls its rows
/// under a fixed toolbar.
fn outline_pane(
    settings: &Settings,
    ui: &MenuUi<'_>,
    viewport: Viewport,
    pinned: bool,
    context: &ViewContext<ConfigApp>,
) -> View {
    // What the toolbar gets: the card's own width, inside its padding.
    let inner = viewport.inset(CARD_INSET);
    let at = tree::insertion_point(settings, ui.root, Some(ui.selected), ui.collapsed);
    let rows = tree::rows(settings, ui.root, ui.collapsed, ui.show_comments, at);

    let mut outline = Stack::vertical(space::XXS);
    let mut other = 0;
    for row in &rows {
        let indent = indent_of(row.depth);
        match &row.kind {
            TreeRowKind::Item {
                id,
                hidden_comments,
            } => {
                if let Some(item) = tree::item(settings, *id) {
                    let view = item_row(settings, item, row.depth, *hidden_comments, ui, context);
                    let view = drop_target(view, DropTarget::Row(*id), ui, context);
                    outline = outline.child(format!("i{}", id.get()), view);
                }
            }
            TreeRowKind::Comment {
                first_line, lines, ..
            } => {
                other += 1;
                outline =
                    outline.child(format!("c{other}"), comment_row(first_line, *lines, indent));
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

    outline = outline.child("drop", drop_zone(ui, context));

    // Showing comments is a toggle in the status bar, not here.
    let toolbar = Stack::vertical(space::S)
        .child("add", add_row(inner, context))
        .maybe("picker", ui.app_picker.then(|| app_picker(inner, context)))
        .maybe("more", ui.more_open.then(|| more_row(inner, context)));

    if pinned {
        // The toolbar stays put; only the rows scroll under it, so a new item
        // can be added wherever the selection is.
        let rows = scroll_y(
            Border::new()
                .padding(Thickness::new(0.0, 0.0, SCROLLBAR, 0.0))
                .content(outline),
        );
        return card().content(
            Grid::new()
                .rows([GridLength::Auto, GridLength::STAR])
                .row_spacing(space::M)
                .keyed_children([("toolbar", cell(toolbar, 0, 0)), ("rows", cell(rows, 1, 0))]),
        );
    }
    card().content(
        Stack::vertical(space::M)
            .child("toolbar", toolbar)
            .child("rows", outline),
    )
}

// ---- the toolbar -----------------------------------------------------

/// A toolbar button that fills its cell: an icon and a label.
fn tool(symbol: Symbol, label: &str, accent: bool, on_click: Callback<()>) -> View {
    Button::new()
        .style(if accent {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Default
        })
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
            (
                "app",
                tool(
                    Symbol::AllApps,
                    "App…",
                    true,
                    context.callback(|()| Msg::Menu(MenuMsg::AppPickerToggled)),
                ),
            ),
            (
                "group",
                tool(Symbol::Folder, "Group", false, add(NewKind::Group)),
            ),
            (
                "separator",
                tool(Symbol::Remove, "Separator", false, add(NewKind::Separator)),
            ),
            (
                "more",
                tool(
                    Symbol::More,
                    "More",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::MoreToggled)),
                ),
            ),
        ])
        .build(viewport)
}

/// Where an app comes from. The choice of source is a row of buttons inline
/// under the toolbar; the Start menu's long list opens as a dialog instead
/// ([`start_modal`]), so it never pushes the outline out of view. The Store
/// source (8.6) joins them.
fn app_picker(viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let grid = AutoGrid::new(120.0)
        .max_columns(4)
        .gap(space::S, space::S)
        .items([
            (
                "browse",
                tool(
                    Symbol::OpenFile,
                    "Browse…",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::AppBrowse)),
                ),
            ),
            (
                "start",
                tool(
                    Symbol::AllApps,
                    "Start menu…",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::StartOpened(Source::Shortcuts, None))),
                ),
            ),
            (
                "store",
                tool(
                    Symbol::Shop,
                    "Store apps…",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::StartOpened(Source::Store, None))),
                ),
            ),
            (
                "running",
                tool(
                    Symbol::Play,
                    "Running apps…",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::StartOpened(Source::Running, None))),
                ),
            ),
            (
                "manual",
                tool(
                    Symbol::Edit,
                    "Manual",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::Add(NewKind::App))),
                ),
            ),
            (
                "close",
                tool(
                    Symbol::Cancel,
                    "Close",
                    false,
                    context.callback(|()| Msg::Menu(MenuMsg::AppPickerToggled)),
                ),
            ),
        ])
        .build(viewport);
    Stack::vertical(space::S)
        .child("title", text::section_title("Add an app"))
        .child("grid", grid)
        .child(
            "hint",
            text::caption(
                "Browse adds several programs, shortcuts or files at once; Start menu, Store apps and Running apps pick from a list; Manual adds an empty app to fill in.",
            ),
        )
        .into()
}

fn more_row(viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let add = |kind: NewKind| context.callback(move |()| Msg::Menu(MenuMsg::Add(kind)));
    AutoGrid::new(120.0)
        .max_columns(3)
        .gap(space::S, space::S)
        .items([
            (
                "titled",
                tool(Symbol::Tag, "Titled separator", false, add(NewKind::Titled)),
            ),
            (
                "folder",
                tool(
                    Symbol::Folder,
                    "Folder (browse)",
                    false,
                    add(NewKind::Folder),
                ),
            ),
            (
                "break",
                tool(Symbol::ViewAll, "Column break", false, add(NewKind::Break)),
            ),
            (
                "about",
                tool(Symbol::Help, "About", false, add(NewKind::About)),
            ),
            (
                "quit",
                tool(Symbol::Cancel, "Quit", false, add(NewKind::Quit)),
            ),
            (
                "settings",
                tool(Symbol::Setting, "Settings", false, add(NewKind::Settings)),
            ),
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
    let indent = indent_of(depth);

    // Groups below the menu itself fold; everything else has an empty slot of
    // the same width, so names line up. The menu's own row has no slot.
    let expand: View = if depth == 0 {
        Border::new().width(0.0).into()
    } else if item.kind == ItemKind::Group {
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
        .style(if selected {
            ButtonStyle::Accent
        } else {
            ButtonStyle::Subtle
        })
        .fill()
        .horizontal_content_alignment(HorizontalAlignment::Stretch)
        .on_click(context.callback(move |()| Msg::Menu(MenuMsg::Select(id))))
        .content(body);

    let trailing: View = if hidden_comments > 0 {
        icon_button(
            Symbol::Comment,
            &format!(
                "{hidden_comments} hidden comment{} — click to show",
                if hidden_comments == 1 { "" } else { "s" }
            ),
            context.callback(|()| Msg::ShowCommentsToggled(true)),
        )
    } else {
        Border::new().width(0.0).into()
    };

    Grid::new()
        .columns([GridLength::Auto, GridLength::STAR, GridLength::Auto])
        .keyed_children([
            (
                "expand",
                cell(
                    Border::new()
                        .margin(Thickness::new(indent, 0.0, 0.0, 0.0))
                        .content(expand),
                    0,
                    0,
                ),
            ),
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
                        (
                            "title",
                            cell(
                                text::caption(title.to_owned())
                                    .vertical_alignment(VerticalAlignment::Center),
                                0,
                                0,
                            ),
                        ),
                        ("rule", cell(rule, 0, 1)),
                    ]),
                None => rule,
            }
        }
        ItemKind::Break => text::caption("— column break —").into(),
        kind => {
            let mut parts = Stack::horizontal(space::S)
                .child(
                    "icon",
                    ui.item_icon(settings, item, crate::app_icons::SHOWN),
                )
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
                parts = parts.child(
                    "count",
                    text::caption(tree::count_inside(settings, item.id).to_string()),
                );
            }
            for (index, hotkey) in item.hotkeys.iter().enumerate() {
                parts = parts.child(
                    format!("key{index}"),
                    keycaps(std::slice::from_ref(&hotkey.key), false),
                );
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
        .child(
            "icon",
            SymbolIcon::new()
                .symbol(Symbol::Comment)
                .opacity(emphasis::SUBTLE),
        )
        .child(
            "text",
            text::single_line(first_line.to_owned()).opacity(emphasis::MUTED),
        );
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
    Border::new()
        .margin(Thickness::new(indent + EXPAND_SLOT, 0.0, 0.0, 0.0))
        .padding(Thickness::xy(space::M, 0.0))
        .content(
            Stack::horizontal(space::S)
                .child("bar", bar)
                .child("text", text::caption("next item goes here")),
        )
}

/// Comments with no item after them to hang a marker on.
fn hidden_row(count: usize, indent: f64, context: &ViewContext<ConfigApp>) -> View {
    let marker = icon_button(
        Symbol::Comment,
        &format!(
            "{count} hidden comment{} — click to show",
            if count == 1 { "" } else { "s" }
        ),
        context.callback(|()| Msg::ShowCommentsToggled(true)),
    );
    Border::new()
        .margin(Thickness::new(indent + EXPAND_SLOT, 0.0, 0.0, 0.0))
        .content(
            Stack::horizontal(space::S)
                .child("marker", marker)
                .child("text", text::caption("hidden at the end")),
        )
}

impl MenuUi<'_> {
    /// `item`'s icon, `size` DIPs square: the real one once it has loaded,
    /// its symbol until then (and where it has none).
    pub fn item_icon(&self, settings: &Settings, item: &Item, size: f64) -> View {
        let spec = crate::app_icons::spec_for(settings, item, &mut *self.builtins.borrow_mut());
        crate::app_icons::icon_or_symbol(
            self.icons.image(spec.as_ref(), size),
            symbol_of(&item.kind),
            size,
        )
    }
}

// ---- dropping files --------------------------------------------------

/// Makes `view` take dropped files: a row accepts them (spec T5/§3.3), is
/// outlined while one is over it, and sends the paths in drop order.
fn drop_target(
    view: impl Into<View>,
    target: DropTarget,
    ui: &MenuUi<'_>,
    context: &ViewContext<ConfigApp>,
) -> View {
    let over = ui.drag_over == Some(target);
    Border::new()
        .border_thickness(Thickness::new(2.0, 2.0, 2.0, 2.0))
        .corner_radius(CornerRadius::uniform(radius::CONTROL))
        .border_brush_optional(over.then_some(ThemeBrush::Accent))
        .drop_policy(
            DragDropPolicy::new()
                .storage_items(DragDropAction::new(DragDropOperation::Copy).caption("Add to menu")),
        )
        .on_drag_enter(context.callback(move |kind: DragKind| match kind {
            DragKind::StorageItems => Msg::Menu(MenuMsg::DragOver(Some(target))),
            _ => Msg::Ignored,
        }))
        .on_drag_leave(context.callback(|()| Msg::Menu(MenuMsg::DragOver(None))))
        .on_drop(context.callback(move |data: DroppedData| match data {
            DroppedData::StorageItems(items) => Msg::Menu(MenuMsg::Dropped(
                target,
                items.into_iter().map(|item| item.path).collect(),
            )),
            _ => Msg::Menu(MenuMsg::DragOver(None)),
        }))
        .content(view)
}

/// The strip under the rows: drop files here to add them where the marker is.
fn drop_zone(ui: &MenuUi<'_>, context: &ViewContext<ConfigApp>) -> View {
    let hint = Border::new()
        .padding(Thickness::xy(space::M, space::S))
        .content(text::caption(
            "Drop programs, shortcuts or folders here to add them at the marker (Start menu apps: use the picker)",
        ));
    drop_target(hint, DropTarget::Zone, ui, context)
}

// ---- the Start menu source ---------------------------------------------

/// How tall the list of shortcuts is in its dialog before it scrolls.
const START_LIST_HEIGHT: f64 = 380.0;

/// The lower-case `exe` of every app already in this tray menu, expanded, and
/// `shell:AppsFolder\<id>` for every Store app (the path of its picker entry).
fn programs_in_menu(
    settings: &Settings,
    root: ItemId,
    builtins: &std::cell::RefCell<y_pop::builtins::Builtins>,
) -> HashSet<String> {
    let mut builtins = builtins.borrow_mut();
    settings
        .all_items()
        .filter(|item| tree::is_inside(settings, root, item.id))
        .filter_map(|item| match &item.kind {
            ItemKind::App(app) => match (&app.app_id, &app.exe) {
                (Some(id), _) => Some(format!(r"{}\{id}", y_core::apps_folder::FOLDER)),
                (None, Some(exe)) => Some(exe.clone()),
                (None, None) => None,
            },
            _ => None,
        })
        .map(|exe| {
            settings
                .expand_text(&exe, &mut *builtins)
                .trim()
                .trim_matches('"')
                .to_lowercase()
        })
        .collect()
}

/// What [`start_modal`] needs besides the settings.
pub struct StartUi<'a> {
    /// The tray menu the apps are added to.
    pub root: ItemId,
    pub start: &'a crate::start_menu::StartMenu,
    pub entries: &'a [crate::start_menu::StartEntry],
    pub builtins: &'a std::cell::RefCell<y_pop::builtins::Builtins>,
    pub icons: &'a crate::app_icons::IconStore,
}

/// Search, tick, add: every shortcut under the Start menu's `Programs`
/// folders, or every Store app, in a dialog over the whole window so the
/// outline keeps its place underneath. Entries already in this menu are
/// flagged, not hidden. With a target item it picks one Store app for it.
pub fn start_modal(
    settings: &Settings,
    ui: &StartUi<'_>,
    window: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let count = ui.start.checked.len();
    let ok = context.callback(|()| Msg::Menu(MenuMsg::StartAdd));
    let cancel = context.callback(|()| Msg::Menu(MenuMsg::StartClosed));
    let title = match (ui.start.target, ui.start.source, count) {
        (Some(_), _, _) => "Choose a Store app".to_owned(),
        (None, Source::Shortcuts, 0) => "Add apps from the Start menu".to_owned(),
        (None, Source::Store, 0) => "Add Store apps".to_owned(),
        (None, Source::Running, 0) => "Add running apps".to_owned(),
        (None, Source::Shortcuts, n) => format!("Add apps from the Start menu — {n} ticked"),
        (None, Source::Store, n) => format!("Add Store apps — {n} ticked"),
        (None, Source::Running, n) => format!("Add running apps — {n} ticked"),
    };
    crate::ui::editor::modal(
        window,
        title,
        |_| start_list(settings, ui, context),
        count > 0,
        cancel,
        ok,
    )
}

fn start_list(settings: &Settings, ui: &StartUi<'_>, context: &ViewContext<ConfigApp>) -> View {
    if !ui.start.ready() {
        return text::caption(match ui.start.source {
            Source::Shortcuts => "Reading the Start menu…",
            Source::Store => "Reading the Store apps…",
            Source::Running => "Looking for open windows…",
        })
        .into();
    }

    let (listed, more) = ui.start.listed(ui.entries);
    let present = programs_in_menu(settings, ui.root, ui.builtins);
    let mut list = Stack::vertical(space::XXS);
    for entry in &listed {
        let path = entry.path.clone();
        let spec = entry.icon_spec();
        let icon = crate::app_icons::icon_or_symbol(
            ui.icons.image(Some(&spec), crate::app_icons::SHOWN),
            Symbol::AllApps,
            crate::app_icons::SHOWN,
        );
        let already = [entry.target.as_str(), entry.path.as_str()]
            .iter()
            .any(|text| !text.is_empty() && present.contains(&text.to_lowercase()));
        let mut label = Stack::horizontal(space::S)
            .child("icon", icon)
            .child("name", text::single_line(entry.name.clone()));
        if already {
            label = label.child("already", text::caption("already in this menu"));
        }
        let check = CheckBox::new()
            .is_checked(ui.start.checked.contains(&entry.path))
            .on_is_checked_changed(
                context.callback(move |_: bool| Msg::Menu(MenuMsg::StartToggle(path.clone()))),
            )
            .content(label);
        list = list.child(entry.path.clone(), check);
    }
    if listed.is_empty() {
        list = list.child("none", text::caption("Nothing matches."));
    }
    if more > 0 {
        list = list.child(
            "more",
            button(
                &format!("Show more ({more} left)"),
                false,
                true,
                context.callback(|()| Msg::Menu(MenuMsg::StartMore)),
            ),
        );
    }

    Stack::vertical(space::S)
        .child(
            "search",
            text_box(
                ui.start.search.clone(),
                "Search the Start menu",
                context.callback(|text: String| Msg::Menu(MenuMsg::StartSearch(text))),
            ),
        )
        .child(
            "list",
            Border::new()
                .height(START_LIST_HEIGHT)
                .content(scroll_y(list)),
        )
        .into()
}
