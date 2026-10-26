# y-pop search — development plan

How to build the search described in [`overlay-modes.md`](overlay-modes.md) §2
(revision 4) and shown in its mockup, one phase at a time. Each phase ends with
something that builds, passes `cargo clippy --workspace --all-targets` with no
warnings and `cargo test --workspace`, and can be merged on its own. The user
commits; nothing here is committed by the session that does the work.

This file is the running progress record, like `y-pop-config/docs/plan.md`.
Update a task's status in the same change that does the work.

Status: ⬜ todo · 🔨 in progress · ✅ done · 👁 done, verify live (the user
runs it and reports; sessions don't drive the desktop themselves)

## Where we are, and how to resume

| Phase | State |
|---|---|
| 0 Decisions | ✅ |
| 1 `<search>` in the configuration | 👁 |
| 2 Search core: matcher and index | ✅ |
| 3 Text input and canvas primitives | 👁 |
| 4 The search window, first usable version | 🔨 |
| 5 Result actions | ⬜ |
| 6 History, Recent, sections, collapsed mode | ⬜ |
| 7 Browsing the tray menus | ⬜ |
| 8 Prefixes and the hotkey check | ⬜ |
| 9 The Start menu section | ⬜ |
| 10 Search-only groups and folders | ⬜ |
| 11 `y-pop-config` support | ⬜ |
| later | Search layer in the boards; plugins |

**Resume here:** phase 4, tasks 4.6 and 4.7 (small: reload while open is already handled; trim the working set on close), then 4.8 (tray right click). Open 👁 checks: typing, the list, hover, click, wheel, Enter launching, Tab into a group and Backspace/Shift+Tab/Esc back (4.3–4.5), the start-up balloon and the search hotkey (phase 1), icon quality at 32 and 48 px and `y-snap`'s overlays unchanged (phase 3). Task 1.9 (`tray="true"`) is for later.

---

## What exists already (2026-10-26)

Read before starting; most of the Win32 plumbing search needs is there.

| Need | Already in the code |
|---|---|
| A focusable layered window with a message handler | `y_core::overlay::Overlay::with_handler`, `OverlaySpec` (its "let the window take keyboard focus" flag drops `WS_EX_NOACTIVATE`), `Overlay::activate` |
| Drawing | `Canvas`: `fill_rect`, `stroke_rect`, `draw_text`, `restore_alpha`, `round_corners_with_border`; `Font::measure`; `ColorFont` |
| Theme colours for an overlay | `y-pop/src/tooltip.rs` already colours an overlay from the popup's `MenuDesign` |
| Launching | `y_pop::launch::launch(hwnd, &App)` (elevation, Store apps, `%clipboard%`) |
| Browsed folders | `y_pop::browse::list(dir, &Browse, level)` |
| Icons | `y_core::icon::IconCache` (`for_file`, `for_store_app`, `for_directory`), `Icon::to_bitmap(size)` |
| Hotkeys | `y_core::hotkey::HotkeyManager`, `Hotkey::parse`, `key_from_name`, `key_name`, `Fallback::LowLevelHook` |
| Start menu sources | `y_core::known_folders` (`StartMenuPrograms`, `CommonStartMenuPrograms`), `y_core::file_info::read_shell_link`, `y_core::apps_folder::store_apps` |
| Open windows (for a later plugin) | `y_core::open_windows::running_programs` |
| Reload from `y-pop-config` | `y_pop::ipc` (`reload_message`, `request_reload`) |
| The editable model `y-pop-config` saves | `y_pop::settings` (`load`, `save`) |

Missing, and built by this plan: drawing an icon onto a `Canvas`; icons
larger than menu size; a caret and a selection; `y-pop-config` command-line
arguments; anything that knows about `<search>`.

---

## Phase 0 — Decisions

Settle these before writing code. Each has a recommendation.

| # | Question | Recommendation |
|---|---|---|
| 0.1 | `overlay-modes.md` §4 builds `MenuLayout` and the `list` presenter before search. Must search wait for them? | **No.** Search walks `ConfigTree` into its own index (phase 2) and opens its own window (phase 4). It needs neither `MenuLayout` nor the `Presenter` trait. Reveal (phase 5) uses the native popup that exists today. Update §4 to put search first. |
| 0.2 | Where does the code live? | `y-pop/src/search/` (one module per concern: `config`, `index`, `matcher`, `input`, `layout`, `window`, `history`, `start_menu`). The matcher and the input state machine take no `y-pop` type, so they can move to `y-core` when `y-term` needs them (spec §2.12). The `Canvas` additions go to `y-core` straight away. |
| 0.3 | How are search-only groups and folders stored? | In the same `ConfigTree` arena as everything else, under a new `ConfigTree::search` (not in `roots`), so `NodeId`, `launch`, icons and `browse` work on them unchanged. `roots` keeps meaning "tray icons". |
| 0.4 | Do `start`/`exit` events fire for search-only apps? | Yes: `all_nodes` should walk them too, or the `<app event="start">` user puts in a search-only group silently does nothing. Check every other `all_nodes` caller when changing it. |
| 0.5 | One search window kept alive, or created per opening? | Created per opening and dropped on close, like the native popup: no memory held between uses. Rebuilding the index is per *load*, not per opening. Revisit only if opening is visibly slow. |

**Done when:** the table is answered (here) and §4 of `overlay-modes.md`
says the same. ✅ All five recommendations accepted as written; §4 now
puts search first (2026-10-26).

---

## Phase 1 — `<search>` in the configuration

Parse it, keep it, register its hotkey. No window yet: the hotkey logs
"search would open" and nothing else.

| | Task | Status |
|---|---|---|
| 1.1 | `model.rs`: `SearchConfig { hotkey: Option<HotkeyBinding>, collapsed, results: Sections\|Flat, row_actions, history, sections: Vec<Section> }` and `enum Section { Recent { count }, Menus { browse: Columns\|AZ, include: Vec<…> }, StartMenu { exclude: Vec<…> }, Group(NodeId), Folder(NodeId), Plugin(…) }`. `ConfigTree::search: Option<SearchConfig>`. | ✅ |
| 1.2 | `config/reader.rs`: read `<search>` (root only; a second one warns). No children → `[Recent{5}, Menus, StartMenu]`; children → exactly those, plus `Menus` if missing. `<group>`/`<app browse="true">` children become nodes in the arena (0.3). `<hotkey>` inside a search-only group warns. `<plugin>` parses and warns "not implemented yet" (kept for later). Unknown child: warning. | ✅ |
| 1.3 | `keywords="…"` attribute on any node (stored interned, like the other strings). | ✅ |
| 1.4 | `popup-style="runner"`: warning becomes "replaced by `<search>`" (spec §1.1). | ✅ |
| 1.5 | `all_nodes` reaches search-only nodes (0.4); review its callers. | ✅ |
| 1.6 | `main.rs`: register the search hotkey with `HotkeyManager` (same `override` fallback as other bindings); `WM_HOTKEY` for it logs and returns. (The tray entry became task 4.8.) | 👁 |
| 1.9 | `<search tray="true">` (default off): parsed in the reader and `y_pop::settings`, round-tripped. `main.rs` adds a tray icon for search after the menus' icons; its click will open search (phase 4). | ⬜ |
| 1.7 | **`y_pop::settings` keeps `<search>`.** Today an element the editable model doesn't know is saved back as a *comment*, so saving from `y-pop-config` would disable search. Load/save `<search>` and its children as real elements, round-trip tested, even before `y-pop-config` has a screen for it (phase 11). | ✅ |
| 1.8 | Tests: parsing defaults, each section, every warning, `keywords`, settings round trip with a `<search>` that has every kind of child. | ✅ |

**Done when:** a `y-pop.xml` with the spec §2.2 example loads with no
warning except the plugin one, and survives a load/save in `y-pop-config`
unchanged. 👁: the hotkey writes its log line.

**Built (2026-10-26)** — tests in `config.rs` and `settings/tests.rs`:
`model::{SearchConfig, Section, …}`, `Reader::read_search`, `NodeData::keywords`,
`Settings::search` (`SearchSettings`, round-tripped byte for byte, variable
rename and "used in" reach the search items), `ConfigTree::search_hotkey`
(the id after the last node hotkey), `main.rs` registers it and logs
"search would open".

Decisions taken while building, to confirm or change:

- **`include`/`exclude` are `;`-separated**, not space-separated as the spec
  example had it: a tray menu can be called "Admin tools". Spec §2.2/§2.5
  should say so.
- **`keywords` has no field in `Settings::Item` yet**: it round-trips as an
  unknown attribute (`extra_attrs`) until 11.4 gives it an editor.
- **`<plugin>` keeps `allow` and `section`** as known attributes (not
  warned), but only `name`, `src` and `prefix` are modelled.
- **No "Search" element and no search icon by default** (decided). Search
  is reached from the **right-click menu** of any tray icon: left click opens
  the menu as written; right click opens the same menu followed by a
  separator and the built-in extras **Open Search** (only when `<search>`
  exists), **About**, **Quit**. Right-click does nothing different today
  (`main.rs` treats `Select`/`ContextMenu`/`DoubleClick` alike): that is
  phase 4's task 4.8. An optional `tray="true"` on `<search>` adds a tray
  icon of its own for search (task 1.9). Confirmed: an extra whose
  kind the menu already holds (`<about/>`, `<quit/>`) is not added twice.
- Spec §2.5's `browse="false"` / `global="false"` on sections are not parsed
  (`browse` already means columns/a-z on `<menus>`); they warn as unknown
  attributes. Needs a spec decision before phase 6.

---

## Phase 2 — Search core: matcher and index

Pure Rust, no Win32, fully unit-tested. Spec §2.6, §2.8, §2.12.

| | Task | Status |
|---|---|---|
| 2.1 | `matcher`: fuzzy subsequence, case- and accent-insensitive (fold once when indexing, not per query), word-start and run bonuses, gap cost; returns score and matched character positions. Scoring order of spec §2.8: whole-name prefix > word-start prefix > acronym > substring > scattered. | ✅ |
| 2.2 | Hotkey queries: when the query starts with a modifier, compare as a set of keys (`shift+ctrl+esc` = `Ctrl+Shift+Esc`); prefix matches on a partial combination. | ✅ |
| 2.3 | `index`: one flat `Vec<Entry>` (source, `NodeId` or source-specific id, kind, folded fields: name, keywords, exe file name, path in tree, tooltip, hotkey). Built from `ConfigTree` at load and reload; other sources append later (phases 9, 10). | ✅ |
| 2.4 | `query(&Index, &str, scope) -> Vec<Hit>`: field weights (name 1, keywords 0.8, exe 0.6, path 0.45, tooltip 0.4, hotkey strong), commands ranked lower, ties in config order, "matched by" field recorded for the second line. A reusable result buffer: no allocation per entry per keystroke. | ✅ |
| 2.5 | Result grouping (`results="sections"`: Top hit, then groups, at most five each) and `flat`. | ✅ |
| 2.6 | Tests: ranking cases from the mockup (`out` → Outlook, `cmd` → Command line by exe, `work rem`, `win+n`), accents, ties, grouping; a timing test on 5,000 synthetic entries with a generous bound (catches an accidental quadratic, not a benchmark). | ✅ |

**Done when:** the tests above pass; nothing is wired to a window yet.

**Built (2026-10-26)** in `y-pop/src/search/` (`matcher`, `hotkey`, `index`,
`query`, `results`; 51 tests). Choices made while building:

- **Score bands**, so the spec's order holds whatever the lengths: prefix
  850–900 > word-start prefix 700–750 > acronym 550–600 > substring 400–450 >
  scattered 1–300 (a shorter text wins inside a band). Table in `matcher.rs`.
- **Folding is one char to one char** (`ß`→`s`, `æ`→`a`, `œ`→`o`), so match
  positions in the folded text are positions in the displayed name.
- **A query is split into words; each must match** some field (`work rem`),
  scored by its best field × the field weight, averaged. The "matched by"
  field is the first one that wasn't the name.
- **Hotkey queries**: a query whose first word is a modifier is also compared
  with each entry's hotkeys as a key set: same set 1000, same modifiers with
  the key still being typed 900, a partial combination 700. Alias spellings go
  through `y_core::hotkey::key_from_name`/`key_name`.
- **Commands** (`About`, `Quit`, `Settings` nodes, later the `>` ones) are
  scored at half. Browsed folders are `Kind::Folder`; their files are not
  indexed.
- `Scope` has `All` and `Within(group)` only. The `include="…"` limit of
  `<menus>` arrives with phase 7.
- No camelCase word boundaries (text is folded to lower case before matching).

---

## Phase 3 — Text input and canvas primitives

| | Task | Status |
|---|---|---|
| 3.1 | `input` (pure): text, caret, selection; insert, Backspace/Delete, Ctrl+Backspace/Delete (word), Home/End, Shift+arrows, Ctrl+A, paste (text given by the caller), replace-all (for prefix buttons). Caret and selection in UTF-16 units to match GDI measuring. Tests. | ✅ |
| 3.2 | `y_core::overlay::Canvas::draw_icon(&Icon, x, y, size)`: through `Icon::to_bitmap` or `DrawIconEx` into the canvas DC; alpha kept correct (premultiplied, like the rest of the canvas). Test on a known icon's pixels. | ✅ |
| 3.3 | Larger icons: `IconCache` keyed by size as well (spec §1.2.4); search uses 32 px rows and 20 px browse rows. Check `ExtractIconExW`'s large slot vs `SHGetImageList(SHIL_EXTRALARGE)`; pick by result quality, note why in the doc comment. | ✅ |
| 3.4 | Canvas: caret (1–2 px bar), selection highlight behind text, text with ellipsis in a width (`Font::measure` + trim). Respect the draw-order rule (fill, text, `restore_alpha`, corners last). | ✅ |

**Done when:** tests pass; `y-snap`'s overlays unaffected (its tests and a
quick 👁).

