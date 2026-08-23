//! Reading `y-snap.xml`.
//!
//! The file is the whole user interface: y-snap has no settings dialog, so a
//! mistake in it has to come back as something better than "invalid". A
//! malformed document is fatal and reported with its line number; anything
//! else — an unknown action, a key combination that does not parse, an
//! attribute nobody recognises — is collected as a warning, and the rest of
//! the file still loads. Losing every binding because one of them has a typo
//! would be the worse failure.
//!
//! See `README.md` for the schema.

use std::fmt;
use std::path::PathBuf;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
};
use y_core::hotkey::{Hotkey, key_from_name};
use y_core::overlay::Color;
use y_core::theme;
use y_core::xml::{self, Document, Element};

use crate::action::{Action, Target};

/// The file y-snap looks for.
pub const FILE_NAME: &str = "y-snap.xml";

/// Everything the configuration file says.
#[derive(Debug)]
pub struct Settings {
    /// The file this came from, for the tray menu and error messages.
    pub path: PathBuf,
    pub hotkeys: Vec<Binding>,
    pub mouse: Mouse,
    pub overlay: Overlay,
    /// Popup shown when the pointer hovers a window's minimize button.
    pub min_button: Vec<QuickAction>,
    /// Problems that did not stop the file from loading, one line each.
    pub warnings: Vec<String>,
}

/// One tile of the popup shown when hovering a window's minimize button —
/// `<minbutton>`'s own, smaller vocabulary of `<action>`: no key (the mouse
/// is what drives it), no group (there is only ever one row of tiles).
#[derive(Clone, Debug)]
pub struct QuickAction {
    pub action: Action,
    pub label: Option<String>,
    pub icon: Option<String>,
}

/// One `<hotkey>`.
#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub hotkey: Hotkey,
    pub action: Action,
    pub target: Target,
    /// Which desktop `desktop-move` moves to, 1-based; 0 when unset.
    #[allow(dead_code, reason = "read once the desktop actions are written")]
    pub desktop: u32,
    /// Take the combination with a low-level keyboard hook when the system
    /// refuses to register it — how a `Win+…` combination the shell already
    /// owns can be reclaimed. Off by default: a combination belonging to
    /// another application belongs to that application.
    pub steal: bool,
}

/// The `<mouse>` gestures. Each is a button plus the modifiers held with it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mouse {
    pub drag: Option<Gesture>,
    pub resize: Option<Gesture>,
    pub switch: Option<Gesture>,
}

#[derive(Clone, Copy, Debug)]
pub struct Gesture {
    pub button: Button,
    pub modifiers: HOT_KEY_MODIFIERS,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    Wheel,
}

impl Button {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "left" => Self::Left,
            "right" => Self::Right,
            "middle" => Self::Middle,
            "wheel" => Self::Wheel,
            _ => return None,
        })
    }
}

/// The `<overlay>` popup: how it looks, and what is on it.
#[derive(Clone, Debug)]
pub struct Overlay {
    /// Outline the target window while the overlay is open, so it is obvious
    /// which window is about to be acted on.
    pub focus_border: bool,
    pub border_width: u32,
    pub corner_radius: u32,
    /// Alpha of the popup's own background. The tiles stay opaque.
    pub alpha: u8,
    /// Default grid, for tiles in no group.
    pub columns: u32,
    pub tile: (u32, u32),
    pub key_font_size: u32,
    pub label_font_size: u32,
    pub icon_font: String,
    pub icon_font_size: u32,
    pub palette: Palette,
    pub groups: Vec<Group>,
    pub items: Vec<Item>,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            focus_border: false,
            border_width: 2,
            corner_radius: 8,
            alpha: 235,
            columns: 4,
            tile: (88, 64),
            key_font_size: 16,
            label_font_size: 9,
            // Substituted silently by GDI when missing, so an unavailable
            // font costs a placeholder glyph, not a failure.
            icon_font: "Segoe Fluent Icons".to_owned(),
            icon_font_size: 22,
            palette: Palette::LIGHT,
            groups: Vec::new(),
            items: Vec::new(),
        }
    }
}

/// Tile colours: normal, hovered, active (the toggle this tile drives is
/// currently on) and disabled.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub back: Color,
    pub text: Color,
    pub hover_back: Color,
    pub hover_text: Color,
    pub active_back: Color,
    pub active_text: Color,
    pub disabled_back: Color,
    pub disabled_text: Color,
    pub border: Color,
}

