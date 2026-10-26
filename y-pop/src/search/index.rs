//! The index: one flat list of everything search can find, built from the
//! loaded [`ConfigTree`] at load and reload (other sources append later —
//! the Start menu, phase 9), never per keystroke.
//!
//! Every text field is **folded** here ([`super::matcher::fold`]), once, so a
//! query only ever compares ready-made lower-case accent-free text.

use crate::model::{ConfigTree, NodeId, NodeKind};

use super::hotkey::KeyCombo;
use super::matcher::fold;

/// Where an entry comes from. Decides its group in a result list and the
/// "only in search" label.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// A tray menu (or something in one).
    Menu,
    /// A group or folder of `<search>` that no tray icon shows.
    SearchOnly,
    /// The Windows Start menu (phase 9).
    StartMenu,
}

/// What an entry is. Decides its group and whether it is ranked lower.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    App,
    Group,
    /// A browsed folder; its files are only searched through `/`.
    Folder,
    /// A y-pop command: About, Quit, Settings… (and, phase 8, the `>` ones).
    /// Ranked below everything else.
    Command,
}

/// One searchable thing.
#[derive(Clone, Debug)]
pub struct Entry {
    pub source: Source,
    pub kind: Kind,
    /// The tree node it stands for. (Entries of other sources get their own
    /// kind of id when they exist.)
    pub id: NodeId,
    /// The groups it sits in, outermost first. Empty for a tray icon's own
    /// group or app.
    pub ancestors: Vec<NodeId>,
    /// What the entry is made of, for deciding which actions apply to it.
    pub facts: Facts,
    name: Box<str>,
    keywords: Box<str>,
    exe: Box<str>,
    path: Box<str>,
    tooltip: Box<str>,
    hotkeys: Vec<KeyCombo>,
}

/// What an entry is made of, beyond its kind: enough to say which actions
/// make sense for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Facts {
    /// A program that already launches elevated (`elevate="true"`), so
    /// "run as administrator" has nothing to add.
    pub elevated: bool,
    /// There is a file or folder behind it: Explorer can show it, and it has a
    /// path to copy.
    pub file: bool,
    /// A Store app, launched by its identity rather than a file.
    pub store: bool,
}

/// Which field of an entry a query matched, for the second line of a result
/// ("matched keyword *mail*").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Name,
    Keywords,
    Exe,
    /// The groups it sits in (`Work › Servers`).
    Path,
    Tooltip,
    Hotkey,
}

impl Field {
    /// How much a match in this field counts, in thousandths of the name's.
    /// (The hotkey has no weight: a combination typed in full is certain.)
    pub fn weight(self) -> u32 {
        match self {
            Self::Name | Self::Hotkey => 1000,
            Self::Keywords => 800,
            Self::Exe => 600,
            Self::Path => 450,
            Self::Tooltip => 400,
        }
    }
}

impl Entry {
    pub fn new(source: Source, id: NodeId, kind: Kind, name: &str) -> Self {
        Self {
            source,
            kind,
            id,
            ancestors: Vec::new(),
            facts: Facts::default(),
            name: fold(name).into(),
            keywords: "".into(),
            exe: "".into(),
            path: "".into(),
            tooltip: "".into(),
            hotkeys: Vec::new(),
        }
    }

    pub fn with_keywords(mut self, keywords: &str) -> Self {
        self.keywords = fold(keywords).into();
        self
    }

    /// The program's file name (`cmd.exe`, not its folder).
    pub fn with_exe(mut self, exe: &str) -> Self {
        let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
        self.exe = fold(file).into();
        self
    }

    /// The names of the groups it sits in, outermost first.
    pub fn with_path(mut self, groups: &[&str]) -> Self {
        self.path = fold(&groups.join(" › ")).into();
        self
    }

    pub fn with_tooltip(mut self, tooltip: &str) -> Self {
        self.tooltip = fold(tooltip).into();
        self
    }

    pub fn with_hotkey(mut self, hotkey: KeyCombo) -> Self {
        self.hotkeys.push(hotkey);
        self
    }

    /// The text fields a query word is tried against, with the field each
    /// is. Empty ones are left out.
    pub fn fields(&self) -> impl Iterator<Item = (Field, &str)> {
        [
            (Field::Name, &*self.name),
            (Field::Keywords, &*self.keywords),
            (Field::Exe, &*self.exe),
            (Field::Path, &*self.path),
            (Field::Tooltip, &*self.tooltip),
        ]
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn hotkeys(&self) -> &[KeyCombo] {
        &self.hotkeys
    }
}

/// Everything searchable, in configuration order — which is what breaks a
/// tie between two equally good matches.
#[derive(Default, Debug)]
pub struct Index {
    entries: Vec<Entry>,
}

impl Index {
    /// The tray menus, then `<search>`'s own groups and folders.
    pub fn from_tree(tree: &ConfigTree) -> Self {
        let mut index = Self::default();
        let mut groups = Vec::new();
        for &root in &tree.roots {
            index.add_node(tree, root, Source::Menu, &mut groups);
        }
        if let Some(search) = &tree.search {
            for node in search.nodes() {
                index.add_node(tree, node, Source::SearchOnly, &mut groups);
            }
        }
        index
    }

