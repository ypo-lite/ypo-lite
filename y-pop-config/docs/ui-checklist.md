# UI acceptance checklist — y-pop-config

Per screen, checkable against the code. The rules are in
[`ui-toolkit.md`](../../ui-kit/docs/ui-toolkit.md). Update this file in the same change that changes
the layout it describes. One table per screen, added with the screen.

Legend: ✅ done (by reading the code) · 👁 done, **verify live** (depends on WinUI
rendering the code can't prove) · ⚠️ partial / open call · ❌ missing.

Every 👁 row is checked by resizing a real window across the thresholds:
`shell::EXPANDED_PANE_MIN_WINDOW` (1000px), each table's `min_width()`, and each
`Split`, down to `shell::MIN_WINDOW` (560×440).

## Shell (`main.rs`, `ui-kit` `shell`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | Screens get the width that's really left for them | ✅ | `shell::layout` subtracts pane, chrome allowance and page padding; screens only see `layout.content`. (Phase 3's stub screens don't take a `Viewport` yet; the real ones must.) |
| 2 | Nav pane gives room back on narrow windows | 👁 | Labelled 220px pane ≥ 1000px, 48px rail below. On the rail the group parents ("Settings", "Tray menus") open as flyouts: check they are usable. |
| 3 | Page scrolls vertically only | ✅ | `page_frame` → `scroll_y`. |
| 4 | Status bar never pushes its buttons off-window | ✅ | `Grid [Star, Auto, Auto, Auto]`; the status trims, its tooltip has the full text and the file path. |
| 5 | Window can't be resized below what the layout supports | 👁 | `WindowConstraints` min 560×440. |
| 6 | Save is the visually primary action | ✅ | Accent, disabled when clean. Tooltip says what is and isn't kept. |
| 7 | Load warnings are shown before the first save | 👁 | `InfoBar` above every screen, closable. Unlike `y-snap-config`'s, the text says a save loses nothing. Check how a long list wraps. |
| 8 | A save applies to the running y-pop | 👁 | `y_pop::ipc::request_reload` after save; the status says whether y-pop was running. |
| 9 | Reload never silently discards edits | 👁 | With unsaved changes the bar asks "Discard unsaved changes?" inline (no `ContentDialog`). Check it fits next to the buttons at 560px. |
| 10 | One level of Undo for a destructive edit | 👁 | "Deleted X — Undo" in the bar after deleting a tray menu. Check it and the Reload question don't both show. |
| 11 | Nav groups are not selectable | 👁 | "Settings" and "Tray menus" are expanded parents with `selects_on_invoked(false)`: `NavigationView` has no header item in `windows-reactor` 0.100. Check clicking them does nothing and they stay open. |
| 12 | The screen and "show comments" are remembered | ✅ | `prefs.rs`, `y-pop-config.ini` next to `y-pop.xml`; unit-tested. |

## Placeholder screens (phase 3)

General, Skin, Variables and the tray menu list are stubs: a title, a line of
what's coming, and counts. They have no layout to check yet.
