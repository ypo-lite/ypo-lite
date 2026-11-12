//! `y-coustic.xml`: reading it, and writing it back.
//!
//! Same loader shape as `y-snap`'s: a malformed document aborts the load with
//! a line number ([`LoadError::Malformed`]); anything recognized-but-wrong (an
//! unknown action, a bad key combination, an unknown attribute) is a warning
//! in [`Settings::warnings`] and the rest of the file still loads.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use y_core::hotkey::Hotkey;
use y_core::overlay::Color;
use y_core::xml::{self, Document, Element, Writer};

use crate::appearance;
use crate::controls::{self, Controls, KeysGroup, Layer, Plain, Scope};
use crate::input::{Input, MouseBinding};
use crate::kept::{self, Kept, Note, Part};
use crate::model::{Action, Target};

/// The file y-coustic looks for.
pub const FILE_NAME: &str = "y-coustic.xml";

/// The shipped default, used when no file exists: a fresh install works with
/// nothing to copy.
pub const DEFAULT_XML: &str = include_str!("../rsc/y-coustic.xml");

/// The step range, in percent of the full range (VulCan's).
pub const STEP_RANGE: std::ops::RangeInclusive<u32> = 1..=30;
pub const DEFAULT_STEP: u32 = 5;

/// How long the HUD stays, in milliseconds.
pub const TIMEOUT_RANGE: std::ops::RangeInclusive<u32> = 100..=10_000;
pub const DEFAULT_TIMEOUT: u32 = 1200;
/// How long the HUD takes to fade in and out, in milliseconds; `0` is no fade.
pub const FADE_RANGE: std::ops::RangeInclusive<u32> = 0..=1000;

/// Whether this process has already kept the original file as a backup.
static BACKED_UP: AtomicBool = AtomicBool::new(false);

/// Everything the configuration file says.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The file this came from, for the tray menu and error messages.
    pub path: PathBuf,
    /// Percent of the full range one step moves, within [`STEP_RANGE`].
    pub step: u32,
    /// The single `<hotkey>`s, as written. What to register is
    /// [`Settings::bindings`], which adds the expansion of `controls`.
    pub hotkeys: Vec<Binding>,
    /// The `<controls>` groups and layers, as written.
    pub controls: Controls,
    /// The `<mouse>` wheel and button bindings, in file order.
    pub mouse: Vec<MouseBinding>,
    pub watch: Watch,
    pub mixer: MixerConfig,
    pub tray: TrayConfig,
    pub hud: HudConfig,
    /// What a save gives back besides the values: comments, attributes and
    /// elements y-coustic cannot use. See [`crate::kept`].
    pub kept: Kept,
    /// Problems that did not stop the file from loading, one line each.
    pub warnings: Vec<String>,
}

/// One hotkey and what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub hotkey: Hotkey,
    pub action: Action,
    pub target: Target,
    /// Register on the keyboard hook only, never through `RegisterHotKey`:
    /// what a media key needs (see `y_core::hotkey::Fallback::HookOnly`).
    pub hook_only: bool,
}

/// What to do when an endpoint changes from outside y-coustic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    /// Nothing, and nothing resident for that endpoint.
    Off,
    /// Keep the tray icon true (the microphone's muted variant).
    Tray,
    /// The same, and show the HUD.
    Hud,
}

impl WatchMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Tray => "tray",
            Self::Hud => "hud",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "tray" => Some(Self::Tray),
            "hud" => Some(Self::Hud),
            _ => None,
        }
    }
}

/// The `<watch>` section. The defaults follow from what Windows already does:
/// it shows a flyout for the master when a volume key reaches it, so a HUD
/// there would double it; it shows nothing for the microphone, where a wrong
/// belief costs the most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watch {
    pub master: WatchMode,
    pub microphone: WatchMode,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            master: WatchMode::Off,
            microphone: WatchMode::Hud,
        }
    }
}

/// The look of the HUD's window: its style, size, corners and font.
pub const SCALE_RANGE: std::ops::RangeInclusive<u32> = 50..=300;
pub const RADIUS_RANGE: std::ops::RangeInclusive<u32> = 0..=40;
pub const BORDER_RANGE: std::ops::RangeInclusive<u32> = 0..=8;
pub const OPACITY_RANGE: std::ops::RangeInclusive<u32> = 10..=100;
pub const DEFAULT_RADIUS: u32 = 10;
pub const DEFAULT_FONT: &str = "Segoe UI";

/// The HUD: where, how long, how it looks and whether the pointer can use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudConfig {
    /// Milliseconds the HUD stays after the last change, within
    /// [`TIMEOUT_RANGE`].
    pub timeout: u32,
    pub position: Position,
    /// Whether the pointer can use the HUD: the wheel, a click or a drag on it,
    /// a click on its glyph. `false` keeps it click-through, and turns off the
    /// hover on the tray icon, which would offer a bar that cannot be used.
    pub interactive: bool,
    pub style: Style,
    /// Size in percent of the design size, within [`SCALE_RANGE`].
    pub scale: u32,
    /// Corner radius, in design pixels (before `scale` and the DPI).
    pub radius: u32,
    /// Border width in design pixels, `0` for none.
    pub border: u32,
    /// How opaque the whole HUD is, in percent, within [`OPACITY_RANGE`].
    pub opacity: u32,
    /// The font family of the text.
    pub font: String,
    /// The colours, shared with the mixer: `<colors>`.
    pub colors: Colors,
    /// Milliseconds to fade in and out, within [`FADE_RANGE`]; `0` shows and
    /// hides at once.
    pub fade: u32,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            position: Position::BottomCenter,
            interactive: true,
            style: Style::Bar,
            scale: 100,
            radius: DEFAULT_RADIUS,
            border: 0,
            opacity: 100,
            font: DEFAULT_FONT.to_owned(),
            colors: Colors::default(),
            fade: 0,
        }
    }
}

/// The colours of one look: each is optional, what is left out comes from the
/// theme (see `Palette` in the HUD).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tint {
    /// The window's background.
    pub panel: Option<Color>,
    /// Text and glyphs.
    pub text: Option<Color>,
    /// The empty part of a bar.
    pub track: Option<Color>,
    /// The filled part of a bar, while sound is on.
    pub accent: Option<Color>,
    /// The filled part while muted.
    pub muted: Option<Color>,
}

/// The colours of the HUD and of the mixer, which share them: one set for
/// everything, and one per kind of volume so a glance tells them apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Colors {
    /// For every kind. Its `accent` is not the microphone's: that one is red
    /// unless `<microphone accent>` says otherwise, because mistaking a live
    /// microphone for the master is the costly mistake.
    pub all: Tint,
    pub master: Tint,
    pub microphone: Tint,
    pub application: Tint,
}

/// What the HUD looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Glyph, a horizontal bar and the percentage.
    Bar,
    /// A tall one: glyph above, a vertical bar, the percentage below.
    VBar,
    /// A ring that fills clockwise, the glyph and the percentage inside.
    Ring,
    /// Glyph and percentage, no bar.
    Text,
    /// A small bar: glyph and bar, no percentage.
    Compact,
}

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bar => "bar",
            Self::VBar => "vbar",
            Self::Ring => "ring",
            Self::Text => "text",
            Self::Compact => "compact",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "bar" => Some(Self::Bar),
            "vbar" => Some(Self::VBar),
            "ring" => Some(Self::Ring),
            "text" => Some(Self::Text),
            "compact" => Some(Self::Compact),
            _ => None,
        }
    }
}

/// Where the HUD sits on the monitor under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    /// Next to the mouse pointer when it shows.
    FollowCursor,
}

impl Position {
    pub fn name(self) -> &'static str {
        match self {
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
            Self::CenterLeft => "center-left",
            Self::Center => "center",
            Self::CenterRight => "center-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomCenter => "bottom-center",
            Self::BottomRight => "bottom-right",
            Self::FollowCursor => "follow-cursor",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "top-left" => Some(Self::TopLeft),
            "top-center" => Some(Self::TopCenter),
            "top-right" => Some(Self::TopRight),
            "center-left" => Some(Self::CenterLeft),
            "center" | "centre" => Some(Self::Center),
            "center-right" => Some(Self::CenterRight),
            "bottom-left" => Some(Self::BottomLeft),
            "bottom-center" => Some(Self::BottomCenter),
            "bottom-right" => Some(Self::BottomRight),
            "follow-cursor" => Some(Self::FollowCursor),
            _ => None,
        }
    }

    /// On the lower edge of the screen, where a mixer opens upward from.
    pub fn is_bottom(self) -> bool {
        matches!(
            self,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight | Self::FollowCursor
        )
    }
}

/// How long the pointer rests on the tray icon before the HUD shows, in
/// milliseconds; `0` turns the hover off.
pub const HOVER_DELAY_RANGE: std::ops::RangeInclusive<u32> = 0..=5000;
pub const DEFAULT_HOVER_DELAY: u32 = 600;

/// What a click on the tray icon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Nothing.
    None,
    /// Open or close the mixer.
    Mixer,
    /// Show the master volume on the HUD, changing nothing.
    Hud,
    /// Toggle the master's mute.
    Mute,
    /// Toggle the microphone's mute.
    MuteMic,
    /// One step up on the master.
    VolumeUp,
    /// One step down on the master.
    VolumeDown,
}

impl TrayAction {
    /// The name used in the configuration file.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Mixer => "mixer",
            Self::Hud => "hud",
            Self::Mute => "mute",
            Self::MuteMic => "mute-mic",
            Self::VolumeUp => "volume-up",
            Self::VolumeDown => "volume-down",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "none" => Self::None,
            "mixer" => Self::Mixer,
            "hud" => Self::Hud,
            "mute" => Self::Mute,
            "mute-mic" => Self::MuteMic,
            "volume-up" => Self::VolumeUp,
            "volume-down" => Self::VolumeDown,
            _ => return None,
        })
    }
}

/// The tray icon: what resting the pointer on it, and clicking it, do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayConfig {
    /// Milliseconds the pointer rests on the icon before the HUD shows, within
    /// [`HOVER_DELAY_RANGE`]. `0` keeps the standard tooltip instead.
    pub hover_delay: u32,
    /// A single left click. With a `double_click` set it waits the system's
    /// double-click time, to know it is not the first half of one.
    pub click: TrayAction,
    /// A double left click; `None` leaves the single click immediate.
    pub double_click: TrayAction,
    /// A middle click.
    pub middle_click: TrayAction,
    /// Whether the wheel over the icon steps the master volume. Needs the hover
    /// (`hover-delay` above 0): the shell only reports the pointer on an icon
    /// without a tooltip.
    pub wheel: bool,
    /// How the icon changes with the state of the devices: `fixed` (never) by
    /// default, see [`crate::appearance`].
    pub icon: appearance::Rule,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            hover_delay: DEFAULT_HOVER_DELAY,
            click: TrayAction::Mixer,
            double_click: TrayAction::None,
            middle_click: TrayAction::None,
            wheel: true,
            icon: appearance::Rule::Fixed,
        }
    }
}

