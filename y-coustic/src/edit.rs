//! Changing the lists of a [`Settings`] the way an editor does: add, remove,
//! move, duplicate. The typed lists and the [`Part`](crate::kept::Part)s that
//! go with them (comments, unknown attributes) are changed together, so a
//! comment stays with its entry through every move.
//!
//! Nothing here knows about a window: the functions take an index and answer
//! with what happened (the new index, or `false`/`None` when there was nothing
//! to do), which is what a settings app needs to select the right row and to
//! say what it did. An app that wants an Undo clones the `Settings` first.
//!
//! What goes with an entry that is removed: its comments and attributes go
//! too. They are about it, and keeping them for the entry that happens to
//! follow would make them say something else.

use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;

use crate::config::{Binding, Settings};
use crate::controls::{KeysGroup, Layer, Scope};
use crate::input::MouseBinding;
use crate::kept::Part;

/// The lists an editor changes. A group's own layers are edited through the
/// `*_group_layer` functions, which name the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    /// `<controls>`' `<keys>` groups.
    Keys,
    /// `<hotkeys>`.
    Hotkeys,
    /// `<mouse>`: wheels and buttons, in the order they were written.
    Mouse,
}

/// Parts are parallel to the values, and a list changed without them (a test,
/// a program that builds settings by hand) is short of some: they read as empty.
fn aligned<T, P: Default + Clone>(values: &[T], parts: &mut Vec<P>) {
    parts.resize(values.len(), P::default());
}

fn insert<T, P: Default + Clone>(
    values: &mut Vec<T>,
    parts: &mut Vec<P>,
    index: usize,
    value: T,
) -> usize {
    aligned(values, parts);
    let index = index.min(values.len());
    values.insert(index, value);
    parts.insert(index, P::default());
    index
}

fn remove<T, P: Default + Clone>(values: &mut Vec<T>, parts: &mut Vec<P>, index: usize) -> bool {
    aligned(values, parts);
    if index >= values.len() {
        return false;
    }
    values.remove(index);
    parts.remove(index);
    true
}

/// Swaps `index` with the one before it (`down` false) or after it.
fn step<T, P: Default + Clone>(
    values: &mut [T],
    parts: &mut Vec<P>,
    index: usize,
    down: bool,
) -> Option<usize> {
    aligned(values, parts);
    let other = if down {
        index.checked_add(1)?
    } else {
        index.checked_sub(1)?
    };
    if index >= values.len() || other >= values.len() {
        return None;
    }
    values.swap(index, other);
    parts.swap(index, other);
    Some(other)
}

/// A copy right after `index`, with no comments or attributes of its own.
fn duplicate<T: Clone, P: Default + Clone>(
    values: &mut Vec<T>,
    parts: &mut Vec<P>,
    index: usize,
) -> Option<usize> {
    aligned(values, parts);
    let copy = values.get(index)?.clone();
    Some(insert(values, parts, index + 1, copy))
}

impl Settings {
    /// How many entries `list` has.
    pub fn len_of(&self, list: List) -> usize {
        match list {
            List::Keys => self.controls.keys.len(),
            List::Hotkeys => self.hotkeys.len(),
            List::Mouse => self.mouse.len(),
        }
    }

    /// Adds a `<keys>` group at `index` (the end if past it); its index.
    pub fn insert_keys(&mut self, index: usize, group: KeysGroup) -> usize {
        insert(&mut self.controls.keys, &mut self.kept.keys, index, group)
    }

    /// Adds a `<hotkey>` at `index`; its index.
    pub fn insert_hotkey(&mut self, index: usize, binding: Binding) -> usize {
        insert(&mut self.hotkeys, &mut self.kept.hotkeys, index, binding)
    }

    /// Adds a `<wheel>` or `<button>` at `index`; its index.
    pub fn insert_mouse(&mut self, index: usize, binding: MouseBinding) -> usize {
        insert(&mut self.mouse, &mut self.kept.mouse, index, binding)
    }

    /// The layers of group `group`, when it has its own (not a target), with the
    /// parts that go with them kept parallel.
    fn group_layers_mut(&mut self, group: usize) -> Option<(&mut Vec<Layer>, &mut Vec<Part>)> {
        aligned(&self.controls.keys, &mut self.kept.keys);
        let Scope::Layers(layers) = &mut self.controls.keys.get_mut(group)?.scope else {
            return None;
        };
        Some((layers, &mut self.kept.keys[group].layers))
    }

    /// How many layers of its own group `group` has; 0 for a target or no group.
    pub fn group_layer_count(&self, group: usize) -> usize {
        match self.controls.keys.get(group).map(|group| &group.scope) {
            Some(Scope::Layers(layers)) => layers.len(),
            _ => 0,
        }
    }