**Built (2026-10-26).** `y-pop/src/search/input.rs` (13 tests); in `y-core`:
`Icon::to_pixels`, `IconCache::with_size`, `Canvas::{blit, draw_icon,
draw_caret, draw_text_ellipsis}`, `Font::caret_offsets`. Choices:

- **`Input` keeps byte offsets inside and reports UTF-16 units**; keys step
  by character (an emoji is one Backspace, two units). Text capped at 200
  characters; control characters dropped, line breaks and tabs become spaces.
- **A cache has one size** (`IconCache::with_size(32)`) rather than keying
  every entry by size: search uses one size per cache anyway. Tiers: ≤16 px
  small slot, ≤32 px the large slot (`ExtractIconExW`/`SHGFI_LARGEICON`, a
  true 32 px frame, downscaled for 20 px rows), >32 px the shell image
  factory (the 48/256 px frames; needs an existing file, ignores an icon
  index, falls back to the large slot). Stock and the generic folder icon
  stop at 32. The choice is by reasoning, not by comparing renders: 👁.
- **The selection highlight needs no primitive of its own**: `fill_rect`
  before the text, between two `Font::caret_offsets` entries; the caret is
  `draw_caret` after it.
- **Ellipsis is `DT_END_ELLIPSIS`** rather than measure-and-trim, which is
  exact for free; it also sets `DT_NOPREFIX` so `Q&A` stays `Q&A`.
  (`draw_text` itself keeps its old behaviour, `&` prefix included.)
