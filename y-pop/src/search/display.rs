//! The text of a result row, decided without drawing it: how a title splits
//! into matched and unmatched stretches, what a heading says, and what the
//! second line of a row says. Pure, so the window only has to paint.

use std::ops::Range;

use crate::model::{ConfigTree, NodeId, NodeKind};

use super::commands::{Command, Line, PREFIXES};
use super::files::{CAP, Progress};
use super::index::{Entry, Field, Kind, Source};
use super::keycheck::Availability;
use super::matcher::plain;
use super::results::Heading;

/// A stretch of a title, in UTF-16 units (what
/// `y_core::overlay::Font::caret_offsets` is indexed by), drawn in one style.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Run {
    pub units: Range<usize>,
    /// Letters the query matched, to be emphasized.
    pub matched: bool,
}

/// `text` cut into alternating unmatched and matched stretches. `positions`
/// are the character indices the matcher found, ascending (as
/// [`super::query::Searcher::highlight`] returns them).
pub fn runs(text: &str, positions: &[usize]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut unit = 0;
    let mut next = positions.iter().copied().peekable();

    for (index, c) in text.chars().enumerate() {
        let matched = next.next_if_eq(&index).is_some();
        let end = unit + c.len_utf16();
        match runs.last_mut() {
            Some(last) if last.matched == matched => last.units.end = end,
            _ => runs.push(Run {
                units: unit..end,
                matched,
            }),
        }
        unit = end;
    }
    runs
}

/// The small caption above a group of results.
pub fn heading(heading: Heading) -> &'static str {
    match heading {
        Heading::Recent => "RECENT",
        Heading::Browse => "BROWSE",
        Heading::Hotkeys => "HOTKEYS",
        Heading::Prefixes => "PREFIXES",
        Heading::TopHit => "TOP HIT",
        Heading::Apps => "APPS",
        Heading::StartMenu => "START MENU",
        Heading::MenusAndGroups => "MENUS AND GROUPS",
        Heading::Folders => "FOLDERS",
        Heading::Files => "FILES",
        Heading::Commands => "COMMANDS",
        Heading::All => "",
    }
}

/// What a [`Line`] needs to be worded: facts only the window can read.
#[derive(Clone, Copy, Default)]
pub struct LineContext<'a> {
    /// Whether y-pop starts with Windows now.
    pub startup: bool,
    /// The search hotkey, written out.
    pub search_hotkey: Option<&'a str>,
    /// The combination `#` is checking and what the system said of it; `None`
    /// for the answer while it is still being asked.
    pub check: Option<(&'a str, Option<Availability>)>,
    /// How the listing of `/` is going.
    pub scan: Progress,
}

/// The words of one row of a prefixed list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LineText {
    pub title: String,
    pub subtitle: String,
    /// At the right end, in the dim hotkey style.
    pub trailing: Option<String>,
}

/// The title, second line and right-hand text of `line`.
pub fn line_text(line: Line, context: &LineContext) -> LineText {
    let plain = |title: &str, subtitle: &str| LineText {
        title: title.to_owned(),
        subtitle: subtitle.to_owned(),
        trailing: None,
    };
    match line {
        Line::Command(command) => LineText {
            trailing: (command == Command::StartWithWindows)
                .then(|| if context.startup { "On" } else { "Off" }.to_owned()),
            ..plain(command.label(), command.description())
        },
        Line::Prefix(prefix) => {
            let info = PREFIXES.iter().find(|info| info.prefix == prefix);
            LineText {
                title: format!("{prefix}  {}", info.map_or("", |info| info.name)),
                subtitle: info.map_or("", |info| info.description).to_owned(),
                trailing: None,
            }
        }
        Line::SearchHotkey => LineText {
            trailing: context.search_hotkey.map(str::to_owned),
            ..plain("Open Search", "The search hotkey")
        },
        Line::Scan => {
            let Progress {
                listing,
                folders,
                skipped,
                found,
                capped,
                ..
            } = context.scan;
            let left_out = match skipped {
                0 => String::new(),
                1 => "1 network folder skipped: index=\"true\" includes it".to_owned(),
                n => format!("{n} network folders skipped: index=\"true\" includes them"),
            };
            if listing {
                plain(&format!("Listing folders… {found} found"), &left_out)
            } else if capped {
                plain(
                    &format!("Only the first {CAP} files are searched"),
                    &left_out,
                )
            } else if skipped > 0 {
                let title = left_out.split(':').next().unwrap_or_default();
                plain(title, "index=\"true\" on a folder lists it anyway")
            } else if folders == 0 {
                plain(
                    "No browsed folders",
                    "/ searches the folders that have browse=\"true\"",
                )
            } else {
                plain("", "")
            }
        }
        Line::Check => {
            let Some((typed, availability)) = context.check else {
                return plain("Checking…", "");
            };
            let key = y_core::hotkey::Hotkey::parse(typed)
                .map_or_else(|| typed.to_owned(), |hotkey| hotkey.to_string());
            match availability {
                None => plain(&format!("Checking {key}…"), ""),
                Some(Availability::Free) => plain(
                    &format!("{key} is free"),
                    "No other program holds it. Enter opens the settings to assign it.",
                ),
                Some(Availability::Reserved(owner)) => plain(
                    &format!("{key} is taken by {owner}"),
                    "Windows keeps it. override=\"true\" could still claim it with the low-level hook.",
                ),
                Some(Availability::Taken) => plain(
                    &format!("{key} is taken"),
                    "Another program holds it (Windows does not say which). override=\"true\" could still claim it.",
                ),
            }
        }
    }
}

