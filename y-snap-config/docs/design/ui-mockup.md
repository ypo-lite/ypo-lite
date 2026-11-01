# UI mockup

`mockup.html` is a static, non-functional layout mock of the settings app —
open it directly in a browser (double-click, or `start mockup.html`). It has
no dependency on `windows-reactor` or this crate; it's HTML/CSS/JS built to
scope the screens and their layout before writing the real WinUI code, not a
preview of the real thing (WinUI's actual Fluent controls look different).

Also published as a Claude Artifact for easier iteration:
<https://claude.ai/code/artifact/5f33aa86-6db8-4d3c-ade2-a969a7ce639c>

Five screens, behind a left nav rail — the same shape `main.rs` now uses for
real:

- **Hotkeys** — table of key combo / action / target / desktop / steal-from-shell.
- **Mouse gestures** — one card per gesture (drag/resize): button + modifier chips.
- **Mouse shortcuts** — table of trigger / action / target / desktop; the trigger opens an editor with modifier chips and Button / Wheel tilt / Side button groups. Not in the HTML mock (it postdates it).
- **Action overlay** — the one screen worth mocking for real: layout fields,
  palette swatches and the tile list on the left, a live preview on the
  right that mirrors the actual popup's tile grid (including the per-desktop
  group).
- **Minimize button** — hover-delay slider, tiles per row, right-click action, popup tiles.

Data shown is the real shipped `y-snap/rsc/y-snap.xml`, not placeholder text.

## From mock to WinUI

The mock gets two things for free from CSS that WinUI doesn't give you, and
losing them on the way into Rust caused most of the app's layout bugs:

1. **Sibling fields share a width that comes from their grid, not their
   label.** CSS grid columns (`.field-grid`, `.palette-grid`,
   `.gesture-grid`) do this. In the app, `ui::layout::AutoGrid` does.
2. **Every screen degrades on a narrow window.** The mock uses
   `@media (max-width: 860px)`. The app has no CSS. It passes a
   `ui::viewport::Viewport` (the width each piece really gets) down the tree,
   and `AutoGrid`, `Split` and `Table` reflow from it.

[`ui-toolkit.md`](../../ui-kit/docs/ui-toolkit.md) is the guide to both;
[`ui-checklist.md`](ui-checklist.md) tracks each screen against them.

## Deliberate departures from this mock

Each is also noted in the code where it happens.

- **Combos are picked, never typed or captured.** The mock's "click to set…"
  implies live key capture, which `windows-reactor` can't do (no key events),
  and a raw Win32 hook was turned down for a settings field. Instead, the Key
  cell opens `key_picker` under its row: keycaps readout, modifier chips, a
  search box, and one category of keys at a time.
- **Overlay tabs instead of one stacked column.** Stacked, Layout + Palette +
  Groups + Tiles ran far taller than the window. The preview stays beside (or
  below) whichever tab is open.
- **Tiles are a list with a per-tile editor**, closer to the mock's
  `.tile-list` than the earlier seven-column grid of inputs.
- **The preview scales to fit** instead of drawing tiles at their configured
  pixel size, and shows no hover/active states (there's no live pointer to
  drive them).
- **Hotkey and group tables turn into cards** below their minimum width,
  rather than the mock's tables, which simply shrink.
- **The nav pane collapses to an icon rail** below a 1000px window. The mock's
  rail is always labelled.
- **Not editable yet**: `label-on`, the icon font and its size, the label font
  size and `theme`. These round-trip through `Settings::save()` untouched. New
  groups get a `group-N` id that can't be renamed in the UI.
- **Icons have a picker** (emoji only: suggestions per action, search, categories) beside the free-text field. Fluent icon glyphs aren't offered, because nothing here can render them (`TextBlock` has no font choice in this crate).
- **Minimize-button preview** is a stub title bar with the popup under its minimize button, scaled to fit. The mock's slider gained a number box beside it for exact values.