- `Canvas::blit` blends (source over, premultiplied) where the fills
  replace; it is pure pixel work, so it is safe on either side of GDI text.
- `store_logo` now shares `shell_image` with the sized lookups (same retries,
  same result); COM is initialised inside it.

---

## Phase 4 — The search window, first usable version

The smallest search that is worth using: hotkey, type, see ranked results
from the tray menus, Enter launches, Esc closes.

| | Task | Status |
|---|---|---|
| 4.1 | `layout` (pure): from the rows to show and the window width, the rectangles of the input row, each result row, headers, footer; hit-testing a point to a row. Width 560–720 px by monitor, scaled by DPI. Tests. | ✅ |
| 4.2 | `window`: a focusable `Overlay` with its own `WindowHandler`. Placed in the upper third of the monitor under the cursor (`y_core::monitor`). `SetForegroundWindow` from the hotkey path; if activation fails, show "click to type". Close on `WM_ACTIVATE`/`WA_INACTIVE`, Esc, click outside. | ✅ |
| 4.3 | Keyboard: `WM_CHAR` into `input`, `WM_KEYDOWN` for navigation (↑ ↓ PgUp PgDn Enter Esc Tab Shift+Tab). Mouse: hover selects, click opens, wheel scrolls. | 👁 |
| 4.4 | Rendering: input row (icon, chip for the scope, text, caret, placeholder), result rows (icon, highlighted matched characters, second line, hotkey or Alt+n on the right), section headers, footer of key hints. Colours from the active skin (`MenuDesign`, like `tooltip.rs`); `panel-alpha`/`tile-radius` skin additions can wait. | 👁 |
| 4.5 | Enter: launch through `launch::launch`; on a group: Tab-style scope into it (chip), Backspace on empty / Shift+Tab goes back. Icons loaded synchronously for the visible rows only (async is `y-pop-plan.md` step 7, not needed yet). | 👁 |
| 4.6 | Reload while open: close the window, then swap the tree (same rule as `reload_pending` for the native popup). | ⬜ |
| 4.7 | Working set: trim after close, like after the native popup. | ⬜ |
| 4.8 | Tray right click: the menu plus a separator and **Open Search** (if `<search>`), **About**, **Quit**; left click unchanged. Needs `TrayEvent::ContextMenu` told apart from `Select`, and the extras handled in `menu::build` without disturbing the menu's own command ids. Also the click of the `tray="true"` icon (1.9). | ⬜ |

