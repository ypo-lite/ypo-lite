//! What the prefixes bring (spec §2.7): the y-pop commands of `>`, the lines of
//! `?`, and the kinds of row that are neither a result nor a section. Pure.

/// A y-pop command, run with `>`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Reload,
    OpenSettings,
    EditConfig,
    OpenFolder,
    ClearHistory,
    /// Registers y-pop to start at logon, or takes that away.
    StartWithWindows,
    About,
    Quit,
}

impl Command {
    /// In the order the list shows them. (*Refresh the Start menu* and *Reload
    /// plugins* of the spec join when there is a Start menu and plugins.)
    pub const ALL: [Self; 8] = [
        Self::Reload,
        Self::OpenSettings,
        Self::EditConfig,
        Self::OpenFolder,
        Self::ClearHistory,
        Self::StartWithWindows,
        Self::About,
        Self::Quit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Reload => "Reload configuration",
            Self::OpenSettings => "Open settings",
            Self::EditConfig => "Edit y-pop.xml",
            Self::OpenFolder => "Open configuration folder",
            Self::ClearHistory => "Clear search history",
            Self::StartWithWindows => "Start with Windows",
            Self::About => "About",
            Self::Quit => "Quit",
        }
    }

    /// The second line of its row.
    pub fn description(self) -> &'static str {
        match self {
            Self::Reload => "Read y-pop.xml again",
            Self::OpenSettings => "Open y-pop-config",
            Self::EditConfig => "Open the file in its editor",
            Self::OpenFolder => "Show the folder of y-pop.xml in Explorer",
            Self::ClearHistory => "Forget what was launched from search",
            Self::StartWithWindows => "Turn running y-pop at logon on or off",
            Self::About => "Version of y-pop",
            Self::Quit => "Close y-pop and its tray icons",
        }
    }

    /// Other words that find it (`restart` finds *Reload*).
    pub fn keywords(self) -> &'static str {
        match self {
            Self::Reload => "refresh restart config",
            Self::OpenSettings => "preferences options config",
            Self::EditConfig => "xml file text",
            Self::OpenFolder => "explorer directory",
            Self::ClearHistory => "recent forget reset",
            Self::StartWithWindows => "startup logon autostart boot",
            Self::About => "version info",
            Self::Quit => "exit close",
        }
    }
}

/// A row of a prefixed list that is not an item of the index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    Command(Command),
    /// A prefix, in the list of `?`: Enter types it.
    Prefix(char),
    /// The search hotkey itself, in the list of `#`.
    SearchHotkey,
    /// `#` and a free-or-taken answer for the combination typed
    /// (`State::check` has the combination).
    Check,
}

/// One prefix and what it does, for the line under the sections and for `?`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PrefixInfo {
    pub prefix: char,
    /// One word, on the chip.
    pub name: &'static str,
    pub description: &'static str,
}

/// The prefixes that work, in the order they are listed.
pub const PREFIXES: [PrefixInfo; 3] = [
    PrefixInfo {
        prefix: '>',
        name: "commands",
        description: "y-pop commands: reload, settings, history…",
    },
    PrefixInfo {
        prefix: '#',
        name: "hotkeys",
        description: "Hotkeys: every binding, or check whether a key is free",
    },
    PrefixInfo {
        prefix: '?',
        name: "help",
        description: "This list of prefixes",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_says_what_it_is() {
        for command in Command::ALL {
            assert!(!command.label().is_empty());
            assert!(!command.description().is_empty());
            assert!(!command.keywords().is_empty());
        }
        let mut labels: Vec<_> = Command::ALL.iter().map(|c| c.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), Command::ALL.len(), "no two alike");
    }

    #[test]
    fn prefixes_are_single_distinct_symbols() {
        let mut seen = Vec::new();
        for info in PREFIXES {
            assert!(!info.prefix.is_alphanumeric());
            assert!(!seen.contains(&info.prefix));
            seen.push(info.prefix);
        }
    }
}
