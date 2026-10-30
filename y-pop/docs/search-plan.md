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
| 4 The search window, first usable version | 👁 |
| 5 Result actions | 🔨 |
| 6 History, Recent, sections, collapsed mode | 👁 |
| 6b Windows's history replaces y-pop's | 👁 |
| 7 Browsing the tray menus | 👁 |
| 8 Prefixes and the hotkey check | 👁 |
| 9 The Start menu section | ⬜ |
| 10 Search-only groups and folders | ⬜ |
| 11 `y-pop-config` support | ⬜ |
| later | Search layer in the boards; plugins |

**Resume here:** the live check of phase 6b (its "Done when" is the first thing to look at: the launch route changed for every app), then of 8.4 (its checklist is under "8.4 built"), then of phase 7 (its checklist is under "What to look at"). Everything through 8 is built (awaiting a live look; the views of the Start menu and of a search-only folder come with phases 9, 10); the next thing to build is phase 9, 10 or 11. The user manual is `docs/user-manual.md`: update it with each feature. Still to look at (👁): all of phase 4 (checklist under its "Done when"), the action menu, the row buttons, Reveal and the configured shortcuts of 5.2–5.6, the start-up balloon and the search hotkey (phase 1), icon quality at 32 and 48 px and `y-snap`'s overlays unchanged (phase 3). Task 1.9 (`tray="true"`) is for later.

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
- **`keywords` now has a field (11.4)**; before that it round-tripped as an
  unknown attribute (`extra_attrs`).
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
| 4.6 | Reload while open: close the window, then swap the tree (same rule as `reload_pending` for the native popup). | ✅ (done with 4.2) |
| 4.7 | Working set: trim after close, like after the native popup. | ✅ decided: no trim |
| 4.8 | Tray right click: the menu plus a separator and **Open Search** (if `<search>`), **About**, **Quit**; left click unchanged. Needs `TrayEvent::ContextMenu` told apart from `Select`, and the extras handled in `menu::build` without disturbing the menu's own command ids. Also the click of the `tray="true"` icon (1.9). | 👁 |

**Done when:** 👁 the user opens search with the hotkey, finds and launches
items from every tray menu, and it closes cleanly; the empty box simply
shows nothing yet (phase 6 fills it).

**What to look at for that 👁** (everything phase 4 built, none of it seen
running yet except the frame of 4.2):

1. Hotkey opens the window; the hotkey again, Esc on an empty box, a click
   outside and Alt+Tab close it.
2. Typing shows the text, caret and selection (Shift+arrows, Ctrl+A);
   Ctrl+C/X/V work; the window grows with the results and shrinks back.
3. Rows: icon, name with the matched letters coloured, second line, hotkey on
   the right; hover selects; the wheel scrolls; ↑ ↓ PgUp PgDn walk the
   results and keep the selection in view.
4. Enter and a click launch an app; Enter on a group goes inside it as a
   chip; Backspace on an empty box, Shift+Tab and Esc come back out.
5. Right click on a tray icon: the menu, a rule, Open Search (with its
   hotkey), About, Quit — and no second About/Quit when the menu has its own;
   left click unchanged; right click on an icon that is a single app shows
   just the three.
6. The look in each skin (`default`, `flat`, `dark`, `native` light and dark).

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
- Not yet: the `Alt+n` hint on the right (done in 5.1).

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

**4.8 built (2026-10-26).** `ConfigTree::tray_extras(root)` (pure, 5 tests in
`config.rs`) decides what a right click adds; `menu::build_with` appends it
(5 tests in `menu.rs`); `YPop::on_message` tells `TrayEvent::ContextMenu` from
the rest.

- The extras follow the menu, set apart by a rule if the menu has anything:
  **Open Search** (only if the file has `<search>`, showing its hotkey),
  **About**, **Quit**.
- **"Already holds" means at the top level of that icon's menu** (or the icon
  is itself a `<quit/>`/`<about/>`). One in a submenu does not count, so the
  right click still offers it. Each icon is judged on its own menu.
- An icon that is a single `<app>` has no menu; its right click shows just the
  extras, which gives it a way to quit.
- Left click and double click are `YPop::run`, as before; hotkeys never show
  the extras.
- Command ids of the extras sit just below the discovered files' base, far
  from any node's.

**4.6 (reload while open) was already done by 4.2.** `YPop::reload` closes
the search window after the new file has loaded and before the tree is
swapped, so a file that does not load leaves an open window alone. Anything
the closed window had already posted is ignored: its generation is stale, or
there is no window to act on.

**4.7 (working set): decided against.** The plan assumed the native popup
trims after closing; it does not (`y-pop` trims once, at start-up, in
`y_core::app::run`). And `y_core::memory` explains why not to do it per use:
trimming frees nothing, the pages come back by page fault on the next touch,
so trimming after every close only slows the next opening. What gives memory
back is dropping what the window held, which a close does: the overlay and its
canvas, the index, the icon pixels and cache, the fonts. To see what an
opening costs, look at Task Manager's *Memory* column before and after
(measured properly in task 9.6, once the Start menu makes the index large).

---

## Phase 5 — Result actions

Spec §2.9 (rev. 2026-10-26: actions are configured, with an action menu).

