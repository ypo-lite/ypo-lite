# PortAL — source analysis

Reading notes on `../guipro-git/PortAL` (the C/C++ original) ahead of designing
`y-pop`. Two things already exist there worth knowing about before writing a
line of `y-pop` code:

* **`PortAL/REFACTORING.md`** — a C++17 redesign document written against the
  same codebase, with a full feature inventory, global-variable inventory and
  a proposed object model. Treated below as the most reliable feature list,
  since it was written by re-reading every source file.
* **`PortAL/portal-rust/`** — an earlier, unfinished Rust rewrite attempt
  (~4700 lines across 15 files, not part of this workspace, not using
  `y-core`). It reached phase 3–4 of its own migration plan (tray icon,
  hotkeys, launch, most of `menu.rs`). Worth reading module-by-module as prior
  art — particularly `menu.rs` (owner-draw) and `xml_config.rs` — before
  reinventing those decisions, but it is not a dependency: `y-pop` follows
  `y-core` conventions instead (see [[y-pop-plan]]).

## Feature inventory

Everything `portal.xml` can currently express, and the C++ module that
implements it (`PortAL/src/*.cpp`):

| Feature | Module | Notes |
|---|---|---|
| XML config loading, `%appdata%`/binary-dir search order | `xmlconfig.cpp`, `config.cpp` | Auto-reloads on file change (watcher thread). |
| `<group>` / `<app>` / `<sep>` / `<about>` / `<quit>` node tree | `portalConfig.cpp` | `group` at root → tray icon; `app` at root → launches directly on tray click. |
| Up to 10 hotkeys per node (`key`/`mod`/`over`, `key1`/`mod1`/`over1`, …) | `hotKey.cpp` | `over` = low-level hook override, same concept as `y_core::hotkey::Fallback::LowLevelHook`. |
| Menu accelerator keys (`&` in `name`) | `menu.cpp` | Win32 native (`&` in `AppendMenuW` text), not part of the hotkey system. |
| Variable expansion: builtin (`%win%`, `%mydocs%`, …) + user-defined (`<variables><var/></variables>`) + `%clipboard%`/`%clipboards%` (lazy) | `common.cpp` | See table in `configuration.md`. |
| Application launch: `CreateProcess`, `ShellExecute` (`shell="true"`), elevation (`elevate="true"`), RT/Store apps | `launch.cpp` | `defaultshell` is a global default, overridable per-node. |
| Folder browsing (`browse="true"`, extension filter, recurse) — expands to a dynamic submenu of a folder's contents | `launch.cpp`, `config.cpp` (`g_portal_files`) | Not present in the `portal-rust` attempt at all; **easy to under-scope**. |
| Icon loading from exe/ico/dll (`ico="path,index"`), sync or per-icon background thread | `iconManager.cpp` | See below — this is the async-icon precedent for the "load icons async" PoC. |
| Optional path-string deduplication (`USE_PATH_STORAGE` build flag, off by default) | `pathStorage.cpp` | See below — the memory-footprint precedent. |
| Menu theming: 3 built-in skins (`default`/`flat`/`dark`) + OS-derived default + custom `<skin>` (base/selected color pairs, border, gradient background, gradient icon backdrop, edge style) | `menu.cpp`, `xmlconfig.cpp` | Owner-draw (`WM_MEASUREITEM`/`WM_DRAWITEM`) — the single largest module (46 KB `menu.cpp`). |
| Shortcut text column (`showshortcut` global default, `showsc` per-node override) | `menu.cpp` | Right-aligned hint text, not a real accelerator — cosmetic only. |
| `default="true"` → bold item | `menu.cpp` | Maps to `MFT_STRING` + bold via owner-draw, or `SetMenuDefaultItem` if not owner-draw. |
| `break="true"` → menu column break | `menu.cpp` | `MF_MENUBREAK`. |
| `autoopen="true"` → open tray submenu on hover, no click | `trayIcon.cpp` | Documented as "requested but not recommended." |
| Events: `start`/`exit` (`lock`/`unlock` never implemented) | `launch.cpp` | Fire-and-forget launch of the node's `exe` at PortAL start/exit. |
| Tray icon per top-level `<group>`/`<app>`, balloon notifications | `trayIcon.cpp` | One `NOTIFYICONDATA` per root node — `y_core::tray::TrayIcon` already covers the single-icon case; multiple roots means multiple `TrayIcon`s. |
| Config file watcher, live reload | `config.cpp` | `FindFirstChangeNotification` thread → `PostMessage` → reload on the main thread. Same shape as `y-snap`'s hook pattern. |
| Version check (opens GitHub releases page) | `updater.cpp` | Low priority; arguably out of scope for a "do just what they need to do" rewrite — flag as optional. |

