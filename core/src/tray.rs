//! RAII wrapper around a shell notification (tray) icon.
//!
//! Dropping a [`TrayIcon`] removes it from the notification area, so the icon
//! can never survive its owner — the classic "ghost icon left behind" bug.
//!
//! The owner window must forward the `TaskbarCreated` broadcast to
//! [`TrayIcon::restore`]; [`crate::app::TrayApp`] does it for you.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIIF_LARGE_ICON,
    NIIF_NONE, NIIF_NOSOUND, NIIF_RESPECT_QUIET_TIME, NIIF_USER, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFY_ICON_INFOTIP_FLAGS, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP,
};
use windows::core::Result;

/// `NIN_KEYSELECT` (`WM_USER + 1`) — the keyboard counterpart to
/// `NIN_SELECT` (Enter/Space with the icon focused via Tab), not bound by
/// the `windows` crate. Documented in `shellapi.h` alongside `NIN_SELECT`,
/// which the crate does bind, immediately below it in value.
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;

// The balloon notifications, likewise defined relative to `NIN_SELECT`
// (`WM_USER` + n in `shellapi.h`) rather than trusting the binding to have them.
const NIN_BALLOONSHOW: u32 = NIN_SELECT + 2;
const NIN_BALLOONHIDE: u32 = NIN_SELECT + 3;
const NIN_BALLOONTIMEOUT: u32 = NIN_SELECT + 4;
const NIN_BALLOONUSERCLICK: u32 = NIN_SELECT + 5;

use crate::resources::{copy_to_buffer, copy_to_buffer_ellipsis};

pub struct TrayIcon {
    data: NOTIFYICONDATAW,
}

impl TrayIcon {
    /// Registers an icon in the notification area.
    ///
    /// * `owner` receives `callback_message` for every mouse event on the icon
    ///   — decode it with [`TrayEvent::decode`].
    /// * `id` must be unique among the icons owned by `owner`
    ///   ([`crate::ids::TRAY_ICON_ID`] for the usual single one).
    /// * `icon` should be a small (`SM_CXSMICON`) icon; see
    ///   [`crate::resources::load_app_icon`]. The shell does not take
    ///   ownership of the handle, so a shared resource icon is the right fit.
    pub fn new(
        owner: HWND,
        id: u32,
        callback_message: u32,
        icon: HICON,
        tooltip: &str,
    ) -> Result<Self> {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: owner,
            uID: id,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: callback_message,
            hIcon: icon,
            ..Default::default()
        };
        copy_to_buffer(tooltip, &mut data.szTip);

        let mut icon = Self { data };
        icon.register()?;
        Ok(icon)
    }

    /// Adds the icon to the shell and opts into the version 4 callback
    /// protocol (screen coordinates in `wParam`, event id in `lParam`).
    fn register(&mut self) -> Result<()> {
        unsafe { Shell_NotifyIconW(NIM_ADD, &self.data) }.ok()?;

        self.data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        unsafe { Shell_NotifyIconW(NIM_SETVERSION, &self.data) }.ok()?;
        Ok(())
    }

    /// Re-adds the icon after Explorer restarted (the `TaskbarCreated`
    /// broadcast).
    pub fn restore(&mut self) -> Result<()> {
        // The previous instance is already gone with the old taskbar; a stale
        // NIM_DELETE would fail harmlessly, so go straight to NIM_ADD.
        self.register()
    }

    /// Updates the tooltip shown when hovering the icon.
    pub fn set_tooltip(&mut self, tooltip: &str) -> Result<()> {
        copy_to_buffer(tooltip, &mut self.data.szTip);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok()
    }

    /// Swaps the icon, for applications that reflect state in the tray
    /// (muted / unmuted, active / idle).
    pub fn set_icon(&mut self, icon: HICON) -> Result<()> {
        self.data.hIcon = icon;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok()
    }

    /// Shows a balloon / toast notification from the icon.
    ///
    /// The shell may drop it silently — quiet hours, focus assist, or the user
    /// having turned notifications off for the app — so never make anything
    /// depend on it having been seen. Whether it was clicked is reported as
    /// [`TrayEvent::BalloonClicked`] on the icon's callback message.
    pub fn notify(&mut self, notification: &Notification) -> Result<()> {
        let flags = self.data.uFlags;
        self.data.uFlags |= NIF_INFO;
        self.data.dwInfoFlags = notification.info_flags();
        if let Notify::User(icon) = notification.level {
            self.data.hBalloonIcon = icon;
        }
        // Over-long text ends in an ellipsis rather than being cut silently.
        copy_to_buffer_ellipsis(notification.title, &mut self.data.szInfoTitle);
        copy_to_buffer_ellipsis(notification.body, &mut self.data.szInfo);

        let result = unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) }.ok();

        // NIF_INFO is a one-shot: leaving it set would repeat the balloon on
        // the next unrelated NIM_MODIFY.
        self.data.uFlags = flags;
        self.data.hBalloonIcon = Default::default();
        result
    }
}