| | Task | Status |
|---|---|---|
| 5.1 | **The first cut of the keyboard actions** (Ctrl+Enter, Shift+Enter, Ctrl+C, Alt+n) with fixed keys. Superseded by 5.2–5.4: its pieces (the actions, `shell::reveal`, `find_program`, `display::target`, `Key::Digit`) are the engine. | ✅ then reworked |
| 5.2 | **Configuration**: `Chord` (a shortcut, parsed and validated), `ActionId`, `<actions>` in the reader (`menu-key`, `quick-pick`, `<action do key button>`, defaults, every warning), `SearchConfig::actions`, `y_pop::settings` keeping `<actions>` with its comments. Tests. | ✅ |
| 5.3 | **The engine in `State`**: the fixed keys go; bindings come from the configuration. Applicability per entry (admin not for `elevate="true"` programs, Store apps; show-in-folder and copy-path need a file; reveal only for tray-menu items…). `State::actions_for_selected`. Quick pick on/off. Tests. | ✅ |
| 5.4 | **The action menu**: `Alt+Enter` / right click / `Shift+F10` and the Menu key; a list on the selected row with each action's shortcut; ↑↓ Enter, click, the shortcuts, Esc; closes on any other focus change. Drawn by the window, hit-tested by `layout`. | 👁 |
| 5.5 | **Row buttons**: the actions with `button="true"` and a **⋯** on the selected row (`row-actions`), with hit-testing in `layout`. | 👁 |
| 5.6 | **Reveal** (`reveal`, default Ctrl+R) with the native popup: open the item's tray menu at the tray icon and open the submenu chain down to it (best effort; `TrackPopupMenu` can't preselect a row; spec §2.9). If opening nested submenus programmatically proves unreliable, open the tray menu and say which submenu holds it, and note the simplification in the doc comment. | 👁 |
| 5.7 | **Edit** (`edit`, default Ctrl+E): `y-pop-config --select "<path in tree>"`. Needs phase 11.1; until then, Edit opens `y-pop-config` without selecting. | 👁 (`--select` built in 11.1) |

**Done when:** 👁 each action from the menu, from its shortcut and from its
button; a `<actions>` that removes or rebinds some.

**5.2 and 5.3 built (2026-10-26)** — the actions are configured (spec §2.9):

- `search::keys::Chord` (a shortcut, 5 tests): `Ctrl+Shift+Enter` and the
  like, validated. Refused, with a reason: a bare letter/digit/`Del` (it would
  type), `Ctrl+A/C/V/X` (they edit), `Alt+1…9` (quick pick), `Alt+F4`, and
  keys like `Esc` or the arrows. Usable: Ctrl/Alt + letter or digit, Enter
  with any modifiers, F1–F12, Ctrl/Alt + Del. `Key` gained `Function(n)` and
  `Digit(0..9)`; `Key`/`Mods` moved out of `state.rs`.
- `model::{ActionId, ActionBinding, ActionsConfig}`; `<actions menu-key=""
  quick-pick="">` with `<action do key button>` in the reader (10 tests):
  exactly what is listed, in that order; `open` always present (on `Enter`
  unless another action has it); every bad shortcut a warning that keeps the
  action without it; two on one shortcut, or one on the menu key, the second
  loses it; `<actions>` alone does not disable the default sections. The
  defaults are in `ActionsConfig::default`: `Enter` opens, `Ctrl+R` reveals,
  `Ctrl+E` edits, `Ctrl+Del` forgets, menu key `Alt+Enter`; admin,
  show-in-folder, copy-path (and add-to-menu) in the menu only.
- `y_pop::settings` keeps `<actions>` as elements with their comments, bad
  values included (4 tests, byte for byte).
