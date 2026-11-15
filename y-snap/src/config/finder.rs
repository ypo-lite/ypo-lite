//! The `<finder>` section: the mouse finder's styles and their triggers.
//!
//! The finder shows a short animation around the pointer so that a lost one
//! can be found again, mostly on a multi-monitor desktop. The section holds
//! what is global (the guard, the exclude list, the defaults every style
//! inherits) and a list of named `<style>` elements. A style is a look (shape,
//! size, colour...) plus the automatic triggers that play it: `<tap>` (a
//! modifier tapped several times) and `<shake>` (the pointer shaken, with or
//! without a modifier held). The `find-mouse` action plays a style too, the
//! first one or the one marked `default="true"`.
//!
//! A missing `<finder>` section, or one without styles, means no automatic
//! trigger at all: only the action is available, with the built-in look. The
//! defaults below are the values tuned live with the Phase 0 probes
//! (`docs/design/mouse-finder-plan.md`); a few — noted on the field — were
//! never exercised and are first guesses.
//!
//! Severity follows the rest of the file: a value that is not a number, a
//! colour or a known name, or that falls outside its range, is a warning and
//! the default is used; a trigger or a style that cannot work at all (no key,
//! no name) is skipped with a warning; nothing here is fatal.

use std::fmt::{self, Write as _};
use std::ops::RangeInclusive;

use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
use y_core::overlay::Color;
use y_core::xml::Element;

use super::{Reader, color_hex, escape_attr, format_modifiers, parse_modifiers};

/// How an indicator draws itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    /// Rings converging on the pointer over a soft halo.
    #[default]
    Beacon,
    /// The monitor dims, a clear circle closes in on the pointer.
    Spotlight,
    /// Lines across the whole monitor, with a ring at the pointer.
    Crosshair,
    /// A pulsing border on the pointer's monitor and a map of all monitors.
    Monitor,
    /// A persistent ring and glow that follow the pointer.
    Halo,
    /// A lens showing the screen behind the pointer as a negative.
    Negative,
    /// A ring painted black or white against what is behind it.
    Contrast,
}

impl Shape {
    pub const ALL: [Self; 7] = [
        Self::Beacon,
        Self::Spotlight,
        Self::Crosshair,
        Self::Monitor,
        Self::Halo,
        Self::Negative,
        Self::Contrast,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Beacon => "beacon",
            Self::Spotlight => "spotlight",
            Self::Crosshair => "crosshair",
            Self::Monitor => "monitor",
            Self::Halo => "halo",
            Self::Negative => "negative",
            Self::Contrast => "contrast",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shape| shape.name() == name.trim())
    }

    /// Whether the shape stays until dismissed or timed out, instead of
    /// playing once for the style's duration.
    pub fn is_persistent(self) -> bool {
        matches!(self, Self::Halo | Self::Negative | Self::Contrast)
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a style does besides showing the indicator.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Recall {
    /// Only show where the pointer is.
    #[default]
    None,
    /// Also move the pointer to the middle of the active window.
    Center,
}

impl Recall {
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Center => "center",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim() {
            "none" => Some(Self::None),
            "center" => Some(Self::Center),
            _ => None,
        }
    }
}

/// The attributes `<finder>` offers as defaults and every `<style>` may
/// override.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub shape: Shape,
    /// How long a non-persistent indicator plays, in milliseconds.
    pub duration_ms: u32,
    /// Base radius, in dips.
    pub size: u32,
    pub color: Color,
    /// Darken the colour on a light backdrop and lighten it on a dark one,
    /// so it never melts into what is behind it.
    pub auto_color: bool,
    /// How dark the spotlight makes the screen, in percent.
    pub dim: u32,
    pub recall: Recall,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            duration_ms: 1000,
            size: 160,
            color: Color::rgb(0xFF, 0xB0, 0x00),
            auto_color: false,
            dim: 60,
            recall: Recall::None,
        }
    }
}

/// A modifier a [`Tap`] listens to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TapKey {
    Ctrl,
    Shift,
    Alt,
    Win,
}

impl TapKey {
    pub const ALL: [Self; 4] = [Self::Ctrl, Self::Shift, Self::Alt, Self::Win];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
            Self::Win => "win",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.trim().to_ascii_lowercase();
        match lower.as_str() {
            "windows" | "super" | "meta" => Some(Self::Win),
            "control" => Some(Self::Ctrl),
            _ => Self::ALL.into_iter().find(|key| key.name() == lower),
        }
    }
}

/// Which physical key of a pair counts for a [`Tap`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Side {
    #[default]
    Any,
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim() {
            "any" => Some(Self::Any),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// A modifier tapped `count` times, quickly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tap {
    pub key: TapKey,
    pub side: Side,
    pub count: u32,
    /// A press held longer than this is not a tap, and forgets the previous
    /// one: "press, hold, press again" must not count.
    pub max_hold_ms: u32,
    /// Two taps count together when their presses are at most this far apart.
    pub max_gap_ms: u32,
}

