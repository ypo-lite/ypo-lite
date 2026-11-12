# UI toolkit guide

How to build screens in `y-snap-config` (and later any WinUI 3 app in this repo)
so they match the mock, line up, and fit any window size. The code lives in
`ui-kit/src/`; the screens built on it in `y-snap-config` (`hotkeys.rs`, `mouse.rs`, `overlay/`) are the
worked examples. For how `windows-reactor` itself works, see
[`docs/windows-reactor.md`](../../docs/windows-reactor.md); this guide is about
what we build on top of it.

## Why this exists

Before `ui/` existed, every screen called raw `windows-reactor` controls and each
bug got fixed in place, one screen at a time. Three root causes kept coming back:

1. **Layout decisions read the wrong width.** One `NARROW_WIDTH` compared against
   the *window* width, while the content really had the window minus the 200px
   nav pane, page margins and, on Overlay, a 55/45 split. The "wide" layout was
   picked for a ~250px column.
2. **Fixed-pixel things sat inside containers that can't grow.** The on-screen
   keyboard rows (~570px, no wrap) sat in a vertical-only scroller, and the preview
   tiles (config pixel sizes) sat in a `Star` column. Both were clipped off the
   window.
3. **Native WinUI defaults disagree per control.** `ComboBox`, `Button` and
   `ToggleButton` hug their content at the left, `ComboBox` sits at the top, and
   `ToggleSwitch` has a 154px minimum width. A row mixing them is ragged, and the
   old "Steal" switch spilled into its neighbouring column.

The toolkit turns each fix into the default, so it can't be forgotten on the next
screen.

## The rules

1. **Never write a literal size, font size, opacity or radius in a screen.** Use
   `ui::tokens` (`space::M`, `font::CAPTION`, `emphasis::MUTED`, …). If no token
   fits, add one to `tokens.rs`. Don't inline the number.
2. **Every screen function takes a `Viewport` and decides from it.** Never from
   the window size, and never from a constant that assumes the window size.
3. **Pass the right `Viewport` down.** Any container that splits width gives
   each child its share: `inset` for padding, `AutoGrid::item_viewport` for a grid
   cell, `Split::primary`/`secondary` for panes, `Table::detail_viewport` for a
   row's editor.
4. **Nothing scrolls sideways; things reflow.** The page scrolls vertically and
   that's the only scroller. Anything that might not fit uses `AutoGrid`, `Split`
   or `Table`, which reflow, or scales itself to its `Viewport` (the overlay
   preview does this).
5. **Use toolkit controls, not raw ones.** `combo`, `number_box`, `switch`,
   `chip`… come pre-aligned with `Fill`. If you need a raw control, call `.fill()`
   on it.
6. **`ui/` never imports `crate::Msg`.** Toolkit functions take `View`s and
   `Callback`s (`context.callback(...)`), so they stay reusable by the next app.
7. **Key children by identity, not position.** Use `Stack`/`AutoGrid` keys like
   `"editor"`, a row index or a tile id, so a child appearing or disappearing
   doesn't remount its neighbours.

## Mental model: the `Viewport` flows down

```
window (on_window_size) ─▶ shell::layout()
                              │ minus nav pane (220 / 48), chrome allowance, page padding
                              ▼
                       content Viewport ─▶ screen fn(…, viewport, context)
                              │
            ┌─────────────────┼─────────────────────┐
            ▼                 ▼                     ▼
   Split::primary()   AutoGrid::item_viewport()   Table::detail_viewport()
            │                 │                     │
     .inset(CARD_INSET)  .inset(CARD_INSET)   .inset(CARD_INSET)  ← inside a card/panel
            ▼                 ▼                     ▼
      tab content        gesture card body     key picker in an editor
```

A `Viewport` is just a width in DIPs. It only drives **discrete** choices: how
many columns, table or cards, side by side or stacked. WinUI still does the
actual sizing (`Star` columns, `Stretch`), so a `Viewport` that's a few pixels
off only moves a threshold a little. It never causes overflow. That's why every
decision compares against honest *minimum* widths, and why `shell.rs` keeps a
`CHROME_ALLOWANCE` of slack.