- `search::actions::applies` (7 tests): admin not for `elevate="true"`
  programs nor Store apps or groups; show-in-folder and copy-path need a file
  (copy also takes a Store app's address); reveal only for tray-menu items;
  edit for tray-menu and search-only ones; add-to-menu Start-menu items;
  forget never yet. `Entry::facts` carries what that needs.
- `State` now takes the `ActionsConfig`: a shortcut runs its action if it
  applies, otherwise nothing. The fixed keys of 5.1 are gone (`Ctrl+Enter`,
  `Shift+Enter`, `Ctrl+C`), and **Ctrl+C is text only** again.
  `available_actions` gives the menu's lines; `quick_pick`/`menu_key` expose
  the two switches. `Alt+n` follows `quick-pick`.
- In the app: `Edit` opens `y-pop-config` (not yet on the entry, 5.7);
  `Reveal` only logs (5.6). The menu itself is 5.4.

**5.4 built (2026-10-26)** — the action menu:

- **Opening**: the configured `menu-key` (default `Alt+Enter`), a right click
  on a result row (it selects that row first), `Shift+F10` and the Menu key
  (`Key::ContextMenu`). Nothing opens with no result selected or when no
  action applies.
- **In it** (`State::menu_keypress`, pure): `↑` `↓` wrap, `Home` `End`, `Enter`
  chooses the lit line (whatever `Enter` is bound to outside), `Esc` or the
  key that opened it closes it, and an action's own shortcut works if that
  action is in the menu. Typing waits. Hover lights a line; a click chooses
  it; a click anywhere else only puts the menu away; the wheel closes it.
  Choosing closes it, then does the action; `Open` on a group goes inside it.
- **Looks** (`layout::with_menu`, 5 tests): a 300 px panel (scaled) at the
  window's right edge, tucked under its row, or above it if it would pass the
  tallest the window can be, never over the input row; the window grows to
  hold it. Lines are the action's label and its shortcut on the right; the lit
  line in the skin's selection colours. The footer now names the menu key
  and, while the menu is open, its own keys.
- 9 new `State` tests, 5 new `layout` tests. Painting and the mouse routing
  are the window's, so 👁.

**5.5 built (2026-10-26)** — the row buttons:

- **Which**: on the selected row only, the actions with `button="true"` that
  apply to it (`State::row_buttons`), in the configured order, then a **⋯**
  (Segoe MDL2 `E712`) that opens the action menu. `row-actions="false"`
  (`State::with_row_actions`) removes all of them, the **⋯** too; the menu
  stays reachable by key and right click. A button is a pill labelled with
  `ActionId::short_label` (Admin, Folder, Copy, Reveal, Edit, Add, Forget):
  the menu keeps the long labels, since several buttons share one row.
- **Layout** (`Layout::with_row_buttons`, 5 tests): flush with the row's right
  padding, each as wide as its label plus padding. They may take half the
  row at most; past that the *last* labelled ones are left out (the menu still
  has them) and the **⋯** always stays. `hit_test` finds `Hit::Button(n)` /
  `Hit::More` before the row under them. The row's title, subtitle and hotkey
  end a gap before the strip.
- **Mouse**: hovering a button lights it without touching the selection; a
  click does the action (`State::button_click`) or opens the menu. While the
  menu is open the buttons stay drawn but do not light.
- 3 new `State` tests. Painting and the routing are the window's, so 👁.
- Simplifications: no keyboard focus on the buttons (their actions have
  shortcuts and the menu); a right click on a button does nothing.

**5.6 built (2026-10-26)** — Reveal in menu:

- `menu::reveal_steps(tree, node)` (pure, 4 tests) returns the tray icon whose
  menu holds the node and the keys that walk it there: `Down` once per item
  above the target (one more in the first menu, where nothing is lit yet),
  `Right` to open each submenu, which lights its first item. Separators are
  skipped by the keyboard; a titled separator of a *native* menu is a greyed
  item and is not (`MF_DISABLED | MF_GRAYED`), a themed one is `MF_SEPARATOR`
  and is. Breaks add nothing. `ConfigTree::path_to` gives the chain.
- `YPop::reveal` opens the root's popup at its tray icon
  (`TrayIcon::rect`, new in `y-core`: `Shell_NotifyIconGetRect`; top of the icon,
  centred, falling back to the cursor) after queuing the keys with
  `PostMessageW` of `WM_KEYDOWN`/`WM_KEYUP` to the owner window **before**
  `TrackPopupMenu` starts, so `show_menu` hands them to its menu loop. Posted,
  not `SendInput`: they cannot reach any other window. Nothing happens, with a
  log line, for an item in no tray menu or under an icon with no menu; a
  root that is a group has its menu opened with nothing lit.
- **Not verified live**: that the menu loop takes keys posted to the owner
  rather than to the menu window, and that it does so queued ahead of the
  loop. If it does not, the fix is a `WM_TIMER` on the owner that posts them
  once `WM_INITMENUPOPUP` has been seen, or `SendInput`.
- This departs from the spec's "can open the submenu but not highlight":
  the highlight works too, by key presses. Spec §2.9 should be corrected after 👁.

**Fix (2026-10-26) — the footer under an action menu.** When the menu reached
the footer, the window grew but the footer stayed where it was, under the menu.
`Layout::with_menu` now keeps the room for the footer below the menu and moves
the footer to the new bottom whenever the menu reaches it; a menu that fits above
it leaves it alone (2 tests).

**5.1 (the first cut) built (2026-10-26),** kept for what it left behind: `state::Action` (`Open`, `RunAsAdmin`,
`ContainingFolder`, `CopyPath`) rides on `Outcome::Activate(hit, action)`;
`Key::Digit` for the number row. 7 new `State` tests, 2 for `display`, 2 for
`y_core::shell`.

- **Ctrl+Enter** runs the program elevated this once (a copy of the `App` with
  `elevate` set), apps only. **Shift+Enter** shows the file in Explorer
  (`y_core::shell::reveal`: `explorer /select,`); a bare `notepad.exe` is
  looked up with `find_program` (`SearchPathW`) first; apps and folders, not
  Store apps (no file). **Ctrl+C** copies the path: the program or folder, or
  `shell:AppsFolder\<id>` for a Store app (`display::target`).
- **Ctrl+C has two meanings** and the box wins: text selected in the input is
  copied as text; with none, the selected result's path. (Ctrl+X and Ctrl+V
  are text only.)
- **Alt+1…9** opens that result, counting rows from the top whatever group
  they are under; on a group it goes inside. The rows show `Alt+n` on the
  right when they have no hotkey of their own (a hotkey, when there is one,
  takes the place). The characters Alt+key would type are swallowed, so no
  beep.
- Copying and the actions that need nothing from the app are done by the
  window; the rest are handed to `YPop::run_search_pick` after it has closed.
- Actions that do not apply do nothing (admin on a group, a path of a command),
  and do not go inside a group either; only Enter and Alt+n do that.

---

## Phase 6 — History, Recent, sections, collapsed mode

Spec §2.4, §2.10. After this phase the empty box looks like the mockup.

