//! The edits the simple screens make to a [`Settings`], as plain data and one
//! function. No window: a screen builds an [`Edit`] from what the user did,
//! [`apply`] writes it (clamped to the ranges the reader uses) and says whether
//! anything changed, which is what decides if the file is dirty. A value equal
//! to the current one changes nothing, because WinUI controls report a value
//! when it is first set, not only when the user changes it.

use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL};
use y_coustic::appearance::Rule;
use y_coustic::config::{
    Anchor, BORDER_RANGE, FADE_RANGE, HOVER_DELAY_RANGE, MAX_ROWS_RANGE, OPACITY_RANGE, Position,
    RADIUS_RANGE, SCALE_RANGE, STEP_RANGE, Settings, Style, TIMEOUT_RANGE, TrayAction, WatchMode,
};
use y_coustic::controls::{Layer, Plain, Scope};
use y_coustic::edit::{List, toggle_modifier};
use y_coustic::input::{Input, MouseBinding};
use y_coustic::model::{Action, Target};

/// A device whose changes from outside can be watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Master,
    Microphone,
}

/// The three ways to click the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Single,
    Double,
    Middle,
}

/// One of the three keys of a `<keys>` group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKey {
    Up,
    Down,
    Mute,
}

/// What a single hotkey does, without the level that `SetVolume` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    VolumeUp,
    VolumeDown,
    Mute,
    SetVolume,
    Mixer,
}

impl ActionKind {
    pub fn of(action: Action) -> Self {
        match action {
            Action::VolumeUp => Self::VolumeUp,
            Action::VolumeDown => Self::VolumeDown,
            Action::Mute => Self::Mute,
            Action::SetVolume(_) => Self::SetVolume,
            Action::Mixer => Self::Mixer,
        }
    }

    /// The action of this kind, taking the level from `current` when that is
    /// one too: switching away and back keeps what was typed.
    fn action(self, current: Action) -> Action {
        match self {
            Self::VolumeUp => Action::VolumeUp,
            Self::VolumeDown => Action::VolumeDown,
            Self::Mute => Action::Mute,
            Self::SetVolume => match current {
                Action::SetVolume(_) => current,
                _ => Action::SetVolume(DEFAULT_LEVEL),
            },
            Self::Mixer => Action::Mixer,
        }
    }
}

/// The level a new *set the level* hotkey starts at.
const DEFAULT_LEVEL: u8 = 50;

/// What a new mouse binding starts as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Wheel,
    Button,
}

/// How a group is aimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Target,
    Layers,
}

impl ScopeKind {
    pub fn of(scope: &Scope) -> Self {
        match scope {
            Scope::Target(_) => Self::Target,
            Scope::Layers(_) => Self::Layers,
        }
    }
}