**Done when:** 👁 the user opens search with the hotkey, finds and launches
items from every tray menu, and it closes cleanly; the empty box simply
shows nothing yet (phase 6 fills it).

**4.2 built and looked at (2026-10-26).** `y-pop/src/search_window.rs`, in
the binary like `tooltip.rs` (the library stays Win32-free). The handler only
posts private messages carrying the opening's generation to the tray window.

**4.3 built (2026-10-26).** The behaviour is `y_pop::search::state::State`
(pure, 11 tests): `Event`s in (`Char`, `Paste`, `Key`), `Outcome`s out
(`Redraw`, `Close`, `Activate(hit)`, `Copy`, `Paste`). Decisions:

- **Esc** clears the text, and closes only when there is none (it was "close"
  in 4.2). **Up/Down** stop at the ends rather than wrap; headers are skipped;
  **PgUp/PgDn** move `PAGE` = 5 results. **Tab** does nothing until 4.5.
- **Left/Right/Home/End** edit the caret, never the list. Ctrl+A/C/X/V work
  on the input through the new `y_core::clipboard` (`text`, `set_text`; no
  test, since one would overwrite your clipboard).
- **The index is built for each opening** from the tree as it is (cheap at
  menu sizes) and dropped with the window; revisit in phase 9 when the Start
  menu makes it big.
