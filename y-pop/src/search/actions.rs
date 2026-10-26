//! Which actions make sense for which result (spec §2.9). Pure.
//!
//! An action that does not apply is not in the action menu and its shortcut
//! does nothing — so a key bound to "run as administrator" quietly leaves a
//! group, or a program that already needs elevation, alone.

use crate::model::{ActionBinding, ActionId, ActionsConfig};

use super::index::{Entry, Kind, Source};
use super::keys::Chord;

/// Whether `id` can be done to `entry`.
pub fn applies(id: ActionId, entry: &Entry) -> bool {
    let has_path = matches!(entry.kind, Kind::App | Kind::Folder);
    match id {
        ActionId::Open => true,
        // y-pop already knows which programs need elevation; this is for the
        // others. A Store app is launched by identity, which has none.
        ActionId::RunAsAdmin => {
            entry.kind == Kind::App && entry.facts.file && !entry.facts.elevated
        }
        ActionId::ShowInFolder => has_path && entry.facts.file,
        ActionId::CopyPath => has_path && (entry.facts.file || entry.facts.store),
        ActionId::Reveal => entry.source == Source::Menu,
        ActionId::Edit => matches!(entry.source, Source::Menu | Source::SearchOnly),
        ActionId::AddToMenu => entry.source == Source::StartMenu,
        // Only a row of the history can be forgotten: `available` adds it there.
        ActionId::Forget => false,
    }
}

/// One line of an action menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Available {
    pub id: ActionId,
    /// Its shortcut, shown beside it.
    pub key: Option<Chord>,
    /// Shown as a button on the selected row.
    pub button: bool,
}

/// The configured actions that apply to `entry`, in the configured order.
/// `forgettable`: the entry is a row of the history (Recent), the only place
/// **Forget** applies.
pub fn available(actions: &ActionsConfig, entry: &Entry, forgettable: bool) -> Vec<Available> {
    actions
        .actions
        .iter()
        .filter(|binding| {
            applies(binding.id, entry) || (forgettable && binding.id == ActionId::Forget)
        })
        .map(|&ActionBinding { id, key, button }| Available { id, key, button })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::search::index::Index;
    use crate::variables::Variable;

    fn index() -> Index {
        let mut builtins: Vec<Variable> = Vec::new();
        let tree = config::load_str(
            r#"<y-pop>
                <search><group name="Admin"><app name="Regedit" exe="regedit.exe"/></group></search>
                <group name="Work">
                    <app name="Plain" exe="plain.exe"/>
                    <app name="Elevated" exe="e.exe" elevate="true"/>
                    <app name="Store" app-id="Pub.App_x!App"/>
                    <app name="Files" exe="C:\Files" browse="true"/>
                    <quit/>
                </group>
            </y-pop>"#,
            &mut builtins,
        )
        .unwrap();
        Index::from_tree(&tree)
    }

    fn entry<'a>(index: &'a Index, name: &str) -> &'a Entry {
        index
            .entries()
            .iter()
            .find(|entry| entry.name() == name)
            .unwrap()
    }

    fn applying(index: &Index, name: &str) -> Vec<&'static str> {
        ActionId::ALL
            .into_iter()
            .filter(|&id| applies(id, entry(index, name)))
            .map(ActionId::name)
            .collect()
    }

    #[test]
    fn a_program_gets_the_program_actions() {
        let index = index();
        assert_eq!(
            applying(&index, "plain"),
            [
                "open",
                "run-as-admin",
                "show-in-folder",
                "copy-path",
                "reveal",
                "edit"
            ]
        );
    }

    #[test]
    fn a_program_that_already_runs_elevated_is_not_offered_admin() {
        let index = index();
        let actions = applying(&index, "elevated");
        assert!(!actions.contains(&"run-as-admin"), "{actions:?}");
        assert!(actions.contains(&"show-in-folder"));
    }

    #[test]
    fn a_store_app_has_no_file_to_show_but_an_address_to_copy() {
        let index = index();
        assert_eq!(
            applying(&index, "store"),
            ["open", "copy-path", "reveal", "edit"]
        );
    }

    #[test]
    fn a_folder_can_be_shown_and_copied_but_not_run() {
        let index = index();
        assert_eq!(
            applying(&index, "files"),
            ["open", "show-in-folder", "copy-path", "reveal", "edit"]
        );
    }

    #[test]
    fn groups_and_commands_only_open_reveal_and_edit() {
        let index = index();
        assert_eq!(applying(&index, "work"), ["open", "reveal", "edit"]);
        assert_eq!(applying(&index, "quit"), ["open", "reveal", "edit"]);
    }

    #[test]
    fn something_only_in_search_has_no_menu_to_be_revealed_in() {
        let index = index();
        let actions = applying(&index, "regedit");
        assert!(!actions.contains(&"reveal"), "{actions:?}");
        assert!(actions.contains(&"edit"));
        assert_eq!(applying(&index, "admin"), ["open", "edit"]);
    }

    #[test]
    fn the_menu_lists_what_is_configured_and_applies_in_the_configured_order() {
        let index = index();
        let mut config = ActionsConfig::default();
        config
            .actions
            .retain(|binding| binding.id != ActionId::CopyPath);
        config.actions.reverse();

        let listed: Vec<_> = available(&config, entry(&index, "plain"), false)
            .iter()
            .map(|a| (a.id.name(), a.key.map(|key| key.to_string())))
            .collect();
        assert_eq!(
            listed,
            [
                ("edit", Some("Ctrl+E".to_owned())),
                ("reveal", Some("Ctrl+R".to_owned())),
                ("show-in-folder", None),
                ("run-as-admin", None),
                ("open", Some("Enter".to_owned())),
            ]
        );
    }
}
