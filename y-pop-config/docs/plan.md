# y-pop-config — development plan

How to build the app described in [`spec.md`](spec.md) and shown in
[`mockup.html`](mockup.html), one phase at a time. Each phase ends with
something that builds, passes `cargo clippy --workspace --all-targets` with no
warnings and `cargo test --workspace`, and can be merged on its own.

This file is the running progress record, like `y-snap-config/docs/roadmap.md`.
Update a row's status in the same change that does the work.

Status: ⬜ todo · 🔨 in progress · ✅ done · 👁 done, verify live

## Ground rules

- **The mockup is the reference for layout and behaviour, not for looks.**
  WinUI controls look different. Where the app has to depart from the mock
  (a `windows-reactor` limit, a better idea found while building), note it
  in `ui-mockup.md` (to create, same shape as `y-snap-config`'s) and in the
  code, as the porting rules in `CLAUDE.md` ask.
- **Follow `ui-kit/docs/ui-toolkit.md`.** Every screen takes a
  `Viewport`, no literal sizes, `AutoGrid`/`Split`/`Table` for anything that
  must reflow, keyed children. Keep a `ui-checklist.md` for this app from
  the first screen on.
- **Logic that can be tested without a window is.** Tree edits, insertion
  points, path tokenizing, `.lnk`/exe resolution results and XML round
  trips all live in plain modules with unit tests. View code only maps
  state to controls.
- **Nothing in the user's file is lost silently.** From the first build
  that can save, comments, unknown attributes and broken entries survive a
  save or are listed in the warnings banner first (phase 2).
- **No `unsafe` in the app crate.** COM and Win32 calls (file dialog, shell
  links, icons, WIC) go in `y-core` or a thin wrapper module, the same split
  the workspace already uses.

## Phase overview

| Phase | Delivers | Depends on |
|---|---|---|
| 0 | Decisions closed, shared UI crate | — |
| 1 | `y_core::xml` keeps comments; XML writer | — |
| 2 | `y-pop` library: editable `Settings`, save, reload IPC, `<settings/>` item | 1 |
| 3 | `y-pop-config` crate, shell, load/save loop | 0, 2 |
| 4 | General and Variables screens | 3 |
| 5 | Tray menu screen: tree, inspector, add and move | 3 |
| 6 | Full app inspector, hotkeys, "Run now" | 5 |
| 7 | Real icons and the icon picker | 5 |
| 8 | Fast app creation: drop, file dialog, app picker | 6, 7 |
| 9 | Comment editing: toggle, inspector, comment out | 5 |
| 10 | Skin screen with a real-pixel preview | 3, 7 |
| 11 | Polish: validation markers, keyboard paths, live verification | all |

Phases 4, 7, 9 and 10 are independent once their dependencies are done and
can be done in any order. 5 → 6 → 8 is the critical path: it delivers the
main screen.

---

## Phase 0 — Decisions and shared UI

| # | Item | Status | Notes |
|---|---|---|---|
| 0.1 | Close spec §5 open questions | ✅ | Running apps in the picker, Store-app logos as icons, "Start with Windows", shared UI. Record each answer in `spec.md`. |
| 0.2 | **Extract the UI toolkit into a crate** | ✅ | **Done:** `ui-kit` (lib `ypo_ui`) holds `ui/*` and `keys.rs`; `key_picker.rs` and `reorder.rs` still mention `ConfigApp`/`Msg`, so they stay in `y-snap-config` until 6.6 needs a callback-based key picker. Original note: Recommended over copying. Move `y-snap-config/src/ui/` to a new workspace crate (e.g. `ui-kit`, lib name `ypo_ui`), along with `key_picker.rs` + `keys.rs` and the generic half of `reorder.rs`. Rule 6 (`ui/` never imports `crate::Msg`) already makes this mechanical. `icon_picker.rs` + `icons.rs` (the emoji picker) stay in `y-snap-config`, since y-pop's icon picker is a different thing. Move `ui-toolkit.md` with it. |
| 0.3 | `y-snap-config` builds on the extracted crate | ✅ | Pure move, no behaviour change. Its `ui-checklist.md` stays valid. Check one live run. |

**Done when** `y-snap-config` runs unchanged on the shared crate.

---

## Phase 1 — `y_core::xml`: keep comments, write XML

The parser drops comments today (`core/src/xml.rs`, both `<!--` branches).
Spec §2.8 needs them.

| # | Item | Status | Notes |
|---|---|---|---|
| 1.1 | Comment nodes in the arena | ✅ | Store each comment (text, line) as an arena node of its own kind. **`Element::children()` keeps returning elements only**, so `y-snap`'s and `y-pop`'s readers and all their tests are untouched. Add `Element::children_with_comments()` (an iterator of `Node::Element \| Node::Comment`), plus `Document::prolog_comments()` for comments before the root element. |
| 1.2 | "Blank line before" flag | ✅ | Per node (element or comment): was there an empty line between it and the previous sibling? It drives "attached" comments (spec §2.8) and keeps the file's grouping on save. |
| 1.3 | `y_core::xml::Writer` | ✅ | A small builder: `open(name)`, `attr(k, v)`, `close()`, `empty()`, `comment(text)`, `blank_line()`, tab indentation. Escapes attribute values (`& < > "`). In comments, replaces `--` with `- -`, and indents multi-line text to the current depth (strip common indentation on read, re-add on write). |
| 1.4 | Tests | ✅ | Comments before the root, between children, inside an empty-ish element; multi-line text and its indentation; `--`; parse → write → parse gives the same elements and comments. |

**Done when** both apps' existing test suites pass untouched and the new
tests cover 1.1–1.3.

---

## Phase 2 — `y-pop` library: an editable configuration

The runtime `ConfigTree` expands `%vars%` and interns strings, so it can't be
written back (spec §1.5). The editor gets its own model in the library, next
to the reader, so both apply the same rules.

| # | Item | Status | Notes |
|---|---|---|---|
| 2.1 | `y_pop::settings` model | ✅ | **An owned tree, not the runtime arena.** The editor inserts and moves constantly; the arena's "build once, never mutate" design (`docs/y-pop-plan.md`) doesn't apply here, and the memory argument doesn't either. Shape: `Settings { defaults (each Option<raw string/bool>), skin: SkinSettings, variables: Vec<VarEntry>, file_notes: Vec<String>, roots: Vec<Entry>, warnings }`; `Entry = Item(Item) \| Comment(Comment)`; `Item { id: ItemId, kind, lead: Vec<Comment>, blank_before, name, icon, tooltip, hotkeys: Vec<HotkeyEntry>, flags, show_shortcuts: Option<bool>, popup: Option<..>, app: Option<AppFields>, inner_comments: Vec<Comment>, extra_attrs: Vec<(String, String)>, children: Vec<Entry> }`. Every string is **raw** (`%win%\notepad.exe`, never expanded). `ItemId` is a counter assigned at load, stable across moves: the UI keys rows and selection by it. |
| 2.2 | `Settings::load_from` | ✅ | Parse once with comments (phase 1), and take the warnings from the real reader (`config::load_str` on the same text), so warnings are exactly what y-pop would report. Comment placement follows spec §2.8: directly above an item with no blank line → `lead` (attached); elsewhere → a free `Entry::Comment`; before the root → `file_notes`. **As built:** a comment above the first child stays with it when it is an item and attached; only comments before a leading `<variables>`/`<skin>`, or loose ones before the first child, are the `intro` notes (`prolog`/`intro`/`epilog` are three lists). Variables/skin are written in that order before the menus, whatever order the file had. |
| 2.3 | **Nothing lost on save** | ✅ | Unknown attributes are kept in `extra_attrs` and written back (still warned). An element the reader rejects (unknown element, `<app>` with no target) becomes a **disabled-item comment** holding its original XML (spec C6) instead of being dropped, and is listed in the banner. This goes beyond `y-snap-config`'s "saving drops it" and suits a file that now keeps comments. |
| 2.4 | `Settings::to_xml` / `save_to` | ✅ | Built on `xml::Writer`: attributes in a fixed canonical order per element, defaults omitted, tri-states written only when set. Save writes a temp file in the same folder and renames it over the original, so a crash never leaves half a file. Keep one `y-pop.xml.bak` from before the first save of a session. |
| 2.5 | Round-trip tests | ✅ | (a) The shipped `rsc/y-pop.xml` → `Settings` → `to_xml` → `config::load_str`: no warnings, same nodes and raw values as loading the original. (b) Same comment count and text. (c) `to_xml` is idempotent: saving twice gives identical bytes. (d) Unknown attributes and rejected elements survive. |
| 2.6 | Move app-independent code into the library | ✅ | `builtins.rs` (the config app shows builtins resolved on this PC), the `DEFAULT`/`FLAT`/`DARK` palettes from `skins.rs` (the Skin preview must use the real values, not a copy), and `launch.rs` (for "Run now", so the app launches exactly as y-pop does). All three use only `y-core`, which the library already depends on. The binary keeps calling them from their new place. |
| 2.7 | `y_pop::ipc` | ✅ | Mirror `y_snap::ipc`: `CLASS`, `CONFIG_EXE`, `CONFIG_INSTANCE`, `CONFIG_TITLE`, `reload_message()`, `request_reload()`, `is_running()`. |
| 2.8 | y-pop reloads on request | 👁 | Handle the reload message: build the new `ConfigTree`, and only if that succeeds, unregister hotkeys, rebuild tray icons and register again. A broken file keeps the old configuration and shows the error, as at start-up. This also completes the "reload without restart" half of `docs/y-pop-plan.md` step 8. The file watcher stays a separate item. **Built** (`YPop::reload`, deferred while a popup is open); 👁 needs a live check with a saved file. |
| 2.9 | `<settings/>` menu item | 👁 | **Schema addition.** A y-pop tray menu is the user's own, so there is no fixed place to put "Settings…". Add a node kind like `<about/>`/`<quit/>` that launches `y-pop-config.exe` from next to `y-pop.exe` (falling back to opening the XML, with a notification, as `y-snap`'s tray does). Add it to the shipped `y-pop.xml`, and to `Settings`, the reader, the menu builder and the config app's "More" add menu. **Built:** `NodeKind::Settings`, reader, menu, tray action, shipped file. Still to do: the config app's "More" add menu (5.7). 👁: opening the app from the tray needs a live check once `y-pop-config.exe` exists. |

