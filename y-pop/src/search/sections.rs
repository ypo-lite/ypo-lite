//! The sections the empty box lists under **Browse** (spec §2.4, §2.5), and
//! the places a box can be inside.
//!
//! This is the flat form of task 6.4: opening a section puts it in the box as
//! a chip and lists what is in it, the way entering a group does. The richer
//! views — Miller columns for the tray menus (phase 7), the A–Z list of the Start
//! menu (phase 9), a folder's files (phase 10) — replace the list section by
//! section later. A section that has nothing to open yet is not listed.

use crate::model::{ConfigTree, NodeId, Section};

use super::index::{Index, Kind, Source};
use super::matcher::plain;

/// Somewhere the box can be scoped to: shown as a chip, and the search looks
/// only there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// Every tray menu: the `<menus>` section.
    Menus,
    /// A group (a search-only section, or one entered with `Tab`).
    Group(NodeId),
    /// Everything Windows remembers: the list behind the few rows of Recent.
    Recent,
}

impl Place {
    /// What the chip and the section row say.
    pub fn name(self, tree: &ConfigTree) -> String {
        match self {
            Self::Menus => "Tray menus".to_owned(),
            Self::Group(group) => plain(&tree.node(group).name),
            Self::Recent => "All recent".to_owned(),
        }
    }
}

/// One row of the **Browse** list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SectionRow {
    pub place: Place,
    /// The apps in it (in [`Place::Recent`], the items), for the second line.
    pub apps: usize,
}

/// The rows to list, in configuration order. Only what can be opened:
/// the tray menus and search-only groups, if they hold an app.
pub fn rows(tree: &ConfigTree, index: &Index) -> Vec<SectionRow> {
    let Some(search) = &tree.search else {
        return Vec::new();
    };
    let apps = |keep: &dyn Fn(&super::index::Entry) -> bool| {
        index
            .entries()
            .iter()
            .filter(|entry| entry.kind == Kind::App && keep(entry))
            .count()
    };
    search
        .sections
        .iter()
        .filter_map(|section| match section {
            Section::Menus { .. } => Some(SectionRow {
                place: Place::Menus,
                apps: apps(&|entry| entry.source == Source::Menu),
            }),
            Section::Group(group) => Some(SectionRow {
                place: Place::Group(*group),
                apps: apps(&|entry| entry.ancestors.contains(group)),
            }),
            // Later: a Start menu, a folder's files, a plugin's results.
            Section::Recent { .. }
            | Section::StartMenu { .. }
            | Section::Folder(_)
            | Section::Plugin(_) => None,
        })
        .filter(|row| row.apps > 0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::variables::Variable;

    fn listed(xml: &str) -> (ConfigTree, Vec<(String, usize)>) {
        let tree = config::load_str(xml, &mut Vec::<Variable>::new()).unwrap();
        let index = Index::from_tree(&tree);
        let listed = rows(&tree, &index)
            .iter()
            .map(|row| (row.place.name(&tree), row.apps))
            .collect();
        (tree, listed)
    }

    #[test]
    fn the_tray_menus_and_the_search_only_groups_are_listed_in_order() {
        let (_, listed) = listed(
            r#"<y-pop>
                <search>
                    <group name="Admin"><app name="A" exe="a.exe"/><app name="B" exe="b.exe"/></group>
                    <start-menu/>
                </search>
                <group name="Work"><app name="C" exe="c.exe"/><group name="Sub"><app name="D" exe="d.exe"/></group></group>
            </y-pop>"#,
        );
        // <menus/> is implied, placed first when nothing else says otherwise;
        // the Start menu has no view yet.
        assert_eq!(
            listed,
            [("Tray menus".to_owned(), 2), ("Admin".to_owned(), 2)]
        );
    }

    #[test]
    fn a_section_with_no_app_in_it_is_not_listed() {
        let (_, listed) = listed(
            r#"<y-pop>
                <search><group name="Empty"/></search>
                <group name="Work"><quit/></group>
            </y-pop>"#,
        );
        assert!(listed.is_empty(), "{listed:?}");
    }

    #[test]
    fn no_search_no_sections() {
        let (_, listed) =
            listed(r#"<y-pop><group name="Work"><app name="C" exe="c.exe"/></group></y-pop>"#);
        assert!(listed.is_empty());
    }
}
