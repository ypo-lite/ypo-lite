# y-pop-config — specification

A WinUI 3 settings app for `y-pop.xml`, built the way `y-snap-config` is:
`windows-reactor` plus the `ui/` toolkit (`ui-kit/docs/ui-toolkit.md`).
This document has three parts: a review of what `y-pop.xml` can hold today,
the requirements for the editor, and the decisions behind the first mockup
(`mockup.html`, next to this file).

Status: **draft, written before the crate exists.** Nothing here is built yet.
The step-by-step build order is in [`plan.md`](plan.md).

The mock (`mockup.html`) is static HTML/CSS/JS, like `y-snap-config`'s. Open
it directly in a browser. It's also published as a Claude Artifact:
<https://claude.ai/artifact/XyAZuS7e9G7JzzGCM3Gz2i>. Its sample data is the
shipped `y-pop.xml`, plus a "Dev" menu and a root "Task Manager" app,
which are marked as examples.

---

## 1. Review: what `y-pop.xml` holds

Source of truth: `y-pop/src/config.rs`, `config/reader.rs`, `theme.rs`,
`model.rs`, and the shipped `y-pop/rsc/y-pop.xml`.

### 1.1 Root `<y-pop>` attributes (global defaults)

| Attribute | Values | Default | Notes |
|---|---|---|---|
| `shell-execute` | bool | `false` | Default launch API for every `app` (`ShellExecuteEx` vs `CreateProcess`). Per-app `shell` overrides it. |
| `show-shortcuts` | bool | `true` | Show the hotkey hint column in menus. Tri-state per node (inherit/on/off). |
| `popup` | `native` \| `overlay` | `native` | Renderer. |
| `popup-style` | `list` \| `runner` \| `radial` | `list` | Only with `popup="overlay"`. `runner`/`radial` load with a warning and fall back to `list`. |
| `theme` | `native` \| `default` \| `flat` \| `dark` \| `auto` | `native` | `black` is accepted as an alias for `dark`. `auto` follows Windows light/dark each time a menu opens. |

### 1.2 `<variables>` (root only, first one wins)

`<var name="" value=""/>`, in document order. Each value is expanded against
the builtins and the variables **above** it, never below. Builtins (from
`y-pop/src/builtins.rs`): `%pop%` (exe folder), `%config%` (config path),
`%win%`, `%system%`, `%system86%`, `%programfiles%`, `%programfiles86%`,
`%mydocs%`, `%mymusic%`, `%myvideo%`, `%desktop%`, `%appdata%` (local),
`%appdata-r%` (roaming), `%user%`. `%clipboard%`/`%clipboards%` are expanded
when the item launches, not at load.

### 1.3 `<skin>` (root only, overrides the active theme)

Attributes: `back-color`, `text-color`, `hover-back-color`,
`hover-text-color`, `disabled-text-color` (`RRGGBB`/`RRGGBBAA`, `#`
optional), `padding` (gap between rows, px), `margin` (gap inside a row,
px), `edge` (`idle`/`hover`/`always`).
Children, each all-or-nothing (an incomplete one is dropped with a warning):
`<border size round color/>`, `<background-gradient start end direction/>`,
`<icon-gradient start end direction/>` (`direction` = `h`/`v`, default `h`).
A `<skin>` without `theme=` themes the popup as if `theme="auto"` were set.

### 1.4 The tree

Each direct child of `<y-pop>` is a **root**: a `<group>` becomes its own
tray icon with a menu, and an `<app>` becomes a tray icon that launches on
click. Under a group, children are menu entries in document order.

