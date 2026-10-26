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
mod menu;
mod search_window;
mod tooltip;

use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_RIGHT};
use windows::Win32::UI::WindowsAndMessaging::{
    HMENU, MB_ICONINFORMATION, MB_OK, MessageBoxW, PostMessageW, WM_DRAWITEM, WM_HOTKEY,
    WM_INITMENUPOPUP, WM_KEYDOWN, WM_KEYUP, WM_MEASUREITEM, WM_MENUCHAR, WM_MENUSELECT, WM_TIMER,
};
use windows::core::{HSTRING, PCWSTR};

use y_core::{
    app::TrayApp,
    hotkey::{Fallback, HotkeyManager},
    ids::{TRAY_ICON_ID, WM_TRAY_CALLBACK},
    monitor,
    prelude::*,
    resources::{IconSize, app_icon_or_stock},
    tray::{Notification, Notify, TrayEvent},
    window,
};

use y_pop::{
    config, ipc,
    model::{ActionId, ConfigTree, EventMask, NodeId, NodeKind, TrayExtras},
};

const APP: AppInfo = AppInfo::new("Y-Pop", ipc::CLASS).single_instance();

/// How many warnings a balloon shows before pointing at the file. A
/// notification the user cannot read is a notification wasted.
const MAX_REPORTED: usize = 6;

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
    /// [`ipc::reload_message`]'s id, looked up once: `y-pop-config` sends it
    /// after a save.
    reload_message: u32,
    /// A reload asked for while a popup was open. The open popup is built
    /// from (and resolves its selection against) the current tree, so the
    /// tree is only swapped once it has closed.
    reload_pending: bool,
    /// Hover tooltips for the open popup — see `tooltip`'s module doc.
    tooltip: tooltip::Tooltip,
    /// The search window. Made in `on_create`, once there is an owner
    /// window for it to post to.
    search: Option<search_window::SearchWindow>,
    /// The keys a Reveal queues for the popup it is about to open; taken by
    /// `show_menu`.
    reveal_steps: Vec<menu::Step>,
}

impl YPop {
    fn new() -> Self {
        y_core::log::line("loading configuration");

        let tree = match y_pop::builtins::load() {
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
            reload_message: ipc::reload_message(),
            reload_pending: false,
            tooltip: tooltip::Tooltip::default(),
            search: None,
            reveal_steps: Vec::new(),
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
            NodeKind::Group => self.show_menu(hwnd, Some(node), position, None),
            NodeKind::App(app) => launch_and_report(hwnd, app),
            NodeKind::Quit => window::quit(0),
            NodeKind::About => show_about(),
            NodeKind::Settings => self.open_settings(),
            // Never reached: a menu never assigns a command id to one of
            // these (see `menu::append_children`), and neither does a
            // hotkey binding meaningfully target one — kept only so this
            // match stays exhaustive as `NodeKind` grows.
            NodeKind::Separator | NodeKind::Break => {}
        }
    }

    /// A right click on the tray icon of `root`: its menu — an icon that is
    /// not a group has none — then **Open Search**, **About** and **Quit**,
    /// the ones its menu does not already hold. A left click is
    /// [`YPop::run`], untouched.
    fn show_context_menu(&mut self, hwnd: HWND, root: NodeId, position: POINT) {
        let extras = self.tree.tray_extras(root);
        let group = matches!(self.tree.node(root).kind, NodeKind::Group).then_some(root);
        self.show_menu(hwnd, group, position, Some(extras));
    }

    /// What the user chose in the search window, once it has closed:
    /// `action` done to `node`. (Copying a path is done by the window.)
    fn run_search_pick(&mut self, hwnd: HWND, node: NodeId, action: ActionId) {
        match action {
            ActionId::Open => {
                let position = monitor::cursor_position().unwrap_or(POINT { x: 0, y: 0 });
                self.run(hwnd, node, position);
            }
            // The same launch with elevation asked for, this time only.
            ActionId::RunAsAdmin => {
                if let NodeKind::App(app) = &self.tree.node(node).kind {
                    let mut app = app.clone();
                    app.elevate = true;
                    launch_and_report(hwnd, &app);
                }
            }
            ActionId::ShowInFolder => {
                let entry_file = match &self.tree.node(node).kind {
                    NodeKind::App(app) => app.exe.as_deref(),
                    _ => None,
                };
                // A bare `notepad.exe` is found the way a launch would find it.
                let path = entry_file.and_then(|file| {
                    let path = std::path::Path::new(file);
                    if path.is_absolute() {
                        Some(path.to_path_buf())
                    } else {
                        y_core::shell::find_program(file)
                    }
                });
                match path {
                    Some(path) => y_core::shell::reveal(&path),
                    None => y_core::log::line("search: no file to show in Explorer"),
                }
            }
            // Until the `y-pop-config --select` of task 5.7, the settings app
            // opens without the entry selected.
            ActionId::Edit => self.open_settings(),
            ActionId::Reveal => self.reveal(hwnd, node),
            // Handled in the window, which has the clipboard.
            ActionId::CopyPath => {}
            // Not reachable yet: a Start-menu item and a history row.
            ActionId::AddToMenu | ActionId::Forget => {}
        }
    }

