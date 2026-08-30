//! Y-Pop — `docs/y-pop-plan.md`'s roadmap steps 1–5.
//!
//! Tray icon(s), hotkey registration and launching (including `start`/`exit`
//! events) work end to end. The popup is `menu`'s native `HMENU`, themed
//! (owner-draw colours, icons, titled separators) per `<y-pop theme="...">`
//! — see that module's doc comment. `WM_MEASUREITEM`/`WM_DRAWITEM` below are
//! what a themed popup needs forwarded from this window's own message loop;
//! an unthemed (`theme="native"`) popup never triggers either message, so
//! this has no effect there. The overlay renderer (roadmap step 6) is still
//! a later phase, built on a shared `MenuLayout` step this one doesn't have
//! yet.

#![windows_subsystem = "windows"]

mod browse;
mod builtins;
mod launch;
mod menu;
mod skins;

use windows::Win32::UI::WindowsAndMessaging::{
    HMENU, MB_ICONINFORMATION, MB_OK, MessageBoxW, WM_DRAWITEM, WM_HOTKEY, WM_INITMENUPOPUP,
    WM_MEASUREITEM,
};
use windows::core::{HSTRING, PCWSTR};

use y_core::{
    app::TrayApp,
    hotkey::{Fallback, HotkeyManager},
    ids::{TRAY_ICON_ID, WM_TRAY_CALLBACK},
    monitor,
    prelude::*,
    resources::{IconSize, app_icon_or_stock},
    tray::TrayEvent,
    window,
};

use y_pop::{
    config,
    model::{ConfigTree, EventMask, NodeId, NodeKind},
};

const APP: AppInfo = AppInfo::new("Y-Pop", "YpoLite.YPop").single_instance();

struct YPop {
    tree: ConfigTree,
    tray: Vec<TrayIcon>,
    /// Held only to keep every registration alive — dropping it (on
    /// destroy) unregisters everything at once.
    hotkeys: Option<HotkeyManager>,
    /// The currently-open popup's browse state, if any — `Some` for exactly
    /// as long as a `PopupMenu::track` call is on the stack inside
    /// `show_menu`. Lives here, not as a local in that method, because
    /// `WM_INITMENUPOPUP` needs to reach it while that call is still
    /// blocked — see `menu`'s module doc for why that reentrant call is
    /// safe to rely on here.
    session: Option<menu::Session>,
}

impl YPop {
    fn new() -> Self {
        y_core::log::line("loading configuration");

        let tree = match builtins::load() {
            Ok(tree) => tree,
            Err(error) => {
                y_core::log::line(&error);
                ConfigTree::empty(vec![error.to_string()])
            }
        };
        for warning in &tree.warnings {
            y_core::log::line(format_args!("{}: {warning}", config::FILE_NAME));
        }

        Self {
            tree,
            tray: Vec::new(),
            hotkeys: None,
            session: None,
        }
    }

    /// Fires every `App` node asking for `event`, wherever it sits in the
    /// tree — PortAL's `start`/`exit` events (see
    /// `docs/portal-analysis.md`'s feature inventory).
    fn fire_events(&self, event: EventMask) {
        for id in self.tree.all_nodes() {
            if let NodeKind::App(app) = &self.tree.node(id).kind
                && app.events.contains(event)
            {
                launch_and_report(HWND::default(), app);
            }
        }
    }

    /// Acting on `node`, wherever it was reached from: a tray icon click, a
    /// hotkey, or a selection out of an already-open popup. A `Group` opens
    /// its own popup at `position` rather than being a no-op, so this one
    /// function serves all three call sites.
    fn run(&mut self, hwnd: HWND, node: NodeId, position: POINT) {
        match &self.tree.node(node).kind {
            NodeKind::Group => self.show_menu(hwnd, node, position),
            NodeKind::App(app) => launch_and_report(hwnd, app),
            NodeKind::Quit => window::quit(0),
            NodeKind::About => show_about(),
            // Never reached: a menu never assigns a command id to one of
            // these (see `menu::append_children`), and neither does a
            // hotkey binding meaningfully target one — kept only so this
            // match stays exhaustive as `NodeKind` grows.
            NodeKind::Separator => {}
        }
    }

    fn show_menu(&mut self, hwnd: HWND, group: NodeId, position: POINT) {
        if self.session.is_some() {
            // A popup is already open. This is reachable re-entrantly — a
            // tray double-click delivers *both* NIN_SELECT (for the first
            // click; Explorer can't yet know it will become a double-click)
            // and WM_LBUTTONDBLCLK, each independently "clicked" per
            // `on_message`'s WM_TRAY_CALLBACK handling — and opening a
            // second popup here would overwrite `self.session`, permanently
            // breaking the first popup's WM_INITMENUPOPUP handling for the
            // rest of its lifetime: confirmed via a log trace showing
            // exactly that sequence. `session` has room for exactly one
            // open popup's browse bookkeeping, so a second activation
            // attempt while one is already open is simply ignored rather
            // than layered.
            return;
        }

        let (popup, session) = menu::build(&self.tree, group);
        // Reachable from `on_message`'s WM_INITMENUPOPUP arm while `track`
        // below is still blocked — see `YPop::session`'s doc comment.
        self.session = Some(session);

        let selected = popup.track(hwnd, position);
        // Taken, not just read: nothing after this point still needs a
        // browse folder's bookkeeping, and a stray WM_INITMENUPOPUP for
        // some *other* menu arriving after this popup has closed (should
        // not happen, but see `resolve_selection`'s doc on failing softly)
        // must not find a session that's already stale.
        let mut session = self.session.take().unwrap_or_else(menu::Session::new);

        if let Some(id) = selected
            && let Some(selection) = menu::resolve_selection(id, &mut session)
        {
            match selection {
                menu::Selection::Node(node) => self.run(hwnd, node, position),
                menu::Selection::Launch(app) => launch_and_report(hwnd, &app),
            }
        }
    }
}

