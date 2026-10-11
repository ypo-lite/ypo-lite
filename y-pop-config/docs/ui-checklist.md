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

## General (`general.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | A setting's control sits beside its text, or under it when narrow | 👁 | `setting()` switches at 460px of card content. Check at 560px window and at the 1000px pane switch. |
| 2 | Menu style is one choice, the chosen one obvious | 👁 | `segmented` (Accent for the chosen one, not a checked toggle). |
| 3 | Overlay layout appears only with the overlay, and says what isn't built | 👁 | Runner and Radial are disabled with a "Coming later" tooltip; a file that already says `runner` shows it selected. |
| 4 | Switches show their real state | ✅ | Defaults when the attribute is absent: shell-execute off, show-shortcuts on. A value equal to the current one is not an edit (unit-tested). |
| 5 | The file's path is visible and its folder and XML can be opened | 👁 | Both buttons go through `y_core::shell::open`. |
| 6 | File notes can be read, edited, added, deleted | 👁 | Created empty and filled one render later (`notes_ready`), or WinUI keeps only the first line. One multi-line `TextBox` with every note joined; no "Show comments" switch here. Editing several notes merges them at the top (Undo offered); `--` is flagged. |

## Variables (`variables.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | The table fits any width | 👁 | `Table` with 5 columns, minimum ≈ 760px; below, each row is a card. Check the card layout, including the spanning comment rows (shown as plain lines). |
| 2 | Comment rows are full width | 👁 | `Row::spanning`. |
| 3 | Hidden comments are not forgotten | 👁 | A speech-bubble button beside the next variable's name (or a line at the end) turns them back on. |
| 4 | Renaming rewrites the users, and a bad name says why | 👁 | Live on every keystroke; the refusal is a caption under the row. Check that a half-typed name doesn't make the box fight the typist. |
| 5 | ↑/↓/delete are always visible and never reflow the row | 👁 | `THREE_ICON_BUTTONS` reserved. |
| 6 | Deleting a used variable names where it is used | 👁 | Inline question under the row; unused ones go at once, with Undo. |
| 7 | The built-ins are listed as they resolve on this PC | 👁 | Read-only table; long values trim with a tooltip. |

## Tray menu (`menu_view.rs`, `inspector.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | The outline and the inspector share the page, or stack | 👁 | `Split` with minimums 300 / 320 (≈ 620 + gap). Stacked, the inspector is directly under the outline. Check where it switches at 560px and 1000px windows. |
| 2 | A row is selectable, focusable, and readable | 👁 | Each row is a `Button` (Accent when selected). Check indent, the ▾/▸ button, long names trimming, and that keycaps stay readable on the Accent row. |
| 3 | Nothing nests a button inside a button | ✅ | The expand button and the hidden-comments button are siblings of the row button in a `Grid`. |
| 4 | The insertion marker shows where the next item goes | 👁 | An accent bar and "next item goes here" at the right depth: after the selected row, or at the end of a selected open group, or of the menu. Tested as `tree::insertion_point` / `rows`. |
| 5 | Separators and column breaks read as what they are | 👁 | A rule (with its title in front), and "— column break —". |
| 6 | Comments are visible and say they are read-only | 👁 | Muted one-liners with "N lines"; not selectable until phase 9. Hidden, they leave a speech-bubble button on the next item (or "hidden at the end"). |
| 7 | The toolbar fits any width | 👁 | `AutoGrid` of buttons (App, Group, Separator, More; and the six under More). Check the wrap at the narrowest outline. |
| 8 | The move bar says why a button is off | 👁 | ↑ ↓ ← → each enabled on its own, tooltips: "no group above to move into", "already at the top of this menu". |
| 9 | "Move to…" starts over after a move | 👁 | Keyed by the item's place; check it doesn't keep showing the group the item was just sent to, and that choosing a group in another menu opens that menu. |
| 10 | Switching the selected item starts every field fresh | 👁 | The fields are keyed by the item's id. Check the caret and a half-typed value don't leak between items. |
| 11 | Every destructive edit can be undone | 👁 | Move, move to, delete, turn into a menu: one level, in the status bar. |
| 12 | A tray menu is deleted only after a question | 👁 | Inline "Remove this tray icon and everything in it?" in the inspector. |
| 13 | A root that isn't a menu has no outline | 👁 | An app, About… as a tray icon: the inspector alone, with "Turn into a menu". |
| 14 | The XML preview is what a save writes | ✅ | `Item::to_xml`, the writer a save uses; tested. |

## Placeholder screens

Skin is still a stub from phase 3.