## Icon loading (`iconManager.cpp`) — the async precedent

Confirms the shape the user described for the "load icons async" PoC:

- A flat `Vec<iconLoader>` cache keyed by filename (or by extension alone for
  unknown-icon files — an exe/dll/ico/lnk always gets its own entry, anything
  else shares one cache slot *per extension*, since `SHGetFileInfo` returns
  the same generic icon for every file of a given type).
- Reference-counted: `preloadIcon` bumps a counter on repeat requests,
  `unloadIcon` decrements and frees at zero.
- Two load paths, chosen once per process by `iconloaderThread` (a config
  bool, not per-icon):
  - **Sync**: `_loadIcon` runs inline, on the thread building the menu.
  - **Async**: one `CreateThread` *per icon*, each with its own mutex; the
    owner-draw `WM_DRAWITEM` handler (`retrieveIconFrom`) tries a 10 ms
    `WaitForSingleObject` on that mutex and, if the icon isn't ready yet,
    registers the requesting `PortalMenuItem*` on the loader so the thread
    knows who to notify; on completion the thread `PostMessage`s a private
    `WM_MYREDRAWITEM` (per item) then `WM_MYREDRAWMENU` (whole menu) back to
    the main thread.
- Known problems worth *not* reproducing: cleanup calls `TerminateThread`
  (unsafe, can leak a stuck `CoInitializeEx`); the async loader retries
  `SHGetFileInfo` in a `Sleep(5)` loop up to 30 times with a comment admitting
  the author never found out why the first call was flaky; `HICON` crosses
  threads with no synchronization other than the mutex around the assignment.
  A Rust port gets the same *shape* (background load → post a redraw) without
  needing a thread per icon or `TerminateThread`, i.e. a small thread pool /
  work queue with a `mpsc` channel and cooperative shutdown, matching the
  `portal-rust` attempt's own sketch (`icon_manager.rs` — never wired up).

## Path deduplication (`pathStorage.cpp`) — the memory precedent

This is the "special structure to avoid duplication of *path*" the user
mentioned. It was opt-in (`USE_PATH_STORAGE`), off by default, and never
finished (`remove`/`search` are both stubs that `return false`/`0`
unconditionally). The idea: entries form a **prefix trie stored as a flat
`Vec`**, not a tree of pointers —

- `add(path)` walks existing entries looking for a *shared prefix* with an
  existing entry (`wcscmpidx`, longest common prefix length). If a long-enough
  common prefix is found partway through an existing entry, that entry is
  **split** into a shared-prefix parent slice and its original-suffix child,
  and the new path is added as a sibling child under the same parent.
  Otherwise, if a full existing entry is itself a prefix of the new path, it
  recurses one level deeper (`next_parent`) to keep splitting further down;
  otherwise a brand new root entry is appended.
- Each slice stores its own suffix text plus a `parent` index (1-based, `0` =
  root) and the **total reconstructed length**, not just its own suffix's
  length — `get(id)` walks parent pointers writing suffixes backward into a
  buffer it has already sized correctly from that stored total.
- So `C:\Program Files\A\x.exe` and `C:\Program Files\B\y.exe` end up sharing
  one `C:\Program Files\` slice, referenced by two children — which is
  exactly the case a portable-launcher config full of `%programfiles%\...`
  entries hits over and over.
- This is real complexity for real payoff only at scale (many hundreds of
  `app` nodes with long shared path prefixes) — see [[y-pop-plan]] for how to
  get the same effect idiomatically in Rust without hand-rolling a trie.

## Skin / owner-draw menu (`menu.cpp`)

The largest and gnarliest C++ module (46 KB) — `WM_MEASUREITEM` +
`WM_DRAWITEM` + a hand-rolled `PortalMenuDesign`/`PortalMenuItem` linked-list,
GDI gradient fills, and (per `common.h`) an experimental subclassing hack
(`DEV_MENU_DESIGN_SUBCLASSING_FOR_BORDER`) just to avoid the native white
border around an owner-draw popup — never finished. This module is the
strongest argument in [[y-pop-plan]] for *not* going the owner-draw-`HMENU`
route at all and instead rendering the popup as a `y_core::overlay::Overlay`
window the way `y-snap` already does, which sidesteps `WM_MEASUREITEM`/
`WM_DRAWITEM` and the border problem entirely.

### The theme data model (`menu.h`'s `PortalMenuDesign`), exact

Read in full for the [[y-pop-plan|Phase 5 gap analysis]] — not just the
feature list above, the actual struct and the three built-in skins' literal
values:

```c
typedef struct _ColorPair     { COLORREF textcolor; COLORREF background; } ColorPair;
typedef struct _ColorGradient { UINT16 direction; COLORREF start; COLORREF end; } ColorGradient;