fn launch_and_report(hwnd: HWND, app: &y_pop::model::App) {
    if let Err(error) = launch::launch(hwnd, app) {
        y_core::log::line(format_args!("launch failed: {error}"));
    }
}

fn show_about() {
    let message = HSTRING::from(format!("{} {}", APP.name, env!("CARGO_PKG_VERSION")));
    let caption = HSTRING::from(APP.name);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

impl TrayApp for YPop {
    fn on_create(&mut self, hwnd: HWND) -> Result<()> {
        let mut hotkeys = HotkeyManager::new(hwnd);
        for (id, binding) in self.tree.hotkey_bindings() {
            // Only ever installs the low-level hook once RegisterHotKey has
            // already refused the combination, never proactively — see
            // `HotkeyBinding::can_override`'s doc comment.
            let fallback = if binding.can_override {
                Fallback::LowLevelHook
            } else {
                Fallback::None
            };
            if let Err(error) = hotkeys.register(id, binding.hotkey, fallback) {
                y_core::log::line(format_args!(
                    "hotkey {} could not be registered: {error}",
                    binding.hotkey
                ));
            }
        }
        self.hotkeys = Some(hotkeys);

        let icon = app_icon_or_stock(IconSize::Small);
        for (index, &root) in self.tree.roots.iter().enumerate() {
            let name = &self.tree.node(root).name;
            let tray = TrayIcon::new(
                hwnd,
                TRAY_ICON_ID + index as u32,
                WM_TRAY_CALLBACK,
                icon,
                name,
            )?;
            self.tray.push(tray);
        }

        self.fire_events(EventMask::START);
        Ok(())
    }

    fn on_taskbar_created(&mut self) {
        for tray in &mut self.tray {
            let _ = tray.restore();
        }
    }

    fn on_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        if msg == WM_TRAY_CALLBACK {
            // Version 4's callback packs the clicked icon's own `uID` into
            // lParam's high word (TrayEvent::decode only unpacks the
            // position and the mouse event, not this) — see
            // `y_core::tray::TrayIcon::new`'s doc comment on the protocol.
            let icon_id = (lparam.0 as u32) >> 16;
            let (event, position) = TrayEvent::decode(wparam, lparam);

            // A tray icon receives far more than clicks over this one
            // message — WM_MOUSEMOVE alone fires repeatedly just from the
            // cursor sitting over it. Only an actual click (or its keyboard
            // equivalent) opens anything; TrayEvent::Other must not.
            let clicked = matches!(
                event,
                TrayEvent::Select | TrayEvent::ContextMenu | TrayEvent::DoubleClick
            );

            if clicked
                && let Some(index) = icon_id.checked_sub(TRAY_ICON_ID)
                && let Some(&root) = self.tree.roots.get(index as usize)
            {
                self.run(hwnd, root, position);
            }
            return Some(LRESULT(0));
        }

        // A themed popup's items are `MFT_OWNERDRAW` (see `menu`'s module
        // doc's "Theme" section) — Windows sends these two whenever one
        // needs sizing or painting. `handle_measure_item`/`handle_draw_item`
        // return `false` for anything not one of ours (a menu built without
        // a theme never triggers either message at all), in which case
        // falling through to `DefWindowProc` — the `None` every other
        // unhandled message here already gets — is correct.
        if msg == WM_MEASUREITEM && y_core::menu::handle_measure_item(lparam) {
            return Some(LRESULT(1));
        }
        if msg == WM_DRAWITEM && y_core::menu::handle_draw_item(lparam) {
            return Some(LRESULT(1));
        }

        if msg == WM_HOTKEY {
            if let Some(node) = self.tree.hotkey_owner(wparam.0 as u32) {
                let position = monitor::cursor_position().unwrap_or(POINT { x: 0, y: 0 });
                self.run(hwnd, node, position);
            }
            return Some(LRESULT(0));
        }

        // Arrives re-entrantly, while `show_menu`'s call to `track` is
        // still on the stack — see `YPop::session`'s doc comment and
        // `menu`'s module doc for why that's expected here, not a bug.
        if msg == WM_INITMENUPOPUP {
            if let Some(session) = self.session.as_mut() {
                let hmenu = HMENU(wparam.0 as *mut _);
                menu::on_init_menu_popup(session, hmenu);
            } else {
                // Should not happen now that `show_menu` refuses to open a
                // second popup over an open one (see its own doc comment) —
                // worth knowing about if it ever does again.
                y_core::log::line("on_message: WM_INITMENUPOPUP with no open session");
            }
            return Some(LRESULT(0));
        }

        None
    }

    fn on_destroy(&mut self) {
        self.fire_events(EventMask::EXIT);
        self.hotkeys = None; // unregisters every hotkey
        self.tray.clear(); // removes every tray icon
    }
}

fn main() -> ! {
    // Next to wherever y-pop.xml itself lives — same reasoning as that
    // file's own search order (portable next to the exe, or the roaming
    // profile): whichever place the user already knows to look in.
    let log_path = config::path().with_file_name("y-pop.log");
    let _ = y_core::log::init(&log_path);
    y_core::log::line("y-pop starting");

    y_core::app::start(&APP, || YPop::new().into_handler(&APP))
}
