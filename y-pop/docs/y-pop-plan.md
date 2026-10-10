# y-pop — planning notes

`y-pop` is PortAL's rewrite: an application launcher driven by hotkeys and
popup menus. This is a planning document, not an implementation — see
[[portal-analysis]] for what it's based on.

## Decisions

Resolved by the user against the open questions this document originally
ended with:

- **Menu rendering: both backends, user-selectable.** Native owner-draw
  `HMENU` is required; the custom overlay popup is kept as an alternative
  display mode a user can opt into per config. See "The menu" below for what
  that means for the architecture.
- **Type-to-search: in v1.** See the caveat under "The menu" — this is a
  route-B-native feature; on the native `HMENU` backend it degrades to Win32's
  own accelerator-letter jump rather than true filtering.
- **Folder browsing (`browse="true"`): in v1.** The data model accounts for
  dynamically generated children from the start (see "Data model" below).
- **RT/Store app launch: in v1.** `IApplicationActivationManager` support,
  same as PortAL's build-flag-gated feature, but always on rather than
  conditional.
- **Updater: out of scope.** No GitHub-releases version check, matching
  `y-snap`, which has none either.
- **The overlay backend is a family, not one shape.** Confirmed after a
  follow-up: `list` (classic themed popup), `runner` (PowerToys-Run-style
  single search box, and the likely shape of `y-term`'s own eventual UI —
  see "The menu" below), and `radial` (pie/cursor menu) are different enough
  to need a shared `Presenter` trait rather than one implementation bent
  three ways. `list` is the only one this plan treats as v1-required (see
  "Open questions").
- **The config tree is a flat arena, not a recursive owned tree** — confirmed
  after a follow-up that the field-shape sketch in "Data model" was not yet
  the memory layout; see that section for the actual `NodeId`-indexed
  design, O(1) hotkey dispatch, and where string interning plugs in.

## Design principles (carried from `y-snap` / `y-core`)

- One `App` struct, `y_core::app::start(&APP, ...)`, `TrayApp` impl — same
  shape as every other app in the workspace.
- Config is a clean-slate `y-pop.xml`, hand-parsed with `y_core::xml`, not a
  `portal.xml` reader. Repeated data (hotkeys, variables) as child elements,
  matching `y-snap.xml`'s own convention, not PortAL's numbered
  `key`/`key1`/`key2`… attributes.
- No DLL/plugin system, no compile-time feature flags standing in for runtime
  config (PortAL's `#ifdef ICON_MANAGER` / `USE_PATH_STORAGE` become "always
  on, done properly" — memory safety removes most of the reason those were
  opt-in).
- Malformed XML aborts with a line number; a recognized-but-wrong attribute
  (bad hotkey, unknown action) is a non-fatal warning balloon, rest of the
  file still loads — the severity split `y-snap/src/config.rs` already
  follows.
- `unsafe`/Win32 plumbing belongs in `y-core` (or a thin `y-pop`-local wrapper
  module) — `y-pop`'s own code should read as a plain tree walk and `match`.

## Config model (`y-pop.xml`)

Sketch, not a spec — attribute names are placeholders:

```xml
<y-pop shell-execute="false" show-shortcuts="true" theme="auto">
  <variables>
    <var name="docs" value="%mydocs%\projects"/>
  </variables>

  <group name="Dev tools" icon="%portal%\dev.ico">
    <hotkeys><hotkey key="Win+D"/></hotkeys>
    <separator name="Editors"/>
    <app name="VS Code" exe="%programfiles%\...\Code.exe" tooltip="Open the last workspace"/>
    <app name="Terminal" exe="%system%\cmd.exe" path="C:\"/>
    <app name="Recent projects" exe="%docs%" browse="true" filter="*.sln"/>
    <separator/>
    <quit/>
  </group>
</y-pop>
```

Node kinds: `group` (submenu, or tray root), `app` (launch — `browse` is an
`app` attribute, not its own node kind), `separator` (optionally titled),
`about`, `quit`, `settings` (opens `y-pop-config`, next to `y-pop.exe`), `break` (later addition — a menu-column break at exactly
this point among its siblings, `y_core::menu::PopupMenu::add_break`/
`MF_MENUBREAK`; a node of its own rather than PortAL's `break="true"`
attribute on the item the break follows, see `crate::model::NodeKind::Break`
for why). Every node but `break` itself: `name`, `icon`, `hotkeys` (0+
`<hotkey>` children), `default`, `tooltip` (new — see below).

`app` adds: `exe`, `param`, `path`, `shell`, `elevate`, `events`
(`start`/`exit`), `browse`/`filter`/`depth`/`open-folder` — `depth` (later
addition, replacing an earlier `recurse` boolean) caps how many levels of
subfolders get their own submenu, and `open-folder` (`all`/`max-depth`/
`never`) controls whether — and when — a browsed submenu also carries an
extra entry to open the folder it lists directly; see `crate::model::Browse`
for the exact semantics.

Variable expansion: builtin table (`%win%`, `%mydocs%`, …) resolved once at
load from `SHGetKnownFolderPath`, custom `<var>` overlaid on top,
`%clipboard%`/`%clipboards%` resolved lazily at launch. Pure function, no
globals — `fn expand(text: &str, vars: &[Variable]) -> String`.

## Data model (Rust)

The first sketch of this section showed field *shapes* (what data a node
carries), not the actual memory layout — worth being explicit that those are
different questions and the second one matters for this app specifically.
Confirmed: **flat arena, not a recursive owned tree.** A `Vec<Node>` where
each node owns a `Vec<Node>` of children (my first sketch) is one heap
allocation per node plus pointer-chasing for every lookup — precisely the
shape criticized in [[portal-analysis]] for the C++ original's `PortalProg`
linked structure, just with the manual memory management removed rather than
the layout problem. `y_core::xml::Document` already sets the precedent for
the right shape (one flat node arena, children referenced by index, root
first) — the config tree follows the same pattern:

```rust
/// One arena, built once at load and swapped wholesale on reload (see
/// "config lifecycle" in the roadmap) — never mutated node-by-node.
pub struct ConfigTree {
    nodes: Vec<NodeData>,
    /// WM_HOTKEY's wParam is the registered id; ids are allocated
    /// sequentially from a fixed base, so this is a direct array index, not
    /// a hash lookup — O(1) with no hashing on the hot path a keypress runs.
    hotkey_owner: Vec<NodeId>,      // index (id - HOTKEY_ID_BASE) -> node
    strings: StringPool,            // see below; dropped after load if unneeded later
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NodeId(u32);              // index into `nodes`; u32 halves the
                                      // pointer-width of a `usize` index on x64

struct NodeData {
    kind: NodeKind,
    name: Arc<str>,                  // interned — see StringPool below
    icon: Option<IconSource>,
    tooltip: Option<Arc<str>>,
    hotkeys: Vec<Hotkey>,            // almost always 0 or 1; an empty Vec is
                                      // one pointer-sized word and no heap
                                      // allocation, so the common case costs
                                      // nothing extra without reaching for a
                                      // small-vec crate the workspace doesn't
                                      // otherwise depend on
    flags: NodeFlags,                // bitset: default / auto_open — one
                                      // byte, not separate bools (break is
                                      // its own NodeKind, not a flag here —
                                      // see below)
    children: Vec<NodeId>,           // empty for a leaf or an unexpanded Browse
}

pub enum NodeKind {
    Group,
    App { exe: Arc<str>, param: Option<Arc<str>>, path: Option<Arc<str>>, shell: bool, elevate: bool, browse: Option<Browse> },
    Separator,
    About,
    Quit,
    Break,                            // later addition — see the "Node kinds" note above
}

pub struct Browse {
    pub filter: Option<Arc<str>>,    // extension filter, e.g. "*.sln"
    pub depth: u32,                  // levels of subfolders shown as submenus;
                                      // 0 (default) shows no folders at all
    pub open_folder: OpenFolder,     // All | MaxDepth (default) | Never
}
```

Three things doing the "lite" work here, each cheap on its own and additive:

- **One allocation for the tree**, not one per node — `nodes: Vec<NodeData>`
  is a single contiguous buffer; `children: Vec<NodeId>` is small and
  index-only, no ownership, no drop recursion (a deep tree freed recursively
  through owned `Box`/`Vec<Node>` children is also a stack-depth risk on
  drop, not just a speed one).
