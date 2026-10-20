//! The windows y-snap has hidden: `iconize` and `traynize`.
//!
//! Both take a window off the screen (see [`window::hide_away`]) and keep it
//! here until it is brought back. They differ in where the user finds it:
//!
//! * **Iconized** — the window gets a notification-area icon of its own: its
//!   own icon, its title as the tooltip. A click brings it back; a right click
//!   offers *Open* and *Close*. (HulK's `ICONIZE_OPT_SYSTRAY`.)
//! * **Traynized** — no icon of its own. It is listed in y-snap's tray menu,
//!   which is where [`Hidden::traynized`] is read. (HulK's
//!   `ICONIZE_OPT_INTERNAL`.)
//!
//! # What keeps this honest
//!
//! A hidden window has no UI left, so y-snap is the only thing that can bring
//! it back — which makes every way an entry can go stale a way to lose a
//! window or to leave a dead icon behind:
//!
//! * the application **closes the window** while hidden → [`Hidden::purge`]
//!   notices on its timer;
//! * the application **shows it again itself** (a second launch of a
//!   single-instance program does this) → the same purge drops the entry
//!   rather than keeping an icon for a window already on screen;
//! * the **handle is reused** by a new window → every entry also remembers the
//!   process id, and an entry whose handle now belongs to another process is
//!   dead;
//! * **y-snap quits** → [`Hidden::restore_all`], from `on_destroy`;
//! * **y-snap is killed** → nothing can be done from here. A cache file that
//!   survives the crash is a planned, opt-in answer: see
//!   `docs/iconize-traynize-plan.md`, "Crash recovery". The registry is built
//!   so it has exactly one place to hook into (`changed`).
//!
//! The timer runs only while something is hidden, so an idle y-snap does not
//! wake up for this.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use y_core::{
    icon::Icon,
    ids::{IDM_FIRST, TRAY_ICON_ID, WM_TRAY_CALLBACK},
    process,
    resources::{IconSize, app_icon_or_stock},
    tray::TrayIcon,
};

use crate::window;

/// How many windows can be hidden at once, Iconized and Traynized together.
/// An Iconized window costs a notification-area slot, and a tray that fills up
/// is worse than a refusal. Must stay within a `u64`'s bits (see [`Slots`]).
pub const MAX_HIDDEN: usize = 48;
const _: () = assert!(MAX_HIDDEN <= 64);

/// How often a hidden window is checked for having died or reappeared.
const PURGE_INTERVAL_MS: u32 = 5_000;

/// `WM_TIMER` id of the check. `1` and `2` belong to the mouse hook.
pub const PURGE_TIMER_ID: usize = 3;

/// First menu command of the traynized entries; one per slot from here.
pub const IDM_ENTRY_FIRST: u32 = IDM_FIRST + 100;

/// Longest title shown in a menu item, in characters.
pub const MENU_TITLE_MAX: usize = 50;

/// Where a hidden window is found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Behind an icon of its own in the notification area.
    Iconized,
    /// In y-snap's tray menu.
    Traynized,
}

/// Why a window was not hidden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// [`MAX_HIDDEN`] windows are hidden already.
    Full,
    /// The window belongs to an elevated process and y-snap is not elevated,
    /// so Windows would silently ignore the request.
    Elevated,
    /// The window is gone, or was not on screen to begin with.
    Gone,
    /// The notification icon could not be created. The window was left alone.
    NoIcon,
}

impl Refusal {
    /// What to tell the user, or `None` when there is nothing worth saying.
    pub fn message(self) -> Option<String> {
        match self {
            Self::Full => Some(format!("Too many hidden windows ({MAX_HIDDEN}).")),
            Self::Elevated => Some(
                "That window runs as administrator; y-snap would have to as well.".to_owned(),
            ),
            Self::Gone => None,
            Self::NoIcon => Some("Could not create the notification icon.".to_owned()),
        }
    }
}

/// One hidden window.
pub struct Entry {
    pub window: HWND,
    pub mode: Mode,
    /// Index in the registry's slot table; the key of the tray icon id and
    /// the menu command.
    pub slot: usize,
    /// The title when it was hidden — what the tooltip and the menu show.
    pub title: String,
    /// Executable name, `"notepad"`: what a right-click menu names it by.
    pub app: String,
    pid: u32,
    /// A private copy of the window's icon. `None` falls back to y-snap's own.
    pub icon: Option<Icon>,
    /// Present for [`Mode::Iconized`] only. Dropping it removes the icon.
    tray: Option<TrayIcon>,
}

impl Entry {
    /// Whether the window is still the one that was hidden.
    fn is_alive(&self) -> bool {
        window::exists(self.window) && process::window_process_id(self.window) == self.pid
    }
}