/// How long ago something was, in a few words: `just now`, `2 min ago`,
/// `3 h ago`, `yesterday`, `5 days ago`.
pub fn ago(seconds: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    match seconds {
        s if s < MINUTE => "just now".to_owned(),
        s if s < HOUR => format!("{} min ago", s / MINUTE),
        s if s < DAY => format!("{} h ago", s / HOUR),
        s if s < 2 * DAY => "yesterday".to_owned(),
        s => format!("{} days ago", s / DAY),
    }
}

/// The path to put on the clipboard for `entry` (Ctrl+C): the program or
/// folder it runs, `%variables%` already expanded by the reader; for a Store
/// app, the `shell:AppsFolder\…` address that opens it. `None` for a group or
/// a command, which have none.
pub fn target(tree: &ConfigTree, entry: &Entry) -> Option<String> {
    let NodeKind::App(app) = &tree.node(entry.node()?).kind else {
        return None;
    };
    match (&app.exe, &app.app_id) {
        (Some(exe), _) => Some(exe.to_string()),
        (None, Some(id)) => Some(format!(r"shell:AppsFolder\{id}")),
        (None, None) => None,
    }
}

/// The file or folder `entry` runs, for showing it in Explorer (Shift+Enter).
/// `None` for a Store app, which is not a file, and for anything that is not
/// an app.
pub fn file_of(tree: &ConfigTree, entry: &Entry) -> Option<String> {
    match &tree.node(entry.node()?).kind {
        NodeKind::App(app) => app.exe.as_ref().map(|exe| exe.to_string()),
        _ => None,
    }
}

/// The second line of a result row. Found by its name, it says where the
/// thing is — the menus it sits in and the program behind it; found by
/// anything else, it says what matched (spec §2.6).
pub fn subtitle(tree: &ConfigTree, entry: &Entry, matched: Field) -> String {
    let Some(id) = entry.node() else {
        // The item of the history says it itself: see `history::Item::detail`.
        return String::new();
    };
    let node = tree.node(id);
    match matched {
        Field::Keywords => {
            if let Some(keywords) = &node.keywords {
                return format!("Keywords: {keywords}");
            }
        }
        Field::Exe => {
            if let NodeKind::App(app) = &node.kind
                && let Some(exe) = &app.exe
            {
                let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
                return format!("Program: {file}");
            }
        }
        Field::Tooltip => {
            if let Some(tooltip) = &node.tooltip {
                return tooltip.to_string();
            }
        }
        Field::Hotkey => {
            if let Some(binding) = node.hotkeys.first() {
                return format!("Hotkey: {}", binding.hotkey);
            }
        }
        // The path is part of the default line.
        Field::Name | Field::Path => {}
    }
    location(tree, id, entry)
}