impl Palette {
    /// Close to Windows 11's own flyouts: accent blue for hover, green for
    /// "this is currently on".
    pub const LIGHT: Self = Self {
        back: Color::rgb(249, 249, 249),
        text: Color::rgb(0, 0, 0),
        hover_back: Color::rgb(0, 120, 215),
        hover_text: Color::rgb(255, 255, 255),
        active_back: Color::rgb(16, 124, 16),
        active_text: Color::rgb(255, 255, 255),
        disabled_back: Color::rgb(200, 200, 200),
        disabled_text: Color::rgb(130, 130, 130),
        border: Color::rgb(0, 120, 215),
    };

    pub const DARK: Self = Self {
        back: Color::rgb(32, 32, 32),
        text: Color::rgb(255, 255, 255),
        active_back: Color::rgb(49, 150, 79),
        disabled_back: Color::rgb(58, 58, 58),
        disabled_text: Color::rgb(130, 130, 130),
        ..Self::LIGHT
    };

    fn for_theme(name: &str) -> Option<Self> {
        match name.trim() {
            "light" => Some(Self::LIGHT),
            "dark" => Some(Self::DARK),
            "auto" if theme::is_dark() => Some(Self::DARK),
            "auto" => Some(Self::LIGHT),
            _ => None,
        }
    }
}

/// A `<group>`: a row of tiles laid out on its own, with its own grid. What
/// puts the per-desktop tiles on one wide row instead of in the main grid.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "laid out by the overlay, which is not written yet"
)]
pub struct Group {
    pub id: String,
    pub columns: Option<u32>,
    pub tile_width: Option<u32>,
    pub tile_height: Option<u32>,
}

/// One `<action>` tile of the overlay.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "drawn by the overlay, which is not written yet")]
pub struct Item {
    /// The key that triggers it. Absent only for `desktop-list`, which has no
    /// tile of its own — it expands into tiles that carry their own keys.
    pub key: Option<VIRTUAL_KEY>,
    pub action: Action,
    pub label: Option<String>,
    /// Shown in place of `label` while the toggle is on: "Pin" / "Unpin".
    pub label_on: Option<String>,
    /// A glyph from the icon font, shown instead of the key letter.
    pub icon: Option<String>,
    /// The `<group>` this tile belongs to, if any.
    pub group: Option<String>,
    /// Which desktop `desktop-move` moves to, 1-based; 0 when unset.
    pub desktop: u32,
}

/// Why the configuration could not be read at all.
#[derive(Debug)]
pub enum LoadError {
    Missing(PathBuf),
    Unreadable(PathBuf, std::io::Error),
    Malformed(PathBuf, xml::Error),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(path) => write!(f, "No configuration file at {}", path.display()),
            Self::Unreadable(path, error) => {
                write!(f, "Cannot read {}: {error}", path.display())
            }
            Self::Malformed(path, error) => {
                write!(f, "{}, {error}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl Settings {
    /// Reads the configuration from wherever [`path`] finds it.
    pub fn load() -> Result<Self, LoadError> {
        Self::load_from(path())
    }

    pub fn load_from(path: PathBuf) -> Result<Self, LoadError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(LoadError::Missing(path));
            }
            Err(error) => return Err(LoadError::Unreadable(path, error)),
        };

        let document =
            Document::parse(&text).map_err(|error| LoadError::Malformed(path.clone(), error))?;

        let mut reader = Reader::default();
        let root = document.root();

        let settings = Self {
            hotkeys: reader.hotkeys(root),
            mouse: reader.mouse(root),
            overlay: reader.overlay(root),
            min_button: reader.min_button(root),
            warnings: std::mem::take(&mut reader.warnings),
            path,
        };
        Ok(settings)
    }
}

/// Where the configuration file lives, in the order the places are tried.
///
/// Portable first: an executable in a folder with its own `y-snap.xml` next
/// to it is a copy the user carries around, and it should not be quietly
/// steered to a file in their roaming profile.
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
/// written — a path with a stray `%` in it is better shown back to the user
/// intact than silently emptied.
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