/// How many rows the mixer shows before it scrolls.
pub const MAX_ROWS_RANGE: std::ops::RangeInclusive<u32> = 3..=20;
pub const DEFAULT_MAX_ROWS: u32 = 8;

/// The mixer: where it opens, how tall it gets, what it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerConfig {
    pub anchor: Anchor,
    /// Rows shown before the list scrolls with the selection, within
    /// [`MAX_ROWS_RANGE`].
    pub max_rows: u32,
    /// Whether Windows' own system sounds get a last row.
    pub system_sounds: bool,
}

impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            anchor: Anchor::Tray,
            max_rows: DEFAULT_MAX_ROWS,
            system_sounds: true,
        }
    }
}

/// Where the mixer opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Next to the tray icon (the pointer's place when the shell does not say
    /// where the icon is).
    Tray,
    /// At the mouse pointer.
    Cursor,
    /// Where the HUD appears.
    Hud,
}

impl Anchor {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tray => "tray",
            Self::Cursor => "cursor",
            Self::Hud => "hud",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "tray" => Some(Self::Tray),
            "cursor" => Some(Self::Cursor),
            "hud" => Some(Self::Hud),
            _ => None,
        }
    }
}

/// Why the configuration could not be read at all.
#[derive(Debug)]
pub enum LoadError {
    Missing(PathBuf),
    Unreadable(PathBuf, io::Error),
    Malformed(PathBuf, xml::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "No configuration file at {}", path.display()),
            Self::Unreadable(path, error) => write!(f, "Cannot read {}: {error}", path.display()),
            Self::Malformed(path, error) => write!(f, "{}, {error}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

impl Settings {
    /// Nothing configured yet, for a file that does not exist: what a
    /// settings app edits on a machine where saving will create `path`.
    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            step: DEFAULT_STEP,
            hotkeys: Vec::new(),
            controls: Controls::default(),
            mouse: Vec::new(),
            watch: Watch::default(),
            mixer: MixerConfig::default(),
            tray: TrayConfig::default(),
            hud: HudConfig::default(),
            kept: Kept::default(),
            warnings: Vec::new(),
        }
    }

    /// Reads the configuration from wherever [`path`] finds it.
    pub fn load() -> Result<Self, LoadError> {
        Self::load_from(path())
    }

    pub fn load_from(path: PathBuf) -> Result<Self, LoadError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(LoadError::Missing(path));
            }
            Err(error) => return Err(LoadError::Unreadable(path, error)),
        };
        Self::parse(path, &text)
    }

    /// Reads `text` as the contents of `path`.
    pub fn parse(path: PathBuf, text: &str) -> Result<Self, LoadError> {
        let document =
            Document::parse(text).map_err(|error| LoadError::Malformed(path.clone(), error))?;

        let mut reader = Reader::default();
        let root = document.root();
        reader.sections(root);
        reader.keep(&document);

        let step = reader.step(root);
        let hotkeys = reader.hotkeys(root);
        let controls = reader.controls(root);
        let mouse = reader.mouse(root);
        let watch = reader.watch(root);
        let mixer = reader.mixer(root);
        let tray = reader.tray(root);
        let mut hud = reader.hud(root);
        hud.colors = reader.colors(root);

        // What the groups and layers cross into can collide with each other or
        // with a single hotkey: say so now, once, not at every registration.
        let (_, crossing) = controls::expand(&hotkeys, &controls);
        reader.warnings.extend(crossing);

        Ok(Self {
            path,
            step,
            hotkeys,
            controls,
            mouse,
            watch,
            mixer,
            tray,
            hud,
            kept: reader.kept,
            warnings: reader.warnings,
        })
    }

    /// Everything to register: the single hotkeys, then the groups crossed with
    /// the layers. The same expansion that produced the load warnings.
    pub fn bindings(&self) -> Vec<Binding> {
        controls::expand(&self.hotkeys, &self.controls).0
    }

    /// Whether the configuration has any group of volume keys: when the tray
    /// offers the toggle below.
    pub fn has_media_group(&self) -> bool {
        self.controls.keys.iter().any(KeysGroup::has_media_keys)
    }

    /// Whether any media group lets y-coustic take the bare volume keys: what
    /// the tray's *Replace the Windows volume indicator* toggle shows.
    pub fn replaces_windows_volume(&self) -> bool {
        self.controls
            .keys
            .iter()
            .any(|group| group.has_media_keys() && group.plain == Plain::Replace)
    }

    /// Sets `plain` on every media group: the tray toggle.
    pub fn set_replace_windows_volume(&mut self, replace: bool) {
        let plain = if replace {
            Plain::Replace
        } else {
            Plain::Windows
        };
        for group in &mut self.controls.keys {
            if group.has_media_keys() {
                group.plain = plain;
            }
        }
    }

    /// Renders this configuration as `y-coustic.xml`, in the shape
    /// [`Settings::parse`] reads back, with the comments, unknown attributes
    /// and unusable elements the file had ([`crate::kept`]). Sections come in a
    /// fixed order, each with its attributes in a fixed order; the empty lines
    /// of the original are kept where they were, the others are the writer's.
    /// Writing what was read gives the same bytes again.
    pub fn to_xml(&self) -> String {
        let mut writer = Writer::new();
        writer.declaration();
        kept::write_notes(&mut writer, &self.kept.prolog);
        writer.open("y-coustic");
        for name in kept::SECTIONS {
            self.write_section(&mut writer, name);
        }
        kept::write_notes(&mut writer, &self.kept.root_tail);
        writer.close();
        // Comments after the root element go on lines of their own.
        kept::write_notes(&mut writer, &self.kept.epilog);
        writer.finish()
    }

    /// One section, preceded by its comments and an empty line. A section
    /// that is empty by default is left out unless the file had it.
    fn write_section(&self, w: &mut Writer, name: &str) {
        let kept_section = self.kept.section(name);
        let present = kept_section.is_some();
        let wanted = match name {
            "controls" => !self.controls.is_empty(),
            "mouse" => !self.mouse.is_empty(),
            "colors" => self.hud.colors != Colors::default(),
            _ => true,
        };
        if !wanted && !present {
            return;
        }

        let part = kept_section.map(|section| &section.part);
        w.blank_line();
        if let Some(part) = part {
            kept::write_notes(w, &part.lead);
            if part.blank_before {
                w.blank_line();
            }
        }
        w.open(name);
        match name {
            "volume" => w.attr("step", &self.step.to_string()),
            "watch" => {
                w.attr("master", self.watch.master.name());
                w.attr("microphone", self.watch.microphone.name());
            }
            "tray" => self.write_tray_attributes(w),
            "mixer" => {
                w.attr("anchor", self.mixer.anchor.name());
                w.attr("max-rows", &self.mixer.max_rows.to_string());
                w.attr("system-sounds", &self.mixer.system_sounds.to_string());
            }
            "hud" => self.write_hud_attributes(w),
            "colors" => write_tint(w, &self.hud.colors.all),
            _ => {}
        }
        kept::write_extra(w, part);

        match name {
            "controls" => self.write_controls(w),
            "hotkeys" => self.write_hotkeys(w),
            "mouse" => self.write_mouse(w),
            "colors" => write_kind_colors(w, &self.hud.colors),
            _ => {}
        }
        if let Some(section) = kept_section {
            kept::write_notes(w, &section.tail);
        }
        w.close();
    }

    fn write_tray_attributes(&self, w: &mut Writer) {
        w.attr("hover-delay", &self.tray.hover_delay.to_string());
        w.attr("click", self.tray.click.name());
        w.attr("double-click", self.tray.double_click.name());
        w.attr("middle-click", self.tray.middle_click.name());
        w.attr("wheel", &self.tray.wheel.to_string());
        w.attr("icon", self.tray.icon.name());
    }

    fn write_hud_attributes(&self, w: &mut Writer) {
        let hud = &self.hud;
        w.attr("style", hud.style.name());
        w.attr("position", hud.position.name());
        w.attr("timeout", &hud.timeout.to_string());
        w.attr("interactive", &hud.interactive.to_string());
        w.attr("scale", &hud.scale.to_string());
        w.attr("radius", &hud.radius.to_string());
        w.attr("border", &hud.border.to_string());
        w.attr("opacity", &hud.opacity.to_string());
        w.attr("font", &hud.font);
        w.attr("fade", &hud.fade.to_string());
    }

    fn write_controls(&self, w: &mut Writer) {
        for (index, group) in self.controls.keys.iter().enumerate() {
            let kept = self.kept.keys.get(index);
            let part = kept.map(|group| &group.part);
            // A group with children of its own is a block: set it apart.
            if index > 0 && matches!(&group.scope, Scope::Layers(layers) if !layers.is_empty()) {
                w.blank_line();
            }
            kept::open_entry(w, "keys", part);
            for (name, key) in [("up", group.up), ("down", group.down), ("mute", group.mute)] {
                if let Some(key) = key {
                    w.attr(name, &key.to_string());
                }
            }
            if group.has_media_keys() {
                w.attr("plain", group.plain.name());
            }
            if let Scope::Target(target) = &group.scope {
                w.attr("target", target.name());
            }
            kept::write_extra(w, part);
            if let Scope::Layers(layers) = &group.scope {
                for (at, layer) in layers.iter().enumerate() {
                    let layer_part = kept.and_then(|group| group.layers.get(at));
                    kept::open_entry(w, "layer", layer_part);
                    w.attr("mod", &controls::format_modifiers(layer.modifiers));
                    w.attr("target", layer.target.name());
                    kept::write_extra(w, layer_part);
                    w.close();
                }
            }
            if let Some(kept) = kept {
                kept::write_notes(w, &kept.tail);
            }
            w.close();
        }
    }

    fn write_hotkeys(&self, w: &mut Writer) {
        for (index, binding) in self.hotkeys.iter().enumerate() {
            kept::open_entry(w, "hotkey", self.kept.hotkeys.get(index));
            w.attr("key", &binding.hotkey.to_string());
            w.attr("action", binding.action.name());
            if binding.target != Target::Master {
                w.attr("target", binding.target.name());
            }
            if let Action::SetVolume(percent) = binding.action {
                w.attr("value", &percent.to_string());
            }
            kept::write_extra(w, self.kept.hotkeys.get(index));
            w.close();
        }
    }

    fn write_mouse(&self, w: &mut Writer) {
        for (index, binding) in self.mouse.iter().enumerate() {
            let part = self.kept.mouse.get(index);
            let modifiers = controls::format_modifiers(binding.modifiers);
            match (binding.input, binding.action) {
                (Input::Wheel | Input::WheelTilt, _) => {
                    kept::open_entry(w, "wheel", part);
                    w.attr("mod", &modifiers);
                    if binding.input == Input::WheelTilt {
                        w.attr("axis", "horizontal");
                    }
                    if binding.target != Target::Master {
                        w.attr("target", binding.target.name());
                    }
                    if binding.invert {
                        w.attr("invert", "true");
                    }
                }
                (input, action) => {
                    kept::open_entry(w, "button", part);
                    w.attr("button", input.button_name().unwrap_or("left"));
                    w.attr("mod", &modifiers);
                    let action = action.unwrap_or(Action::Mute);
                    w.attr("action", action.name());
                    if let Action::SetVolume(percent) = action {
                        w.attr("value", &percent.to_string());
                    }
                    if binding.target != Target::Master && action != Action::Mixer {
                        w.attr("target", binding.target.name());
                    }
                }
            }
            kept::write_extra(w, part);
            w.close();
        }
    }

    /// Writes [`Settings::to_xml`] to [`Settings::path`].
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&self.path)
    }

    /// Writes [`Settings::to_xml`] to `path`, creating the folder: on a new
    /// machine `%APPDATA%\Ypo-Lite` does not exist yet.
    ///
    /// The text goes to a temporary file beside `path` and is renamed over it, so
    /// a crash or a full disk never leaves half a file where the user's was. The
    /// first save of a process also keeps what was there as `y-coustic.xml.bak`.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let backup = path.is_file() && !BACKED_UP.swap(true, Ordering::Relaxed);
        self.write_safely(path, backup)
    }

    /// [`Settings::save_to`] with the backup decided by the caller.
    fn write_safely(&self, path: &Path, backup: bool) -> io::Result<()> {
        if backup {
            let mut backup = path.as_os_str().to_owned();
            backup.push(".bak");
            std::fs::copy(path, backup)?;
        }

        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, self.to_xml())?;
        std::fs::rename(&temporary, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }
}