impl Tap {
    pub fn new(key: TapKey) -> Self {
        Self {
            key,
            side: Side::Any,
            count: 2,
            max_hold_ms: 200,
            max_gap_ms: 400,
        }
    }
}

/// A kind of shake a [`Shake`] trigger can recognise.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShakeShape {
    /// Back and forth, mostly sideways.
    Horizontal,
    /// Up and down.
    Vertical,
    /// Round and round.
    Circle,
}

impl ShakeShape {
    pub const ALL: [Self; 3] = [Self::Horizontal, Self::Vertical, Self::Circle];

    pub fn name(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
            Self::Circle => "circle",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shape| shape.name() == name.trim())
    }
}

/// Shaking the mouse, optionally with modifiers held. Distances are in pixels
/// and speeds in pixels per second, physical and not scaled by the monitor: the
/// values were tuned in physical pixels on one machine, so treat them as
/// starting points.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shake {
    /// Held while shaking. Read, never swallowed: with a modifier down the
    /// user already says "I am looking for something", so the defaults relax.
    pub modifiers: HOT_KEY_MODIFIERS,
    /// The kinds of shake that trigger, in the order of [`ShakeShape::ALL`].
    pub shapes: [bool; 3],
    /// How far back the pointer is looked at, in milliseconds.
    pub window_ms: u32,
    /// Travel back from the last extreme that counts as a reversal.
    pub segment: u32,
    /// Mean speed below which nothing is a shake.
    pub speed: u32,
    /// Direction reversals a horizontal or vertical shake needs.
    pub reversals: u32,
    /// Travel along the main axis over travel along the other one, for a
    /// shake to count as horizontal (or vertical).
    pub dominance: f32,
    /// Smooth turning a circle needs, in degrees.
    pub circle_degrees: u32,
    /// Mean speed below which a circle does not count. Its own value because
    /// a circle is naturally slower than a shake. **Not tuned yet.**
    pub circle_speed: u32,
    /// After a detection, everything is ignored for this long.
    pub cooldown_ms: u32,
}

impl Shake {
    /// The defaults for a shake with `modifiers` held: strict without one,
    /// relaxed (the tuned Ctrl profile) with one.
    pub fn new(modifiers: HOT_KEY_MODIFIERS) -> Self {
        let held = modifiers.0 != 0;
        Self {
            modifiers,
            shapes: [true; 3],
            window_ms: 700,
            segment: 120,
            speed: if held { 1200 } else { 3000 },
            reversals: if held { 2 } else { 3 },
            dominance: 1.5,
            circle_degrees: 450,
            circle_speed: 1500,
            cooldown_ms: 1000,
        }
    }

    pub fn has(&self, shape: ShakeShape) -> bool {
        self.shapes[shape as usize]
    }

    pub fn all_shapes(&self) -> bool {
        self.shapes.iter().all(|on| *on)
    }
}

/// A named look and the triggers that play it.
#[derive(Clone, PartialEq, Debug)]
pub struct Style {
    pub name: String,
    pub look: Look,
    pub taps: Vec<Tap>,
    pub shakes: Vec<Shake>,
}

impl Style {
    pub fn new(name: impl Into<String>, look: Look) -> Self {
        Self {
            name: name.into(),
            look,
            taps: Vec::new(),
            shakes: Vec::new(),
        }
    }
}

/// The `<finder>` section.
#[derive(Clone, PartialEq, Debug)]
pub struct Finder {
    /// What every style starts from.
    pub look: Look,
    /// Keep the automatic triggers quiet while a full-screen window (a game in
    /// its menus, a video) is on the pointer's monitor. Applies to the
    /// automatic triggers only: an explicit `find-mouse` always shows. A
    /// confined pointer and the exclude list apply whatever this says.
    pub guard: bool,
    /// Executable names (no folder, no `.exe`, lower case) that never trigger
    /// the finder automatically, whatever the style.
    pub exclude: Vec<String>,
    pub styles: Vec<Style>,
    /// Index into `styles` of the one `find-mouse` plays when it names none.
    pub default_style: usize,
}

impl Default for Finder {
    fn default() -> Self {
        Self {
            look: Look::default(),
            guard: true,
            exclude: Vec::new(),
            styles: Vec::new(),
            default_style: 0,
        }
    }
}

impl Finder {
    /// The style `name` asks for; the default one when `name` is `None` or
    /// unknown; the built-in look when there is no style at all.
    pub fn style(&self, name: Option<&str>) -> Style {
        let named = name.and_then(|name| self.styles.iter().find(|style| style.name == name));
        named
            .or_else(|| self.styles.get(self.default_style))
            .cloned()
            .unwrap_or_else(|| Style::new("default", self.look))
    }
}