/// Walks the document, collecting warnings as it goes.
#[derive(Default)]
struct Reader {
    warnings: Vec<String>,
}

impl Reader {
    fn warn(&mut self, element: Element<'_>, message: impl fmt::Display) {
        self.warnings
            .push(format!("line {}: {message}", element.line()));
    }

    /// Reports attributes nobody reads — almost always a typo, and silently
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

    /// The `action` attribute, shared by `<hotkey>` and `<action>`.
    fn action(&mut self, element: Element<'_>) -> Option<Action> {
        let name = element.attr("action")?;
        match Action::from_name(name) {
            Some(action) => Some(action),
            None => {
                self.warn(element, format!("unknown action {name:?}"));
                None
            }
        }
    }

    fn hotkeys(&mut self, root: Element<'_>) -> Vec<Binding> {
        let Some(section) = root.child("hotkeys") else {
            return Vec::new();
        };

        let mut bindings = Vec::new();

        for element in section.children() {
            if element.name() != "hotkey" {
                let name = element.name();
                self.warn(element, format!("<{name}> is not a <hotkey>"));
                continue;
            }
            self.check(
                element,
                &["key", "action", "target", "desktop", "steal-from-shell"],
            );

            let Some(action) = self.action(element) else {
                if element.attr("action").is_none() {
                    self.warn(element, "a <hotkey> needs an action attribute");
                }
                continue;
            };

            // Only on a tile, where it becomes several tiles; a key can only
            // ever fire one thing.
            if action == Action::DesktopList {
                self.warn(element, "desktop-list only works on an overlay tile");
                continue;
            }

            let Some(text) = element.attr("key") else {
                self.warn(element, "a <hotkey> needs a key attribute");
                continue;
            };
            let Some(hotkey) = Hotkey::parse(text) else {
                self.warn(element, format!("{text:?} is not a key combination"));
                continue;
            };

            bindings.push(Binding {
                hotkey,
                action,
                target: self.target(element),
                desktop: self.desktop(element),
                steal: element.attr_bool("steal-from-shell").unwrap_or(false),
            });
        }

        bindings
    }

    fn target(&mut self, element: Element<'_>) -> Target {
        let Some(name) = element.attr("target") else {
            return Target::default();
        };

        Target::from_name(name).unwrap_or_else(|| {
            self.warn(element, format!("unknown target {name:?}"));
            Target::default()
        })
    }

    fn desktop(&mut self, element: Element<'_>) -> u32 {
        match element.attr("desktop") {
            None => 0,
            Some(text) => element.attr_u32("desktop").unwrap_or_else(|| {
                self.warn(element, format!("{text:?} is not a desktop number"));
                0
            }),
        }
    }

    fn mouse(&mut self, root: Element<'_>) -> Mouse {
        let Some(section) = root.child("mouse") else {
            return Mouse::default();
        };

        let mut mouse = Mouse::default();

        for element in section.children() {
            let slot = match element.name() {
                "drag" => &mut mouse.drag,
                "resize" => &mut mouse.resize,
                "switch" => &mut mouse.switch,
                name => {
                    self.warn(element, format!("<{name}> is not a mouse gesture"));
                    continue;
                }
            };
            self.check(element, &["button", "mod"]);

            let Some(name) = element.attr("button") else {
                self.warn(element, "a mouse gesture needs a button attribute");
                continue;
            };
            let Some(button) = Button::from_name(name) else {
                self.warn(element, format!("unknown button {name:?}"));
                continue;
            };

            let text = element.attr("mod").unwrap_or("Alt");
            let Some(modifiers) = parse_modifiers(text) else {
                self.warn(element, format!("{text:?} is not a set of modifiers"));
                continue;
            };

            *slot = Some(Gesture { button, modifiers });
        }

        mouse
    }

    fn min_button(&mut self, root: Element<'_>) -> Vec<QuickAction> {
        let Some(section) = root.child("minbutton") else {
            return Vec::new();
        };

        let mut items = Vec::new();

        for element in section.children() {
            if element.name() != "action" {
                let name = element.name();
                self.warn(element, format!("<{name}> does not belong in <minbutton>"));
                continue;
            }
            self.check(element, &["action", "label", "icon"]);

            let Some(action) = self.action(element) else {
                continue;
            };

            // Only actions that act directly on the hovered window make
            // sense here: there is no key to drive desktop-list's per-desktop
            // expansion, and overlay would show the big overlay from inside
            // this small one.
            if !action.needs_window() || matches!(action, Action::DesktopList | Action::Overlay) {
                self.warn(element, format!("{action} cannot be used in <minbutton>"));
                continue;
            }

            items.push(QuickAction {
                action,
                label: element.attr("label").map(str::to_owned),
                icon: element.attr("icon").map(str::to_owned),
            });
        }

        items
    }

