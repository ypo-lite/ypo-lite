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

use y_core::single_instance;

/// y-snap's main window class — also its single-instance mutex name
/// (`AppInfo::single_instance` derives one from the other).
pub const CLASS: &str = "YpoLite.YSnap";

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