/// The accepted range of each number, shared by the reader and by a settings
/// app so that both agree on what is allowed.
pub mod range {
    use std::ops::RangeInclusive;

    pub const DURATION_MS: RangeInclusive<u32> = 300..=5000;
    pub const SIZE: RangeInclusive<u32> = 40..=600;
    pub const DIM: RangeInclusive<u32> = 0..=100;
    pub const TAP_COUNT: RangeInclusive<u32> = 2..=4;
    pub const TAP_HOLD_MS: RangeInclusive<u32> = 100..=400;
    pub const TAP_GAP_MS: RangeInclusive<u32> = 200..=700;
    pub const SHAKE_WINDOW_MS: RangeInclusive<u32> = 300..=1500;
    pub const SHAKE_SEGMENT: RangeInclusive<u32> = 40..=300;
    pub const SHAKE_SPEED: RangeInclusive<u32> = 400..=8000;
    pub const SHAKE_REVERSALS: RangeInclusive<u32> = 1..=6;
    pub const SHAKE_DOMINANCE: RangeInclusive<f32> = 1.0..=4.0;
    pub const SHAKE_CIRCLE_DEGREES: RangeInclusive<u32> = 270..=900;
    pub const SHAKE_CIRCLE_SPEED: RangeInclusive<u32> = 400..=6000;
    pub const SHAKE_COOLDOWN_MS: RangeInclusive<u32> = 300..=3000;
}

const FINDER: &[&str] = &["shape", "duration", "size", "color", "auto-color", "dim", "recall", "guard"];
const STYLE: &[&str] = &[
    "shape", "duration", "size", "color", "auto-color", "dim", "recall", "name", "default",
];
const TAP: &[&str] = &["key", "side", "count", "hold", "gap"];
const SHAKE: &[&str] = &[
    "mod", "shapes", "window", "segment", "speed", "reversals", "dominance", "degrees",
    "circle-speed", "cooldown",
];