| | Task | Status |
|---|---|---|
| 6.1 | `history` (pure + file): `y-pop.history` next to the config; lines `key \t count \t last-launch`; keys `menu:<path>`, `start:<target>`, `folder:<path>`; cap 200; written when search closes; unknown/vanished keys skipped and dropped on next write. `history="false"` writes nothing. Tests. | ✅ |
| 6.2 | Ranking bonus from history, computed once per opening. | ✅ |
| 6.3 | Empty box: Recent (`count`, with "2 min ago"), first row selected so hotkey + Enter relaunches; Ctrl+Del forgets one. | 👁 |
| 6.4 | Sections list (one row per section, count, ›); → / Enter / Tab opens one as a scope chip; ← / Backspace comes back. Each section's "opened" content: menus (phase 7), Start menu (phase 9), search-only (phase 10). Until those exist, only sections that work are shown. | 👁 (flat lists) |
| 6.5 | The prefix line under the sections (clickable; phase 8 makes the prefixes work). | 👁 |
| 6.6 | **`collapsed="true"`**: open on the input row alone with a ⌄ button; ↓ or ⌄ unfolds, ↑ on the first row folds, erasing the text folds again, Esc order: clear text, leave section, fold, close. Footer hidden while folded. The window resizes (the overlay is recreated or resized: decide by which is simpler with `UpdateLayeredWindow`). | 👁 |
| 6.7 | `>Clear search history` (with phase 8's commands, or a temporary tray entry). | 👁 (as `>Clear search history`) |

**Done when:** 👁 both opening modes, Recent across restarts, forgetting an
entry.

> **History is no longer y-pop's own**: tasks 6.1, 6.2 and 6.7 below were
> replaced by phase 6b (2026-10-30). `y-pop.history`, its keys and
> `>Clear search history` are gone; the notes are kept for the record.

**6.1–6.3 built (2026-10-26)** — history and Recent:

- `search::history` (pure but for `load`/`save`, 10 tests): `y-pop.history`
  next to the config, `key<TAB>count<TAB>last` per line, newest first, 200
  lines at most. Bad lines are skipped, a key twice keeps the later. Keys are
  `menu:Work/Servers/Putty` (names of the groups above, then its own) and
  `search:Admin/Regedit` for search-only items — `start:` waits for phase 9, and
  a browsed folder's launch is just its `menu:`/`search:` entry. Commands
  (About, Quit…) are not remembered. Two items with the same path and name
  share a history line: accepted.
- **Written when the window closes**, only if something changed. Opening
  prunes it against the index (a vanished or renamed item is dropped on the
  next write). Only `open` and `run-as-admin` count as launches. A group's
  Open enters it, so it is not one. `history="false"`: nothing read, nothing
  written, no bonus, no Recent.
- **Bonus** (`history::bonus`, once per opening via `Searcher::set_bonuses`):
  up to 40 for how often (2 a launch, 20 launches) plus 20 within a day or 10
  within a week; at most 60 (`MAX_BONUS`) on a 0–1000 score. It moves things
  inside a band and across a close neighbour, never a scattered match over a
  prefix.
- **Recent**: `State::start` receives the rows (`history::listing`, at most
  `count` of `<recent count>`, only for items that still exist) and shows the
  empty box: a `RECENT` header and the launches, the newest selected, so hotkey
  then Enter relaunches it. The second line leads with `display::ago`
  ("2 min ago · Work › Servers · …"). Typing replaces it; erasing the text
  brings it back; inside a group it is not shown. No `<recent>` in a `<search>`
  that lists its sections: no list (but launches are still recorded, for the
  bonus).
- **Forget** (`Ctrl+Del`, the menu, a button): applies on a Recent row only —
  `actions::available(…, forgettable)` — and stays open, the selection keeping
  its place (`State::remove_recent`). It was `false` everywhere before.
- 19 new tests. The painting of the header and the time are the window's, so 👁.

**6.4 built (2026-10-26), the flat-list form** — the sections list:

- `search::sections` (`rows`, 3 tests): what the empty box lists under
  **BROWSE**, in configuration order, one `Row::Section` each: **Tray menus**
  (`<menus>`, implied) and each search-only **group**, if they hold an app, with
  their app count on the second line. Not listed because they have nothing to
  open yet: `<start-menu>` (phase 9), search-only folders (10.2), plugins. The
  list is computed once per opening.
- **Opening** (`Enter`, `→`, `Tab`, a click): the section becomes a chip, like a
  group entered with `Tab` — `Place::Menus` or `Place::Group(id)` replaces the
  `NodeId` scopes (`State::scopes` now returns `&[Place]`). Inside, the empty
  box lists the section's items: the tray icons for **Tray menus** (the roots,
  whose groups `Tab` goes into), the group's direct children for a group.
  Typing searches only there (`Scope::Menus` is new: entries of the tray menus,
  not the search-only ones). `←` on an empty box, or `Backspace`, comes back
  with the section's row selected again.
- Selection rests on section rows as on results (`first_selectable`); they have
  no actions, buttons or menu, and are not counted by `Alt+n`.
- **Replaced later, section by section**: the tray menus by Miller columns
  with a details pane (7.1) and the A–Z list (7.2), both done; the Start menu
  gets 9.4.
- 10 new `State` tests, 1 `Scope` change. Painting a section row (folder glyph,
  name, "N apps", ›) is the window's, so 👁.

**Fix (2026-10-26) — menu mnemonics in names.** A tray menu's names carry
`&` markers (`&Word`, `Publ&isher`, `&Office`). Search drew them raw, where GDI
swallowed the `&` and underlined the next letter, and the matched-letter runs
were drawn as separate pieces, so the first letter vanished. `matcher::plain`
now strips them (`&&` is a literal `&`) in the index (name and path), on the
rows, in the second line's group path and in the chips. A whole-name prefix
match now sees the first letter too. The tray menu itself keeps its markers, and
history keys use the raw names, which are the identity.

**8.1, 8.2, 8.3, 8.5 and 6.5, 6.7 built (2026-10-26)** — the prefixes:

- A **prefix** is the first character of the box, on the outermost box only
  (inside a section or a group `>` is text): `State::prefix`. The text after it
  (`rest`, trimmed) is the query. `search::commands` (pure, 2 tests) holds
  `Command`, `Line` (the rows of a prefixed list that are not index entries:
  a command, a prefix, the search hotkey, a key check) and `PREFIXES`.
