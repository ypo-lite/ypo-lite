//! `<controls>`: groups of keys, each aimed by a target or by layers of its own.
//!
//! A modifier chooses what a set of keys controls. One `<keys>` group
//! (`up`, `down`, `mute`) is aimed one of two ways: a `target` (the keys control
//! that, with no modifier held) or its own `<layer>`s (a modifier and a target
//! each, so the modifier held says what the keys control). It expands at load
//! into plain bindings. Nothing is shared between groups: a set of keys that
//! only handles the microphone is a group with `target="mic"`. Pure: no Win32
//! beyond the modifier constants, so the expansion runs in tests and the
//! settings app can share it.
//!
//! The groups stay in [`Settings`](crate::config::Settings) as written, and the
//! expansion is computed from them. Saving the settings writes the groups back,
//! not the bindings they expand to: a layer edited in a settings app must stay
//! one layer.
//!
//! Media keys (`VolumeUp`, `VolumeDown`, `VolumeMute`) are the one special
//! case: the keyboard hook is the only route that treats them the same on every
//! machine (see `Fallback::HookOnly`), and the bare key is Windows' own, so a
//! group's `plain` attribute says whether y-coustic takes it:
//!
//! | `plain=` | Bare volume keys | Modifier + volume key |
//! | --- | --- | --- |
//! | `windows` (default) | Windows, with its flyout | y-coustic layers |
//! | `replace` | y-coustic, with its HUD | y-coustic layers |

use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use y_core::hotkey::Hotkey;

use crate::config::Binding;
use crate::model::{Action, Target};

/// Whether a media group lets the bare keys through to Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Plain {
    /// The bare key is Windows': its volume, its flyout.
    #[default]
    Windows,
    /// The bare key is y-coustic's.
    Replace,
}

impl Plain {
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Replace => "replace",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "windows" => Some(Self::Windows),
            "replace" => Some(Self::Replace),
            _ => None,
        }
    }
}

/// What a group's keys control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// One thing, with no modifier held: `target="mic"`.
    Target(Target),
    /// What the modifier held says: the group's own `<layer>`s. With none, the
    /// group controls nothing (and is warned about).
    Layers(Vec<Layer>),
}

impl Default for Scope {
    fn default() -> Self {
        Self::Layers(Vec::new())
    }
}

impl Scope {
    /// The layers the scope stands for: a target is the one layer with no
    /// modifier.
    pub fn layers(&self) -> Vec<Layer> {
        match self {
            Self::Target(target) => vec![Layer {
                modifiers: 0,
                target: *target,
            }],
            Self::Layers(layers) => layers.clone(),
        }
    }
}

/// One `<keys>` element: up to three keys, each optional, and what they
/// control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysGroup {
    pub up: Option<Hotkey>,
    pub down: Option<Hotkey>,
    pub mute: Option<Hotkey>,
    /// Only meaningful for media keys.
    pub plain: Plain,
    pub scope: Scope,
}

impl KeysGroup {
    /// How a message names the group: its keys, as written.
    pub fn label(&self) -> String {
        let keys: Vec<String> = [self.up, self.down, self.mute]
            .into_iter()
            .flatten()
            .map(|key| {
                // Auto-repeat is not part of what a person wrote.
                let own = HOT_KEY_MODIFIERS(key.modifiers.0 & !MOD_NOREPEAT.0);
                Hotkey::new(own, key.key).to_string()
            })
            .collect();
        format!("the keys {}", keys.join(" / "))
    }

    /// The keys the group has, with the action each stands for.
    fn entries(&self) -> impl Iterator<Item = (Action, Hotkey)> {
        [
            (Action::VolumeUp, self.up),
            (Action::VolumeDown, self.down),
            (Action::Mute, self.mute),
        ]
        .into_iter()
        .filter_map(|(action, key)| key.map(|key| (action, key)))
    }

