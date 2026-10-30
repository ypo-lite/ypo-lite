//! The Miller columns the tray menus section opens as (spec §2.4): the tray
//! menus, then what the selection before holds, then a preview of what is
//! selected in the last column.
//!
//! Pure: a column is a list of index entries. The last one is the **focused**
//! column, and it is the one [`super::state::State`] holds as its rows, so moving,
//! acting, shortcuts and the action menu are the box's own; only what is to the
//! left of it, and the preview to the right, live here.
//!
//! What a group holds comes from the index. **A browsed folder is listed when its
//! column opens, never in advance**: selecting one previews its details, `→` goes
//! in, and the column waits ([`Pane::waiting`]) for the thread of
//! [`super::files::Files::list_column`]. So walking down a menu never touches a
//! disk, and a slow share only holds up the column that asked for it.

use crate::model::NodeId;

use super::index::{Entry, Index, Kind, Source};

/// One column.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pane {
    /// Positions in [`Index::entries`], in the order shown.
    pub items: Vec<u32>,
    /// Which of `items` is selected. For the focused column, as of when it
    /// last stopped being shown: the box's own selection is the current one.
    pub selected: usize,
    /// The browsed folder (its entry) whose listing this column waits for.
    pub waiting: Option<u32>,
}

impl Pane {
    fn of(items: Vec<u32>) -> Self {
        Self {
            items,
            selected: 0,
            waiting: None,
        }
    }

    fn waiting_for(folder: u32) -> Self {
        Self {
            waiting: Some(folder),
            ..Self::of(Vec::new())
        }
    }
}

/// What the area to the right of the focused column shows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Preview {
    /// A group's items, a column that opens with `→`.
    Items(Vec<u32>),
    /// The details of this entry.
    Details(u32),
    /// Nothing is selected.
    Nothing,
}

/// A column as the window draws it: the columns, then the preview.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PaneView {
    pub items: Vec<u32>,
    pub selected: Option<usize>,
    pub waiting: bool,
    /// The preview of an entry, instead of a list.
    pub details: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Columns {
    /// Never empty; the last is the focused one.
    panes: Vec<Pane>,
    /// The box's rows are the focused column right now: nothing is typed, and
    /// no group was entered from the results of a search.
    pub(super) showing: bool,
}

impl Columns {
    /// The first column: the tray menus `<menus include>` keeps.
    pub fn new(index: &Index) -> Self {
        let roots = positions(index, |entry| {
            entry.source == Source::Menu && entry.ancestors.is_empty() && !entry.left_out
        });
        Self {
            panes: vec![Pane::of(roots)],
            showing: false,
        }
    }

    pub fn panes(&self) -> &[Pane] {
        &self.panes
    }

    pub(super) fn focused_mut(&mut self) -> &mut Pane {
        self.panes.last_mut().expect("never empty")
    }

    pub(super) fn focused(&self) -> &Pane {
        self.panes.last().expect("never empty")
    }

    /// How many columns are left of the focused one.
    pub fn depth(&self) -> usize {
        self.panes.len() - 1
    }

    /// Whether `→` goes anywhere from `entry`.
    pub fn opens(entry: &Entry) -> bool {
        entry.kind == Kind::Group || entry.facts.browsable
    }

    /// The column that opens from `position`, if there is one: a group that
    /// holds something, or a browsed folder, which then waits for its listing.
    pub(super) fn open(&mut self, index: &Index, position: u32) -> Option<&Pane> {
        let entry = &index.entries()[position as usize];
        let pane = if entry.facts.browsable {
            Pane::waiting_for(position)
        } else {
            let items = children(index, entry.node().filter(|_| Self::opens(entry))?);
            if items.is_empty() {
                return None;
            }
            Pane::of(items)
        };
        self.panes.push(pane);
        self.panes.last()
    }

    /// Back one column. `false` from the first.
    pub(super) fn back(&mut self) -> bool {
        self.depth() > 0 && self.panes.pop().is_some()
    }

    /// Closes the columns right of `pane`, which becomes the focused one.
    pub(super) fn focus(&mut self, pane: usize) {
        self.panes.truncate(pane + 1);
    }

    /// The listing of `folder` is here. `false` if no column waits for it any
    /// more.
    pub(super) fn listed(&mut self, folder: u32, items: Vec<u32>) -> bool {
        let pane = self.focused_mut();
        if pane.waiting != Some(folder) {
            return false;
        }
        *pane = Pane::of(items);
        true
    }

    /// What `selected`, the box's selection, shows to the right of the column.
    pub fn preview(index: &Index, selected: Option<u32>) -> Preview {
        let Some(position) = selected else {
            return Preview::Nothing;
        };
        let entry = &index.entries()[position as usize];
        match entry.node().filter(|_| entry.kind == Kind::Group) {
            Some(group) => Preview::Items(children(index, group)),
            None => Preview::Details(position),
        }
    }