    /// Adds a layer to the group at `index` (the end if past it); its index.
    /// `None` when the group is aimed by a target, or is not there.
    pub fn insert_group_layer(&mut self, group: usize, index: usize, layer: Layer) -> Option<usize> {
        let (layers, parts) = self.group_layers_mut(group)?;
        Some(insert(layers, parts, index, layer))
    }

    /// Removes the group's layer, with its comments.
    pub fn remove_group_layer(&mut self, group: usize, index: usize) -> bool {
        self.group_layers_mut(group)
            .is_some_and(|(layers, parts)| remove(layers, parts, index))
    }

    /// Moves the group's layer one place up or down; its new index.
    pub fn move_group_layer(&mut self, group: usize, index: usize, down: bool) -> Option<usize> {
        let (layers, parts) = self.group_layers_mut(group)?;
        step(layers, parts, index, down)
    }

    /// Aims group `group` by `scope`. Its layers' comments go with the layers
    /// when it changes to a target. Whether anything changed.
    pub fn set_group_scope(&mut self, group: usize, scope: Scope) -> bool {
        aligned(&self.controls.keys, &mut self.kept.keys);
        let Some(entry) = self.controls.keys.get_mut(group) else {
            return false;
        };
        if entry.scope == scope {
            return false;
        }
        entry.scope = scope;
        self.kept.keys[group].layers.clear();
        true
    }

    /// Gives group `to` a copy of the layers group `from` is aimed by (a target
    /// counts as its one layer), without their comments. Whether anything changed.
    pub fn copy_group_layers(&mut self, from: usize, to: usize) -> bool {
        let Some(layers) = self.controls.keys.get(from).map(|group| group.scope.layers()) else {
            return false;
        };
        from != to && self.set_group_scope(to, Scope::Layers(layers))
    }

    /// Removes the entry, with its comments. False when there is no such entry.
    pub fn remove_entry(&mut self, list: List, index: usize) -> bool {
        match list {
            List::Keys => remove(&mut self.controls.keys, &mut self.kept.keys, index),
            List::Hotkeys => remove(&mut self.hotkeys, &mut self.kept.hotkeys, index),
            List::Mouse => remove(&mut self.mouse, &mut self.kept.mouse, index),
        }
    }

    /// Moves the entry one place up (earlier in the file): its new index, or
    /// `None` when it is the first or does not exist. Its comments move with it.
    pub fn move_entry_up(&mut self, list: List, index: usize) -> Option<usize> {
        self.step_entry(list, index, false)
    }

    /// Moves the entry one place down: its new index, or `None`.
    pub fn move_entry_down(&mut self, list: List, index: usize) -> Option<usize> {
        self.step_entry(list, index, true)
    }

    fn step_entry(&mut self, list: List, index: usize, down: bool) -> Option<usize> {
        match list {
            List::Keys => step(&mut self.controls.keys, &mut self.kept.keys, index, down),
            List::Hotkeys => step(&mut self.hotkeys, &mut self.kept.hotkeys, index, down),
            List::Mouse => step(&mut self.mouse, &mut self.kept.mouse, index, down),
        }
    }

    /// A copy of the entry right after it, without its comments: its index.
    pub fn duplicate_entry(&mut self, list: List, index: usize) -> Option<usize> {
        match list {
            List::Keys => duplicate(&mut self.controls.keys, &mut self.kept.keys, index),
            List::Hotkeys => duplicate(&mut self.hotkeys, &mut self.kept.hotkeys, index),
            List::Mouse => duplicate(&mut self.mouse, &mut self.kept.mouse, index),
        }
    }

    /// What the groups, the layers and the single hotkeys cross into, said once
    /// each: a combination used twice (the second use is ignored) and a layer
    /// that uses `Alt`. The same lines `Settings::parse` reports at load.
    pub fn crossing_warnings(&self) -> Vec<String> {
        crate::controls::expand(&self.hotkeys, &self.controls).1
    }

    /// The same problems, each with the row it belongs on, and every binding with
    /// where it came from.
    pub fn expansion(&self) -> crate::controls::Expansion {
        crate::controls::expand_origins(&self.hotkeys, &self.controls)
    }

