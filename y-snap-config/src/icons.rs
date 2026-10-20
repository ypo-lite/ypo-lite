//! The icon catalog behind the icon picker: a curated set of emoji that
//! read well at tile size, grouped by meaning, each with search keywords,
//! plus a few suggestions per [`Action`].
//!
//! Emoji only — no Segoe Fluent Icons private-use glyphs, even though that
//! is the overlay's default `icon-font`: those codepoints are invisible in
//! any other font, and `windows-reactor` gives a `TextBlock` no font
//! choice, so neither this picker nor the preview could show them. Emoji
//! render in both popups (the shipped `y-snap.xml` already uses them) and
//! here. Anything not in the catalog is still one Win + . away, typed into
//! the field.

use y_snap::action::Action;

/// A tab of the icon picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum IconCategory {
    #[default]
    Windows,
    Arrows,
    Tray,
    Pin,
    Desktops,
    Pointer,
    Marks,
}

/// `(emoji, space-separated search keywords)`.
type Entry = (&'static str, &'static str);

const WINDOWS: &[Entry] = &[
    ("➖", "minimize minus line"),
    ("➕", "plus add new"),
    ("❌", "close cross delete x"),
    ("✖️", "close multiply x"),
    ("🔲", "maximize square button"),
    ("🔳", "square button frame overlay"),
    ("⬜", "white square maximize"),
    ("⬛", "black square"),
    ("🖼️", "frame picture borderless border"),
    ("🪟", "window glass"),
    ("🗂️", "tabs dividers list overlay group"),
    ("📂", "folder open"),
    ("🗃️", "card box file"),
    ("🎛️", "control knobs panel overlay"),
    ("🚪", "door quit exit"),
];
const ARROWS: &[Entry] = &[
    ("⬆️", "up arrow top"),
    ("⬇️", "down arrow bottom minimize"),
    ("⬅️", "left arrow previous"),
    ("➡️", "right arrow next move"),
    ("↗️", "up right arrow maximize"),
    ("↘️", "down right arrow"),
    ("↕️", "up down arrow vertical"),
    ("↔️", "left right arrow horizontal"),
    ("🔼", "up button shade roll"),
    ("🔽", "down button unroll"),
    ("⏫", "fast up roll shade"),
    ("⏬", "fast down"),
    ("◀️", "left triangle previous back"),
    ("▶️", "right triangle next play"),
    ("⏮️", "previous track first"),
    ("⏭️", "next track last"),
    ("↩️", "return back undo restore"),
    ("🔄", "reload refresh cycle"),
    ("🔃", "reload clockwise refresh"),
    ("🔁", "repeat loop restore"),
    ("🔀", "shuffle move swap"),
];
const TRAY: &[Entry] = &[
    ("📤", "outbox tray send traynize"),
    ("📥", "inbox tray receive iconize"),
    ("📦", "package box iconize"),
    ("🙈", "hide see no evil"),
    ("👻", "ghost transparent fade hide"),
    ("🫥", "dotted face invisible fade"),
    ("🌫️", "fog fade transparency"),
    ("🚫", "prohibited hide no"),
    ("👁️", "eye show visible"),
    ("💤", "sleep zzz"),
];
const PIN: &[Entry] = &[
    ("📌", "pin pushpin top"),
    ("📍", "pin round location"),
    ("🧷", "safety pin"),
    ("🔝", "top on top always"),
    ("🔒", "lock locked"),
    ("🔓", "unlock unlocked"),
    ("⭐", "star favourite"),
    ("🏷️", "label tag"),
];
const DESKTOPS: &[Entry] = &[
    ("🖥️", "desktop computer monitor screen"),
    ("💻", "laptop computer"),
    ("🆕", "new"),
    ("🔢", "numbers digits list desktops"),
    ("1️⃣", "one 1 first"),
    ("2️⃣", "two 2 second"),
    ("3️⃣", "three 3 third"),
    ("4️⃣", "four 4 fourth"),
    ("🗺️", "map overview"),
];
const POINTER: &[Entry] = &[
    ("🎯", "target center cursor focus"),
    ("🖱️", "mouse cursor pointer"),
    ("👆", "point up finger"),
    ("👉", "point right finger"),
    ("🔍", "search magnifier zoom"),
    ("✨", "sparkles magic"),
    ("💡", "idea light bulb"),
];
const MARKS: &[Entry] = &[
    ("✅", "check done ok yes"),
    ("✔️", "check mark tick"),
    ("⚠️", "warning caution"),
    ("❓", "question help"),
    ("⛔", "no entry stop quit"),
    ("🔴", "red circle record"),
    ("🟢", "green circle go"),
    ("🔵", "blue circle"),
    ("⚙️", "gear settings"),
    ("🔧", "wrench tool fix"),
    ("⚡", "lightning fast power"),
    ("🔥", "fire hot"),
    ("🗑️", "trash bin delete close"),
    ("♻️", "recycle reload"),
    ("💾", "save disk floppy"),
];

impl IconCategory {
    pub const ALL: [Self; 7] =
        [Self::Windows, Self::Arrows, Self::Tray, Self::Pin, Self::Desktops, Self::Pointer, Self::Marks];