typedef struct _PortalMenuDesign {
    UINT16 border_size;
    UINT16 border_round;
    COLORREF border_color;
    ColorPair* base;                  // NULL = "use system colours"
    ColorPair* selected;              // NULL = "use system highlight colours"
    ColorGradient* background_gradiant;
    BYTE edge;                        // PORTAL_ST_NONE/SEL/UNSEL/ALL bitmask
    ColorGradient* icon_gradiant;
} PortalMenuDesign;
```

`g_PortalMenuDesign == NULL` means "no skin at all" (plain native `HMENU`,
no owner-draw) — `y-pop`'s `Theme::Native` is the direct equivalent. The
three built-ins (`menu.cpp`, module-scope):

**Corrected 2026-08-28** — the table below originally read the literals as
`#RRGGBB` hex strings and was wrong for every non-symmetric colour in it,
caught only once a `y-pop` build made from these numbers rendered yellow
where PortAL actually shows blue. `menu.cpp`'s `design1`/`design2`/`design3`
assign these as raw C integer literals straight into a `COLORREF` field
(`0xf9d996,	// border_color`, not a hex-string parse at all), and
`COLORREF` is little-endian `0x00BBGGRR` — so `0xf9d996`'s true channels are
B=`0xf9`, G=`0xd9`, R=`0x96`, a light **blue** (`RGB(0x96, 0xd9, 0xf9)`), not
tan. This is the same BGR/RGB gotcha the "custom `<skin>` XML" section right
below correctly describes for `xmlconfig.cpp`'s `getColor` bug — just not
applied here, to these literals, the first time this table was written.
Every colour cell below is now the correctly-decoded one; the two symmetric
ones (`#ffffff`, `#000000`) were never wrong since channel order doesn't
matter for them.

| Skin | border (size/round/color) | base | selected | bg gradient | edge | icon gradient |
|---|---|---|---|---|---|---|
| `default` | 1 / 10 / `#96d9f9` (blue) | — (system) | — (system) | `#ffffff → #96d9f9`, vertical | `SEL` | — |
| `flat` | 1 / 0 / `#a8d2fd` (blue) | — (system) | `#000000` on `#edf4fc` (pale blue) | — | none | — |
| `dark`/`black` | 0 / 0 / — | `#ffffff` on `#000000` | — (system) | `#96d9f9 → #000000`, horizontal | `ALL` | `#96d9f9 → #000000`, horizontal |

None of the three sets both `base` *and* `selected` — every built-in leans
on system highlight/menu colours for at least one state, which is why
`InitMenuVars` always reads `COLOR_MENUHILIGHT`/`COLOR_HIGHLIGHTTEXT`/
`COLOR_MENUTEXT`/`COLOR_GRAYTEXT` up front regardless of whether a skin is
active — they're the fallback for whichever half of `base`/`selected` a
given skin left `NULL`. `y-pop`'s `MenuColors` has no such "half-set,
fall back to system colour" mode — every one of its six slots is always a
concrete value (from the base theme, then overridden) — a deliberate
simplification worth naming explicitly if this is ever cited as a
compatibility gap: it isn't one, since `y-pop.xml` isn't `portal.xml` (see
[[y-snap-porting-decisions]]).