- **`>` commands** (8.1, 6.7): Reload configuration, Open settings, Edit
  `y-pop.xml`, Open configuration folder, Clear search history, Start with
  Windows (shows On/Off), About, Quit; found by label, or at 60 % by other
  words (`restart` finds Reload). *Refresh the Start menu* and *Reload plugins*
  join with phases 9 and later. Enter → `Outcome::Command`, handed to the owner
  (`SearchWindow::take_command`, `YPop::run_command`) after the window has
  closed — except *Clear search history*, which the window does itself since it
  holds the history (it also deletes `y-pop.history`). Start with Windows
  toggles `HKCU\…\Run` under `ipc::STARTUP_NAME`.
- **`#` hotkeys** (8.2): empty, the search hotkey first, then every entry with a
  hotkey, in index order; with text, the entries with a hotkey ranked as in a
  plain search (`Scope::Hotkeys`; a combination typed in any order matches as a
  set). Enter launches what the key opens. The search hotkey's row closes.
- **`#<combination>` check** (8.3, `search::keycheck`, 3 tests): a *complete*
  combination (`y_core::hotkey::Hotkey::parse`) that no entry and not the search
  hotkey uses adds a **Line::Check** row. The window answers it on the next
  layout (`Open::probe_check`, once per typed combination): a
  `HotkeyManager::register` with `Fallback::None` on the overlay's own window,
  then `unregister` at once. Refused: named from a built-in table of the keys
  Windows keeps (`keycheck::RESERVED`, 23 of them) or "taken" with a note that
  `override="true"` could still claim it. Free: Enter opens `y-pop-config`
  (plain open, no `--select` yet; see 5.7). Nothing happens on Enter otherwise.
- **`?`** (8.5): the prefixes that work, Enter types one; the same list is the
  line under the sections.
- **The line of prefixes** (6.5): the last row of the empty box, not selectable
  (`Row::selectable`), a chip for each prefix (`Layout::with_prefix_chips`,
  `Hit::Prefix`, 2 tests); a click types it (`State::prefix_click`).
- 21 new tests (state 13, layout 2, display 2, commands 2, keycheck 3). 👁 for
  the painting, the probe (`RegisterHotKey` on the search window: a key held by a
  program must be seen as taken), the commands and the toggle of Start with
  Windows.
- **8.4, `/`** came later: see "8.4 built" under phase 8.

**6.6 built (2026-10-26)** — `collapsed="true"`:

- **State** (`State::with_collapsed`, 8 tests): `folded` is true on opening; a
  folded box has no rows and nothing selected, so `Enter`, the action menu and
  the shortcuts do nothing. `↓`/`PgDn` or a click on **⌄** unfold it to Recent
  (or the group entered) with the first row selected. Folds again on: `↑` on the
  first row of the empty box, `Esc` with nothing typed (the order is text,
  group, fold, close), and erasing the text with no group entered. Typing
  unfolds. Leaving a group with `Backspace`/`Shift+Tab` does not fold (you asked
  to see it). Not collapsed: none of this exists.
- **Layout**: `Layout::with_chevron` (2 tests) puts a 26 px **⌄** (`Hit::Chevron`)
  at the input row's right end, and the text field stops before it. It is shown
  only while folded.
- **Window**: the overlay is created at the tallest size, footer included
  (`Open::tallest`; the surface cannot grow), and `relayout` drops the footer
  while folded, so the window is one row high and grows when unfolded. The
  window is placed for the tallest size, so unfolding never pushes it off the
  screen.
- **Decision**: the spec's "Esc clears the text, then leaves a section, then
  folds, then closes" has one overlap: clearing the text also folds (the same
  rule as erasing it), so it is one `Esc`, not two. Written here so it is not a
  surprise.

---

## Phase 6b — Windows's history replaces y-pop's

Decided 2026-10-30: y-pop lists what Windows remembers, not a list of its own;
what it launches reaches that list; and more than `count` can be reached.
Spec §2.4, §2.10.

| | Task | Status |
|---|---|---|
| 6b.1 | `y_core::recent`: the `Recent` folder, `UserAssist` (read only), `add_file`, `tracks_files`, the two forgets. `y_core::apps_folder::all_apps`. Tests, on real data too (counts only). | ✅ |
| 6b.2 | `search::history` rewritten: `listing` (pure) merges launches and files, makes a launch of a tree item that item, keeps the rest if `shell:AppsFolder` knows it, and gives the bonuses. `Target` and `Source::History`, `Kind::File`, `Heading::Files`. | ✅ |
| 6b.3 | `launch::Usage`: a logged launch takes the shell with `SEE_MASK_FLAG_LOG_USAGE`, a document goes to `SHAddToRecentDocs`. | 👁 |
| 6b.4 | **All recent**: `Place::Recent`, `with_recent_shown`, the row after Recent, the list behind it, filtering. | 👁 |
| 6b.5 | Forget reaches Windows. `>Clear search history` removed. | 👁 |
| 6b.6 | `shell="false"` is respected and not recorded (`App::shell` is now an `Option`). `open-toggle-history`, default `Ctrl+Shift+Enter`, with `Usage::Forced`. | 👁 |

**Done when:** 👁

1. A program launched from a tray menu appears in Recent at the next opening,
   as that tray item. **This is the claim nothing could test without launching
   something**: that `SEE_MASK_FLAG_LOG_USAGE` reaches `UserAssist` on this
   Windows, and for a Store app through its `shell:AppsFolder` address. If it
   does not, the fallback is writing the 72-byte record ourselves, which y-pop
   avoids because the format is undocumented.