    /// Whether any key of the group is a media key, which is when `plain`
    /// has something to say.
    pub fn has_media_keys(&self) -> bool {
        self.entries().any(|(_, key)| is_media_key(key.key))
    }
}

/// One `<layer>`: held modifiers, and what the group's keys then control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer {
    /// `MOD_*` flags, without `MOD_NOREPEAT`; `0` for the bare keys.
    pub modifiers: u32,
    pub target: Target,
}

/// The `<controls>` section as written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Controls {
    pub keys: Vec<KeysGroup>,
}

impl Controls {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// `VolumeUp`, `VolumeDown` or `VolumeMute`.
pub fn is_media_key(key: VIRTUAL_KEY) -> bool {
    key == VK_VOLUME_UP || key == VK_VOLUME_DOWN || key == VK_VOLUME_MUTE
}

/// The modifiers of a `mod` attribute: `none`, or `Win`, `Ctrl`, `Alt`, `Shift`
/// joined by `+`, in any order and case.
pub fn parse_modifiers(text: &str) -> Option<u32> {
    let mut modifiers = 0;
    for part in text.split('+') {
        modifiers |= match part.trim().to_ascii_lowercase().as_str() {
            "none" if text.trim().eq_ignore_ascii_case("none") => 0,
            "win" | "windows" => MOD_WIN.0,
            "ctrl" | "control" => MOD_CONTROL.0,
            "alt" => MOD_ALT.0,
            "shift" => MOD_SHIFT.0,
            _ => return None,
        };
    }
    Some(modifiers)
}

/// The spelling [`parse_modifiers`] reads back: `none`, or `Win+Ctrl+Alt+Shift`
/// in that order, the same as a hotkey's own.
pub fn format_modifiers(modifiers: u32) -> String {
    let mut text = String::new();
    for (flag, name) in [
        (MOD_WIN, "Win"),
        (MOD_CONTROL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
    ] {
        if modifiers & flag.0 != 0 {
            if !text.is_empty() {
                text.push('+');
            }
            text.push_str(name);
        }
    }
    if text.is_empty() {
        "none".to_owned()
    } else {
        text
    }
}

/// What a binding is keyed on: the key and its modifiers, not auto-repeat.
fn identity(hotkey: Hotkey) -> (u16, u32) {
    (hotkey.key.0, hotkey.modifiers.0 & !MOD_NOREPEAT.0)
}

/// Where a binding came from: a single `<hotkey>`, or a layer of a group (a
/// group aimed by a target has its one layer, `0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Hotkey(usize),
    Layer { group: usize, layer: usize },
}

/// What a problem is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The n-th single hotkey.
    Hotkey(usize),
    /// The n-th group as a whole: it controls nothing.
    Group(usize),
    /// A layer of a group: it uses `Alt`, or one of its keys is used twice.
    Layer { group: usize, layer: usize },
}

/// A warning, and the row of an editor it belongs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub place: Place,
    pub message: String,
}

/// Everything [`expand`] works out: the bindings with where each came from, and
/// the problems with where each belongs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    pub bindings: Vec<(Binding, Origin)>,
    pub problems: Vec<Problem>,
}

/// The bindings to register: the single `<hotkey>`s, then every group crossed
/// with its own layers, and the warnings the crossing produced.
///
/// * A media key's binding goes through the hook only (`hook_only`).
/// * With `plain="windows"`, the bare media keys are left out: they stay
///   Windows'.
/// * A layer holding `Alt` is kept but warned about: swallowing `Alt+key` leaves
///   the system seeing a lone `Alt` tap, which moves focus to a menu bar.
/// * A combination used twice keeps the first use and warns about the second.
pub fn expand(hotkeys: &[Binding], controls: &Controls) -> (Vec<Binding>, Vec<String>) {
    let expansion = expand_origins(hotkeys, controls);
    (
        expansion
            .bindings
            .into_iter()
            .map(|(binding, _)| binding)
            .collect(),
        expansion
            .problems
            .into_iter()
            .map(|problem| problem.message)
            .collect(),
    )
}