**Custom `<skin>` XML** (`xmlconfig.cpp::loadMenuSkin`, documented in
`configuration.md`'s "Skin" section):

```xml
<skin base="" selected="#000:#FCF4ED" edge="true">
  <border size="1" round="5" color="#ffffff"/>
  <background color="#000000:#ffffff" direction="v"/>
  <icon color="#ffffff:#000000"/>
</skin>
```

- `base`/`selected` attributes: a **colour pair**, `text,background` (comma
  *or* space *or* `|` *or* `:` as separator — `wcstok_s` with all four in its
  delimiter set), each colour 3 or 6 hex digits, `#` optional. An empty or
  malformed pair colour parses to black (`getColor` returns `_wtoi` on
  anything that isn't 3 or 6 hex chars, which is `0` for `""`) — not a
  parse error, not a fallback to "unset."
- `edge`: `true`/`sel`/`unsel`/anything-else — `PORTAL_ST_ALL`/`_SEL`/
  `_UNSEL`/`_NONE`. Controls a vertical `EDGE_ETCHED` rule between the icon
  column and the text, per row, gated on whether *that row's* selected
  state is in the bitmask — not a whole-popup border.
- `<border size="" round="" color="">`: two bugs worth knowing about before
  treating this as a spec rather than prior art — `color` is never actually
  read; the code reads `item.attribute(L"round").value()` a second time
  instead (copy-paste of the line above it, confirmed by reading
  `loadMenuSkin`), so a custom skin's border is *always* black
  (`getColor("")` → `0`) regardless of what `color=` says. And `getColor`
  parses a 6-hex-digit string as `wcstol(..., 16)` straight into a
  `COLORREF`, which Windows defines as `0x00BBGGRR` — so `color="#RRGGBB"`
  as documented actually *sets* `0x00RRGGBB`, i.e. red and blue are swapped
  from what the hex string visually says, for every colour PortAL parses
  this way (`base`, `selected`, gradients, border, all of it). Neither bug
  is worth reproducing; `y-pop`'s `y_core::overlay::Color::parse` (already
  reused for `*-color` attributes per [[y-pop-plan]]) does the channel
  order correctly and `border-color` is wired to something that's actually
  read.
- `<background>`/`<icon>`: a **gradient**, `getColorPair`'s `textcolor`/
  `background` repurposed as `start`/`end` (same "colour pair" string
  syntax, not a second parser) plus `direction="h"`/`"v"` (default
  horizontal). If the string only has *one* colour (no separator found),
  `<background>` falls back to setting `selected->background` to that one
  colour instead of allocating a gradient at all (a `NULL`-deref waiting to
  happen if `selected` itself is `NULL` at that point — not exercised by
  any of the three built-ins, but a real landmine in a hand-written custom
  skin) — `<icon>` in the single-colour case instead builds a degenerate
  "gradient" whose `start == end`, i.e. a flat fill, which is the safer of
  the two fallbacks and the one worth carrying the *spirit* of (a single
  colour is always a valid input, never a crash) without the inconsistency
  between the two code paths.

### `OnMeasureItem`/`OnDrawItem`, exact pixel numbers

For anyone tuning `y-core::menu`'s own constants (`ICON_SIZE`, `TEXT_PAD`,
`ICON_GAP`, `ARROW_WIDTH`, `SEPARATOR_HEIGHT`, `MIN_ITEM_HEIGHT`) against
what PortAL actually shipped, not a guess:

- **Row height**: separator = `10px` flat; normal row =
  `iconSize.cy + 8` (`SM_CYSMICON` + 8, so ~24px at 100% DPI/16px icons —
  `y-core`'s `MIN_ITEM_HEIGHT = 22` is in the same ballpark, not copied
  verbatim).