    /// The entries the focused column was reached through, outermost first:
    /// the breadcrumb.
    pub fn crumbs(&self) -> Vec<u32> {
        let depth = self.depth();
        self.panes[..depth]
            .iter()
            .filter_map(|pane| pane.items.get(pane.selected).copied())
            .collect()
    }
}

/// What `group` directly holds, in file order.
fn children(index: &Index, group: NodeId) -> Vec<u32> {
    positions(index, |entry| entry.ancestors.last() == Some(&group))
}

fn positions(index: &Index, keep: impl Fn(&Entry) -> bool) -> Vec<u32> {
    index
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, entry)| keep(entry))
        .map(|(position, _)| position as u32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::variables::Variable;

    fn index(xml: &str) -> Index {
        let tree = config::load_str(xml, &mut Vec::<Variable>::new()).unwrap();
        Index::from_tree(&tree)
    }

    const MENUS: &str = r#"<y-pop>
        <search><group name="Admin"><app name="Regedit" exe="r.exe"/></group></search>
        <group name="Work">
            <app name="Mail" exe="m.exe"/>
            <group name="Servers"><app name="Putty" exe="p.exe"/></group>
            <group name="Hollow"/>
            <app name="Scripts" exe="C:\Scripts" browse="true"/>
        </group>
        <app name="Alone" exe="a.exe"/>
    </y-pop>"#;

    fn names(index: &Index, items: &[u32]) -> Vec<String> {
        items
            .iter()
            .map(|&item| index.entries()[item as usize].name().to_owned())
            .collect()
    }

    fn position(index: &Index, name: &str) -> u32 {
        index
            .entries()
            .iter()
            .position(|entry| entry.name() == name)
            .unwrap() as u32
    }

    #[test]
    fn the_first_column_is_the_tray_menus() {
        let index = index(MENUS);
        let columns = Columns::new(&index);
        assert_eq!(names(&index, &columns.focused().items), ["work", "alone"]);
        assert_eq!(columns.depth(), 0);
    }

    #[test]
    fn a_group_opens_on_what_it_holds_in_file_order_and_not_deeper() {
        let index = index(MENUS);
        let mut columns = Columns::new(&index);
        let work = position(&index, "work");
        assert!(columns.open(&index, work).is_some());
        assert_eq!(
            names(&index, &columns.focused().items),
            ["mail", "servers", "hollow", "scripts"]
        );
        assert_eq!(columns.crumbs(), [work]);
    }

    #[test]
    fn nothing_opens_from_an_app_or_an_empty_group() {
        let index = index(MENUS);
        let mut columns = Columns::new(&index);
        assert!(columns.open(&index, position(&index, "alone")).is_none());
        assert!(columns.open(&index, position(&index, "hollow")).is_none());
        assert_eq!(columns.depth(), 0);
    }

    #[test]
    fn a_browsed_folder_waits_for_its_listing_and_a_stale_one_is_dropped() {
        let index = index(MENUS);
        let mut columns = Columns::new(&index);
        let (scripts, mail) = (position(&index, "scripts"), position(&index, "mail"));
        assert!(columns.open(&index, scripts).is_some());
        assert_eq!(columns.focused().waiting, Some(scripts));

        assert!(!columns.listed(mail, vec![mail]), "not what it waits for");
        assert!(columns.focused().items.is_empty());
        assert!(columns.listed(scripts, vec![mail]));
        assert_eq!(columns.focused().waiting, None);
        assert_eq!(columns.focused().items, [mail]);
    }

    #[test]
    fn back_stops_at_the_first_column() {
        let index = index(MENUS);
        let mut columns = Columns::new(&index);
        columns.open(&index, position(&index, "work"));
        assert!(columns.back());
        assert!(!columns.back());
        assert_eq!(columns.depth(), 0);
    }

    #[test]
    fn the_preview_of_a_group_is_a_list_and_of_anything_else_its_details() {
        let index = index(MENUS);
        let (servers, mail) = (position(&index, "servers"), position(&index, "mail"));
        assert_eq!(
            Columns::preview(&index, Some(servers)),
            Preview::Items(vec![position(&index, "putty")])
        );
        assert_eq!(Columns::preview(&index, Some(mail)), Preview::Details(mail));
        let scripts = position(&index, "scripts");
        assert_eq!(
            Columns::preview(&index, Some(scripts)),
            Preview::Details(scripts),
            "a browsed folder is not read to preview it"
        );
        assert_eq!(Columns::preview(&index, None), Preview::Nothing);
    }

    #[test]
    fn include_narrows_the_first_column() {
        let index = index(
            r#"<y-pop>
                <search><menus include="&amp;work"/></search>
                <group name="Work"><app name="Mail" exe="m.exe"/></group>
                <group name="Play"><app name="Game" exe="g.exe"/></group>
            </y-pop>"#,
        );
        assert_eq!(
            names(&index, &Columns::new(&index).focused().items),
            ["work"]
        );
    }
}