/// Every window y-snap has hidden, in the order it hid them.
#[derive(Default)]
pub struct Hidden {
    entries: Vec<Entry>,
    slots: Slots,
    /// The window the purge timer was set on; `Some` while the timer runs.
    timer: Option<HWND>,
}

impl Hidden {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Hides `target`, behind an icon or in the menu according to `mode`.
    /// `owner` is y-snap's own window: it receives the tray callbacks of an
    /// Iconized window, and the purge timer.
    ///
    /// Hiding a window that is already hidden here succeeds and does nothing.
    pub fn hide(&mut self, owner: HWND, target: HWND, mode: Mode) -> Result<(), Refusal> {
        if self.entries.iter().any(|entry| entry.window == target) {
            return Ok(());
        }
        if !window::exists(target) || !window::is_visible(target) {
            return Err(Refusal::Gone);
        }

        let pid = process::window_process_id(target);
        if !process::can_drive(pid) {
            return Err(Refusal::Elevated);
        }

        let slot = self.slots.take().ok_or(Refusal::Full)?;
        let app = process::executable_stem(pid).unwrap_or_default();
        let title = window::title(target);
        let icon = Icon::of_window(target);

        // The icon goes up *before* the window goes away: if it cannot be
        // made, the window is still where the user left it.
        let tray = match mode {
            Mode::Traynized => None,
            Mode::Iconized => {
                let handle = icon
                    .as_ref()
                    .map_or_else(|| app_icon_or_stock(IconSize::Small), Icon::handle);
                let tooltip = if title.is_empty() { &app } else { &title };
                match TrayIcon::new(owner, tray_id(slot), WM_TRAY_CALLBACK, handle, tooltip) {
                    Ok(tray) => Some(tray),
                    Err(_) => {
                        self.slots.release(slot);
                        return Err(Refusal::NoIcon);
                    }
                }
            }
        };

        window::hide_away(target);

        self.entries.push(Entry {
            window: target,
            mode,
            slot,
            title,
            app,
            pid,
            icon,
            tray,
        });
        self.timer = self.timer.or_else(|| start_timer(owner));
        self.changed();
        Ok(())
    }

    /// The entry behind `slot`, if any.
    pub fn get(&self, slot: usize) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.slot == slot)
    }

    /// The Traynized entries, in the order they were hidden — the "Hidden
    /// windows" section of the tray menu.
    pub fn traynized(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.mode == Mode::Traynized)
    }

    /// Brings back the window in `slot` and forgets it. `false` if there is
    /// no such entry.
    pub fn restore(&mut self, slot: usize) -> bool {
        match self.remove(slot) {
            Some(entry) => {
                if entry.is_alive() {
                    window::bring_back(entry.window);
                }
                true
            }
            None => false,
        }
    }

    /// Brings back the window hidden most recently (`uniconize`).
    pub fn restore_last(&mut self) {
        if let Some(slot) = self.entries.last().map(|entry| entry.slot) {
            self.restore(slot);
        }
    }

    /// Brings every hidden window back, the most recent first
    /// (`uniconize-all`, and quitting).
    pub fn restore_all(&mut self) {
        while let Some(slot) = self.entries.last().map(|entry| entry.slot) {
            self.restore(slot);
        }
    }

    /// Asks the window in `slot` to close, and forgets it. The window is shown
    /// first: a hidden window may not act on `WM_CLOSE`, and one that asks
    /// "save changes?" needs somewhere to ask.
    pub fn close(&mut self, slot: usize) {
        if let Some(entry) = self.remove(slot)
            && entry.is_alive()
        {
            window::bring_back(entry.window);
            window::close(entry.window);
        }
    }

    /// Drops every entry whose window died, was reused by another process, or
    /// came back on its own. Called from the purge timer.
    pub fn purge(&mut self) {
        let stale: Vec<usize> = self
            .entries
            .iter()
            .filter(|entry| !entry.is_alive() || window::is_visible(entry.window))
            .map(|entry| entry.slot)
            .collect();

        for slot in stale {
            // Not `restore`: a window that is already on screen needs no
            // help, and a dead one cannot have any.
            self.remove(slot);
        }
    }

    /// Puts the notification icons back after Explorer restarted.
    pub fn restore_tray_icons(&mut self) {
        for tray in self.entries.iter_mut().filter_map(|entry| entry.tray.as_mut()) {
            let _ = tray.restore();
        }
    }

    /// Takes the entry out and frees what it held: its slot, its tray icon
    /// (dropped with the entry), and the purge timer if nothing is left.
    fn remove(&mut self, slot: usize) -> Option<Entry> {
        let index = self.entries.iter().position(|entry| entry.slot == slot)?;
        let entry = self.entries.remove(index);
        self.slots.release(slot);

        if self.entries.is_empty()
            && let Some(owner) = self.timer.take()
        {
            let _ = unsafe { KillTimer(Some(owner), PURGE_TIMER_ID) };
        }
        self.changed();
        Some(entry)
    }

    /// The set of hidden windows just changed. The single place a crash
    /// recovery cache would be written from (see the module doc).
    fn changed(&self) {}
}

