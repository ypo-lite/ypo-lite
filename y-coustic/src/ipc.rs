//! What another process may say to a running y-coustic, and how to find out
//! whether one is running at all. In the library so the tray application and
//! `y-coustic-config` derive every name from the same constants. Same design
//! as `y_snap::ipc` and `y_pop::ipc`: see the first one's module doc for why a
//! registered window message is enough, and for the elevated-sender caveat.
//!
//! One thing so far: "reload `y-coustic.xml`", which the settings app sends
//! after a successful save so the change applies without a trip to the tray
//! menu.

use y_core::single_instance;

/// y-coustic's main window class, and its single-instance mutex name.
pub const CLASS: &str = "YpoLite.YCoustic";

/// The value name under `HKCU\...\Run` for "Start with Windows".
pub const STARTUP_NAME: &str = "Y-Coustic";

/// The settings app's executable, looked for next to `y-coustic.exe` by the
/// tray's *Settings…* item.
pub const CONFIG_EXE: &str = "y-coustic-config.exe";

/// The settings app's single-instance mutex name.
pub const CONFIG_INSTANCE: &str = "YpoLite.YCousticConfig";

/// The settings app's window title: what a second launch finds the open one by
/// (a WinUI window's class name is the framework's, not ours).
pub const CONFIG_TITLE: &str = "y-coustic — Configuration";

/// The registered message that asks a running y-coustic to reload its
/// configuration. Same id in every process that asks for it.
pub fn reload_message() -> u32 {
    single_instance::broadcast_message(&format!("{CLASS}.Reload"))
}

/// Whether y-coustic is running in this logon session.
pub fn is_running() -> bool {
    single_instance::is_running(CLASS)
}

/// Asks a running y-coustic to reload `y-coustic.xml`. Returns whether there
/// was one to ask, not whether the reload succeeded: y-coustic keeps its
/// previous configuration and reports a broken file itself.
pub fn request_reload() -> bool {
    if !is_running() {
        return false;
    }
    single_instance::notify_running_instance(reload_message());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reload_message_is_registered_and_stable() {
        let message = reload_message();
        assert_ne!(message, 0);
        assert_eq!(message, reload_message());
    }

    #[test]
    fn names_belong_to_y_coustic() {
        assert!(CLASS.ends_with("YCoustic"));
        assert!(CONFIG_INSTANCE.starts_with(CLASS));
        assert!(CONFIG_EXE.starts_with("y-coustic"));
    }
}
