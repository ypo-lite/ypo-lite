//! The Variables screen: the names usable as `%name%` in any path of the
//! file, in the order that gives them meaning — a value sees the builtins and
//! only the variables above it.
//!
//! Renaming is live and rewrites every `%old%` that resolves to the variable
//! (`Settings::rename_variable`); a name that can't be used says why under
//! its row and changes nothing. Deleting a variable that is still used asks
//! first and says where.

use y_pop::settings::{NameError, Settings, UsedIn, VarEntry};
use y_pop::variables::Lookup;

use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use windows_reactor::*;

/// How many comments the variables list holds, shown or not.
pub fn count_comments(settings: &Settings) -> usize {
    settings.variables.iter().filter(|entry| matches!(entry, VarEntry::Comment(_))).count()
}

/// What the screen remembers between renders.
#[derive(Clone, Default, Debug)]
pub struct VariablesUi {
    /// A rename that was refused, on the entry it was typed into.
    pub name_error: Option<(usize, NameError)>,
    /// A delete waiting for "Delete anyway" because the variable is used.
    pub confirming_delete: Option<usize>,
}

const COLUMNS: [Column; 5] = [
    Column::flex("Name", 120.0),
    Column::flex("Value", 170.0),
    Column::flex("Resolves to", 170.0),
    Column::fixed("Used by", 64.0),
    Column::actions(THREE_ICON_BUTTONS),
];

const BUILTIN_COLUMNS: [Column; 2] = [Column::fixed("Name", 150.0), Column::flex("On this PC", 240.0)];

/// How many places a tooltip lists before "…and N more".
const USAGE_LINES: usize = 8;

pub fn variables_view(
    settings: &Settings,
    builtins: &mut dyn Lookup,
    show_comments: bool,
    ui: &VariablesUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let count = settings.variables.len();
    let mut rows: Vec<Row> = Vec::new();
    // Comments seen since the last variable, while they're hidden.
    let mut hidden = 0;

    for (index, entry) in settings.variables.iter().enumerate() {
        match entry {
            VarEntry::Comment(comment) => {
                if show_comments {
                    let line: View = Stack::horizontal(space::S)
                        .child("icon", SymbolIcon::new().symbol(Symbol::Comment).opacity(emphasis::SUBTLE))
                        .child("text", text::caption(comment.text.clone()))
                        .into();
                    rows.push(Row::spanning(format!("c{index}"), line));
                } else {
                    hidden += 1;
                }
            }
            VarEntry::Var(var) => {
                let users = settings.variable_users(index);
                let resolved = settings.resolved_value(index, builtins);

                let name = text_box(var.name.clone(), "name", context.callback(move |text| Msg::VarNameChanged(index, text)));
                let name_cell: View = if hidden > 0 {
                    Grid::new()
                        .columns([GridLength::STAR, GridLength::Auto])
                        .column_spacing(space::XS)
                        .keyed_children([
                            ("name", cell(name, 0, 0)),
                            ("hidden", cell(hidden_marker(hidden, context), 0, 1)),
                        ])
                } else {
                    name.into()
                };
                hidden = 0;

                let value = text_box(var.value.clone(), "value", context.callback(move |text| Msg::VarValueChanged(index, text)));
                let resolves: View = text::single_line(resolved.clone()).opacity(emphasis::MUTED).tooltip(resolved);

                let used: View = if users.is_empty() {
                    text::placeholder().into()
                } else {
                    text::body(users.len().to_string()).tooltip(usage_text(settings, &users))
                };

                let actions: View = Stack::horizontal(space::XS)
                    .child(
                        "move",
                        move_buttons(
                            index,
                            count,
                            context.callback(move |()| Msg::VarMoved { index, up: true }),
                            context.callback(move |()| Msg::VarMoved { index, up: false }),
                        ),
                    )
                    .child(
                        "delete",
                        icon_button(Symbol::Delete, "Delete variable", context.callback(move |()| Msg::VarDeleteAsked(index))),
                    )
                    .into();

                let detail = row_detail(settings, index, &users, ui, context);
                rows.push(Row::new(format!("v{index}"), vec![name_cell, value.into(), resolves, used, actions]).detail(detail));
            }
        }
    }
    if hidden > 0 {
        let line: View = Stack::horizontal(space::S)
            .child("text", text::caption(format!("{hidden} comment{} hidden at the end.", if hidden == 1 { "" } else { "s" })))
            .child("show", hidden_marker(hidden, context))
            .into();
        rows.push(Row::spanning("hidden-end", line));
    }

    let table = Table::new(&COLUMNS)
        .empty_message("No variables yet — add one below.")
        .rows(rows)
        .build(viewport);

    let toolbar: View = Stack::horizontal(space::L)
        .child("add", add_button("Add variable", context.callback(|()| Msg::VarAdded)))
        .into();

    page(
        "Variables",
        "Names you can use as %name% in any path or text below. A variable sees the built-in ones and the \
         variables above it, so the order matters.",
        Stack::vertical(space::XL)
            .child(
                "mine",
                section("Your variables", Stack::vertical(space::L).child("table", table).child("toolbar", toolbar)),
            )
            .child("builtin", section("Built in", builtin_table(builtins, viewport))),
    )
}