- **Hover and click are wired through `Layout::hit_test` but have no rows to
  land on yet**: the window is still its fixed first size. That arrives with
  4.4, which also needs the overlay to change height (a layered window's
  surface is fixed at creation, so the canvas will be made at the largest
  size and `UpdateLayeredWindow` shown at the size in use) and a rect-based
  `round_corners_with_border`.
- `Enter` / a click only log `search: would open "<name>"` until 4.5.

**4.4 built (2026-10-26).** Pure parts in the library, painting in
`search_window.rs`:

- `search::display` (5 tests): a title cut into matched/unmatched runs in
  UTF-16 units, the heading captions (`TOP HIT`, `APPS`…), and the second
  line: where the thing is (`Only in search · Work › Servers · mstsc.exe`,
  `4 items`, `Command`), or what matched when it was not the name
  (`Keywords: mail email`, `Program: OUTLOOK.EXE`, `Hotkey: Win+O`).
- `layout::max_height`, `State::highlight`.
- In `y-core`: `Overlay::show_at_size` / `refresh_size` (the surface is made at
  the tallest the window gets and shown at the height in use, since a layered
  window's surface cannot grow) and `Canvas::round_rect_corners_with_border`
  (the curved border on part of the surface; the whole-surface one calls it).
- The window relayouts after every change; typing returns to the top of the
  list, keys keep the selection in view, hover and the wheel do not.
  Wheel: 3 rows per notch. A thin thumb shows where the visible rows sit.
- Rows: 32 px icon (resolved like the tray menus', `menu::resolve_icon`, loaded
  when the row is first shown), name with the matched letters in an accent
  colour (on the selected row: full text colour against a slightly dimmed
  rest), second line, the hotkey on the right (`show-shortcuts` respected).
  Selected row: the skin's selected colours, or a tint of the accent over the
  panel without a skin; rounded. The input row has a search glyph
  (`Segoe MDL2 Assets`, Windows 10/11).
- Names too long for their box are cut with an ellipsis, without emphasis.
- Not yet: the `Alt+n` hint on the right (task 5.1).

**4.5 built (2026-10-26).** Mostly in `State` (7 new tests, 18 in all):

- **Enter** (or a click) on a **group** goes inside it; on anything else it
  acts. **Tab** goes inside the selected group and does nothing on an app.
  Going in clears the text that found the group, and the box lists what the
  group holds, in file order, one flat list. Typing then searches under it
  (any depth), and the placeholder says `Search in <group>…`.
- **Back out**: Backspace on an empty box, or Shift+Tab, drops the last chip.
  **Esc** clears the text, then drops a chip, then closes.
- Groups nest: each chip is one level (`Work`, `Servers`).
- **Chips** in the input row, before the text; text scrolled out of the field
  is covered by the glyph's and chips' area rather than running under them.
- **Acting**: the window closes and hands the chosen node to the app
  (`SearchWindow::take_activation`), which runs it with the same `YPop::run`
  as a pick from a tray menu: an app launches (elevation, Store apps,
  `%clipboard%`), a browsed folder opens, `<quit/>`, `<about/>` and
  `<settings/>` do what they say. Closing first lets the new program take the
  foreground.
- Not done: a chip's own × to click; History (phase 6) will record the pick
  here.

---

## Phase 5 — Result actions

Spec §2.9.

| | Task | Status |
|---|---|---|
| 5.1 | Ctrl+Enter (run as administrator), Shift+Enter (containing folder), Ctrl+C (path, `%vars%` expanded), Alt+1…9. | ⬜ |
| 5.2 | Row buttons on the selected row (`row-actions`), with hit-testing in `layout`. | ⬜ |
| 5.3 | **Reveal** (Ctrl+R) with the native popup: open the item's tray menu at the tray icon and open the submenu chain down to it (best effort; `TrackPopupMenu` can't preselect a row; spec §2.9). If opening nested submenus programmatically proves unreliable, open the tray menu and say which submenu holds it, and note the simplification in the doc comment. | ⬜ |
| 5.4 | **Edit** (Ctrl+E): `y-pop-config --select "<path in tree>"`. Needs phase 11.1; until then, Edit opens `y-pop-config` without selecting. | ⬜ |