    fn overlay(&mut self, root: Element<'_>) -> Overlay {
        let Some(section) = root.child("overlay") else {
            return Overlay::default();
        };

        let mut overlay = Overlay::default();
        self.check(
            section,
            &[
                "theme",
                "focus-border",
                "border-width",
                "corner-radius",
                "alpha",
                "columns",
                "tile-width",
                "tile-height",
                "key-font-size",
                "label-font-size",
                "icon-font",
                "icon-font-size",
                "back-color",
                "text-color",
                "hover-back-color",
                "hover-text-color",
                "active-back-color",
                "active-text-color",
                "disabled-back-color",
                "disabled-text-color",
                "border-color",
            ],
        );

        // The theme is only a set of defaults: it is applied first so that an
        // explicit colour attribute still wins, and a file can pick a theme
        // and change one colour instead of listing all nine.
        let theme = section.attr("theme").unwrap_or("auto");
        overlay.palette = Palette::for_theme(theme).unwrap_or_else(|| {
            self.warn(section, format!("unknown theme {theme:?}"));
            Palette::LIGHT
        });

        let palette = &mut overlay.palette;
        for (name, slot) in [
            ("back-color", &mut palette.back),
            ("text-color", &mut palette.text),
            ("hover-back-color", &mut palette.hover_back),
            ("hover-text-color", &mut palette.hover_text),
            ("active-back-color", &mut palette.active_back),
            ("active-text-color", &mut palette.active_text),
            ("disabled-back-color", &mut palette.disabled_back),
            ("disabled-text-color", &mut palette.disabled_text),
            ("border-color", &mut palette.border),
        ] {
            let Some(text) = section.attr(name) else {
                continue;
            };
            match Color::parse(text) {
                Some(color) => *slot = color,
                // Borrowing `palette` mutably here, so the warning cannot go
                // through `self` until the loop is over.
                None => self
                    .warnings
                    .push(format!("line {}: {text:?} is not a colour", section.line())),
            }
        }

        overlay.focus_border = section.attr_bool("focus-border").unwrap_or(false);
        self.number(section, "border-width", &mut overlay.border_width);
        self.number(section, "corner-radius", &mut overlay.corner_radius);
        self.number(section, "columns", &mut overlay.columns);
        self.number(section, "tile-width", &mut overlay.tile.0);
        self.number(section, "tile-height", &mut overlay.tile.1);
        self.number(section, "key-font-size", &mut overlay.key_font_size);
        self.number(section, "label-font-size", &mut overlay.label_font_size);
        self.number(section, "icon-font-size", &mut overlay.icon_font_size);

        let mut alpha = overlay.alpha as u32;
        self.number(section, "alpha", &mut alpha);
        overlay.alpha = alpha.min(255) as u8;

        if let Some(font) = section.attr("icon-font") {
            overlay.icon_font = font.to_owned();
        }

        for element in section.children() {
            match element.name() {
                "group" => {
                    self.check(element, &["id", "columns", "tile-width", "tile-height"]);
                    let Some(id) = element.attr("id") else {
                        self.warn(element, "a <group> needs an id attribute");
                        continue;
                    };
                    overlay.groups.push(Group {
                        id: id.to_owned(),
                        columns: element.attr_u32("columns"),
                        tile_width: element.attr_u32("tile-width"),
                        tile_height: element.attr_u32("tile-height"),
                    });
                }

                "action" => {
                    if let Some(item) = self.item(element) {
                        overlay.items.push(item);
                    }
                }

                name => self.warn(element, format!("<{name}> does not belong in <overlay>")),
            }
        }

        overlay
    }

