# y-snap-config roadmap

Gaps found by auditing the app against everything `y-snap.xml` can hold
(`y-snap/src/config.rs`) and against how `y-snap` picks up a saved file. Every
section of the file has a screen; what's left is below, in the order to do it.

Status: ⬜ todo · 🔨 in progress · ✅ done (verify live where noted)

## 1. Data safety: saving must not lose anything silently

Save regenerates the whole file from `Settings` (`Settings::to_xml`). Anything
`Settings` doesn't hold is gone after the first save.

| # | Item | Status | Notes |
|---|---|---|---|
| 1.1 | **Show load warnings.** | ✅ | `Settings::load` skips broken entries (unknown action, bad combo, action not allowed in `<minbutton>`, unknown attribute) and records a warning, but the app never showed them, so the first save dropped those entries from the file without a word. Now: a closable warning banner above every screen listing them, and the status line warns that saving will drop them. After a successful save the list is cleared: the file no longer contains those entries. |
| 1.2 | **Keep `theme="auto"` across saves.** | ✅ (`y-snap` change) | `to_xml` used to write all ten colours resolved at load time instead of the theme. The shipped file is `auto`, so the first save stopped the overlay following light/dark mode. Now `Overlay` keeps its `theme` and the palette that theme produced (`theme_palette`), and `to_xml` writes `theme` plus only the colours that differ from it. Palette tab: Theme dropdown (auto/light/dark; switching resets colours to the theme) and a "N colours customised" note with a reset. |
| 1.3 | **Say that comments and formatting aren't kept.** | ✅ | By design (`to_xml` is a generator). Stated in the Save button's tooltip. |

## 2. From the app to a running y-snap

| # | Item | Status | Notes |
|---|---|---|---|
| 2.1 | **Apply on save.** | ✅ (verify live) | After a successful save the app checks whether y-snap is running (its single-instance mutex, `y_core::single_instance::is_running`) and, if so, broadcasts the registered `YpoLite.YSnap.Reload` message (`y_snap::ipc`); y-snap reloads on it. Status: "Saved and applied — y-snap is reloading it" or "Saved — y-snap isn't running; it'll use these settings when it starts". A broken file is still reported by y-snap's own notification. Not handled: a y-snap running elevated (UIPI blocks the message); it doesn't run elevated today. |
| 2.2 | **The tray opens this app.** | ✅ (verify live) | Tray menu: "Settings…" launches `y-snap-config.exe` from next to `y-snap.exe` (falling back to the XML, with a notification saying why); "Edit y-snap.xml" keeps direct file access; double-clicking the icon opens Settings. The app is now single-instance (`YpoLite.YSnapConfig` mutex): a second launch brings the open window forward (`y_core::single_instance::bring_to_front`, found by title), so two editors can't overwrite each other's saves. |

## 3. Settings with no editor yet (round-trip untouched today)

| # | Item | Status | Notes |
|---|---|---|---|
| 3.1 | Overlay text: `key-font-size`, `label-font-size`, `icon-font`, `icon-font-size` | ⬜ | A "Text" group on the Layout tab; the preview already reads them. `icon-font` as free text with the common choices (Segoe UI Emoji, Segoe Fluent Icons). |
| 3.2 | Tile `label-on` | ⬜ | In the tile editor, only for actions with a toggle state (maximize, on top, borderless, fade, roll up, hide, pin). |
| 3.3 | Rename a group | ⬜ | Rename in place, rewriting every tile's `group` reference in the same message. |

## 4. Quality of life

| # | Item | Status | Notes |
|---|---|---|---|
| 4.1 | Reorder hotkeys, overlay groups and tiles, popup tiles | ✅ (verify live) | A "Reorder" toggle beside each list's add button (shown once there are 2+ rows) swaps each row's actions for ↑/↓ (`reorder.rs`), at the same width, so the tables don't reflow when the mode switches. Reordering and editing are separate modes: entering either leaves the other, so an editor never points at a moved row. Hotkeys and Groups actions columns widened to two buttons to fit ↑/↓ (their card-layout thresholds moved up ~48px). |
| 4.2 | Conflict warnings | ⬜ | Same combo bound twice; two overlay tiles on one key. Inline on the row (a warning icon with a tooltip), not blocking. |

## Not planned

- Live key capture (decided: picker only, no keyboard hook).
- Preserving comments/formatting: would need an XML editor rather than a generator; out of proportion for a settings app.
- Fluent icon glyphs in the icon picker: nothing in `windows-reactor` can render them (no font choice on `TextBlock`).