    /// The mouse bindings that an earlier one shadows (same input, same
    /// modifiers): the first wins at run time, so these never fire. Each is the
    /// index of the shadowed binding.
    pub fn shadowed_mouse_bindings(&self) -> Vec<usize> {
        self.mouse
            .iter()
            .enumerate()
            .filter(|(index, binding)| {
                self.mouse[..*index]
                    .iter()
                    .any(|earlier| {
                        earlier.input == binding.input && earlier.modifiers == binding.modifiers
                    })
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// `modifiers` with `modifier` switched: what a chip for one modifier key does.
pub fn toggle_modifier(modifiers: u32, modifier: HOT_KEY_MODIFIERS) -> u32 {
    modifiers ^ modifier.0
}

/// Whether `modifiers` has `modifier`.
pub fn has_modifier(modifiers: u32, modifier: HOT_KEY_MODIFIERS) -> bool {
    modifiers & modifier.0 != 0
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_SHIFT};

    use super::*;
    use crate::model::Target;

    fn load(body: &str) -> Settings {
        Settings::parse(
            PathBuf::from("test.xml"),
            &format!("<y-coustic>{body}</y-coustic>"),
        )
        .expect("loads")
    }

    fn hotkeys() -> Settings {
        load(
            r#"<hotkeys>
                <!-- first -->
                <hotkey key="Win+F1" action="mute"/>
                <!-- second -->
                <hotkey key="Win+F2" action="volume-up"/>
                <!-- third -->
                <hotkey key="Win+F3" action="volume-down"/>
            </hotkeys>"#,
        )
    }

    fn lead(settings: &Settings, index: usize) -> &str {
        &settings.kept.hotkeys[index].lead[0].text
    }

    #[test]
    fn a_comment_moves_with_its_entry() {
        let mut settings = hotkeys();
        assert_eq!(settings.move_entry_down(List::Hotkeys, 0), Some(1));
        assert_eq!(lead(&settings, 0), "second");
        assert_eq!(lead(&settings, 1), "first");
        assert_eq!(settings.hotkeys[1].action.name(), "mute");

        assert_eq!(settings.move_entry_up(List::Hotkeys, 2), Some(1));
        assert_eq!(lead(&settings, 1), "third");

        assert_eq!(settings.move_entry_up(List::Hotkeys, 0), None);
        assert_eq!(settings.move_entry_down(List::Hotkeys, 2), None);
        assert_eq!(settings.move_entry_down(List::Hotkeys, 9), None);
    }

    #[test]
    fn removing_an_entry_removes_its_comments_and_nobody_elses() {
        let mut settings = hotkeys();
        assert!(settings.remove_entry(List::Hotkeys, 1));
        assert_eq!(settings.hotkeys.len(), 2);
        assert_eq!(lead(&settings, 0), "first");
        assert_eq!(lead(&settings, 1), "third");
        assert!(!settings.remove_entry(List::Hotkeys, 5));
        let text = settings.to_xml();
        assert!(!text.contains("second"), "{text}");
    }

    #[test]
    fn a_new_entry_has_no_comments_and_the_others_keep_theirs() {
        let mut settings = hotkeys();
        let binding = settings.hotkeys[0];
        assert_eq!(settings.insert_hotkey(1, binding), 1);
        assert!(settings.kept.hotkeys[1].is_empty());
        assert_eq!(lead(&settings, 0), "first");
        assert_eq!(lead(&settings, 2), "second");

        // Past the end is the end.
        assert_eq!(settings.insert_hotkey(99, binding), 4);

        assert_eq!(settings.duplicate_entry(List::Hotkeys, 0), Some(1));
        assert!(settings.kept.hotkeys[1].is_empty());
        assert_eq!(settings.duplicate_entry(List::Hotkeys, 99), None);
    }

    #[test]
    fn a_list_built_without_parts_can_still_be_edited() {
        let mut settings = hotkeys();
        let binding = settings.hotkeys[0];
        settings.kept.hotkeys.clear();
        settings.hotkeys.push(binding);
        assert_eq!(settings.move_entry_up(List::Hotkeys, 3), Some(2));
        assert_eq!(settings.kept.hotkeys.len(), settings.hotkeys.len());
    }

    #[test]
    fn an_edited_file_saves_with_the_comments_where_the_entries_went() {
        let mut settings = hotkeys();
        settings.move_entry_down(List::Hotkeys, 0);
        let text = settings.to_xml();
        let second = text.find("<!-- second -->").unwrap();
        let first = text.find("<!-- first -->").unwrap();
        assert!(second < first, "{text}");
        let again = Settings::parse(PathBuf::from("test.xml"), &text).unwrap();
        assert_eq!(again.hotkeys, settings.hotkeys);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    }

    #[test]
    fn a_group_s_layers_are_edited_apart_and_keep_their_comments() {
        let mut settings = load(
            r#"<controls>
                <!-- keys -->
                <keys up="Win+NumAdd">
                    <!-- bare -->
                    <layer mod="none" target="master"/>
                    <layer mod="Shift" target="mic"/>
                </keys>
                <keys mute="Win+Alt+M" target="mic"/>
            </controls>"#,
        );
        assert_eq!(settings.len_of(List::Keys), 2);
        assert_eq!(settings.group_layer_count(0), 2);
        // A target-aimed group has no layers of its own to edit.
        assert_eq!(settings.group_layer_count(1), 0);
        let extra = Layer { modifiers: MOD_CONTROL.0, target: Target::Focused };
        assert_eq!(settings.insert_group_layer(1, 0, extra), None);

        assert_eq!(settings.insert_group_layer(0, 1, extra), Some(1));
        assert_eq!(settings.group_layer_count(0), 3);
        assert_eq!(settings.kept.keys[0].layers[0].lead[0].text, "bare");
        assert!(settings.kept.keys[0].layers[1].is_empty());

        // The comment stays with its layer through a move.
        assert_eq!(settings.move_group_layer(0, 0, true), Some(1));
        assert_eq!(settings.kept.keys[0].layers[1].lead[0].text, "bare");
        assert_eq!(settings.move_group_layer(0, 2, true), None);

        assert!(settings.remove_group_layer(0, 1));
        assert_eq!(settings.group_layer_count(0), 2);
        assert!(!settings.remove_group_layer(0, 9));
        let text = settings.to_xml();
        assert!(!text.contains("bare"), "{text}");
        assert!(text.contains("<!-- keys -->"), "{text}");
    }

    #[test]
    fn a_group_s_scope_and_its_layers_can_be_copied() {
        let mut settings = load(
            r#"<controls>
                <keys up="Win+NumAdd">
                    <layer mod="none" target="master"/>
                    <layer mod="Shift" target="mic"/>
                </keys>
                <keys mute="Win+Alt+M" target="mic"/>
            </controls>"#,
        );
        // The target group takes the layers of the first.
        assert!(settings.copy_group_layers(0, 1));
        assert_eq!(settings.group_layer_count(1), 2);
        assert_eq!(settings.controls.keys[0].scope, settings.controls.keys[1].scope);
        assert!(!settings.copy_group_layers(0, 1), "nothing changes the second time");
        assert!(!settings.copy_group_layers(0, 0));
        assert!(!settings.copy_group_layers(0, 9));

        // And back to one target: the layers' comments go with them.
        assert!(settings.set_group_scope(1, Scope::Target(Target::Mic)));
        assert_eq!(settings.group_layer_count(1), 0);
        assert!(!settings.set_group_scope(1, Scope::Target(Target::Mic)));
    }

    #[test]
    fn removing_or_moving_a_group_takes_its_layers_and_their_comments_along() {
        let mut settings = load(
            r#"<controls>
                <keys up="F1"><!-- one --><layer mod="none"/></keys>
                <keys up="F2"><!-- two --><layer mod="Ctrl"/></keys>
            </controls>"#,
        );
        assert_eq!(settings.move_entry_down(List::Keys, 0), Some(1));
        assert_eq!(settings.kept.keys[1].layers[0].lead[0].text, "one");
        assert_eq!(settings.kept.keys[0].layers[0].lead[0].text, "two");
        assert!(settings.remove_entry(List::Keys, 0));
        assert_eq!(settings.kept.keys.len(), 1);
        assert_eq!(settings.kept.keys[0].layers[0].lead[0].text, "one");

        assert_eq!(settings.duplicate_entry(List::Keys, 0), Some(1));
        assert_eq!(settings.group_layer_count(1), 1);
        assert!(settings.kept.keys[1].is_empty(), "a copy has no comments");
    }

    #[test]
    fn what_is_used_twice_is_found() {
        let settings = load(
            r#"<hotkeys>
                <hotkey key="Win+F1" action="mute"/>
                <hotkey key="Win+F1" action="volume-up"/>
            </hotkeys>"#,
        );
        assert_eq!(settings.hotkeys.len(), 2);
        assert_eq!(settings.crossing_warnings().len(), 1);

        let mut mouse = load(r#"<mouse><wheel mod="Alt"/></mouse>"#);
        let again = mouse.mouse[0];
        mouse.insert_mouse(1, again);
        assert_eq!(mouse.shadowed_mouse_bindings(), vec![1]);
        mouse.mouse[1].modifiers = MOD_ALT.0 | MOD_SHIFT.0;
        assert!(mouse.shadowed_mouse_bindings().is_empty());
    }

    #[test]
    fn modifiers_toggle_one_key_at_a_time() {
        let mut modifiers = 0;
        modifiers = toggle_modifier(modifiers, MOD_CONTROL);
        assert!(has_modifier(modifiers, MOD_CONTROL));
        modifiers = toggle_modifier(modifiers, MOD_SHIFT);
        assert!(has_modifier(modifiers, MOD_SHIFT));
        modifiers = toggle_modifier(modifiers, MOD_CONTROL);
        assert!(!has_modifier(modifiers, MOD_CONTROL));
        assert!(has_modifier(modifiers, MOD_SHIFT));
        assert!(!has_modifier(modifiers, MOD_ALT));
    }
}
