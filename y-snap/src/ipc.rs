//! What another process may say to a running y-snap, and how to find out
//! whether one is running at all. In the library so the tray app and
//! `y-snap-config` derive every name from the same constants.
//!
//! Only one thing so far: "reload `y-snap.xml`", which `y-snap-config` sends
//! after a successful save so the change applies without a trip to the tray
//! menu. It's a registered window message broadcast to top-level windows —
//! the same mechanism `y_core::app` uses for its "activate" message, and why
//! y-snap's main window is hidden top-level rather than message-only (see
//! `skeleton/README.md`). Any process can send it; the worst it can do is
//! make y-snap re-read its own file.
//!
//! A y-snap running elevated won't receive it from a non-elevated sender
//! (UIPI filters posted messages upward). y-snap doesn't run elevated, so
//! this isn't worked around.

use y_core::overlay::Color;
use y_core::single_instance;

use crate::config::{Look, Shape, finder_range};

/// y-snap's main window class — also its single-instance mutex name
/// (`AppInfo::single_instance` derives one from the other).
pub const CLASS: &str = "YpoLite.YSnap";

/// The value name under `HKCU\...\Run` for "Start with Windows" — the same one
/// the installer's start-up task writes, so the two stay one setting.
pub const STARTUP_NAME: &str = "Y-Snap";

/// The settings app's executable, looked for next to `y-snap.exe` by the
/// tray's "Settings…" item.
pub const CONFIG_EXE: &str = "y-snap-config.exe";

/// The settings app's single-instance mutex name.
pub const CONFIG_INSTANCE: &str = "YpoLite.YSnapConfig";

/// The settings app's window title — what a second launch finds the open
/// one by (a WinUI window's class name is the framework's, not ours).
pub const CONFIG_TITLE: &str = "y-snap — Configuration";

/// The registered message that asks a running y-snap to reload its
/// configuration. Same id in every process that asks for it.
pub fn reload_message() -> u32 {
    single_instance::broadcast_message(&format!("{CLASS}.Reload"))
}

/// Whether y-snap is running in this logon session.
pub fn is_running() -> bool {
    single_instance::is_running(CLASS)
}

/// Asks a running y-snap to reload `y-snap.xml`. Returns whether there was
/// one to ask — not whether the reload succeeded: y-snap reports a broken
/// file itself, in its own notification.
pub fn request_reload() -> bool {
    if !is_running() {
        return false;
    }
    single_instance::notify_running_instance(reload_message());
    true
}

/// The registered message that asks a running y-snap to play a finder look
/// once, as the settings app's Test button does with a style that is not
/// saved yet. The look travels in the message's two parameters, see
/// [`pack_look`]. Same id in every process that asks for it.
pub fn finder_test_message() -> u32 {
    single_instance::broadcast_message(&format!("{CLASS}.FinderTest"))
}

/// Asks a running y-snap to play `look` now. Returns whether there was one to
/// ask.
pub fn request_finder_test(look: &Look) -> bool {
    if !is_running() {
        return false;
    }
    let (wparam, lparam) = pack_look(look);
    single_instance::notify_running_instance_with(finder_test_message(), wparam, lparam);
    true
}

/// A [`Look`] as the two message parameters. Nothing else is needed to play
/// it: a shape, a few small numbers and a colour. `recall` is left out (a test
/// never moves the pointer).
///
/// `wParam`: shape (bits 0–7), auto colour (8), dimming (9–15), duration in
/// ms (16–31), size (32–47). `lParam`: the colour as `0xRRGGBB`.
pub fn pack_look(look: &Look) -> (usize, isize) {
    let shape = Shape::ALL
        .iter()
        .position(|s| *s == look.shape)
        .unwrap_or(0) as u64;
    let wparam = shape
        | u64::from(look.auto_color) << 8
        | u64::from(look.dim.min(100)) << 9
        | u64::from(look.duration_ms.min(u32::from(u16::MAX))) << 16
        | u64::from(look.size.min(u32::from(u16::MAX))) << 32;
    let color =
        u32::from(look.color.r) << 16 | u32::from(look.color.g) << 8 | u32::from(look.color.b);
    (wparam as usize, color as isize)
}

/// The look [`pack_look`] made, or `None` for parameters no y-snap would send.
/// Any process can post this message, so what comes out is held to the ranges
/// the configuration file itself obeys.
pub fn unpack_look(wparam: usize, lparam: isize) -> Option<Look> {
    let wparam = wparam as u64;
    let shape = *Shape::ALL.get((wparam & 0xFF) as usize)?;
    let in_range = |value: u64, range: std::ops::RangeInclusive<u32>| {
        u32::try_from(value).ok().filter(|v| range.contains(v))
    };
    let color = u32::try_from(lparam).ok().filter(|c| *c <= 0x00FF_FFFF)?;
    Some(Look {
        shape,
        auto_color: wparam >> 8 & 1 == 1,
        dim: in_range(wparam >> 9 & 0x7F, finder_range::DIM)?,
        duration_ms: in_range(wparam >> 16 & 0xFFFF, finder_range::DURATION_MS)?,
        size: in_range(wparam >> 32 & 0xFFFF, finder_range::SIZE)?,
        color: Color::rgb((color >> 16) as u8, (color >> 8) as u8, color as u8),
        ..Look::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_survives_the_trip() {
        let look = Look {
            shape: Shape::Spotlight,
            duration_ms: 2500,
            size: 333,
            color: Color::rgb(1, 2, 3),
            auto_color: true,
            dim: 80,
            ..Look::default()
        };
        let (w, l) = pack_look(&look);
        assert_eq!(unpack_look(w, l), Some(look));
        let default = Look::default();
        let (w, l) = pack_look(&default);
        assert_eq!(unpack_look(w, l), Some(default));
    }

    #[test]
    fn what_no_y_snap_would_send_is_refused() {
        assert_eq!(unpack_look(0xFF, 0), None, "unknown shape");
        assert_eq!(unpack_look(0, -1), None, "negative colour");
        assert_eq!(unpack_look(0, 0x0100_0000), None, "colour too wide");
        // A zero duration is outside the accepted range.
        assert_eq!(unpack_look(0, 0), None);
    }
}