    /// **Reveal in menu**: opens the tray menu that holds `node` at its tray
    /// icon, and walks it down to the item with the keyboard
    /// ([`menu::reveal_steps`]). Best effort — see that function. A menu
    /// holds the item only if it is in the tray menus, not just in search.
    fn reveal(&mut self, hwnd: HWND, node: NodeId) {
        let Some((root, steps)) = menu::reveal_steps(&self.tree, node) else {
            y_core::log::line("search: nothing to reveal, the item is in no tray menu");
            return;
        };
        if !matches!(self.tree.node(root).kind, NodeKind::Group) {
            y_core::log::line("search: that tray icon has no menu to open");
            return;
        }
        // The top of the icon, centred: the menu opens upwards from there. The
        // cursor only if the shell cannot say where the icon is.
        let index = self.tree.roots.iter().position(|&id| id == root);
        let position = index
            .and_then(|index| self.tray.get(index))
            .and_then(|icon| icon.rect())
            .map(|rect| POINT {
                x: (rect.left + rect.right) / 2,
                y: rect.top,
            })
            .or_else(|| monitor::cursor_position().ok())
            .unwrap_or(POINT { x: 0, y: 0 });
        self.reveal_steps = steps;
        self.show_menu(hwnd, Some(root), position, None);
    }

    /// Does what an entry of the right-click menu says.
    fn run_extra(&mut self, extra: menu::Extra) {
        match extra {
            menu::Extra::Search => {
                if let Some(search) = self.search.as_mut()
                    && let Err(error) = search.open(&self.tree)
                {
                    y_core::log::line(format_args!("search could not open: {error}"));
                }
            }
            menu::Extra::About => show_about(),
            menu::Extra::Quit => window::quit(0),
        }
    }

    fn show_menu(
        &mut self,
        hwnd: HWND,
        group: Option<NodeId>,
        position: POINT,
        extras: Option<TrayExtras>,
    ) {
        // Keys for this popup only, whether or not it ends up opening.
        let steps = std::mem::take(&mut self.reveal_steps);
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

        let (popup, session) = menu::build_with(&self.tree, group, extras);
        // Reachable from `on_message`'s WM_INITMENUPOPUP arm while `track`
        // below is still blocked — see `YPop::session`'s doc comment.
        self.session = Some(session);

        // Queued before the menu starts looping; it takes them as the user's
        // own key presses. Posted to this thread's queue, not injected: they
        // can only ever reach this popup.
        for step in steps {
            let key = match step {
                menu::Step::Down => VK_DOWN,
                menu::Step::Right => VK_RIGHT,
            };
            for message in [WM_KEYDOWN, WM_KEYUP] {
                let _ =
                    unsafe { PostMessageW(Some(hwnd), message, WPARAM(key.0 as usize), LPARAM(0)) };
            }
        }
        let selected = popup.track(hwnd, position);
        self.tooltip.hide(hwnd);
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
                menu::Selection::Extra(extra) => self.run_extra(extra),
            }
        }

        if std::mem::take(&mut self.reload_pending) {
            self.reload(hwnd);
        }
    }

    /// Registers every hotkey and creates one tray icon per root, for the
    /// current tree. The counterpart of dropping `hotkeys` and clearing
    /// `tray`, which a reload does first.
    fn install(&mut self, hwnd: HWND) -> Result<()> {
        let mut hotkeys = HotkeyManager::new(hwnd);
        let search = self.tree.search_hotkey();
        let mut refused = Vec::new();
        let mut hint = false;
        for (id, binding) in self.tree.hotkey_bindings().chain(search) {
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
                refused.push(binding.hotkey.to_string());
                hint |= !binding.can_override;
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
        self.report(&refused, hint);
        Ok(())
    }

    /// Tells the user, once and in one balloon, what the configuration got
    /// wrong: hotkeys another application already owns, then the file's own
    /// warnings. Needs a tray icon to speak from, so it runs at the end of
    /// [`YPop::install`]; a configuration with no tray menu has none, and
    /// those problems stay in the log.
    fn report(&mut self, refused: &[String], hint: bool) {
        let mut lines = Vec::new();
        if !refused.is_empty() {
            lines.push(format!(
                "Already taken by another application: {}",
                refused.join(", ")
            ));
            if hint {
                lines.push("Add override=\"true\" to claim one anyway.".to_owned());
            }
        }
        lines.extend(self.tree.warnings.iter().take(MAX_REPORTED).cloned());
        if self.tree.warnings.len() > MAX_REPORTED {
            lines.push(format!(
                "…and {} more, see {}",
                self.tree.warnings.len() - MAX_REPORTED,
                config::path().display()
            ));
        }

        if !lines.is_empty()
            && let Some(tray) = self.tray.first_mut()
        {
            let _ = tray.notify(
                &Notification::new("Configuration", &lines.join("\n")).level(Notify::Warning),
            );
        }
    }

    /// Re-reads `y-pop.xml` and swaps it in — `y-pop-config` asks for this
    /// after a save. A file that doesn't load leaves the running
    /// configuration alone (the error goes to the log, as at start-up), so a
    /// half-typed edit never takes the hotkeys away. Only a tree that did load
    /// unregisters the old hotkeys and tray icons.
    ///
    /// `start`/`exit` events are not fired again: they belong to the
    /// process, not to the file.
    fn reload(&mut self, hwnd: HWND) {
        if self.session.is_some() {
            self.reload_pending = true;
            return;
        }

        y_core::log::line("reloading configuration");
        let tree = match y_pop::builtins::load() {
            Ok(tree) => tree,
            Err(error) => {
                y_core::log::line(format_args!(
                    "reload failed, keeping the old configuration: {error}"
                ));
                return;
            }
        };
        for warning in &tree.warnings {
            y_core::log::line(format_args!("{}: {warning}", config::FILE_NAME));
        }

        // Like the native popup, the search window is closed before the tree
        // it was opened on is swapped out.
        if let Some(search) = self.search.as_mut() {
            search.close();
        }
        self.hotkeys = None;
        self.tray.clear();
        self.tree = tree;
        if let Err(error) = self.install(hwnd) {
            y_core::log::line(format_args!(
                "reload: tray icons could not be created: {error}"
            ));
        }
    }

    /// Opens the settings app (`y-pop-config.exe`, expected next to this
    /// executable — the two ship together). Without it, opens the file
    /// itself, so `<settings/>` never silently does nothing. The settings app
    /// is single-instance: a second click brings the open window forward.
    fn open_settings(&self) {
        let app = std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name(ipc::CONFIG_EXE));
        match app.filter(|app| app.is_file()) {
            Some(app) => y_core::shell::open(&app),
            None => {
                y_core::log::line(format_args!(
                    "{} isn't installed next to y-pop — opening the file instead",
                    ipc::CONFIG_EXE
                ));
                y_core::shell::open(&config::path());
            }
        }
    }
}