/// What a group is aimed by once it is changed to `kind`.
fn scope_as(current: &Scope, kind: ScopeKind) -> Scope {
    match (kind, current) {
        (ScopeKind::Target, Scope::Layers(layers)) => {
            Scope::Target(layers.first().map_or(Target::Master, |layer| layer.target))
        }
        (ScopeKind::Layers, Scope::Target(target)) => Scope::Layers(vec![Layer {
            modifiers: 0,
            target: *target,
        }]),
        _ => current.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    Step(u32),
    Watch(Endpoint, WatchMode),
    HoverDelay(u32),
    TrayClick(Click, TrayAction),
    TrayWheel(bool),
    TrayIcon(Rule),
    MixerAnchor(Anchor),
    MaxRows(u32),
    SystemSounds(bool),

    // Keys
    /// The tray toggle: every media group lets y-coustic take the bare volume keys.
    ReplaceWindowsVolume(bool),
    /// Whether a group lets Windows keep its bare volume keys.
    Plain(usize, Plain),
    /// Empties one key of a group; refused for its last key.
    ClearGroupKey(usize, GroupKey),
    /// How a group is aimed: by one target, or by layers of its own. Changing to a
    /// target keeps the first layer's target; changing to layers makes the target
    /// the one layer with no modifier, so nothing is lost that way round.
    GroupScope(usize, ScopeKind),
    /// The target of a group aimed by one target.
    GroupTarget(usize, Target),
    /// A modifier of one of a group's layers, switched on or off.
    GroupLayerModifier(usize, usize, HOT_KEY_MODIFIERS),
    GroupLayerTarget(usize, usize, Target),
    AddGroupLayer(usize),
    MoveGroupLayer {
        group: usize,
        index: usize,
        up: bool,
    },
    RemoveGroupLayer(usize, usize),
    /// A copy of the group right after it, with its layers.
    DuplicateGroup(usize),
    /// Gives the first group a copy of the layers of the second.
    CopyLayers {
        to: usize,
        from: usize,
    },
    HotkeyAction(usize, ActionKind),
    HotkeyTarget(usize, Target),
    /// The level of a *set the level* hotkey, in percent.
    HotkeyLevel(usize, u8),
    Move {
        list: List,
        index: usize,
        up: bool,
    },
    Remove(List, usize),

    // Mouse
    /// What a binding listens for. Changing between a wheel and a button gives it
    /// what the new kind needs: a button an action, a wheel none.
    MouseInput(usize, Input),
    /// A modifier of a binding, switched on or off; refused for the last one of an
    /// input that may not go bare.
    MouseModifier(usize, HOT_KEY_MODIFIERS),
    MouseTarget(usize, Target),
    /// What a button does (a wheel has no action to choose).
    MouseAction(usize, ActionKind),
    /// The level of a button that sets one, in percent.
    MouseLevel(usize, u8),
    /// A wheel only: up lowers and down raises.
    MouseInvert(usize, bool),
    AddMouse(MouseKind),

    // HUD
    HudStyle(Style),
    HudPosition(Position),
    HudTimeout(u32),
    HudFade(u32),
    HudScale(u32),
    HudRadius(u32),
    HudBorder(u32),
    HudOpacity(u32),
    HudInteractive(bool),
}

impl Edit {
    /// What to call the Undo of this edit, for those worth one: a removed entry, or
    /// layers that something replaces.
    pub fn undo_label(self, settings: &Settings) -> Option<&'static str> {
        let layers_of = |group: usize| match settings.controls.keys.get(group).map(|g| &g.scope) {
            Some(Scope::Layers(layers)) => layers.len(),
            _ => 0,
        };
        match self {
            Self::Remove(List::Keys, _) => Some("Deleted a group of keys"),
            Self::RemoveGroupLayer(..) => Some("Deleted a layer"),
            // One target is a lossy summary of several layers.
            Self::GroupScope(group, ScopeKind::Target) if layers_of(group) > 1 => {
                Some("Replaced a group's layers with one target")
            }
            Self::CopyLayers { to, .. } if layers_of(to) > 1 => {
                Some("Replaced a group's layers with a copy")
            }
            Self::Remove(List::Hotkeys, _) => Some("Deleted a hotkey"),
            Self::Remove(List::Mouse, _) => Some("Deleted a mouse binding"),
            _ => None,
        }
    }
}

/// Sets `value` unless it is already there; whether it changed.
fn set<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        false
    } else {
        *slot = value;
        true
    }
}

fn clamped(value: u32, range: &std::ops::RangeInclusive<u32>) -> u32 {
    value.clamp(*range.start(), *range.end())
}

/// A number a control reported as a float, as the whole number it stands for.
pub fn whole(value: f64) -> u32 {
    if value.is_finite() && value > 0.0 {
        value.round() as u32
    } else {
        0
    }
}

/// The HUD's font family, from the text box. An empty box is not a font: the
/// previous one stays until something is typed, so clearing the box to type
/// another name does not rewrite what is being typed.
pub fn set_font(settings: &mut Settings, text: &str) -> bool {
    let name = text.trim();
    !name.is_empty() && set(&mut settings.hud.font, name.to_owned())
}