2. Every kind of app still launches as before now that all of them take the
   shell: arguments, working directory, `%clipboard%`, `elevate`, a `.bat`, a
   browsed file, a Store app.
3. Recent shows what the Start menu's lists show; with recent items on in
   Settings, files too; a file opened from a browsed folder is listed.
4. **All recent**: the row, `Enter`, `→` and a click, scrolling, filtering,
   coming back. Its second line says when Windows keeps no recent files.
5. `Ctrl+Del` on an app and on a file: gone at the next opening, and from the
   Start menu; the file itself still there.
6. `history="false"`: no Recent, and a launch leaves no record.
   `shell="false"`: launched directly, no record, and `Ctrl+Shift+Enter` records
   it; on any other app `Ctrl+Shift+Enter` leaves no record. Whether the shell
   records a document by itself on that route is unknown: check a file too.
7. The hotkey feels as quick as before, right after y-pop starts too.

**Built (2026-10-30).** What the real data said, on the machine this was
written on (read-only, counts only):

- `UserAssist` holds 173 launches and reads in about 2 ms, so it is read at each
  opening. Its run count is 0 on most records that have a time (11 of 187 had
  one), so a record is judged by its **time**; the 72-byte layout held.
- Its ids are `shell:AppsFolder`'s, for the exe key. 60 match, which is what
  drops the installers and temporary programs. The shortcuts' key (`.lnk`
  paths) matches none of them: it is only used to recognise a tree item.
- `shell:AppsFolder` takes 365 ms to enumerate in a debug build: too slow for a
  hotkey. `AppNames` reads it on its own thread, starting when y-pop does; only
  an opening with nothing read yet waits. It is reread behind the scenes when
  older than ten minutes, so a new install can take that long to be listed.
- `Start_TrackDocs` is 0 here and the `Recent` folder empty: recent files can
  only be seen with that switch on. No test exercised a real file row.

Decisions:

- **A launch of a tree item is that item**, matched by app id, full path, or the
  file name when `exe` has no folder. So a row keeps its name, hotkey and
  actions, and nothing is listed twice.
- **History-only items are search results too** (`Source::History`, headed
  Apps or Files). They need `<recent>`: without it, only the tree's items get
  the bonus. Files come from their shortcut's name: a file in a tray menu can
  be listed twice, since matching it would mean opening every shortcut.
- **A user-asked launch takes the shell** (`Usage::Logged`), since the flag
  exists only there, **unless the app says `shell="false"`**: that stays
  `CreateProcessW` and is not recorded. No attribute is not the same as `false`
  (`App::shell` is an `Option`), or nothing would be recorded by default. The
  global `shell-execute` plays no part: the shipped `y-pop.xml` sets it to
  `false`, and `y-pop` does not apply it to apps anyway (`y-pop-config/docs/plan.md`).
  A launch that fails through the shell shows the shell's message rather than
  only logging. `start`/`exit` events and `y-pop-config`'s Run stay
  `Usage::Silent`, on the old route.
- **`open-toggle-history` is one action, not two**, so one key does the opposite
  of `Enter` on any item, and its label says which way. It is not offered with
  `history="false"`. `Usage::Forced` takes the shell this once. Being an action,
  it exists in search only; a `<search>` with its own `<actions>` has it only if
  listed.
- **Forget acts on Windows**, every record of the item, as the Start menu's
  *Remove from list* does. A second list of hidden items would have brought the
  private history back.
- **`>Clear search history` is gone**: emptying the system's history from a
  launcher is too much for one `Enter`, and Windows has a setting for it.
- **All recent is a row, not a prefix**: it follows Recent, so `↓` reaches it,
  and it opens like any section, as a chip. It is shown only when there is more
  than `count`. Results in it are one flat list.
- A vanished file is not filtered out: checking could block on a network drive.
  Opening it shows the shell's message; Forget removes it.

---

## Phase 7 — Browsing the tray menus

| | Task | Status |
|---|---|---|
| 7.1 | `<menus browse="columns">`: Miller columns (three visible), breadcrumb, details pane with Open / Reveal / Edit; ← → between columns, ← from the first column leaves the section; browsed folders listed when their column opens (`browse::list`), never in advance. | 👁 |
| 7.2 | `browse="a-z"`: alphabetical list with letter headers and a clickable letter index. | 👁 |
| 7.3 | `include="…"` limits the section (and the plain search) to some tray menus. | ✅ |

**Done when:** 👁 walking every menu with the keyboard only, and with the
mouse only.

**7.1 built (2026-10-30)** — `search::columns` (7 tests, pure); with 7.2 and 7.3,
21 new in `State`, 9 in `layout`, 5 in `files`, 2 in `display`, 2 in `config`:

- **The focused column is `State::rows`.** Moving, shortcuts, the action menu
  and `Alt+n` needed no change, and a row of it is a `Hit` like any other. Only
  the columns to its left and the preview are new: `Columns` holds them,
  `State::column_panes` hands them to the window, which draws them instead of
  the rows. Typing replaces the columns with results; erasing finds them as they
  were (`Columns::showing`, and each column keeps its selection). `Tab` into a
  group from those results stacks on top, and `Backspace` comes back to the columns.
- **Files of a folder are index entries**, `Source::Column`, which no search
  scope sees, with `Target::File`. So `Files::app`, `icon`, `detail`, the actions
  and `Pick::File` work as for `/`, and Reveal and Edit do not apply. Going into a
  folder twice lists it twice: the index only lives as long as the window.
