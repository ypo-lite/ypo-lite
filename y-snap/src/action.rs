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
    /// Hide the window behind an icon of its own in the notification area.
    Iconize,
    /// Hide the window and list it in y-snap's tray menu.
    Traynize,
    /// Bring back the most recently hidden window.
    Uniconize,
    /// Bring back every hidden window.
    UniconizeAll,

    // Virtual desktops.
    DesktopNext,
    DesktopPrev,
    DesktopNew,
    /// Move the window to the desktop named by the `desktop` attribute, and
    /// switch to that desktop.
    DesktopMove,
    /// Move the window to the next desktop (wrapping from the last to the
    /// first) and switch to it.
    DesktopMoveNext,
    /// Move the window to the previous desktop (wrapping from the first to the
    /// last) and switch to it.
    DesktopMovePrev,
    /// Like `desktop-move`, but stay on the current desktop.
    DesktopSend,
    /// Like `desktop-move-next`, but stay on the current desktop.
    DesktopSendNext,
    /// Like `desktop-move-prev`, but stay on the current desktop.
    DesktopSendPrev,
    /// Only on an overlay tile: expands into one tile per virtual desktop
    /// when the overlay opens, so the tiles follow desktops being created and
    /// removed instead of having to be written out by hand.
    DesktopList,
    /// Like `desktop-list`, but the tiles `desktop-send` instead of
    /// `desktop-move`: they send the window and stay on the current desktop.
    DesktopSendList,
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
            "uniconize-all" => Self::UniconizeAll,

            "desktop-next" => Self::DesktopNext,
            "desktop-prev" => Self::DesktopPrev,
            "desktop-new" => Self::DesktopNew,
            "desktop-move" => Self::DesktopMove,
            "desktop-move-next" => Self::DesktopMoveNext,
            "desktop-move-prev" => Self::DesktopMovePrev,
            "desktop-send" => Self::DesktopSend,
            "desktop-send-next" => Self::DesktopSendNext,
            "desktop-send-prev" => Self::DesktopSendPrev,
            "desktop-list" => Self::DesktopList,
            "desktop-send-list" => Self::DesktopSendList,
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
            Self::UniconizeAll => "uniconize-all",
            Self::DesktopNext => "desktop-next",
            Self::DesktopPrev => "desktop-prev",
            Self::DesktopNew => "desktop-new",
            Self::DesktopMove => "desktop-move",
            Self::DesktopMoveNext => "desktop-move-next",
            Self::DesktopMovePrev => "desktop-move-prev",
            Self::DesktopSend => "desktop-send",
            Self::DesktopSendNext => "desktop-send-next",
            Self::DesktopSendPrev => "desktop-send-prev",
            Self::DesktopList => "desktop-list",
            Self::DesktopSendList => "desktop-send-list",
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
                | Self::UniconizeAll
                | Self::DesktopNext
                | Self::DesktopPrev
                | Self::DesktopNew
        )
    }

    /// Whether the action reads the `desktop` attribute: the numbered pair,
    /// as opposed to `-next`/`-prev`, which go by the current desktop.
    pub fn takes_desktop(self) -> bool {
        matches!(self, Self::DesktopMove | Self::DesktopSend)
    }

    /// `desktop-list` or `desktop-send-list`: not a tile but a generator of
    /// them, one per virtual desktop, only meaningful on an overlay.
    pub fn is_desktop_list(self) -> bool {
        matches!(self, Self::DesktopList | Self::DesktopSendList)
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

    /// The name [`Target::from_name`] reads back.
    pub fn name(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Pointed => "pointed",
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
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
            Action::UniconizeAll,
            Action::DesktopNext,
            Action::DesktopPrev,
            Action::DesktopNew,
            Action::DesktopMove,
            Action::DesktopMoveNext,
            Action::DesktopMovePrev,
            Action::DesktopSend,
            Action::DesktopSendNext,
            Action::DesktopSendPrev,
            Action::DesktopList,
            Action::DesktopSendList,
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

    #[test]
    fn every_target_round_trips_through_its_name() {
        for target in [Target::Focused, Target::Pointed] {
            assert_eq!(Target::from_name(target.name()), Some(target));
        }
    }
}
