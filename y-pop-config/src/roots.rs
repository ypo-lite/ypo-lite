//! The tray menus: the items directly under `<y-pop>`. Each one is a tray
//! icon, and each gets its own entry in the navigation pane.
//!
//! Plain functions on [`Settings`], no UI — so what "add a menu", "delete a
//! menu" and "what is this menu called" mean is tested without a window. A
//! root is addressed by [`ItemId`], never by index, so reordering roots
//! doesn't change which screen is open.

use y_pop::settings::{Entry, Item, ItemId, ItemKind, Settings};

/// Every root item, in tray-icon order. Comments between them are not menus.
pub fn menus(settings: &Settings) -> impl Iterator<Item = &Item> {
    settings.roots.iter().filter_map(|entry| match entry {
        Entry::Item(item) => Some(item),
        Entry::Comment(_) => None,
    })
}

#[cfg(test)]
pub fn find(settings: &Settings, id: ItemId) -> Option<&Item> {
    menus(settings).find(|item| item.id == id)
}

/// Where the root sits among the roots (comments counted), for remembering
/// the open screen across runs — ids only live as long as one load.
pub fn position(settings: &Settings, id: ItemId) -> Option<usize> {
    menus(settings).position(|item| item.id == id)
}

pub fn at_position(settings: &Settings, position: usize) -> Option<ItemId> {
    menus(settings).nth(position).map(|item| item.id)
}

/// The item `--select` names (see `y_pop::ipc::path_of`): positions among
/// the items — comments don't count — from a tray menu down. Returns the tray
/// menu it is in and the item. `None` when the path leads nowhere, which is
/// what a file edited since y-pop read it looks like.
pub fn resolve_path(settings: &Settings, path: &str) -> Option<(ItemId, ItemId)> {
    // `s/…`: a search-only group or folder, counted among those.
    let (only, path) = match path.strip_prefix("s/") {
        Some(rest) => (true, rest),
        None => (false, path),
    };
    let mut steps = path.split('/').map(|step| step.trim().parse::<usize>().ok());
    let first = steps.next()??;
    let root = if only {
        crate::search_only::roots(settings).nth(first)?
    } else {
        menus(settings).nth(first)?
    };
    let mut item = root;
    for step in steps {
        item = item
            .children
            .iter()
            .filter_map(|entry| match entry {
                Entry::Item(child) => Some(child),
                Entry::Comment(_) => None,
            })
            .nth(step?)?;
    }
    Some((root.id, item.id))
}

/// A menu name as it will read in the menu: `&F` is the mnemonic F, shown
/// underlined, and `&&` is a literal `&`. A lone `&` at the end means
/// nothing and is dropped, as Windows does.
///
/// The underline is U+0332, COMBINING LOW LINE, after the letter:
/// `windows-reactor`'s `TextBlock` has no underline or inline runs, and a
/// plain text is all a nav item, a tooltip or an ellipsized row can hold.
pub fn mnemonic(name: &str) -> String {
    let mut shown = String::with_capacity(name.len());
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        if c != '&' {
            shown.push(c);
            continue;
        }
        match chars.next() {
            Some('&') => shown.push('&'),
            Some(next) => {
                shown.push(next);
                shown.push('\u{332}');
            }
            None => {}
        }
    }
    shown
}

/// What the navigation pane calls a root: its name (see [`mnemonic`]), or what it is when it
/// has none.
pub fn label(item: &Item) -> String {
    match item.name.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => mnemonic(name),
        None => match item.kind {
            ItemKind::Group => "Unnamed menu".to_owned(),
            ItemKind::App(_) => "Unnamed app".to_owned(),
            ItemKind::Separator => "Separator".to_owned(),
            ItemKind::About => "About".to_owned(),
            ItemKind::Quit => "Quit".to_owned(),
            ItemKind::Settings => "Settings".to_owned(),
            ItemKind::Break => "Column break".to_owned(),
        },
    }
}

/// The first hotkey as written (`Win+Y`), as a hint beside the name.
pub fn first_hotkey(item: &Item) -> Option<&str> {
    item.hotkeys.first().map(|hotkey| hotkey.key.as_str())
}