/// The little speech bubble that stands in for comments that are hidden.
fn hidden_marker(count: usize, context: &ViewContext<ConfigApp>) -> View {
    icon_button(
        Symbol::Comment,
        &format!("{count} comment{} hidden — click to show", if count == 1 { "" } else { "s" }),
        context.callback(|()| Msg::ShowCommentsToggled(true)),
    )
}

/// Under a row: why a rename was refused, or the question before deleting a
/// variable that is still used.
fn row_detail(
    settings: &Settings,
    index: usize,
    users: &[UsedIn],
    ui: &VariablesUi,
    context: &ViewContext<ConfigApp>,
) -> Option<View> {
    if let Some((at, error)) = ui.name_error
        && at == index
    {
        return Some(text::caption(error.to_string()).into());
    }
    if ui.confirming_delete == Some(index) {
        let question = format!(
            "Still used by {} place{}: {}",
            users.len(),
            if users.len() == 1 { "" } else { "s" },
            usage_text(settings, users).replace('\n', ", ")
        );
        return Some(
            Stack::vertical(space::S)
                .child("where", text::caption(question))
                .child(
                    "buttons",
                    Stack::horizontal(space::S)
                        .child("yes", button("Delete anyway", true, true, context.callback(move |()| Msg::VarDeleteConfirmed(index))))
                        .child("no", button("Keep it", false, true, context.callback(|()| Msg::VarDeleteCancelled))),
                )
                .into(),
        );
    }
    None
}

/// Where a variable is used, one place per line, for a tooltip or a question.
fn usage_text(settings: &Settings, users: &[UsedIn]) -> String {
    let mut lines: Vec<String> = users
        .iter()
        .take(USAGE_LINES)
        .map(|used| match used {
            UsedIn::Item { id, field } => {
                let name = settings
                    .items()
                    .find(|item| item.id == *id)
                    .map(crate::roots::label)
                    .unwrap_or_default();
                format!("{name} ({field})")
            }
            UsedIn::Variable { index } => match settings.variables.get(*index) {
                Some(VarEntry::Var(var)) => format!("variable {}", var.name),
                _ => "a variable".to_owned(),
            },
        })
        .collect();
    if users.len() > USAGE_LINES {
        lines.push(format!("…and {} more", users.len() - USAGE_LINES));
    }
    lines.join("\n")
}

/// The tokens y-pop provides, resolved on this PC.
fn builtin_table(builtins: &mut dyn Lookup, viewport: Viewport) -> View {
    let rows: Vec<Row> = y_pop::builtins::NAMES
        .iter()
        .map(|name| {
            let value = builtins.get(name).unwrap_or_default();
            Row::new(
                *name,
                vec![
                    text::body(format!("%{name}%")).into(),
                    text::single_line(value.clone()).opacity(emphasis::MUTED).tooltip(value),
                ],
            )
        })
        .collect();

    Stack::vertical(space::M)
        .child("table", Table::new(&BUILTIN_COLUMNS).rows(rows).build(viewport))
        .child(
            "clipboard",
            text::caption(
                "%clipboard% and %clipboards% are different: they are read at the moment an entry launches, so they \
                 never show a value here.",
            ),
        )
        .into()
}