- **Row width**: `text width + iconSize.cx + 54`, plus (if the item shows a
  shortcut or is a submenu) `shortcut text width + 15` on top of that. The
  `54` bakes in icon-column start (`8px` left inset) + icon-to-text gap +
  right-side padding all in one constant — `y-core` splits the equivalent
  into named `TEXT_PAD`/`ICON_GAP` rather than one opaque number, a
  readability improvement, not a behavior change worth chasing exact parity
  on (Windows doesn't let a `WM_MEASUREITEM` caller get pixel-for-pixel
  layout control back from the rest of the theming pipeline anyway — DPI,
  font metrics and per-monitor scaling all move these numbers around, so
  PortAL's own `54` was never a hard contract even in the original).
- **Icon draw position**: `(rect.left + 8, rect.top + 4)`, drawn at
  `iconSize.cx × iconSize.cy` (the OS's own small-icon metric, not a fixed
  16px — PortAL never hardcodes `16`; `y-core::menu::ICON_SIZE = 16` is a
  `y-pop`-side choice to stop depending on `SM_CXSMICON`, worth being
  explicit is a deliberate divergence, not an oversight, if display-scaling
  fidelity to PortAL is ever revisited).
- **Text rect**: `left = rect.left + iconSize.cx + 24`, `top = rect.top +
  4`, drawn `DT_LEFT | DT_BOTTOM` (bottom-anchored within the row, not
  vertically centred — `y-core`'s `draw_text` uses `DT_VCENTER` instead,
  another acknowledged divergence rather than a bug to fix).
- **Shortcut text**: right-aligned, `t.right -= 10` (an extra `-= 10` again
  if the row is also a submenu arrow, to clear the arrow glyph), drawn in
  `COLOR_GRAYTEXT` (or the skin's `base->textcolor` if the row itself isn't
  hovered) — now built in `y-core::menu` (`paint::draw_normal_item`'s own
  shortcut branch), with one deliberate divergence: `y-pop` draws it in
  [`MenuDesign::disabled_text`] rather than a hardcoded `COLOR_GRAYTEXT`,
  since a fixed system gray goes unreadable dark-on-dark under the `dark`
  skin — PortAL's own choice, not carried over. `text_rect`'s right edge
  already stops before the arrow column when the row is a submenu (see
  `draw_normal_item`'s own layout), so no second `-= 10` was needed the way
  PortAL's shared `tRectText` needed one.
- **Submenu arrow**: `16×16`, right-edge-anchored inside the text rect,
  drawn via a manual `DrawFrameControl(DFC_MENU, DFCS_MENUARROW)` bitmap
  recoloured to the row's own text colour then blitted with `SRCINVERT`/
  `SRCAND`/`SRCINVERT` (a manual colour-substitution blit, since
  `DrawFrameControl` itself only draws in a fixed system colour) — `y-core`
  draws a literal `"▶"` glyph instead (`draw_normal_item`'s own comment
  calls this "deliberately simpler"), which loses PortAL's exact native
  arrow shape but not the recolouring behaviour that mattered (both draw in
  the row's own theme colour, both `ExcludeClipRect` the system arrow away
  for the same documented reason).
- **Title separator** (`MENU_TITLE`): text centred (`(rect.right/2) -
  tSize.cx/2`), `EDGE_ETCHED` rules flanking it on both sides with a `5px`
  outer margin and `2px` gap from the text — `y-core`'s themed titled
  separator (per [[y-pop-plan]]'s roadmap step 5) matches this look, not
  measured against these exact numbers pixel-for-pixel but the same visual
  shape (text-in-the-middle, rule-on-both-sides).
- **Icon-gradient backdrop**: drawn *only* for an unselected row
  (`!ODS_SELECTED`), a rect from the row's left edge to `iconSize.cx + 18`
  wide — i.e. a coloured backdrop specifically behind the icon column,
  independent of the row's own `base`/`selected` background. Nothing in
  `y-core::menu::MenuColors` has an equivalent slot — see the gap table in
  [[y-pop-plan]].

## Config schema takeaways for a clean-slate `y-pop.xml`

Per [[y-snap-porting-decisions]], `y-pop.xml` is a new schema inspired by
`portal.xml`, not a compatible reader for it. Worth carrying over as *ideas*,
not as attribute-for-attribute compatibility:

- Root element with global defaults (`shell-execute`, `show-shortcuts`) that
  every node can override — matches `y-snap.xml`'s own "global default,
  per-node override" pattern already established for other settings.
- Node kinds: group (submenu / tray root), app (launch), separator (with
  optional title text), about, quit — `browse` (folder listing) treated as an
  **app option**, not a fifth node kind, since it still launches things.
- Variables block, expanded once at load (`%win%`, `%mydocs%`, custom
  `<var>`), plus lazy ones resolved at launch time (`%clipboard%`).
- Multiple hotkeys per node — `y-core`'s `HotkeyManager::register` already
  takes one `Hotkey` per call with an explicit `id`, so "up to 10 per node" is
  just "call `register` up to 10 times with 10 ids for one node," no core
  change needed; the `key1`/`mod1`/`over1`-style flat attribute repetition is
  a C++-XML-parsing artifact worth dropping in favor of repeated `<hotkey/>`
  child elements (`y-snap.xml` already prefers child elements over numbered
  attributes for repeated data — follow that, not PortAL's).