- **O(1) hotkey dispatch** — `hotkey_owner` is addressed directly by id, no
  `HashMap`, no linear scan over "all hotkey nodes" the way PortAL's
  `g_portal->hotkeys` vector implies. This is the "fast hotkey lookup"
  asked about directly: a keypress's `WM_HOTKEY` arrives with the id already
  in hand, and turning that into a `NodeId` is one array read.
- **String interning avoids duplication**, per the layered plan already in
  "New features" below — `Arc<str>` fields here are what that pool feeds.
  Two nodes with the same `exe` string (or an `icon` reusing the `exe` path
  verbatim, PortAL's own `ico="%win%\explorer.exe"` default) share one
  allocation instead of copying the string per node.

No dependency added for any of this (`Vec`, `Arc`, a hand-rolled bitset —
matches the workspace's existing zero-external-dependency stance: hand-rolled
XML parser over `quick-xml`, `windows`/`windows-core`/`embed-resource` are the
only three dependencies across the whole workspace today).

`browse` (in scope for v1) means a `Browse` node's `children` is not what
`y-pop.xml` literally wrote — it stays empty in the parsed `ConfigTree`, and
gets expanded by listing the folder **at menu-build time**, not at
config-parse time, so a file dropped into a watched folder shows up next
time the menu opens without touching `y-pop.xml` at all. The menu-layout step
(see "The menu" below) does this expansion once per open, producing synthetic
`App` entries in the `MenuLayout` it hands to whichever renderer is active —
the `ConfigTree` arena itself is never mutated by it. Both backends see the
same expanded `MenuLayout`, so folder browsing is not a per-backend concern.

No `nextSameHotkey` linked list, no non-owning `flat`/`hotkeys` index vectors
that can dangle across a reload — PortAL's own
[[portal-analysis|known issue]]. Reload replaces the whole `ConfigTree` value
(new arena built, then swapped in), never patches the old one in place, so
there is no window where a dependent subsystem can observe a half-updated
tree.

## The menu: a native backend, and a family of overlay presenters

> **Status note:** this section (and the `MenuLayout` step it describes)
> still reflects the *original* plan. What actually got built in
> `y-pop/src/menu.rs` (roadmap step 3) skipped the standalone `MenuLayout`
> struct and did the tree-walk directly, with a `Session` carrying the
> per-open lazy-browse and icon state instead. See the roadmap's step 3 for
> what shipped and why revisiting a real `MenuLayout` is worth reconsidering
> once the overlay backend (step 6) needs the same walk a second way. The
> architectural reasoning below (why two backends, why a `Presenter` trait)
> still holds; only the "one shared `MenuLayout` step" mechanism changed.

PortAL's `menu.cpp` is 46 KB of `WM_MEASUREITEM`/`WM_DRAWITEM` owner-draw,
including an abandoned window-subclassing hack just to remove the native
white border. `y-pop` ships both ways to get a themed, iconned popup, and the
user picks per-config (a top-level `<y-pop popup="native">` /
`popup="overlay"`, plausibly overridable per `group` for anyone who wants a
tray root menu one way and a deep submenu tree the other):

**A — Owner-draw native `HMENU`.** Extend `y_core::menu::PopupMenu` with
`MIIM_BITMAP` icons, `WM_MEASUREITEM`/`WM_DRAWITEM` handlers for custom
colors, and `MFT_SEPARATOR` variants for titled separators. Gets native menu
behavior for free (keyboard nav, submenu timing, click-outside dismiss,
accessibility). Async icon *repaint* is not the problem it looks like:
PortAL's `menu.cpp::RefreshTrayMenuItem` (confirmed by reading it) does a
no-op `GetMenuItemInfo`/`SetMenuItemInfoW` re-commit on one item, which makes
Windows re-issue `WM_MEASUREITEM`/`WM_DRAWITEM` for *only that item* — no
flicker, no full menu rebuild — and it works live, because `TrackPopupMenu`'s
nested message loop still dispatches `PostMessageW`s sent to the owner
window, so an icon-loader thread's "node N is ready" message is handled
immediately without the popup closing. Same `WM_APP`-message-to-owner-window
shape `y-snap` already uses (`WM_MIN_HOVER`, `WM_MIN_ACTION`); the only new
bookkeeping is a node id → `(HMENU, position)` lookup, which the shared
`MenuLayout` step provides for free. Tooltips-per-item are the one place this
backend is genuinely harder: owner-draw items have no `HWND` of their own to
hang a tooltip control off, so it needs a manual tooltip window tracked
against `WM_MENUSELECT`/mouse position — real, bounded work, but work.
Type-to-search has no seat here either: `TrackPopupMenu` owns the message
loop, so this backend keeps Win32's native behavior instead — typing jumps to
the next `&`-accelerator match, not a filter.

**B — Custom presenter on `y_core::overlay::Overlay`.** Render on a layered
window using the `Canvas` primitive `y-snap`'s own overlay already draws with
(`fill_rect`, `draw_text`, `round_corners`) — but "the overlay" is not one
shape. A themed nested-list popup, a PowerToys-Run-style single search box
with a filtered flat result list, and a cursor-centered radial/pie menu are
different enough in layout, navigation and how a `Group` is even represented
that forcing them through one presenter would bend all three toward
whichever was designed first. So B is a **presenter trait**, not a single
implementation:

```rust
pub trait Presenter {
    /// Runs its own input loop until a leaf is chosen or the popup is
    /// dismissed. Owns keyboard nav, submenu/group handling, and
    /// click-outside dismiss — none of that comes free the way
    /// `TrackPopupMenu` gives it away for backend A.
    fn show(&mut self, layout: &MenuLayout, at: POINT) -> Option<NodeId>;
}
```

with (at least) three implementations, each free to interpret `Group` /
navigation differently — a `list` presenter drills into a submenu the way a
native menu does, a `runner` presenter flattens the whole tree into one
filterable result set with no drilling at all, a `radial` presenter opens a
nested ring per `Group`:

- **`list`** — the closest analogue to A: a vertical popup, submenus flown
  out to the side, arrow-key navigation, type-to-search filtering the current
  level's visible rows. The default overlay presenter, and the only one this
  plan treats as v1-required (see roadmap) — it's the direct proof that the
  `Presenter` seam works before anything more exotic is built on it.
- **`runner`** — single input box, live-filtered flat list below it, no
  submenu drilling (a `Group` either flattens its children into the
  searchable set or is itself a filterable "go into" entry, config's choice).
  This is not a `y-pop`-only idea: it's functionally what `y-term`'s
  eventual UI is described as needing in `CLAUDE.md` (currently placeholder
  wiring only), which argues for building this presenter, if not directly in
  `y-core`, at least in a shape `y-term` can adopt rather than re-invent —
  worth an explicit decision once `y-term` has an actual design, not assumed
  here.
- **`radial`** — items arranged around the cursor, direction (and optionally
  distance) selects, à la classic pie menus / Opera's mouse gestures.
  Genuinely different math (angle/distance hit-testing instead of row
  hit-testing) and a hard usability ceiling: it degrades badly past
  roughly 8–12 items, so it only makes sense as a presenter choice for
  *specific* small groups, not a blanket replacement for `list`. Treat as the
  most speculative of the three — see open questions.

**Future `list` idea, not started: multi-button rows** (2026-08-28,
user-requested, captured here not built) — the row-of-icon-buttons Firefox/
Chrome/Windows-11-Explorer show (cut/copy/paste, or Explorer's own Cut/Copy/
Rename/Share/Delete strip). Structurally impossible on backend A: a classic
`TrackPopupMenu`/owner-draw menu has exactly one hit-test rectangle per
`MENUITEMINFO`, and Windows itself owns that hit-testing — owner-draw only
changes how a row paints, never how many independently-clickable regions
live inside it. Only viable once `list` owns its own input loop (this
section's whole reason for existing). Candidate uses for a launcher (not a
text editor, so "cut/copy/paste" itself doesn't map, but the same
"toolbar-on-a-row instead of a submenu hop" idea does):
  - A per-`app`-row trailing button strip: Run as admin / Open containing
    folder / Copy path, instead of a submenu for the same three actions.
  - Same for a `browse`-discovered file/folder row.
  - A small utility row pinned atop a `Group`'s own popup: Reload config /
    Edit `y-pop.xml` / Pin this group.
Needs, once actually designed: a new row kind (an item is either a leaf/
submenu row like today, or a strip of `N` icon-only buttons sharing one row
height), sub-rect hit-testing within that row, and left/right arrow-key
navigation between buttons in a focused row (up/down still moves between
*rows*). Left for whenever `list` itself gets designed — not a `MenuDesign`
(backend A theming) concern, and not scoped or estimated yet.

Because A and every B presenter ship from the same data, the menu-building
code in `y-pop` needs one shared step regardless — walk the `ConfigTree`
into a backend-neutral `MenuLayout` (items, icons, tooltips, shortcuts,
submenus, in order, `Browse` already expanded) — and each renderer consumes
it: `native::show(&MenuLayout) -> Option<NodeId>` wraps `PopupMenu` plus the
owner-draw handlers; each `Presenter` impl wraps `Overlay`/`Canvas` plus its
own input loop. Async icon refresh is a per-renderer concern behind this same
seam (native re-commits `MENUITEMINFO`; a presenter just redraws its own
region) but all of them read from one shared icon cache (see gaps below), so
the cache itself never forks per renderer or per presenter. `PopupMenu` in
`y-core` keeps its existing unthemed role for plain context/tray menus
elsewhere in the workspace; only `y-pop`'s *item* popups route through this
seam.

The `Presenter` trait is a new `y-core` module either way (`y_core::popup`,
or `y-pop`-local if it turns out `y-snap`/`y-pop`/`y-coustic` don't actually
share enough of it) — see gaps below.

## `y-core` gaps

Whichever menu route is picked:

- ✅ **Icon loading — done** (roadmap step 4): `y_core::icon`'s `IconCache`/
  `Icon`, exactly as sketched below, plus `Icon::to_bitmap`/`Bitmap` (step 5)
  for the `HICON`→`HBITMAP` conversion a menu's `hbmpItem` actually needs.
  Original sketch, kept for the historical reasoning: nothing in `y-core`
  extracts an `HICON` from a file path today. New `y_core::icon` module:
  `ExtractIconEx` for `.exe`/`.dll`/`.ico` (with the `,index` suffix PortAL's
  `ico="...,6"` syntax uses), `SHGetFileInfo` fallback for everything else
  (documents, folders — one shared icon per extension, per
  [[portal-analysis]]). Returns an RAII `Icon(HICON)` with `Drop` →
  `DestroyIcon`, same shape as `TrayIcon`.
- **Async icon cache**: a small worker-thread pool (not one thread per icon —
  see [[portal-analysis]]'s critique of `TerminateThread`), `mpsc`-based,
  reference-counted by path. On completion, `PostMessageW`s a private
  `WM_APP`-based message back to the owner window carrying which node's icon
  is ready — same pattern `y-snap/src/mouse.rs` (`WM_MIN_HOVER`) and
  `min_overlay.rs` (`WM_MIN_ACTION`) already use for hook-thread → main-thread
  handoff. The PoC mentioned by the user is exactly this piece; finishing it
  is most of "async icons" done.
- **Theme/color parsing**: `y_core::overlay::Color::parse` and `is_dark()`
  already exist and are directly reusable for a `theme="auto"` skin — no gap,
  just reuse.

For the overlay presenters (B), additionally:

- **`Presenter` trait + shared plumbing**: dismiss-on-click-outside,
  multi-monitor placement, the `MenuLayout` → `Presenter` handoff —
  `y_core::overlay::Overlay` gives the layered window and drawing surface but
  none of this; it's new, and shared by every presenter rather than
  duplicated per one.
- **`list` presenter**: keyboard nav (arrow keys, accelerator letters,
  Home/End), type-to-search filtering, submenu open/close with hover delay.
  The v1-required one — see roadmap.
- **`runner` presenter**: single text-input row, live-filtered flat result
  list. Lighter than `list` in navigation terms (no submenu flyout logic) but
  needs the filtering/ranking logic `list`'s type-to-search also wants, so
  worth factoring that piece to be shared rather than written twice.
- **`radial` presenter**: angle/distance hit-testing instead of row
  hit-testing, a materially different input model from the other two. Most
  speculative — see open questions on which presenters are actually in v1.
- **Tooltip overlay**: a second, smaller `Overlay` shown after a hover delay
  next to the hovered row, hidden on any other input. Straightforward for
  `list` (and `runner`, over its result rows) once the presenter owns its own
  input loop instead of relying on `TrackPopupMenu`; doesn't obviously apply
  to `radial`, where "the row" isn't a row.

For the native backend (A), additionally:

- ✅ **`MIIM_BITMAP` plumbing — done** (roadmap step 5): `HICON` → `HBITMAP`
  via `Icon::to_bitmap`, attached with `y_core::menu`'s `set_item_icon`
  (leaf items, addressable by command id) and `set_item_icon_by_position`
  (submenu items, which have none — `AppendMenuW`'s "id" slot for `MF_POPUP`
  is the submenu's own handle instead).
- **Live icon refresh is cheap, not a gap, but still not built**: a node id
  → `(HMENU, position)` table (`set_item_icon_by_position` above is half of
  this already) plus PortAL's own `GetMenuItemInfo`/`SetMenuItemInfoW`
  no-op re-commit trick — see "The menu" above — is all it takes to force
  `WM_MEASUREITEM`/`WM_DRAWITEM` for one item while the popup is still open.
  Confirmed by reading `menu.cpp::RefreshTrayMenuItem`; carry the technique
  over as-is once step 7 (async icons) actually needs it — nothing loads an
  icon asynchronously yet, so there is nothing to refresh.
- Tooltips need a manual `TTM_*` tooltip control tracked against
  `WM_MENUSELECT` — this is fighting the same owner-draw friction PortAL hit,
  and (along with type-to-search and full theming below) is the reason B
  exists as an alternative rather than the only backend.
- Type-to-search does not apply here (see "The menu" above) — no gap to
  close, just the native accelerator-letter behavior `PopupMenu` already
  gets from `AppendMenuW`'s `&` handling.

## New features beyond PortAL

- **Item tooltips** (user-requested): shown on hover-delay next to a menu
  row, config `tooltip="..."` attribute per node. Trivial content-wise; the
  cost is entirely in the popup route decided above.
- **Async icon loading with live refresh** (user-requested, PoC exists):
  covered above — worker pool + redraw message. Matters specifically for
  icons on a network share or removable disk, where `SHGetFileInfo` can block
  for seconds; the menu should open immediately with placeholder icons and
  fill them in as they arrive, never block menu-open on icon load (PortAL's
  own sync path does block, when `iconloaderThread` is off).
- **Path/string memory dedup**: PortAL's `pathStorage.cpp` hand-rolls a
  prefix trie in C++ because `wchar_t*` gave it nothing for free. In Rust,
  most of that win comes from much simpler idiomatic tools — worth doing in
  layers, cheapest first:
  1. **Exact-string interning**: a `HashMap<Box<str>, Arc<str>>` pool built
     once while parsing `y-pop.xml`, so `Node.exe`/`.icon`/`.path` are
     `Arc<str>` and two nodes pointing at the identical string (a shared
     `%programfiles%\...` value copy-pasted across `app` entries, or an
     `exe` reused verbatim as its own `ico`) share one allocation. A few
     lines, no new data structure, handles the common case (repeated *whole*
     strings) that a launcher config full of copy-pasted entries actually
     has.
  2. **Prefix sharing** (the actual case PortAL's trie targets — same
     directory, different filename): only worth it if profiling on a large
     real config shows (1) isn't enough. If so, reach for a crate
     (`radix_trie`, or a `qp-trie`) rather than hand-rolling PortAL's
     `pathSlice` vector — the C++ version's own `remove`/`search` were never
     finished, which is a sign of how fiddly it is to get right.
  3. Whichever layer is used, the interner only needs to matter at the
     `Node` tree level — a `PathBuf`/`String` passed to `CreateProcessW` at
     launch time is a one-off allocation regardless, so this is about the
     resident config tree's footprint, not launch-time cost.
- **Fuzzy filter / type-to-search** (new idea, confirmed in scope for v1):
  on the overlay backend, typing while the menu is open filters visible rows
  instead of only jumping to an accelerator letter — cheap once the popup
  owns its own keyboard input, and the single most-copied feature across
  modern launchers (PowerToys Run, Wox/Flow). Does not apply to the native
  backend — see "The menu" above; a config using `popup="native"` keeps
  Win32's accelerator-letter jump instead, which is worth a line in the
  `y-pop` README once written so it doesn't read as a missing feature.

## Phased roadmap

This section is now the running source of truth for progress, not just the
original plan — a step marked ✅ is done and tested; 🟡 is partially done,
with what's left spelled out; unmarked is still ahead. Mirrors `y-snap`'s own
history and the `portal-rust` attempt's phase order, adjusted for the
dual-backend menu decision. Building the native backend first was
deliberate: it's the required one — building the overlay backend first would
risk shaping any shared seam around only what the overlay needs.

1. ✅ **Data model + XML parser** — `y-pop/src/model.rs` (the flat
   `ConfigTree` arena), `config.rs` (the parser: pure Rust, no Win32, no
   filesystem or registry access in its own test suite), `variables.rs`, the
   default `rsc/y-pop.xml`. Shipped different from the original sketch in two
   ways: the `<hotkeys>` wrapper element was dropped in favour of one or more
   bare `<hotkey>` children directly on a node (less nesting for the common
   single-binding case); the low-level-hook-override attribute is spelled
   `override="true"` in the XML (`HotkeyBinding::can_override` in Rust,
   `override` itself being a reserved word) rather than `y-snap.xml`'s
   `steal-from-shell` — a deliberate rename for a plainer config file, not an
   oversight; `y-snap.xml` keeps its own name unchanged.
2. ✅ **Core Win32 shell** — tray icon(s), hotkey registration (including
   `can_override` → `Fallback::LowLevelHook`), launch
   (`ShellExecuteExW`/`CreateProcessW`/elevation/RT-app activation via
   `IApplicationActivationManager`, with `%clipboard%`/`%clipboards%`
   resolved lazily right here, not at parse time), `start`/`exit` events.
   Needed two new `y_core` modules the original gap list didn't anticipate:
   `known_folders` (lazy, cached `SHGetKnownFolderPath` resolution — generic,
   not `y-pop`-specific, so it lives in `core` rather than as a `y-pop`-local
   helper) and `log` (a file logger — confirmed by testing, not assumed,
   that `eprintln!`/`println!` are not visible at all from a
   `windows_subsystem = "windows"` binary run under a Git Bash/MinTTY
   terminal via `cargo run`, unlike a native `conhost.exe` console; every
   `y-pop` diagnostic goes through `y_core::log::line` now, not `eprintln!`).
3. ✅ **Folder browsing — shipped, but not through a `MenuLayout`.** The
   `MenuLayout` step described under "The menu" above didn't get built as
   its own struct; what shipped instead is `y-pop/src/menu.rs`'s `Session` +
   `on_init_menu_popup`, populating a `browse` submenu lazily from
   `WM_INITMENUPOPUP` (PortAL's own trick — confirmed by reading
   `menu.cpp::OnInitMenuPopup`/`HandleSubDirectory`, not assumed), with two
   separate command-id spaces (tree nodes vs. files discovered at browse
   time) rather than one unified layout structure. Reasonable shortcut for a
   native-only popup; worth revisiting once step 6 needs to walk the same
   tree a second way for the overlay backend — see the status note under
   "The menu" above.
4. ✅ **Icon loading, sync, with a cache** — `y_core::icon`: `IconCache`/
   `Icon` (`ExtractIconExW` for `.exe`/`.dll`/`.ico`/`.lnk`, `,index` support
   including PortAL's/Windows' own negative-index-means-resource-id
   convention passed straight through; `SHGetFileInfoW` fallback for
   everything else, cached by extension; one shared entry for every
   directory). Two real Win32 quirks the test suite found, not assumed:
   `SHGFI_USEFILEATTRIBUTES` is reliable for a directory lookup but not a
   regular-file one (which now requires the first file of a given extension
   to actually exist — matches PortAL's own `_loadIcon`); the shell/COM
   machinery behind icon extraction isn't safe to call concurrently from
   multiple uninitialized threads, which only matters for `cargo test`'s
   default parallel runner (production is single-threaded) — those tests
   serialize with a mutex.
5. ✅ **Native backend (A) — icons and theming both done.** Icons:
   `Icon::to_bitmap` (`HICON`→`HBITMAP` via `DrawIconEx` into a zeroed
   `CreateDIBSection`, its bits premultiplied in place once so both a native
   `hbmpItem` and a themed `AlphaBlend` read the same buffer correctly) and
   `y_core::menu`'s `set_item_icon`/`set_item_icon_by_position`/`item_count`
   — position-addressing turned out to be necessary, not optional, because a
   submenu item has no command id of its own (`AppendMenuW`'s "id" slot for
   `MF_POPUP` is the submenu's own handle instead) — are wired into every
   place `y-pop` adds an item: leaf items, a `Group`'s own icon, and both
   files and subfolders `browse` discovers. `Bitmap` (new `y_core::icon`
   RAII type) is owned by `Session` for exactly as long as the popup stays
   open, since Windows does not free a custom `hbmpItem` on its own.
   Theming: `y_core::menu::PopupMenu::themed`/`new_maybe_themed` build every
   item `MFT_OWNERDRAW` instead of plain `MF_STRING`, with a boxed
   `ItemData` (text, kind, the `MenuColors` palette) behind `dwItemData` —
   necessary, not a choice, because `dwTypeData` stops being text at all
   once an item is owner-drawn (`AppendMenuW`'s own documented behaviour).
   `PopupMenu::drop` walks the whole live menu tree reclaiming every
   `ItemData` right before `DestroyMenu`, rather than tracking Rust-side
   ownership through `add_submenu`'s handle move — confirmed necessary by a
   nested-submenu drop test, not assumed. `set_item_icon`/
   `set_item_icon_by_position` needed no signature change at all: they probe
   the item's own `fType` for `MFT_OWNERDRAW` and, if set, write the bitmap
   into `ItemData` instead of `hbmpItem` (never auto-painted once
   owner-drawn), so every existing `y-pop` call site kept working unchanged.
   `handle_measure_item`/`handle_draw_item` (new `y_core::menu` free
   functions, forwarded from `y-pop/src/main.rs`'s `WM_MEASUREITEM`/
   `WM_DRAWITEM`) paint background/text/hover/disabled colours, a themed
   icon via `AlphaBlend`, a hover border via `FrameRect`, and a titled or
   untitled separator drawn with an `EDGE_ETCHED` groove — PortAL's own
   `MENU_SEP`/`MENU_TITLE` look (`menu.cpp::OnDrawItem`, confirmed by
   reading it, and by the user asking for it back explicitly), not a
   flat-coloured rule invented for this port, replacing step 3's
   disabled-row placeholder for a titled one. A submenu item draws its own
   arrow glyph ("▶", deliberately simpler than PortAL's own manually-drawn
   and recoloured triangle) *and* then calls `ExcludeClipRect` over that
   same rectangle — Windows still draws its own arrow for any `MF_POPUP`
   item after `WM_DRAWITEM` returns, even one that's `MFT_OWNERDRAW`, in a
   fixed system colour, and that native arrow is what needs suppressing,
   not duplicating: an earlier version of this module drew its own arrow
   with no clip exclusion and produced a visible double arrow (caught by
   the user), and a version after that removed the custom arrow entirely
   on the mistaken theory that Windows never draws one for an owner-drawn
   item — which "fixed" the double arrow but left only Windows' own
   fixed-colour one, invisible against a dark theme until a row is hovered
   into a lighter highlight (also caught by the user, against a real
   screenshot). PortAL's own `menu.cpp::OnDrawItem` does the same
   draw-then-exclude, with a comment reading "in order to not redraw the
   system arrow" that settles what the first version got wrong — confirmed
   by reading it, not assumed a second time. Native
   `popup="native"` (no theme) still falls back to a disabled row for a
   titled separator, since plain `HMENU` has no titled-separator rendering
   of its own to draw one for real. `Theme` (`native`/`light`/`dark`/`auto`,
   parsed in `y-pop/src/config.rs`, `Theme::Auto` resolved against
   `y_core::theme::is_dark` fresh each time a popup opens, not cached at
   load) is `y-pop`'s own config concern; `MenuColors` only defines the
   shape `y_core::menu` draws with — the palette values themselves
   (`LIGHT`/`DARK` in `y-pop/src/menu.rs`) are `y-pop`'s choice, the same
   split `y-snap`'s `Palette::LIGHT`/`DARK` already draws between itself and
   `y-core`. On top of `Theme`, six `*-color` attributes
   (`back-color`/`text-color`/`hover-back-color`/`hover-text-color`/
   `disabled-text-color`/`border-color`, parsed into a `ThemeOverrides` —
   `Option<Color>` per slot, `None` meaning "keep the base theme's value")
   let a config override individual colours on top of whichever `theme`
   resolves to, PortAL's own answer to "I like dark but want my own accent"
   — the same `*-color`-attribute shape (and the same "theme applied first,
   an explicit colour still wins") `y-snap/src/config.rs`'s own overlay
   palette override loop already uses. Setting a colour without also
   setting `theme=` resolves as `theme="auto"` had been written too (see
   `resolve_colors`'s own doc comment) rather than silently doing nothing,
   since a config that names a colour clearly wants *something* themed to
   paint it onto. Two more fixes, both found by the user against a real
   PortAL screenshot, not assumed: a `Group` (and a `browse` folder, already
   covered) now defaults to the generic folder icon the same way PortAL's
   own stock menu shows one for every submenu, not just an icon the config
   explicitly gives it (`resolve_icon`); and every themed row now reserves a
   fixed icon-column width (`y_core::menu::ICON_SIZE`, exported so `y-pop`
   never keeps a second copy of the same number) whether or not that
   particular item has an icon, so a mix of icon and no-icon rows still
   lines its text up in one column — matching what a native menu already
   does for free by always reserving its own icon/check gutter, which the
   owner-draw path previously didn't. **Not done**: the live-icon-refresh
   re-commit trick (nothing loads an icon asynchronously yet — see step 7 —
   so there is nothing yet to refresh; the position-addressing this needs
   already exists from this step), a themed `add_check_item` (unused by any
   caller yet, so left native-only rather than built ahead of a need).
   **Phase 5 gap analysis** (against PortAL's full `PortalMenuDesign`, now
   documented exactly in [[portal-analysis]]'s "theme data model" section —
   this is the delta, not a re-summary):

   | PortAL feature | `y-pop` today | Verdict |
   |---|---|---|
   | Per-row border `size`/`round`/`color` | One 1px `FrameRect` hover border, no size/round config | **Keep simplified.** Rounded owner-draw regions need the same `RoundRect`+`SelectClipPath` dance PortAL's own comment shows is fiddly (and its `color` attribute is provably dead code — see [[portal-analysis]]) for a look Windows 11's own flyouts don't use either (square hover rects). Revisit only if a user asks for rounded corners specifically. |
   | Background gradient (`<background color="a:b" direction="h\|v">`) | Flat `back`/`hover_back` fill only | **Open — see below.** The one PortAL feature here that's purely cosmetic *and* cheap (`GradientFill` is already how `y-snap`'s own overlay never needed to reach, but `y_core::overlay::Canvas` has no equivalent primitive either — this would be new, not reused). |
   | Icon-column gradient backdrop (unselected rows only) | **Built.** `y_core::menu::MenuDesign::icon_backdrop` (`Option<Gradient>`), painted by `paint::paint_icon_backdrop` before the icon/text — idle rows only, matching PortAL's own `!ODS_SELECTED` gate. `dark` (`design3`) sets it to the identical gradient its own hover fill uses; `default`/`flat` don't set it, unchanged from before. Config-facing as `<skin><icon-gradient start="" end="" direction=""/></skin>`, `<background-gradient>`'s own shape, not PortAL's `<icon color="a:b">` (its single-colour fallback is provably buggy — see `y-pop/src/theme.rs::ThemeOverrides::icon_gradient`'s own doc comment). | Reversed from "Drop" once the user asked for it directly. |
   | `edge` (`PORTAL_ST_SEL/UNSEL/ALL/NONE` vertical rule between icon and text) | **Built.** `y_core::menu::MenuDesign::edge` (`Option<EdgeRule>`), drawn by `paint::draw_normal_item` at the icon/text boundary via the same `DrawEdge`/`EDGE_ETCHED` groove the separator rule already uses. `default` → `Idle`, `dark` → `Always`, `flat` → `None` — the same three skins' actual runtime behaviour PortAL ships, but *not* its `_SEL`/`_UNSEL` bit names: `menu.cpp::OnDrawItem`'s `itemSelected` local is assigned `!(itemState & ODS_SELECTED)`, inverted from what it's named, so `_SEL` literally gates on idle rows and `_UNSEL` on selected ones. `y_core::menu::EdgeRule`'s `Idle`/`Hover`/`Always` name the actual row state instead of carrying that inversion into every `y-pop.xml` a user writes; see its own doc comment. Config-facing as `<skin edge="idle"\|"hover"\|"always">`. | Reversed from "Drop, unless requested" once the user asked. |
   | `base`/`selected` half-set → fall back to system colours | Every `MenuColors` slot always concrete | **Keep as-is.** `y-pop`'s `theme="auto"` already tracks the live OS light/dark setting for the *whole* palette (`y_core::theme::is_dark`), which is the better analogue to "use system colours" than PortAL's per-slot `NULL` fallback — no gap to close. |
   | Shortcut-text column (right-aligned hint, `showshortcut`/`showsc`) | **Built.** `y_core::menu::PopupMenu::add_item_with_shortcut`/`add_submenu_with_shortcut` draw it themed (`paint::draw_normal_item`, in `MenuDesign::disabled_text` rather than PortAL's hardcoded `COLOR_GRAYTEXT` — see `portal-analysis`'s pixel-numbers section for that divergence) and compose it natively (`"{text}\t{shortcut}"`, Win32's own free accelerator-column rendering); `y-pop/src/menu.rs::shortcut_text` resolves `show-shortcuts` inheritance and formats the hotkey via `y_core::hotkey::Hotkey`'s own `Display`. | Was the real gap this row used to describe — resolved, not dropped. |
   | Custom `<skin>`-equivalent per-`y-pop.xml` design (not just `theme=`/`*-color`) | `theme` + 6 flat `*-color` overrides, no gradient/border/edge attributes | **Matches decision already made.** [[portal-analysis]]'s config-schema-takeaways section already chose "global defaults + per-node override," not a `<skin>` sub-tree; the 6-slot `*-color` shape is that decision applied to colour specifically. No new gap — a border/gradient *value* to override doesn't exist yet only because the border/gradient *feature* doesn't (see rows above), not because the override mechanism is missing. |

   **Recommendation for what's actually next**, in priority order: (1)
   ~~shortcut-text rendering~~ — done (see the table row above); (2)
   background gradient support *if* a user asks for
   it (cheap once asked for, not worth building speculatively — GDI
   `GradientFill` on an owner-draw row is a small, self-contained addition
   to `handle_draw_item`, same shape as the existing solid-fill path it
   would sit next to), (3) border rounding — lowest priority, most GDI
   complexity for the least-requested visual effect, only reconsider given
   an explicit ask.

   **Superseded 2026-08-28** — the user explicitly asked for round border,
   gradients and margins, so the "revisit only if asked"/"drop" verdicts
   above no longer hold for those three rows. See "Theme system v2" below
   for the design this reopens; the table above stays as the historical
   record of what was deliberately deferred and why, not rewritten.

## Theme system v2 — `MenuDesign` (✅ built 2026-08-28, corrected three times after)

Implemented as designed below, then corrected three times more after the
user caught real problems in what actually shipped — every correction kept
here as its own record, not folded silently into the original text:

1. **Missing `MIM_BACKGROUND`.** The design below claimed `OnInitMenuPopup`
   already painted the whole popup's background — assumed by analogy with
   PortAL, never checked against this codebase, and wrong: nothing in
   `y-pop` ever called `SetMenuInfo`. Once rows stopped filling their full
   `rcItem` (to make room for the new inset), the gaps showed the OS's raw
   default menu background instead of the theme. Fixed by
   `core/src/menu.rs`'s new `set_menu_background`, called from both places a
   themed `HMENU` is ever created, its brush reclaimed by the same
   recursive walk that already reclaims `ItemData`.
2. **`margin`/`padding` swapped, and `LIGHT`/`DARK` were not PortAL's
   skins.** Two separate mistakes the user called out together: (a) the
   single inset field below, named `margin`, actually behaved like CSS
   `margin` (gap *between* rows) — the user's own vocabulary for this
   codebase puts it the other way, `margin` is space *within* an element
   and `padding` is space *between* elements, so the field needed
   splitting into two (`padding` = between rows, `margin` = between a row's
   border and its content) rather than renamed in place; (b) the shipped
   `LIGHT`/`DARK` consts were `y-pop`-invented Windows-11-styled values,
   not PortAL's actual three skins (`default`/`flat`/`dark`, already
   documented in [[portal-analysis]]'s own table) despite that data having
   been available the whole time. Both fixed together: `Theme` is now
   `Native`/`Default`/`Flat`/`Dark`/`Auto`, `y-pop/src/menu.rs`'s
   `DEFAULT`/`FLAT`/`DARK` consts reproduce PortAL's documented
   colours/border/gradient values exactly (system-colour slots stood in
   with a concrete black-on-white/accent-blue pair, since `MenuDesign` has
   no partial-system-fallback mode — see [[portal-analysis]]'s own note on
   why that's the better analogue anyway), and `MenuDesign` carries both
   `padding: u32` and `margin: u32` as independent fields. The
   "`Per-row margin`" section below is kept for its still-correct mechanics
   (the inset-and-let-the-popup-background-show-through trick) but its
   naming and single-field shape are superseded by the split described
   here — read `padding` wherever it says the old, single `margin`.
3. **Yellow where it should be blue — a `COLORREF` byte-order bug, in the
   *analysis*, not just the port.** Fixing (2) meant reading
   `menu.cpp`'s literal `default`/`flat`/`dark` skin values for the first
   time and copying them in as `Color::rgb(0xf9, 0xd9, 0x96)` and similar —
   reading each hex literal's digits left-to-right as R,G,B. Caught by the
   user testing a real build: PortAL's own colours are blue-toned, not
   yellow/tan. The literals in `menu.cpp` (`0xf9d996,	// border_color` and
   others) are raw C integers assigned straight into a `COLORREF` field, not
   `#RRGGBB` hex strings — `COLORREF` is little-endian `0x00BBGGRR`, so
   `0xf9d996`'s true channels are B=`0xf9`, G=`0xd9`, R=`0x96`, a light blue.
   [[portal-analysis]]'s own "theme data model" table had the identical
   misreading (now corrected there too) despite correctly explaining this
   *exact* BGR/RGB gotcha two sections later, for `xmlconfig.cpp`'s
   `getColor` bug specifically — the knowledge was in the same document and
   didn't get applied to the table sitting above it. `y-pop/src/menu.rs`'s
   `DEFAULT`/`FLAT`/`DARK` now use the correctly-decoded channels
   (`BLUE_ACCENT`/`FLAT_BORDER`/`FLAT_SELECTED_BACK` consts, each with the
   exact literal and decode in its own doc comment), and
   `the_three_built_in_skins_match_portals_own_documented_values` is updated
   to match.

Every code example, struct field name, and skin value below reflects the
*first* build, before any of the three corrections — read them for the
mechanics (inset math, `RoundRect`+`SelectClipPath`, `GradientFill`), not
for the exact field name or the skin numbers, all superseded above.
`core/src/menu.rs` and `y-pop/src/menu.rs` are the source of truth for
current field names (`padding`, `margin`, `DEFAULT`/`FLAT`/`DARK`) and
values.

Reopens the deferred rows of step 5's gap table on explicit request: round
per-row borders, a background gradient, and a new-vs-PortAL idea, per-row
margin. Read together with [[portal-analysis]]'s "theme data model" and
"`OnMeasureItem`/`OnDrawItem`, exact pixel numbers" sections, which this
builds on rather than re-derives — the GDI mechanics below (`RoundRect` +
`SelectClipPath`, `GradientFill`) are PortAL's own (`menu.cpp::OnDrawItem`,
confirmed by reading it a second time for this design), reused because
they're the only way Win32 owner-draw actually offers to get either effect,
not reinvented. `core/src/theme.rs` (light/dark registry detection) is
unrelated and untouched — this section is entirely about
`core/src/menu.rs`'s owner-draw painting.

### Why `MenuColors` (6 flat colours) isn't the right shape any more

`MenuColors` was sized for exactly what `y-pop` themed first: solid fills,
one hover state, one border colour used only as a 1px `FrameRect`. Round
corners and a gradient both need geometry (`round`, `size`, a fill kind)
that a flat colour struct has nowhere to put. Rather than bolt fields onto
`MenuColors` piecemeal, replace it with a small struct-of-structs that
mirrors PortAL's own `PortalMenuDesign` grouping (border / fill / colour
pairs) but keeps every slot concrete — no `Option<ColorPair>`
system-colour-fallback mode, per the gap table's already-settled verdict on
that row, which still holds:

```rust
/// Replaces `MenuColors`. Every owner-drawn `PopupMenu`/`append_*` call site
/// that took `MenuColors` takes this instead — same "one palette, set once
/// at popup-build time" shape, just richer.
pub struct MenuDesign {
    pub base: ColorPair,          // idle row: was `back`/`text`
    pub selected: ColorPair,      // hovered/selected row: was `hover_back`/`hover_text`
    pub disabled_text: Color,     // unchanged
    pub border: Border,
    pub fill: Fill,               // idle rows always paint `base.back` flat;
                                   // `fill` only governs the *selected* row's
                                   // paint, matching PortAL (see below)
    pub margin: u16,              // new — see "Per-row margin"
}

pub struct ColorPair { pub text: Color, pub back: Color }

pub struct Border {
    pub size: u16,   // 0 = none, same meaning as PortAL's `border_size`
    pub round: u16,  // 0 = square corners, same meaning as `border_round`
    pub color: Color,
}

pub enum Fill {
    Solid,                 // selected.back, flat — today's behaviour
    Gradient(Gradient),
}

pub struct Gradient {
    pub direction: GradientDirection, // Horizontal | Vertical
    pub start: Color,
    pub end: Color,
}
```

`MenuDesign::default()` (the neutral fallback for any config path that
doesn't resolve to `LIGHT`/`DARK` at all — there isn't one today, but the
zero-config case should never surprise) keeps `border.size = 0`,
`border.round = 0`, `fill = Fill::Solid`, `margin = 0`: today's exact flush,
square, solid-fill look.

`y-pop`'s own `LIGHT`/`DARK` consts do **not** stay at that neutral default
— their existing doc comment (`y-pop/src/menu.rs`, both consts) already
states the design target: *"Values close to Windows 11's own flyouts, the
same reference point `y-snap`'s own `Palette::LIGHT` uses."* A Windows 11
flyout's hover state is exactly the rounded, inset, filled pill this
`Border`/`margin` work makes possible for the first time — `MenuColors` had
no field to express it, which is a capability gap, not a considered choice
for square corners. Closing that gap means `LIGHT`/`DARK` should actually
adopt it: `border.size = 0` (Windows 11's own flyout highlight has no
stroke, unlike the accent-coloured 1px `FrameRect` outline `y-pop` draws
today — dropping the stroke in favour of a filled pill is the more faithful
match to the stated reference), `border.round = 6`, `margin = 3`,
`fill = Fill::Solid` still (a gradient hover is not part of "look like
Windows 11 flyouts" — see the gradient-scope decision below; gradients stay
opt-in via config, not baked into the shipped themes). This is a visible
look change from today's flat/square hover, called out here explicitly as
one, not an accidental side effect of the `MenuColors`→`MenuDesign` rename.

**Scoped to the selected row, like PortAL — decided, not open.** Reading
`OnDrawItem` again confirms the border, rounding and gradient all live
inside the `ODS_SELECTED` branch — an idle row is always a flat `base.back`
fill, full `rcItem`, no rounding. `MenuDesign` keeps that scoping rather
than adding a "when" bitmask (`PORTAL_ST_SEL/UNSEL/ALL`) up front: PortAL's
own `edge` bitmask is exactly that generality, and the gap table (at the
time this was written) called it "cosmetic, drop unless requested" — no
request for an always-rounded/always-gradient idle row exists, so
`Border`/`Fill` apply to selected only, v1. (`edge` itself was later built,
as its own dedicated `MenuDesign::edge: Option<EdgeRule>` field rather than
retrofitting `Border`/`Fill` with a `when` — see the gap table's own row for
it. This paragraph's `Border`/`Fill` scoping decision stands unchanged.)
Widening it later (`Border`/`Fill` gaining a `when: RowState` field) is
additive, not a rewrite, if ever asked for.

### Per-row margin (new — not a PortAL feature)

PortAL has no real per-row margin; `border_size` doubles as the only inset
(the border is painted, then the fill starts `border_size` in from it —
there is no gap that isn't itself part of the border). What "margin" means
here instead, and the reason it's worth building alongside rounding rather
than as a separate ask: a native `HMENU` popup has no controllable outer
frame — Windows owns the drop-shadow/edge chrome around the whole popup, so
"margin" can't mean "space around the popup." It *can* mean space around
each *row*, which combined with rounding is exactly what gets a Windows-11-
flyout-style look (each hovered row reads as a distinct rounded pill,
floating with a gap on every side, rather than a rounded rect butted flush
against its neighbours):

- [`handle_measure_item`] adds `2 * margin` to both `itemWidth` and
  `itemHeight` — the row's hit-test/layout rect grows to include the gap.
- [`handle_draw_item`] insets `rcItem` by `margin` on all four sides to get
  the *paint rect* before doing anything else (border, fill, gradient, then
  text/icon/arrow, all relative to the inset rect, not `rcItem` directly).
  The `margin`-wide strip left outside the paint rect is never painted by
  this row at all — it needs to show through a `MIM_BACKGROUND` brush
  covering the *whole* popup, PortAL's own `OnInitMenuPopup` mechanism.
  **Corrected after implementation** (this bullet originally, wrongly,
  called that mechanism already existing in `y-pop` and "no new plumbing
  needed" — assumed by analogy with PortAL rather than checked; it did not
  exist anywhere in this codebase, and the first build of this feature
  visibly proved it, showing the OS's raw default menu background through
  every margin gap instead of the theme). `core/src/menu.rs`'s new
  `set_menu_background` is the actual fix: called from both places a themed
  `HMENU` is ever created (`PopupMenu::themed`, and `append_submenu`'s
  themed branch — the lazy-`browse`-folder path), and its brush reclaimed by
  `free_owner_draw_data`'s existing recursive live-tree walk, the same one
  that already reclaims every item's `ItemData`.
- Applies uniformly (idle and selected rows both inset, so the grid of rows
  stays evenly spaced) even though `Border`/`Fill` painting is
  selected-only, per above — an idle row's flat `base.back` fill still
  respects the inset rect, just with `round = 0` effectively (no separate
  flag needed: an idle row simply never asks for the rounded-clip path).
- One `u16`, symmetric on all sides, v1 — not `(top, right, bottom, left)`.
  Simpler config surface, and no concrete request yet for asymmetric
  spacing; widening to four fields later is additive if one shows up.
- Separators are unaffected (`ItemKind::Separator` keeps painting its etched
  rule across the *unmargined* row width — a titled/untitled rule reading as
  a full-width divider is the whole point of it, margin would just shrink
  that visually with no benefit).

### Drawing order in `handle_draw_item`, concretely

For a `Normal` row, selected, with `border.round > 0` and `fill =
Gradient(_)` (the maximal case — every simpler config is a subset of these
steps):

1. Paint rect = `rcItem` inset by `margin`.
2. `BeginPath` / `RoundRect(paint_rect, round, round)` / `EndPath` /
   `SelectClipPath(RGN_COPY)` — clips all subsequent painting in this row to
   the rounded rect. Exactly PortAL's own two-pass trick (confirmed by
   reading `OnDrawItem` again): draw the *unrounded* border-colour fill
   first, clip to the *inset-by-border_size* rounded rect second, so the
   border shows as a ring rather than needing a stroke API GDI doesn't have
   for a variable-radius rect.
3. If `border.size > 0`: flat-fill `paint_rect` with `border.color` (this is
   what becomes the visible ring once step 4 clips a smaller rect on top).
4. Re-clip to `paint_rect` inset by `border.size` (same `RoundRect`+
   `SelectClipPath` dance, smaller radius derived the same way PortAL does:
   same `round` value, smaller rect — not re-parametrized).
5. Fill that inner rect: `GradientFill` with two `TRIVERTEX`s (top-left =
   `start`, bottom-right = `end`, direction selects
   `GRADIENT_FILL_RECT_H`/`_V`) for `Fill::Gradient`, or a flat
   `CreateSolidBrush(selected.back)` fill for `Fill::Solid` — same branch
   point `y-pop`'s existing solid-fill code already has, gradient is a
   sibling arm, not a rewrite of it.
6. Restore the clip region to the full row (`SelectClipRgn` back to what
   `GetClipBox` captured before step 2 — PortAL's own `fullRect` backup,
   same reason: icon/text/arrow drawing after this point must not stay
   clipped to the rounded rect, or square-cornered text/icon content would
   itself get clipped at the corners).
7. Icon, text, submenu arrow — unchanged from today, positioned relative to
   `paint_rect` (so margin shifts them in, same as it already shifts
   `text_rect` in for `TEXT_PAD`) rather than `rcItem`.

An idle row, or `border.round == 0`, skips straight from step 1 to a plain
`FillRect(paint_rect, base.back)` — today's code path, just against the
inset rect instead of `rcItem`.

### XML schema (`y-pop.xml`)

**Superseded 2026-08-28** — the flat `*-color`/`border-*`/
`background-gradient-*` attribute soup below is exactly the "complicated
names" the user asked to avoid; a dedicated `<skin>` child element (the
`<variables>`/`<hotkey>` convention of "give a whole config domain its own
element instead of prefixing every attribute with its name") replaces it.
This also reopens the gap table's row 5 verdict ("matches decision already
made... not a `<skin>` sub-tree") — that verdict is now overturned by
explicit ask, kept below only as the record of what was tried first and why
it stopped working once the attribute count grew past six colours.

One dedicated element, direct child of `<y-pop>`, holding every theme
parameter — mirroring PortAL's own `<skin>` element name and its
`<border>`/`<background>` child nesting (see [[portal-analysis]]'s "Skin /
owner-draw menu" section) more closely than the flat-attribute attempt did,
while keeping the fixed-slot colours (not PortAL's colour-pair strings) and
the two-attribute gradient (not PortAL's single `"start:end"` pair) that the
flat design had already chosen for good reason — reused here, not
re-decided:

```xml
<y-pop theme="dark">
  <skin back-color="#1e1e1e" text-color="#ffffff"
        hover-back-color="#0078d7" hover-text-color="#ffffff"
        disabled-text-color="#787878"
        margin="3">
    <border size="1" round="8" color="#3a3a3a"/>
    <background-gradient start="#2b2b2b" end="#1a1a1a" direction="v"/>
  </skin>
  ...
</y-pop>
```

- `<y-pop theme="...">` keeps its existing meaning unchanged: which built-in
  palette (`native`/`light`/`dark`/`auto`) `<skin>`'s overrides start from.
  That's a "which base do I start from" choice, not a theme *parameter*
  itself, so it stays a root attribute rather than moving under `<skin>`.
- `<skin>` is optional (its absence today already means "no overrides,
  whatever `theme=` resolved to" — unchanged), and every one of its
  attributes/children is independently optional, same "`None` keeps the
  base theme's value" rule the flat design already had. A second `<skin>`
  warns and is ignored, same pattern `read_variables` already uses for a
  second `<variables>` (`root.children_named("variables").skip(1)`).
- **`back-color`/`text-color`/`hover-back-color`/`hover-text-color`/
  `disabled-text-color`**: the five colour slots that already exist as root
  attributes today (`ThemeOverrides`, already shipped and tested — see
  `config.rs`'s `theme_color_overrides_are_read` test), moved onto `<skin>`
  verbatim, same names, same `Color::parse` semantics. This is the one
  actually-breaking rename in this redesign: existing `y-pop.xml` files
  using `<y-pop back-color="...">` would need updating to
  `<y-pop><skin back-color="..."/></y-pop>` — worth a line in whatever
  changelog/README section documents this once built, not silent.
- **`margin`**: new `u16` attribute on `<skin>` itself (not nested — it's
  not part of `<border>`, it governs the row inset every fill respects,
  border included). Same "warn and ignore" severity as an unparseable
  colour for a non-numeric value.
- **`<border size="" round="" color=""/>`**: a child of `<skin>`, all three
  attributes optional and independently defaulting (`size`/`round` → `0`,
  `color` → `border-color`'s old default if that slot still exists, or the
  theme's own accent otherwise). Matches PortAL's own `<border>` child
  element name and attribute names exactly — the one place this design
  copies PortAL's XML shape verbatim, since there was nothing wrong with it
  (unlike PortAL's colour-pair strings elsewhere, `border`'s three
  independent numeric/colour attributes never had the parsing bugs
  [[portal-analysis]] documents for `base`/`selected`/the gradients).
- **`<background-gradient start="" end="" direction=""/>`**: a child of
  `<skin>`, not PortAL's bare `<background>` — named explicitly because
  (unlike PortAL's version, which is one of three things depending on
  context: a gradient, a single-colour flat override, or a crash on bad
  input per [[portal-analysis]]) this one is always exactly a gradient,
  nothing else, so the element name says so rather than reusing an
  ambiguous `<background>` that would need its own doc comment every time
  to explain what shape it actually takes. `start`/`end`: two `Color`
  attributes, not PortAL's single pair-string, for the same dead-code/
  channel-swap reasons already settled in the flat design (kept, not
  re-litigated). `direction`: `"h"`/`"v"`, default `"h"` if the element is
  present but `direction` isn't, matching PortAL's own default. Setting only
  one of `start`/`end` is a config error (warn, gradient not applied) — same
  "half-set is a mistake" stance as before.
- No per-node override (`<skin>` lives directly under `<y-pop>` only, not
  under `<group>`) — same scope limit as the flat design had, for the same
  reason (a per-node `MenuDesign` is a bigger architectural change, not
  assumed here).

### No longer dropped: the icon-column gradient and `edge`

Both were "drop, unless requested" at the time this section was written —
the user later asked for them directly (comparing `y-pop` against PortAL's
own theming feature set), so both are now built: `MenuDesign::icon_backdrop`
and `MenuDesign::edge`, config-facing as `<skin><icon-gradient .../></skin>`
and `<skin edge="idle"|"hover"|"always">` — see the step-5 gap table's own
rows for the detail, including why `edge`'s config values rename PortAL's
own `_sel`/`_unsel` rather than reusing them (its `itemSelected` local is
inverted from its name, so those bit names don't mean what they read).

### Migration shape

- `core/src/menu.rs`: `MenuColors` → `MenuDesign` (rename, restructure,
  same call sites: `PopupMenu::themed`, `append_owner_draw`, `ItemData`).
  `handle_measure_item`/`handle_draw_item` gain the margin/round/gradient
  logic above; every existing test in that module's `#[cfg(test)] mod
  tests` needs its `test_colors()` fixture rewritten to `MenuDesign`'s new
  shape but should otherwise keep passing unchanged (today's look is
  `MenuDesign`'s zero-margin/zero-round/solid-fill case).
- `y-pop/src/menu.rs`: `LIGHT`/`DARK` become `MenuDesign` consts;
  `resolve_colors` (probably renamed `resolve_design`) grows the new
  `ThemeOverrides` slots the same way it already layers `*-color` onto the
  base theme.
- `y-pop/src/config.rs`: `ThemeOverrides` gains `margin: Option<u16>`,
  `border: Option<BorderOverride { size, round, color }>`,
  `background_gradient: Option<(Color, Color, GradientDirection)>` (or three
  separate `Option`s — decide at implementation time based on which reads
  cleaner against the existing `any_set()` helper). `Reader::theme_overrides`
  changes shape more than it grows: today it reads five colour attributes
  straight off `root`; the `<skin>` version instead does `let Some(skin) =
  root.child("skin") else { return ThemeOverrides::default() }` (the
  `read_variables`/`root.child("variables")` idiom, reused, not a new
  pattern), warns on a second `<skin>` the same way (`root.children_named
  ("skin").skip(1)`), reads the five colours plus `margin` off `skin`
  itself, then `skin.child("border")` and `skin.child("background-gradient")`
  each get their own small attribute reader and their own `self.check(...)`
  call. `load_str`'s root-level `reader.check(root, &[...])` list drops the
  five `*-color` names and `border-color` (no longer valid *root*
  attributes) and gains `"skin"`; a new `self.check(skin, &[...])` and two
  more `self.check` calls for `<border>`/`<background-gradient>` replace
  that one list's job for the moved attributes.
- Existing tests in `config.rs` that write `<y-pop back-color="...">` etc.
  directly (`theme_color_overrides_are_read`,
  `an_invalid_theme_color_warns_and_is_left_unset`, and any other test
  hitting `ThemeOverrides` fields) need their fixture XML rewritten to
  `<y-pop><skin back-color="..."/></y-pop>` — mechanical, but real,
  since `ThemeOverrides` is already shipped and tested code, not only a
  design surface.
- `rsc/y-pop.xml` (the shipped default config) is not required to demonstrate
  any of this — a config with no `<skin>` at all still resolves through
  `resolve_design` to `LIGHT`/`DARK` exactly as before, so nothing here is a
  required config-schema change for existing configs that never used the
  five colour overrides either; only `LIGHT`/`DARK`'s own field values
  change look (see "Why `MenuColors` isn't the right shape any more" above),
  and any real-world config that *does* use today's flat `back-color=`
  needs the mechanical `<skin>`-wrapping migration noted above.

### Decisions closed out (no longer open)

Three questions this section originally left open — closed by reading
PortAL's source and this repo's own existing code more carefully, not by
asking further:

- **Should `LIGHT`/`DARK` ship non-zero `round`/`margin` by default?**
  Yes — resolved above (`border.round = 6`, `margin = 3`, `border.size = 0`)
  by their own doc comment's already-stated Windows-11-flyout reference,
  which square/flush corners never actually delivered on; not a PortAL
  question at all since PortAL has no margin concept, but PortAL's own
  built-in skins are still worth noting as a second data point: its
  *default* skin ships `border_round = 10` out of the box (round-by-default
  was PortAL's own choice too, per [[portal-analysis]]'s skin table) while
  its *dark* skin ships `round = 0` with a gradient instead — i.e. PortAL
  itself didn't treat "rounded" as universal either, it varied the look per
  skin, which is exactly what config-level `border-*`/`background-gradient-*`
  overrides (already in this design) let a `y-pop.xml` author do on top of
  whichever concrete default `LIGHT`/`DARK` ship with.
- **Gradient scope** — selected-row-only, matching PortAL's own
  `ODS_SELECTED`-gated code in `OnDrawItem`; this was already confirmed by
  reading that function, restated once more above as "decided, not open" to
  close the loop.
- **Per-level (tray root vs. submenu) theming** — one `MenuDesign` for the
  whole popup tree, no distinction. PortAL itself never exposes this either:
  `g_PortalMenuDesign` is one process-global pointer, set once by
  `setMenuSkin`, read by every `OnDrawItem` call regardless of which `HMENU`
  it's for — there is no per-menu skin slot in PortAL to have ported in the
  first place. Matches `y-pop`'s own existing single-`MenuColors`-for-the-
  whole-tree behaviour today; no design gap to close.

6. **`Presenter` trait + `list` presenter (B)** — not started.
7. **Async icon cache + live refresh, tooltips** — not started. Wraps a
   worker pool around step 4's extraction functions; not a replacement for
   the sync `IconCache`, which stays the foundation both paths build on.
8. 🟡 **Events, file watcher/live reload, variables.** `start`/`exit` events
   shipped in step 2; custom `<var>`s and builtin expansion shipped in step
   1. Reloading without a restart shipped with `y-pop-config` (`y_pop::ipc`: a
   registered message after a save; a file that does not load keeps the old
   configuration). **Not done**: watching `y-pop.xml` for edits made by hand.
9. 🟡 **Memory work.** Exact-string interning (`variables::Pool`) shipped in
   step 1, alongside the flat `ConfigTree` arena itself (also part of that
   step, ahead of where this list originally placed the "memory work"
   framing — the arena was needed from the start, not bolted on later).
   **Not done**: prefix sharing (layer 2 of the plan below) — still
   deliberately deferred until profiling a real config shows layer 1 isn't
   enough.
10. **`runner` / `radial` presenters** — not started; blocked on step 6.

**Also shipped, not itemized in the original plan at all:**

- **Tray click-detection fix** (`y_core::tray::TrayEvent::Select`, a `core`
  change) — the original `ContextMenu`/`DoubleClick`/`Other` split let
  `y-pop` treat mere mouse-hover (`WM_MOUSEMOVE`, delivered continuously
  while the cursor sits over the icon) as a click, since `Other` was the
  catch-all for anything not already named. A real, live bug, root-caused
  via `y_core::log`'s trace rather than guessed at.
- **Reentrant-popup guard** (`YPop::show_menu`) — a tray icon double-click
  delivers *both* `NIN_SELECT` and `WM_LBUTTONDBLCLK`, and each
  independently opened its own popup; the second silently clobbered the
  first's `Session` (browse/icon bookkeeping is one-popup-at-a-time state),
  permanently breaking that first popup's `WM_INITMENUPOPUP` handling for
  the rest of its life. Also found live via the log, not assumed —
  `show_menu` now refuses to open a second popup while one is already open.

Updater (`updater.cpp`'s GitHub-releases check) is out of scope — see
Decisions above.

## Open questions

- **Which overlay presenters ship in v1?** This plan assumes `list` only,
  with `runner` and `radial` explicitly deferred behind the `Presenter` seam
  (see roadmap step 10) rather than assumed in scope. Confirm, particularly
  for `runner` given the `y-term` overlap — building it now versus waiting
  for `y-term`'s own design is a real scheduling choice, not just a
  nice-to-have ordering.