/// `Name.EXE` → `name`: how `<exclude process="…">` and the running process
/// are compared.
pub fn normalize_process(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

/// Which triggers are already taken, so that a second identical one — in the
/// same style or another — is reported and ignored instead of fighting the
/// first.
#[derive(Default)]
struct Taken {
    taps: Vec<(TapKey, Side, u32)>,
    shakes: Vec<(u32, ShakeShape)>,
}

impl Reader {
    pub(super) fn finder(&mut self, root: Element<'_>) -> Finder {
        let Some(section) = root.child("finder") else {
            return Finder::default();
        };
        self.check(section, FINDER);

        let mut finder = Finder::default();
        self.look(section, &mut finder.look);
        self.flag(section, "guard", &mut finder.guard);

        let mut taken = Taken::default();
        let mut default_seen = false;
        for element in section.children() {
            match element.name() {
                "exclude" => {
                    self.check(element, &["process"]);
                    match element.attr("process").map(normalize_process) {
                        Some(process) if !process.is_empty() => {
                            if !finder.exclude.contains(&process) {
                                finder.exclude.push(process);
                            }
                        }
                        _ => self.warn(element, "<exclude> needs a process name"),
                    }
                }
                "style" => {
                    let Some(style) = self.style(element, &finder, &mut taken) else {
                        continue;
                    };
                    let mut wanted = false;
                    self.flag(element, "default", &mut wanted);
                    if wanted {
                        if default_seen {
                            self.warn(element, "a second default style; the first one stays");
                        } else {
                            default_seen = true;
                            finder.default_style = finder.styles.len();
                        }
                    }
                    finder.styles.push(style);
                }
                name => self.warn(element, format!("<{name}> does not belong in <finder>")),
            }
        }

        finder
    }

    /// Reads one `<style>`; `None` (with a warning) when it cannot exist.
    fn style(&mut self, element: Element<'_>, finder: &Finder, taken: &mut Taken) -> Option<Style> {
        self.check(element, STYLE);
        let name = element.attr("name").map(str::trim).unwrap_or("");
        if name.is_empty() {
            self.warn(element, "a <style> needs a name");
            return None;
        }
        if finder.styles.iter().any(|style| style.name == name) {
            self.warn(element, format!("a second style named {name:?} is ignored"));
            return None;
        }

        let mut style = Style::new(name, finder.look);
        self.look(element, &mut style.look);

        for child in element.children() {
            match child.name() {
                "tap" => {
                    if let Some(tap) = self.tap(child, taken) {
                        style.taps.push(tap);
                    }
                }
                "shake" => {
                    if let Some(shake) = self.shake(child, taken) {
                        style.shakes.push(shake);
                    }
                }
                other => self.warn(child, format!("<{other}> does not belong in <style>")),
            }
        }
        Some(style)
    }

    fn look(&mut self, element: Element<'_>, look: &mut Look) {
        if let Some(text) = element.attr("shape") {
            match Shape::from_name(text) {
                Some(shape) => look.shape = shape,
                None => {
                    let known: Vec<_> = Shape::ALL.iter().map(|s| s.name()).collect();
                    self.warn(element, format!("unknown shape {text:?} ({})", known.join(", ")));
                }
            }
        }
        self.ranged(element, "duration", &mut look.duration_ms, range::DURATION_MS);
        self.ranged(element, "size", &mut look.size, range::SIZE);
        self.ranged(element, "dim", &mut look.dim, range::DIM);
        self.flag(element, "auto-color", &mut look.auto_color);
        if let Some(text) = element.attr("color") {
            match Color::parse(text) {
                Some(color) => look.color = color,
                None => self.warn(element, format!("color {text:?} is not a colour (#RRGGBB)")),
            }
        }
        if let Some(text) = element.attr("recall") {
            match Recall::from_name(text) {
                Some(recall) => look.recall = recall,
                None => self.warn(element, format!("unknown recall {text:?} (none or center)")),
            }
        }
    }

    fn tap(&mut self, element: Element<'_>, taken: &mut Taken) -> Option<Tap> {
        self.check(element, TAP);
        let Some(text) = element.attr("key") else {
            self.warn(element, "a <tap> needs a key (ctrl, shift, alt or win)");
            return None;
        };
        let Some(key) = TapKey::from_name(text) else {
            self.warn(element, format!("unknown tap key {text:?} (ctrl, shift, alt or win)"));
            return None;
        };

        let mut tap = Tap::new(key);
        if let Some(text) = element.attr("side") {
            match Side::from_name(text) {
                Some(side) => tap.side = side,
                None => self.warn(element, format!("unknown side {text:?} (any, left or right)")),
            }
        }
        self.ranged(element, "count", &mut tap.count, range::TAP_COUNT);
        self.ranged(element, "hold", &mut tap.max_hold_ms, range::TAP_HOLD_MS);
        self.ranged(element, "gap", &mut tap.max_gap_ms, range::TAP_GAP_MS);

        let id = (tap.key, tap.side, tap.count);
        if taken.taps.contains(&id) {
            self.warn(element, "this tap is already used by another trigger; ignored");
            return None;
        }
        taken.taps.push(id);
        Some(tap)
    }

    fn shake(&mut self, element: Element<'_>, taken: &mut Taken) -> Option<Shake> {
        self.check(element, SHAKE);
        let modifiers = match element.attr("mod") {
            None => HOT_KEY_MODIFIERS(0),
            Some(text) => match parse_modifiers(text) {
                Some(modifiers) => modifiers,
                None => {
                    self.warn(element, format!("{text:?} is not a set of modifiers"));
                    return None;
                }
            },
        };

        // Defaults depend on the modifiers, so they are known before the rest.
        let mut shake = Shake::new(modifiers);
        if let Some(text) = element.attr("shapes") {
            match parse_shapes(text) {
                Some(shapes) => shake.shapes = shapes,
                None => self.warn(
                    element,
                    format!("shapes {text:?} is not any or a list of horizontal, vertical, circle"),
                ),
            }
        }
        self.ranged(element, "window", &mut shake.window_ms, range::SHAKE_WINDOW_MS);
        self.ranged(element, "segment", &mut shake.segment, range::SHAKE_SEGMENT);
        self.ranged(element, "speed", &mut shake.speed, range::SHAKE_SPEED);
        self.ranged(element, "reversals", &mut shake.reversals, range::SHAKE_REVERSALS);
        self.real(element, "dominance", &mut shake.dominance, range::SHAKE_DOMINANCE);
        self.ranged(element, "degrees", &mut shake.circle_degrees, range::SHAKE_CIRCLE_DEGREES);
        self.ranged(element, "circle-speed", &mut shake.circle_speed, range::SHAKE_CIRCLE_SPEED);
        self.ranged(element, "cooldown", &mut shake.cooldown_ms, range::SHAKE_COOLDOWN_MS);

        // A kind of shake with the same modifiers belongs to one trigger only.
        let mut clashed = false;
        for shape in ShakeShape::ALL {
            if shake.has(shape) {
                if taken.shakes.contains(&(modifiers.0, shape)) {
                    shake.shapes[shape as usize] = false;
                    clashed = true;
                } else {
                    taken.shakes.push((modifiers.0, shape));
                }
            }
        }
        if clashed {
            self.warn(element, "a shape already used by another trigger with the same modifiers is ignored");
        }
        if shake.shapes == [false; 3] {
            return None;
        }
        Some(shake)
    }

    /// A number that has to fall inside `allowed`; otherwise a warning and
    /// `slot` keeps its default.
    fn ranged(&mut self, element: Element<'_>, name: &str, slot: &mut u32, allowed: RangeInclusive<u32>) {
        let Some(text) = element.attr(name) else {
            return;
        };
        match element.attr_u32(name) {
            Some(value) if allowed.contains(&value) => *slot = value,
            Some(value) => self.warn(
                element,
                format!("{name} is {value}, which is outside {}..{}", allowed.start(), allowed.end()),
            ),
            None => self.warn(element, format!("{name} is {text:?}, which is not a number")),
        }
    }

    fn real(&mut self, element: Element<'_>, name: &str, slot: &mut f32, allowed: RangeInclusive<f32>) {
        let Some(text) = element.attr(name) else {
            return;
        };
        match text.trim().parse::<f32>() {
            Ok(value) if allowed.contains(&value) => *slot = value,
            Ok(value) => self.warn(
                element,
                format!("{name} is {value}, which is outside {}..{}", allowed.start(), allowed.end()),
            ),
            Err(_) => self.warn(element, format!("{name} is {text:?}, which is not a number")),
        }
    }

    fn flag(&mut self, element: Element<'_>, name: &str, slot: &mut bool) {
        let Some(text) = element.attr(name) else {
            return;
        };
        match element.attr_bool(name) {
            Some(value) => *slot = value,
            None => self.warn(element, format!("{name} {text:?} is not true or false")),
        }
    }
}

/// `any`, or a comma-separated list of shake kinds.
fn parse_shapes(text: &str) -> Option<[bool; 3]> {
    if text.trim() == "any" {
        return Some([true; 3]);
    }
    let mut shapes = [false; 3];
    for part in text.split(',') {
        shapes[ShakeShape::from_name(part)? as usize] = true;
    }
    Some(shapes)
}

fn format_shapes(shapes: [bool; 3]) -> String {
    let names: Vec<_> = ShakeShape::ALL
        .into_iter()
        .filter(|shape| shapes[*shape as usize])
        .map(ShakeShape::name)
        .collect();
    names.join(",")
}

/// Writes the attributes of `look` that differ from `base`.
fn write_look(out: &mut String, look: &Look, base: &Look) {
    let mut put = |name: &str, value: String| {
        let _ = write!(out, " {name}=\"{}\"", escape_attr(&value));
    };
    if look.shape != base.shape {
        put("shape", look.shape.to_string());
    }
    if look.duration_ms != base.duration_ms {
        put("duration", look.duration_ms.to_string());
    }
    if look.size != base.size {
        put("size", look.size.to_string());
    }
    if look.color != base.color {
        put("color", color_hex(look.color));
    }
    if look.auto_color != base.auto_color {
        put("auto-color", look.auto_color.to_string());
    }
    if look.dim != base.dim {
        put("dim", look.dim.to_string());
    }
    if look.recall != base.recall {
        put("recall", look.recall.name().to_owned());
    }
}

fn write_tap(out: &mut String, tap: &Tap) {
    let default = Tap::new(tap.key);
    let _ = write!(out, "\t\t\t<tap key=\"{}\"", tap.key.name());
    if tap.side != default.side {
        let _ = write!(out, " side=\"{}\"", tap.side.name());
    }
    for (name, value, default) in [
        ("count", tap.count, default.count),
        ("hold", tap.max_hold_ms, default.max_hold_ms),
        ("gap", tap.max_gap_ms, default.max_gap_ms),
    ] {
        if value != default {
            let _ = write!(out, " {name}=\"{value}\"");
        }
    }
    out.push_str("/>\n");
}

fn write_shake(out: &mut String, shake: &Shake) {
    let d = Shake::new(shake.modifiers);
    out.push_str("\t\t\t<shake");
    if shake.modifiers.0 != 0 {
        let _ = write!(out, " mod=\"{}\"", format_modifiers(shake.modifiers));
    }
    if !shake.all_shapes() {
        let _ = write!(out, " shapes=\"{}\"", format_shapes(shake.shapes));
    }
    for (name, value, default) in [
        ("window", shake.window_ms, d.window_ms),
        ("segment", shake.segment, d.segment),
        ("speed", shake.speed, d.speed),
        ("reversals", shake.reversals, d.reversals),
        ("degrees", shake.circle_degrees, d.circle_degrees),
        ("circle-speed", shake.circle_speed, d.circle_speed),
        ("cooldown", shake.cooldown_ms, d.cooldown_ms),
    ] {
        if value != default {
            let _ = write!(out, " {name}=\"{value}\"");
        }
    }
    if shake.dominance != d.dominance {
        let _ = write!(out, " dominance=\"{}\"", shake.dominance);
    }
    out.push_str("/>\n");
}

/// Writes `<finder>` with only what differs from the defaults, and nothing at
/// all when everything is default — so a file that never mentioned the finder
/// does not grow a section on the first save.
pub(super) fn write_finder(out: &mut String, finder: &Finder) {
    if *finder == Finder::default() {
        return;
    }
    let default = Finder::default();

    out.push_str("\t<finder");
    write_look(out, &finder.look, &default.look);
    if finder.guard != default.guard {
        let _ = write!(out, " guard=\"{}\"", finder.guard);
    }
    if finder.exclude.is_empty() && finder.styles.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");

    for process in &finder.exclude {
        let _ = writeln!(out, "\t\t<exclude process=\"{}\"/>", escape_attr(process));
    }
    for (index, style) in finder.styles.iter().enumerate() {
        let _ = write!(out, "\t\t<style name=\"{}\"", escape_attr(&style.name));
        if index == finder.default_style && index != 0 {
            out.push_str(" default=\"true\"");
        }
        write_look(out, &style.look, &finder.look);
        if style.taps.is_empty() && style.shakes.is_empty() {
            out.push_str("/>\n");
            continue;
        }
        out.push_str(">\n");
        for tap in &style.taps {
            write_tap(out, tap);
        }
        for shake in &style.shakes {
            write_shake(out, shake);
        }
        out.push_str("\t\t</style>\n");
    }
    out.push_str("\t</finder>\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_SHIFT};

    fn load(name: &str, text: &str) -> Settings {
        let path = std::env::temp_dir().join(format!("y-snap-finder-{name}.xml"));
        std::fs::write(&path, text).expect("temporary file");
        let settings = Settings::load_from(path.clone()).expect("loads");
        let _ = std::fs::remove_file(&path);
        settings
    }

    #[test]
    fn no_section_means_the_defaults_and_no_automatic_trigger() {
        let settings = load("absent", "<y-snap></y-snap>");
        assert_eq!(settings.finder, Finder::default());
        assert!(settings.finder.styles.is_empty());
        assert!(settings.warnings.is_empty());
        // The action still has a look to play.
        assert_eq!(settings.finder.style(None).look, Look::default());
    }

    #[test]
    fn the_defaults_are_the_tuned_values() {
        let strict = Shake::new(HOT_KEY_MODIFIERS(0));
        assert_eq!((strict.speed, strict.reversals, strict.segment), (3000, 3, 120));
        let held = Shake::new(MOD_CONTROL);
        assert_eq!((held.speed, held.reversals), (1200, 2));
        let tap = Tap::new(TapKey::Ctrl);
        assert_eq!((tap.count, tap.max_hold_ms, tap.max_gap_ms), (2, 200, 400));
        assert!(Finder::default().guard);
    }

    #[test]
    fn reads_styles_with_their_triggers() {
        let settings = load(
            "all",
            r##"<y-snap>
                 <finder guard="false" size="200" color="#00C8FF">
                   <exclude process="Game.EXE"/>
                   <exclude process="vlc"/>
                   <style name="quick" shape="halo" auto-color="true" duration="1500">
                     <tap key="ctrl" hold="150" gap="350"/>
                     <shake mod="Ctrl"/>
                   </style>
                   <style name="radar" default="true" dim="70" recall="center">
                     <shake shapes="horizontal, vertical" speed="2500" dominance="2.0"/>
                     <tap key="Shift" side="right" count="3"/>
                   </style>
                   <style name="screens" shape="monitor">
                     <shake shapes="circle" degrees="540" circle-speed="1800"
                            window="800" segment="100" reversals="4" cooldown="1200"/>
                     <shake mod="Ctrl+Shift"/>
                   </style>
                 </finder>
               </y-snap>"##,
        );
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        let finder = &settings.finder;
        assert!(!finder.guard);
        assert_eq!(finder.exclude, ["game", "vlc"]);
        assert_eq!(finder.look.size, 200);
        assert_eq!(finder.styles.len(), 3);
        assert_eq!(finder.default_style, 1);

        // Looks cascade: finder defaults, then the style.
        let quick = &finder.styles[0];
        assert_eq!(quick.look.shape, Shape::Halo);
        assert_eq!(quick.look.size, 200);
        assert_eq!(quick.look.color, Color::rgb(0x00, 0xC8, 0xFF));
        assert!(quick.look.auto_color);
        assert_eq!(quick.look.duration_ms, 1500);
        let radar = &finder.styles[1];
        assert_eq!(radar.look.shape, Shape::Beacon);
        assert_eq!((radar.look.dim, radar.look.recall), (70, Recall::Center));

        let tap = quick.taps[0];
        assert_eq!((tap.key, tap.side, tap.count), (TapKey::Ctrl, Side::Any, 2));
        assert_eq!((tap.max_hold_ms, tap.max_gap_ms), (150, 350));
        // A held modifier relaxes the defaults.
        let held = quick.shakes[0];
        assert_eq!(held.modifiers, MOD_CONTROL);
        assert_eq!((held.speed, held.reversals), (1200, 2));
        assert!(held.all_shapes());

        let shake = radar.shakes[0];
        assert_eq!(shake.shapes, [true, true, false]);
        assert_eq!((shake.speed, shake.dominance), (2500, 2.0));
        let tap = radar.taps[0];
        assert_eq!((tap.key, tap.side, tap.count), (TapKey::Shift, Side::Right, 3));

        let circle = finder.styles[2].shakes[0];
        assert_eq!(circle.shapes, [false, false, true]);
        assert_eq!((circle.circle_degrees, circle.circle_speed), (540, 1800));
        assert_eq!((circle.window_ms, circle.segment), (800, 100));
        assert_eq!((circle.reversals, circle.cooldown_ms), (4, 1200));
        assert_eq!(finder.styles[2].shakes[1].modifiers, MOD_CONTROL | MOD_SHIFT);

        // The action can ask for a style by name, or take the default one.
        assert_eq!(finder.style(Some("screens")).look.shape, Shape::Monitor);
        assert_eq!(finder.style(None).name, "radar");
        assert_eq!(finder.style(Some("missing")).name, "radar");
    }

    #[test]
    fn a_bad_value_is_a_warning_and_keeps_the_default() {
        let settings = load(
            "bad",
            r##"<y-snap>
                 <finder shape="sparkle" duration="12" size="big" color="red" wobble="1">
                   <exclude/>
                   <surprise/>
                   <style name="a" recall="teleport" auto-color="maybe">
                     <tap key="ctrl" count="9" side="middle"/>
                     <shake shapes="wiggle" dominance="9.5"/>
                     <dance/>
                   </style>
                 </finder>
               </y-snap>"##,
        );
        let finder = &settings.finder;
        let default = Look::default();
        assert_eq!(finder.look, default);
        assert!(finder.exclude.is_empty());
        let style = &finder.styles[0];
        assert_eq!(style.look, default);
        assert_eq!(style.taps[0], Tap::new(TapKey::Ctrl));
        assert_eq!(style.shakes[0].dominance, Shake::new(HOT_KEY_MODIFIERS(0)).dominance);
        assert!(style.shakes[0].all_shapes());

        let all = settings.warnings.join("\n");
        for expected in [
            "unknown shape",
            "outside 300..5000",
            "size is \"big\"",
            "not a colour",
            "no wobble attribute",
            "needs a process name",
            "<surprise> does not belong in <finder>",
            "unknown recall",
            "auto-color \"maybe\" is not true or false",
            "count is 9",
            "unknown side",
            "shapes \"wiggle\"",
            "dominance is 9.5",
            "<dance> does not belong in <style>",
        ] {
            assert!(all.contains(expected), "{expected:?} missing from:\n{all}");
        }
        assert_eq!(settings.warnings.len(), 14, "{all}");
    }

    #[test]
    fn what_cannot_work_is_skipped_with_a_warning() {
        let settings = load(
            "skipped",
            r##"<y-snap>
                 <finder>
                   <style/>
                   <style name="one">
                     <tap/>
                     <tap key="hyper"/>
                     <shake mod="hyper"/>
                     <tap key="ctrl"/>
                     <shake shapes="circle"/>
                   </style>
                   <style name="one"/>
                   <style name="two" default="true">
                     <tap key="Control"/>
                     <shake shapes="circle,vertical"/>
                     <shake shapes="circle"/>
                   </style>
                   <style name="three" default="true"/>
                 </finder>
               </y-snap>"##,
        );
        let finder = &settings.finder;
        let names: Vec<_> = finder.styles.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["one", "two", "three"]);
        assert_eq!(finder.default_style, 1);
        assert_eq!(finder.styles[0].taps.len(), 1);
        assert_eq!(finder.styles[0].shakes.len(), 1);
        // The same tap, and a circle already taken, are left to the first.
        assert!(finder.styles[1].taps.is_empty());
        assert_eq!(finder.styles[1].shakes.len(), 1);
        assert_eq!(finder.styles[1].shakes[0].shapes, [false, true, false]);

        let all = settings.warnings.join("\n");
        for expected in [
            "needs a name",
            "needs a key",
            "unknown tap key",
            "not a set of modifiers",
            "second style named \"one\"",
            "already used by another trigger",
            "second default style",
        ] {
            assert!(all.contains(expected), "{expected:?} missing from:\n{all}");
        }
    }

    #[test]
    fn a_second_section_is_ignored_with_a_warning() {
        let settings = load(
            "twice",
            r#"<y-snap><finder size="100"/><finder size="300"/></y-snap>"#,
        );
        assert_eq!(settings.finder.look.size, 100);
        assert_eq!(settings.warnings.len(), 1);
        assert!(settings.warnings[0].contains("second <finder>"));
    }

    #[test]
    fn a_default_finder_is_not_written() {
        let settings = load("quiet", "<y-snap><finder/></y-snap>");
        assert!(!settings.to_xml().contains("<finder"));
    }

    #[test]
    fn only_what_changed_is_written_and_it_reads_back() {
        let mut settings = load("round", "<y-snap></y-snap>");
        let finder = &mut settings.finder;
        finder.look.color = Color::rgb(1, 2, 3);
        finder.exclude = vec!["game".into(), "vlc".into()];
        let mut quick = Style::new("quick", finder.look);
        quick.look.shape = Shape::Halo;
        quick.look.auto_color = true;
        quick.taps.push(Tap::new(TapKey::Ctrl));
        let mut shake = Shake::new(MOD_CONTROL);
        shake.speed = 900;
        shake.dominance = 1.75;
        quick.shakes.push(shake);
        let mut radar = Style::new("radar", finder.look);
        let mut tap = Tap::new(TapKey::Shift);
        tap.side = Side::Left;
        tap.count = 3;
        radar.taps.push(tap);
        let mut circle = Shake::new(HOT_KEY_MODIFIERS(0));
        circle.shapes = [false, false, true];
        radar.shakes.push(circle);
        finder.styles = vec![quick, radar];
        finder.default_style = 1;

        let xml = settings.to_xml();
        assert!(xml.contains("<style name=\"quick\" shape=\"halo\" auto-color=\"true\">"), "{xml}");
        assert!(xml.contains("speed=\"900\""), "{xml}");
        assert!(xml.contains("shapes=\"circle\""), "{xml}");
        assert!(xml.contains("default=\"true\""), "{xml}");
        assert!(!xml.contains("segment"), "defaults stay out: {xml}");
        assert!(!xml.contains("duration="), "defaults stay out: {xml}");
        assert!(!xml.contains("hold="), "defaults stay out: {xml}");

        let path = std::env::temp_dir().join("y-snap-finder-roundtrip.xml");
        std::fs::write(&path, &xml).expect("temporary file");
        let reloaded = Settings::load_from(path.clone()).expect("reloads");
        let _ = std::fs::remove_file(&path);
        assert!(reloaded.warnings.is_empty(), "{:?}", reloaded.warnings);
        assert_eq!(reloaded.finder, settings.finder);
    }

    #[test]
    fn process_names_are_compared_without_case_or_extension() {
        assert_eq!(normalize_process(" Game.EXE "), "game");
        assert_eq!(normalize_process("vlc"), "vlc");
        assert_eq!(normalize_process(".exe"), "");
    }

    #[test]
    fn every_name_round_trips() {
        for shape in Shape::ALL {
            assert_eq!(Shape::from_name(shape.name()), Some(shape));
        }
        for key in TapKey::ALL {
            assert_eq!(TapKey::from_name(key.name()), Some(key));
        }
        for shape in ShakeShape::ALL {
            assert_eq!(ShakeShape::from_name(shape.name()), Some(shape));
        }
        assert_eq!(Shape::from_name("Beacon"), None);
    }
}

#[cfg(test)]
mod binding_tests {
    use crate::config::Settings;

    fn load(name: &str, text: &str) -> Settings {
        let path = std::env::temp_dir().join(format!("y-snap-finder-bindings-{name}.xml"));
        std::fs::write(&path, text).expect("temporary file");
        let settings = Settings::load_from(path.clone()).expect("loads");
        let _ = std::fs::remove_file(&path);
        settings
    }

    #[test]
    fn a_binding_picks_a_style_by_name_and_it_reads_back() {
        let settings = load(
            "pick",
            r#"<y-snap>
                 <hotkeys>
                   <hotkey key="Ctrl+Alt+F" action="find-mouse" style="radar"/>
                   <hotkey key="Ctrl+Alt+G" action="find-mouse"/>
                 </hotkeys>
                 <mouse>
                   <shortcut button="middle" mod="Win" action="find-mouse" style="quick"/>
                 </mouse>
                 <finder>
                   <style name="quick"/>
                   <style name="radar"/>
                 </finder>
               </y-snap>"#,
        );
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys[0].style, Some(1));
        assert_eq!(settings.hotkeys[1].style, None);
        assert_eq!(settings.mouse_shortcuts[0].style, Some(0));

        let xml = settings.to_xml();
        assert!(xml.contains("action=\"find-mouse\" style=\"radar\""), "{xml}");
        assert!(xml.contains("action=\"find-mouse\" style=\"quick\""), "{xml}");
    }

    #[test]
    fn an_unknown_style_or_a_style_on_another_action_is_a_warning() {
        let settings = load(
            "unknown",
            r#"<y-snap>
                 <hotkeys>
                   <hotkey key="Ctrl+Alt+F" action="find-mouse" style="nope"/>
                   <hotkey key="Ctrl+Alt+G" action="minimize" style="quick"/>
                 </hotkeys>
                 <finder><style name="quick"/></finder>
               </y-snap>"#,
        );
        assert_eq!(settings.hotkeys[0].style, None);
        assert_eq!(settings.hotkeys[1].style, None);
        let all = settings.warnings.join("\n");
        assert!(all.contains("no finder style named"), "{all}");
        assert!(all.contains("only applies to the find-mouse"), "{all}");
    }
}
