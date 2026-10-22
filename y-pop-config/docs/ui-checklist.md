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
| 3 | Page scrolls vertically only | ✅ | `page_frame` → `scroll_y`; a screen that scrolls its own panes uses `page_frame_pinned` (no page scroll, the screen gets the window's height; see Tray menu 15). |
| 4 | Status bar never pushes its buttons off-window | ✅ | `Grid [Star, Auto, Auto, Auto, Auto]` (status, tools, extra, Reload, Save); the status trims, its tooltip has the full text and the file path. |
| 5 | Window can't be resized below what the layout supports | 👁 | `WindowConstraints` min 560×440. |
| 6 | Save is the visually primary action | ✅ | Accent, disabled when clean. Tooltip says what is and isn't kept. |
| 7 | Load warnings are shown before the first save | 👁 | `InfoBar` above every screen, closable. Unlike `y-snap-config`'s, the text says a save loses nothing. Check how a long list wraps. |
| 8 | A save applies to the running y-pop | 👁 | `y_pop::ipc::request_reload` after save; the status says whether y-pop was running. |
| 9 | Reload never silently discards edits | 👁 | With unsaved changes the bar asks "Discard unsaved changes?" inline (no `ContentDialog`). Check it fits next to the buttons at 560px. |
| 10 | One level of Undo for a destructive edit | 👁 | "Deleted X — Undo" in the bar after deleting a tray menu. Check it and the Reload question don't both show. |
| 11 | Nav groups are not selectable | 👁 | "Settings" and "Tray menus" are expanded parents with `selects_on_invoked(false)`: `NavigationView` has no header item in `windows-reactor` 0.100. Check clicking them does nothing and they stay open. |
| 12 | The screen and "show comments" are remembered | ✅ | `prefs.rs`, `y-pop-config.ini` next to `y-pop.xml`; unit-tested. |
| 13 | "Show comments" is one toggle in the status bar | 👁 | A comment-icon `ToggleButton` (`BarStatus::tools`) after the status, only on Variables and on a tray menu; tooltip "Show/Hide comments (N)". It replaces the switches that were on those screens. Check its checked state is visible (`ToggleButton` has no Subtle style) and that it fits next to Undo / the Reload question at 560px. |

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
| 3 | Hidden comments are not forgotten | 👁 | A speech-bubble button beside the next variable's name (or a line at the end) turns them back on; so does the status-bar toggle (Shell 13). |
| 4 | Renaming rewrites the users, and a bad name says why | 👁 | Live on every keystroke; the refusal is a caption under the row. Check that a half-typed name doesn't make the box fight the typist. |
| 5 | ↑/↓/delete are always visible and never reflow the row | 👁 | `THREE_ICON_BUTTONS` reserved. |
| 6 | Deleting a used variable names where it is used | 👁 | Inline question under the row; unused ones go at once, with Undo. |
| 7 | The built-ins are listed as they resolve on this PC | 👁 | Read-only table; long values trim with a tooltip. |

## Tray menu (`menu_view.rs`, `inspector.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | The outline and the inspector share the page, or stack | 👁 | `Split` with minimums 300 / 320 (≈ 620 + gap). Stacked, the inspector is directly under the outline. Check where it switches at 560px and 1000px windows. |
| 2 | A row is selectable, focusable, and readable | 👁 | Each row is a `Button` (Accent when selected). Check indent, the ▾/▸ button, long names trimming, and that keycaps stay readable on the Accent row. The menu's own row is a heading with no expand slot, so its items start flush left; each level inside a group indents by `INDENT` (`space::M`). |
| 3 | Nothing nests a button inside a button | ✅ | The expand button and the hidden-comments button are siblings of the row button in a `Grid`. |
| 4 | The insertion marker shows where the next item goes | 👁 | An accent bar and "next item goes here" at the right depth: after the selected row, or at the end of a selected open group, or of the menu. Tested as `tree::insertion_point` / `rows`. |
| 5 | Separators and column breaks read as what they are | 👁 | A rule (with its title in front), and "— column break —". |
| 6 | Comments are visible and say they are read-only | 👁 | Muted one-liners with "N lines"; not selectable until phase 9. Hidden, they leave a speech-bubble button on the next item (or "hidden at the end"). |
| 7 | The toolbar fits any width | 👁 | `AutoGrid` of buttons (App, Group, Separator, More; and the six under More). Check the wrap at the narrowest outline, and that the toolbar stays put while the rows scroll (15). |
| 8 | The move bar says why a button is off | 👁 | ↑ ↓ ← → each enabled on its own, tooltips: "no group above to move into", "already at the top of this menu". |
| 9 | "Move to…" starts over after a move | 👁 | Keyed by the item's place; check it doesn't keep showing the group the item was just sent to, and that choosing a group in another menu opens that menu. |
| 10 | Switching the selected item starts every field fresh | 👁 | The fields are keyed by the item's id. Check the caret and a half-typed value don't leak between items. |
| 11 | Every destructive edit can be undone | 👁 | Move, move to, delete, turn into a menu: one level, in the status bar. |
| 12 | A tray menu is deleted only after a question | 👁 | Inline "Remove this tray icon and everything in it?" in the inspector. |
| 13 | A root that isn't a menu has no outline | 👁 | An app, About… as a tray icon: the inspector alone, with "Turn into a menu". |
| 14 | The XML preview is what a save writes | ✅ | `Item::to_xml`, the writer a save uses; tested. |
| 15 | Side by side, the page doesn't scroll: the panes do | 👁 | `menu_view::is_pinned` → `page_frame_pinned`; the header and the warnings banner stay on top. The outline card fills the height; its toolbar is fixed and only the rows scroll under it. The inspector scrolls on its own. Check the toolbar and the inspector stay in view after scrolling a long menu and selecting a row far down. |
| 16 | Scrollbars don't cover the cards | 👁 | Each scrolling pane keeps `SCROLLBAR` (12px) free on its right: a guess, check it on the rows and on the inspector. |
| 17 | Stacked, the whole page scrolls as before | 👁 | Below the `Split` minimums (≈ 620px of content) the outline's toolbar scrolls with it. Check the switch both ways keeps the selection and focus. |
| 18 | Hotkeys: edit in a modal, Cancel really cancels | 👁 | Edit / "Add hotkey" opens `editor::modal` (overlay over the whole window, up to 860px) holding the shared `key_picker`. OK is disabled until a key is chosen; Cancel restores the old text, or drops a hotkey just added. Edits show live behind it. Check the card is opaque and readable on Mica, that it scrolls in a short window, that category tabs/key grid reflow with the window width, and that the search box and chips update while it is open. Known: Tab can reach controls behind it and Esc does not cancel (no key events in `windows-reactor`). Unit-tested: `menu_tests` (build/override/remove/undo, cancel). |
| 19 | A combination used twice is flagged | 👁 | Under the hotkey: "Also used by X: only one of them will work." (compared case- and order-insensitively through `Hotkey::parse`). **Not done:** the marker on the outline row (11.1). |
| 20 | An app's target mode keeps what was typed | 👁 | "Program or file / Folder as submenu / Store app" (`segmented`). Switching stashes the other mode's value for the session; check going back and forth, and that Folder shows the "Folder listing" section only. |
| 21 | The target readout says what it resolves to | 👁 | "expanded path — found / not found / why not checked", under the field. Check a `%variable%` path, a bare name (`notepad.exe`), a network path and an empty field; Browse… is disabled on purpose. |
| 22 | Launch and folder fields fit and reflow | 👁 | Arguments (+ Insert variable, clipboard note), Start in, the shell choice ("Inherit (on/off)" follows the file's default), administrator / start / exit switches; Filter, Depth (+ words) and the open-folder entry in an `AutoGrid`. Check them from 560px to wide, and that Insert variable starts over after each insertion. |
| 23 | "Run now" reports in the status bar | 👁 | Starts the item as y-pop would; the bar says "Ran X" or "Couldn't run X: …" until the next edit or screen change. Check with the clipboard in the arguments. |

## Placeholder screens

Skin is still a stub from phase 3.
