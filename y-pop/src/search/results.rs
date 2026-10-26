//! Laying a ranked list of hits out for display (spec §2.8).
//!
//! `results="sections"`: the best match alone as **Top hit**, then the rest
//! in groups of at most [`PER_GROUP`], in a fixed order of groups. Only
//! groups with something in them appear. `results="flat"`: one list.

use crate::model::ResultsLayout;

use super::index::{Index, Kind, Source};
use super::query::Hit;

/// At most this many rows in each group (the Top hit is one more).
pub const PER_GROUP: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Heading {
    /// The last launches, on the empty box.
    Recent,
    /// The sections, on the empty box.
    Browse,
    /// The hotkeys, under `#`.
    Hotkeys,
    /// The prefixes, under `?`.
    Prefixes,
    TopHit,
    /// Apps of the tray menus and of search-only groups.
    Apps,
    StartMenu,
    MenusAndGroups,
    Folders,
    Commands,
    /// `results="flat"`: no grouping, no heading shown.
    All,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Group {
    pub heading: Heading,
    pub hits: Vec<Hit>,
}

/// `hits` (best first, as [`super::query::Searcher::search`] returns them)
/// arranged for `layout`. A hit shown as the Top hit is not repeated in its
/// group.
pub fn arrange(index: &Index, hits: &[Hit], layout: ResultsLayout) -> Vec<Group> {
    let Some((&top, rest)) = hits.split_first() else {
        return Vec::new();
    };
    if layout == ResultsLayout::Flat {
        return vec![Group {
            heading: Heading::All,
            hits: hits.to_vec(),
        }];
    }

    let mut groups = vec![Group {
        heading: Heading::TopHit,
        hits: vec![top],
    }];
    for heading in [
        Heading::Apps,
        Heading::StartMenu,
        Heading::MenusAndGroups,
        Heading::Folders,
        Heading::Commands,
    ] {
        let members: Vec<Hit> = rest
            .iter()
            .copied()
            .filter(|hit| heading_of(index, hit) == heading)
            .take(PER_GROUP)
            .collect();
        if !members.is_empty() {
            groups.push(Group {
                heading,
                hits: members,
            });
        }
    }
    groups
}

fn heading_of(index: &Index, hit: &Hit) -> Heading {
    let entry = &index.entries()[hit.entry as usize];
    match (entry.kind, entry.source) {
        (Kind::Command, _) => Heading::Commands,
        (Kind::Folder, _) => Heading::Folders,
        (Kind::Group, _) => Heading::MenusAndGroups,
        (Kind::App, Source::StartMenu) => Heading::StartMenu,
        (Kind::App, _) => Heading::Apps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeId;
    use crate::search::index::Entry;
    use crate::search::query::{Query, Scope, Searcher};

    fn entry(n: u32, source: Source, kind: Kind, name: &str) -> Entry {
        Entry::new(source, NodeId::from_u32(n), kind, name)
    }

    fn index() -> Index {
        let mut index = Index::default();
        for n in 0..8 {
            index.push(entry(n, Source::Menu, Kind::App, &format!("Tool {n}")));
        }
        index.push(entry(10, Source::StartMenu, Kind::App, "Tool from Start"));
        index.push(entry(11, Source::Menu, Kind::Group, "Toolbox"));
        index.push(entry(12, Source::Menu, Kind::Folder, "Tools folder"));
        index.push(entry(13, Source::Menu, Kind::Command, "Tool settings"));
        index
    }

    fn hits(index: &Index, text: &str) -> Vec<Hit> {
        Searcher::new()
            .search(index, &Query::new(text), Scope::All)
            .to_vec()
    }

    #[test]
    fn sections_are_a_top_hit_then_capped_groups_in_a_fixed_order() {
        let index = index();
        let hits = hits(&index, "tool");
        let groups = arrange(&index, &hits, ResultsLayout::Sections);

        let headings: Vec<Heading> = groups.iter().map(|group| group.heading).collect();
        assert_eq!(
            headings,
            [
                Heading::TopHit,
                Heading::Apps,
                Heading::StartMenu,
                Heading::MenusAndGroups,
                Heading::Folders,
                Heading::Commands,
            ]
        );
        assert_eq!(groups[0].hits, [hits[0]]);
        assert_eq!(groups[1].hits.len(), PER_GROUP, "capped");
        assert!(
            !groups[1].hits.contains(&hits[0]),
            "the top hit is not repeated"
        );
    }

    #[test]
    fn empty_groups_do_not_appear() {
        let index = index();
        let hits = hits(&index, "from start");
        let groups = arrange(&index, &hits, ResultsLayout::Sections);
        assert_eq!(groups.len(), 1, "only a Top hit");
        assert_eq!(groups[0].heading, Heading::TopHit);
    }

    #[test]
    fn flat_is_one_ranked_list_without_a_cap() {
        let index = index();
        let hits = hits(&index, "tool");
        let groups = arrange(&index, &hits, ResultsLayout::Flat);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].heading, Heading::All);
        assert_eq!(groups[0].hits, hits);
        assert!(hits.len() > PER_GROUP);
    }

    #[test]
    fn no_hits_no_groups() {
        assert!(arrange(&index(), &[], ResultsLayout::Sections).is_empty());
        assert!(arrange(&index(), &[], ResultsLayout::Flat).is_empty());
    }
}
