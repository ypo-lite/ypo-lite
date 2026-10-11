//! The General screen: how y-pop draws its menus and launches what you
//! pick, where its file is, and the notes the file starts and ends with.

use y_pop::settings::{PopupStyle, Renderer, Settings};

use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;

pub fn general_view(
    settings: &Settings,
    path: &std::path::Path,
    notes_ready: bool,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let globals = &settings.globals;
    let renderer = globals.popup.unwrap_or(Renderer::Native);
    let style = globals.popup_style.unwrap_or(PopupStyle::List);

    // The setting's own card gets the page's width.
    let menu_style = setting(
        "Menu style",
        "Native is the Windows menu, drawn by the system. Overlay is y-pop's own window, drawn from the skin.",
        segmented(
            &[Choice::new("Native"), Choice::new("Overlay")],
            Some(if renderer == Renderer::Native { 0 } else { 1 }),
            context.callback(Msg::RendererChosen),
        ),
        viewport,
    );

    let layout: Option<View> = (renderer == Renderer::Overlay).then(|| {
        setting(
            "Overlay layout",
            "How the overlay arranges its entries. Only the list is built so far.",
            segmented(
                &[
                    Choice::new("List"),
                    Choice::disabled("Runner", "Coming later"),
                    Choice::disabled("Radial", "Coming later"),
                ],
                Some(match style {
                    PopupStyle::List => 0,
                    PopupStyle::Runner => 1,
                    PopupStyle::Radial => 2,
                }),
                context.callback(Msg::PopupStyleChosen),
            ),
            viewport,
        )
    });

    let shell_execute = globals.shell_execute.unwrap_or(false);
    let show_shortcuts = globals.show_shortcuts.unwrap_or(true);
    let launching = Stack::vertical(space::M)
        .child(
            "shell",
            setting(
                "Launch through the shell",
                "Start programs the way Explorer does: documents open in their app, and \"Run as administrator\" \
                 can ask for elevation. Each entry can override it.",
                switch(shell_execute, context.callback(Msg::ShellExecuteToggled)),
                viewport,
            ),
        )
        .child(
            "shortcuts",
            setting(
                "Show shortcuts",
                "Print an entry's hotkey at the right of its row. Groups and entries can override it.",
                switch(show_shortcuts, context.callback(Msg::ShowShortcutsToggled)),
                viewport,
            ),
        );

    let file = setting(
        "Configuration file",
        &path.display().to_string(),
        Stack::horizontal(space::S)
            .child("folder", button("Open folder", false, true, context.callback(|()| Msg::OpenConfigFolder)))
            .child("edit", button("Edit XML…", false, true, context.callback(|()| Msg::EditConfigXml))),
        viewport,
    );

    page(
        "General",
        "How y-pop draws its menus and launches what you pick.",
        Stack::vertical(space::XL)
            .child(
                "menus",
                section(
                    "Popup menus",
                    Stack::vertical(space::M).child("style", menu_style).maybe("layout", layout),
                ),
            )
            .child("launching", section("Launching", launching))
            .child("file", section("File", file))
            .child("notes", notes(settings, notes_ready, context)),
    )
}

/// Every comment the file starts and ends with, as one text. Edited, they
/// become a single note at the top of the file (see
/// `Settings::set_file_notes_text`); until then each stays where it was.
/// `ready` is false on the box's first render: see `ConfigApp::notes_ready`.
fn notes(settings: &Settings, ready: bool, context: &ViewContext<ConfigApp>) -> View {
    let text = if ready { settings.file_notes_text() } else { String::new() };

    let mut body = Stack::vertical(space::S)
        .child("explanation", text::caption(NOTES_HELP))
        .child("text", text_area(text.clone(), "Notes about this file", context.callback(Msg::NotesEdited)));
    // C4: `--` can't be in an XML comment; the writer spells it `- -`.
    if text.contains("--") {
        body = body.child("warning", text::caption("-- isn't allowed in an XML comment; it will be saved as - -."));
    }
    if settings.file_notes_count() > 1 {
        body = body.child(
            "merge",
            text::caption("These come from several places in the file. Once you edit them they are saved together at the top."),
        );
    }
    section("Notes in the file", body)
}

const NOTES_HELP: &str = "Comments at the top and bottom of y-pop.xml. They are kept when you save.";
