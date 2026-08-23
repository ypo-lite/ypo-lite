//! What y-snap can be asked to do, and to which window.
//!
//! One vocabulary for both entry points. HulK kept two lists — one for
//! hotkeys, one for the overlay — which drifted: `shade` and `pin` existed
//! only in the overlay, `uniconize` only as a hotkey, for no reason a user
//! could have guessed. Here every action can be bound to a key *and* placed
//! on a tile, and the configuration file uses the same names for both.

use std::fmt;

/// Everything the configuration file can ask for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    // y-snap itself.
    /// Show the action overlay for the target window.
    Overlay,
    /// Re-read the configuration file.
    Reload,
    /// Quit y-snap.
    Quit,

    // The target window.
    Minimize,
    /// Maximize, or restore an already maximized window.
    Maximize,
    Close,
    /// Keep the window above the others, or stop.
    AlwaysOnTop,
    /// Strip the title bar and frame, or put them back.
    Borderless,
    /// Make the window semi-transparent, or opaque again.
    Transparency,
    /// Roll the window up into its title bar, or unroll it.
    Shade,
    /// Hide the window from the taskbar, or show it again.
    HideTaskbar,
    /// Move the mouse pointer to the middle of the window.
    CenterCursor,

    // Hiding windows away.
    /// Hide the window behind an icon in y-snap's own list.
    Iconize,
    /// Hide the window behind its own icon in the notification area.
    Traynize,
    /// Bring back the most recently hidden window.
    Uniconize,

    // Virtual desktops.
    DesktopNext,
    DesktopPrev,
    DesktopNew,
    /// Move the window to the desktop named by the `desktop` attribute.
    DesktopMove,
    /// Only on an overlay tile: expands into one tile per virtual desktop
    /// when the overlay opens, so the tiles follow desktops being created and
    /// removed instead of having to be written out by hand.
    DesktopList,
    /// Show the window on every desktop, or only on its own.
    PinToggle,
}

impl Action {
    /// The name used in the configuration file.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "overlay" => Self::Overlay,
            "reload" => Self::Reload,
            "quit" => Self::Quit,

            "minimize" => Self::Minimize,
            "maximize" => Self::Maximize,
            "close" => Self::Close,
            "always-on-top" => Self::AlwaysOnTop,
            "borderless" => Self::Borderless,
            "transparency" => Self::Transparency,
            "shade" => Self::Shade,
            "hide-taskbar" => Self::HideTaskbar,
            "center-cursor" => Self::CenterCursor,

            "iconize" => Self::Iconize,
            "traynize" => Self::Traynize,
            "uniconize" => Self::Uniconize,

            "desktop-next" => Self::DesktopNext,
            "desktop-prev" => Self::DesktopPrev,
            "desktop-new" => Self::DesktopNew,
            "desktop-move" => Self::DesktopMove,
            "desktop-list" => Self::DesktopList,
            "pin-toggle" => Self::PinToggle,

            _ => return None,
        })
    }

    /// The name [`Action::from_name`] reads back.
    pub fn name(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Reload => "reload",
            Self::Quit => "quit",
            Self::Minimize => "minimize",
            Self::Maximize => "maximize",
            Self::Close => "close",
            Self::AlwaysOnTop => "always-on-top",
            Self::Borderless => "borderless",
            Self::Transparency => "transparency",
            Self::Shade => "shade",
            Self::HideTaskbar => "hide-taskbar",
            Self::CenterCursor => "center-cursor",
            Self::Iconize => "iconize",
            Self::Traynize => "traynize",
            Self::Uniconize => "uniconize",
            Self::DesktopNext => "desktop-next",
            Self::DesktopPrev => "desktop-prev",
            Self::DesktopNew => "desktop-new",
            Self::DesktopMove => "desktop-move",
            Self::DesktopList => "desktop-list",
            Self::PinToggle => "pin-toggle",
        }
    }

    /// Whether the action needs a window to act on. The ones that do are not
    /// worth firing at all without one — pressing the hotkey over the desktop
    /// should do nothing, not act on whatever was focused before.
    pub fn needs_window(self) -> bool {
        !matches!(
            self,
            Self::Reload
                | Self::Quit
                | Self::Uniconize
                | Self::DesktopNext
                | Self::DesktopPrev
                | Self::DesktopNew
        )
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Which window an action applies to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Target {
    /// The window with the keyboard focus.
    #[default]
    Focused,
    /// The window under the mouse pointer, falling back to the focused one.
    /// What HulK's hotkeys mostly used: the hand is already on the mouse.
    Pointed,
}

impl Target {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim() {
            "focused" => Some(Self::Focused),
            "pointed" => Some(Self::Pointed),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_round_trips_through_its_name() {
        for action in [
            Action::Overlay,
            Action::Reload,
            Action::Quit,
            Action::Minimize,
            Action::Maximize,
            Action::Close,
            Action::AlwaysOnTop,
            Action::Borderless,
            Action::Transparency,
            Action::Shade,
            Action::HideTaskbar,
            Action::CenterCursor,
            Action::Iconize,
            Action::Traynize,
            Action::Uniconize,
            Action::DesktopNext,
            Action::DesktopPrev,
            Action::DesktopNew,
            Action::DesktopMove,
            Action::DesktopList,
            Action::PinToggle,
        ] {
            assert_eq!(Action::from_name(action.name()), Some(action));
        }
    }

    #[test]
    fn rejects_unknown_names() {
        for name in ["", "minimise", "always_on_top", "Minimize"] {
            assert_eq!(Action::from_name(name), None, "{name:?}");
        }
    }
}
