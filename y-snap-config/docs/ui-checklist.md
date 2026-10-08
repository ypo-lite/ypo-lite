# UI acceptance checklist

Per screen, checkable against the code. The rules are in
[`ui-toolkit.md`](ui-toolkit.md). Update this file in the same change that changes
the layout it describes.

Legend: ✅ done (by reading the code) · 👁 done, **verify live** (depends on WinUI
rendering the code can't prove) · ⚠️ partial / open call · ❌ missing.

Every 👁 row is checked by resizing a real window across the thresholds:
`shell::EXPANDED_PANE_MIN_WINDOW` (1000px), each table's `min_width()`, and the
Overlay `Split`, down to `shell::MIN_WINDOW` (560×440).

## Shell (`main.rs`, `shell.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | Screens get the width that's really left for them | ✅ | `shell::layout` subtracts pane, chrome allowance and page padding; screens only see `layout.content`. |
| 2 | Nav pane gives room back on narrow windows | 👁 | Labelled 220px pane ≥ 1000px, 48px icon rail (`LeftCompact`) below. Check the rail shows tooltips and the switch doesn't flicker at the threshold. |
| 3 | Page scrolls vertically only; nothing scrolls sideways | ✅ | `page_frame` → `scroll_y`. No other `ScrollViewer` in the app. |
| 4 | Status bar never pushes its buttons off-window | ✅ | `Grid [Star, Auto, Auto]`; status trims with an ellipsis, full text and file path in the tooltip. |
| 5 | Window can't be resized below what the layout supports | 👁 | `WindowConstraints` min 560×440. |
| 6 | Save is the visually primary action | ✅ | Accent style, disabled when nothing changed. Tooltip says comments and formatting aren't kept. |
| 7 | Parts of the file that failed to load are shown before a save can drop them | 👁 | `shell::warnings_banner` (an `InfoBar`) above every screen, closable; status turns red ("saving drops what couldn't be read") while there are unsaved changes. Check how a long list wraps. |
| 8 | A save applies to the running y-snap | 👁 | `y_snap::ipc::request_reload` after save; status says whether y-snap was running. Check: edit a hotkey, Save, use it straight away without touching the tray. |

## Hotkeys (`hotkeys.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | Cells align under their header and fill their column | 👁 | One `Table` grid; every cell centred on its row and `fill()`ed. |
| 2 | The table fits any width without scrolling sideways | ✅ | Flex columns fill the page; below `min_width()` (~670) rows become label/value cards. |
| 3 | The Steal switch stays inside its column | 👁 | `switch()` drops the 154px native minimum, and fills the on/off slots with a zero-size element. An empty slot made WinUI show its localized "Activé"/"Désactivé", cropped. |
| 4 | The key combo is readable in the row | ✅ | Keycaps with printed labels (`'`, not `OEM7`). |
| 5 | The combo editor opens next to the row it edits | ✅ | `Row::detail`, directly under the row. |
| 6 | The key picker never runs off-window | ✅ | One category at a time, keys in an `AutoGrid`; tabs turn into a dropdown under 720px. |
| 7 | Any key is reachable, including F13–F24 and media keys, and findable by name | ✅ | `keys.rs` catalog plus search box; test asserts every entry resolves. |
| 8 | A new hotkey opens straight into its picker | ✅ | `Msg::HotkeyAdded` → `EditHotkey(last)`. |
| 9 | "Steal from shell" is explained | ✅ | `note` under the table, as in the mock. |

## Mouse gestures (`mouse.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | Cards reflow 3 → 2 → 1 per row | ✅ | `AutoGrid::new(250).max_columns(3)`. |
| 2 | A card never clips its own content | ✅ | `Auto` rows, unlike the old `VariableSizedWrapGrid`'s fixed 210px height. |
| 3 | Modifier chips are equal width and fit the card | ✅ | Chip `AutoGrid` built from the card's own `Viewport`. |
| 4 | An off gesture reads as off | ✅ | `emphasis::OFF` opacity; controls disabled. |

## Action overlay (`overlay/`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | Editor and preview sit side by side only when both fit | ✅ | `Split` with the active tab's own minimum and a 300px preview minimum. |
| 2 | Layout fields are equal width whatever their labels | ✅ | `AutoGrid::new(110).max_columns(3)`. |
| 3a | Theme is kept across saves | ✅ | Theme dropdown (switching resets the colours), "N of 10 colours differ" + Reset. `y-snap` saves `theme` plus only the differing colours (tested there). |
| 3 | Swatches are equal width; the open one is marked | ✅ | `AutoGrid::new(96).max_columns(5)`; Accent style on the open swatch. |
| 4 | Groups fit the pane | ✅ | `Table` → cards below its minimum. |
| 5 | Tiles are scannable and fit the pane | ✅ | Compact list (key, icon, label + action, group) with the full editor per row. |
| 5b | Tile icons are choosable without knowing Win + . | ✅ | Same shared `icon_picker` as the Minimize-button tiles (see there). |
| 6 | The preview never overflows its pane | ✅ | Scales tiles, gaps and fonts by one factor, shown as "Live · 72%" when scaled. |
| 7 | The preview matches the real popup | 👁 | Panel = `back` at `alpha` + `panel_border`; tiles on `back`, no stroke, radius `min(corner_radius, 6)` (as `ActionOverlay::paint`). No hover/active states. |
| 8 | The ColorPicker fits the editor pane | 👁 | Palette tab minimum is 360px, assumed ≥ the picker's natural width. Confirm. |

## Minimize button (`min_button.rs`)

| # | Requirement | Status | Notes |
|---|---|---|---|
| 1 | Settings and preview side by side only when both fit | ✅ | `Split`, 360 / 300 minimums. |
| 2 | Hover delay settable by feel and exactly | ✅ | `range_input`: slider (0–2000 ms, 50 ms steps, stretched to fit a larger hand-edited value) + number box. |
| 3 | Opening the screen doesn't mark it dirty | 👁 | Delay and right-click handlers ignore a value equal to the current one, since WinUI reports a value on first render. Check "Unsaved changes" doesn't appear just from opening the screen. |
| 4 | Only valid actions offered | ✅ | `y_snap::config::is_minbutton_action`, now public, the same rule the loader uses; test-pinned. |
| 5 | Tiles listed and edited in place | ✅ | `Table` + per-row `editor`, as on the Overlay Tiles tab. |
| 7 | Icons are choosable without knowing Win + . | 👁 | Shared `icon_picker`: suggestions for the action, keyword search, categories, Win + . hint. New tiles and action changes fill in a suggested icon unless one was picked by hand. Check that the emoji render in colour in the picker buttons. |
| 6 | Preview matches the real popup | 👁 | One row of 72×40 tiles, 18px glyph (icon, else label, else "?"), overlay `back`+`alpha`, corner radius; centred under the minimize button (as `MinOverlay::show`), clamped to the stage. |

## Cross-cutting

| # | Requirement | Status | Notes |
|---|---|---|---|
| 0 | Every ordered list can be reordered without the table reflowing | 👁 | Hotkeys, overlay groups, overlay tiles, popup tiles: Reorder toggle → ↑/↓ in the actions column (ends disabled). Check the order shows up in both popups after Save. |
| 1 | No literal sizes in screen modules | ✅ | Only domain values: field ranges, column/grid minimums, and the popup's own pixel geometry in `preview.rs` (named constants). |
| 2 | No screen reads the window size | ✅ | Only `main.rs` → `shell::layout`. |
| 3 | `ui/` is app-agnostic | ✅ | No `crate::Msg`/`ConfigApp` imports under `src/ui/`. |