- **`Layout::columns`**: a 26 px breadcrumb over 320 px of panes, the height fixed
  so that the window does not jump, in three equal thirds. `Layout::rows` holds the
  focused column too, which is what the action menu hangs from. Others are reached
  by `Hit::Pane(pane, row)`, the buttons by `Hit::Details(n)`. A column other than
  the focused one scrolls by itself to show its selection, which is also where it
  starts when it takes the focus (`Open::column_depth`).
- Rows are 30 px with 20 px icons from the 32 px cache; the details pane has a
  second cache at 48 px (`IconCache` has one size each).

Decisions, to confirm or change:

- **A browsed folder is read on `→`, on a thread, not when selected** (the mockup
  previews its files). Previewing would read a folder for every row walked past,
  and block on a slow share; the plan said "when their column opens". Selected, it
  shows its details with `→ lists what is inside`. `Files::list_column` runs one
  listing at a time: a later one replaces it, and a result nobody waits for is
  dropped. The column says `Listing…` meanwhile. At most `CAP` files.
- **`Enter` on a group or browsed folder goes in**, as in the mockup. **Open** on
  the details pane or in the action menu opens the folder in Explorer.
- **A click selects; a click on a group or folder also goes in.** An app is opened
  from the **Open** button: the window handles no double click, and opening on a
  second click would launch things by accident, since going in selects the first
  row.
- **Hover does not select in the columns**: it would change the preview, and with
  it what is under the pointer one column over.
- **`Esc` leaves the section at once from any column**, as in the mockup;
  `Backspace` goes back one column, like `←`. **`Home`/`End`** go to the ends of
  a column.
- The details buttons are the configured `open`, `reveal`, `edit` that apply, in
  the configured order, and stay with `row-actions="false"`: they are the pane's,
  not a row's.

Simplifications: the wheel scrolls the focused column wherever the pointer is; a
line of the details pane is cut with an ellipsis rather than wrapped; `Reveal`
from a column closes the window, as it does from a result.

**7.2 built (2026-10-30)**: `Row::Letter` (a `RowKind::Header`), `State::letters`,
`letter_click`; `Spec::band`, `Layout::with_letters`, `Hit::Letter`,
`layout::scroll_to_top`.

- **Apps only** (spec §2.4): no groups, no browsed folders, no built-ins. Sorted
  on the folded name, ties in file order. A name that does not start with a
  letter goes under `#`, first.
- **The letters are a band above the list, not a row in it** (the mockup's is
  sticky): after a jump to `M` they must still be there for a jump to `T`.
- A jump puts the letter's header on top, unless the window would no longer be
  full: `Z` does not leave three rows in a shrunken window.
- Typing searches as usual, and the band goes until the text is erased.

**`browse="list"` added (2026-10-30), after the fact.** 7.1 had replaced the flat
form of 6.4, the plan having called it temporary; the user wanted to keep it. It is
`MenusBrowse::List` (and `settings::MenusMode::List`), and needs nothing of its own:
`Place::Menus` lists the tray icons, and a group is entered as `Place::Group`.

**7.3 built (2026-10-30).** `SearchConfig::includes_menu`, `Entry::left_out`.

- **What is left out stays in the index**, and only `Scope::All` and `Scope::Menus`
  skip it, with the sections' counts, the columns and A–Z. Dropping it would have
  made `#` say that a hotkey of a left-out menu is unused, and `/` lose its folders.
  Recent still lists it: that list is Windows's.
- Names are compared like search compares: case, accents and `&` aside. A name
  no tray menu has is a warning; it comes after the load, since `<search>` can
  stand before the menus.

What to look at for the 👁:

1. **Tray menus** opens on the columns: the tray icons, the first one's content,
   a breadcrumb, the footer's keys. `↑↓ → ←` walk every menu; `←` from the first
   column comes out on the **Tray menus** row; `Esc` too, from any depth.
2. An app shows its details: icon (at 48 px: sharp?), path, program, hotkey,
   tooltip, Open / Reveal / Edit, each working. `Enter`, `Ctrl+R`, `Ctrl+E` and
   the action menu work too. Does the text fit a third of the window?
3. A browsed folder: its details, then `→` lists it (`Listing…` on a slow one),
   sub-folders go further when `depth` allows; a file opens; **Open** on the
   folder opens Explorer. A `\\…` folder does not freeze the window.
4. The same with the mouse only: a click on a group or folder goes in, a click on
   a column to the left comes back, right click opens the action menu.
5. Long columns: the selection stays in view, the thumb moves, the wheel scrolls.
6. Typing inside shows results; erasing brings the columns back where they were.
7. `browse="a-z"`: the letters, a click on one, the last letter, typing.
   `browse="list"`: the tray icons, `Enter` into a group as a chip, back out.
8. `include="…"`: the section's count, the columns, the plain search; `#` still
   lists a left-out menu's hotkeys.

---

## Phase 8 — Prefixes and the hotkey check

Spec §2.7, §2.14.

