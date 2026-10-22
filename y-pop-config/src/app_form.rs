//! The inspector's form for an `<app>`: what it launches, how, and — for a
//! folder — how the folder is listed. (Name, tooltip, icon, hotkeys and the
//! default switch are shared with the other kinds, and stay in
//! `inspector.rs`.)
//!
//! Every text is the raw attribute, `%variables%` and all; the readout under
//! the target shows what y-pop makes of it and whether it is there.

use y_pop::model::OpenFolder;
use y_pop::settings::{AppFields, Item, ItemId, Settings, VarEntry};

use crate::menu_edit::{FlagField, MenuMsg, TargetMode, TextField, depth_words};
use crate::menu_view::MenuUi;
use crate::target_check::{self, Found};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;

/// `%clipboard%` and `%clipboards%`: not variables of the file, read when the
/// item is launched (only arguments make use of them).
const CLIPBOARD: [&str; 2] = ["clipboard", "clipboards"];

/// Target, launch options, and the folder options when the app is a folder.
pub fn sections(
    settings: &Settings,
    ui: &MenuUi<'_>,
    item: &Item,
    app: &AppFields,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let mut form = Stack::vertical(space::L)
        .child("target", section("Target", target(settings, ui, item.id, app, viewport, context)))
        .child("launch", section("Launch", launch(settings, item.id, app, viewport, context)));
    if TargetMode::of(app) == TargetMode::Folder {
        form = form.child("folder", section("Folder listing", folder(item.id, app, viewport, context)));
    }
    let id = item.id;
    form = form.child(
        "test",
        section(
            "Test",
            Stack::vertical(space::S)
                .child("run", button("Run now", false, true, context.callback(move |()| Msg::Menu(MenuMsg::RunNow(id)))))
                .child("note", text::caption("Starts it exactly as y-pop would, with what is on the clipboard now. The result shows in the status bar.")),
        ),
    );
    form.into()
}

fn text_of(id: ItemId, field: TextField, context: &ViewContext<ConfigApp>) -> Callback<String> {
    context.callback(move |text: String| Msg::Menu(MenuMsg::Text(id, field, text)))
}

/// "Insert variable…": the file's variables, then the builtins (and the
/// clipboard pair where they apply). Keyed by the text it will append to, so
/// after an insertion the box starts over instead of keeping the choice.
fn insert_variable(
    settings: &Settings,
    id: ItemId,
    field: TextField,
    current: &str,
    clipboard: bool,
    context: &ViewContext<ConfigApp>,
) -> View {
    let mut names: Vec<String> = settings
        .variables
        .iter()
        .filter_map(|entry| match entry {
            VarEntry::Var(var) => Some(var.name.clone()),
            VarEntry::Comment(_) => None,
        })
        .collect();
    names.extend(y_pop::builtins::NAMES.iter().map(|name| (*name).to_owned()));
    if clipboard {
        names.extend(CLIPBOARD.iter().map(|name| (*name).to_owned()));
    }
    let shown: Vec<String> = names.iter().map(|name| format!("%{name}%")).collect();
    let picker: View = combo(
        shown,
        None,
        context.callback(move |choice: Option<usize>| match choice.and_then(|index| names.get(index)) {
            Some(name) => Msg::Menu(MenuMsg::InsertVariable(id, field, name.clone())),
            None => Msg::Ignored,
        }),
    )
    .placeholder_text("Insert variable…")
    .into();
    Stack::vertical(0.0).child(format!("insert-{field:?}-{current}"), picker).into()
}

fn target(
    settings: &Settings,
    ui: &MenuUi<'_>,
    id: ItemId,
    app: &AppFields,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let mode = TargetMode::of(app);
    let choices = [Choice::new("Program or file"), Choice::new("Folder as submenu"), Choice::new("Store app")];
    let selected = TargetMode::ALL.iter().position(|&m| m == mode);
    let modes = segmented(
        &choices,
        selected,
        context.callback(move |index: usize| Msg::Menu(MenuMsg::TargetMode(id, TargetMode::ALL[index]))),
    );

    let body: View = if mode == TargetMode::Store {
        let app_id = app.app_id.clone().unwrap_or_default();
        Stack::vertical(space::S)
            .child(
                "field",
                field("Store app id", text_box(app_id, "Publisher.App_8wekyb3d8bbwe!App", text_of(id, TextField::AppId, context))),
            )
            .child("hint", text::caption("The app's AppUserModelID. Choosing from the installed apps comes with the app picker."))
            .into()
    } else {
        let raw = app.exe.clone().unwrap_or_default();
        let folder = mode == TargetMode::Folder;
        let label = if folder { "Folder" } else { "Program or file" };
        let placeholder = if folder { r"%mydocs%\Projects" } else { r"%programfiles%\App\app.exe" };

        let browse: View = button("Browse…", false, false, context.callback(|()| Msg::Ignored));
        let tools = AutoGrid::new(180.0)
            .max_columns(2)
            .item("insert", insert_variable(settings, id, TextField::Target, &raw, false, context))
            .item("browse", browse.tooltip("Comes with the file dialog"))
            .build(viewport);

        let expanded = settings.expand_text(&raw, &mut *ui.builtins.borrow_mut());
        Stack::vertical(space::S)
            .child("field", field(label, text_box(raw, placeholder, text_of(id, TextField::Target, context))))
            .child("tools", tools)
            .child("readout", readout(&expanded, folder))
            .into()
    };

    Stack::vertical(space::M).child("mode", modes).child("body", body).into()
}

