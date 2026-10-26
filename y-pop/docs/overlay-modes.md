# y-pop — overlay modes: search and the boards

Specification proposal for the display modes that `y-pop-plan.md` leaves for
later (roadmap step 10):

- **Search**: one hotkey, one input box, and the whole configuration behind
  it, plus the sections added to it (recent launches, the Start menu,
  search-only apps, plugins). It replaces the earlier `runner` idea (a launcher listing every app).
- **The boards**: icon-first layouts for one group. `radial` (driven by the
  numpad as much as by the mouse), `grid`, `strip`, and later `free`.

Status: **proposal, nothing built.** `popup="overlay" popup-style="runner"`
and `"radial"` already parse today (`y-pop/src/config/reader.rs`) and fall
back to `list` with a warning. The `Presenter` trait and the `list`
presenter (roadmap step 6) do not exist yet. Every attribute name below is a
proposal. The choices still open are listed at the end.

The mockup (`overlay-modes-mockup.html`, next to this file) is static
HTML/CSS/JS. Open it in a browser. It's also published as a Claude Artifact:
<https://claude.ai/artifact/KKi2YZokuswnYUTUTae7Rb>. Each mode in it works
with the mouse and the keyboard, has switches for the options this document
leaves open, and shows its proposed XML. Its sample data is the shipped
`y-pop.xml`, plus examples: a "Dev" and a "Utilities" group, a "Task Manager"
app, and a second tray menu, "Work".

### Revision 2 (2026-10-25)

After a first review of the mockup:

1. **Radial**: usable from the numpad. Slots *are* numpad positions, 5 is the
   center button, 0 is a modifier (§3.2).
2. **Runner → Search**: the launcher list was too close to what every other
   launcher does. It becomes a search over everything in the configuration,
   enabled with its own `<search>` element and hotkey, opening on an empty
   box rather than a list of apps (§2).
3. **Grid**: dimming non-matching tiles in place made matches hard to find.
   Typing now opens the shared search layer over the board, or, in hint mode,
   each tile carries a letter that launches it (§3.3).

### Revision 3 (2026-10-25)

1. **Radial pages**: 0 no longer switches pages. It only made sense on the
   few rings that have a second page, and the mouse had no way to do it.
   Pages are now reached with the wheel, clickable page tabs under the ring,
   or `+`/`-`. The alternative, a plain "More" group in a slot, is described
   next to it. If neither convinces, pages are dropped (§3.2, "Pages").
2. **Search: browsing.** Search also needs a way to walk every available
   app without typing. `↓` on the empty box switches to a browse view:
   columns (menu › group › item, with a details pane) or one A–Z list
   (§2.4).

### Revision 4 (2026-10-25)

Search only, after a review of Revision 3's mockup:

1. **Recent is back on the empty box.** Revision 2 made the box open on
   prefix hints, which hid the history. The box now opens on the last
   launches, first one selected, so the hotkey then Enter relaunches the
   last thing (§2.4, §2.10).