| | Task | Status |
|---|---|---|
| 8.1 | `>` commands: reload, open settings, edit `y-pop.xml`, open configuration folder, refresh Start menu, start with Windows, about, quit. (Clear history was dropped by 6b.) | 👁 (without Start menu refresh, which has no Start menu yet) |
| 8.2 | `#` hotkeys: every binding (search's own included), filtered. | 👁 |
| 8.3 | `#<combination>` check: `RegisterHotKey` probe on the search window, unregistered at once; built-in table of keys Windows reserves; "free" row opens `y-pop-config` ready to assign (plain open until 11.x). | 👁 |
| 8.4 | `/` files in browsed folders: listed on first `/` per opening on a background thread, results posted back (`WM_APP` pattern) and appended without moving the selection; network folders skipped unless `index="true"`. | 👁 |
| 8.5 | `?` lists the prefixes. | 👁 |

**Done when:** 👁 each prefix; `/` stays responsive on a slow folder.

**8.4 built (2026-10-30)** — `search::files` (10 tests), and 5 new in `State`, 1
in `display`:

- **The thread** (`Files::start`) walks with the same `browse::list` as the tray
  menus, so `filter`, `depth` and `open-folder` mean the same; that moved
  `browse` into the library. Breadth-first over all the folders at once, local
  ones before network ones. It sends a folder at a time, so a slow folder does
  not hold back what is already found, and wakes the window with
  `WM_SEARCH_FILES` at most once between two takes, so a walk through thousands
  of small folders does not flood the queue.
- **Results** append to the `Index` as `Source::BrowsedFile`, `Target::File`, so
  rows already shown stay valid. `Scope::All` leaves them out and `Scope::Files`
  is all it sees; `/` is one flat list under **FILES**, in listing order with
  nothing typed, ranked otherwise. `State::files_changed` keeps the selection on
  the same result, and an open action menu with it.
- **Running one** is what the tray menu runs: `Pick::File(App)`, with the
  folder's `shell` and `elevate`, through the same `run_app_action`.
- **A note at the end of the list** (`Line::Scan`, not selectable): `Listing
  folders… N found`, then only if there is something to say: the cap, network
  folders skipped, or no browsed folders at all. Last, so that no row moves when
  it goes. On a long list that means scrolling down to read about a skipped
  folder.
- **`index="true"`** is on any browsed `<app>`; `Browse::index`. In `y_pop::
  settings` it round-trips as an unknown attribute until 11.4 gives it an editor.

Decisions, none of them in the spec:

- **Sub-folders are results too**, opened in Explorer. A folder of project
  checkouts is one of §2.5.3's uses, and it is the folders you look for there.
- **At most 5000 files per opening** (`files::CAP`), and the note says so.
- A file's name is searched, its full file name (with the `.exe` the list hides)
  and the browsed folder's name, so `/scripts dep` narrows.
- **No icon for a file on a network folder**: icons load on the window's thread,
  and reading one there would block typing.
- **No `Run as administrator` on a file** (it needs `Kind::App`). `elevate="true"`
  on the folder still applies, as in the menu.
- **A mapped network drive is not a network folder**: only `\\…` is, as the spec
  says. It is listed, and the thread is what keeps that from blocking.
- A thread stuck on an unresponsive share cannot be interrupted: it ends when the
  system gives up. Another opening's `/` starts a thread of its own.

**Fix (2026-10-30) — a crash on `/`.** `Listing folders…` has no second line, and
`Canvas::draw_text_ellipsis` handed the empty text to `DrawTextW`, which read
through the dangling pointer of an empty `Vec` (access violation in `user32`).
Empty text now draws nothing, in `y-core`, for every caller; a test in
`overlay.rs` reproduced the crash at the same address before the fix.

What to look at for the 👁:

1. `/` lists the files of the tray menus' and the search-only browsed folders,
   and opens one (a file, then a sub-folder in Explorer). `/scripts dep` narrows.
   A plain search does not find them.
2. The action menu on a file: Show in folder, Copy path; no Reveal, no Edit.
3. A file opened from `/` shows up in Recent, as one from a tray menu does.
4. A big folder: rows join while the selection stays put, typing stays quick,
   and closing the window meanwhile is clean.
5. A `\\…` folder: skipped and said so; with `index="true"`, listed, no icons.
6. The line of prefixes has four chips and they fit the window.

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

Can start any time after phase 1; 11.1 and 11.2 unblock 5.7 and 9.5.

| | Task | Status |
|---|---|---|
| 11.1 | Command line `--select "<path in tree>"`; if already running, forward it and bring the window to the front. **As built:** the path is the positions among siblings, `2/0/5` (`ipc::path_of`, `roots::resolve_path`), not names. The second launch writes `y-pop-config.request` next to `y-pop.xml` (`ipc::post_request`); the open window polls for it twice a second on a background task, as nothing here owns the window procedure. Search-only items have no path until 11.3: Edit opens the app without selecting. | 👁 |
| 11.2 | `--new-app` with prefilled fields (from the Start menu): `--name`, `--target`, `--args`, `--icon`, `--dir`. **As built:** adds an app with those fields to the open tray menu (else the first, else a new one) and selects it, exactly as the toolbar does. It does not open the picker or focus a field (the config plan's 11.2 "new items focus their first field" is still open). | 👁 |
| 11.3 | **Reworked after a review on 2026-10-30 (11.3.1–11.3.8 built, verify live, 5.2 left): see `y-pop-config/docs/search-screen-plan.md`.** A **Search** node in the outline: hotkey, collapsed, results, row actions, history; the sections list (add, remove, reorder); search-only groups and folders edited like menu items. | ⬜ |
| 11.4 | `keywords` field in the item inspector, and `index` for a browsed folder. **As built:** `Item::keywords` and `AppFields::index` in `y_pop::settings` (loaded, written, round-tripped); a Keywords box beside Tooltip for apps, groups and About/Quit/Settings; a "Search inside it with /" switch under a folder app's Filter, Depth and Open fields. Attributes now save in the order name, icon, tooltip, keywords, then the app's own. | 👁 |

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