/// Writes `edit` into `settings`; whether anything changed.
pub fn apply(settings: &mut Settings, edit: Edit) -> bool {
    match edit {
        Edit::Step(step) => set(&mut settings.step, clamped(step, &STEP_RANGE)),
        Edit::Watch(Endpoint::Master, mode) => set(&mut settings.watch.master, mode),
        Edit::Watch(Endpoint::Microphone, mode) => set(&mut settings.watch.microphone, mode),
        Edit::HoverDelay(delay) => set(
            &mut settings.tray.hover_delay,
            clamped(delay, &HOVER_DELAY_RANGE),
        ),
        Edit::TrayClick(Click::Single, action) => set(&mut settings.tray.click, action),
        Edit::TrayClick(Click::Double, action) => set(&mut settings.tray.double_click, action),
        Edit::TrayClick(Click::Middle, action) => set(&mut settings.tray.middle_click, action),
        Edit::TrayWheel(on) => set(&mut settings.tray.wheel, on),
        Edit::TrayIcon(rule) => set(&mut settings.tray.icon, rule),
        Edit::MixerAnchor(anchor) => set(&mut settings.mixer.anchor, anchor),
        Edit::MaxRows(rows) => set(&mut settings.mixer.max_rows, clamped(rows, &MAX_ROWS_RANGE)),
        Edit::SystemSounds(on) => set(&mut settings.mixer.system_sounds, on),
        Edit::ReplaceWindowsVolume(on) => {
            if settings.replaces_windows_volume() == on {
                false
            } else {
                settings.set_replace_windows_volume(on);
                true
            }
        }
        Edit::Plain(index, plain) => settings
            .controls
            .keys
            .get_mut(index)
            .is_some_and(|group| set(&mut group.plain, plain)),
        Edit::ClearGroupKey(index, which) => {
            let Some(group) = settings.controls.keys.get_mut(index) else {
                return false;
            };
            let kept = [group.up, group.down, group.mute]
                .iter()
                .filter(|key| key.is_some())
                .count();
            let slot = match which {
                GroupKey::Up => &mut group.up,
                GroupKey::Down => &mut group.down,
                GroupKey::Mute => &mut group.mute,
            };
            // A group with no key at all is not a group y-coustic reads.
            if kept < 2 || slot.is_none() {
                return false;
            }
            *slot = None;
            true
        }
        Edit::GroupScope(group, kind) => {
            let Some(current) = settings.controls.keys.get(group).map(|g| g.scope.clone()) else {
                return false;
            };
            settings.set_group_scope(group, scope_as(&current, kind))
        }
        Edit::GroupTarget(group, target) => {
            matches!(
                settings.controls.keys.get(group).map(|g| &g.scope),
                Some(Scope::Target(_))
            ) && settings.set_group_scope(group, Scope::Target(target))
        }
        Edit::GroupLayerModifier(group, index, modifier) => {
            match settings.controls.keys.get_mut(group).map(|g| &mut g.scope) {
                Some(Scope::Layers(layers)) => layers
                    .get_mut(index)
                    .map(|layer| layer.modifiers = toggle_modifier(layer.modifiers, modifier))
                    .is_some(),
                _ => false,
            }
        }
        Edit::GroupLayerTarget(group, index, target) => {
            match settings.controls.keys.get_mut(group).map(|g| &mut g.scope) {
                Some(Scope::Layers(layers)) => layers
                    .get_mut(index)
                    .is_some_and(|layer| set(&mut layer.target, target)),
                _ => false,
            }
        }
        Edit::AddGroupLayer(group) => {
            let at = settings.group_layer_count(group);
            settings
                .insert_group_layer(
                    group,
                    at,
                    Layer {
                        modifiers: MOD_CONTROL.0,
                        target: Target::Focused,
                    },
                )
                .is_some()
        }
        Edit::MoveGroupLayer { group, index, up } => settings
            .move_group_layer(group, index, !up)
            .is_some(),
        Edit::RemoveGroupLayer(group, index) => settings.remove_group_layer(group, index),
        Edit::DuplicateGroup(group) => settings.duplicate_entry(List::Keys, group).is_some(),
        Edit::CopyLayers { to, from } => settings.copy_group_layers(from, to),
        Edit::HotkeyAction(index, kind) => settings
            .hotkeys
            .get_mut(index)
            .is_some_and(|binding| {
                let action = kind.action(binding.action);
                set(&mut binding.action, action)
            }),
        Edit::HotkeyTarget(index, target) => settings
            .hotkeys
            .get_mut(index)
            .is_some_and(|binding| set(&mut binding.target, target)),
        Edit::HotkeyLevel(index, level) => settings.hotkeys.get_mut(index).is_some_and(|binding| {
            matches!(binding.action, Action::SetVolume(_))
                && set(&mut binding.action, Action::SetVolume(level.min(100)))
        }),
        Edit::Move { list, index, up } => if up {
            settings.move_entry_up(list, index)
        } else {
            settings.move_entry_down(list, index)
        }
        .is_some(),
        Edit::Remove(list, index) => settings.remove_entry(list, index),
        Edit::MouseInput(index, input) => settings.mouse.get_mut(index).is_some_and(|binding| {
            if binding.input == input {
                return false;
            }
            binding.input = input;
            if input.is_wheel() {
                binding.action = None;
            } else {
                binding.invert = false;
                binding.action.get_or_insert(Action::Mute);
            }
            // A bare wheel or main button would take over every scroll or click.
            if binding.modifiers == 0 && !input.may_be_bare() {
                binding.modifiers = MOD_ALT.0;
            }
            true
        }),
        Edit::MouseModifier(index, modifier) => {
            settings.mouse.get_mut(index).is_some_and(|binding| {
                let next = toggle_modifier(binding.modifiers, modifier);
                if next == 0 && !binding.input.may_be_bare() {
                    return false;
                }
                binding.modifiers = next;
                true
            })
        }
        Edit::MouseTarget(index, target) => settings
            .mouse
            .get_mut(index)
            .is_some_and(|binding| set(&mut binding.target, target)),
        Edit::MouseAction(index, kind) => settings.mouse.get_mut(index).is_some_and(|binding| {
            let Some(current) = binding.action else {
                return false;
            };
            let action = kind.action(current);
            let mut changed = set(&mut binding.action, Some(action));
            // The mixer is opened, not aimed.
            if action == Action::Mixer {
                changed |= set(&mut binding.target, Target::Master);
            }
            changed
        }),
        Edit::MouseLevel(index, level) => settings.mouse.get_mut(index).is_some_and(|binding| {
            matches!(binding.action, Some(Action::SetVolume(_)))
                && set(&mut binding.action, Some(Action::SetVolume(level.min(100))))
        }),
        Edit::MouseInvert(index, on) => settings
            .mouse
            .get_mut(index)
            .is_some_and(|binding| binding.input.is_wheel() && set(&mut binding.invert, on)),
        Edit::HudStyle(style) => set(&mut settings.hud.style, style),
        Edit::HudPosition(position) => set(&mut settings.hud.position, position),
        Edit::HudTimeout(value) => set(&mut settings.hud.timeout, clamped(value, &TIMEOUT_RANGE)),
        Edit::HudFade(value) => set(&mut settings.hud.fade, clamped(value, &FADE_RANGE)),
        Edit::HudScale(value) => set(&mut settings.hud.scale, clamped(value, &SCALE_RANGE)),
        Edit::HudRadius(value) => set(&mut settings.hud.radius, clamped(value, &RADIUS_RANGE)),
        Edit::HudBorder(value) => set(&mut settings.hud.border, clamped(value, &BORDER_RANGE)),
        Edit::HudOpacity(value) => set(&mut settings.hud.opacity, clamped(value, &OPACITY_RANGE)),
        Edit::HudInteractive(on) => set(&mut settings.hud.interactive, on),
        Edit::AddMouse(kind) => {
            let at = settings.len_of(List::Mouse);
            let binding = match kind {
                MouseKind::Wheel => MouseBinding {
                    input: Input::Wheel,
                    modifiers: MOD_ALT.0,
                    target: Target::Master,
                    action: None,
                    invert: false,
                },
                MouseKind::Button => MouseBinding {
                    input: Input::Middle,
                    modifiers: MOD_ALT.0,
                    target: Target::Master,
                    action: Some(Action::Mute),
                    invert: false,
                },
            };
            settings.insert_mouse(at, binding);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn settings() -> Settings {
        Settings::parse(
            PathBuf::from("test.xml"),
            y_coustic::config::DEFAULT_XML,
        )
        .expect("the shipped file loads")
    }

    #[test]
    fn an_edit_changes_its_value_and_says_so_once() {
        let mut settings = settings();
        let edits = [
            Edit::Step(12),
            Edit::Watch(Endpoint::Master, WatchMode::Hud),
            Edit::Watch(Endpoint::Microphone, WatchMode::Off),
            Edit::HoverDelay(900),
            Edit::TrayClick(Click::Single, TrayAction::Hud),
            Edit::TrayClick(Click::Double, TrayAction::Mute),
            Edit::TrayClick(Click::Middle, TrayAction::MuteMic),
            Edit::TrayWheel(false),
            Edit::TrayIcon(Rule::MicMuted),
            Edit::MixerAnchor(Anchor::Cursor),
            Edit::MaxRows(12),
            Edit::SystemSounds(false),
        ];
        for edit in edits {
            assert!(apply(&mut settings, edit), "{edit:?} changed nothing");
            assert!(!apply(&mut settings, edit), "{edit:?} reported a change twice");
        }
        assert_eq!(settings.step, 12);
        assert_eq!(settings.watch.master, WatchMode::Hud);
        assert_eq!(settings.tray.double_click, TrayAction::Mute);
        assert_eq!(settings.mixer.max_rows, 12);
    }

    #[test]
    fn numbers_are_kept_inside_the_ranges_the_reader_uses() {
        let mut settings = settings();
        apply(&mut settings, Edit::Step(0));
        assert_eq!(settings.step, *STEP_RANGE.start());
        apply(&mut settings, Edit::Step(500));
        assert_eq!(settings.step, *STEP_RANGE.end());
        apply(&mut settings, Edit::HoverDelay(99_999));
        assert_eq!(settings.tray.hover_delay, *HOVER_DELAY_RANGE.end());
        apply(&mut settings, Edit::MaxRows(1));
        assert_eq!(settings.mixer.max_rows, *MAX_ROWS_RANGE.start());
    }

    #[test]
    fn a_group_never_loses_its_last_key() {
        let mut settings = settings();
        let numpad = settings
            .controls
            .keys
            .iter()
            .position(|group| !group.has_media_keys())
            .expect("the shipped file has a numpad group");
        // Up, down and mute: two can go, the last cannot.
        assert!(apply(&mut settings, Edit::ClearGroupKey(numpad, GroupKey::Mute)));
        assert!(apply(&mut settings, Edit::ClearGroupKey(numpad, GroupKey::Down)));
        assert!(!apply(&mut settings, Edit::ClearGroupKey(numpad, GroupKey::Up)));
        assert!(settings.controls.keys[numpad].up.is_some());
        // Already empty: nothing to clear.
        assert!(!apply(&mut settings, Edit::ClearGroupKey(numpad, GroupKey::Mute)));
    }

    #[test]
    fn a_group_s_layers_toggle_move_and_go_one_at_a_time() {
        let mut settings = settings();
        let layers = |settings: &Settings| settings.controls.keys[0].scope.layers();
        let before = layers(&settings)[0].modifiers;
        assert!(apply(&mut settings, Edit::GroupLayerModifier(0, 0, MOD_CONTROL)));
        assert_ne!(layers(&settings)[0].modifiers, before);
        assert!(apply(&mut settings, Edit::GroupLayerModifier(0, 0, MOD_CONTROL)));
        assert_eq!(layers(&settings)[0].modifiers, before);
        assert!(!apply(&mut settings, Edit::GroupLayerModifier(0, 99, MOD_CONTROL)));
        assert!(!apply(&mut settings, Edit::GroupLayerModifier(99, 0, MOD_CONTROL)));

        assert!(apply(&mut settings, Edit::GroupLayerTarget(0, 0, Target::Pointed)));
        assert!(!apply(&mut settings, Edit::GroupLayerTarget(0, 0, Target::Pointed)));

        let (first, second) = (layers(&settings)[0], layers(&settings)[1]);
        assert!(apply(&mut settings, Edit::MoveGroupLayer { group: 0, index: 0, up: false }));
        assert_eq!(layers(&settings)[0], second);
        assert_eq!(layers(&settings)[1], first);
        assert!(!apply(&mut settings, Edit::MoveGroupLayer { group: 0, index: 0, up: true }));

        let count = layers(&settings).len();
        assert!(apply(&mut settings, Edit::AddGroupLayer(0)));
        assert_eq!(layers(&settings).len(), count + 1);
        assert!(apply(&mut settings, Edit::RemoveGroupLayer(0, count)));
        assert_eq!(layers(&settings).len(), count);
        assert!(!apply(&mut settings, Edit::RemoveGroupLayer(0, 99)));
    }

    #[test]
    fn a_group_changes_between_a_target_and_layers_without_losing_what_it_can_keep() {
        let mut settings = settings();
        // Several layers into one target: the first layer's target, and an Undo.
        let first_target = settings.controls.keys[0].scope.layers()[0].target;
        let edit = Edit::GroupScope(0, ScopeKind::Target);
        assert_eq!(
            edit.undo_label(&settings),
            Some("Replaced a group's layers with one target")
        );
        assert!(apply(&mut settings, edit));
        assert_eq!(settings.controls.keys[0].scope, Scope::Target(first_target));
        assert!(!apply(&mut settings, edit));

        // A target back into layers is one layer with no modifier: nothing lost,
        // so nothing to undo.
        let back = Edit::GroupScope(0, ScopeKind::Layers);
        assert_eq!(back.undo_label(&settings), None);
        assert!(apply(&mut settings, back));
        assert_eq!(
            settings.controls.keys[0].scope,
            Scope::Layers(vec![Layer { modifiers: 0, target: first_target }])
        );

        // The target of a group that is aimed by one.
        assert!(!apply(&mut settings, Edit::GroupTarget(0, Target::Mic)));
        apply(&mut settings, Edit::GroupScope(0, ScopeKind::Target));
        assert!(apply(&mut settings, Edit::GroupTarget(0, Target::Mic)));
        assert!(!apply(&mut settings, Edit::GroupTarget(0, Target::Mic)));
    }

    #[test]
    fn a_group_is_duplicated_with_its_layers_and_layers_can_be_copied() {
        let mut settings = settings();
        let groups = settings.controls.keys.len();
        assert!(apply(&mut settings, Edit::DuplicateGroup(0)));
        assert_eq!(settings.controls.keys.len(), groups + 1);
        assert_eq!(settings.controls.keys[0], settings.controls.keys[1]);

        apply(&mut settings, Edit::GroupScope(1, ScopeKind::Target));
        assert!(apply(&mut settings, Edit::CopyLayers { to: 1, from: 0 }));
        assert_eq!(settings.controls.keys[1].scope, settings.controls.keys[0].scope);
        assert!(!apply(&mut settings, Edit::CopyLayers { to: 1, from: 0 }));
    }

    #[test]
    fn a_hotkey_keeps_its_level_and_never_goes_above_100() {
        let mut settings = settings();
        assert!(apply(&mut settings, Edit::HotkeyAction(0, ActionKind::SetVolume)));
        assert!(apply(&mut settings, Edit::HotkeyLevel(0, 70)));
        assert_eq!(settings.hotkeys[0].action, Action::SetVolume(70));
        assert!(apply(&mut settings, Edit::HotkeyLevel(0, 250)));
        assert_eq!(settings.hotkeys[0].action, Action::SetVolume(100));
        // Choosing the same kind again is not an edit, level included.
        assert!(!apply(&mut settings, Edit::HotkeyAction(0, ActionKind::SetVolume)));
        // A level is only for the action that has one.
        assert!(apply(&mut settings, Edit::HotkeyAction(0, ActionKind::Mute)));
        assert!(!apply(&mut settings, Edit::HotkeyLevel(0, 20)));
        assert_eq!(ActionKind::of(settings.hotkeys[0].action), ActionKind::Mute);
    }

    #[test]
    fn the_tray_toggle_and_a_group_s_plain_are_not_edits_when_nothing_changes() {
        let mut settings = settings();
        let on = settings.replaces_windows_volume();
        assert!(!apply(&mut settings, Edit::ReplaceWindowsVolume(on)));
        assert!(apply(&mut settings, Edit::ReplaceWindowsVolume(!on)));
        assert_eq!(settings.replaces_windows_volume(), !on);
        let media = settings
            .controls
            .keys
            .iter()
            .position(|group| group.has_media_keys())
            .expect("a media group");
        let plain = settings.controls.keys[media].plain;
        assert!(!apply(&mut settings, Edit::Plain(media, plain)));
    }

    #[test]
    fn moving_and_removing_go_through_the_library_and_removing_has_an_undo_label() {
        let mut settings = settings();
        let count = settings.hotkeys.len();
        assert_eq!(Edit::Remove(List::Hotkeys, 0).undo_label(&settings), Some("Deleted a hotkey"));
        assert_eq!(Edit::Step(3).undo_label(&settings), None);
        assert!(apply(&mut settings, Edit::Remove(List::Hotkeys, 0)));
        assert_eq!(settings.hotkeys.len(), count - 1);
        assert!(!apply(&mut settings, Edit::Remove(List::Hotkeys, 99)));
    }

    #[test]
    fn a_binding_changes_between_a_wheel_and_a_button_with_what_each_needs() {
        let mut settings = settings();
        let wheel = settings
            .mouse
            .iter()
            .position(|binding| binding.input == Input::Wheel)
            .expect("the shipped file has a wheel binding");

        // To a button: it gets an action, and loses the wheel's direction.
        settings.mouse[wheel].invert = true;
        assert!(apply(&mut settings, Edit::MouseInput(wheel, Input::Middle)));
        assert_eq!(settings.mouse[wheel].action, Some(Action::Mute));
        assert!(!settings.mouse[wheel].invert);
        assert!(!apply(&mut settings, Edit::MouseInput(wheel, Input::Middle)));

        // Back to a wheel: no action.
        assert!(apply(&mut settings, Edit::MouseInput(wheel, Input::WheelTilt)));
        assert_eq!(settings.mouse[wheel].action, None);

        // A wheel has no action to choose, a button has no direction.
        assert!(!apply(&mut settings, Edit::MouseAction(wheel, ActionKind::Mixer)));
        assert!(apply(&mut settings, Edit::MouseInvert(wheel, true)));
        assert!(apply(&mut settings, Edit::MouseInput(wheel, Input::Left)));
        assert!(!apply(&mut settings, Edit::MouseInvert(wheel, true)));
    }

    #[test]
    fn only_the_side_buttons_may_go_without_a_modifier() {
        let mut settings = settings();
        assert!(apply(&mut settings, Edit::AddMouse(MouseKind::Wheel)));
        let wheel = settings.mouse.len() - 1;
        assert_eq!(settings.mouse[wheel].modifiers, MOD_ALT.0);
        // The last modifier of a wheel cannot be switched off...
        assert!(!apply(&mut settings, Edit::MouseModifier(wheel, MOD_ALT)));
        // ...another can be added first, and then Alt can go.
        assert!(apply(&mut settings, Edit::MouseModifier(wheel, MOD_CONTROL)));
        assert!(apply(&mut settings, Edit::MouseModifier(wheel, MOD_ALT)));
        assert_eq!(settings.mouse[wheel].modifiers, MOD_CONTROL.0);

        // A side button may be bare.
        assert!(apply(&mut settings, Edit::MouseInput(wheel, Input::X1)));
        assert!(apply(&mut settings, Edit::MouseModifier(wheel, MOD_CONTROL)));
        assert_eq!(settings.mouse[wheel].modifiers, 0);
        // And becoming a main button again puts a modifier back.
        assert!(apply(&mut settings, Edit::MouseInput(wheel, Input::Right)));
        assert_ne!(settings.mouse[wheel].modifiers, 0);
        assert!(settings.shadowed_mouse_bindings().len() <= 1);
    }

    #[test]
    fn the_mixer_takes_no_target_and_a_level_belongs_to_set_the_level() {
        let mut settings = settings();
        assert!(apply(&mut settings, Edit::AddMouse(MouseKind::Button)));
        let button = settings.mouse.len() - 1;
        assert!(apply(&mut settings, Edit::MouseTarget(button, Target::Mic)));
        assert!(!apply(&mut settings, Edit::MouseLevel(button, 30)));

        assert!(apply(&mut settings, Edit::MouseAction(button, ActionKind::SetVolume)));
        assert!(apply(&mut settings, Edit::MouseLevel(button, 30)));
        assert_eq!(settings.mouse[button].action, Some(Action::SetVolume(30)));
        assert!(!apply(&mut settings, Edit::MouseLevel(button, 30)));

        assert!(apply(&mut settings, Edit::MouseAction(button, ActionKind::Mixer)));
        assert_eq!(settings.mouse[button].action, Some(Action::Mixer));
        assert_eq!(settings.mouse[button].target, Target::Master);
    }

    #[test]
    fn what_was_edited_in_the_mouse_list_is_what_y_coustic_reads_back() {
        let mut settings = settings();
        apply(&mut settings, Edit::AddMouse(MouseKind::Button));
        let last = settings.mouse.len() - 1;
        apply(&mut settings, Edit::MouseInput(last, Input::X2));
        apply(&mut settings, Edit::MouseAction(last, ActionKind::SetVolume));
        apply(&mut settings, Edit::MouseLevel(last, 40));
        let again = Settings::parse(PathBuf::from("test.xml"), &settings.to_xml()).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.mouse, settings.mouse);
    }

    #[test]
    fn the_hud_settings_are_clamped_and_an_unchanged_value_is_not_an_edit() {
        let mut settings = settings();
        let edits = [
            Edit::HudStyle(Style::Ring),
            Edit::HudPosition(Position::TopRight),
            Edit::HudTimeout(3000),
            Edit::HudFade(200),
            Edit::HudScale(150),
            Edit::HudRadius(4),
            Edit::HudBorder(2),
            Edit::HudOpacity(80),
            Edit::HudInteractive(false),
        ];
        for edit in edits {
            assert!(apply(&mut settings, edit), "{edit:?} changed nothing");
            assert!(!apply(&mut settings, edit), "{edit:?} reported a change twice");
        }
        apply(&mut settings, Edit::HudTimeout(1));
        assert_eq!(settings.hud.timeout, *TIMEOUT_RANGE.start());
        apply(&mut settings, Edit::HudScale(9999));
        assert_eq!(settings.hud.scale, *SCALE_RANGE.end());
        apply(&mut settings, Edit::HudOpacity(0));
        assert_eq!(settings.hud.opacity, *OPACITY_RANGE.start());
        apply(&mut settings, Edit::HudBorder(99));
        assert_eq!(settings.hud.border, *BORDER_RANGE.end());
        apply(&mut settings, Edit::HudFade(99_999));
        assert_eq!(settings.hud.fade, *FADE_RANGE.end());
        apply(&mut settings, Edit::HudRadius(500));
        assert_eq!(settings.hud.radius, *RADIUS_RANGE.end());
    }

    #[test]
    fn the_font_is_trimmed_and_never_emptied() {
        let mut settings = settings();
        assert!(set_font(&mut settings, "  Cascadia Code "));
        assert_eq!(settings.hud.font, "Cascadia Code");
        assert!(!set_font(&mut settings, "Cascadia Code"));
        assert!(!set_font(&mut settings, "   "));
        assert_eq!(settings.hud.font, "Cascadia Code");
        let again = Settings::parse(PathBuf::from("test.xml"), &settings.to_xml()).unwrap();
        assert_eq!(again.hud, settings.hud);
    }

    #[test]
    fn a_control_s_float_becomes_a_whole_number() {
        assert_eq!(whole(7.4), 7);
        assert_eq!(whole(7.5), 8);
        assert_eq!(whole(-3.0), 0);
        assert_eq!(whole(f64::NAN), 0);
    }

    #[test]
    fn what_was_edited_is_what_y_coustic_reads_back() {
        let mut settings = settings();
        apply(&mut settings, Edit::Step(9));
        apply(&mut settings, Edit::TrayIcon(Rule::MicMuted));
        apply(&mut settings, Edit::MixerAnchor(Anchor::Hud));
        let again = Settings::parse(PathBuf::from("test.xml"), &settings.to_xml()).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.step, 9);
        assert_eq!(again.tray.icon, Rule::MicMuted);
        assert_eq!(again.mixer.anchor, Anchor::Hud);
        assert_eq!(again.kept.comment_count(), settings.kept.comment_count());
    }
}