/// Appends an empty menu named "New menu" (or "New menu 2", …, so two are
/// never confused in the pane) and returns its id.
pub fn add_menu(settings: &mut Settings) -> ItemId {
    let taken: Vec<String> = menus(settings)
        .filter_map(|item| item.name.clone())
        .collect();
    let name = (1..)
        .map(|n| {
            if n == 1 {
                "New menu".to_owned()
            } else {
                format!("New menu {n}")
            }
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_default();

    let mut item = settings.new_item(ItemKind::Group);
    item.name = Some(name);
    // Set apart from whatever came before, like a new paragraph.
    item.blank_before = !settings.roots.is_empty();
    let id = item.id;
    settings.roots.push(Entry::Item(item));
    id
}

/// Removes a root (and the comments attached to it, which are part of it)
/// and returns it.
pub fn delete(settings: &mut Settings, id: ItemId) -> Option<Item> {
    let index = settings
        .roots
        .iter()
        .position(|entry| matches!(entry, Entry::Item(item) if item.id == id))?;
    match settings.roots.remove(index) {
        Entry::Item(item) => Some(item),
        Entry::Comment(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_pop::variables::Variable;

    fn load(text: &str) -> Settings {
        Settings::load_str(text, &mut Vec::<Variable>::new()).expect("valid")
    }

    #[test]
    fn menus_skip_comments_and_keep_order() {
        let settings = load(
            r#"<y-pop><group name="A"/><!-- between --><group name="B"/><app name="C" exe="c.exe"/></y-pop>"#,
        );
        let names: Vec<String> = menus(&settings).map(label).collect();
        assert_eq!(names, ["A", "B", "C"]);
    }

    #[test]
    fn a_root_is_found_by_id_and_by_position() {
        let settings = load(r#"<y-pop><group name="A"/><group name="B"/></y-pop>"#);
        let b = menus(&settings).nth(1).expect("two menus").id;
        assert_eq!(
            find(&settings, b).and_then(|item| item.name.as_deref()),
            Some("B")
        );
        assert_eq!(position(&settings, b), Some(1));
        assert_eq!(at_position(&settings, 1), Some(b));
        assert_eq!(at_position(&settings, 2), None);
    }

    #[test]
    fn a_path_leads_to_an_item_counting_items_not_comments() {
        let settings = load(
            r#"<y-pop><group name="A"/><!-- c --><group name="B"><!-- c --><app name="x" exe="x.exe"/><group name="G"><app name="y" exe="y.exe"/></group></group></y-pop>"#,
        );
        let name = |path: &str| {
            let (_, id) = resolve_path(&settings, path)?;
            crate::tree::item(&settings, id)?.name.clone()
        };
        assert_eq!(name("1").as_deref(), Some("B"));
        assert_eq!(name("1/1/0").as_deref(), Some("y"));
        let (root, _) = resolve_path(&settings, "1/1/0").expect("there");
        assert_eq!(position(&settings, root), Some(1));

        assert_eq!(name("2"), None);
        assert_eq!(name("1/5"), None);
        assert_eq!(name("1/x"), None);
        assert_eq!(name(""), None);
    }

    #[test]
    fn an_s_path_leads_into_the_search_only_items() {
        let settings = load(
            r#"<y-pop><search><recent/><group name="G"><app name="a" exe="a.exe"/></group></search><group name="T"/></y-pop>"#,
        );
        let (root, item) = resolve_path(&settings, "s/0/0").expect("there");
        assert_eq!(
            crate::tree::item(&settings, item).and_then(|i| i.name.as_deref()),
            Some("a")
        );
        assert_eq!(
            crate::tree::item(&settings, root).and_then(|i| i.name.as_deref()),
            Some("G")
        );
        assert!(resolve_path(&settings, "s/1").is_none());
        assert!(resolve_path(&settings, "s/x").is_none());
    }

    #[test]
    fn an_unnamed_root_is_labelled_by_what_it_is() {
        let settings = load(r#"<y-pop><group/><quit name=""/></y-pop>"#);
        let labels: Vec<String> = menus(&settings).map(label).collect();
        assert_eq!(labels, ["Unnamed menu", "Quit"]);
    }

    #[test]
    fn the_first_hotkey_is_the_hint() {
        let settings = load(
            r#"<y-pop><group name="A"><hotkey key="Win+Y"/><hotkey key="Win+U"/></group></y-pop>"#,
        );
        assert_eq!(
            first_hotkey(menus(&settings).next().expect("one")),
            Some("Win+Y")
        );
    }

    #[test]
    fn new_menus_get_distinct_names_and_ids() {
        let mut settings = Settings::empty();
        let first = add_menu(&mut settings);
        let second = add_menu(&mut settings);
        assert_ne!(first, second);

        let names: Vec<String> = menus(&settings).map(label).collect();
        assert_eq!(names, ["New menu", "New menu 2"]);

        // And what was added is a menu y-pop can load.
        let written = settings.to_xml();
        let tree = y_pop::config::load_str(&written, &mut Vec::<Variable>::new()).expect("valid");
        assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
        assert_eq!(tree.roots.len(), 2);
    }

    #[test]
    fn deleting_a_root_takes_its_attached_comment_with_it() {
        let mut settings =
            load(r#"<y-pop><group name="A"/><!-- about B --><group name="B"/></y-pop>"#);
        let b = menus(&settings).nth(1).expect("two").id;

        let removed = delete(&mut settings, b).expect("there");
        assert_eq!(removed.lead.len(), 1);
        assert_eq!(settings.roots.len(), 1);
        assert!(!settings.to_xml().contains("about B"));
        assert!(delete(&mut settings, b).is_none());
    }
}

#[cfg(test)]
mod mnemonic_tests {
    use super::mnemonic;

    #[test]
    fn an_ampersand_underlines_the_next_character_and_doubled_is_literal() {
        assert_eq!(mnemonic("&File"), "F\u{332}ile");
        assert_eq!(mnemonic("Save &As"), "Save A\u{332}s");
        assert_eq!(mnemonic("R&&D"), "R&D");
        assert_eq!(mnemonic("&&&x"), "&x\u{332}");
        assert_eq!(mnemonic("End&"), "End");
        assert_eq!(mnemonic("Plain"), "Plain");
    }
}
