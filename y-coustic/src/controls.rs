//! `<controls>`: groups of keys combined with layers of modifiers.
//!
//! A modifier chooses what a set of keys controls. One `<keys>` group
//! (`up`, `down`, `mute`) plus one `<layer>` list (a modifier and a target)
//! expands at load into plain bindings, so the same layers work on media keys
//! and on any other three keys. Pure: no Win32 beyond the modifier constants,
//! so the expansion runs in tests and the settings app can share it.
//!
//! The groups and layers stay in [`Settings`](crate::config::Settings) as
//! written, and the expansion is computed from them. Saving the settings writes
//! the groups back, not the bindings they expand to: a layer edited in a
//! settings app must stay one layer.
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

use std::fmt::Write as _;

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

/// One `<keys>` element: up to three keys, each optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeysGroup {
    pub up: Option<Hotkey>,
    pub down: Option<Hotkey>,
    pub mute: Option<Hotkey>,
    /// Only meaningful for media keys.
    pub plain: Plain,
}

impl KeysGroup {
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
    pub layers: Vec<Layer>,
}

impl Controls {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.layers.is_empty()
    }

    /// Writes the section back in the shape the reader takes.
    pub fn write_xml(&self, out: &mut String) {
        out.push_str("\t<controls>\n");
        for group in &self.keys {
            out.push_str("\t\t<keys");
            for (name, key) in [("up", group.up), ("down", group.down), ("mute", group.mute)] {
                if let Some(key) = key {
                    let _ = write!(out, " {name}=\"{key}\"");
                }
            }
            if group.has_media_keys() {
                let _ = write!(out, " plain=\"{}\"", group.plain.name());
            }
            out.push_str("/>\n");
        }
        if !self.keys.is_empty() && !self.layers.is_empty() {
            out.push('\n');
        }
        for layer in &self.layers {
            let _ = writeln!(
                out,
                "\t\t<layer mod=\"{}\" target=\"{}\"/>",
                format_modifiers(layer.modifiers),
                layer.target.name()
            );
        }
        out.push_str("\t</controls>\n\n");
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

/// The bindings to register: the single `<hotkey>`s, then every group crossed
/// with every layer, and the warnings the crossing produced.
///
/// * A media key's binding goes through the hook only (`hook_only`).
/// * With `plain="windows"`, the bare media keys are left out: they stay
///   Windows'.
/// * A layer holding `Alt` is kept but warned about: swallowing `Alt+key` leaves
///   the system seeing a lone `Alt` tap, which moves focus to a menu bar.
/// * A combination used twice keeps the first use and warns about the second.
pub fn expand(hotkeys: &[Binding], controls: &Controls) -> (Vec<Binding>, Vec<String>) {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut warnings = Vec::new();

    let mut push = |binding: Binding, origin: &str, warnings: &mut Vec<String>| {
        if bindings
            .iter()
            .any(|known| identity(known.hotkey) == identity(binding.hotkey))
        {
            warnings.push(format!(
                "{} is used twice: the use in {origin} is ignored",
                binding.hotkey
            ));
        } else {
            bindings.push(binding);
        }
    };

    for binding in hotkeys {
        push(*binding, "<hotkeys>", &mut warnings);
    }

    for layer in &controls.layers {
        if layer.modifiers & MOD_ALT.0 != 0 {
            warnings.push(format!(
                "<controls>: the layer {} uses Alt: releasing Alt alone afterwards \
                 focuses the menu bar of the window in front",
                format_modifiers(layer.modifiers)
            ));
        }
    }

    for group in &controls.keys {
        for layer in &controls.layers {
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
                    "<controls>",
                    &mut warnings,
                );
            }
        }
    }

    (bindings, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> Option<Hotkey> {
        Some(Hotkey::parse(text).expect("valid"))
    }

    fn media_group(plain: Plain) -> KeysGroup {
        KeysGroup {
            up: key("VolumeUp"),
            down: key("VolumeDown"),
            mute: key("VolumeMute"),
            plain,
        }
    }

    fn numpad_group() -> KeysGroup {
        KeysGroup {
            up: key("Win+NumAdd"),
            down: key("Win+NumSub"),
            mute: key("Win+Numpad0"),
            plain: Plain::Windows,
        }
    }

    fn layers() -> Vec<Layer> {
        vec![
            Layer {
                modifiers: 0,
                target: Target::Master,
            },
            Layer {
                modifiers: MOD_SHIFT.0,
                target: Target::Mic,
            },
        ]
    }

    fn texts(bindings: &[Binding]) -> Vec<String> {
        bindings
            .iter()
            .map(|b| format!("{} {} {}", b.hotkey, b.action.name(), b.target.name()))
            .collect()
    }

    #[test]
    fn plain_windows_leaves_the_bare_media_keys_alone() {
        let controls = Controls {
            keys: vec![media_group(Plain::Windows)],
            layers: layers(),
        };
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
        let controls = Controls {
            keys: vec![media_group(Plain::Replace)],
            layers: layers(),
        };
        let (bindings, warnings) = expand(&[], &controls);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(bindings.len(), 6);
        assert!(texts(&bindings).contains(&"VolumeUp volume-up master".to_owned()));
        assert!(texts(&bindings).contains(&"Shift+VolumeMute mute mic".to_owned()));
    }

    #[test]
    fn media_keys_use_the_hook_only_and_the_others_do_not() {
        let controls = Controls {
            keys: vec![media_group(Plain::Replace), numpad_group()],
            layers: layers(),
        };
        let (bindings, _) = expand(&[], &controls);
        assert_eq!(bindings.len(), 12);
        for binding in &bindings {
            let media = is_media_key(binding.hotkey.key);
            assert_eq!(binding.hook_only, media, "{}", binding.hotkey);
        }
    }

    #[test]
    fn a_group_that_is_not_media_gets_every_layer() {
        // `plain` has nothing to say about it: its bare keys already carry Win.
        let controls = Controls {
            keys: vec![numpad_group()],
            layers: layers(),
        };
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
    fn a_group_with_only_some_keys_expands_only_those() {
        let group = KeysGroup {
            up: key("F11"),
            down: None,
            mute: key("F10"),
            plain: Plain::Windows,
        };
        let controls = Controls {
            keys: vec![group],
            layers: vec![Layer {
                modifiers: MOD_CONTROL.0,
                target: Target::Master,
            }],
        };
        let (bindings, _) = expand(&[], &controls);
        assert_eq!(
            texts(&bindings),
            ["Ctrl+F11 volume-up master", "Ctrl+F10 mute master"]
        );
    }

    #[test]
    fn the_original_modifiers_survive_the_expansion() {
        let controls = Controls {
            keys: vec![numpad_group()],
            layers: vec![Layer {
                modifiers: MOD_CONTROL.0 | MOD_SHIFT.0,
                target: Target::Mic,
            }],
        };
        let (bindings, _) = expand(&[], &controls);
        assert_eq!(bindings[0].hotkey.to_string(), "Win+Ctrl+Shift+NumAdd");
        // `Hotkey::parse` set no-repeat; the expansion keeps it.
        assert!(bindings[0].hotkey.modifiers.0 & MOD_NOREPEAT.0 != 0);
    }

    #[test]
    fn a_collision_keeps_the_first_use_and_warns_about_the_second() {
        let single = Binding {
            hotkey: Hotkey::parse("Shift+VolumeUp").unwrap(),
            action: Action::Mute,
            target: Target::Master,
            hook_only: true,
        };
        let controls = Controls {
            keys: vec![media_group(Plain::Windows)],
            layers: layers(),
        };
        let (bindings, warnings) = expand(&[single], &controls);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("Shift+VolumeUp"));
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
    fn an_alt_layer_is_kept_and_warned_about() {
        let controls = Controls {
            keys: vec![numpad_group()],
            layers: vec![Layer {
                modifiers: MOD_ALT.0,
                target: Target::Master,
            }],
        };
        let (bindings, warnings) = expand(&[], &controls);
        assert_eq!(bindings.len(), 3);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Alt"));
    }

    #[test]
    fn a_layer_on_an_application_is_expanded_without_a_warning() {
        let controls = Controls {
            keys: vec![media_group(Plain::Windows), numpad_group()],
            layers: vec![
                Layer {
                    modifiers: MOD_CONTROL.0,
                    target: Target::Focused,
                },
                Layer {
                    modifiers: MOD_SHIFT.0,
                    target: Target::Mic,
                },
            ],
        };
        let (bindings, warnings) = expand(&[], &controls);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(bindings.iter().filter(|b| b.target == Target::Focused).count(), 6);
        assert_eq!(bindings.iter().filter(|b| b.target == Target::Mic).count(), 6);
        assert_eq!(bindings.len(), 12);
    }

    #[test]
    fn no_groups_or_no_layers_expand_to_nothing() {
        let layers = layers();
        let (bindings, _) = expand(
            &[],
            &Controls {
                keys: vec![],
                layers: layers.clone(),
            },
        );
        assert!(bindings.is_empty());
        let (bindings, _) = expand(
            &[],
            &Controls {
                keys: vec![media_group(Plain::Replace)],
                layers: vec![],
            },
        );
        assert!(bindings.is_empty());
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
        let controls = Controls {
            keys: vec![media_group(Plain::Replace), numpad_group()],
            layers: layers(),
        };
        let mut out = String::new();
        controls.write_xml(&mut out);
        assert!(out.contains(
            r#"<keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="replace"/>"#
        ));
        assert!(out.contains(r#"<keys up="Win+NumAdd" down="Win+NumSub" mute="Win+Numpad0"/>"#));
        assert!(out.contains(r#"<layer mod="Shift" target="mic"/>"#));
        assert!(out.contains(r#"<layer mod="none" target="master"/>"#));
    }
}