/// [`expand`], with where each binding and each warning comes from: what a
/// settings app needs to put a marker on the right row.
pub fn expand_origins(hotkeys: &[Binding], controls: &Controls) -> Expansion {
    let mut bindings: Vec<(Binding, Origin)> = Vec::new();
    let mut problems = Vec::new();

    let mut push = |binding: Binding, origin: Origin, label: &str, problems: &mut Vec<Problem>| {
        if bindings
            .iter()
            .any(|(known, _)| identity(known.hotkey) == identity(binding.hotkey))
        {
            let (place, from) = match origin {
                Origin::Hotkey(index) => (Place::Hotkey(index), "<hotkeys>"),
                Origin::Layer { group, layer } => (Place::Layer { group, layer }, label),
            };
            problems.push(Problem {
                place,
                message: format!(
                    "{} is used twice: the use in {from} is ignored",
                    binding.hotkey
                ),
            });
        } else {
            bindings.push((binding, origin));
        }
    };

    for (index, binding) in hotkeys.iter().enumerate() {
        push(*binding, Origin::Hotkey(index), "<hotkeys>", &mut problems);
    }

    for (group_index, group) in controls.keys.iter().enumerate() {
        let label = group.label();
        let layers = group.scope.layers();
        if layers.is_empty() {
            problems.push(Problem {
                place: Place::Group(group_index),
                message: format!("{label} controls nothing: give it a target or a <layer>"),
            });
        }
        for (layer_index, layer) in layers.iter().enumerate() {
            if layer.modifiers & MOD_ALT.0 != 0 {
                problems.push(Problem {
                    place: Place::Layer {
                        group: group_index,
                        layer: layer_index,
                    },
                    message: format!(
                        "{label}: the layer {} uses Alt: releasing Alt alone afterwards \
                         focuses the menu bar of the window in front",
                        format_modifiers(layer.modifiers)
                    ),
                });
            }
        }
        for (layer_index, layer) in layers.iter().enumerate() {
            for (action, key) in group.entries() {
                let media = is_media_key(key.key);
                let own = key.modifiers.0 & !MOD_NOREPEAT.0;
                let bare = own == 0 && layer.modifiers == 0;
                if media && bare && group.plain == Plain::Windows {
                    continue; // the bare key stays Windows'
                }

                let combined = Hotkey::new(
                    HOT_KEY_MODIFIERS(key.modifiers.0 | layer.modifiers),
                    key.key,
                );
                push(
                    Binding {
                        hotkey: combined,
                        action,
                        target: layer.target,
                        hook_only: media,
                    },
                    Origin::Layer {
                        group: group_index,
                        layer: layer_index,
                    },
                    &label,
                    &mut problems,
                );
            }
        }
    }

    Expansion { bindings, problems }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Option<Hotkey> {
        Some(Hotkey::parse(text).expect("valid"))
    }

    fn media_group(plain: Plain, scope: Scope) -> KeysGroup {
        KeysGroup {
            up: key("VolumeUp"),
            down: key("VolumeDown"),
            mute: key("VolumeMute"),
            plain,
            scope,
        }
    }

    fn numpad_group(scope: Scope) -> KeysGroup {
        KeysGroup {
            up: key("Win+NumAdd"),
            down: key("Win+NumSub"),
            mute: key("Win+Numpad0"),
            plain: Plain::Windows,
            scope,
        }
    }

    fn layer(modifiers: u32, target: Target) -> Layer {
        Layer { modifiers, target }
    }

    /// The bare keys on the master, Shift on the microphone.
    fn layers() -> Scope {
        Scope::Layers(vec![
            layer(0, Target::Master),
            layer(MOD_SHIFT.0, Target::Mic),
        ])
    }

    fn controls(keys: Vec<KeysGroup>) -> Controls {
        Controls { keys }
    }

    fn texts(bindings: &[Binding]) -> Vec<String> {
        bindings
            .iter()
            .map(|b| format!("{} {} {}", b.hotkey, b.action.name(), b.target.name()))
            .collect()
    }

    #[test]
    fn plain_windows_leaves_the_bare_media_keys_alone() {
        let controls = controls(vec![media_group(Plain::Windows, layers())]);
        let (bindings, warnings) = expand(&[], &controls);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            texts(&bindings),
            [
                "Shift+VolumeUp volume-up mic",
                "Shift+VolumeDown volume-down mic",
                "Shift+VolumeMute mute mic",
            ]
        );
    }

    #[test]
    fn plain_replace_takes_the_bare_media_keys() {
        let controls = controls(vec![media_group(Plain::Replace, layers())]);
        let (bindings, warnings) = expand(&[], &controls);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(bindings.len(), 6);
        assert!(texts(&bindings).contains(&"VolumeUp volume-up master".to_owned()));
        assert!(texts(&bindings).contains(&"Shift+VolumeMute mute mic".to_owned()));
    }

    #[test]
    fn media_keys_use_the_hook_only_and_the_others_do_not() {
        let controls = controls(vec![
            media_group(Plain::Replace, layers()),
            numpad_group(layers()),
        ]);
        let (bindings, _) = expand(&[], &controls);
        assert_eq!(bindings.len(), 12);
        for binding in &bindings {
            let media = is_media_key(binding.hotkey.key);
            assert_eq!(binding.hook_only, media, "{}", binding.hotkey);
        }
    }

    #[test]
    fn a_group_that_is_not_media_gets_every_one_of_its_layers() {
        // `plain` has nothing to say about it: its bare keys already carry Win.
        let controls = controls(vec![numpad_group(layers())]);
        let (bindings, warnings) = expand(&[], &controls);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            texts(&bindings),
            [
                "Win+NumAdd volume-up master",
                "Win+NumSub volume-down master",
                "Win+Numpad0 mute master",
                "Win+Shift+NumAdd volume-up mic",
                "Win+Shift+NumSub volume-down mic",
                "Win+Shift+Numpad0 mute mic",
            ]
        );
    }

    #[test]
    fn a_group_with_a_target_controls_that_and_nothing_else() {
        let mute = KeysGroup {
            up: None,
            down: None,
            mute: key("Win+Alt+M"),
            plain: Plain::Windows,
            scope: Scope::Target(Target::Mic),
        };
        let focused = KeysGroup {
            up: key("Win+Alt+PageUp"),
            down: key("Win+Alt+PageDown"),
            mute: None,
            plain: Plain::Windows,
            scope: Scope::Target(Target::Focused),
        };
        let (bindings, warnings) = expand(&[], &controls(vec![mute, focused]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            texts(&bindings),
            [
                "Win+Alt+M mute mic",
                "Win+Alt+PageUp volume-up focused",
                "Win+Alt+PageDown volume-down focused",
            ]
        );
    }

    #[test]
    fn groups_do_not_share_layers() {
        // The media group has three layers, the microphone's mute has none of
        // them: the keys of one are not crossed with the layers of the other.
        let media = media_group(
            Plain::Replace,
            Scope::Layers(vec![
                layer(0, Target::Master),
                layer(MOD_SHIFT.0, Target::Mic),
                layer(MOD_CONTROL.0, Target::Focused),
            ]),
        );
        let mute = KeysGroup {
            up: None,
            down: None,
            mute: key("Win+Alt+M"),
            plain: Plain::Windows,
            scope: Scope::Target(Target::Mic),
        };
        let (bindings, warnings) = expand(&[], &controls(vec![media, mute]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(bindings.len(), 9 + 1);
        assert_eq!(
            bindings
                .iter()
                .filter(|b| b.hotkey.to_string() == "Win+Alt+M")
                .count(),
            1
        );
    }

    #[test]
    fn a_target_is_the_one_layer_with_no_modifier() {
        assert_eq!(
            Scope::Target(Target::Mic).layers(),
            vec![layer(0, Target::Mic)]
        );
        assert!(Scope::default().layers().is_empty());
    }

    #[test]
    fn a_group_with_only_some_keys_expands_only_those() {
        let group = KeysGroup {
            up: key("F11"),
            down: None,
            mute: key("F10"),
            plain: Plain::Windows,
            scope: Scope::Layers(vec![layer(MOD_CONTROL.0, Target::Master)]),
        };
        let (bindings, _) = expand(&[], &controls(vec![group]));
        assert_eq!(
            texts(&bindings),
            ["Ctrl+F11 volume-up master", "Ctrl+F10 mute master"]
        );
    }

    #[test]
    fn the_original_modifiers_survive_the_expansion() {
        let controls = controls(vec![numpad_group(Scope::Layers(vec![layer(
            MOD_CONTROL.0 | MOD_SHIFT.0,
            Target::Mic,
        )]))]);
        let (bindings, _) = expand(&[], &controls);
        assert_eq!(bindings[0].hotkey.to_string(), "Win+Ctrl+Shift+NumAdd");
        // `Hotkey::parse` set no-repeat; the expansion keeps it.
        assert!(bindings[0].hotkey.modifiers.0 & MOD_NOREPEAT.0 != 0);
    }

    #[test]
    fn a_collision_keeps_the_first_use_and_names_the_group_of_the_second() {
        let single = Binding {
            hotkey: Hotkey::parse("Shift+VolumeUp").unwrap(),
            action: Action::Mute,
            target: Target::Master,
            hook_only: true,
        };
        let controls = controls(vec![media_group(Plain::Windows, layers())]);
        let (bindings, warnings) = expand(&[single], &controls);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("Shift+VolumeUp"));
        assert!(warnings[0].contains("the keys VolumeUp / VolumeDown / VolumeMute"));
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].action, Action::Mute, "the single one wins");
    }

    #[test]
    fn two_singles_on_one_key_collide_too() {
        let one = Binding {
            hotkey: Hotkey::parse("Win+F1").unwrap(),
            action: Action::Mute,
            target: Target::Master,
            hook_only: false,
        };
        let (bindings, warnings) = expand(&[one, one], &Controls::default());
        assert_eq!(bindings.len(), 1);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn two_groups_on_one_key_collide_and_the_first_wins() {
        let first = numpad_group(Scope::Target(Target::Master));
        let second = numpad_group(Scope::Target(Target::Mic));
        let (bindings, warnings) = expand(&[], &controls(vec![first, second]));
        assert_eq!(bindings.len(), 3);
        assert!(bindings.iter().all(|b| b.target == Target::Master));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn an_alt_layer_is_kept_and_warned_about_with_its_group() {
        let controls = controls(vec![numpad_group(Scope::Layers(vec![layer(
            MOD_ALT.0,
            Target::Master,
        )]))]);
        let (bindings, warnings) = expand(&[], &controls);
        assert_eq!(bindings.len(), 3);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Alt"));
        assert!(warnings[0].contains("Win+NumAdd"));
    }

    #[test]
    fn a_layer_on_an_application_is_expanded_without_a_warning() {
        let scope = Scope::Layers(vec![
            layer(MOD_CONTROL.0, Target::Focused),
            layer(MOD_SHIFT.0, Target::Mic),
        ]);
        let controls = controls(vec![
            media_group(Plain::Windows, scope.clone()),
            numpad_group(scope),
        ]);
        let (bindings, warnings) = expand(&[], &controls);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            bindings
                .iter()
                .filter(|b| b.target == Target::Focused)
                .count(),
            6
        );
        assert_eq!(
            bindings.iter().filter(|b| b.target == Target::Mic).count(),
            6
        );
        assert_eq!(bindings.len(), 12);
    }

    #[test]
    fn every_binding_and_every_problem_knows_where_it_comes_from() {
        let single = Binding {
            hotkey: Hotkey::parse("Shift+VolumeUp").unwrap(),
            action: Action::Mute,
            target: Target::Master,
            hook_only: true,
        };
        let alt = KeysGroup {
            up: key("F1"),
            down: None,
            mute: None,
            plain: Plain::Windows,
            scope: Scope::Layers(vec![layer(MOD_ALT.0, Target::Master)]),
        };
        let nothing = media_group(Plain::Windows, Scope::default());
        let media = media_group(Plain::Windows, layers());
        let expansion = expand_origins(&[single], &controls(vec![alt, nothing, media]));

        // The single hotkey comes first, then the layers of the groups.
        assert_eq!(expansion.bindings[0].1, Origin::Hotkey(0));
        assert!(
            expansion
                .bindings
                .iter()
                .any(|(_, origin)| *origin == Origin::Layer { group: 0, layer: 0 })
        );

        let places: Vec<Place> = expansion.problems.iter().map(|p| p.place).collect();
        // Alt on the first group's only layer; the second group controls nothing;
        // the third group's Shift+VolumeUp is the single hotkey's.
        assert!(
            places.contains(&Place::Layer { group: 0, layer: 0 }),
            "{places:?}"
        );
        assert!(places.contains(&Place::Group(1)), "{places:?}");
        assert!(
            places.contains(&Place::Layer { group: 2, layer: 1 }),
            "{places:?}"
        );
        assert_eq!(places.len(), 3);

        // `expand` says the same in words.
        let (bindings, warnings) = expand(&[single], &controls(vec![]));
        assert_eq!(bindings.len(), 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_group_with_nothing_to_control_expands_to_nothing_and_says_so() {
        let (bindings, warnings) = expand(
            &[],
            &controls(vec![media_group(Plain::Replace, Scope::default())]),
        );
        assert!(bindings.is_empty());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("controls nothing"));

        let (bindings, warnings) = expand(&[], &Controls::default());
        assert!(bindings.is_empty() && warnings.is_empty());
    }

    #[test]
    fn modifiers_parse_and_format() {
        assert_eq!(parse_modifiers("none"), Some(0));
        assert_eq!(parse_modifiers("NONE"), Some(0));
        assert_eq!(
            parse_modifiers("ctrl + shift"),
            Some(MOD_CONTROL.0 | MOD_SHIFT.0)
        );
        assert_eq!(parse_modifiers("Win+Alt"), Some(MOD_WIN.0 | MOD_ALT.0));
        assert_eq!(parse_modifiers("Ctrl+none"), None);
        assert_eq!(parse_modifiers("Hyper"), None);
        assert_eq!(parse_modifiers(""), None);

        for text in ["none", "Win", "Ctrl+Shift", "Win+Ctrl+Alt+Shift"] {
            assert_eq!(format_modifiers(parse_modifiers(text).unwrap()), text);
        }
    }

    #[test]
    fn plain_names_round_trip() {
        for plain in [Plain::Windows, Plain::Replace] {
            assert_eq!(Plain::from_name(plain.name()), Some(plain));
        }
        assert_eq!(Plain::from_name("both"), None);
    }

    #[test]
    fn the_section_writes_back_what_it_read() {
        let controls = controls(vec![
            media_group(Plain::Replace, layers()),
            numpad_group(Scope::Target(Target::Master)),
        ]);
        let mut settings = crate::config::Settings::empty(std::path::PathBuf::from("test.xml"));
        settings.controls = controls;
        let out = settings.to_xml();
        assert!(out.contains(
            r#"<keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="replace">"#
        ));
        assert!(out.contains(
            r#"<keys up="Win+NumAdd" down="Win+NumSub" mute="Win+Numpad0" target="master"/>"#
        ));
        assert!(out.contains(r#"<layer mod="Shift" target="mic"/>"#));
        assert!(out.contains(r#"<layer mod="none" target="master"/>"#));
    }
}