2. **Sections replace the browse mode.** `↓` on the empty box no longer
   switches to a separate browse view: it walks Recent, then a list of
   sections, and `→` opens one. The children of `<search>` are those
   sections, in order: `<recent>`, `<menus>` (the tray menus, as columns or
   A–Z), `<start-menu>` (Windows' Start menu and Store apps), search-only
   `<group>`s (apps no menu shows), and plugins (§2.4, §2.5).
3. **Plugins**, in the spirit of PowerToys' Command Palette, as sandboxed
   WebAssembly modules declared with `<plugin>`: a prefix, an optional
   section, explicit capabilities (§2.11).
4. **Variables are dropped** from search (the `%` prefix). `?` lists the
   prefixes instead (§2.6, §2.7).
5. The `empty`, `browse` and `scope` attributes of `<search>` are replaced
   by the sections. Search sub-sections are renumbered: the search layer is
   now §2.13, the search core §2.12, the hotkey check §2.14.

Decided after reviewing Revision 4 (2026-10-26):

- `<start-menu/>` is a **default section**: `<search hotkey="…"/>` alone
  means `<recent/><menus/><start-menu/>` (§2.2).
- Plugins use **`wasmi`**, for its low memory footprint (§2.11).
- Plugins **come later**, after the boards (§4, step 9).
- The `/` prefix stays, and browsed folders can also be **search-only**:
  placed in `<search>`, they are in no tray menu and under no hotkey
  (§2.5.3).

---

## 1. Where these modes fit

### 1.1 The family

| What | Shape | Best for | Enabled by |
|---|---|---|---|
| `list` | Vertical themed popup, submenus fly out | Any tree, any size | `popup-style="list"` (roadmap step 6, v1) |
| **search** | Input box, ranked results from the tray menus and the sections added to it (recent, Start menu, search-only groups, plugins) | Finding anything, by name, keyword, path or hotkey | `<search hotkey="…"/>` on `<y-pop>` (§2) |
| `radial` | Ring or 3×3 keypad of slots | 4–8 frequent items, numpad or mouse flicks | `popup-style="radial"` |
| `grid` | Tiles in sections, like the Start menu | 10–40 items you recognize by icon | `popup-style="grid"` |
| `strip` | One row of icons (a dock) | A handful of items near the cursor or an edge | `popup-style="strip"` |
| `free` *(later)* | Tiles placed by hand in `y-pop-config` | Designing your own panel | `popup-style="free"` |

Search is **not a `popup-style`**. A popup style says how *one group* is
shown. Search is not about one group: it covers every tray menu at once, and
it is also the layer any overlay presenter shows when you start typing in it
(§2.13). So `popup-style="runner"` goes away. The reader should warn that it
is replaced by `<search>`, instead of today's "not implemented yet".

`popup-style` can already be set on `<y-pop>` and on any `<group>`
(`y-pop-config/docs/spec.md` §1.1, §1.4). That is how modes mix: a `radial`
tray root can hold a group that opens as a `list`.

### 1.2 What every overlay presenter needs first

1. **`MenuLayout`, for real.** The plan's status note under "The menu" says
   the backend-neutral walk was skipped for the native popup and should be
   reconsidered once a second presenter exists. Every mode here needs it.
   Per item: kind, name, icon source, tooltip, shortcut text, `keywords`,
   layout hints (§3.6) and the item's *path* in the tree. Browse folders
   stay lazy.
2. **The `Presenter` host**: placement on the monitor under the cursor (at
   its DPI), activation and focus (§1.3), dismiss (click outside, `Esc`,
   focus loss), one place that turns a chosen item into a launch, and a way
   for one presenter to hand an item to another (a radial slot opening a
   `list`).
3. **The search core** (§2.12): the matcher, the index of the configuration,
   and the text-input state machine. Pure Rust except the input box's
   drawing, unit-tested like `config.rs`.
4. **Large icons.** `y_core::icon` returns menu-size icons
   (`y_core::menu::ICON_SIZE`). Search wants 24–32 px, the boards 32–48 px
   (`SHGetImageList(SHIL_EXTRALARGE)` or `ExtractIconExW`'s large slot). The
   cache key needs a size.
5. **Canvas primitives.** `y_core::overlay::Canvas` has rectangles, rounded
   corners and GDI text. Search needs a caret and a text selection. The
   radial needs anti-aliased circles and ring sectors. Plain pixel loops on
   the existing buffer, testable without a window.

### 1.3 Win32 constraints that shape the design

- **Keyboard input needs real focus.** `y-snap`'s overlays are
  non-activating. Every presenter here reads the keyboard (search, numpad,
  hint letters), so it must be the foreground window. A `WM_HOTKEY` handler
  and a tray click both run with foreground rights, so `SetForegroundWindow`
  works from those paths. Opened from elsewhere (the low-level-hook fallback
  of `override="true"`), activation can fail. The presenter then still shows
  and takes the mouse, and says "click to type".
- **No child controls on a per-pixel-alpha window.** A child `EDIT` does not
  paint on an `UpdateLayeredWindow` surface, so the input box is
  hand-rolled: `WM_CHAR`, caret, selection, `Ctrl+Backspace`, paste. IME
  composition is the risky part. v1 can show it in the system IME window
  placed at the caret (`ImmSetCompositionWindow`) instead of inline.
- **Dismiss on deactivation**: `WM_ACTIVATE` with `WA_INACTIVE` closes the
  popup, as clicking outside a native menu does.
- **No hooks.** The radial gesture (§3.2) comes from the overlay's own mouse
  messages, with capture while a button is held.
- **Text draw order** stays the overlay rule from `CLAUDE.md`: fill, text,
  `restore_alpha`, corners last.

---

## 2. Search

### 2.1 What it is

One input box over **the tray menus, plus the sections the user adds to
`<search>`**: recent launches, the Windows Start menu, groups of apps that
only search shows, and plugins. It answers four questions in one place:

- *Launch it*: type a few letters, press Enter.
- *Again*: the box opens on what was launched last; Enter relaunches it.
- *Where is it?* Every result says which menu, group or section it comes
  from, and `Ctrl+R` reveals a menu item in its tray menu.
- *What is this key?* Type a hotkey (`win+n`) to see what it opens, or `#` to
  list them all and check whether one is free.

What it searches is **only what a section brings in**. The Start menu is
one of the default sections, so out of the box search finds installed apps
as well as `y-pop.xml`'s entries; a `<search>` that lists its sections
without `<start-menu/>` never looks outside `y-pop.xml`. There is no file
index outside the folders the configuration browses, and no web results.

### 2.2 Turning it on

```xml
<y-pop>
  <search hotkey="Alt+Space" results="sections" row-actions="true">
    <recent count="5"/>
    <menus browse="columns"/>
    <start-menu/>
    <group name="Admin tools">
      <app name="Registry Editor" exe="%win%\regedit.exe"/>
      <app name="Services" exe="services.msc"/>
    </group>
    <app name="Scripts" exe="%mydocs%\Scripts" browse="true"/>
    <!-- later (§2.11): -->
    <plugin name="Calculator" src="plugins\calc.wasm" prefix="="/>
  </search>
  <group name="y-pop">…</group>
  <group name="Work">…</group>
</y-pop>
```

- `<search>` is optional, root only. Without it there is no global search,
  but typing in an overlay presenter still opens the search layer for that
  presenter's group (§2.13). A second `<search>` warns and is ignored.
- `hotkey`: same syntax and `override` fallback as `<hotkey>`. One attribute
  rather than a `<hotkey>` child, because search has exactly one entry
  point.
- **The children are the sections** (§2.5), in the order the empty box lists
  them. `<search hotkey="…"/>` with no children means
  `<recent/><menus/><start-menu/>`. As soon as one child is written, the
  list is exactly what is written (except `<menus/>`, always present).
- `collapsed`: `false` (default) opens on Recent and the sections;
  `true` opens on the input row alone (§2.4).
- `results`, `row-actions`: §2.8, §2.9.
- Also reachable without the hotkey: **Open Search** in the right-click menu
  of any tray icon (a left click opens the menu as written; a right click
  opens the same menu plus a separator, **Open Search**, **About** and
  **Quit**), and the `/` key in any overlay presenter. There is no search
  icon unless `tray="true"` asks for one.

### 2.3 Anatomy

```
┌────────────────────────────────────────────────────────────┐
│ 🔍 Search apps, menus, hotkeys…▌                            │  input
├────────────────────────────────────────────────────────────┤
│ RECENT                                                     │
│ ▣ Notepad                                [⧉] [✎] [✕]        │  selected: Enter relaunches
│   y-pop · %win%\notepad.exe                                │
│ ▣ Firefox                                     25 min ago   │
│   Start menu · …\firefox.exe                               │
│ BROWSE                                                     │
│ ▣ Tray menus                                         ›     │  → opens the section
│   2 tray menus · 21 apps · by menu                         │
│ ▣ Start menu                                         ›     │
│ ▣ Admin tools                                        ›     │
│ ▣ Scripts                                            ›     │  search-only folder
│ Prefixes: > Commands  # Hotkeys  / Files  ? Help           │
├────────────────────────────────────────────────────────────┤
│ ↑↓ Move  ↵ Open  → Open a section  Ctrl+Del Forget  Esc    │
└────────────────────────────────────────────────────────────┘
```

Width 560–720 px, placed in the upper third of the monitor under the cursor.
The input row never moves; the results grow downwards.

### 2.4 The empty box: recent, then the sections

The box opens with nothing typed and shows, top to bottom:

1. **Recent** (if `<recent>` is present): the last `count` launches (default
   5), from any section, newest first, each with when it was launched. The
   first row is selected, so `Alt+Space`, `Enter` relaunches the last thing.
   `Ctrl+Del` (or the ✕ button) forgets one.
2. **Browse**: one row per section, in config order, each with a count
   ("Start menu · 23 apps"). A section from a plugin carries a *plugin*
   badge.
3. One line of prefixes (§2.7), each clickable.

`↓` and `↑` walk these rows like any list. On a section row, `→`, `Enter` or
`Tab` **opens the section**: its name goes into the box as a chip, the list
shows its content, and typing searches inside it. `←` or `Backspace` on an
empty box comes back. What an open section shows with nothing typed depends
on its shape:

| Section | Shows when opened |
|---|---|
| Tray menus, `browse="columns"` *(default)* | Miller columns: the tray menus, then the content of what is selected in the column before (group or browsed folder), and a **details pane** for an app (big icon, path in the tree, exe, hotkey, tooltip; Open / Reveal / Edit). `↑`/`↓` move, `→` goes in, `←` comes back (out of the section from the first column). |
| Tray menus, `browse="a-z"` | Every app of every tray menu in one alphabetical list, letter headers, a clickable letter index. |
| Start menu | A–Z, like the Start menu's "All apps"; a shortcut inside a Start-menu folder says so on its second line. |
| A search-only `<group>` | Its items, in config order. |
| A search-only folder | Its files and sub-folders, listed when opened; `→` on a sub-folder goes in. |
| A plugin section | What the plugin returns for an empty query. |

This replaces Revision 3's `empty=` attribute and its separate browse mode:
browsing is now opening a section, and `↓` no longer switches modes.

**Collapsed** (`<search collapsed="true">`): for users who want a bare
launcher box, the box opens as **its input row alone**, with a small ⌄
button at its right end. Nothing else changes; the list is just folded:

- `↓`, or a click on ⌄, unfolds Recent and the sections, first row selected.
  `↑` on that first row folds them again.
- Typing shows results as usual. Erasing the text folds the list again
  (going back up from an opened section does not: you asked to see it).
- `Esc` clears the text, then leaves an opened section, then folds the
  list, then closes.
- `Enter` on the folded box does nothing (there is no selection to act on),
  so collapsed mode loses the "hotkey, Enter relaunches the last thing"
  shortcut; that is the trade the user makes by choosing it.
- The footer of key hints is hidden while folded, so the window is one row
  high.

### 2.5 Sections and their sources

Each child of `<search>` is one section. All of them are searched by a
plain query, and each can be opened alone (§2.4).

| Element | Brings in | Notes |
|---|---|---|
| `<recent count="5"/>` | The last launches | Not a source of its own: it lists items from the other sections. Needs the history file (§2.10). |
| `<menus/>` | Every tray menu: groups, apps, browsed folders, built-ins | Present implicitly if omitted, placed after `<recent>`. `browse="columns\|a-z"`. `include="y-pop;Work"` limits it to some tray menus, `;`-separated because a menu name may contain spaces (replaces Revision 3's `scope`). |
| `<start-menu/>` | Shortcuts in the Start menu's Programs folders, and installed Store apps | On by default (§2.2). §2.5.1. |
| `<group name="…">` | Apps that **only search shows** | §2.5.2. |
| `<app browse="true" …/>` | A folder that **only search shows** | §2.5.3. |
| `<plugin …/>` | Whatever the module returns | Later (§2.11). A plugin only gets a section if it says so (`section="…"`). |

A section can be hidden from the empty box but still searched with
`browse="false"`, and searched only when opened with `global="false"`.

#### 2.5.1 The Start menu

- **What is read**: `.lnk` files under `FOLDERID_Programs` and
  `FOLDERID_CommonPrograms`, and the items of the virtual
  `FOLDERID_AppsFolder` (Store and packaged apps, launched as
  `shell:AppsFolder\<AUMID>`, which y-pop already supports). Each shortcut
  is resolved with `IShellLinkW` to its target, for de-duplication and for
  "containing folder".
- **Filtered out** by default: uninstallers, "Read me", help and website
  links (by name and target extension, a small list), and duplicates
  between the user and common folders. `exclude="…"` adds name patterns.
- **When**: read on a background thread the first time search opens, cached
  for the life of the process, and refreshed after a change notification
  on the two folders (`SHChangeNotifyRegister`). The apps folder has no
  cheap change signal, so it is re-enumerated in the background at most
  once per opening and results that change appear without moving the
  selection. Opening search never waits for it: until the index is ready,
  the section says "reading the Start menu…".
- **De-duplication**: an app that is both in a tray menu and in the Start
  menu (same target exe or AUMID) appears **once** in results, as the menu
  entry, because that one carries the user's name, keywords and hotkey. The
  Start menu section still lists it when opened.
- **Add to a menu** (`Ctrl+P`, the ＋ button): starts `y-pop-config` with a
  new entry filled from the shortcut (name, exe, arguments, icon, working
  directory), ready to drop into a tray menu. This is what makes the Start
  menu section useful beyond launching: it is where new menu entries come
  from.
- No admin rights, no COM server of our own, no Windows Search index. The
  memory cost is one string set per shortcut, a few hundred entries.

#### 2.5.2 Search-only groups

A `<group>` directly inside `<search>` is an ordinary group (same children,
same attributes, nested groups allowed) that **has no tray icon and no
hotkey**. Its items only appear in search: in results, labelled "only in
search" on their second line, and as a section that opens to their list.

What it is for: things you want to reach by name but never want to see in a
menu: admin consoles (`regedit`, `services.msc`, `eventvwr.msc`), rarely used
tools, long tails of scripts. Several are allowed; each is its own section.

- A `<hotkey>` child inside such a group warns and is ignored: it would
  make it a tray menu in all but name.
- `Reveal` does not apply (there is no menu to reveal it in); `Edit` does.
- `y-pop-config` shows these groups under a "Search" node, next to the tray
  menus.

#### 2.5.3 Search-only folders

A browsed folder (`<app … browse="true">`) placed directly inside
`<search>`, or inside a search-only group, is the folder counterpart of
§2.5.2: it appears in no tray menu and under no hotkey, only in this box.
Typical use: a folder of scripts, notes or project checkouts you want to
find by name, without a menu entry that would list them all.

- **A section of its own** when it sits directly in `<search>`: opened, it
  lists its files, listed at that moment, never in advance; sub-folders can
  be entered with `→`.
- **Its files are searched through `/`** (§2.7), together with the menus'
  browsed folders. That is the point of keeping `/`: it is the one place
  where the files of every browsed folder, in a menu or search-only, are
  searched at once. The folder itself is a plain result like any folder.
- Same attributes as a browsed folder in a menu (including `index="true"`
  for a network folder), same lazy listing on first use of `/` per opening.
- `Reveal` does not apply; `Edit` does. In `y-pop-config` they sit under the
  same "Search" node as the search-only groups.

### 2.6 What is searched, and how

The index is built from the loaded `ConfigTree` at load and reload, and
from each source when it is ready, never per keystroke.

| Field | Weight | Notes |
|---|---|---|
| Name | 1 | Matched characters are emphasized. |
| `keywords` | 0.8 | New attribute on any node: space-separated extra words. Start-menu entries get none; plugins may supply them. |
| Exe file name | 0.6 | `cmd` finds "Command line" through `cmd.exe`. |
| Path in the tree | 0.45 | `work rem` finds "Work › Remote Desktop". |
| Tooltip | 0.4 | Start-menu shortcuts use their comment field. |
| Hotkey | strong | Only when the query starts with a modifier (`win`, `ctrl`, `alt`, `shift`). Order-insensitive: `shift+ctrl+esc` finds `Ctrl+Shift+Esc`. |

A result found by anything other than its name says which field matched on
its second line ("matched keyword *mail*", "hotkey *Win+N*").

Tray roots, groups and sections are results too. `Tab` on one makes it the
scope (a chip in the box); `Backspace` on an empty query or `Shift+Tab` goes
back up. Browsed folders are results; their *files* are only searched
through `/` (§2.7) or by tabbing into the folder, so opening search never
waits on a slow disk or a network share.

**Variables are no longer searched.** Revision 3 had a `%` prefix listing
them with their values. It answered a question nobody asks from a launcher;
the place to see variables is `y-pop-config`. Copying a path still expands
them.

### 2.7 Prefixes

A first character picks what to search. Typed, or clicked on the empty
box's prefix line.

| Prefix | Searches | Enter does | Notes |
|---|---|---|---|
| *(none)* | Every section with `global="true"` (the default), and commands, ranked lower | Launch / open | |
| `>` | y-pop commands | Run it | Reload configuration, Open settings, Edit `y-pop.xml`, Open configuration folder, Clear search history, Refresh the Start menu, Reload plugins, Start with Windows, About, Quit. |
| `#` | Hotkeys | Launch what the key opens | `#` alone lists every binding in the file, including the search hotkey and each tray menu's. `#ctrl+alt+x` with no match **checks the key** (§2.14). |
| `/` | Files inside browsed folders: the menus' and the search-only ones (§2.5.3) | Open the file | Folders are listed on first use of `/` per opening, in the background; results join as they arrive. Network folders (`\\…`) are skipped unless `index="true"` is set on them, and say so. |
| `?` | The prefixes | Type that prefix | Lists every prefix, the plugins' included. |
| *a plugin's* | That plugin only | The plugin's action | `prefix="="` on `<plugin>`. One character, not a letter or digit, not one of the above. Two plugins with the same prefix: the second loses it, with a warning. |

### 2.8 Results

`results="sections"` (default): the best match first as **Top hit**, then
the rest grouped, at most five per group: *Apps* (tray menus and
search-only groups), *Start menu*, one group per plugin that returned
something, *Menus and groups*, *Folders*, *Commands*. `results="flat"`: one
ranked list.

Ranking (shared matcher, §2.12): whole-name prefix > word-start prefix >
acronym > contiguous substring > scattered characters, times the field
weight, plus the history bonus. Ties keep config order, never alphabetical.
A plugin result carries its own score (0–1), scaled into the same range,
so a plugin cannot push itself above an exact name match. The one exception
is a plugin that claims the whole query (the calculator on `12*7+3`), which
is shown as Top hit; §2.11.

### 2.9 Result actions

What can be done to the selected result is **configured**, not fixed: the
actions are named built-ins, you choose which exist, which have a shortcut and
which get a button, and the **action menu** lists the ones that apply to the
selected row with their shortcuts.

| `do=` | Menu label | Applies to | What it does |
|---|---|---|---|
| `open` | Open | all | Launch; on a menu, group, folder or section: open it in the box. Always present. |
| `run-as-admin` | Run as administrator | programs that do not already have `elevate="true"` | The launch with `elevate`, for this once. y-pop already knows which programs need it; this is for the others. |
| `show-in-folder` | Show in folder | programs with a file, folders, files | Explorer, with the file selected. |
| `copy-path` | Copy path | programs, folders, files | The path, `%vars%` expanded (a Store app: its `shell:AppsFolder\…` address). |
| `reveal` | Reveal in menu | tray-menu items | Opens the item's tray menu with the item highlighted, its submenus open. With an overlay `list` this is exact. With the native popup, y-pop can open the right submenu chain but not highlight a row (`TrackPopupMenu` has no "preselect" API), so it opens the submenu that contains the item. |
| `edit` | Edit in settings | tray-menu and search-only items | Starts `y-pop-config` on that entry (new command-line argument `--select "<path in tree>"`; if it is already running, sent through `y_pop::ipc`). |
| `add-to-menu` | Add to a menu | Start-menu items | §2.5.1. |
| `forget` | Forget | recent rows | Removes it from the history (§2.10). |

An action that does not apply to the selected row is not in its menu and its
shortcut does nothing.

```xml
<search …>
  <actions menu-key="Alt+Enter" quick-pick="true">
    <action do="open" key="Enter"/>
    <action do="run-as-admin"/>                    <!-- in the menu only -->
    <action do="reveal" key="Ctrl+R" button="true"/>
    <action do="edit" key="Ctrl+E" button="true"/>
  </actions>
</search>
```

- **`<actions>` is optional.** Without it, the defaults are: `open` on
  `Enter`, `run-as-admin`, `show-in-folder` and `copy-path` in the menu only,
  `reveal` on `Ctrl+R` and `edit` on `Ctrl+E` (both with a button), `forget` on
  `Ctrl+Del`. Written, it lists **exactly** the actions that exist, in the
  order of the menu, except that `open` is always there (added first, on
  `Enter`, if missing).
- **`key`**: a shortcut in the result list, written like a hotkey
  (`Ctrl+Shift+Enter`). Usable: a letter or digit with `Ctrl` or `Alt`,
  `Enter` alone or with any modifiers, a function key, `Ctrl+Del`. Not
  usable, and a warning: the keys that edit the box (`Ctrl+A`, `Ctrl+C`,
  `Ctrl+V`, `Ctrl+X`), `Alt+1…9`, plain letters, the arrows and the like. Two
  actions on one shortcut: the second loses it, with a warning.
- **`button="true"`** puts the action on the selected row as a small button;
  `row-actions="false"` on `<search>` hides all buttons. A **⋯** button always
  ends the row and opens the action menu.
- **The action menu** opens on `Alt+Enter` (`menu-key` rebinds it), on a right
  click on a row, and on `Shift+F10` or the Menu key. `↑` `↓` `Enter` and a
  click choose; the actions' own shortcuts work inside it; `Esc` closes it.
- **`quick-pick="false"`** turns off `Alt+1…9` (open the n-th result) and the
  `Alt+n` hints on the rows.
- Plugin results bring their own actions (§2.11). Custom actions (re-launch the
  entry with another `param` or elevation, e.g. "run with the clipboard") are
  not part of the first cut.

### 2.10 History

`<recent>` lists it; the ranking bonus uses it whether `<recent>` is shown
or not. `<search history="false">` turns both off and writes nothing.

- Stored next to the config as `y-pop.history`: a short text file, one line
  per item (key, count, last launch), capped at 200 lines, written when
  search closes.
- Keyed by **source and identity**, never `NodeId`, which changes on every
  reload: `menu:Work/Outlook` (path in the tree; renaming an item resets
  its history), `start:<target exe or AUMID>`, `plugin:<name>:<id the
  plugin gives>`. A plugin result is only remembered if the plugin marks it
  as stable.
- An entry whose item no longer exists is skipped when listing and dropped
  at the next write.
- The bonus is computed once per opening, so rows don't reorder between
  keystrokes for reasons the user can't see.
- Cleared with `>Clear search history`; one entry with `Ctrl+Del`.

### 2.11 Plugins (WebAssembly)

**Later, not in search's first version** (§4, step 9). The built-in
sections don't depend on plugins; this section fixes the design so that
nothing built before rules it out.

PowerToys' Command Palette shows the value: the box stays the same, and
extensions add commands (a calculator, a window switcher, a clipboard
history, a project list…) without the host knowing about them. Its
extensions are separate processes talking COM/WinRT, installed as packages.
That is the wrong weight for y-pop. The proposal here: **plugins are
WebAssembly modules**, one `.wasm` file each, loaded in-process in a
sandbox, declared in `y-pop.xml`.

```xml
<plugin name="Windows" src="plugins\windows.wasm"
        prefix="!" section="Open windows" global="false"
        allow="windows"/>
```

| Attribute | Meaning |
|---|---|
| `src` | Path to the module, relative to the config folder; `%vars%` allowed. |
| `name` | Display name: group header, section name default, history key. |
| `prefix` | Optional one-character prefix (§2.7). |
| `section` | If present, the plugin is also a section with this name, opened like the others. |
| `global` | `true` (default): its results join plain queries. `false`: only through its prefix or section. |
| `allow` | The host capabilities it gets (below). Nothing by default. |
| any other attribute | Passed to the plugin as a setting, as a string (`units="metric"`). |

#### What a plugin can do

The interface is deliberately small. A plugin is a set of exported
functions:

- `describe() -> manifest`: name, version, the prefix it would like, whether
  it can claim a whole query, the settings it understands.
- `query(text, generation) -> items`: called after each keystroke that
  reaches it (debounced, see below), and with an empty text to fill its
  section.
- `invoke(item_id, action_id)`: the user picked a result or one of its
  actions.

An **item** is: id, title, subtitle, icon (a built-in icon name, a file path
whose icon the host extracts, or a small PNG), score (0–1), whether it
**claims** the query (Top hit), whether it is **stable** (worth a history
entry), and a list of actions. An **action** is one of a few host verbs, so
the host draws and runs them and the plugin never touches a window:

| Action | Host does |
|---|---|
| `launch(path, args)` | Starts it, through the same code as a menu item. Needs `allow="launch"`. |
| `open-url(url)` | Default browser. Needs `allow="launch"`. |
| `copy(text)` | Clipboard. Always allowed (the user asked for it). |
| `activate-window(token)` | Brings a window forward. Needs `allow="windows"`. |
| `call(action_id)` | Calls `invoke`, for the plugin's own logic. |
| `set-query(text)` | Replaces the box's text: drill-down, like a sub-page. |

Capabilities in `allow`, each a host function the module can import, and
the only way it reaches the outside: `launch`, `windows` (list top-level
windows: title, exe, icon; activate one), `clipboard-read`, `fs-read`
(only under the folders listed in `fs-read-paths`), `http` (GET only, with
`http-hosts` listing the hosts), `env` (read environment variables). A
module that imports something it isn't allowed fails to instantiate, with a
warning balloon naming the missing capability: the configuration's usual
non-fatal severity (CLAUDE.md, "XML config parsing").

#### Runtime

- **`wasmi`** (a WebAssembly interpreter in pure Rust), decided over
  `wasmtime` for its memory footprint. The plugins expected are small (a few keystrokes' worth of
  string work), so interpreter speed is enough, and it keeps y-pop's
  binary and working set in its "low memory" budget: no JIT, no code cache,
  no extra threads. Both use the same module format, so the choice can be
  revisited without changing plugins.
- **Off the UI thread.** One worker thread runs every plugin. A keystroke
  posts the query with a **generation number**; the worker drops queries
  overtaken by a newer one, and posts results back with `PostMessageW`
  (the `WM_APP` pattern from CLAUDE.md). Results from an old generation are
  thrown away. Built-in results never wait for plugins; plugin rows join
  when they arrive, below the rows already shown, so the selection doesn't
  jump.
- **Budgets.** Each call runs with a fuel limit (`wasmi`'s instruction
  metering, about 20 ms of work) and a memory cap (default 16 MB per
  instance). A plugin that exceeds either is stopped for this call; three
  times in a row and it is disabled until `>Reload plugins`, with a balloon.
- **Lazy.** A module is compiled and instantiated the first time search
  opens, not at y-pop start, and dropped after search has been closed for a
  few minutes (then the working set is trimmed, like the rest of y-pop).
- **The ABI.** The component model (WIT interfaces) would be the clean way
  to describe items, but `wasmi` doesn't support it. So v1 is a core-module
  ABI: `alloc`/`free` exports, and each call passes one UTF-8 buffer in and
  gets one back, in a small line-based format the host parses by hand (no
  JSON dependency, same spirit as `core/src/xml.rs`). A `y-pop-plugin`
  crate wraps it for Rust authors (`#[plugin] fn query(…)`); any language
  that compiles to `wasm32-unknown-unknown` can follow the format.

#### Examples to ship with the feature

Each one tests a different part of the interface:

- **Calculator** (`=`, `global="true"`): claims the query when the whole
  text is an expression (`12*7+3`), so it is Top hit; Enter copies the
  result. No capabilities.
- **Windows** (`!`, section "Open windows", `global="false"`): lists
  top-level windows, Enter activates one. `allow="windows"`.
- **Projects** (section, `fs-read`): one row per sub-folder of a code
  folder, actions "open in VS Code" and "open terminal here". Shows
  settings (`root="%mydocs%\Github"`) and multi-action items.

Not proposed: installing plugins from a catalog, auto-update, or plugins
drawing their own UI. A plugin is a file the user copies and declares.

### 2.12 The search core (shared)

One matcher, one index type, one text-input state machine, used by search,
the search layer (§2.13), `list`'s type-to-search, and later `y-term`.
Fuzzy subsequence, case- and accent-insensitive, word starts and runs scored
higher, gaps cost a little. Budget: a few thousand entries per keystroke in
well under a millisecond, no allocation per entry (the Start menu adds a
few hundred). Written in `y-pop` with no `y-pop` type in its signatures (it
takes `(label, fields, secondary text, icon handle)` and returns indices),
moved into `y-core` when `y-term` consumes it.

### 2.13 The search layer inside other presenters

Typing a letter in any overlay presenter that doesn't use letters itself
opens the same search, **scoped to that presenter's group**, drawn as a
panel over it. The board behind fades but keeps its layout. `Esc` closes the
layer and returns to the untouched board. This is what replaces the grid's
in-place dimming (§3.3). The radial opens it with `/` (or `0`, §3.2), since
its digits are taken. Sections and plugins are not part of the layer: it
searches the group it was opened from.

### 2.14 Hotkey check

`#` followed by a full combination that nothing in `y-pop.xml` uses shows
one more row:

- **"Ctrl+Alt+X is free"**: y-pop tried `RegisterHotKey` with it on its own
  window and it succeeded (then unregistered it immediately), so no other
  application holds it either. Enter opens `y-pop-config` ready to assign it.
- **"Win+E is taken by File Explorer (Windows)"**: the registration failed.
  For keys Windows itself reserves, y-pop can name the owner from a small
  built-in table. For a key another application holds, Windows doesn't say
  which one, and the row says so. It also notes that `override="true"`
  could still claim it with the low-level hook.

---

## 3. The boards

### 3.1 What the boards share

- **Tiles.** Icon plus label. A group tile previews up to four of its icons.
- **Groups open in place** with a back control ("‹ Explorers"). Radial opens
  a nested ring instead (§3.2).
- **Browse folders hand off to `list`**, anchored to the tile: their contents
  have unknown size and no icons worth arranging.
- **Typing** opens the search layer (§2.13), or, where `typing="hints"`,
  hint letters (§3.3). Not dimming in place.
- **Spatial arrows** move to the nearest tile in that direction; `Enter`
  launches or opens, `Backspace` goes back, `Esc` closes the layer, then goes
  back, then closes.
- **Separators**: a titled one starts a section with a header, an untitled
  one a section with a rule. `break` starts a new column in `grid`, ignored
  elsewhere.

### 3.2 `radial`: ring or keypad, numpad first

#### Slots are numpad positions

```
   7  8  9         nw  n  ne
   4  5  6    =     w  ·  e        (5 is the center button, 0 a modifier)
   1  2  3         sw  s  se
      0
```

An 8-slot ring maps one-to-one onto the numpad around 5. A 4-slot ring uses
8, 6, 2, 4 only, and a diagonal digit there does nothing (with a short
message). `slot="9"` and `slot="ne"` are the same position; digits are
probably what most people will write.

This makes the radial a **numpad menu that can also be flicked**: the same
position is the same item whether it is reached by `9`, by a flick up and to
the right, or by a click. Muscle memory carries across all three.

Nested groups compose: `6` opens Dev, `2` picks Git Bash. So "62" is a
two-key shortcut that was never configured as one. Possible later addition,
from marking menus: **expert mode**. If the whole sequence is typed within
~250 ms of opening, nothing is drawn at all, and the ring only appears when
you hesitate.

#### Shape

`shape="ring"` (default) or `shape="keypad"`: a 3×3 grid of square cells
that mirrors the numpad exactly. Same slots, same keys, same hit-testing by
direction, so a flick works on both. The keypad shape reads better for
numpad users and labels fit more easily. The ring reads better for mouse
flicks. Both show the digit in each slot's corner.

#### 5, the center button

The center (the hub, numpad 5, or a mouse release in the center) is one
button whose job is an option:

| `five=` | 5 does | Notes |
|---|---|---|
| `center` *(proposed default)* | Opens the **center slot**: `slot="5"` at the root; in a nested group, its `default="true"` item. With none set, behaves as `back`. | Gives the most-used item the easiest key. The hub shows its icon. |
| `back` | Back one level; closes at the root. | The old behaviour. `.` and `Esc` do this anyway. |
| `repeat` | Relaunch the last item launched from this radial. | "Do it again" without remembering where it was. Remembered per session, or across sessions with `history`. |

#### 0, the modifier

| `zero=` | 0 does | Notes |
|---|---|---|
| `actions` *(proposed default)* | Arms an action: the next digit opens an **action ring** for that slot instead of launching it. `0 5` acts on the current group itself. | Action ring, 4 slots: `8` Run as administrator, `6` Open containing folder, `2` Copy path, `4` Edit in y-pop-config; `5` launches normally. For a group: `8` Open as list, `6` Edit. Right-click on a slice opens the same ring, so the mouse has it too. |
| `search` | Opens the search layer scoped to this ring. | Same as `/`. |

(Revision 2 had a third value, `page`. It was removed: on a ring without a
second page, 0 would do nothing, and it gave the mouse no way to change
page. Pages are handled separately, below.)

`0 then digit` (tap 0, then the digit) is proposed rather than *holding* 0,
because holding a key while pressing another on the same hand is awkward on
the numpad, and a tap works identically with the digit row.

#### The other numpad keys

| Key | Does |
|---|---|
| `.` | Back one level; closes at the root. |
| `Enter` | Picks the highlighted slot (arrows highlight); with nothing highlighted, same as 5. |
| `+` / `-` | Next / previous page, if the ring has pages. |
| `/` | Search layer, scoped to this ring. |
| `*` | Unassigned. A candidate for "toggle labels" or "reveal this ring in y-pop-config". |

#### Opening it from the numpad

The radial's hotkey can itself be a numpad key (`Ctrl+Num0`, or a bare key
like `Num/` if the user gives up that key). The hotkey parser needs the
`Num0`…`Num9`, `NumDecimal`, `NumAdd`, `NumSubtract`, `NumMultiply`,
`NumDivide`, `NumEnter` names. Check `y_core::hotkey`'s key table before
assuming they exist.

#### Win32 details

- **Read the numpad by scan code, not by virtual key.** With Num Lock off,
  numpad 8 arrives as `VK_UP`, 7 as `VK_HOME`, 5 as `VK_CLEAR`, and `.` as
  `VK_DELETE`. The dedicated arrow and navigation keys send the same virtual
  keys but with the **extended-key bit** (bit 24 of `lParam`) set; the
  numpad's don't. So "not extended + navigation VK" means numpad, whatever
  the Num Lock state. Numpad Enter is the reverse: `VK_RETURN` *with* the
  extended bit.
- **Keyboards without a numpad**: the digit row gives the same numbers, so
  the same meaning (`keys="numpad"`, default, accepts both). An alternative
  cluster that keeps the 2D shape: `keys="qwe"` maps Q W E / A S D / Z X C
  onto 7 8 9 / 4 5 6 / 1 2 3, with Space as 0.
- No hook needed: the overlay is the foreground window while open, so plain
  `WM_KEYDOWN`.

#### Mouse

Unchanged from the first revision. Selection is by direction from the edge
of the center out to twice the radius. Press, flick, release is a marking
menu. A click beyond twice the radius closes. The hub is the 5 button, and
right-click is the 0 modifier on that slice.

#### Rings, slots, pages

- A ring has 4 or 8 slots: 1–4 items get 4, 5–8 get 8. More than 8 loads with
  a warning and the extra items go into a "More" slot placed last, unless
  the group uses `<page>`.
- Items fill slots in config order (8, 9, 6, 3, 2, 1, 4, 7, clockwise from
  north) unless they pin one with `slot`. An empty slot stays empty, so adding
  an item never moves the others.
- **Pages** (still to be confirmed, see below).
- A nested group opens a new ring **centered where the pointer is** (or on
  the slot, from the keyboard), parent faded behind. The other option, a
  second, larger ring around the first, is kept in the mockup for
  comparison only.

#### Pages: keep only if they work with the mouse

A `<page>` child of a radial group would hold up to 8 more items with their
own slots, so 16 items share one hotkey. The test it has to pass is that a
mouse-only user can reach page 2 as easily as a numpad user. Proposed ways
in, all available together whenever a ring has more than one page (and none
of them shown otherwise):

| Input | Changes page by |
|---|---|
| Mouse | **Wheel** over the ring, and **page tabs** ("1", "2") drawn under the ring; clicking one switches. The hub also shows "1/2". |
| Numpad | `+` / `-`. |
| Digit row / `qwe` | `PgDn` / `PgUp`. |

The wheel is the main mouse path: no movement away from the ring, and it
works mid-gesture (button held, wheel turned, flick, release). The tabs make
pages visible, since a wheel alone is undiscoverable.

**The alternative: no pages, a "More" slot.** Page 2 becomes an ordinary
group placed in a slot (`<group name="More" slot="7">`), opening a nested
ring like any group. It needs no new concept, no new input and no new
element, and works identically with mouse, numpad and flicks (`7` then a
digit). The cost is one slot on page 1 and one extra step.

Recommendation: ship the **More slot** (it is just a group, so it exists
anyway), and add `<page>` only if real use shows the extra step matters. The
mockup has both ("Page 2: Pages / More slot") so they can be compared.

```xml
<group name="Quick" popup="overlay" popup-style="radial">
  <hotkey key="Ctrl+Num0"/>
  <radial shape="keypad" keys="numpad" five="center" zero="actions"/>
  <app name="Notepad" slot="8" exe="%win%\notepad.exe"/>
  <group name="Dev" slot="6">
    <app name="VS Code" default="true" …/>   <!-- 6 then 5 -->
    …
  </group>
  <app name="File Explorer" slot="5" exe="%win%\explorer.exe"/>
  <group name="More" slot="7">            <!-- the "More slot" alternative -->
    <app name="Calculator" exe="calc.exe"/>
    …
  </group>
</group>
```

The plan's caution still holds: radial is for **one small, frequently used
group**. `y-pop-config` should warn past 8 items per page or 2 levels of
nesting.

### 3.3 `grid`

Tiles in sections, like the Start menu's pinned area.

- `columns` (default 5). Tile sizes per node: `tile="small"` (icon only),
  `"medium"` (default), `"wide"` (2×1, icon and label side by side plus the
  tooltip).
- Groups open in place (§3.1). Inline expansion was considered. It shifts
  every row below, which works against learning positions.
- **Typing**, `typing=`:
  - `search` *(default)*: the search layer (§2.13) over the board, over the
    whole board's subtree (nested groups included). Each result shows its
    path below the current group. `↑`/`↓` move in the results, `Enter`
    launches. `Esc` returns to the board as it was.
  - `hints`: every tile shows a letter; pressing it launches the tile or
    opens the group, whose tiles get their own letters. The default letter
    is the first letter of the name not already taken. `hint="n"` pins one.
    No search at all in this mode, and `/` still opens the search layer.
- The first revision's "dim non-matching tiles in place" is dropped. Matches
  were hard to spot among dimmed tiles, and nested items were invisible
  unless a badge was added. The mockup keeps it under "Dim (old)" to
  compare.
- Parameters: `<grid columns="5" tile-size="84" labels="always|hover"
  typing="search|hints"/>`.

### 3.4 `strip`

One row of icons, like a dock. Unchanged from the first revision.

- `anchor="cursor"` (default) or a screen edge (`bottom`/`top`/`left`/
  `right`, vertical for left/right).
- Separators become thin dividers; labels on hover, or `labels="always"`.
- A group or folder opens a `list` stack above its icon.
- `typing` as in `grid`; hint letters suit a strip well.
- Parameters: `<strip anchor="cursor" icon-size="40" labels="hover|always"/>`.

### 3.5 `free` (later)

Tiles placed by hand on a panel of a chosen size: `cell="3,1"` (column, row)
plus `tile` per node, written by a drag-and-drop editor in `y-pop-config`.
Without that editor it isn't worth shipping. Tiles without a `cell` take the
first free cells, so a new item never disappears. Titled separators can carry
a `cell` too, to label areas.

### 3.6 Layout hints are per presenter

`slot`, `tile`, `hint` and `cell` each belong to one presenter. A node can
carry several, so the same group can be a radial on one hotkey and a grid on
another. A hint the current presenter doesn't use is **silently ignored**. A
wrong value (`slot="up"`, `cell="a,b"`) is a non-fatal warning.

### 3.7 Skin

Boards and search use the active theme and `<skin>`: `back`/`text` for the
panel, `hover-*` for selection, the `disabled-text-color` slot for second
lines and headers, `<border>` for the outline. Proposed additions on
`<skin>`: `tile-radius` (default from `<border round>`) and `panel-alpha`
(0–255). A blurred background is not proposed: blur doesn't combine with
`UpdateLayeredWindow`.

---

## 4. Build order

Search has its own step-by-step plan: [`search-plan.md`](search-plan.md),
and it is built **first**. It walks `ConfigTree` into its own index and
opens its own window, so it needs neither `MenuLayout` nor the `Presenter`
trait; Reveal uses the native popup that exists today.

1. **Search** (§2), following `search-plan.md`: `<search>` configuration,
   core (§2.12), text input and Canvas primitives, the window, result
   actions, history/`<recent>`/sections, `<menus>`, prefixes and the hotkey
   check, `<start-menu>` (a default section) with "Add to a menu", then
   search-only groups and folders. Large icons are part of it (plan
   phase 3).
2. **`MenuLayout`** (§1.2), with the native popup moved onto it first so
   nothing regresses.
3. **`Presenter` host + `list`** (roadmap step 6), with type-to-search on
   top of the search core.
4. **`radial`** with the numpad (§3.2). Ring first, then keypad shape, pages,
   and the action ring.
5. **`grid`** with the search layer and hint letters, then **`strip`**.
6. **`free`** with the `y-pop-config` editor.
7. **Plugins** (§2.11), later, once search has been used for a while: the
   worker thread and `wasmi` host, the ABI and the `y-pop-plugin` crate,
   then the three example plugins.

Search comes before the boards: it is useful with any presenter, including
the native menu, and the boards' typing depends on it.

---

## 5. Open questions

1. **5's default**: `center` (proposed), `back`, or `repeat`? And should a
   nested group's center be its `default="true"` item, or always "back"?
2. **0's default**: `actions` (proposed) or `search`? Should 0 be
   tap-then-digit (proposed) or hold-while-pressing?
2b. **Radial pages**: "More slot" only (recommended), `<page>` with wheel and
   tabs as well, or neither?
2c. **Tray menus section**: `columns` (proposed) or `a-z` as the default?
   Should `a-z` include browsed folders' files too, or stay apps only?
2d. **Plugin capabilities**: is the `allow` list in §2.11 the right first
   set? (Not urgent: plugins come later.)
3. **Expert mode** for the radial (no drawing if the digits come fast
   enough): wanted?
4. **`keys="qwe"`**: worth supporting, or is the digit row enough for
   keyboards without a numpad?
6. **Reveal with the native popup** can only open the right submenu, not
   highlight the row. Acceptable, or should Reveal always use an overlay
   `list` even when `popup="native"`?
7. **Edit**: is starting `y-pop-config` on an entry (`--select`) the right
   handoff, or should Edit open `y-pop.xml` in a text editor at that line
   when `y-pop-config` isn't installed?
8. **Grid typing default**: `search` (proposed) or `hints`?
9. **Names**: `grid`, `strip`, `free`, `<page>`, `five`/`zero` as attribute
   names (the last two are short but odd; `center-key`/`zero-key` are
   alternatives).
