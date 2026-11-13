//! The three small screens: General, Tray icon and Mixer. Each is a function of
//! the settings and a `Viewport`; every control reports an [`Edit`], which
//! `ConfigApp::update` applies.

use std::path::Path;

use windows_reactor::*;
use y_coustic::appearance::Rule;
use y_coustic::config::{
    Anchor, HOVER_DELAY_RANGE, MAX_ROWS_RANGE, STEP_RANGE, Settings, TrayAction, WatchMode,
};

use crate::edit::{Click, Edit, Endpoint, whole};
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};

/// A control that reports a float: its whole number, as an edit.
pub(crate) fn number(
    context: &ViewContext<ConfigApp>,
    make: fn(u32) -> Edit,
) -> (Callback<f64>, Callback<Option<f64>>) {
    (
        context.callback(move |value: f64| Msg::Edit(make(whole(value)))),
        context.callback(move |value: Option<f64>| match value {
            Some(value) => Msg::Edit(make(whole(value))),
            None => Msg::Ignored,
        }),
    )
}

/// A combo box over `options`: the chosen one as an edit.
pub(crate) fn choose<T: Copy + 'static>(
    context: &ViewContext<ConfigApp>,
    options: &'static [(T, &'static str)],
    make: impl Fn(T) -> Edit + 'static,
) -> Callback<Option<usize>> {
    context.callback(
        move |index: Option<usize>| match index.and_then(|index| options.get(index)) {
            Some((value, _)) => Msg::Edit(make(*value)),
            None => Msg::Ignored,
        },
    )
}