    /// `node`, then what is under it. `groups` is the chain of groups above.
    fn add_node(
        &mut self,
        tree: &ConfigTree,
        node: NodeId,
        source: Source,
        groups: &mut Vec<(NodeId, String)>,
    ) {
        let data = tree.node(node);
        let kind = match &data.kind {
            NodeKind::Separator | NodeKind::Break => return,
            NodeKind::Group => Kind::Group,
            NodeKind::App(app) if app.browse.is_some() => Kind::Folder,
            NodeKind::App(_) => Kind::App,
            NodeKind::About | NodeKind::Quit | NodeKind::Settings => Kind::Command,
        };

        let names: Vec<&str> = groups.iter().map(|(_, name)| name.as_str()).collect();
        let mut entry = Entry::new(source, node, kind, &data.name).with_path(&names);
        entry.ancestors = groups.iter().map(|&(id, _)| id).collect();
        if let Some(keywords) = &data.keywords {
            entry = entry.with_keywords(keywords);
        }
        if let Some(tooltip) = &data.tooltip {
            entry = entry.with_tooltip(tooltip);
        }
        if let NodeKind::App(app) = &data.kind
            && let Some(exe) = &app.exe
        {
            entry = entry.with_exe(exe);
        }
        if let NodeKind::App(app) = &data.kind {
            entry.facts = Facts {
                elevated: app.elevate,
                file: app.exe.is_some() && app.app_id.is_none(),
                store: app.app_id.is_some(),
            };
        }
        for binding in &data.hotkeys {
            // `Display` writes what `KeyCombo::parse` reads back.
            if let Some(combo) = KeyCombo::parse(&binding.hotkey.to_string()) {
                entry = entry.with_hotkey(combo);
            }
        }
        self.entries.push(entry);

        if matches!(data.kind, NodeKind::Group) {
            groups.push((node, data.name.to_string()));
            for child in tree.children(node) {
                self.add_node(tree, child, source, groups);
            }
            groups.pop();
        }
    }

    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::variables::Variable;

    fn tree(xml: &str) -> ConfigTree {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(xml, &mut builtins).expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        tree
    }

    #[test]
    fn menus_come_first_in_document_order_then_search_only_nodes() {
        let tree = tree(
            r#"<y-pop>
                <search><group name="Admin"><app name="Regedit" exe="regedit.exe"/></group></search>
                <group name="Work"><app name="Mail" exe="mail.exe"/><separator/><quit/></group>
            </y-pop>"#,
        );
        let index = Index::from_tree(&tree);
        let names: Vec<(&str, Source, Kind)> = index
            .entries()
            .iter()
            .map(|entry| (entry.name(), entry.source, entry.kind))
            .collect();
        assert_eq!(
            names,
            [
                ("work", Source::Menu, Kind::Group),
                ("mail", Source::Menu, Kind::App),
                ("quit", Source::Menu, Kind::Command),
                ("admin", Source::SearchOnly, Kind::Group),
                ("regedit", Source::SearchOnly, Kind::App),
            ],
            "no separator or break entries"
        );
    }

    #[test]
    fn fields_are_folded_and_the_exe_is_a_file_name() {
        let tree = tree(
            r#"<y-pop><group name="Bureau"><group name="Réseau">
                <app name="Café" exe="C:\Windows\System32\CMD.EXE" keywords="Boisson CHAUDE" tooltip="Un Café">
                    <hotkey key="Win+Shift+N"/>
                </app>
            </group></group></y-pop>"#,
        );
        let index = Index::from_tree(&tree);
        let cafe = index.entries().iter().find(|e| e.name() == "cafe").unwrap();

        let fields: Vec<(Field, &str)> = cafe.fields().collect();
        assert_eq!(
            fields,
            [
                (Field::Name, "cafe"),
                (Field::Keywords, "boisson chaude"),
                (Field::Exe, "cmd.exe"),
                (Field::Path, "bureau › reseau"),
                (Field::Tooltip, "un cafe"),
            ]
        );
        assert_eq!(cafe.hotkeys(), [KeyCombo::parse("shift+win+n").unwrap()]);
        assert_eq!(cafe.ancestors.len(), 2);
    }

    #[test]
    fn a_browsed_folder_is_a_folder_and_its_files_are_not_listed() {
        let tree = tree(r#"<y-pop><app name="Scripts" exe="C:\Scripts" browse="true"/></y-pop>"#);
        let index = Index::from_tree(&tree);
        assert_eq!(index.len(), 1);
        assert_eq!(index.entries()[0].kind, Kind::Folder);
    }

    #[test]
    fn an_empty_tree_is_an_empty_index() {
        assert!(Index::from_tree(&ConfigTree::empty(Vec::new())).is_empty());
    }
}