/// A balloon notification: build with [`Notification::new`] and the chained
/// setters, show with [`TrayIcon::notify`].
///
/// The shell limits the title to 63 and the body to 255 UTF-16 units; anything
/// longer is shortened with a trailing `…`. Five short lines is about what a
/// body can carry — callers assembling many lines budget them themselves.
#[derive(Clone, Copy)]
pub struct Notification<'a> {
    title: &'a str,
    body: &'a str,
    level: Notify,
    large_icon: bool,
    silent: bool,
    respect_quiet_time: bool,
}

impl<'a> Notification<'a> {
    pub fn new(title: &'a str, body: &'a str) -> Self {
        Self {
            title,
            body,
            level: Notify::None,
            large_icon: false,
            silent: false,
            respect_quiet_time: false,
        }
    }

    /// The icon shown beside the text (none by default).
    pub fn level(mut self, level: Notify) -> Self {
        self.level = level;
        self
    }

    /// Shows a [`Notify::User`] icon at large size. Only meaningful with
    /// that level.
    pub fn large_icon(mut self) -> Self {
        self.large_icon = true;
        self
    }

    /// No notification sound.
    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// Withheld while the user is in a full-screen app, presentation or
    /// game, instead of interrupting it. Use for anything that can wait.
    pub fn respect_quiet_time(mut self) -> Self {
        self.respect_quiet_time = true;
        self
    }

    fn info_flags(&self) -> NOTIFY_ICON_INFOTIP_FLAGS {
        let mut flags = self.level.flags();
        if self.large_icon {
            flags |= NIIF_LARGE_ICON;
        }
        if self.silent {
            flags |= NIIF_NOSOUND;
        }
        if self.respect_quiet_time {
            flags |= NIIF_RESPECT_QUIET_TIME;
        }
        flags
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // Nothing useful to do on failure while unwinding.
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data) };
    }
}

/// Icon shown next to a balloon notification.
#[derive(Clone, Copy, Default)]
pub enum Notify {
    #[default]
    None,
    Info,
    Warning,
    Error,
    /// A custom icon. The handle is borrowed: the shell copies it while
    /// showing the balloon, so a shared resource icon is the right fit.
    User(HICON),
}

impl Notify {
    fn flags(self) -> NOTIFY_ICON_INFOTIP_FLAGS {
        match self {
            Notify::None => NIIF_NONE,
            Notify::Info => NIIF_INFO,
            Notify::Warning => NIIF_WARNING,
            Notify::Error => NIIF_ERROR,
            Notify::User(_) => NIIF_USER,
        }
    }
}