`SizeClass` (Compact < 560 ≤ Medium < 900 ≤ Expanded) is for density decisions
such as page padding. A component that knows its own minimum width should use
`viewport.fits(min)` instead.

## What's in the box

| Need | Use | Notes |
|---|---|---|
| A screen | `page(title, lede, body)` | Title and lede from the type ramp, `XL` gap to the body. |
| Vertical list of blocks | `Stack::vertical(space::L).child(key, view)…` | `.maybe(key, Option<View>)` for optional children. |
| N same-kind fields/cards/swatches | `AutoGrid::new(min_item).max_columns(n).items(…).build(viewport)` | Equal widths, wraps to fewer columns, `Auto` rows (never clips height). `.keep_columns()` for same-size buttons whose count varies (search results), so two results don't stretch to half the row each. |
| Editor + preview | `Split::new(viewport).min_widths(a, b).weights(…)` | Ask `primary()`/`secondary()` first, build the panes, then call `build`. |
| Rows of records | `Table::new(&COLUMNS).rows(…).build(viewport)` | `Column::flex(header, min)`, `fixed`, `actions`. Becomes cards below `min_width()`. |
| Edit one row in place | `Row::new(key, cells).detail(Some(editor(…)))` | The editor spans the row, directly under it. |
| Cancel/Done editor | `editor(title, body, on_cancel, on_done)` | A `panel` with the actions in its header. |
| Let a list be reordered | `reorder::list_footer` + `reorder::row_actions` (app side), built on `reorder_toggle` + `move_buttons` (toolkit) | Size the actions column `TWO_ICON_BUTTONS` so ↑/↓ fits where the row actions were. |
| Labelled field | `field("Label", control)` | Caption above the control. |
| One setting: text left, control right | `setting(title, description, control, viewport)` | A card; the control goes under the text below `SETTING_SIDE_BY_SIDE_MIN`. Pass the width the *card* gets. |
| One setting whose control needs room (a slider) | `setting_wide(title, description, control)` | A card; the control is always under the text, at full width. |
| A few exclusive options | `segmented(&[Choice::new("A"), Choice::disabled("B", "why")], selected, on_select)` | The chosen one is Accent. Re-selecting reports its index again: ignore an unchanged value. |
| A multi-line text field | `text_area(text, placeholder, on_change)` | Wraps; Enter inserts a line. |
| A full-width row in a table | `Row::spanning(key, view)` | A comment, a section label. In card layout it is the view alone. |
| Inputs | `combo`, `number_box`, `text_box`, `switch`, `switch_enabled`, `labeled_switch`, `chip`, `button`, `icon_button`, `add_button` | All pre-filled; `icon_button` always has a tooltip. |
| A bounded number to set by feel *and* exactly | `range_input(value, min, max, step, on_slide, on_type)` | Slider + number box; map both callbacks to one message that ignores unchanged values. |
| Key combo display | `keycaps(&labels, large)` | Joined by `+`. Labels from `key_picker::combo_labels` / `keys::label`. |
| A key that can be changed | `key_field(shown, tooltip, on_open, clear)` | `keycaps` on a filling button; the ✕ stays aligned down a list, greyed when there is nothing to clear. |
| Choosing a key | `key_picker`, or `key_picker_with` for less than a hotkey can hold | `KeyRules` limits the modifiers and keys on offer; `problem` is a line under the combination. Keys are chosen, never typed. |
| Card / panel / note / section | `card()`, `panel(title, trailing, body)`, `note(text)`, `section(title, body)` | `card()` returns the `Border` builder, so `.opacity()` etc. still chain. |
| Text | `text::title/lede/body/strong/caption/section_title/single_line/placeholder` | They return `TextBlock`, so you can still chain. |
| Place a `View` in a `Grid` cell | `cell(view, row, column)` | `.grid_row()` only exists on native builders, so this wraps the view in a `Border`. |
| Page scrolling | `scroll_y(view)` | The shell already does this; screens shouldn't need it. |

## Recipes

### A form of fields

```rust
let fields = AutoGrid::new(110.0).max_columns(3).items([
    ("columns", field("Columns", number_box(Some(4.0), 1.0, 12.0, context.callback(Msg::ColumnsChanged)))),
    ("width",   field("Tile width", number_box(…))),
]);
Stack::vertical(space::L).child("fields", fields.build(viewport)).child("toggle", labeled_switch(…, …, …))
```