/// Where the configuration file lives, in the order the places are tried:
/// `-config <path>`, then next to the executable (a portable copy), then
/// `%APPDATA%\Ypo-Lite`.
pub fn path() -> PathBuf {
    if let Some(path) = argument_path() {
        return path;
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        let portable = directory.join(FILE_NAME);
        if portable.is_file() {
            return portable;
        }
    }

    match std::env::var_os("APPDATA") {
        Some(appdata) => PathBuf::from(appdata).join("Ypo-Lite").join(FILE_NAME),
        None => PathBuf::from(FILE_NAME),
    }
}

/// `-config <path>`, with `%VARIABLES%` expanded.
fn argument_path() -> Option<PathBuf> {
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        if argument.eq_ignore_ascii_case("-config") || argument.eq_ignore_ascii_case("--config") {
            return Some(PathBuf::from(expand(&arguments.next()?)));
        }
    }
    None
}

/// Replaces `%NAME%` with the environment variable, leaving an unset one as
/// written: a path with a stray `%` in it is better shown back intact than
/// silently emptied.
fn expand(text: &str) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];

        match after.find('%') {
            Some(end) => {
                match std::env::var(&after[..end]) {
                    Ok(value) => expanded.push_str(&value),
                    Err(_) => expanded.push_str(&rest[start..=start + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                expanded.push_str(&rest[start..]);
                return expanded;
            }
        }
    }

    expanded.push_str(rest);
    expanded
}

/// The attributes each element reads: what `check` accepts, and what a save
/// does not need to keep as an extra.
const VOLUME_ATTRS: &[&str] = &["step"];
const WATCH_ATTRS: &[&str] = &["master", "microphone"];
const HOTKEY_ATTRS: &[&str] = &["key", "action", "target", "value"];
const KEYS_ATTRS: &[&str] = &["up", "down", "mute", "plain", "target"];
const LAYER_ATTRS: &[&str] = &["mod", "target"];
const WHEEL_ATTRS: &[&str] = &["mod", "target", "invert", "axis"];
const BUTTON_ATTRS: &[&str] = &["button", "mod", "action", "target", "value"];
const TRAY_ATTRS: &[&str] = &[
    "hover-delay",
    "click",
    "double-click",
    "middle-click",
    "wheel",
    "icon",
];
const MIXER_ATTRS: &[&str] = &["anchor", "max-rows", "system-sounds"];
const HUD_ATTRS: &[&str] = &[
    "style",
    "position",
    "timeout",
    "interactive",
    "scale",
    "radius",
    "border",
    "opacity",
    "font",
    "fade",
];
const COLOR_ATTRS: &[&str] = &["panel", "text", "track", "accent", "muted"];

/// The attributes the section element `name` reads. The three lists read none
/// of their own.
fn section_attrs(name: &str) -> &'static [&'static str] {
    match name {
        "volume" => VOLUME_ATTRS,
        "watch" => WATCH_ATTRS,
        "tray" => TRAY_ATTRS,
        "mixer" => MIXER_ATTRS,
        "hud" => HUD_ATTRS,
        "colors" => COLOR_ATTRS,
        _ => &[],
    }
}

fn notes_of<'a>(comments: impl Iterator<Item = xml::Comment<'a>>) -> Vec<Note> {
    comments
        .map(|comment| Note::new(comment.text(), comment.blank_before()))
        .collect()
}

/// Walks the document, collecting warnings as it goes.
#[derive(Default)]
struct Reader {
    warnings: Vec<String>,
    kept: Kept,
}

impl Reader {
    fn warn(&mut self, element: Element<'_>, message: impl fmt::Display) {
        self.warnings
            .push(format!("line {}: {message}", element.line()));
    }

    /// Collects what a save must give back besides the values (see
    /// [`crate::kept`]): the comments at every level, the attributes no one
    /// reads, and the elements no one can use, which become comments. Run
    /// before the sections are read, which add their lists' parts and tails.
    fn keep(&mut self, document: &Document) {
        let root = document.root();
        self.kept.prolog = notes_of(document.prolog_comments());
        self.kept.epilog = notes_of(document.epilog_comments());

        let mut pending: Vec<Note> = Vec::new();
        for node in root.children_with_comments() {
            let element = match node {
                xml::Node::Comment(comment) => {
                    pending.push(Note::new(comment.text(), comment.blank_before()));
                    continue;
                }
                xml::Node::Element(element) => element,
            };
            let name = element.name();
            if !kept::SECTIONS.contains(&name) || self.kept.section(name).is_some() {
                // Unknown, or a second copy: the first is the one read.
                pending.push(Note::new(kept::element_xml(element), element.blank_before()));
                continue;
            }
            let part = Part {
                lead: std::mem::take(&mut pending),
                blank_before: element.blank_before(),
                extra: kept::unknown_attributes(element, section_attrs(name)),
            };
            let tail = if kept::LISTS.contains(&name) {
                Vec::new()
            } else {
                // A section with no entries: whatever comments it holds.
                notes_of(element.children_with_comments().filter_map(|node| match node {
                    xml::Node::Comment(comment) => Some(comment),
                    xml::Node::Element(_) => None,
                }))
            };
            *self.kept.section_mut(name) = kept::Section { part, tail };
        }
        self.kept.root_tail = pending;
    }