fn start_timer(owner: HWND) -> Option<HWND> {
    let id = unsafe { SetTimer(Some(owner), PURGE_TIMER_ID, PURGE_INTERVAL_MS, None) };
    (id != 0).then_some(owner)
}

// ---------------------------------------------------------------------------
// Ids. The notification icon and the menu command of an entry both derive from
// its slot, so a callback finds its entry without a lookup table.
// ---------------------------------------------------------------------------

/// The notification icon id of an Iconized window in `slot`. y-snap's own icon
/// keeps [`TRAY_ICON_ID`].
pub fn tray_id(slot: usize) -> u32 {
    TRAY_ICON_ID + 1 + slot as u32
}

/// The slot behind a notification icon id, if it is one of ours.
pub fn slot_of_tray_id(id: u32) -> Option<usize> {
    let slot = id.checked_sub(TRAY_ICON_ID + 1)? as usize;
    (slot < MAX_HIDDEN).then_some(slot)
}

/// The menu command of the Traynized window in `slot`.
pub fn menu_id(slot: usize) -> u32 {
    IDM_ENTRY_FIRST + slot as u32
}

/// The slot behind a menu command, if it is one of ours.
pub fn slot_of_menu_id(id: u32) -> Option<usize> {
    let slot = id.checked_sub(IDM_ENTRY_FIRST)? as usize;
    (slot < MAX_HIDDEN).then_some(slot)
}

/// `text` cut to `max` characters, with a trailing `…` when it was.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Free slots, as a bitmap: bit `n` set means slot `n` is in use.
#[derive(Default)]
struct Slots(u64);

impl Slots {
    /// The lowest free slot, now marked used. `None` when all
    /// [`MAX_HIDDEN`] are taken.
    fn take(&mut self) -> Option<usize> {
        let slot = (!self.0).trailing_zeros() as usize;
        if slot >= MAX_HIDDEN {
            return None;
        }
        self.0 |= 1 << slot;
        Some(slot)
    }

    fn release(&mut self, slot: usize) {
        self.0 &= !(1 << slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_handed_out_lowest_first_and_reused() {
        let mut slots = Slots::default();
        assert_eq!(slots.take(), Some(0));
        assert_eq!(slots.take(), Some(1));
        assert_eq!(slots.take(), Some(2));

        slots.release(1);
        assert_eq!(slots.take(), Some(1));
        assert_eq!(slots.take(), Some(3));
    }

    #[test]
    fn slots_run_out_at_the_cap() {
        let mut slots = Slots::default();
        for expected in 0..MAX_HIDDEN {
            assert_eq!(slots.take(), Some(expected));
        }
        assert_eq!(slots.take(), None);

        slots.release(7);
        assert_eq!(slots.take(), Some(7));
        assert_eq!(slots.take(), None);
    }

    #[test]
    fn tray_ids_round_trip_and_never_collide_with_y_snaps_own_icon() {
        for slot in 0..MAX_HIDDEN {
            let id = tray_id(slot);
            assert_ne!(id, TRAY_ICON_ID);
            assert_eq!(slot_of_tray_id(id), Some(slot));
        }
        assert_eq!(slot_of_tray_id(TRAY_ICON_ID), None);
        assert_eq!(slot_of_tray_id(0), None);
        assert_eq!(slot_of_tray_id(tray_id(MAX_HIDDEN)), None);
    }

    #[test]
    fn menu_ids_round_trip_and_stay_clear_of_other_commands() {
        for slot in 0..MAX_HIDDEN {
            assert_eq!(slot_of_menu_id(menu_id(slot)), Some(slot));
        }
        assert_eq!(slot_of_menu_id(IDM_ENTRY_FIRST - 1), None);
        assert_eq!(slot_of_menu_id(menu_id(MAX_HIDDEN)), None);
    }

    #[test]
    fn shorten_cuts_with_an_ellipsis_and_leaves_short_text_alone() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("exactly10!", 10), "exactly10!");
        assert_eq!(shorten("a longer title", 8), "a longe…");
        // Characters, not bytes.
        assert_eq!(shorten("éééééé", 4), "ééé…");
    }
}
