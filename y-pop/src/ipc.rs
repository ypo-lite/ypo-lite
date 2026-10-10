//! What another process may say to a running y-pop, and how to find out
//! whether one is running at all. In the library so the tray app and
//! `y-pop-config` derive every name from the same constants. Same design as
//! `y_snap::ipc` — see its module doc for why a registered window message is
//! enough, and for the elevated-sender caveat.
//!
//! One thing so far: "reload `y-pop.xml`", sent by `y-pop-config` after a
//! successful save.

use y_core::single_instance;

/// y-pop's main window class — also its single-instance mutex name
/// (`AppInfo::single_instance` derives one from the other).
pub const CLASS: &str = "YpoLite.YPop";

/// The settings app's executable, looked for next to `y-pop.exe` by the
/// `<settings/>` menu item.
pub const CONFIG_EXE: &str = "y-pop-config.exe";

/// The settings app's single-instance mutex name.
pub const CONFIG_INSTANCE: &str = "YpoLite.YPopConfig";

/// The settings app's window title — what a second launch finds the open
/// one by (a WinUI window's class name is the framework's, not ours).
pub const CONFIG_TITLE: &str = "y-pop — Configuration";

/// The registered message that asks a running y-pop to reload its
/// configuration. Same id in every process that asks for it.
pub fn reload_message() -> u32 {
    single_instance::broadcast_message(&format!("{CLASS}.Reload"))
}

/// Whether y-pop is running in this logon session.
pub fn is_running() -> bool {
    single_instance::is_running(CLASS)
}

/// Asks a running y-pop to reload `y-pop.xml`. Returns whether there was one
/// to ask — not whether the reload succeeded: y-pop keeps its previous
/// configuration and reports a broken file itself.
pub fn request_reload() -> bool {
    if !is_running() {
        return false;
    }
    single_instance::notify_running_instance(reload_message());
    true
}
