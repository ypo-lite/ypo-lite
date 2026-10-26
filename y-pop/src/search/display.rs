//! The text of a result row, decided without drawing it: how a title splits
//! into matched and unmatched stretches, what a heading says, and what the
//! second line of a row says. Pure, so the window only has to paint.

use std::ops::Range;

use crate::model::{ConfigTree, NodeKind};

use super::index::{Entry, Field, Kind, Source};
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
        Heading::TopHit => "TOP HIT",
        Heading::Apps => "APPS",
        Heading::StartMenu => "START MENU",
        Heading::MenusAndGroups => "MENUS AND GROUPS",
        Heading::Folders => "FOLDERS",
        Heading::Commands => "COMMANDS",
        Heading::All => "",
    }
}

/// The second line of a result row. Found by its name, it says where the
/// thing is — the menus it sits in and the program behind it; found by
/// anything else, it says what matched (spec §2.6).
pub fn subtitle(tree: &ConfigTree, entry: &Entry, matched: Field) -> String {
    let node = tree.node(entry.id);
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
    location(tree, entry)
}

/// Where an entry is: `Only in search · Work › Servers · C:\Windows\mstsc.exe`.
fn location(tree: &ConfigTree, entry: &Entry) -> String {
    let node = tree.node(entry.id);
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
        parts.push(path.join(" › "));
    }
    match (&node.kind, entry.kind) {
        (NodeKind::App(app), _) => {
            if let Some(target) = app.exe.as_deref().or(app.app_id.as_deref()) {
                parts.push(target.to_owned());
            }
        }
        (_, Kind::Group) => {
            let count = tree.children(entry.id).count();
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