    fn item(&mut self, element: Element<'_>) -> Option<Item> {
        self.check(
            element,
            &[
                "key", "action", "label", "label-on", "icon", "group", "desktop",
            ],
        );

        let action = self.action(element)?;

        // Every tile needs a key: the overlay is driven by one key press.
        // desktop-list is the exception — it is not a tile, it becomes tiles.
        let key = match element.attr("key") {
            Some(text) => match key_from_name(text) {
                Some(key) => Some(key),
                None => {
                    self.warn(element, format!("{text:?} is not a key"));
                    return None;
                }
            },
            None if action == Action::DesktopList => None,
            None => {
                self.warn(element, "an overlay <action> needs a key attribute");
                return None;
            }
        };

        Some(Item {
            key,
            action,
            label: element.attr("label").map(str::to_owned),
            label_on: element.attr("label-on").map(str::to_owned),
            icon: element.attr("icon").map(str::to_owned),
            group: element.attr("group").map(str::to_owned),
            desktop: self.desktop(element),
        })
    }

    /// An optional number, left alone when absent and reported when wrong.
    fn number(&mut self, element: Element<'_>, name: &str, slot: &mut u32) {
        let Some(text) = element.attr(name) else {
            return;
        };

        match element.attr_u32(name) {
            Some(value) => *slot = value,
            None => self.warn(
                element,
                format!("{name} is {text:?}, which is not a number"),
            ),
        }
    }
}

