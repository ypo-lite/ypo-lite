# y-pop — user manual

y-pop is a tray utility: an icon in the notification area whose menu launches
your programs, and a **search window** that finds them by typing. Everything
is configured in one file, `y-pop.xml`.

This manual describes **what works today**. Each section ends, where it
matters, with what is not there yet. The design behind it is in
`overlay-modes.md` (the search spec) and `search-plan.md` (the build plan);
this page is for using it. It grows with each feature.

- [The configuration file](#the-configuration-file)
- [The search window](#the-search-window)
- [Actions](#actions)
- [History and Recent](#history-and-recent)
- [Prefixes](#prefixes)
- [Sections](#sections)
- [Collapsed mode](#collapsed-mode)
- [Configuring the search](#configuring-the-search)
- [Not there yet](#not-there-yet)
- [Troubleshooting](#troubleshooting)

---

## The configuration file

`y-pop.xml` lives in `%APPDATA%\Ypo-Lite\`. y-pop reads it at start-up and
again when `y-pop-config` saves it.

A mistake in the XML itself (a tag left open) stops the file from loading, and
the message gives the line. Anything else that is recognised but wrong — an
unknown attribute, a shortcut that cannot work — is only a **warning**: a
balloon at start-up tells you, the rest of the file still loads, and the
details are in the log.

A minimal file:

```xml
<y-pop>
    <search hotkey="Alt+Space"/>
    <group name="Work">
        <app name="Notepad" exe="%win%\notepad.exe"/>
    </group>
</y-pop>
```

Each top-level element is a **tray icon**. A group gives the icon a menu; the
menu is the group's contents.

---

## The search window

Press the search hotkey (`hotkey` on `<search>`, e.g. `Alt+Space`), or choose
**Open Search** in the right-click menu of a tray icon. A box opens in the
upper part of the screen; type to search, and the results appear under it.

### What is found

Everything in your tray menus (the apps, groups and commands in `y-pop.xml`),
plus whatever `<search>` adds on its own ([search-only items](#search-only-items)).
A word matches the **name**, the **keywords**, the **program's file name**,
the **path of groups** it sits in, and the **tooltip**; with several words,
each must match something. Matching is fuzzy: a name's word starts, its initials (`out` finds *Open Utility Tool*) or letters spread
across the name, best kind of match first.

Each result shows its icon, its name (the letters you typed emphasised), a
second line saying where it comes from, and its hotkey if it has one.

### Keys

| Key | Does |
| --- | --- |
| `↑` `↓` | move over the results (headers are skipped) |
| `PgUp` `PgDn` | move five results |
| `Enter` | open the selected result (see [Actions](#actions)) |
| `Alt+1` … `Alt+9` | open the *n*-th result at once. The rows show the number. |
| `Tab` (`Enter`, `→` on a section row) | go **inside** the selected group or section: it becomes a chip in the box and the search looks only there |
| `Shift+Tab`, `←` or `Backspace` on an empty box | leave the innermost group or section |
| `Esc` | clear the text; then leave the groups; then close |
| `Ctrl+C` | copy the selected text of the box; with none selected, copy the **path** of the selected result |
| `Ctrl+V`, `Ctrl+X`, `Ctrl+A` | paste, cut, select all (in the box) |
| `Ctrl+←` `Ctrl+→`, `Ctrl+Backspace`, `Ctrl+Del` | by word |
| `Alt+Enter`, `Shift+F10`, the Menu key | the [action menu](#the-action-menu) |
| `Ctrl+Del` | forget the selected row of Recent (in Windows too) |

With the mouse: hovering selects a row, a click opens it, a right click opens
the action menu, the wheel scrolls. Clicking outside the window, or on another
window, closes it.

The window sits on the monitor with the cursor, scales with its DPI, and takes
the colours of your tray menu's `theme`: `default`, `flat`, `dark`, `graphite`,
`midnight`, `sand` or `auto`, or Windows' light or dark setting with `native`.

To give it colours of its own, put a `theme` and/or a `<skin>` on `<search>`
(same attributes as the menus' `<skin>`; **Skin → Search window** in
y-pop-config):

```xml
<y-pop theme="flat">
    <skin back-color="ebebeb"/>            <!-- the tray menus -->
    <search theme="dark">                  <!-- the search window -->
        <skin hover-back-color="2563eb"/>  <!-- optional -->
    </search>
</y-pop>
```

The window's look never mixes with the menus': a `theme` on `<search>` drops
the menus' `<skin>`, and a `<skin>` on `<search>` replaces theirs (on the
menus' theme unless `<search>` names one). With neither, it follows the menus.

The window takes these from a skin:

| In `<skin>` | In the search window |
|---|---|
| `back-color`, `text-color` | The panel and its text. |
| `hover-back-color`, `hover-text-color` | The selected row, always a solid colour. |
| `disabled-text-color` | Second lines, headings and the footer's hints. |
| `<border>` | The window's outline and the radius of its corners. |

`padding`, `margin`, `edge`, `separator`, `<background-gradient>` and
`<icon-gradient>` are about menu rows. In the `<skin>` of `<search>` they do
nothing and y-pop warns about them.

---

## Actions

An **action** is something you can do to a result. Which ones apply depends on
the result:

| Action | Menu label | Applies to |
| --- | --- | --- |
| `open` | Open | everything. On a group it goes inside it, like `Tab`. |
| `open-toggle-history` | Open without history, or Open and add to history | apps and files, unless `history="false"` |
| `run-as-admin` | Run as administrator | programs (not ones already `elevate="true"`, not Store apps) |
| `show-in-folder` | Show in folder | programs and folders that are files |
| `copy-path` | Copy path | programs and folders (a Store app copies its `shell:AppsFolder\…` address) |
| `reveal` | Reveal in menu | items of the tray menus |
| `edit` | Edit in settings | items of the tray menus and search-only items |
| `add-to-menu` | Add to a menu | Start menu items *(not built yet)* |
| `forget` | Forget | the rows of Recent |

An action that does not apply is simply not offered, and its shortcut does
nothing.

There are three ways to reach one, and each is configurable
([below](#actions-1)):

1. its **shortcut**, from the result list;
2. its **button**, on the selected row;
3. the **action menu**.

Defaults: `Enter` opens, `Ctrl+R` reveals, `Ctrl+E` edits, `Ctrl+Del` forgets;
the others have no shortcut. Reveal and Edit have a button.

### The action menu

`Alt+Enter`, `Shift+F10` or the Menu key opens it on the selected row; so does
a right click on any row. It lists the actions that apply, each with its
shortcut on the right.

- `↑` `↓` move (wrapping), `Enter` does the lit one, a click does the one
  clicked.
- An action's own shortcut works inside it too.
- `Esc`, the key that opened it, a click anywhere else, or the wheel closes it.
- Typing waits while it is open.

It opens under its row, at the right edge, or above the row if there is no
room below.

### The row buttons

The selected row shows small buttons for the actions configured with
`button="true"`, and a **⋯** at its end that opens the action menu. Buttons
use short names (Admin, Folder, Copy, Reveal, Edit, Add, Forget). If they would
take more than half the row, the last ones are left out — they are still in
the menu; the **⋯** is always there.

`row-actions="false"` on `<search>` removes all of them. (The menu is still
reachable with the keyboard and a right click.)

### Reveal in menu

`Ctrl+R` (or the button, or the menu) closes the search and opens the tray
menu that holds the result, **at its tray icon**, with the submenus opened down
to the item and the item highlighted.

This is done by pressing the arrow keys in the menu for you, so it is *best
effort*: it counts the items above the target, which is exact for the menus
y-pop builds, but if the menu looks different from what y-pop expects (for
example a titled separator in a native menu behaves differently from a plain
one) the highlight may land on a neighbour. If the tray icon's position is
unknown (it is in the overflow area), the menu opens at the cursor.
Search-only items are in no tray menu, so there is nothing to reveal: the
action is not offered for them.

### Edit

Opens `y-pop-config` on the item: its tray menu (or, for a search-only group or
folder, its own page) is shown, the groups above it are unfolded and it is
selected. If `y-pop-config` is already open, that window comes forward and goes
to the item.

`y-pop-config` also understands these on its command line, for a launcher or a
script: `--select <path>` (positions down the tree, `2/0/5`: the sixth entry of
the first group of the third menu; `s/1/0` for the first entry of the second
search-only group or folder; `search` for the Search page) and `--new-app` with `--name`, `--target`,
`--args`, `--icon` and `--dir` (adds an app to the open tray menu, filled in).

---

## History and Recent

The history is **Windows's own**, the one the Start menu's lists are made of:
apps and files you opened anywhere, from y-pop or not. y-pop keeps no list of
its own. (A `y-pop.history` left by an earlier version is no longer read, and
can be deleted.)

**Recent.** With nothing typed, the box lists the latest under **RECENT**,
newest first, each saying when (*2 min ago · C:\Docs*). The first row is already
selected, so *hotkey, then Enter* opens the last thing again. How many:
`<recent count="5"/>` in `<search>` (without a `<recent>` element there is no
list). Typing replaces the list; erasing the text brings it back. Inside a
group (after `Tab`) it is not shown.

A row for something in your tray menus or search-only groups *is* that item,
with its name, hotkey and actions. Anything else is listed only if the Start
menu knows it as an app, so installers and one-off programs stay out. Apps and
files that only the history knows are found by a plain search too.

**All recent.** If Windows remembers more than `count`, a last row says how many.
`↓` to it then `Enter`, `→` or `Tab`, or a click, opens the whole list (200 at
most) as a chip: scroll it, or type to filter it. `←` or `Backspace` on an empty
box comes back.

**What y-pop tells Windows.** What you launch from a tray menu, a hotkey or
search goes into that history, which takes the shell, so an app with no `shell`
attribute is launched through it. An app with **`shell="false"`** is launched
directly, as it asks, and is not recorded. (`shell-execute` on `<y-pop>` has no
say in this.) `start`/`exit` events and the Run button of `y-pop-config` do not
count.

**The other way, this once.** `Ctrl+Shift+Enter` in search opens the selected
app or file the other way round: *Open without history* for what is normally
recorded, *Open and add to history* for a `shell="false"` app, which then goes
through the shell for that launch. The action menu says which it is. The action
is `open-toggle-history`, so `<actions>` can rebind it. It is search only: the
tray menus and hotkeys always launch the normal way.

**Ranking.** What was launched often and lately rises a little in the results:
up to 60 points on a 0–1000 scale, so it settles ties and near-ties but never
puts a vague match above an exact one. It is worked out once, when the window
opens, so the results do not shuffle while you type.

**Forgetting.** `Ctrl+Del` (or *Forget* in the action menu or its button) takes
the selected row off the list **and out of Windows's history**, like *Remove
from list* in the Start menu. The file or app itself is not touched. It is
offered only on rows of Recent and All recent. To clear everything, use the
Windows setting; y-pop has no command for it.

**If the list stays short.** Windows has two switches, in Settings,
Personalization, Start: *Show most used apps* and *Show recently opened items*
(the second also decides whether Explorer shows recent files). When recent items
are off, Windows records no file, from y-pop or anything else, so none can be
listed; the *All recent* row says so. y-pop does not keep them behind your back.

`history="false"` on `<search>` turns all of this off: Windows's history is not
read, and launches are not reported to it.

---

## Prefixes

The **first character** of the box can pick what to search. Under the sections
the empty box has a line of clickable chips — `> commands`, `# hotkeys`,
`/ files`, `? help` — and typing the character does the same. A prefix only counts in the
outermost box: inside a section or a group, `>` is just text. Erase the
character (or press `Esc`) to come back.

### `>` — y-pop commands

| Command | Does |
| --- | --- |
| Reload configuration | reads `y-pop.xml` again |
| Open settings | opens `y-pop-config` |
| Edit y-pop.xml | opens the file in its editor |
| Open configuration folder | shows the folder in Explorer |
| Start with Windows | turns running y-pop at logon on or off (the row says which) |
| About, Quit | what they say |

Type to filter: `>rel` finds *Reload*, and so do other words (`>restart`).
`Enter` runs the selected one. *Refresh the Start menu* and *Reload plugins*
come with the Start menu and the plugins.

### `#` — hotkeys

`#` alone lists every hotkey of the file: the search hotkey first, then each
item that has one (tray icons included). `Enter` launches what the key opens.
Type after it to narrow: a name (`#notes`), or part of a combination in any
order (`#shift+ctrl` lists the hotkeys that hold both).

**Is a key free?** Type a *whole* combination that nothing in the file uses —
`#ctrl+alt+x` — and a row tells you. y-pop asks Windows by registering the key
on the search window for a moment and letting go at once:

- **"Ctrl+Alt+X is free"**: no other program holds it. `Enter` opens
  `y-pop-config`, to assign it.
- **"Win+E is taken by File Explorer (Windows)"**: refused, and Windows keeps it
  for that (y-pop knows a table of about twenty such keys).
- **"Ctrl+Alt+X is taken"**: another program holds it; Windows does not say
  which. `override="true"` on a `<hotkey>` could still claim it with the
  low-level hook.

A combination the file already uses is just found, not checked.

### `/` — files in browsed folders

`/` searches the files and sub-folders inside every browsed folder
(`browse="true"`), the tray menus' and the ones only in `<search>`. That is the
only place they are searched, so a plain search never waits for a disk.

The folders are listed the first time you type `/` after opening search, in the
background: results join the list as they arrive, without moving your selection.
They follow the folder's `filter` and `depth`, as its tray menu does. Type to
narrow, by the file's name or the browsed folder's (`/scripts dep`). `Enter`
opens what is selected, as the tray menu would: a sub-folder opens in Explorer.
The action menu has **Show in folder** and **Copy path**.

A note at the end of the list says when something was left out:

- **Network folders** (`\\server\share`) are skipped, unless the folder has
  `index="true"`. A drive letter mapped to a share is not recognised as one, and
  is listed. Files on a network folder show no icon.
- **At most 5000** files are listed per opening.

```xml
<app name="Team share" exe="\\server\share\docs" browse="true" index="true"/>
```

### `?` — the prefixes

Lists the prefixes with what each does; `Enter` types the selected one.

---

## Sections

Under Recent, the empty box lists your **sections** under **BROWSE**, one row
each with the number of apps it holds and a ›:

- **Tray menus** — every tray menu (`<menus>`, always there; it comes first
  unless you place it);
- each **search-only group** you wrote in `<search>`.

`Enter`, `→`, `Tab` or a click on a row goes **inside**: the section becomes a
chip in the box, the list shows what is in it, and typing searches only there.
`←` or `Backspace` on an empty box comes back to the section you left. A section
row has no actions.

The Start menu will become an A–Z list and a folder will list its files, as
those parts get built. A section with nothing to open yet (the Start menu, a
browsed folder, a plugin) is not listed.

### Browsing the tray menus

*Tray menus* opens as **columns** by default (see [`browse`](#the-tray-menus-section-menus) for the other views): the tray menus on the left, what
the selected one holds next to it, and so on, with the way you came above them.
Three show at a time.

| Key | In the columns |
| --- | --- |
| `↑` `↓` `PgUp` `PgDn` `Home` `End` | move in the column |
| `→`, `Tab` | go into the selected group or browsed folder |
| `Enter` | the same on a group or folder; opens anything else |
| `←`, `Backspace`, `Shift+Tab` | back one column; from the first, out of the section |
| `Esc` | out of the section, from any column |
| typing | search the tray menus; erase the text to find the columns as you left them |

The rest of the keys work as on any result: `Ctrl+R`, `Ctrl+E`, the action menu.

- An **app** shows its **details** in the last column: what it runs, where it is,
  its hotkey and tooltip, and buttons for **Open**, **Reveal** and **Edit**.
- A **group** shows what it holds. A **browsed folder** shows its details and is
  not read until you go in with `→`, so walking past a slow folder costs nothing;
  it says `Listing…` until its files arrive. To open the folder itself in
  Explorer, use its **Open** button or the action menu.
- With the **mouse**: a click selects, and goes in if it is a group or a folder.
  An app is opened with its **Open** button. A click in a column to the left
  comes back to it, a right click opens the action menu, and the wheel scrolls the
  column that has the selection.

To open it as an alphabetical list instead, or to limit it to some tray menus, see
[The tray menus section: `<menus>`](#the-tray-menus-section-menus).

---

## Collapsed mode

`<search collapsed="true">` is for a bare launcher box: the window opens as **the
input row alone**, with a small **⌄** at its right end, one row high and
without the footer of key hints.

- `↓` (or a click on **⌄**) unfolds the list — Recent — with the first row
  selected. `↑` on that first row folds it again.
- Typing shows the results as usual; erasing the text folds the list again.
  (Going back out of a group with `Backspace` does not: you asked to see it.)
- `Esc` clears the text — which folds the list — or else leaves a group, or
  else folds the list, or else closes the window.
- `Enter` on the folded box does nothing: there is no selection to act on. So
  collapsed mode gives up "hotkey, then Enter relaunches the last thing"; press
  `↓` first.

---

## Configuring the search

All of it is inside the `<search>` element, which goes at the top level of
`y-pop.xml`. Only the first `<search>` is read.

```xml
<search hotkey="Alt+Space" results="sections" row-actions="true">
    <recent count="5"/>
    <menus browse="columns"/>
    <actions menu-key="Alt+Enter" quick-pick="true">
        <action do="open" key="Enter"/>
        <action do="run-as-admin" key="Ctrl+Enter" button="true"/>
        <action do="show-in-folder" key="Ctrl+Shift+Enter"/>
        <action do="copy-path" button="true"/>
        <action do="reveal" key="Ctrl+R" button="true"/>
        <action do="edit" key="Ctrl+E" button="true"/>
    </actions>
    <group name="Admin tools">
        <app name="Registry Editor" exe="%win%\regedit.exe"/>
    </group>
</search>
```

### `<search>` attributes

| Attribute | Default | Meaning |
| --- | --- | --- |
| `hotkey` | none | the key that opens the search, written like a `<hotkey>` (`Alt+Space`, `Ctrl+Win+F`…). Add `override="true"` to take it from another program if need be. |
| `results` | `sections` | `sections`: the best hit, then groups of at most five. `flat`: one list in score order. |
| `row-actions` | `true` | `false` hides the buttons of the selected row. |
| `history` | `true` | `false` neither remembers launches nor lists them nor lets them rank results (see [History](#history-and-recent)) |
| `collapsed` | `false` | `true` opens the box as its input row alone, with a ⌄ button (see [Collapsed mode](#collapsed-mode)) |

### Which sections exist

The children of `<search>` decide what the empty box lists, and in what order:

| In `<search>` | You get |
| --- | --- |
| nothing, or only `<actions>` | the defaults: `<recent/>`, `<menus/>`, `<start-menu/>` |
| any other child | **exactly what you wrote**, plus `<menus/>` if you left it out (after `<recent>`, or first) |

> **Watch out:** the moment you write one section, the defaults are off. A
> `<search>` holding only `<menus include="Work"/>`, or only a `<group>`, has **no
> Recent**. Write `<recent/>` too if you want it.

```xml
<!-- No Recent: only <menus> was written. -->
<search hotkey="Alt+Space">
    <menus include="Work"/>
</search>

<!-- Recent kept, then the tray menus. -->
<search hotkey="Alt+Space">
    <recent/>
    <menus include="Work"/>
    <start-menu/>
</search>
```

`<start-menu/>` does nothing yet, but writing it now keeps the Start menu once it
exists.

### The tray menus section: `<menus>`

`<menus>` is about the **Tray menus** section: how it opens, and which tray menus
count. It changes nothing in the tray itself: your icons, their menus and their
hotkeys stay as they are.

| Attribute | Values | Default | Meaning |
| --- | --- | --- | --- |
| `browse` | `columns`, `a-z`, `list` | `columns` | how the section looks when opened |
| `include` | tray menu names, separated by `;` | every tray menu | which tray menus are in the section and in the plain search |

Only the first `<menus>` is read.

**`browse`: columns or A–Z.** It only changes what you see after opening the
section with nothing typed. Typing inside it gives the same results either way.

| | `browse="columns"` | `browse="a-z"` | `browse="list"` |
| --- | --- | --- | --- |
| Shows | the menus as you built them, one level per column | every app in one list, under letter headers | the tray icons, one row each, with their second line |
| Groups | yes, you walk through them | no: their apps are listed directly | yes: `Enter` or `Tab` goes in, as a chip, and shows one level as a list |
| Browsed folders and their files | yes, listed when you go in | no | the folder is a row, `Enter` opens it in Explorer; its files are not listed |
| About, Quit, Settings | yes | no | yes |
| Good for | "it's somewhere under Work" | "I know its name, not where I put it" | one thing at a time, in full-width rows with their actions |

```xml
<menus browse="a-z"/>
```

**`include`: which tray menus count.** Each name is the `name` of a **top-level**
element of the file, that is, of a tray icon. A group inside a menu is not one.
Case, accents and a menu's `&` marker do not matter, and spaces around a `;` are
ignored. `;` is the separator because a name may contain spaces.

```xml
<search hotkey="Alt+Space">
    <recent/>
    <menus include="Work;Admin tools"/>
</search>
<group name="Work">…</group>
<group name="Admin tools">…</group>
<group name="Games">…</group>   <!-- left out of the search -->
```

What a left-out tray menu, *Games* here, still does:

| | Left-out menu |
| --- | --- |
| Its tray icon, menu and hotkeys | **work as usual** |
| The *Tray menus* section (columns or A–Z) and its app count | not there |
| Plain search | **not found** |
| A search inside *Tray menus* | not found |
| `#` hotkeys | listed: the key is still in use |
| `/` files | its browsed folders are still searched |
| Recent | a launch of it is still listed: that list is Windows's |

Without `include`, or with `include=""`, every tray menu counts.

A name that matches no tray menu is a warning at start-up, and the other names
still apply. If **none** of them match, every tray menu is left out: the section
disappears and a plain search finds nothing from your menus, so read that warning.

`browse` and `include` work together: `<menus browse="a-z" include="Work"/>`. In
`y-pop-config`, the **Search** page has both: use the Edit button of the *Tray
menus* row, choose how it opens, and click the tray menus to count. Renaming a
tray menu there keeps it in `include`, and deleting one takes it out. A menu with
a `;` in its name can't be listed: it counts only while every menu does. After
editing the file by hand, run `>Reload configuration`.

### Browsed folders: `browse="true"`

Not to be confused with `browse` on `<menus>`: the same word for two unrelated
settings. `browse="true"` on `<menus>` is a warning, and the section opens as
columns. This one goes on an **`<app>`**, and turns it into a folder whose content
is listed:

```xml
<app name="Scripts" exe="%mydocs%\Scripts" browse="true" filter="ps1;bat" depth="1"/>
```

| Attribute | Default | Meaning |
| --- | --- | --- |
| `browse="true"` | off | `exe` is a folder: list what is in it |
| `filter` | every file | the extensions to list, separated by `;` or `,` (`ps1`, `*.ps1` and `.ps1` are the same). Folders are not filtered. |
| `depth` | `0` | how many levels of sub-folders you can go into. `0` shows files only, no sub-folders. |
| `open-folder` | `max-depth` | what a tray menu does for opening a folder itself: `all` adds an entry in every submenu, `max-depth` shows the folders at the last level as items that open in Explorer, `never` leaves those out |
| `index="true"` | off | lets `/` list this folder even on a network share (`\\…`) |

Where a browsed folder shows, and what each setting does there:

| | Tray menu | `/` | Columns of *Tray menus* |
| --- | --- | --- | --- |
| Listed | when its submenu opens | the first `/` of an opening | when you go in with `→` |
| `filter`, `depth` | apply | apply | apply |
| `open-folder` | applies | `max-depth`, `never` apply | `max-depth`, `never` apply. `all` adds nothing: the folder has its **Open** button. |
| Network folder | listed | skipped without `index="true"` | listed, `index` or not, without icons |

### Search-only items

In `y-pop-config`, the Search page adds them (*Add group*, *Add folder*); each
then has its own page under *Only in search*, edited like a tray menu, minus what
only a tray icon has (hotkeys, default item, menu style).

Children of `<search>` that are not settings are results that are *only* in
the search, not in any tray menu:

```xml
<search hotkey="Alt+Space">
    <group name="System tools">
        <app name="Notepad" exe="%win%\notepad.exe"/>
        <app name="Services" exe="services.msc"/>
    </group>
    <app name="Scripts" exe="%mydocs%\Scripts" browse="true"/>
</search>
```

- A **`<group>`** holds apps (and groups) like in a tray menu. The group is a
  result too, and its apps carry its name as their path.
- An **`<app>` directly in `<search>` must be a browsed folder**
  (`browse="true"`, see [Browsed folders](#browsed-folders-browsetrue)). A plain
  `<app>` there is ignored with a warning — put it in a `<group>`. Its files are
  searched through `/`; it has no section of its own yet, and being outside the
  tray menus, it is not in the columns.

Any of these children means you are listing the sections yourself: see
[Which sections exist](#which-sections-exist).

### Actions

`<actions>` lists **exactly** the actions you want, in the order they appear
in the menu and as buttons. An action not written is gone from the menu and
its shortcut is free. `open` is always kept (added first, on `Enter`, if you
leave it out).

| Attribute of `<action>` | Meaning |
| --- | --- |
| `do` | one of `open`, `open-toggle-history`, `run-as-admin`, `show-in-folder`, `copy-path`, `reveal`, `edit`, `add-to-menu`, `forget` |
| `key` | its shortcut in the result list |
| `button="true"` | show it as a button on the selected row (default `false`) |

| Attribute of `<actions>` | Default | Meaning |
| --- | --- | --- |
| `menu-key` | `Alt+Enter` | the key that opens the action menu |
| `quick-pick` | `true` | `false` turns off `Alt+1…9` and the numbers on the rows |

**Writing a shortcut.** Modifiers and a key joined by `+`: `Ctrl+Shift+Enter`.
Usable: a letter or digit with `Ctrl` or `Alt`; `Enter` alone or with any
modifiers; `F1`…`F12`; `Ctrl+Del`. Refused, with a warning, because they would
get in the way of typing: a bare letter, digit or `Del`; `Ctrl+A`, `Ctrl+C`,
`Ctrl+V`, `Ctrl+X`; `Alt+1`…`Alt+9`; `Alt+F4`; the arrows, `Esc`, `Tab` and
the like. Two actions on the same key: the second loses it, with a warning.
An action keeps working without a key it was refused.

### Tray menu items

Not search settings, but they feed it: the `name`, `keywords`, `tooltip`,
`exe` and `hotkey` of every item in `y-pop.xml` are searchable. Add
`keywords="…"` to an item to make it findable under other words — in `y-pop-config`, the **Keywords** box of the item. A browsed folder's **Search inside it with /** switch is `index="true"`.

---

## Not there yet

Written in the spec, parsed from the file where it makes sense, but not doing
anything yet:

- **`<start-menu>`**: Start menu apps are not searched yet;
- **hiding a section** while still searching it (`browse="false"` and
  `global="false"` in the spec): these are not read, and only warn;
- **plugins** (`<plugin>` is kept but not run);
- **Add to a menu**;
- **Edit** selecting the item in `y-pop-config`.

---

## Troubleshooting

**Something I put in `<search>` is missing.** Look at the start-up balloon, or
the log: a warning names the line. The usual cause is an `<app>` directly in
`<search>` (see [Search-only items](#search-only-items)).

**The search hotkey does nothing.** Another program owns it. The balloon at
start-up says so. Choose another combination, or add `override="true"`.

**A shortcut I wrote in `<actions>` does nothing.** It was refused (see the
list above) — the balloon says why — or the action does not apply to the
selected result (`run-as-admin` on a group, say).

**Reveal highlights the wrong item, or none.** See
[Reveal in menu](#reveal-in-menu); open an issue with the menu's layout.