/// The lines of text of the details pane of a column (spec §2.4), under the
/// name: where it is, what it runs, its hotkeys, its tooltip, and what `→` does
/// on a folder. Nothing for an entry that is not a node: see `Files::detail`.
pub fn details(tree: &ConfigTree, entry: &Entry) -> Vec<String> {
    let Some(id) = entry.node() else {
        return Vec::new();
    };
    let node = tree.node(id);
    let mut lines = Vec::new();
    if !entry.ancestors.is_empty() {
        let path: Vec<&str> = entry
            .ancestors
            .iter()
            .map(|&id| tree.node(id).name.as_ref())
            .collect();
        lines.push(plain(&path.join(" › ")));
    }
    if let Some(target) = target(tree, entry) {
        lines.push(target);
    }
    if entry.kind == Kind::Command {
        lines.push("Command".to_owned());
    }
    for binding in &node.hotkeys {
        lines.push(binding.hotkey.to_string());
    }
    if let Some(tooltip) = &node.tooltip {
        lines.push(tooltip.to_string());
    }
    if entry.facts.browsable {
        lines.push(FOLDER_HINT.to_owned());
    }
    lines
}

/// What the details of a folder say, as nothing has been read from it yet.
pub const FOLDER_HINT: &str = "→ lists what is inside";