/// A decoded tray callback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayEvent {
    /// Right click, or the keyboard menu key: show the context menu.
    ContextMenu,
    /// Double click, conventionally the icon's default action.
    DoubleClick,
    /// A plain (left) click, or the keyboard equivalent — the icon's
    /// primary, single-click action. Under the version 4 protocol every
    /// [`TrayIcon`] opts into, this is `NIN_SELECT`/`NIN_KEYSELECT`, not
    /// `WM_LBUTTONUP`, which version 4 does not send; `WM_LBUTTONUP` is
    /// matched too as a defensive fallback for a shell that ignored the
    /// version negotiation, not because it is expected to arrive.
    Select,
    /// The user clicked the body of a balloon shown by [`TrayIcon::notify`].
    /// An app that acts on it remembers what its last balloon was about:
    /// only one is live per icon at a time.
    BalloonClicked,
    /// The shell displayed a balloon. Not sent when it suppresses one
    /// (focus assist...), so nothing may depend on it arriving.
    BalloonShown,
    /// A balloon went away unclicked — it timed out, or was dismissed.
    BalloonClosed,
    /// Anything else — hover (`WM_MOUSEMOVE` fires repeatedly while the
    /// cursor sits over the icon) and the rest. Not a
    /// click: code that opens a menu or launches something on a tray
    /// callback must not treat this variant as one, or it fires on mere
    /// mouse movement.
    Other(u32),
}

impl TrayEvent {
    /// Unpacks a version 4 tray callback: the cursor position is packed into
    /// `wParam`, the event id into the low word of `lParam`.
    pub fn decode(wparam: WPARAM, lparam: LPARAM) -> (Self, POINT) {
        let position = POINT {
            x: (wparam.0 & 0xFFFF) as i16 as i32,
            y: ((wparam.0 >> 16) & 0xFFFF) as i16 as i32,
        };

        let event = match (lparam.0 as u32) & 0xFFFF {
            // WM_CONTEXTMENU covers right click and the keyboard menu key;
            // WM_RBUTTONUP is kept for older shells.
            WM_CONTEXTMENU | WM_RBUTTONUP => TrayEvent::ContextMenu,
            WM_LBUTTONDBLCLK => TrayEvent::DoubleClick,
            NIN_SELECT | NIN_KEYSELECT | WM_LBUTTONUP => TrayEvent::Select,
            NIN_BALLOONUSERCLICK => TrayEvent::BalloonClicked,
            NIN_BALLOONSHOW => TrayEvent::BalloonShown,
            NIN_BALLOONHIDE | NIN_BALLOONTIMEOUT => TrayEvent::BalloonClosed,
            other => TrayEvent::Other(other),
        };

        (event, position)
    }

    /// The id of the icon a version 4 callback came from — the `id` it was
    /// registered with — in the high word of `lParam`. Only an app owning
    /// more than one icon needs it.
    pub fn icon_id(lparam: LPARAM) -> u32 {
        ((lparam.0 as u32) >> 16) & 0xFFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 4 `lParam`: event in the low word, icon id in the high one.
    fn lparam(event: u32, id: u32) -> LPARAM {
        LPARAM(((id << 16) | event) as isize)
    }

    #[test]
    fn decodes_clicks_and_the_cursor_position() {
        let wparam = WPARAM(((200u32 << 16) | 100) as usize);
        let (event, position) = TrayEvent::decode(wparam, lparam(WM_CONTEXTMENU, 1));
        assert_eq!(event, TrayEvent::ContextMenu);
        assert_eq!((position.x, position.y), (100, 200));
    }

    #[test]
    fn decodes_balloon_events() {
        for (message, expected) in [
            (NIN_BALLOONUSERCLICK, TrayEvent::BalloonClicked),
            (NIN_BALLOONSHOW, TrayEvent::BalloonShown),
            (NIN_BALLOONTIMEOUT, TrayEvent::BalloonClosed),
            (NIN_BALLOONHIDE, TrayEvent::BalloonClosed),
        ] {
            let (event, _) = TrayEvent::decode(WPARAM(0), lparam(message, 1));
            assert_eq!(event, expected);
        }
    }

    #[test]
    fn hover_is_not_a_click() {
        let (event, _) = TrayEvent::decode(WPARAM(0), lparam(0x0200, 1));
        assert_eq!(event, TrayEvent::Other(0x0200));
    }

    #[test]
    fn reads_the_icon_id_from_the_high_word() {
        assert_eq!(TrayEvent::icon_id(lparam(WM_CONTEXTMENU, 7)), 7);
        assert_eq!(TrayEvent::icon_id(lparam(NIN_SELECT, 0xFFFF)), 0xFFFF);
    }
}