**Done when:** 👁 each action from a result row and from its key.

---

## Phase 6 — History, Recent, sections, collapsed mode

Spec §2.4, §2.10. After this phase the empty box looks like the mockup.

| | Task | Status |
|---|---|---|
| 6.1 | `history` (pure + file): `y-pop.history` next to the config; lines `key \t count \t last-launch`; keys `menu:<path>`, `start:<target>`, `folder:<path>`; cap 200; written when search closes; unknown/vanished keys skipped and dropped on next write. `history="false"` writes nothing. Tests. | ⬜ |
| 6.2 | Ranking bonus from history, computed once per opening. | ⬜ |
| 6.3 | Empty box: Recent (`count`, with "2 min ago"), first row selected so hotkey + Enter relaunches; Ctrl+Del forgets one. | ⬜ |
| 6.4 | Sections list (one row per section, count, ›); → / Enter / Tab opens one as a scope chip; ← / Backspace comes back. Each section's "opened" content: menus (phase 7), Start menu (phase 9), search-only (phase 10). Until those exist, only sections that work are shown. | ⬜ |
| 6.5 | The prefix line under the sections (clickable; phase 8 makes the prefixes work). | ⬜ |
| 6.6 | **`collapsed="true"`**: open on the input row alone with a ⌄ button; ↓ or ⌄ unfolds, ↑ on the first row folds, erasing the text folds again, Esc order: clear text, leave section, fold, close. Footer hidden while folded. The window resizes (the overlay is recreated or resized: decide by which is simpler with `UpdateLayeredWindow`). | ⬜ |
| 6.7 | `>Clear search history` (with phase 8's commands, or a temporary tray entry). | ⬜ |

**Done when:** 👁 both opening modes, Recent across restarts, forgetting an
entry.

---

## Phase 7 — Browsing the tray menus

| | Task | Status |
|---|---|---|
| 7.1 | `<menus browse="columns">`: Miller columns (three visible), breadcrumb, details pane with Open / Reveal / Edit; ← → between columns, ← from the first column leaves the section; browsed folders listed when their column opens (`browse::list`), never in advance. | ⬜ |
| 7.2 | `browse="a-z"`: alphabetical list with letter headers and a clickable letter index. | ⬜ |
| 7.3 | `include="…"` limits the section (and the plain search) to some tray menus. | ⬜ |

**Done when:** 👁 walking every menu with the keyboard only, and with the
mouse only.

---

## Phase 8 — Prefixes and the hotkey check

Spec §2.7, §2.14.

| | Task | Status |
|---|---|---|
| 8.1 | `>` commands: reload, open settings, edit `y-pop.xml`, open configuration folder, clear history, refresh Start menu, start with Windows, about, quit. | ⬜ |
| 8.2 | `#` hotkeys: every binding (search's own included), filtered. | ⬜ |
| 8.3 | `#<combination>` check: `RegisterHotKey` probe on the search window, unregistered at once; built-in table of keys Windows reserves; "free" row opens `y-pop-config` ready to assign (plain open until 11.x). | ⬜ |
| 8.4 | `/` files in browsed folders: listed on first `/` per opening on a background thread, results posted back (`WM_APP` pattern) and appended without moving the selection; network folders skipped unless `index="true"`. | ⬜ |
| 8.5 | `?` lists the prefixes. | ⬜ |

**Done when:** 👁 each prefix; `/` stays responsive on a slow folder.

---

## Phase 9 — The Start menu section

Spec §2.5.1. On by default, so it must be cheap and never block.

| | Task | Status |
|---|---|---|
| 9.1 | `start_menu` worker thread: walk `StartMenuPrograms` and `CommonStartMenuPrograms` for `.lnk`, `read_shell_link` each; `apps_folder::store_apps`. Filter uninstallers, read-me, help and web links; `exclude="…"`. Result posted to the window owner and appended to the index. COM initialised on that thread. | ⬜ |
| 9.2 | Cache for the life of the process; refresh on `SHChangeNotifyRegister` for the two folders; re-enumerate Store apps in the background at most once per opening. "Reading the Start menu…" until the first result. | ⬜ |
| 9.3 | De-duplication against menu apps by target exe or AUMID: the menu entry wins in results; the section still lists everything. | ⬜ |
| 9.4 | Section view: A–Z, Start-menu folder on the second line. | ⬜ |
| 9.5 | **Add to a menu** (Ctrl+P, ＋): `y-pop-config --new-app` with name, target, arguments, icon, working directory (phase 11.2; until then copy the target and say so). | ⬜ |
| 9.6 | Measure: working set before and after the first opening with a typical Start menu; record the number here. | ⬜ |

**Done when:** 👁 finds installed apps and Store apps, no duplicate of a
menu entry, opening search is not slower than in phase 8.

---

## Phase 10 — Search-only groups and folders

Spec §2.5.2, §2.5.3. Parsing exists since phase 1; this is the runtime.

| | Task | Status |
|---|---|---|
| 10.1 | Index them (labelled "only in search"); each top-level one is a section; Reveal hidden for them, Edit kept. | ⬜ |
| 10.2 | A search-only folder's section lists its files when opened, → enters sub-folders; `/` includes its files. | ⬜ |

**Done when:** 👁 the spec §2.2 example's Admin tools and Scripts.

---

## Phase 11 — `y-pop-config` support

Can start any time after phase 1; 11.1 and 11.2 unblock 5.4 and 9.5.

| | Task | Status |
|---|---|---|
| 11.1 | Command line `--select "<path in tree>"`; if already running, forward through `y_pop::ipc` (a second registered message, or `WM_COPYDATA`) and bring it to the front. | ⬜ |
| 11.2 | `--new-app` with prefilled fields (from the Start menu); opens the "add" flow of phase 8 of `y-pop-config/docs/plan.md`. | ⬜ |
| 11.3 | A **Search** node in the outline: hotkey, collapsed, results, row actions, history; the sections list (add, remove, reorder); search-only groups and folders edited like menu items. | ⬜ |
| 11.4 | `keywords` field in the item inspector. | ⬜ |

**Done when:** 👁 search configured entirely from `y-pop-config`.

---

## Later

- **The search layer inside the boards** (spec §2.13): once `grid`/`radial`
  exist; reuses phases 2–4 scoped to one group.
- **Plugins** (spec §2.11): after the boards. `wasmi`, worker thread,
  capability list, the `y-pop-plugin` crate, then Calculator, Windows,
  Projects.
- **Async icons**: if phase 4's synchronous icons for visible rows are
  noticeably slow, `y-pop-plan.md` step 7's worker pool, shared with the
  native popup.
- **IME**: v1 relies on the system IME window placed at the caret
  (`ImmSetCompositionWindow`); inline composition only if asked.