/// What the target comes to, and whether it is there.
fn readout(expanded: &str, folder: bool) -> View {
    let found = match target_check::check(expanded, folder) {
        Found::Yes => "found".to_owned(),
        Found::No => "not found".to_owned(),
        Found::Unchecked(why) => why.to_owned(),
    };
    if expanded.trim().is_empty() {
        return text::caption(found).into();
    }
    text::caption(format!("{expanded} — {found}")).into()
}

fn launch(
    settings: &Settings,
    id: ItemId,
    app: &AppFields,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let flag = move |label: &str, field: FlagField, on: bool| {
        labeled_switch(label, on, context.callback(move |on| Msg::Menu(MenuMsg::Flag(id, field, on))))
    };

    let param = app.param.clone().unwrap_or_default();
    let arguments: View = Stack::vertical(space::S)
        .child(
            "field",
            field("Arguments", text_box(param.clone(), "Command-line arguments", text_of(id, TextField::Param, context))),
        )
        .child("insert", insert_variable(settings, id, TextField::Param, &param, true, context))
        .child("note", text::caption("%clipboard% and %clipboards% are read when the item is launched."))
        .into();
    let start_in = field(
        "Start in",
        text_box(
            app.path.clone().unwrap_or_default(),
            "Working folder (empty: the program's own)",
            text_of(id, TextField::StartIn, context),
        ),
    );

    let inherited = if settings.globals.shell_execute.unwrap_or(false) { "on" } else { "off" };
    let shell = field(
        "Launch through the shell",
        combo(
            [format!("Inherit ({inherited})"), "On".to_owned(), "Off".to_owned()],
            Some(match app.shell {
                None => 0,
                Some(true) => 1,
                Some(false) => 2,
            }),
            context.callback(move |choice: Option<usize>| match choice {
                Some(choice) => Msg::Menu(MenuMsg::ShellExecute(id, choice)),
                None => Msg::Ignored,
            }),
        ),
    );

    Stack::vertical(space::M)
        .child(
            "fields",
            AutoGrid::new(240.0).max_columns(2).item("arguments", arguments).item("start", start_in).build(viewport),
        )
        .child("shell", AutoGrid::new(240.0).max_columns(2).item("shell", shell).build(viewport))
        .child("elevate", flag("Run as administrator", FlagField::Elevate, app.elevate))
        .child("on-start", flag("Also run when y-pop starts", FlagField::OnStart, app.on_start))
        .child("on-exit", flag("Also run when y-pop exits", FlagField::OnExit, app.on_exit))
        .into()
}

fn folder(id: ItemId, app: &AppFields, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View {
    let filter = field(
        "Filter",
        text_box(
            app.filter.clone().unwrap_or_default(),
            "*.sln;*.csproj (empty: everything)",
            text_of(id, TextField::Filter, context),
        ),
    );

    let depth = app.depth;
    let depth_box = field(
        "Depth",
        Stack::vertical(space::XS)
            .child(
                "box",
                number_box(
                    depth.map(f64::from),
                    0.0,
                    200.0,
                    context.callback(move |value: Option<f64>| Msg::Menu(MenuMsg::Depth(id, value))),
                ),
            )
            .child("words", text::caption(depth_words(depth.unwrap_or(0)))),
    );

    let open = field(
        "Entry to open the folder itself",
        combo(
            ["Only for folders at the deepest level", "In every submenu", "Never"],
            Some(match app.open_folder {
                None | Some(OpenFolder::MaxDepth) => 0,
                Some(OpenFolder::All) => 1,
                Some(OpenFolder::Never) => 2,
            }),
            context.callback(move |choice: Option<usize>| match choice {
                Some(choice) => Msg::Menu(MenuMsg::OpenFolder(id, choice)),
                None => Msg::Ignored,
            }),
        ),
    );

    AutoGrid::new(220.0).max_columns(3).item("filter", filter).item("depth", depth_box).item("open", open).build(viewport)
}