fn launch_and_report(hwnd: HWND, app: &y_pop::model::App) {
    if let Err(error) = y_pop::launch::launch(hwnd, app) {
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
        self.search = Some(search_window::SearchWindow::new(hwnd));
        self.install(hwnd)?;
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
                if event == TrayEvent::ContextMenu {
                    self.show_context_menu(hwnd, root, position);
                } else {
                    self.run(hwnd, root, position);
                }
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

        // A themed popup's labels are owner-drawn, so Windows cannot match
        // a typed key to an `&` mnemonic itself — see `handle_menu_char`.
        if msg == WM_MENUCHAR {
            return y_core::menu::handle_menu_char(wparam, lparam);
        }

        // Tooltips: the highlight moving in an open popup, and the hover
        // delay running out. Both arrive re-entrantly from `track`'s own
        // message loop, like WM_INITMENUPOPUP below.
        if msg == WM_MENUSELECT {
            if let Some(session) = self.session.as_ref() {
                let select = y_core::menu::decode_menu_select(wparam, lparam);
                let tip = menu::tooltip_for(session, select);
                self.tooltip.highlight(hwnd, tip, session.design());
            }
            // Not handled: let DefWindowProc see it too (status-bar text).
            return None;
        }
        if msg == WM_TIMER && self.tooltip.on_timer(hwnd, wparam.0) {
            return Some(LRESULT(0));
        }

        // The search window's own requests (typing, clicks, close…). What it
        // was closed to run is run here, once it has gone.
        let handled_by_search = self
            .search
            .as_mut()
            .map(|search| (search.on_message(msg, wparam, lparam, &self.tree), search));
        if let Some((true, search)) = handled_by_search {
            if let Some((node, action)) = search.take_activation() {
                self.run_search_pick(hwnd, node, action);
            }
            return Some(LRESULT(0));
        }

        // Registered at run time, so a guard rather than a constant pattern;
        // 0 means registration failed, and 0 is also `WM_NULL`.
        if msg != 0 && msg == self.reload_message {
            self.reload(hwnd);
            return Some(LRESULT(0));
        }

        if msg == WM_HOTKEY {
            if self.tree.search_hotkey().map(|(id, _)| id) == Some(wparam.0 as u32) {
                // Opens the search window, or closes it if it is open.
                if let Some(search) = self.search.as_mut()
                    && let Err(error) = search.toggle(&self.tree)
                {
                    y_core::log::line(format_args!("search could not open: {error}"));
                }
            } else if let Some(node) = self.tree.hotkey_owner(wparam.0 as u32) {
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
        self.search = None; // destroys its window, if open
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