    /// The entries of a list section, each with what goes with it. `read`
    /// gives the value and the attributes it reads, or `None` after a
    /// warning: the element is then kept as a comment, ahead of the next
    /// entry. The second value is the comments after the last entry.
    fn walk<T>(
        &mut self,
        section: Element<'_>,
        mut read: impl FnMut(&mut Self, Element<'_>) -> Option<(T, &'static [&'static str])>,
    ) -> (Vec<(T, Part)>, Vec<Note>) {
        let mut items = Vec::new();
        let mut pending: Vec<Note> = Vec::new();
        for node in section.children_with_comments() {
            match node {
                xml::Node::Comment(comment) => {
                    pending.push(Note::new(comment.text(), comment.blank_before()));
                }
                xml::Node::Element(element) => match read(self, element) {
                    Some((value, known)) => {
                        let part = Part {
                            lead: std::mem::take(&mut pending),
                            blank_before: element.blank_before(),
                            extra: kept::unknown_attributes(element, known),
                        };
                        items.push((value, part));
                    }
                    None => {
                        pending.push(Note::new(kept::element_xml(element), element.blank_before()))
                    }
                },
            }
        }
        (items, pending)
    }

    /// Classifies the root's children: the sections read, and the unknown. A second
    /// copy of a read section is reported: [`Element::child`] reads only the first,
    /// so the rest would be ignored with nothing else to notice.
    fn sections(&mut self, root: Element<'_>) {
        const READ: &[&str] = &[
            "volume", "watch", "controls", "hotkeys", "mouse", "mixer", "tray", "hud", "colors",
        ];

        let mut seen: Vec<&str> = Vec::new();
        for element in root.children() {
            let name = element.name();
            if READ.contains(&name) {
                if seen.contains(&name) {
                    self.warn(
                        element,
                        format!("a second <{name}> is ignored: only the first one is read"),
                    );
                }
                seen.push(name);
            } else {
                self.warn(element, format!("unknown element <{name}>"));
            }
        }
    }

    /// Reports attributes nobody reads: almost always a typo, and silently
    /// ignoring one means a setting that visibly does nothing.
    fn check(&mut self, element: Element<'_>, known: &[&str]) {
        let unknown: Vec<_> = element
            .attributes()
            .map(|(name, _)| name)
            .filter(|name| !known.contains(name))
            .collect();

        for name in unknown {
            let element_name = element.name();
            self.warn(element, format!("<{element_name}> has no {name} attribute"));
        }
    }

    /// A `true` or `false` attribute, `default` when absent; anything else is
    /// warned and also gives `default`.
    fn flag(&mut self, element: Element<'_>, name: &str, default: bool) -> bool {
        match element
            .attr(name)
            .map(|text| text.trim().to_ascii_lowercase())
        {
            None => default,
            Some(text) if text == "true" => true,
            Some(text) if text == "false" => false,
            Some(text) => {
                self.warn(element, format!("{name}={text:?} is not true or false"));
                default
            }
        }
    }

    /// An integer attribute clamped to `range`, `default` when absent or not
    /// a number; both of the latter two but absence are warned.
    fn number(
        &mut self,
        element: Element<'_>,
        name: &str,
        range: &std::ops::RangeInclusive<u32>,
        default: u32,
    ) -> u32 {
        let Some(text) = element.attr(name) else {
            return default;
        };
        let Ok(value) = text.trim().parse::<u32>() else {
            self.warn(element, format!("{name}={text:?} is not a number"));
            return default;
        };
        let clamped = value.clamp(*range.start(), *range.end());
        if clamped != value {
            self.warn(
                element,
                format!(
                    "{name}={value} is outside {}..{}, using {clamped}",
                    range.start(),
                    range.end()
                ),
            );
        }
        clamped
    }

    fn step(&mut self, root: Element<'_>) -> u32 {
        let Some(element) = root.child("volume") else {
            return DEFAULT_STEP;
        };
        self.check(element, VOLUME_ATTRS);
        self.number(element, "step", &STEP_RANGE, DEFAULT_STEP)
    }

    fn hotkeys(&mut self, root: Element<'_>) -> Vec<Binding> {
        let Some(section) = root.child("hotkeys") else {
            return Vec::new();
        };

        let (items, tail) = self.walk(section, |this, element| {
            if element.name() != "hotkey" {
                let name = element.name();
                this.warn(element, format!("<{name}> is not a <hotkey>"));
                return None;
            }
            this.check(element, HOTKEY_ATTRS);
            this.binding(element).map(|binding| (binding, HOTKEY_ATTRS))
        });
        self.kept.section_mut("hotkeys").tail = tail;
        let (bindings, parts) = items.into_iter().unzip();
        self.kept.hotkeys = parts;
        bindings
    }

    /// The `action` (and, for `set-volume`, `value`) of a `<hotkey>` or a
    /// `<button>`. `None` after a warning.
    fn action(&mut self, element: Element<'_>) -> Option<Action> {
        let element_name = element.name();
        let Some(name) = element.attr("action") else {
            self.warn(
                element,
                format!("a <{element_name}> needs an action attribute"),
            );
            return None;
        };
        let action = match name.trim().to_ascii_lowercase().as_str() {
            "volume-up" => Action::VolumeUp,
            "volume-down" => Action::VolumeDown,
            "mute" => Action::Mute,
            "mixer" => Action::Mixer,
            "set-volume" => {
                let Some(text) = element.attr("value") else {
                    self.warn(element, "set-volume needs a value attribute (0-100)");
                    return None;
                };
                match text.trim().parse::<u8>() {
                    Ok(percent) if percent <= 100 => Action::SetVolume(percent),
                    _ => {
                        self.warn(element, format!("value={text:?} is not 0-100"));
                        return None;
                    }
                }
            }
            "volume" => {
                self.warn(element, "volume only works on a mouse wheel");
                return None;
            }
            _ => {
                self.warn(element, format!("unknown action {name:?}"));
                return None;
            }
        };
        Some(action)
    }

    fn binding(&mut self, element: Element<'_>) -> Option<Binding> {
        let action = self.action(element)?;

        let Some(text) = element.attr("key") else {
            self.warn(element, "a <hotkey> needs a key attribute");
            return None;
        };
        let Some(hotkey) = Hotkey::parse(text) else {
            self.warn(element, format!("{text:?} is not a key combination"));
            return None;
        };

        let target = match element.attr("target") {
            None => Target::Master,
            Some(name) => match Target::from_name(name) {
                Some(target) => target,
                None => {
                    self.warn(element, format!("unknown target {name:?}"));
                    return None;
                }
            },
        };
        if action == Action::Mixer && element.attr("target").is_some() {
            self.warn(element, "mixer takes no target, ignoring it");
        }

        Some(Binding {
            hotkey,
            action,
            target: if action == Action::Mixer {
                Target::Master
            } else {
                target
            },
            // A single hotkey on a media key is the user's explicit choice of
            // key; it needs the hook for the same reason a group's does.
            hook_only: controls::is_media_key(hotkey.key),
        })
    }

    fn controls(&mut self, root: Element<'_>) -> Controls {
        let Some(section) = root.child("controls") else {
            return Controls::default();
        };

        let (items, tail) = self.walk(section, |this, element| match element.name() {
            "keys" => this
                .keys_group(element)
                .map(|group| (group, KEYS_ATTRS)),
            "layer" => {
                this.warn(
                    element,
                    "a <layer> belongs inside a <keys>: it is ignored here, move it into the group it is for",
                );
                None
            }
            name => {
                this.warn(element, format!("<{name}> is not a <keys>"));
                None
            }
        });
        self.kept.section_mut("controls").tail = tail;

        let mut controls = Controls::default();
        self.kept.keys.clear();
        for ((group, layers, group_tail), part) in items {
            controls.keys.push(group);
            self.kept.keys.push(kept::GroupPart {
                part,
                layers,
                tail: group_tail,
            });
        }
        controls
    }

    /// One `<keys>`: its keys, and what they control (a `target`, or `<layer>`
    /// children of its own). The `Part`s of the layers and the comments after
    /// them come with it, for what a save gives back.
    fn keys_group(&mut self, element: Element<'_>) -> Option<(KeysGroup, Vec<Part>, Vec<Note>)> {
        self.check(element, KEYS_ATTRS);

        let key = |this: &mut Self, name: &str| -> Option<Hotkey> {
            let text = element.attr(name)?;
            let parsed = Hotkey::parse(text);
            if parsed.is_none() {
                this.warn(element, format!("{text:?} is not a key combination"));
            }
            parsed
        };
        let up = key(self, "up");
        let down = key(self, "down");
        let mute = key(self, "mute");
        if up.is_none() && down.is_none() && mute.is_none() {
            self.warn(element, "a <keys> needs at least one of up, down or mute");
            return None;
        }

        let plain = match element.attr("plain") {
            None => Plain::Windows,
            Some(text) => Plain::from_name(text).unwrap_or_else(|| {
                self.warn(element, format!("plain={text:?} is not windows or replace"));
                Plain::Windows
            }),
        };

        let (items, tail) = self.walk(element, |this, child| {
            if child.name() != "layer" {
                let name = child.name();
                this.warn(child, format!("<{name}> is not a <layer>"));
                return None;
            }
            this.layer(child).map(|layer| (layer, LAYER_ATTRS))
        });
        let (layers, parts): (Vec<Layer>, Vec<Part>) = items.into_iter().unzip();

        let target = match element.attr("target") {
            None => None,
            Some(name) => Target::from_name(name).or_else(|| {
                self.warn(element, format!("unknown target {name:?}"));
                None
            }),
        };
        let scope = match (layers.is_empty(), target) {
            (false, Some(_)) => {
                self.warn(
                    element,
                    "a <keys> with <layer>s of its own ignores its target: the layers say what it controls",
                );
                Scope::Layers(layers)
            }
            (false, None) => Scope::Layers(layers),
            (true, Some(target)) => Scope::Target(target),
            // Controls nothing: `expand` says so, once, with the group's keys.
            (true, None) => Scope::Layers(Vec::new()),
        };
        Some((
            KeysGroup {
                up,
                down,
                mute,
                plain,
                scope,
            },
            parts,
            tail,
        ))
    }

    fn layer(&mut self, element: Element<'_>) -> Option<Layer> {
        self.check(element, LAYER_ATTRS);

        let modifiers = match element.attr("mod") {
            None => 0,
            Some(text) => match controls::parse_modifiers(text) {
                Some(modifiers) => modifiers,
                None => {
                    self.warn(element, format!("{text:?} is not a modifier list"));
                    return None;
                }
            },
        };
        let target = match element.attr("target") {
            None => Target::Master,
            Some(name) => match Target::from_name(name) {
                Some(target) => target,
                None => {
                    self.warn(element, format!("unknown target {name:?}"));
                    return None;
                }
            },
        };
        Some(Layer { modifiers, target })
    }

    fn mouse(&mut self, root: Element<'_>) -> Vec<MouseBinding> {
        let Some(section) = root.child("mouse") else {
            return Vec::new();
        };

        let mut seen: Vec<(Input, u32)> = Vec::new();
        let (items, tail) = self.walk(section, |this, element| {
            let (binding, known) = match element.name() {
                "wheel" => (this.wheel(element)?, WHEEL_ATTRS),
                "button" => (this.button(element)?, BUTTON_ATTRS),
                name => {
                    this.warn(element, format!("<{name}> is not a <wheel> or a <button>"));
                    return None;
                }
            };

            // The first one wins at run time; say so rather than keep a
            // binding that can never fire.
            if seen.contains(&(binding.input, binding.modifiers)) {
                this.warn(
                    element,
                    "an earlier binding already uses this input with these modifiers, ignoring it",
                );
                return None;
            }
            seen.push((binding.input, binding.modifiers));
            Some((binding, known))
        });
        self.kept.section_mut("mouse").tail = tail;
        let (bindings, parts) = items.into_iter().unzip();
        self.kept.mouse = parts;
        bindings
    }

    /// The `mod` of a mouse binding. A bare wheel or main button would take
    /// over scrolling and clicking everywhere, so it is refused (the side
    /// buttons may be bare). `None` after a warning.
    fn mouse_modifiers(&mut self, element: Element<'_>, input: Input) -> Option<u32> {
        let modifiers = match element.attr("mod") {
            None => 0,
            Some(text) => {
                let Some(modifiers) = controls::parse_modifiers(text) else {
                    self.warn(element, format!("{text:?} is not a modifier list"));
                    return None;
                };
                modifiers
            }
        };
        if modifiers == 0 && !input.may_be_bare() {
            let name = element.name();
            self.warn(
                element,
                format!("a <{name}> needs a mod: a bare one would take over every scroll or click"),
            );
            return None;
        }
        Some(modifiers)
    }

    /// The `target` of a mouse binding, the master when absent. `None` after
    /// a warning.
    fn mouse_target(&mut self, element: Element<'_>) -> Option<Target> {
        match element.attr("target") {
            None => Some(Target::Master),
            Some(name) => {
                let target = Target::from_name(name);
                if target.is_none() {
                    self.warn(element, format!("unknown target {name:?}"));
                }
                target
            }
        }
    }

    fn wheel(&mut self, element: Element<'_>) -> Option<MouseBinding> {
        self.check(element, WHEEL_ATTRS);

        let input = match element.attr("axis").map(|a| a.trim().to_ascii_lowercase()) {
            None => Input::Wheel,
            Some(axis) if axis == "vertical" => Input::Wheel,
            Some(axis) if axis == "horizontal" => Input::WheelTilt,
            Some(axis) => {
                self.warn(
                    element,
                    format!("axis={axis:?} is not vertical or horizontal"),
                );
                return None;
            }
        };
        let modifiers = self.mouse_modifiers(element, input)?;
        let target = self.mouse_target(element)?;
        let invert = match element.attr("invert").map(|a| a.trim().to_ascii_lowercase()) {
            None => false,
            Some(text) if text == "true" => true,
            Some(text) if text == "false" => false,
            Some(text) => {
                self.warn(element, format!("invert={text:?} is not true or false"));
                false
            }
        };

        Some(MouseBinding {
            input,
            modifiers,
            target,
            action: None,
            invert,
        })
    }

    fn button(&mut self, element: Element<'_>) -> Option<MouseBinding> {
        self.check(element, BUTTON_ATTRS);

        let Some(name) = element.attr("button") else {
            self.warn(element, "a <button> needs a button attribute");
            return None;
        };
        let Some(input) = Input::from_button_name(name) else {
            self.warn(
                element,
                format!("{name:?} is not left, right, middle, x1 or x2"),
            );
            return None;
        };
        let modifiers = self.mouse_modifiers(element, input)?;
        let action = self.action(element)?;

        let target = if action == Action::Mixer {
            if element.attr("target").is_some() {
                self.warn(element, "mixer takes no target, ignoring it");
            }
            Target::Master
        } else {
            self.mouse_target(element)?
        };

        Some(MouseBinding {
            input,
            modifiers,
            target,
            action: Some(action),
            invert: false,
        })
    }

    fn watch(&mut self, root: Element<'_>) -> Watch {
        let defaults = Watch::default();
        let Some(element) = root.child("watch") else {
            return defaults;
        };
        self.check(element, WATCH_ATTRS);

        let mode = |this: &mut Self, name: &str, default: WatchMode| match element.attr(name) {
            None => default,
            Some(text) => WatchMode::from_name(text).unwrap_or_else(|| {
                this.warn(element, format!("{name}={text:?} is not off, tray or hud"));
                default
            }),
        };
        Watch {
            master: mode(self, "master", defaults.master),
            microphone: mode(self, "microphone", defaults.microphone),
        }
    }

    fn tray(&mut self, root: Element<'_>) -> TrayConfig {
        let defaults = TrayConfig::default();
        let Some(element) = root.child("tray") else {
            return defaults;
        };
        self.check(element, TRAY_ATTRS);
        let action = |this: &mut Self, name: &str, default: TrayAction| match element.attr(name) {
            None => default,
            Some(text) => TrayAction::from_name(text).unwrap_or_else(|| {
                this.warn(
                    element,
                    format!(
                        "{name}={text:?} is not mixer, hud, mute, mute-mic, volume-up, \
                         volume-down or none"
                    ),
                );
                default
            }),
        };
        TrayConfig {
            hover_delay: self.number(
                element,
                "hover-delay",
                &HOVER_DELAY_RANGE,
                defaults.hover_delay,
            ),
            click: action(self, "click", defaults.click),
            double_click: action(self, "double-click", defaults.double_click),
            middle_click: action(self, "middle-click", defaults.middle_click),
            wheel: self.flag(element, "wheel", defaults.wheel),
            icon: match element.attr("icon") {
                None => defaults.icon,
                Some(text) => appearance::Rule::from_name(text).unwrap_or_else(|| {
                    self.warn(element, format!("icon={text:?} is not fixed or mic-muted"));
                    defaults.icon
                }),
            },
        }
    }

    fn mixer(&mut self, root: Element<'_>) -> MixerConfig {
        let defaults = MixerConfig::default();
        let Some(element) = root.child("mixer") else {
            return defaults;
        };
        self.check(element, MIXER_ATTRS);

        let anchor = match element.attr("anchor") {
            None => defaults.anchor,
            Some(name) => Anchor::from_name(name).unwrap_or_else(|| {
                self.warn(element, format!("anchor={name:?} is not tray, cursor or hud"));
                defaults.anchor
            }),
        };
        let system_sounds = match element
            .attr("system-sounds")
            .map(|text| text.trim().to_ascii_lowercase())
        {
            None => defaults.system_sounds,
            Some(text) if text == "true" => true,
            Some(text) if text == "false" => false,
            Some(text) => {
                self.warn(
                    element,
                    format!("system-sounds={text:?} is not true or false"),
                );
                defaults.system_sounds
            }
        };
        MixerConfig {
            anchor,
            max_rows: self.number(element, "max-rows", &MAX_ROWS_RANGE, defaults.max_rows),
            system_sounds,
        }
    }

    /// `<colors>`: attributes for everything, and `<master>`, `<microphone>`,
    /// `<application>` children for one kind each.
    fn colors(&mut self, root: Element<'_>) -> Colors {
        let Some(element) = root.child("colors") else {
            return Colors::default();
        };
        self.check(element, COLOR_ATTRS);
        let mut colors = Colors {
            all: self.tint(element),
            ..Colors::default()
        };
        for child in element.children() {
            let slot = match child.name() {
                "master" => &mut colors.master,
                "microphone" => &mut colors.microphone,
                "application" => &mut colors.application,
                other => {
                    self.warn(child, format!("<colors> has no <{other}> element"));
                    continue;
                }
            };
            self.check(child, COLOR_ATTRS);
            *slot = self.tint(child);
        }
        colors
    }

    fn tint(&mut self, element: Element<'_>) -> Tint {
        let mut colour = |name: &str| {
            let text = element.attr(name)?;
            let parsed = Color::parse(text);
            if parsed.is_none() {
                self.warn(element, format!("{name}={text:?} is not a colour (#RRGGBB)"));
            }
            parsed
        };
        Tint {
            panel: colour("panel"),
            text: colour("text"),
            track: colour("track"),
            accent: colour("accent"),
            muted: colour("muted"),
        }
    }

    fn hud(&mut self, root: Element<'_>) -> HudConfig {
        let defaults = HudConfig::default();
        let Some(element) = root.child("hud") else {
            return defaults;
        };
        self.check(element, HUD_ATTRS);

        let position = match element.attr("position") {
            None => defaults.position,
            Some(name) => Position::from_name(name).unwrap_or_else(|| {
                self.warn(element, format!("unknown position {name:?}"));
                defaults.position
            }),
        };
        let style = match element.attr("style") {
            None => defaults.style,
            Some(name) => Style::from_name(name).unwrap_or_else(|| {
                self.warn(element, format!("unknown style {name:?}"));
                defaults.style
            }),
        };
        let font = match element.attr("font").map(str::trim) {
            None | Some("") => defaults.font.clone(),
            Some(name) => name.to_owned(),
        };
        HudConfig {
            timeout: self.number(element, "timeout", &TIMEOUT_RANGE, defaults.timeout),
            position,
            interactive: self.flag(element, "interactive", defaults.interactive),
            style,
            scale: self.number(element, "scale", &SCALE_RANGE, defaults.scale),
            radius: self.number(element, "radius", &RADIUS_RANGE, defaults.radius),
            border: self.number(element, "border", &BORDER_RANGE, defaults.border),
            opacity: self.number(element, "opacity", &OPACITY_RANGE, defaults.opacity),
            font,
            colors: Colors::default(),
            fade: self.number(element, "fade", &FADE_RANGE, defaults.fade),
        }
    }
}

/// `#RRGGBB`, or `#RRGGBBAA` when not fully opaque: what [`Color::parse`]
/// reads back.
fn color_hex(color: Color) -> String {
    if color.a == 255 {
        format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
    } else {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            color.r, color.g, color.b, color.a
        )
    }
}

/// The colours a tint sets, as attributes.
fn write_tint(w: &mut Writer, tint: &Tint) {
    for (name, color) in [
        ("panel", tint.panel),
        ("text", tint.text),
        ("track", tint.track),
        ("accent", tint.accent),
        ("muted", tint.muted),
    ] {
        if let Some(color) = color {
            w.attr(name, &color_hex(color));
        }
    }
}

/// The `<master>`, `<microphone>` and `<application>` children that set
/// anything.
fn write_kind_colors(w: &mut Writer, colors: &Colors) {
    for (name, tint) in [
        ("master", &colors.master),
        ("microphone", &colors.microphone),
        ("application", &colors.application),
    ] {
        if *tint != Tint::default() {
            w.open(name);
            write_tint(w, tint);
            w.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_SHIFT};

    use super::*;

    fn load(text: &str) -> Settings {
        Settings::parse(PathBuf::from("test.xml"), text).expect("loads")
    }

    fn doc(body: &str) -> String {
        format!("<y-coustic>{body}</y-coustic>")
    }

    #[test]
    fn the_default_configuration_is_valid() {
        let settings = load(DEFAULT_XML);
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.step, DEFAULT_STEP);
        assert_eq!(settings.hud, HudConfig::default());
        let bindings = settings.bindings();
        for action in [Action::Mute, Action::VolumeUp, Action::VolumeDown] {
            assert!(bindings.iter().any(|b| b.action == action), "{action:?}");
        }
        // The numpad keys on the master, the microphone one layer up, and the
        // bare volume keys left to Windows.
        let spelled: Vec<_> = bindings.iter().map(|b| b.hotkey.to_string()).collect();
        assert!(spelled.contains(&"Win+NumAdd".to_owned()), "{spelled:?}");
        assert!(
            spelled.contains(&"Shift+VolumeUp".to_owned()),
            "{spelled:?}"
        );
        assert!(!spelled.contains(&"VolumeUp".to_owned()), "{spelled:?}");
        assert!(settings.has_media_group());
        assert!(!settings.replaces_windows_volume());
    }

    /// `docs/examples/y-coustic.xml` is what the docs point to as the
    /// complete example: it must keep loading with no warning, and keep using
    /// everything the reader knows.
    #[test]
    fn the_full_example_is_valid_and_uses_every_feature() {
        let settings = load(include_str!("../docs/examples/y-coustic.xml"));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);

        assert_eq!(settings.step, 5);
        assert_eq!(settings.watch, Watch::default());
        assert_eq!(settings.hud, HudConfig::default());
        assert_eq!(settings.controls.keys.len(), 3);
        assert_eq!(settings.controls.keys[0].scope.layers().len(), 2);
        assert_eq!(settings.controls.keys[2].scope, Scope::Target(Target::Focused));
        assert_eq!(settings.hotkeys.len(), 5);
        assert_eq!(settings.mouse.len(), 5);
        assert_eq!(settings.mixer, MixerConfig::default());
        assert_eq!(settings.tray, TrayConfig::default());

        // 3 volume keys on the Shift layer only (plain="windows"), plus
        // 3 numpad keys x 2 layers, plus the 2 keys aimed at the focused
        // application, plus the 5 singles.
        let bindings = settings.bindings();
        assert_eq!(bindings.len(), 3 + 6 + 2 + 5);

        // Every action, both targets and the media route show up.
        for action in [
            Action::VolumeUp,
            Action::VolumeDown,
            Action::Mute,
            Action::SetVolume(30),
        ] {
            assert!(bindings.iter().any(|b| b.action == action), "{action:?}");
        }
        for target in [Target::Master, Target::Mic] {
            assert!(bindings.iter().any(|b| b.target == target), "{target:?}");
        }
        assert!(bindings.iter().any(|b| b.hook_only));
        assert!(bindings.iter().any(|b| !b.hook_only));

        // And it survives a save.
        let again = load(&settings.to_xml());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.bindings(), bindings);
    }

    #[test]
    fn an_empty_document_gives_the_defaults() {
        let settings = load("<y-coustic/>");
        assert!(settings.warnings.is_empty());
        assert_eq!(settings.step, DEFAULT_STEP);
        assert!(settings.hotkeys.is_empty());
        assert_eq!(settings.hud, HudConfig::default());
    }

    #[test]
    fn a_malformed_document_aborts_with_its_line() {
        let error = Settings::parse(
            PathBuf::from("x.xml"),
            "<y-coustic>\n<volume>\n</y-coustic>",
        )
        .unwrap_err();
        assert!(matches!(error, LoadError::Malformed(..)));
    }

    #[test]
    fn the_step_is_clamped_with_a_warning() {
        for (text, expected) in [("0", 1), ("31", 30), ("12", 12)] {
            let settings = load(&doc(&format!("<volume step=\"{text}\"/>")));
            assert_eq!(settings.step, expected);
            assert_eq!(settings.warnings.len(), usize::from(text != "12"));
        }
        let settings = load(&doc("<volume step=\"loud\"/>"));
        assert_eq!(settings.step, DEFAULT_STEP);
        assert_eq!(settings.warnings.len(), 1);
    }

    #[test]
    fn reads_hotkeys() {
        let settings = load(&doc(r#"<hotkeys>
                <hotkey key="Win+NumAdd" action="volume-up"/>
                <hotkey key="Win+F9" action="set-volume" value="30"/>
                <hotkey key="Win+F10" action="mute" target="master"/>
            </hotkeys>"#));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys.len(), 3);
        assert_eq!(settings.hotkeys[0].action, Action::VolumeUp);
        assert_eq!(settings.hotkeys[1].action, Action::SetVolume(30));
        assert_eq!(settings.hotkeys[2].target, Target::Master);
    }

    #[test]
    fn keeps_the_good_bindings_and_reports_the_bad() {
        let settings = load(&doc(r#"<hotkeys>
                <hotkey key="Win+F1" action="explode"/>
                <hotkey key="Win+Nonsense" action="mute"/>
                <hotkey key="Win+F2" action="set-volume"/>
                <hotkey key="Win+F3" action="set-volume" value="101"/>
                <hotkey key="Win+F4" action="volume"/>
                <hotkey action="mute"/>
                <hotkey key="Win+F5"/>
                <hotkey key="Win+F6" action="mute" colour="red"/>
                <hotkey key="Win+F7" action="mute" target="speakers"/>
                <volume/>
            </hotkeys>"#));
        assert_eq!(settings.hotkeys.len(), 1, "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys[0].action, Action::Mute);
        assert_eq!(settings.warnings.len(), 10, "{:?}", settings.warnings);
        assert!(settings.warnings.iter().all(|w| w.starts_with("line ")));
    }

    #[test]
    fn the_microphone_is_a_target() {
        let settings = load(&doc(r#"<hotkeys>
                <hotkey key="Win+F1" action="mute" target="mic"/>
                <hotkey key="Win+F2" action="set-volume" target="mic" value="40"/>
            </hotkeys>"#));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys.len(), 2);
        assert!(settings.hotkeys.iter().all(|b| b.target == Target::Mic));

        // And it survives a save: `target` and `value` are written back.
        let again = load(&settings.to_xml());
        assert_eq!(again.hotkeys, settings.hotkeys);
    }

    #[test]
    fn the_panel_is_an_action_for_a_hotkey_and_a_button() {
        let settings = load(&doc(r#"<hotkeys>
                <hotkey key="Win+Alt+V" action="mixer"/>
                <hotkey key="Win+Alt+B" action="mixer" target="mic"/>
            </hotkeys>
            <mouse>
                <button button="x1" action="mixer"/>
            </mouse>"#));
        assert_eq!(settings.hotkeys.len(), 2);
        assert_eq!(settings.hotkeys[0].action, Action::Mixer);
        assert_eq!(settings.hotkeys[1].target, Target::Master, "no target");
        assert_eq!(settings.mouse[0].action, Some(Action::Mixer));
        // Only the target given to the second hotkey is reported.
        assert_eq!(settings.warnings.len(), 1, "{:?}", settings.warnings);
        assert!(settings.warnings[0].contains("takes no target"));
    }

    #[test]
    fn reads_the_panel_section() {
        let settings = load(&doc(
            r#"<mixer anchor="cursor" max-rows="12" system-sounds="false"/>"#,
        ));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.mixer.anchor, Anchor::Cursor);
        assert_eq!(settings.mixer.max_rows, 12);
        assert!(!settings.mixer.system_sounds);
        assert_eq!(load(&doc("")).mixer, MixerConfig::default());
    }

    #[test]
    fn panel_mistakes_are_warnings_and_defaults() {
        let settings = load(&doc(
            r#"<mixer anchor="moon" max-rows="99" system-sounds="maybe" colour="red"/>"#,
        ));
        // Bad anchor, row count clamped, bad bool, unknown attribute.
        assert_eq!(settings.warnings.len(), 4, "{:?}", settings.warnings);
        assert_eq!(settings.mixer.anchor, Anchor::Tray);
        assert_eq!(settings.mixer.max_rows, *MAX_ROWS_RANGE.end());
        assert!(settings.mixer.system_sounds);
    }

    #[test]
    fn the_panel_section_survives_a_round_trip() {
        let settings = load(&doc(
            r#"<mixer anchor="hud" max-rows="5" system-sounds="false"/>"#,
        ));
        let again = load(&settings.to_xml());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.mixer, settings.mixer);
    }

    #[test]
    fn the_applications_are_targets() {
        let settings = load(&doc(r#"<hotkeys>
                <hotkey key="Win+F1" action="mute" target="pointed"/>
                <hotkey key="Win+F2" action="volume-up" target="focused"/>
            </hotkeys>
            <mouse>
                <wheel mod="Alt+Shift" target="pointed"/>
                <button button="middle" mod="Alt+Shift" action="mute" target="focused"/>
            </mouse>"#));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys[0].target, Target::Pointed);
        assert_eq!(settings.hotkeys[1].target, Target::Focused);
        assert_eq!(settings.mouse[0].target, Target::Pointed);
        assert_eq!(settings.mouse[1].target, Target::Focused);
    }

    #[test]
    fn an_unknown_section_is_a_warning() {
        let settings = load(&doc("<frobnicate/><tray/>"));
        assert_eq!(settings.warnings.len(), 1, "{:?}", settings.warnings);
        assert!(settings.warnings[0].contains("unknown element"));
    }

    #[test]
    fn reads_the_tray_section() {
        let settings = load(&doc(r#"<tray hover-delay="250"/>"#));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.tray.hover_delay, 250);
        assert_eq!(load(&doc("")).tray, TrayConfig::default());
        assert_eq!(load(&doc("")).tray.hover_delay, 600);
        // Zero turns the hover off, and is not a mistake.
        let off = load(&doc(r#"<tray hover-delay="0"/>"#));
        assert!(off.warnings.is_empty());
        assert_eq!(off.tray.hover_delay, 0);
    }

    #[test]
    fn tray_mistakes_are_warnings_and_defaults() {
        let settings = load(&doc(r#"<tray hover-delay="soon" colour="red"/>"#));
        assert_eq!(settings.warnings.len(), 2, "{:?}", settings.warnings);
        assert_eq!(settings.tray.hover_delay, DEFAULT_HOVER_DELAY);

        let clamped = load(&doc(r#"<tray hover-delay="99999"/>"#));
        assert_eq!(clamped.warnings.len(), 1);
        assert_eq!(clamped.tray.hover_delay, *HOVER_DELAY_RANGE.end());
    }

    #[test]
    fn tray_click_actions_are_read_and_checked() {
        let settings = load(&doc(
            r#"<tray click="hud" double-click="mute" middle-click="mute-mic"/>"#,
        ));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.tray.click, TrayAction::Hud);
        assert_eq!(settings.tray.double_click, TrayAction::Mute);
        assert_eq!(settings.tray.middle_click, TrayAction::MuteMic);

        let bad = load(&doc(r#"<tray click="dance"/>"#));
        assert_eq!(bad.warnings.len(), 1);
        assert_eq!(bad.tray.click, TrayAction::Mixer);
        assert_eq!(bad.tray.double_click, TrayAction::None);

        assert_eq!(load(&doc("")).tray.icon, appearance::Rule::Fixed);
        let rule = load(&doc(r#"<tray icon="mic-muted"/>"#));
        assert_eq!(rule.tray.icon, appearance::Rule::MicMuted);
        assert_eq!(load(&rule.to_xml()).tray.icon, appearance::Rule::MicMuted);
        let wrong = load(&doc(r#"<tray icon="sparkle"/>"#));
        assert_eq!(wrong.warnings.len(), 1);
        assert_eq!(wrong.tray.icon, appearance::Rule::Fixed);

        assert!(load(&doc("")).tray.wheel);
        assert!(!load(&doc(r#"<tray wheel="false"/>"#)).tray.wheel);
        let round = load(&doc(r#"<tray wheel="false"/>"#));
        assert!(!load(&round.to_xml()).tray.wheel);
    }

    #[test]
    fn colors_are_read_per_kind_and_survive_a_round_trip() {
        let settings = load(&doc(
            r##"<colors panel="#101010" accent="#00FF00">
                    <microphone accent="#D9534F" muted="#888"/>
                    <application text="#EEEEEE"/>
                </colors>"##,
        ));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        let colors = settings.hud.colors;
        assert_eq!(colors.all.panel, Color::parse("#101010"));
        assert_eq!(colors.all.accent, Color::parse("#00FF00"));
        assert_eq!(colors.microphone.accent, Color::parse("#D9534F"));
        assert_eq!(colors.microphone.muted, Color::parse("#888888"));
        assert_eq!(colors.application.text, Color::parse("#EEEEEE"));
        assert_eq!(colors.master, Tint::default());

        let again = load(&settings.to_xml());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.hud.colors, colors);
        assert_eq!(load(&doc("")).hud.colors, Colors::default());
    }

    #[test]
    fn colour_mistakes_are_warnings() {
        let settings = load(&doc(
            r##"<colors panel="red" shade="1"><master accent="#12"/><speaker/></colors>"##,
        ));
        assert_eq!(settings.warnings.len(), 4, "{:?}", settings.warnings);
        assert_eq!(settings.hud.colors, Colors::default());
    }

    #[test]
    fn the_hud_fade_defaults_to_none_and_is_clamped() {
        assert_eq!(load(&doc("")).hud.fade, 0);
        assert_eq!(load(&doc(r#"<hud fade="150"/>"#)).hud.fade, 150);
        let clamped = load(&doc(r#"<hud fade="99999"/>"#));
        assert_eq!(clamped.hud.fade, *FADE_RANGE.end());
        assert_eq!(clamped.warnings.len(), 1);
        let round = load(&doc(r#"<hud fade="200"/>"#));
        assert_eq!(load(&round.to_xml()).hud.fade, 200);
    }

    #[test]
    fn the_tray_section_survives_a_round_trip() {
        let settings = load(&doc(
            r#"<tray hover-delay="0" click="volume-up" double-click="mixer" middle-click="mute"/>"#,
        ));
        let again = load(&settings.to_xml());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.tray, settings.tray);
    }



    fn mouse_of(body: &str) -> Settings {
        load(&doc(&format!("<mouse>{body}</mouse>")))
    }

    #[test]
    fn reads_wheels_and_buttons() {
        let settings = mouse_of(
            r#"<wheel mod="Alt"/>
               <wheel mod="Alt+Shift" target="mic" invert="true"/>
               <wheel mod="Ctrl" axis="horizontal"/>
               <button button="middle" mod="Alt" action="mute"/>
               <button button="X1" action="set-volume" value="30" target="mic"/>"#,
        );
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        let mouse = &settings.mouse;
        assert_eq!(mouse.len(), 5);
        assert_eq!(mouse[0].input, Input::Wheel);
        assert_eq!(mouse[0].modifiers, MOD_ALT.0);
        assert_eq!(mouse[0].action, None);
        assert_eq!(mouse[1].target, Target::Mic);
        assert!(mouse[1].invert);
        assert_eq!(mouse[2].input, Input::WheelTilt);
        assert_eq!(mouse[3].input, Input::Middle);
        assert_eq!(mouse[3].action, Some(Action::Mute));
        assert_eq!(mouse[4].input, Input::X1);
        assert_eq!(mouse[4].modifiers, 0);
        assert_eq!(mouse[4].action, Some(Action::SetVolume(30)));
    }

    #[test]
    fn a_bare_wheel_or_main_button_is_refused() {
        let settings = mouse_of(
            r#"<wheel/>
               <wheel mod="none"/>
               <button button="left" action="mute"/>
               <button button="middle" mod="none" action="mute"/>"#,
        );
        assert!(settings.mouse.is_empty());
        assert_eq!(settings.warnings.len(), 4, "{:?}", settings.warnings);
        assert!(settings.warnings.iter().all(|w| w.contains("needs a mod")));
    }

    #[test]
    fn mouse_mistakes_are_warnings_and_the_rest_loads() {
        let settings = mouse_of(
            r#"<wheel mod="Hyper"/>
               <wheel mod="Alt" axis="diagonal"/>
               <wheel mod="Alt" target="speakers"/>
               <button mod="Alt" action="mute"/>
               <button button="wheel" mod="Alt" action="mute"/>
               <button button="middle" mod="Alt" action="volume"/>
               <button button="middle" mod="Alt" action="mixer"/>
               <button button="middle" mod="Alt"/>
               <knob/>
               <wheel mod="Shift"/>"#,
        );
        // The mixer button is valid now; the rest of its neighbours are not.
        assert_eq!(settings.mouse.len(), 2);
        assert_eq!(settings.mouse[1].modifiers, MOD_SHIFT.0);
        assert_eq!(settings.warnings.len(), 8, "{:?}", settings.warnings);
    }

    #[test]
    fn a_second_binding_on_the_same_input_is_ignored() {
        let settings = mouse_of(
            r#"<wheel mod="Alt"/>
               <wheel mod="alt" target="mic"/>
               <button button="middle" mod="Alt" action="mute"/>"#,
        );
        assert_eq!(settings.mouse.len(), 2);
        assert_eq!(settings.mouse[0].target, Target::Master);
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("earlier binding"));
    }

    #[test]
    fn the_mouse_section_survives_a_round_trip() {
        let settings = mouse_of(
            r#"<wheel mod="Alt"/>
               <wheel mod="Alt+Shift" target="mic" invert="true"/>
               <wheel mod="Ctrl" axis="horizontal"/>
               <button button="middle" mod="Alt" action="mute" target="mic"/>
               <button button="x2" action="set-volume" value="0"/>"#,
        );
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        let again = load(&settings.to_xml());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.mouse, settings.mouse);
    }
    #[test]
    fn reads_controls_and_expands_them() {
        let settings = load(&doc(r#"<controls>
                <keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="replace">
                    <layer mod="none" target="master"/>
                    <layer mod="Shift" target="mic"/>
                </keys>
                <keys up="F11" mute="F10" target="mic"/>
            </controls>"#));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.controls.keys.len(), 2);
        assert_eq!(settings.controls.keys[0].scope.layers().len(), 2);
        assert_eq!(settings.controls.keys[1].scope, Scope::Target(Target::Mic));
        assert!(settings.hotkeys.is_empty(), "the singles stay as written");
        // 3 media keys x 2 layers, 2 keys x the one target.
        assert_eq!(settings.bindings().len(), 8);
        assert!(settings.replaces_windows_volume());
    }

    #[test]
    fn reports_what_is_wrong_in_controls_and_keeps_the_rest() {
        let settings = load(&doc(r#"<controls>
                <keys up="Win+Nonsense" down="F12"/>
                <keys/>
                <keys up="F1" plain="both" target="mic"/>
                <keys up="F2" colour="red" target="mic"/>
                <keys up="F3">
                    <layer mod="Hyper"/>
                    <layer mod="Ctrl" target="speakers"/>
                    <layer mod="Ctrl"/>
                    <frobnicate/>
                </keys>
                <layer mod="Ctrl"/>
                <frobnicate/>
            </controls>"#));
        // Bad key, a group that controls nothing, empty group, bad plain,
        // unknown attribute, bad mod, bad target, unknown child of a group, a
        // layer outside a group, unknown child of <controls>.
        assert_eq!(settings.warnings.len(), 10, "{:?}", settings.warnings);
        assert_eq!(settings.controls.keys.len(), 4);
        assert_eq!(settings.controls.keys[3].scope.layers().len(), 1);
    }

    #[test]
    fn a_layer_outside_a_group_is_warned_about_and_kept_as_a_comment() {
        let original = load(&doc(r#"<controls>
                <keys up="F1" target="master"/>
                <layer mod="Shift" target="mic"/>
            </controls>"#));
        assert_eq!(original.warnings.len(), 1, "{:?}", original.warnings);
        assert!(original.warnings[0].contains("belongs inside a <keys>"));
        // The group is not crossed with it.
        assert_eq!(original.bindings().len(), 1);

        let text = original.to_xml();
        assert!(text.contains(r#"<!-- <layer mod="Shift" target="mic"/> -->"#), "{text}");
        let again = load(&text);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    }

    #[test]
    fn a_group_with_layers_and_a_target_follows_its_layers() {
        let settings = load(&doc(r#"<controls>
                <keys up="F1" target="mic"><layer mod="Ctrl" target="focused"/></keys>
            </controls>"#));
        assert_eq!(settings.warnings.len(), 1, "{:?}", settings.warnings);
        let bindings = settings.bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].target, Target::Focused);
        // Written back as what it means: its layers.
        let text = settings.to_xml();
        assert!(!text.contains(r#"target="mic""#), "{text}");
    }

    #[test]
    fn each_group_keeps_its_own_layers_through_a_save() {
        let original = load(&doc(r#"<controls>
                <keys up="VolumeUp" plain="replace">
                    <!-- master -->
                    <layer mod="none" target="master"/>
                    <layer mod="Shift" target="mic"/>
                    <!-- after the layers -->
                </keys>

                <!-- a mute of its own -->
                <keys mute="Win+Alt+M" target="mic"/>
            </controls>"#));
        assert!(original.warnings.is_empty(), "{:?}", original.warnings);
        assert_eq!(original.kept.keys[0].layers[0].lead[0].text, "master");
        assert_eq!(original.kept.keys[0].tail[0].text, "after the layers");
        assert_eq!(original.kept.keys[1].part.lead[0].text, "a mute of its own");

        let text = original.to_xml();
        let again = load(&text);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(values(&again), values(&original));
        assert_eq!(again.kept.comment_count(), original.kept.comment_count());
        assert_eq!(again.to_xml(), text);
    }

    #[test]
    fn a_layer_on_an_application_expands_like_any_other() {
        let settings = load(&doc(r#"<controls>
                <keys up="F1"><layer mod="Ctrl" target="focused"/></keys>
            </controls>"#));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        let bindings = settings.bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].target, Target::Focused);
    }

    #[test]
    fn a_collision_between_a_layer_and_a_hotkey_is_reported_at_load() {
        let settings = load(&doc(r#"<controls>
                <keys up="F1"><layer mod="Shift" target="mic"/></keys>
            </controls>
            <hotkeys><hotkey key="Shift+F1" action="mute"/></hotkeys>"#));
        assert_eq!(settings.warnings.len(), 1, "{:?}", settings.warnings);
        assert!(settings.warnings[0].contains("Shift+F1"));
        assert_eq!(settings.bindings().len(), 1);
    }

    #[test]
    fn the_tray_toggle_flips_plain_and_survives_a_save() {
        let mut settings = load(DEFAULT_XML);
        assert!(!settings.replaces_windows_volume());

        settings.set_replace_windows_volume(true);
        assert!(settings.replaces_windows_volume());
        // The numpad group is not a media group and is left as it was.
        assert_eq!(settings.controls.keys[1].plain, Plain::Windows);
        assert!(
            settings
                .bindings()
                .iter()
                .any(|b| b.hotkey.to_string() == "VolumeUp")
        );

        let again = load(&settings.to_xml());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.controls, settings.controls);
        assert!(again.replaces_windows_volume());
        assert_eq!(again.bindings(), settings.bindings());

        settings.set_replace_windows_volume(false);
        assert!(
            !settings
                .bindings()
                .iter()
                .any(|b| b.hotkey.to_string() == "VolumeUp")
        );
    }

    #[test]
    fn a_single_hotkey_on_a_media_key_goes_through_the_hook() {
        let settings = load(&doc(r#"<hotkeys>
                <hotkey key="Win+VolumeMute" action="mute"/>
                <hotkey key="Win+F1" action="mute"/>
            </hotkeys>"#));
        assert!(settings.hotkeys[0].hook_only);
        assert!(!settings.hotkeys[1].hook_only);
    }

    #[test]
    fn reads_the_watch_section() {
        let settings = load("<y-coustic/>");
        assert_eq!(settings.watch, Watch::default());
        assert_eq!(settings.watch.master, WatchMode::Off);
        assert_eq!(settings.watch.microphone, WatchMode::Hud);

        let settings = load(&doc("<watch master=\"tray\" microphone=\"off\"/>"));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.watch.master, WatchMode::Tray);
        assert_eq!(settings.watch.microphone, WatchMode::Off);

        let settings = load(&doc("<watch master=\"loud\" mic=\"hud\"/>"));
        assert_eq!(settings.warnings.len(), 2, "{:?}", settings.warnings);
        assert_eq!(settings.watch, Watch::default());
    }

    #[test]
    fn a_second_section_is_reported() {
        let settings = load(&doc("<volume step=\"3\"/><volume step=\"9\"/>"));
        assert_eq!(settings.step, 3);
        assert_eq!(settings.warnings.len(), 1);
    }


    #[test]
    fn the_hud_is_interactive_unless_told_otherwise() {
        assert!(load(&doc("")).hud.interactive);
        assert!(load(&doc("<hud/>")).hud.interactive);
        let off = load(&doc(r#"<hud interactive="false"/>"#));
        assert!(off.warnings.is_empty(), "{:?}", off.warnings);
        assert!(!off.hud.interactive);

        let bad = load(&doc(r#"<hud interactive="sometimes"/>"#));
        assert_eq!(bad.warnings.len(), 1);
        assert!(bad.hud.interactive, "the default");

        let again = load(&off.to_xml());
        assert!(!again.hud.interactive);
    }
    #[test]
    fn reads_the_overlay() {
        let settings = load(&doc("<hud position=\"top-center\" timeout=\"800\"/>"));
        assert!(settings.warnings.is_empty());
        assert_eq!(settings.hud.position, Position::TopCenter);
        assert_eq!(settings.hud.timeout, 800);

        let settings = load(&doc("<hud position=\"nowhere\" timeout=\"5\"/>"));
        assert_eq!(
            settings.hud,
            HudConfig {
                timeout: 100,
                ..HudConfig::default()
            }
        );
        assert_eq!(settings.warnings.len(), 2);
    }

    #[test]
    fn reads_the_look_of_the_overlay() {
        let settings = load(&doc(
            r#"<hud style="ring" position="follow-cursor" scale="150" radius="0"
                border="2" opacity="80" font="Consolas"/>"#,
        ));
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        let hud = settings.hud;
        assert_eq!(hud.style, Style::Ring);
        assert_eq!(hud.position, Position::FollowCursor);
        assert_eq!(
            (hud.scale, hud.radius, hud.border, hud.opacity),
            (150, 0, 2, 80)
        );
        assert_eq!(hud.font, "Consolas");

        // Out of range is clamped, an unknown style keeps the default; each warns.
        let settings = load(&doc(r#"<hud style="blob" scale="900" opacity="0"/>"#));
        assert_eq!(settings.hud.style, Style::Bar);
        assert_eq!(settings.hud.scale, 300);
        assert_eq!(settings.hud.opacity, 10);
        assert_eq!(settings.warnings.len(), 3);
    }

    #[test]
    fn every_style_and_position_has_a_name_that_reads_back() {
        for style in [Style::Bar, Style::VBar, Style::Ring, Style::Text, Style::Compact] {
            assert_eq!(Style::from_name(style.name()), Some(style));
        }
        for position in [
            Position::TopLeft,
            Position::TopCenter,
            Position::TopRight,
            Position::CenterLeft,
            Position::Center,
            Position::CenterRight,
            Position::BottomLeft,
            Position::BottomCenter,
            Position::BottomRight,
            Position::FollowCursor,
        ] {
            assert_eq!(Position::from_name(position.name()), Some(position));
        }
    }


    /// The settings without what is only about the file's text: what two
    /// loads of the same meaning must share.
    fn values(settings: &Settings) -> Settings {
        let mut values = settings.clone();
        values.kept = Kept::default();
        values.warnings.clear();
        values
    }

    #[test]
    fn the_shipped_file_survives_a_load_and_a_save_with_its_comments() {
        let shipped = include_str!("../rsc/y-coustic.xml");
        let original = Settings::parse(PathBuf::from("test.xml"), shipped).expect("loads");
        assert!(original.warnings.is_empty(), "{:?}", original.warnings);
        assert!(original.kept.comment_count() > 5);

        let text = original.to_xml();
        let again = Settings::parse(PathBuf::from("test.xml"), &text).expect("loads");
        assert!(again.warnings.is_empty(), "{:?}\n{text}", again.warnings);
        assert_eq!(values(&again), values(&original));
        assert_eq!(again.kept.comment_count(), original.kept.comment_count());
        let comment = |text: &str| text.matches("<!--").count();
        assert_eq!(comment(&text), comment(shipped));
    }

    #[test]
    fn saving_over_a_file_keeps_a_backup_and_leaves_no_temporary() {
        let directory = std::env::temp_dir().join(format!("y-coustic-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("y-coustic.xml");
        std::fs::write(&path, "<y-coustic><volume step=\"3\"/></y-coustic>").unwrap();

        let settings = load(&doc(r#"<volume step="9"/>"#));
        settings.write_safely(&path, true).unwrap();

        let saved = Settings::load_from(path.clone()).unwrap();
        assert_eq!(saved.step, 9);
        let backup = std::fs::read_to_string(directory.join("y-coustic.xml.bak")).unwrap();
        assert!(backup.contains("step=\"3\""));
        assert!(!directory.join("y-coustic.xml.tmp").exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn saving_twice_gives_the_same_bytes() {
        let shipped = include_str!("../rsc/y-coustic.xml");
        let first = Settings::parse(PathBuf::from("test.xml"), shipped)
            .expect("loads")
            .to_xml();
        let second = Settings::parse(PathBuf::from("test.xml"), &first)
            .expect("loads")
            .to_xml();
        assert_eq!(first, second);
    }

    #[test]
    fn unknown_attributes_and_unusable_elements_survive_a_save() {
        let original = load(&doc(
            r#"<hotkeys>
                <hotkey key="Win+F1" action="mute" colour="red"/>
                <hotkey key="Win+F2" action="frobnicate"/>
                <hotkey key="Win+F3" action="mute"/>
            </hotkeys>
            <frob size="3"/>"#,
        ));
        assert_eq!(original.hotkeys.len(), 2);
        assert_eq!(original.warnings.len(), 3, "{:?}", original.warnings);

        let text = original.to_xml();
        assert!(text.contains(r#"colour="red""#), "{text}");
        assert!(
            text.contains(r#"<!-- <hotkey key="Win+F2" action="frobnicate"/> -->"#),
            "{text}"
        );
        assert!(text.contains(r#"<!-- <frob size="3"/> -->"#), "{text}");

        // The rejected elements are comments now, nothing more to lose or to
        // warn about; the unknown attribute is kept, and still warned.
        let again = load(&text);
        assert_eq!(again.warnings.len(), 1, "{:?}", again.warnings);
        assert_eq!(values(&again), values(&original));
        assert_eq!(again.to_xml(), text);
    }

    #[test]
    fn comments_in_each_list_stay_with_their_entry_and_the_blank_lines_stay() {
        let original = load(&doc(
            r#"<controls>
                <!-- the keys -->
                <keys up="Win+NumAdd" target="master"/>

                <!-- the second group -->
                <keys up="Win+NumSub" target="mic"/>
                <!-- after the last group -->
            </controls>
            <hotkeys>
                <!-- one -->
                <hotkey key="Win+F1" action="mute"/>

                <hotkey key="Win+F2" action="mute"/>
            </hotkeys>
            <mouse>
                <!-- the wheel -->
                <wheel mod="Alt"/>
            </mouse>"#,
        ));
        assert!(original.warnings.is_empty(), "{:?}", original.warnings);
        assert_eq!(original.kept.keys[0].part.lead[0].text, "the keys");
        assert_eq!(original.kept.keys[1].part.lead[0].text, "the second group");
        assert!(original.kept.keys[1].part.lead[0].blank_before);
        assert_eq!(original.kept.hotkeys[0].lead[0].text, "one");
        assert!(original.kept.hotkeys[1].blank_before);
        assert_eq!(original.kept.mouse[0].lead[0].text, "the wheel");
        assert_eq!(
            original.kept.section("controls").unwrap().tail[0].text,
            "after the last group"
        );

        let text = original.to_xml();
        let again = load(&text);
        assert_eq!(again.kept.comment_count(), original.kept.comment_count());
        assert_eq!(again.to_xml(), text);
        let one = text.find("<!-- one -->").unwrap();
        let first = text.find(r#"key="Win+F1""#).unwrap();
        assert!(one < first);
    }


    #[test]
    fn round_trips_through_to_xml() {
        let original = load(&doc(r#"<volume step="8"/>
            <hotkeys>
                <hotkey key="Win+Alt+NumAdd" action="volume-up"/>
                <hotkey key="Ctrl+F9" action="set-volume" value="45"/>
                <hotkey key="Win+Numpad0" action="mute"/>
            </hotkeys>
            <watch master="hud" microphone="tray"/>
            <hud style="vbar" position="top-right" timeout="2000" scale="120"
                border="1" opacity="90" font="Cascadia &amp; Co"/>"#));
        assert!(original.warnings.is_empty(), "{:?}", original.warnings);

        let text = original.to_xml();
        let again = load(&text);
        assert!(again.warnings.is_empty(), "{:?}\n{text}", again.warnings);
        assert_eq!(values(&again), values(&original));
    }

    #[test]
    fn saves_and_reloads_through_a_file() {
        let path = std::env::temp_dir()
            .join("y-coustic-config-test")
            .join(FILE_NAME);
        let mut settings = Settings::empty(path.clone());
        settings.step = 7;
        settings.save().expect("saves");

        let loaded = Settings::load_from(path.clone()).expect("loads");
        assert_eq!(loaded.step, 7);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        assert!(matches!(
            Settings::load_from(path),
            Err(LoadError::Missing(_))
        ));
    }

    #[test]
    fn expands_environment_variables_in_paths() {
        assert_eq!(expand("100%"), "100%");
        assert_eq!(
            expand("%Y_COUSTIC_SURELY_UNSET%\\a"),
            "%Y_COUSTIC_SURELY_UNSET%\\a"
        );
        // SAFETY: tests in this crate touch no other variable.
        unsafe { std::env::set_var("Y_COUSTIC_TEST_DIR", "C:\\x") };
        assert_eq!(expand("%Y_COUSTIC_TEST_DIR%\\a.xml"), "C:\\x\\a.xml");
    }
}