**Done when** a test can load the shipped file, apply an edit, save it, and
y-pop's own reader loads the result with comments intact. A running y-pop
picks up a saved file without a restart.

---

## Phase 3 — The `y-pop-config` crate and its shell

| # | Item | Status | Notes |
|---|---|---|---|
| 3.1 | Crate scaffold | ✅ | Workspace member; depends on `windows`, `windows-reactor`, `y-core`, the UI crate from 0.2, and `y-pop` (lib). `build.rs` + `rsc/` copied from the skeleton (icon, manifest, version info). Single instance on `ipc::CONFIG_INSTANCE`, bringing the open window forward on a second launch. Window title `ipc::CONFIG_TITLE`. Minimum size as in `y-snap-config` (560×440). **As built:** icon and version info only (no manifest, like `y-snap`), the icon is `y-pop`'s for now. The window shell moved into `ui-kit::shell` (parameterised: `BarStatus`, `warnings_banner(title, advice, ..)`), and `y-snap-config` uses it too. |
| 3.2 | App state | ✅ | `ConfigApp { settings, path, screen: Screen, selection: HashMap<ItemId, ItemId> (per root), show_comments, picker: Option<PickerState>, icon_picker: Option<IconPickerState>, undo: Option<Settings>, dirty, status }`. `Screen = General \| Skin \| Variables \| Menu(ItemId)`: a root is addressed by id, never by index, so reordering roots doesn't change which screen is open. |
| 3.3 | Shell layout | ✅ | `NavigationView`: a "Settings" header with General, Skin and Variables; a "Tray menus" header with one item per root (its name, and its first hotkey as a hint); then "New tray menu". Same pane behaviour as `y-snap-config` (labelled ≥ 1000px, icon rail below). Page scrolls vertically only. **Departure:** `NavigationView` has no header item in `windows-reactor` 0.100, so "Settings" and "Tray menus" are expanded parent items that do not select. "New tray menu" is an item whose tag adds a menu and navigates to it. Needs a live look, especially on the icon rail (checklist shell 2, 11). |
| 3.4 | Warnings banner | ✅ | `InfoBar` above every screen, listing load warnings. Items kept as disabled comments (2.3) say so. **As built:** the text says a save loses nothing (unlike `y-snap-config`'s). |
| 3.5 | Status bar, Save, Reload, Undo | ✅ | Save (accent, disabled when clean) → `save_to` → `ipc::request_reload` → "Saved and applied" / "Saved — y-pop isn't running". Reload re-reads from disk, after an inline confirm if there are unsaved changes (no `ContentDialog`, per the toolkit's popup rule). One-level Undo: snapshot `Settings` before destructive tree edits (delete, move, comment out) and show "Deleted Notepad — Undo" in the status bar. **As built:** Reload with unsaved changes asks inline in the bar; Undo is offered after deleting a tray menu (the first destructive edit that exists). |
| 3.6 | Per-user UI settings | ✅ | "Show comments" and the last screen are remembered in a small `y-pop-config.ini` next to `y-pop.xml` (`y-core`'s `.ini` support), never in `y-pop.xml`. **As built:** `prefs.rs`. The screen is remembered; "show comments" is stored and loaded but its switch arrives with phase 9. |

**Done when** the app opens the real file, switches screens (General,
Skin and Variables as placeholders, menus as a stub list), saves unchanged
settings byte-for-byte equal to a second save, and y-pop reloads.

---

## Phase 4 — General and Variables

Simple screens that prove the shell, the dirty tracking and the save path
before the large one.

| # | Item | Status | Notes |
|---|---|---|---|
| 4.1 | General | ⬜ | Menu style (Native / Overlay as a segmented choice); overlay layout chips with Runner and Radial disabled ("coming later", spec G1); shell-execute and show-shortcuts switches with one-line explanations; file location with "Open folder" and "Edit XML…". |
| 4.2 | File notes | ⬜ | Prolog comments as multi-line text boxes on General, behind the shared "Show comments" switch (spec §2.8). |
| 4.3 | Variables table | ⬜ | `Table` with name, value, "resolves to" and "used by", plus ↑/↓/delete (`reorder` from 0.2: order matters, since a variable only sees the ones above it). Comment entries are full-width rows; when comments are hidden, the next variable shows a marker. Builtins table read-only, resolved through `builtins` (2.6). |
| 4.4 | Rename and delete rules | ⬜ | Renaming rewrites every `%old%` in items and later variables (a pure function in the library, tested). Deleting a variable that's still used says where. |

---

## Phase 5 — Tray menu screen: tree, inspector, add and move

The main screen. Build it without real icons first (phase 7 adds them), with
comments shown read-only from day one so they're visible as soon as the
screen exists.

| # | Item | Status | Notes |
|---|---|---|---|
| 5.1 | **Tree operations module** (`tree.rs`, no UI) | ⬜ | Pure functions on `Settings`, ported from the mockup's script and unit-tested: `find(id)` (including `lead` comments, which report their owner); `insertion_point(selection)`: after the selection, inside it if it's an expanded group, and an attached comment stands for its item; `insert`; `move_up/down/out/into`; `move_to(group)` across roots; `duplicate` (gives new ids, leaves out `lead` comments); `delete` (with attached comments); `wrap_in_group` (later, T9). |
| 5.2 | Flatten for display | ⬜ | `rows(root, expanded, show_comments) -> Vec<Row>` where `Row { id, depth, kind, hidden_comments, is_insertion_marker }`. Unit-tested against the mockup's cases: the marker sits before an attached comment, and a hidden free comment counts towards the next item's marker. |
| 5.3 | Outline view | ⬜ | **Custom rows, not `TreeView`** (spec §3.1). Each row is a `Border`, keyed by `ItemId`, containing indent, chevron button, icon slot, name, and hints (keycaps, "browse", "store", a comment marker). Separator and column-break rows have their own look. The selected row uses the accent style. The insertion marker is its own keyed row. Clicking selects; the chevron collapses. |
| 5.4 | Split layout | ⬜ | `Split` (tree / inspector) with honest minimums; it stacks below. When stacked, put the inspector right under the tree so a selection is visible without scrolling far. |
| 5.5 | Inspector frame | ⬜ | Header: kind, name, "in Explorers". Move bar: ↑, ↓, out, into, duplicate, delete, each disabled with a tooltip saying why when it doesn't apply. "Move to…" `ComboBox` of every group in every menu ("y-pop › Explorers"). A collapsible "Written to y-pop.xml" preview rendered by the real writer. |
| 5.6 | Simple inspectors | ⬜ | Group (name, tooltip, default, "Open on hover" for roots only, show-shortcuts tri-state, menu style override); Separator (title); About/Quit/Settings (name); Column break (an explanation only). Edits apply live: the tree row updates as you type. |
| 5.7 | Add toolbar | ⬜ | App… (opens a placeholder until phase 8), Group, Separator, and More (titled separator, folder, column break, About, Quit, Settings, Comment). New items land at the insertion point, get selected, and focus their first field. |
| 5.8 | Root items | ⬜ | Root inspector: ↑/↓ reorder tray icons. "New tray menu" adds a group and opens it. A root app gets the inspector alone, plus "Turn into a menu". Delete a root after an inline confirmation, since it removes a tray icon. |

**Done when** the shipped menu can be rebuilt from empty with the toolbar and
move buttons alone, saved, and y-pop shows the same menu.

---

## Phase 6 — The app inspector, complete

| # | Item | Status | Notes |
|---|---|---|---|
| 6.1 | Target mode | ⬜ | Three choices: Program or file, Folder as submenu, Store app. Switching keeps the other mode's value for the session, so switching back doesn't lose what was typed (as the mock does). |
| 6.2 | Target field | ⬜ | Path box + "Insert variable" menu + Browse… (phase 8's file dialog; disabled until then). A readout of the expanded path with found / not found (expanded through `variables::expand` + `builtins`; existence checked off the UI thread or cached, since a network path can stall). "Could be written `%programfiles%\…` · Use it" suggestion (`tokenize`, a tested library function). |
| 6.3 | Launch fields | ⬜ | Arguments with a "Clipboard" insert and its "read at launch" note; Start in; Run as administrator; Launch through the shell as a tri-state showing the inherited value; run at y-pop start / exit. |
| 6.4 | Folder fields | ⬜ | Filter, depth (0–200) with a plain-language readout ("files only", "1 level of subfolders"), and the open-folder entry. |
| 6.5 | Store app field | ⬜ | `ComboBox` of installed Store apps (needs the enumeration from 8.5; free text until then). |
| 6.6 | Hotkeys | ⬜ | **First move `key_picker` (and the generic half of `reorder`) into `ui-kit`**, made callback-based so it no longer names `ConfigApp`/`Msg`; `y-snap-config` then uses it from there. Keycaps list, "Add hotkey" opening the shared `key_picker` under the row, a per-binding Override switch, remove. Conflict check across the whole file (same combo twice) as a warning marker. |
| 6.7 | Run now | ⬜ | Calls the library's `launch` (2.6) with the expanded app and the current clipboard; any error goes to the status bar. |

---

## Phase 7 — Icons

| # | Item | Status | Notes |
|---|---|---|---|
| 7.1 | `y-core`: icon → PNG bytes | ⬜ | `Icon` → 32bpp DIB (as `to_bitmap` already does) → PNG via the WIC encoder (`IWICImagingFactory`, `IWICBitmapEncoder`). Needs `Win32_Graphics_Imaging` in the root `Cargo.toml` feature list. The result feeds `Image::source_data(EncodedImage)`. |
| 7.2 | Icon cache in the app | ⬜ | Keyed by `(expanded path, index, size)`, filled lazily as rows render, using `IconCache`'s rules (exe's own icon, folder icon, extension fallback). Loading may block on network paths, so do it off the UI thread and post the result back as a message. Until then, show a neutral placeholder. |
| 7.3 | Real icons in tree rows and inspector headers | ⬜ | Replaces phase 5's placeholders. |
| 7.4 | Icon picker | ⬜ | Inline under the Icon field (spec §2.7). Current icon at 32px plus the attribute value, "From target", source chips (its own program, `shell32.dll`, `imageres.dll`, `explorer.exe`, `ddores.dll`, Other file…), and a grid of every icon in the source (`ExtractIconExW(path, -1)` for the count). Keep the grid's `AutoGrid` capped and lazy: `shell32` alone has 300+ icons. |
| 7.5 | `y-core`: file dialog | ⬜ | `IFileOpenDialog` wrapper: filters, multi-select, start folder; returns paths. Used here for "Other file…" and in phase 8. |

---

## Phase 8 — Creating apps fast

| # | Item | Status | Notes |
|---|---|---|---|
| 8.1 | Resolve a file into an item | ⬜ | `y-core` helpers: `.lnk` → `IShellLinkW` + `IPersistFile` (target, arguments, working folder, icon location); exe description → `GetFileVersionInfoW` / `VerQueryValueW` (`FileDescription`, falling back to the file stem). Then a pure function, `item_from_file(kind, resolved) -> (Item, Origin)`, covering `.exe`, `.lnk`, folder, document/`.url` (with `shell="true"`) and `shell:AppsFolder` shortcuts → `app-id`. Paths are tokenized on the way in. Unit-test the pure part with fake resolver output. |
| 8.2 | Drop targets | ⬜ | `Border::on_drop` on every tree row and on the drop zone, with a `DragDropPolicy` accepting storage items only. Drop on a group row: append inside it. Drop on another row (or its attached comment): after that item. Drop zone or empty space: at the insertion point. `on_drag_enter`/`on_drag_leave` set the row highlight. Several files become several items in drop order. |
| 8.3 | Origin strip | ⬜ | A dismissible note at the top of the inspector after a drop or pick: "Created from Code.lnk", with the one-click alternatives from spec §3.3 (keep as shortcut / use target, show as submenu / open folder, absolute path / variables). Kept only until the item is deselected; never saved. |
| 8.4 | App picker — Browse and Manual | ⬜ | Browse… (multi-select file dialog → 8.1) and Manual (an empty app with Target focused). |
| 8.5 | App picker — Start menu | ⬜ | Scan both `Start Menu\Programs` folders (common + user) for `.lnk` files, resolved with 8.1, off the UI thread, when the picker opens. Searchable list with icons and checkboxes, "already in this menu" flags, and "Add N apps". |
| 8.6 | App picker — Store apps | ⬜ | Enumerate `shell:AppsFolder` (`SHCreateItemFromParsingName` + `IEnumShellItems`), reading the display name and `System.AppUserModel.ID`. Likely needs `Win32_UI_Shell_PropertiesSystem` and `Win32_System_Com_StructuredStorage` (for `PROPVARIANT`). Packaged apps only; desktop apps already come from 8.5. |
| 8.7 | Running apps source | ⬜ | Spec §5.1. A source in the app picker listing the exe of each open window (`y-snap`'s enumeration, filtered like its `BANISHED` list). After 8.5; no model change. |
| 8.8 | Store-app logo through `icon` | ⬜ | Spec §5.2. Decide the reserved `icon` value (proposal `@app`); y-pop resolves the logo from `app-id` at load time (shell item image), empty `icon` keeps the generic icon. Touches y-pop's reader, `Settings` and icon cache, plus an "App logo" choice in the icon picker (7.4). |

**Done when** dragging a Start-menu shortcut and a folder onto the tree
gives working entries that launch from y-pop after Save.

---

## Phase 9 — Comment editing

Comments already load, save and show read-only from phases 2 and 5. This
phase makes them editable (spec §2.8).

| # | Item | Status | Notes |
|---|---|---|---|
| 9.1 | Show/hide switch | ⬜ | In the tree toolbar with the count, and on Variables and General. When off, comment rows aren't built and the item each belongs to gets a marker button with a count that turns them back on. If the selection was a comment, move it to its item. Remembered per user (3.6). |
| 9.2 | Comment inspector | ⬜ | Multi-line `TextBox` (`accepts_return`) with the `--` warning, "Keep with Notepad" attach/detach switch, and the XML preview. Free comments get the move bar (up, down, into), but no "move to" (they belong to a place, not a menu). |
| 9.3 | Add comments | ⬜ | "Comment" in More (free, at the insertion point) and "Add comment above" on every item (attached). |
| 9.4 | Comment out / Restore | ⬜ | "Comment out" serializes the item (and everything inside it) with the writer into a disabled-item comment. "Restore as item" re-parses the comment's text with the real reader and puts the result back, refusing with the reader's warning if the text no longer parses. The disabled row shows its item's name, not raw XML. |
| 9.5 | Skin and inner comments | ⬜ | Comments inside `<skin>` as notes on the Skin screen; comments inside an item's own element (among its `<hotkey>`s) as a "Notes" field in its inspector. |

---

## Phase 10 — Skin screen

| # | Item | Status | Notes |
|---|---|---|---|
| 10.1 | Theme cards | ⬜ | Auto, Default, Flat, Dark, Native as five cards in an `AutoGrid`, each with a small menu image (10.3). Auto's card shows which theme it resolves to right now. |
| 10.2 | Overrides | ⬜ | Five colour swatches that read "theme: #…" until set, with a reset each (the `ColorPicker` opens inline in an editor panel, as on `y-snap-config`'s Palette tab); padding, margin, icon edge; Border, Gradient fill and Icon-column gradient each behind an on/off switch, written whole or not at all. "N overrides on top of Dark" + Reset all. On Native, a note that overrides switch the menu to Auto (spec K5). |
| 10.3 | **Preview drawn by the real painter** | ⬜ | `windows-reactor`'s `Brush` is theme or solid colour only (no gradients), so the preview can't be built from `Border`s faithfully. Instead, run `y_core::menu`'s own owner-draw paint functions against a memory DC for a fake menu (the first tray menu's items, one row hovered, one submenu open), encode to PNG with 7.1, and show it in an `Image` scaled to its `Viewport`. Pixel-identical to y-pop's real menu, and no second copy of the drawing rules. Check first that the paint functions can be called with a plain DC and rectangle (they're written for `WM_DRAWITEM`); if not, split out the rect-and-DC core in `y-core`. Native theme: show a note instead (Windows draws it). |

---

## Phase 11 — Polish and verification

| # | Item | Status | Notes |
|---|---|---|---|
| 11.1 | Problem markers | ⬜ | Per row: target not found, hotkey used twice, empty group (spec T11). A warning glyph with a tooltip; never blocking. |
| 11.2 | Keyboard paths | ⬜ | `windows-reactor` has no key events, so every action must be reachable by Tab + Enter/Space on buttons. Check the tree rows, chevrons, move bar and pickers are focusable in a sensible order. |
| 11.3 | `ui-checklist.md` complete | ⬜ | One table per screen, as in `y-snap-config`. Every 👁 row checked by resizing a real window through each threshold down to 560×440. |
| 11.4 | Live end-to-end pass | ⬜ | With the user, on their machine: open the real file, drop a shortcut, move it into a group, comment one item out, change the theme, Save, check y-pop reloads and the file's comments are intact in a text editor. |
| 11.5 | Later (spec "could") | ⬜ | Sort a section A→Z, wrap the selection in a group, multi-select mode. |

---

## Changes outside the new crate, by phase

| Phase | Crate | Change |
|---|---|---|
| 0 | new UI crate, `y-snap-config` | Toolkit, key picker and reorder helpers move out of `y-snap-config`. |
| 1 | `y-core` | `xml`: comment nodes, blank-line flag, `Writer`. |
| 2 | `y-pop` | `settings` (model, load, save), `ipc`, reload handling, `<settings/>` node; `builtins`, skin palettes and `launch` move into the library. |
| 7 | `y-core`, root `Cargo.toml` | Icon → PNG (WIC), file dialog; `Win32_Graphics_Imaging` feature. |
| 8 | `y-core`, root `Cargo.toml` | Shell-link resolution, file version info, AppsFolder enumeration; property-system features. |
| 10 | `y-core` | Menu paint callable on a memory DC, if it isn't already. |