    pub fn title(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Arrows => "Arrows",
            Self::Tray => "Tray & hide",
            Self::Pin => "Pin & top",
            Self::Desktops => "Desktops",
            Self::Pointer => "Pointer",
            Self::Marks => "Marks",
        }
    }

    pub fn from_title(title: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.title() == title)
    }

    fn entries(self) -> &'static [Entry] {
        match self {
            Self::Windows => WINDOWS,
            Self::Arrows => ARROWS,
            Self::Tray => TRAY,
            Self::Pin => PIN,
            Self::Desktops => DESKTOPS,
            Self::Pointer => POINTER,
            Self::Marks => MARKS,
        }
    }

    pub fn icons(self) -> impl Iterator<Item = &'static str> {
        self.entries().iter().map(|(icon, _)| *icon)
    }

    /// The category `icon` is listed under, if it's in the catalog.
    pub fn of(icon: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.icons().any(|i| i == icon))
    }
}

/// Catalog icons whose keywords contain every word of `query`
/// (case-insensitive). Empty query, no results.
pub fn search(query: &str) -> Vec<&'static str> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    IconCategory::ALL
        .into_iter()
        .flat_map(IconCategory::entries)
        .filter(|(_, keywords)| words.iter().all(|word| keywords.split(' ').any(|k| k.starts_with(word.as_str()))))
        .map(|(icon, _)| *icon)
        .collect()
}

/// A few fitting icons for `action`, best first. The first is what a tile
/// gets automatically ([`default_for`]).
pub fn suggestions(action: Action) -> &'static [&'static str] {
    match action {
        Action::Overlay => &["🎛️", "🗂️", "✨"],
        Action::Reload => &["🔄", "🔃", "♻️"],
        Action::Quit => &["⛔", "🚪", "❌"],
        Action::Minimize => &["➖", "🔽", "⬇️"],
        Action::Maximize => &["🔲", "⬜", "↗️"],
        Action::Close => &["❌", "✖️", "🗑️"],
        Action::AlwaysOnTop => &["🔝", "📌", "⬆️"],
        Action::Borderless => &["🖼️", "⬜", "🔳"],
        Action::Transparency => &["🌫️", "👻", "🫥"],
        Action::Shade => &["🔼", "⏫", "🙈"],
        Action::HideTaskbar => &["🚫", "🙈", "👻"],
        Action::CenterCursor => &["🎯", "🖱️", "✨"],
        Action::Iconize => &["📥", "📦", "⬇️"],
        Action::Traynize => &["📤", "📥", "📦"],
        Action::Uniconize => &["↩️", "📤", "🔁"],
        Action::UniconizeAll => &["📂", "🗃️", "🔁"],
        Action::DesktopNext => &["▶️", "➡️", "⏭️"],
        Action::DesktopPrev => &["◀️", "⬅️", "⏮️"],
        Action::DesktopNew => &["➕", "🆕", "🖥️"],
        Action::DesktopMove => &["🔀", "🖥️", "➡️"],
        Action::PinToggle => &["📌", "📍", "🧷"],
        Action::DesktopList => &["🗂️", "🖥️", "🔢"],
    }
}

/// The icon a tile gets when its action is set and it has no icon of its
/// own yet — or still has the previous action's default, i.e. one nobody
/// chose by hand.
pub fn default_for(action: Action) -> &'static str {
    suggestions(action)[0]
}

/// The icon a tile should have after its action changes from `previous`
/// to `next`: `next`'s default if the current icon is empty or was just
/// `previous`'s default, otherwise unchanged (someone picked it).
pub fn after_action_change(current: Option<&str>, previous: Action, next: Action) -> Option<String> {
    match current {
        None => Some(default_for(next).to_owned()),
        Some(icon) if icon == default_for(previous) => Some(default_for(next).to_owned()),
        Some(icon) => Some(icon.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_icon_is_listed_twice() {
        let mut seen = Vec::new();
        for category in IconCategory::ALL {
            for icon in category.icons() {
                assert!(!seen.contains(&icon), "{icon} in {category:?} is listed twice");
                seen.push(icon);
            }
        }
    }

    #[test]
    fn every_suggestion_is_in_the_catalog() {
        for action in crate::overlay::OVERLAY_ACTIONS {
            for icon in suggestions(action) {
                assert!(IconCategory::of(icon).is_some(), "{icon} suggested for {action} isn't in the catalog");
            }
        }
    }

    #[test]
    fn search_matches_keyword_prefixes() {
        assert_eq!(search("pin"), ["📌", "📍", "🧷"]);
        assert!(search("clo").contains(&"❌"));
        assert!(search("  ").is_empty());
    }

    #[test]
    fn a_hand_picked_icon_survives_an_action_change() {
        let minimize = default_for(Action::Minimize);
        assert_eq!(after_action_change(None, Action::Minimize, Action::Close).as_deref(), Some("❌"));
        assert_eq!(after_action_change(Some(minimize), Action::Minimize, Action::Close).as_deref(), Some("❌"));
        assert_eq!(after_action_change(Some("⭐"), Action::Minimize, Action::Close).as_deref(), Some("⭐"));
    }
}