/// Reads `"Win+Alt"`, `"Alt"`, or `"none"` — the modifier half of a hotkey,
/// on its own, for the mouse gestures.
fn parse_modifiers(text: &str) -> Option<HOT_KEY_MODIFIERS> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Some(HOT_KEY_MODIFIERS(0));
    }

    let mut modifiers = 0;
    for part in trimmed.split('+') {
        modifiers |= match part.trim().to_ascii_lowercase().as_str() {
            "win" | "windows" | "super" | "meta" => MOD_WIN.0,
            "ctrl" | "control" => MOD_CONTROL.0,
            "alt" => MOD_ALT.0,
            "shift" => MOD_SHIFT.0,
            _ => return None,
        };
    }

    Some(HOT_KEY_MODIFIERS(modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips `text` through a real file: `load_from` is the only way in,
    /// and going through it keeps the tests honest about the whole path.
    fn load(name: &str, text: &str) -> Settings {
        let path = std::env::temp_dir().join(format!("y-snap-{name}.xml"));
        std::fs::write(&path, text).expect("temporary file");
        let settings = Settings::load_from(path.clone()).expect("loads");
        let _ = std::fs::remove_file(&path);
        settings
    }

    #[test]
    fn reads_hotkeys() {
        let settings = load(
            "hotkeys",
            r#"<y-snap>
                 <hotkeys>
                   <hotkey key="Win+Space" action="overlay" target="pointed"/>
                   <hotkey key="Win+Escape" action="close" steal-from-shell="true"/>
                 </hotkeys>
               </y-snap>"#,
        );

        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys.len(), 2);

        assert_eq!(settings.hotkeys[0].action, Action::Overlay);
        assert_eq!(settings.hotkeys[0].target, Target::Pointed);
        assert!(!settings.hotkeys[0].steal);

        assert_eq!(settings.hotkeys[1].hotkey.to_string(), "Win+Esc");
        assert_eq!(settings.hotkeys[1].target, Target::Focused);
        assert!(settings.hotkeys[1].steal);
    }

    #[test]
    fn keeps_the_good_bindings_and_reports_the_bad() {
        let settings = load(
            "warnings",
            r#"<y-snap>
                 <hotkeys>
                   <hotkey key="Win+Nope" action="close"/>
                   <hotkey key="Win+F8" action="minimise"/>
                   <hotkey key="Win+F9" action="minimize" target="whatever" colour="red"/>
                 </hotkeys>
               </y-snap>"#,
        );

        assert_eq!(settings.hotkeys.len(), 1);
        assert_eq!(settings.hotkeys[0].action, Action::Minimize);
        assert_eq!(settings.hotkeys[0].target, Target::Focused);
        assert_eq!(settings.warnings.len(), 4, "{:?}", settings.warnings);
        assert!(settings.warnings[0].starts_with("line 3:"));
    }

    #[test]
    fn reads_mouse_gestures() {
        let settings = load(
            "mouse",
            r#"<y-snap>
                 <mouse>
                   <drag button="left" mod="Alt"/>
                   <resize button="right"/>
                   <switch button="sideways" mod="Alt"/>
                 </mouse>
               </y-snap>"#,
        );

        let drag = settings.mouse.drag.expect("drag");
        assert_eq!(drag.button, Button::Left);
        assert_eq!(drag.modifiers, MOD_ALT);

        // No mod attribute: Alt, the default.
        assert_eq!(settings.mouse.resize.expect("resize").modifiers, MOD_ALT);
        assert!(settings.mouse.switch.is_none());
        assert_eq!(settings.warnings.len(), 1);
    }

    #[test]
    fn reads_the_overlay() {
        let settings = load(
            "overlay",
            r##"<y-snap>
                 <overlay theme="dark" columns="5" back-color="#101010" alpha="200">
                   <group id="desktops" columns="9" tile-height="48"/>
                   <action key="M" action="minimize" label="Minimize" icon="M"/>
                   <action action="desktop-list" group="desktops"/>
                   <action action="close"/>
                 </overlay>
               </y-snap>"##,
        );

        let overlay = &settings.overlay;
        assert_eq!(overlay.columns, 5);
        assert_eq!(overlay.alpha, 200);
        assert_eq!(overlay.palette.back, Color::rgb(16, 16, 16));
        // Untouched by back-color, but taken from the dark theme.
        assert_eq!(overlay.palette.text, Color::rgb(255, 255, 255));

        assert_eq!(overlay.groups.len(), 1);
        assert_eq!(overlay.groups[0].columns, Some(9));
        assert_eq!(overlay.groups[0].tile_width, None);

        // The keyless `close` is dropped, `desktop-list` is allowed to be.
        assert_eq!(overlay.items.len(), 2);
        assert_eq!(overlay.items[1].action, Action::DesktopList);
        assert!(overlay.items[1].key.is_none());
        assert_eq!(settings.warnings.len(), 1);
    }

    /// The file shipped in `rsc/` is what a user starts from, and it is also
    /// the only complete example of the schema. It has to load clean.
    #[test]
    fn the_default_configuration_is_valid() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("rsc")
            .join(FILE_NAME);

        let settings = Settings::load_from(path).expect("loads");
        assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
        assert_eq!(settings.hotkeys.len(), 4);
        assert!(settings.mouse.drag.is_some());
        assert!(settings.mouse.resize.is_some());
        assert_eq!(settings.overlay.groups.len(), 1);
        assert_eq!(settings.overlay.items.len(), 16);
        assert_eq!(settings.min_button.len(), 2);
    }

    #[test]
    fn a_broken_document_is_fatal() {
        let path = std::env::temp_dir().join("y-snap-broken.xml");
        std::fs::write(&path, "<y-snap>\n  <hotkeys>\n</y-snap>").expect("temporary file");

        let error = Settings::load_from(path.clone()).expect_err("malformed");
        assert!(matches!(error, LoadError::Malformed(..)));
        assert!(error.to_string().contains("line 3"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_says_where_it_looked() {
        let path = std::env::temp_dir().join("y-snap-absent.xml");
        let _ = std::fs::remove_file(&path);

        let error = Settings::load_from(path.clone()).expect_err("missing");
        assert!(matches!(error, LoadError::Missing(_)));
        assert!(error.to_string().contains("y-snap-absent.xml"));
    }

    #[test]
    fn expands_environment_variables_in_a_path() {
        unsafe { std::env::set_var("Y_SNAP_TEST_DIR", "C:\\somewhere") };

        assert_eq!(
            expand("%Y_SNAP_TEST_DIR%\\y-snap.xml"),
            "C:\\somewhere\\y-snap.xml"
        );
        assert_eq!(expand("%NOT_SET_ANYWHERE%\\x"), "%NOT_SET_ANYWHERE%\\x");
        assert_eq!(expand("50% done"), "50% done");
        assert_eq!(expand("plain"), "plain");
    }

    #[test]
    fn reads_modifier_sets() {
        assert_eq!(parse_modifiers("Win+Alt"), Some(MOD_WIN | MOD_ALT));
        assert_eq!(parse_modifiers(" alt "), Some(MOD_ALT));
        assert_eq!(parse_modifiers("none"), Some(HOT_KEY_MODIFIERS(0)));
        assert_eq!(parse_modifiers("hyper"), None);
    }
}