| Element | Attributes | Children |
|---|---|---|
| `group` | `name`, `icon`, `tooltip`, `default`, `autoopen`, `show-shortcuts`, `popup`, `popup-style` | `<hotkey>`*, any node |
| `app` | `name`, `icon`, `tooltip`, `default`, `show-shortcuts`, `exe` \| `app-id`, `param`, `path`, `shell`, `elevate`, `events` (`start`,`exit`), `browse`, `filter`, `depth` (0–200), `open-folder` (`all`/`max-depth`/`never`) | `<hotkey>`* |
| `separator` | `name` (empty = plain rule, set = titled), `icon`, `tooltip`, `default` | `<hotkey>`* (accepted, meaningless) |
| `about`, `quit`, `settings` | `name` (defaults to "About"/"Quit"/"Settings…"), `icon`, `tooltip`, `default` | `<hotkey>`* |
| `break` | none | none |
| `hotkey` | `key` (`Win+Y`), `override` (bool, low-level hook fallback) | none |

`icon` is `path` or `path,index` (negative index = resource id). With no
`icon`, an app shows the icon of its `exe` and a group shows the generic
folder icon.

### 1.5 What this means for the editor

1. **The loaded `ConfigTree` can't be edited and saved back.** The reader
   expands `%vars%` and interns strings while it parses, so a save from it
   would write `C:\Windows\notepad.exe` where the file said
   `%win%\notepad.exe`. The editor needs a raw model (unexpanded attribute
   strings, same tree shape) and a `to_xml` generator. This is the
   `y_snap::config::Settings` pattern, and it belongs in the `y-pop` library
   so the warnings and validation rules come from the same code the tray app
   runs.
2. **Comments must survive a save.** `y-snap-config` drops them (its
   roadmap "Not planned"). That's not acceptable here: the shipped
   `y-pop.xml` documents itself in comments, and users annotate launcher
   configs heavily. The editor reads comments into the model as items of
   their own, shows them in the tree behind a visibility toggle, and writes
   them back in place (§2.8). Today `y_core::xml` skips comments while
   parsing (`core/src/xml.rs`, both `<!--` branches), so this needs a
   parser change (§4). Whitespace and attribute layout are still
   regenerated. Only the comments themselves, and blank lines between
   items, are kept.
3. **No reload path exists yet.** `y-pop` has no file watcher (plan step 8,
   🟡) and no IPC. A `y_pop::ipc` module like `y_snap::ipc` (a registered
   `YpoLite.YPop.Reload` broadcast, plus the single-instance check) is a
   prerequisite for "Saved and applied".
4. **Several values are tri-state.** `show-shortcuts`, `popup`, and every
   `<skin>` slot can mean "inherit", which is different from writing the
   default value explicitly. The model keeps them as `Option`, and the UI
   shows "Inherit (…)" as a real choice.
5. **Some attributes don't apply to every kind.** `browse`/`filter`/`depth`/
   `open-folder` only matter on an `app` whose target is a folder. `autoopen`
   only matters on a root group. `override` only matters if registration
   fails. The UI shows them only where they apply.

---

## 2. Requirements

Priority: **M** must (first usable build) · **S** should · **C** could
(later).

### 2.1 Shell

| # | Requirement | P |
|---|---|---|
| S1 | Same shell as `y-snap-config`: `NavigationView` (220px labelled, 48px rail below 1000px), page scrolls vertically only, status bar with Reload / Save. | M |
| S2 | Nav sections: **Settings** (General, Skin, Variables), then **Tray menus**, with one entry per root node (group or root app) and its icon, then **+ New tray menu**. | M |
| S3 | Load warnings banner above every screen (`y-snap-config` roadmap 1.1). | M |
| S4 | Save regenerates the file, keeping comments and blank lines between items (§2.8). Save and apply through `y_pop::ipc`. | M |
| S5 | Single instance (`YpoLite.YPopConfig`); the y-pop tray gets "Settings…" (as `y-snap` roadmap 2.2). | S |
| S6 | Undo for destructive tree edits (delete, move). At least one level ("Item deleted — Undo" in the status bar). | S |

### 2.2 General