/// Where an entry is: `Only in search · Work › Servers · C:\Windows\mstsc.exe`.
fn location(tree: &ConfigTree, id: NodeId, entry: &Entry) -> String {
    let node = tree.node(id);
    let mut parts: Vec<String> = Vec::new();

    if entry.source == Source::SearchOnly {
        parts.push("Only in search".to_owned());
    }
    if !entry.ancestors.is_empty() {
        let path: Vec<&str> = entry
            .ancestors
            .iter()
            .map(|&id| tree.node(id).name.as_ref())
            .collect();
        parts.push(plain(&path.join(" › ")));
    }
    match (&node.kind, entry.kind) {
        (NodeKind::App(app), _) => {
            if let Some(target) = app.exe.as_deref().or(app.app_id.as_deref()) {
                parts.push(target.to_owned());
            }
        }
        (_, Kind::Group) => {
            let count = tree.children(id).count();
            parts.push(match count {
                1 => "1 item".to_owned(),
                count => format!("{count} items"),
            });
        }
        (_, Kind::Command) => parts.push("Command".to_owned()),
        _ => {}
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::search::index::Index;
    use crate::variables::Variable;

    fn tree() -> ConfigTree {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(
            r#"<y-pop>
                <search><group name="Admin"><app name="Regedit" exe="C:\Windows\regedit.exe"/></group></search>
                <group name="Work">
                    <app name="Outlook" exe="C:\Office\OUTLOOK.EXE" keywords="mail email" tooltip="Mail client">
                        <hotkey key="Win+O"/>
                    </app>
                    <group name="Servers"><app name="Remote Desktop" exe="mstsc.exe"/></group>
                    <app name="Store thing" app-id="Pub.App_x!App"/>
                    <quit/>
                </group>
            </y-pop>"#,
            &mut builtins,
        )
        .unwrap();
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        tree
    }

    fn entry<'a>(index: &'a Index, name: &str) -> &'a Entry {
        index
            .entries()
            .iter()
            .find(|entry| entry.name() == name)
            .unwrap()
    }

    #[test]
    fn the_details_say_where_it_is_what_it_runs_and_what_opens_it() {
        let tree = tree();
        let index = Index::from_tree(&tree);
        assert_eq!(
            details(&tree, entry(&index, "outlook")),
            ["Work", r"C:\Office\OUTLOOK.EXE", "Win+O", "Mail client"]
        );
        assert_eq!(
            details(&tree, entry(&index, "remote desktop")),
            ["Work › Servers", "mstsc.exe"]
        );
        assert_eq!(details(&tree, entry(&index, "quit")), ["Work", "Command"]);
    }

    #[test]
    fn the_details_of_a_browsed_folder_say_it_has_not_been_read() {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(
            r#"<y-pop><app name="Scripts" exe="C:\Scripts" browse="true"/></y-pop>"#,
            &mut builtins,
        )
        .unwrap();
        let index = Index::from_tree(&tree);
        assert_eq!(
            details(&tree, &index.entries()[0]),
            [r"C:\Scripts", FOLDER_HINT]
        );
    }

    #[test]
    fn runs_alternate_between_plain_and_matched() {
        assert_eq!(
            runs("Outlook", &[0, 1, 2]),
            [
                Run {
                    units: 0..3,
                    matched: true
                },
                Run {
                    units: 3..7,
                    matched: false
                },
            ]
        );
        assert_eq!(
            runs("Open Utility Tool", &[0, 5, 13]),
            [
                Run {
                    units: 0..1,
                    matched: true
                },
                Run {
                    units: 1..5,
                    matched: false
                },
                Run {
                    units: 5..6,
                    matched: true
                },
                Run {
                    units: 6..13,
                    matched: false
                },
                Run {
                    units: 13..14,
                    matched: true
                },
                Run {
                    units: 14..17,
                    matched: false
                },
            ]
        );
        assert_eq!(
            runs("abc", &[]),
            [Run {
                units: 0..3,
                matched: false
            }]
        );
        assert_eq!(runs("", &[]), []);
    }

    #[test]
    fn runs_count_utf16_units_not_characters() {
        // 😀 is one character, two units: the `x` after it starts at unit 3.
        let found = runs("a😀x", &[2]);
        assert_eq!(
            found,
            [
                Run {
                    units: 0..3,
                    matched: false
                },
                Run {
                    units: 3..4,
                    matched: true
                },
            ]
        );
    }

    #[test]
    fn every_heading_but_the_flat_one_has_a_caption() {
        for heading_ in [
            Heading::Recent,
            Heading::Browse,
            Heading::Hotkeys,
            Heading::Prefixes,
            Heading::TopHit,
            Heading::Apps,
            Heading::StartMenu,
            Heading::MenusAndGroups,
            Heading::Folders,
            Heading::Commands,
        ] {
            assert!(!heading(heading_).is_empty());
        }
        assert_eq!(heading(Heading::All), "");
    }

    #[test]
    fn a_line_says_what_it_is_and_what_state_it_is_in() {
        let context = LineContext::default();
        let reload = line_text(Line::Command(Command::Reload), &context);
        assert_eq!(reload.title, "Reload configuration");
        assert_eq!(reload.trailing, None);

        let start = |on| LineContext {
            startup: on,
            ..LineContext::default()
        };
        let startup = Line::Command(Command::StartWithWindows);
        assert_eq!(
            line_text(startup, &start(true)).trailing.as_deref(),
            Some("On")
        );
        assert_eq!(
            line_text(startup, &start(false)).trailing.as_deref(),
            Some("Off")
        );

        let own = LineContext {
            search_hotkey: Some("Alt+Space"),
            ..LineContext::default()
        };
        assert_eq!(
            line_text(Line::SearchHotkey, &own).trailing.as_deref(),
            Some("Alt+Space")
        );
        assert_eq!(line_text(Line::Prefix('#'), &context).title, "#  hotkeys");
    }

    #[test]
    fn a_key_check_answers_free_reserved_or_taken_in_the_keys_own_spelling() {
        let check = |availability| LineContext {
            check: Some(("ctrl+alt+x", availability)),
            ..LineContext::default()
        };
        assert_eq!(
            line_text(Line::Check, &check(Some(Availability::Free))).title,
            "Ctrl+Alt+X is free"
        );
        assert_eq!(
            line_text(Line::Check, &check(Some(Availability::Taken))).title,
            "Ctrl+Alt+X is taken"
        );
        let reserved = LineContext {
            check: Some((
                "win+e",
                Some(Availability::Reserved("File Explorer (Windows)")),
            )),
            ..LineContext::default()
        };
        assert_eq!(
            line_text(Line::Check, &reserved).title,
            "Win+E is taken by File Explorer (Windows)"
        );
        assert!(
            line_text(Line::Check, &check(None))
                .title
                .starts_with("Checking")
        );
    }
    #[test]
    fn the_listing_says_how_it_is_going_and_what_it_left_out() {
        let say = |scan: Progress| {
            line_text(
                Line::Scan,
                &LineContext {
                    scan,
                    ..LineContext::default()
                },
            )
        };
        let base = Progress {
            started: true,
            folders: 2,
            ..Progress::default()
        };
        let listing = say(Progress {
            listing: true,
            found: 12,
            skipped: 1,
            ..base
        });
        assert_eq!(listing.title, "Listing folders… 12 found");
        assert!(listing.subtitle.starts_with("1 network folder skipped"));

        assert_eq!(
            say(Progress {
                capped: true,
                ..base
            })
            .title,
            format!("Only the first {CAP} files are searched")
        );
        assert_eq!(
            say(Progress { skipped: 2, ..base }).title,
            "2 network folders skipped"
        );
        assert_eq!(
            say(Progress { folders: 0, ..base }).title,
            "No browsed folders"
        );
    }

    #[test]
    fn how_long_ago_is_said_in_a_few_words() {
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(2 * 60 + 30), "2 min ago");
        assert_eq!(ago(3 * 3600), "3 h ago");
        assert_eq!(ago(30 * 3600), "yesterday");
        assert_eq!(ago(5 * 86_400 + 100), "5 days ago");
    }

    #[test]
    fn by_name_the_second_line_says_where_it_is() {
        let tree = tree();
        let index = Index::from_tree(&tree);

        assert_eq!(
            subtitle(&tree, entry(&index, "outlook"), Field::Name),
            r"Work · C:\Office\OUTLOOK.EXE"
        );
        assert_eq!(
            subtitle(&tree, entry(&index, "remote desktop"), Field::Name),
            "Work › Servers · mstsc.exe"
        );
        assert_eq!(
            subtitle(&tree, entry(&index, "store thing"), Field::Name),
            "Work · Pub.App_x!App"
        );
        assert_eq!(
            subtitle(&tree, entry(&index, "work"), Field::Name),
            "4 items"
        );
        assert_eq!(
            subtitle(&tree, entry(&index, "servers"), Field::Name),
            "Work · 1 item"
        );
        assert_eq!(
            subtitle(&tree, entry(&index, "quit"), Field::Name),
            "Work · Command"
        );
        assert_eq!(
            subtitle(&tree, entry(&index, "regedit"), Field::Name),
            r"Only in search · Admin · C:\Windows\regedit.exe"
        );
    }

    #[test]
    fn the_path_to_copy_is_the_program_the_folder_or_the_store_address() {
        let tree = tree();
        let index = Index::from_tree(&tree);

        assert_eq!(
            target(&tree, entry(&index, "outlook")).as_deref(),
            Some(r"C:\Office\OUTLOOK.EXE")
        );
        assert_eq!(
            target(&tree, entry(&index, "store thing")).as_deref(),
            Some(r"shell:AppsFolder\Pub.App_x!App")
        );
        assert_eq!(target(&tree, entry(&index, "work")), None, "a group");
        assert_eq!(target(&tree, entry(&index, "quit")), None, "a command");
    }

    #[test]
    fn only_a_file_can_be_shown_in_explorer() {
        let tree = tree();
        let index = Index::from_tree(&tree);

        assert_eq!(
            file_of(&tree, entry(&index, "outlook")).as_deref(),
            Some(r"C:\Office\OUTLOOK.EXE")
        );
        assert_eq!(
            file_of(&tree, entry(&index, "remote desktop")).as_deref(),
            Some("mstsc.exe"),
            "a bare name: the caller looks it up"
        );
        assert_eq!(file_of(&tree, entry(&index, "store thing")), None);
        assert_eq!(file_of(&tree, entry(&index, "servers")), None);
    }

    #[test]
    fn found_by_something_else_it_says_what_matched() {
        let tree = tree();
        let index = Index::from_tree(&tree);
        let outlook = entry(&index, "outlook");

        assert_eq!(
            subtitle(&tree, outlook, Field::Keywords),
            "Keywords: mail email"
        );
        assert_eq!(subtitle(&tree, outlook, Field::Exe), "Program: OUTLOOK.EXE");
        assert_eq!(subtitle(&tree, outlook, Field::Tooltip), "Mail client");
        assert_eq!(subtitle(&tree, outlook, Field::Hotkey), "Hotkey: Win+O");
        assert_eq!(
            subtitle(&tree, outlook, Field::Path),
            subtitle(&tree, outlook, Field::Name),
            "the path is part of the default line"
        );
        // Nothing to say about a field the node does not have: the default.
        let remote = entry(&index, "remote desktop");
        assert_eq!(
            subtitle(&tree, remote, Field::Keywords),
            subtitle(&tree, remote, Field::Name)
        );
    }
}