Pick `min_item` as the narrowest the *control* is usable at (a `NumberBox` ≈
110, a `ComboBox` of action names ≈ 170). Don't size it from the label: labels
wrap.

### A list of records with an inline editor

```rust
const COLUMNS: [Column; 4] = [
    Column::flex("Name", 140.0),
    Column::flex("Action", 150.0),
    Column::fixed("On", 56.0),
    Column::actions(size::ICON_BUTTON),
];

let table = Table::new(&COLUMNS).empty_message("Nothing yet — add one below.");
let detail_viewport = table.detail_viewport(viewport).inset(CARD_INSET); // inside the editor panel
let rows = items.iter().enumerate().map(|(i, item)| {
    let detail = (editing == Some(i)).then(|| editor(…, body(detail_viewport), cancel, done));
    Row::new(i.to_string(), vec![name, combo(…).into(), switch(…), icon_button(Symbol::Delete, "Delete", …)])
        .detail(detail)
});
table.rows(rows).build(viewport)
```

- Give every column a header except actions. In card layout the header becomes
  the label of its value.
- `flex` columns grow in proportion to their minimums, so `min_width()` is exact.
  Keep minimums honest: they decide when the table turns into cards.
- If a table is wide by nature, consider showing *less* per row and moving the
  rest into the row's editor, like the Tiles tab. A table full of combo boxes is
  hard to scan.

### An editor that opens, edits live, and can be cancelled

Model it in `main.rs`, not in the view:

1. Add a variant to `Editing` holding the index (or id) and an `original`
   snapshot.
2. `Msg::EditX(i)` closes any open editor (keeping its edits) and opens this one,
   or closes it if it's already open (toggle).
3. Field messages write straight to `settings`. The table row and the preview
   update while the editor is open.
4. `Msg::EditorClosed(Revert)` restores `original`; `Keep` just closes.
5. Anything that shifts indices (delete, reload) sets `editing = None` first.

### Two panes: editor and live preview

```rust
let split = Split::new(viewport).min_widths(tab_min_width, 300.0).weights(1.2, 1.0);
let editor = build_editor(split.primary());
let preview = build_preview(split.secondary());
split.build(editor, preview)
```

`min_widths` should come from what's actually in the pane (the Overlay screen
asks the active tab). A preview that can't reflow should scale itself to its
`Viewport`; see `overlay/preview.rs`.

### Adding a control type to `ui/controls.rs`

- Call `.fill()` unless the control is meant to be content-sized (icon buttons).
- Take `Callback<T>`, never `ViewContext`.
- Return the builder if it's still one bare control, so callers can chain
  `.is_enabled()`. Once you call `.content(..)`/`.slots(..)`/`.tooltip(..)` it's a
  `View`; take what callers would chain (`enabled`, `accent`) as parameters.
- Pull any size from `tokens`.

## WinUI traps the toolkit already handles

These come from WinUI's default control styles, not from `windows-reactor`'s
source, so `docs/windows-reactor.md` (which only states what the crate source
shows) doesn't list them. They were inferred from the bugs and from WinUI's
documented defaults; confirm them on a live run if one looks off.