| # | Requirement | P |
|---|---|---|
| G1 | Popup renderer: Native / Overlay. With Overlay, style `list`, plus `runner`/`radial` shown disabled ("not implemented yet") so the file never gets a value that only warns. | M |
| G2 | `shell-execute`, `show-shortcuts` switches, each with a one-line explanation. | M |
| G3 | Config file location (read-only path, "Open folder", "Edit XML…"). | M |

### 2.3 Skin

| # | Requirement | P |
|---|---|---|
| K1 | Theme picker as five cards (Native, Default, Flat, Dark, Auto), each a mini menu rendered with that theme. | M |
| K2 | Overrides grouped: Colours (5 swatches, each "inherit" until set), Spacing (`padding`, `margin`), Edge rule, Border, Background gradient, Icon gradient. Each composite (border, gradients) has an on/off switch, so it's written whole or not at all. | M |
| K3 | Live preview: the selected tray menu (default: the first) drawn with the resolved design, one row hovered, one submenu open, one disabled row and a titled separator. Scaled to fit like the `y-snap-config` overlay preview. | M |
| K4 | "N overrides on top of Dark" readout with Reset (as `y-snap-config` 1.2). | M |
| K5 | Native theme greys out the overrides with a note (a native `HMENU` can't be themed). Setting an override while on Native offers a switch to Auto, which mirrors what the loader does. | S |

### 2.4 Variables

| # | Requirement | P |
|---|---|---|
| V1 | Table of custom variables: name, value, resolved value, "used by N". Order matters (a variable sees only the ones above it), so the table can be reordered. | M |
| V2 | Builtins listed read-only with their resolved value on this machine. | M |
| V3 | Renaming a variable rewrites every `%name%` reference in the same edit. Deleting one that's still used warns and lists where. | S |
| V4 | Wherever a path is edited, an "Insert variable" menu and an inline hint: "C:\Windows\… could be written %win%\…". | S |

### 2.5 Tray menu screen (the tree editor)

This is the main screen. One per root node.

**Layout.** `Split`: tree on the left, inspector on the right. Below the
split threshold, the inspector stacks under the tree. The tree pane has a
toolbar (Add buttons), the tree itself, and a drop zone.

| # | Requirement | P |
|---|---|---|
| T1 | **The tree shows the menu as it will look**: icon, name, hotkey hint, nesting by indentation, separators as rules (titled ones with their title), a `break` as a dashed "column break" marker. Groups can be collapsed. | M |
| T2 | **Selection is the insertion point.** An insertion marker always shows where the next new item goes: right after the selected item, or as the last child when the selected item is an expanded group. Every Add, paste, drop and picker result lands there. You build a menu top to bottom by adding, without reordering afterwards. | M |
| T3 | **Add toolbar**: App…, Group, Separator, then a More menu with Titled separator, Folder (browse), Column break, About, Quit. A new group gets its name field focused. A new app opens the **app picker** (T8). | M |
| T4 | **Drag and drop from Explorer** (`Border::on_drop`, `DroppedData::StorageItems`). Drop on a group row: append inside. Drop on any other row: insert after it. Drop on the drop zone or empty space: at the insertion marker. Several files make several items, in drop order, all selected afterwards. | M |
| T5 | **Dropped files are resolved**, with the result shown before or after (see §3.3): `.exe` → name from the version resource's `FileDescription`, falling back to the file stem. `.lnk` → `IShellLink` target, arguments → `param`, working directory → `path`, icon location → `icon`, name = shortcut name. `.url`/documents/scripts → `exe` = the file, `shell="true"`. Folder → app pointing at the folder, with a "Browse as submenu" option. Store app shortcuts (`shell:AppsFolder`) → `app-id`. | M |
| T6 | **Paths are tokenized on the way in**: `C:\Windows\System32\…` becomes `%system%\…`, Program Files becomes `%programfiles%\…`, custom variables are matched too (longest prefix wins). The literal path stays one click away ("Use absolute path"). | S |
| T7 | **Moving without drag**: Move up / Move down, Out of group (becomes the item after its parent), Into group (into the nearest group above, at the end), and **Move to…** (a menu of every group in every tray menu, so items can cross tray icons). Plus Duplicate, Delete, Cut/Paste. All of these live in the inspector header and act on the selection. | M |
| T8 | **App picker** (opens from "App…", inline under the toolbar like `key_picker`, not a dialog): tabs **Start menu** (scans both Start Menu `Programs` folders, searchable, icons, multi-select, "Add 3 apps"), **Store apps** (`shell:AppsFolder` enumeration → `app-id`), **Browse…** (`IFileOpenDialog`, multi-select, filter exe/lnk/all), **Manual** (empty app, inspector focused on Target). Items already in this menu are flagged. | M (Start menu, Browse, Manual) · S (Store apps) |
| T9 | **Group helpers**: Sort children A→Z (inside separator-delimited sections only, so sections survive), and "Make group from selection" (wrap the selected items). | C |
| T10 | **Multi-select** (Ctrl/Shift via checkboxes in a Select mode, since `windows-reactor` has no key events) for move, delete and wrap. | C |
| T11 | Count and validation per row: a warning icon when an `exe` doesn't resolve to an existing file, a hotkey conflicts, or a group is empty. Not blocking. | S |

### 2.6 Inspector (per node kind)

Shared header: kind badge, name, and the T7 actions.

**App**
- *Identity*: Name, Icon (see icon picker), Tooltip.
- *Target*: segmented choice **Program or file** (`exe`) / **Folder**
  (`exe` + `browse`) / **Store app** (`app-id`). Each has a path field with
  Browse… and Insert variable, plus a resolved-path readout with an
  exists / not found indicator.
- *Launch*: Arguments (`param`, with an Insert menu offering `%clipboard%`
  and `%clipboards%` and a note that they are read at launch), Start in
  (`path`), Run as administrator (`elevate`), Launch through the shell
  (`shell`, labelled "Inherit (off)" when unset).
- *Folder browsing* (only for Folder): Filter (`*.sln;*.csproj`), Depth
  (0–200 with a plain-language readout: "files only" / "1 level of
  subfolders"), Open-folder entry (`all`/`max-depth`/`never`).
- *Hotkeys*: keycaps list plus "Add hotkey", using `y-snap-config`'s
  `key_picker` (move it to a shared toolkit place), and an `override` switch
  per binding.
- *Behaviour*: Default item (bold), Show shortcut (inherit/on/off), Run at
  y-pop start / exit (`events`).
- *Test*: "Run now" launches the item as y-pop would (expanded, current
  clipboard).

**Group**: Name, Icon (default folder), Tooltip, Hotkeys, Default, Show
shortcuts, Popup override (inherit/native/overlay). For a root group only:
Open on tray hover (`autoopen`). Footer: "N items".

**Separator**: Title (empty = plain rule), live sample rule.
**About / Quit**: Name, Icon. **Column break**: nothing to edit, one
explanatory line.

### 2.7 Icon picker

Inline under the Icon field, like the emoji picker in `y-snap-config`.

| # | Requirement | P |
|---|---|---|
| I1 | Current icon at 32px plus the resulting attribute value (`%win%\explorer.exe,3`), or "From target" when unset. | M |
| I2 | **From target** (clear the attribute), the default. | M |
| I3 | Every icon in the current source file in a grid (`ExtractIconExW` count, then each index), so an icon is picked by sight, not by index. | M |
| I4 | **Other file…**: `IFileOpenDialog` for `.ico`/`.exe`/`.dll`/`.lnk`/`.png`? (`.png` only if y-pop learns to load it, otherwise filtered out). | M |
| I5 | Quick sources: `shell32.dll`, `imageres.dll`, `explorer.exe`, `ddores.dll`, each opening into I3. | S |
| I6 | Icons render through `Image::source_data(EncodedImage)`: `y_core::icon::Icon` → 32bpp DIB → PNG (WIC encoder) in a new `y-core` helper, cached by `(path, index)`. | M (plumbing) |

### 2.8 Comments

Every `<!-- … -->` in the file is kept, shown, editable, and saved back where
it was.

**Where a comment lives** depends on where it sits in the file:

| In the file | In the app |
|---|---|
| Before `<y-pop>`, or before its first child | **File notes** on the General screen. |
| Among a group's children, or between root nodes | A **comment row** in the tree. |
| Inside `<variables>` | A comment row in the Variables table. |
| Inside `<skin>` | Notes on the Skin screen. |
| Inside an item's own element (among its `<hotkey>`s) | The item's **Notes** field in the inspector. |

**Attached vs free.** A comment that sits directly above an item, with no
blank line between them, is **attached** to that item. That covers the
usual "this explains the next line" case, and every comment in the shipped
file. An attached comment moves, changes menu and is deleted together with
its item. Duplicating an item doesn't copy it. Any other comment is a
**free** row and moves on its own. The inspector toggles between the two
("Keep with Notepad").

| # | Requirement | P |
|---|---|---|
| C1 | A comment row shows as a muted, italic first line, with a line count when longer. An attached comment sits just above its item, at the same indent, joined to it visually. | M |
| C2 | **Show comments** toggle in the tree toolbar, with the number of comments in this menu. When hidden, comment rows aren't drawn, and the item each one belongs to (or the next item, for a free one) gets a small marker with a count. Clicking the marker shows comments again. The setting is remembered per user (not stored in `y-pop.xml`). The same toggle applies to the Variables table. The file notes on General are always shown, as one box. | M |
| C3 | The comment inspector has a multi-line text box, the attached/free toggle and a preview of the written `<!-- … -->`. Line breaks are kept. Leading indentation is removed on load and recreated from the nesting depth on save, so moving a comment doesn't leave it oddly indented. | M |
| C4 | `--` isn't allowed inside an XML comment. The editor flags it inline, and the writer replaces it with `- -` rather than writing a broken file. | M |
| C5 | Add a comment: "Comment" in the Add menu (at the insertion marker, free), and "Add comment above" in any item's inspector (attached). | M |
| C6 | **Disable an item**: "Comment out" turns an item, with everything inside it, into a comment holding its XML, so it stays in the file without showing in the menu. A comment whose text parses as menu elements offers **Restore as item**. This is the safe way to keep an entry without deleting it. | S |
| C7 | Blank lines between items are kept as a per-item "blank line before" flag. It isn't shown in the UI, but it keeps a saved file's grouping close to the original. | S |

---

## 3. Decisions behind the mockup

### 3.1 A custom outline, not the native `TreeView`

`windows-reactor` 0.100's `TreeView` takes `TreeNode { key, text, expanded,
children }` and fires only `on_item_invoked(String)`. That means no icon,
no hotkey hint, no per-row drop target, no way to show a separator or break
as anything but text, and no reorder event. The tree is built from rows
instead (`Border` per row: indent spacer, chevron button, `Image` icon,
text, hint), keyed by node id. That's the `Table` approach, with an indent
column. Each row's `Border` takes the `on_drop` for T4.

### 3.2 Insertion point instead of drag-to-reorder

Reordering inside the app by dragging rows would need the rows to be drag
*sources*. `windows-reactor` only exposes drop-target events on `Border`
(enter/over/leave/drop of storage items or text). It has no `CanDrag` or
drag-start, and no key events for keyboard reorder either. So the design
avoids needing reorder in the first place (T2: items are created where they
belong). What's left is handled by explicit, buttons-only moves (T7), the
same trade `y-snap-config` made with ↑/↓. `GridView::can_reorder_items` +
`on_reordered` exists but is a flat list, so it can't express nesting.

### 3.3 Dropped files: add first, adjust after

A dropped file becomes an item immediately (resolved as T5 describes) and
gets selected. The inspector then shows a dismissible **"Created from
Code.lnk"** strip with one-click alternatives: *Keep as shortcut*
(`exe` = the `.lnk` itself) or *Use target* for a `.lnk`, *Browse as
submenu* or *Open folder* for a folder, *Use absolute path* or *Use
variables*. No dialog sits between a drop and the result.

### 3.4 Tray menus in the nav, not a tree root

Each root node gets its own nav entry (S2), so the tree pane never shows the
invisible `<y-pop>` root, and a group's screen is just that menu. Root-level
order (tray icon creation order) is set by Move up/down on the root's own
inspector, shown when no child is selected. A root **app** gets a plain
inspector screen with no tree.

### 3.5 Things deliberately left out

- A raw XML editor tab. Use "Edit XML…" and Reload.
- Editing `<break>` placement by column (it's just a node, placed like any
  other).
- Preserving exact formatting: attribute order, quoting, indentation and
  line wrapping inside an element. Comments and blank lines are kept
  (§2.8). The rest of the layout is regenerated.
- Live overlay-presenter preview. Only `list` exists, and only the native
  menu look is previewed for now.

---

## 4. Work items outside the new crate

| Where | What |
|---|---|
| `y-core` (`xml`) | Keep comments, opt-in: comment nodes in the arena (text and line), plus a "blank line before" flag on each node. `Element::children()` keeps returning elements only, so `y-snap`'s and `y-pop`'s readers are unchanged. A new `children_with_comments()` and `Document::prolog_comments()` serve the editor. A small XML writer (escaping, `--` handling, indentation) shared by both config apps. `y-snap-config` could then drop its own "comments aren't kept" limitation the same way. |
| `y-pop` (lib) | Raw `Settings` model (comments as items, attached ones carried by the item they precede) + `to_xml`, sharing the reader's validation; `ipc` module (reload message, instance check, config exe name); tray "Settings…". |
| `y-core` | Icon → PNG bytes (WIC); `.lnk` resolution (`IShellLinkW` + `IPersistFile`); exe `FileDescription` (`GetFileVersionInfoW`); `IFileOpenDialog` wrapper; Start-menu / AppsFolder enumeration (maybe `y-pop-config`-local). |
| `y-snap-config/src/ui` | Promote `ui/` to a shared crate or path-include it, since two apps now need it. Rule 6 (`ui/` never imports `crate::Msg`) already makes this mechanical. Same for `key_picker`/`keys`. |

## 5. Decisions on the open questions

Closed in phase 0.1.

1. **Running apps in the picker: yes, later.** A fifth source (exe paths of
   open windows, using `y-snap`'s window enumeration with a banished-class
   filter), added after phase 8. It changes neither the model nor the picker's
   shape. Tracked as plan item 8.7.
2. **Store apps: `app-id`, and the icon follows `icon`.** No PNG, and no stored
   path into the package folder (it holds the version and goes stale). The
   `icon` attribute decides:
   - **empty**: the generic icon (A);
   - **`icon="@app"`** (decided in 8.8): y-pop resolves the app's own logo
     from its `app-id` at load time through the shell (C). On anything that is
     not a Store app it means nothing, and the item gets its default icon.
   It stays a single attribute and the schema gains no new one. The picker
   writes `icon="@app"` for the Store apps it adds; the user can clear it or
   pick a file icon instead.
3. **Start with Windows: not in the config app.** Not an attribute, not a
   General switch. The user's startup is handled outside the editor for now.
4. **Shared UI: done, and wider than the toolkit.** Anything common to our
   graphical apps lives in `ui-kit`, the way `y-core` holds the Win32 plumbing
   but for a more specialised purpose. `ui-kit` exists (plan 0.2) and the key
   picker moves into it (plan 6.6).