pub(crate) fn labels<T>(options: &[(T, &'static str)]) -> Vec<&'static str> {
    options.iter().map(|(_, label)| *label).collect()
}

pub(crate) fn position<T: PartialEq>(options: &[(T, &'static str)], value: T) -> Option<usize> {
    options.iter().position(|(option, _)| *option == value)
}

const WATCH: &[(WatchMode, &str)] = &[
    (WatchMode::Off, "Off"),
    (WatchMode::Tray, "Tray icon"),
    (WatchMode::Hud, "HUD"),
];

/// A watch mode as a short row of choices. The master has no icon variant to
/// keep true, so *Tray icon* would do nothing there.
fn watch_choice(context: &ViewContext<ConfigApp>, endpoint: Endpoint, current: WatchMode) -> View {
    let choices: Vec<Choice<'_>> = WATCH
        .iter()
        .map(|(mode, label)| {
            if endpoint == Endpoint::Master && *mode == WatchMode::Tray {
                Choice::disabled(
                    label,
                    "The master volume has no icon variant, so this would do nothing",
                )
            } else {
                Choice::new(label)
            }
        })
        .collect();
    segmented(
        &choices,
        position(WATCH, current),
        context.callback(move |index: usize| match WATCH.get(index) {
            Some((mode, _)) => Msg::Edit(Edit::Watch(endpoint, *mode)),
            None => Msg::Ignored,
        }),
    )
}

pub fn general_view(
    settings: &Settings,
    path: &Path,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let (slide, type_in) = number(context, Edit::Step);
    let step = setting_wide(
        "Volume step",
        "How many percent one press of a volume key, or one notch of the wheel, moves the level.",
        range_input(
            f64::from(settings.step),
            f64::from(*STEP_RANGE.start()),
            f64::from(*STEP_RANGE.end()),
            1.0,
            slide,
            type_in,
        ),
    );

    let watching = Stack::vertical(space::M)
        .child(
            "master",
            setting(
                "Speakers and headphones",
                "When something other than y-coustic changes the master volume. Off by default: Windows already shows \
                 its own flyout.",
                watch_choice(context, Endpoint::Master, settings.watch.master),
                viewport,
            ),
        )
        .child(
            "microphone",
            setting(
                "Microphone",
                "When something other than y-coustic changes the microphone: the Sound panel, a laptop's microphone \
                 key. On by default, because believing you are muted when you are not costs the most.",
                watch_choice(context, Endpoint::Microphone, settings.watch.microphone),
                viewport,
            ),
        )
        .child(
            "note",
            note(
                "HUD shows the level on the HUD. Tray icon keeps the tray icon true to its rule (see Tray icon). Off \
                 watches nothing, and nothing stays resident for it.",
            ),
        );

    let file = setting(
        "Configuration file",
        &path.display().to_string(),
        Stack::horizontal(space::S)
            .child(
                "folder",
                button(
                    "Open folder",
                    false,
                    path.parent().is_some_and(Path::is_dir),
                    context.callback(|()| Msg::OpenConfigFolder),
                ),
            )
            .child(
                "edit",
                button(
                    "Edit XML…",
                    false,
                    path.is_file(),
                    context.callback(|()| Msg::EditConfigXml),
                ),
            ),
        viewport,
    );

    page(
        "General",
        "The volume step, what to do when something else changes the volume, and the file.",
        Stack::vertical(space::XL)
            .child("step", section("Volume", step))
            .child(
                "watching",
                section("When something else changes the volume", watching),
            )
            .child("file", section("File", file)),
    )
}

const ACTIONS: &[(TrayAction, &str)] = &[
    (TrayAction::None, "Nothing"),
    (TrayAction::Mixer, "Open or close the mixer"),
    (TrayAction::Hud, "Show the master level"),
    (TrayAction::Mute, "Mute or unmute the master"),
    (TrayAction::MuteMic, "Mute or unmute the microphone"),
    (TrayAction::VolumeUp, "Master volume up one step"),
    (TrayAction::VolumeDown, "Master volume down one step"),
];

const ICON_RULES: &[(Rule, &str)] = &[
    (Rule::Fixed, "Never changes"),
    (Rule::MicMuted, "Shows a muted microphone"),
];

pub fn tray_view(
    settings: &Settings,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let tray = &settings.tray;
    let hover_on = tray.hover_delay > 0;

    let (slide, type_in) = number(context, Edit::HoverDelay);
    let hover = setting_wide(
        "Rest the pointer on the icon",
        "Milliseconds before the HUD shows the master level next to the icon. 0 turns it off and gives Windows' own \
         tooltip back. The icon must be on the taskbar, not in the overflow flyout: Windows does not report the \
         pointer on an icon there.",
        range_input(
            f64::from(tray.hover_delay),
            f64::from(*HOVER_DELAY_RANGE.start()),
            f64::from(*HOVER_DELAY_RANGE.end()),
            50.0,
            slide,
            type_in,
        ),
    );

    let click = |title: &str, description: &str, which: Click, current: TrayAction| {
        setting(
            title,
            description,
            combo(
                labels(ACTIONS),
                position(ACTIONS, current),
                choose(context, ACTIONS, move |action| {
                    Edit::TrayClick(which, action)
                }),
            ),
            viewport,
        )
    };
    let mut clicks = Stack::vertical(space::M)
        .child(
            "single",
            click(
                "Click",
                "A left click on the icon.",
                Click::Single,
                tray.click,
            ),
        )
        .child(
            "double",
            click(
                "Double-click",
                "Leave it at Nothing to keep the single click immediate.",
                Click::Double,
                tray.double_click,
            ),
        )
        .child(
            "middle",
            click(
                "Middle click",
                "A click with the wheel button.",
                Click::Middle,
                tray.middle_click,
            ),
        );
    if tray.double_click != TrayAction::None {
        clicks = clicks.child(
            "wait",
            note(
                "With a double-click action, the single click waits for the time Windows allows between the two clicks of \
                 a double-click before it acts.",
            ),
        );
    }

    let wheel = setting(
        "Wheel over the icon",
        if hover_on {
            "Turn the wheel over the icon to step the master volume."
        } else {
            "Needs the rest-the-pointer setting above: Windows only reports the pointer on an icon without a tooltip."
        },
        switch_enabled(
            tray.wheel,
            hover_on,
            context.callback(|on: bool| Msg::Edit(Edit::TrayWheel(on))),
        ),
        viewport,
    );

    let icon = setting(
        "The icon itself",
        "Whether the tray icon changes with the state of the devices. More rules, such as one by volume level, may \
         come.",
        segmented(
            &ICON_RULES
                .iter()
                .map(|(_, label)| Choice::new(label))
                .collect::<Vec<_>>(),
            position(ICON_RULES, tray.icon),
            context.callback(|index: usize| match ICON_RULES.get(index) {
                Some((rule, _)) => Msg::Edit(Edit::TrayIcon(*rule)),
                None => Msg::Ignored,
            }),
        ),
        viewport,
    );

    page(
        "Tray icon",
        "What resting on, clicking and turning the wheel over the tray icon do.",
        Stack::vertical(space::XL)
            .child("hover", section("Hover", hover))
            .child("clicks", section("Clicks", clicks))
            .child("wheel", section("Wheel", wheel))
            .child("icon", section("Icon", icon)),
    )
}

const ANCHORS: &[(Anchor, &str)] = &[
    (Anchor::Tray, "Next to the tray icon"),
    (Anchor::Cursor, "At the pointer"),
    (Anchor::Hud, "Where the HUD shows"),
];

pub fn mixer_view(
    settings: &Settings,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let mixer = &settings.mixer;

    let anchor = setting(
        "Where it opens",
        "Above the tray icon for a bottom taskbar, below it otherwise; or at the mouse pointer; or where the HUD \
         would be.",
        combo(
            labels(ANCHORS),
            position(ANCHORS, mixer.anchor),
            choose(context, ANCHORS, Edit::MixerAnchor),
        ),
        viewport,
    );

    let (slide, type_in) = number(context, Edit::MaxRows);
    let rows = setting_wide(
        "Rows before it scrolls",
        "How many rows the mixer shows at once; with more, it scrolls with the selection.",
        range_input(
            f64::from(mixer.max_rows),
            f64::from(*MAX_ROWS_RANGE.start()),
            f64::from(*MAX_ROWS_RANGE.end()),
            1.0,
            slide,
            type_in,
        ),
    );

    let sounds = setting(
        "System sounds",
        "List the volume of Windows' own sounds (notifications, alerts) as the last row.",
        switch(
            mixer.system_sounds,
            context.callback(|on: bool| Msg::Edit(Edit::SystemSounds(on))),
        ),
        viewport,
    );

    page(
        "Mixer",
        "The window that lists every volume: the master, the microphone and each application.",
        Stack::vertical(space::M)
            .child("anchor", anchor)
            .child("rows", rows)
            .child("sounds", sounds),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_option_list_finds_its_own_values() {
        for (action, _) in ACTIONS {
            assert!(position(ACTIONS, *action).is_some());
        }
        for (anchor, _) in ANCHORS {
            assert!(position(ANCHORS, *anchor).is_some());
        }
        for (mode, _) in WATCH {
            assert!(position(WATCH, *mode).is_some());
        }
        for (rule, _) in ICON_RULES {
            assert!(position(ICON_RULES, *rule).is_some());
        }
    }

    #[test]
    fn every_value_the_file_can_hold_has_an_option() {
        // A value with no option would show an empty box and could not be left
        // alone by the user: the lists must cover the library's names.
        for name in [
            "none",
            "mixer",
            "hud",
            "mute",
            "mute-mic",
            "volume-up",
            "volume-down",
        ] {
            let action = TrayAction::from_name(name).expect("a known action");
            assert!(position(ACTIONS, action).is_some(), "{name}");
        }
        for name in ["tray", "cursor", "hud"] {
            let anchor = Anchor::from_name(name).expect("a known anchor");
            assert!(position(ANCHORS, anchor).is_some(), "{name}");
        }
        for name in ["off", "tray", "hud"] {
            let mode = WatchMode::from_name(name).expect("a known mode");
            assert!(position(WATCH, mode).is_some(), "{name}");
        }
    }

    #[test]
    fn labels_follow_the_options() {
        assert_eq!(labels(ACTIONS).len(), ACTIONS.len());
        assert_eq!(labels(ANCHORS)[0], "Next to the tray icon");
    }
}