| Trap | Symptom | Handled by |
|---|---|---|
| `ComboBox`/`Button`/`ToggleButton` default `HorizontalAlignment::Left`; `ComboBox` defaults to `VerticalAlignment::Top` | Controls don't fill their column; rows look ragged | `Fill::fill()` in every toolkit control; `Table` centres each cell on the row |
| `SymbolIcon` always draws at 20px and `Button` keeps its ~11px side padding; this crate has no builder for either | An icon button forced to 32×32 crops its icon | `icon_button()` is content-sized; columns reserve `size::ICON_BUTTON` (44). Never give an icon button a fixed size. |
| `ToggleSwitch` style `MinWidth` 154 plus "On"/"Off" text | Overflows narrow columns | `switch()`: `min_width(0)`, zero-size on/off content |
| `View::empty()` in a slot sets the native slot to `null`, and a `null` `ToggleSwitch` `OnContent`/`OffContent` means "show WinUI's localized default" | Cropped "Act…"/"Dés…" in the Steal column | Fill a slot you want blank with a real zero-size element (`Border::new().width(0.0).height(0.0)`), never `View::empty()` |
| A controlled `ToggleButton` clicked while already checked unchecks itself natively; the model says "still checked", so the diff sends nothing | The screen and the model disagree | "Selected" is shown with `ButtonStyle::Accent` (key picker, palette swatches), which is re-rendered from state every time |
| `Slider`/`NumberBox` report a value when it's first set, not only on user input | Opening a screen marks it "Unsaved changes" | In `update`, only write and set `dirty` when the value differs from the current one (see `MinButtonDelayChanged`) |
| `NumberBox` coerces an out-of-range value to min/max and fires `ValueChanged` | Rendering a screen marks it dirty | Pass `None` (an empty box) for "unset" instead of an out-of-range sentinel (see `desktop`). Watch for this with any `minimum` above a value the config can hold. |
| `TextBox` ends a line with ``; a lone `
| `windows-reactor` sets a `TextBox`'s `Text` *before* `AcceptsReturn` when it creates one, and a still-single-line WinUI `TextBox` keeps only the first line of the text it is given | A multi-line note shows as its first line, and the box then reports that truncated line as if the user had typed it, so a handler that trusts it **loses the rest** | Create a `text_area` empty and give it its text one render later (see `ConfigApp::notes_ready` in `y-pop-config`: a message queued from `view`, reset whenever the box is remounted), and ignore what it reports until then |
` shows as no break | A multi-line note renders as its first line only | `text_area()` converts both ways, so app code keeps `
` |
| A `TextBox` copy is a lazy clipboard entry that calls back into the app when pasted | After Ctrl+C, closing the window leaves the process stalling for some seconds and then ending with `0xc0000005` (access violation). Flushing *at exit* doesn't help — the runtime that would answer is already gone — and itself stalls | Hold a `y_core::shell::ClipboardFlusher` in the main component (`start()` in `create`): it flushes on the clipboard's change event, while the window is alive, so nothing lazy is left at exit. Untested remedy: confirm live. |
| `ContentDialog` max width ~548px, `Flyout` width follows its anchor | Pickers cramped | Editors are inline page content (`editor`); if one truly needs a popup, `editor::modal` (an overlay, no Esc/Tab trap) |
| `VariableSizedWrapGrid` has one fixed item height | Cards clip when their text wraps | `AutoGrid` (rows are `Auto`) |
| Fixed-pixel content in a vertical-only scroller | Clipped with no scrollbar | Rule 4: reflow or scale, never rely on sideways scrolling |

## New-screen checklist

- [ ] Signature is `fn screen_view(model…, viewport: Viewport, context: &ViewContext<ConfigApp>) -> View`, wrapped in `page(…)`.
- [ ] No literal sizes, font sizes, opacities or radii (`grep -nE "[0-9]+\.[0-9]" src/<screen>.rs` should show only domain values such as min/max of a field).
- [ ] 2+ sibling fields/cards live in one `AutoGrid` (or a `Table`), never a horizontal `StackPanel`.
- [ ] Every container that narrows width hands its children a narrowed `Viewport`.
- [ ] Tables have honest column minimums; you've looked at the card layout too.
- [ ] Optional children (editors, banners) are keyed, not positional.
- [ ] Works at `shell::MIN_WINDOW` (560×472, title bar included): nothing clipped, nothing sideways.
- [ ] Works at a maximized 1920px window: nothing absurdly stretched (cap `max_columns`).
- [ ] Icon-only buttons have tooltips.
- [ ] `ui-checklist.md` updated for the screen.

## Known limits

- **Layout is verified by reading, not by running.** Every change so far was
  checked statically. The checklist rows marked *verify live* still need a human
  at a real window, resized through the thresholds.
- **`Viewport` is computed, not measured.** `windows-reactor` exposes no
  layout/size-changed event on elements. If WinUI chrome changes size (a
  different `NavigationView` template, a wider scrollbar), adjust
  `shell::CHROME_ALLOWANCE` / pane constants, not the screens.
- **No hover states.** `Border` has pointer-enter/exit events, but a re-render
  per pointer move isn't worth it for row hover. Revisit if a design needs it.
