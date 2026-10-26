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
| `Tab` | go **inside** the selected group: it becomes a chip in the box and the search looks only there |
| `Shift+Tab`, `Backspace` on an empty box | leave the innermost group |
| `Esc` | clear the text; then leave the groups; then close |
| `Ctrl+C` | copy the selected text of the box; with none selected, copy the **path** of the selected result |
| `Ctrl+V`, `Ctrl+X`, `Ctrl+A` | paste, cut, select all (in the box) |
| `Ctrl+←` `Ctrl+→`, `Ctrl+Backspace`, `Ctrl+Del` | by word |
| `Alt+Enter`, `Shift+F10`, the Menu key | the [action menu](#the-action-menu) |
| `Ctrl+Del` | forget the selected row of Recent |

With the mouse: hovering selects a row, a click opens it, a right click opens
the action menu, the wheel scrolls. Clicking outside the window, or on another
window, closes it.

The window sits on the monitor with the cursor, scales with its DPI, and takes
the colours of your tray menu's `theme`.

---

## Actions

An **action** is something you can do to a result. Which ones apply depends on
the result:

| Action | Menu label | Applies to |
| --- | --- | --- |
| `open` | Open | everything. On a group it goes inside it, like `Tab`. |
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

Opens `y-pop-config`. It does not yet jump to the item.

---

## History and Recent

y-pop remembers what you launch from the search: `open` and `run-as-admin`
(copying a path, revealing or editing are not launches). It does two things
with it.

**Recent.** With nothing typed, the box lists your last launches under
**RECENT**, newest first, each saying when (*2 min ago · Work › Servers · …*).
The first row is already selected, so *hotkey, then Enter* relaunches the last
thing. How many: `<recent count="5"/>` in `<search>` (without a `<recent>`
element there is no list). Typing replaces the list; erasing the text brings
it back. Inside a group (after `Tab`) it is not shown.

**Ranking.** What you launch often and lately rises a little in the results:
up to 60 points on a 0–1000 scale, so it settles ties and near-ties but never
puts a vague match above an exact one. It is worked out once, when the window
opens, so the results do not shuffle while you type.

**Forgetting.** `Ctrl+Del` (or *Forget* in the action menu or its button) takes
the selected row off the list and out of the history. It is offered only on
rows of Recent.

The history is the file `y-pop.history` next to `y-pop.xml`, written when the
window closes: one line per item, at most 200. An item is known by the names of
the groups above it and its own name, so **renaming or moving an item starts
its history over**; an item that no longer exists is dropped. You can delete
the file at any time. `history="false"` on `<search>` turns all of this off: no
file is read or written.

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

### Search-only items

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
  (`browse="true"`). A plain `<app>` there is ignored with a warning — put it
  in a `<group>`.

### Actions

`<actions>` lists **exactly** the actions you want, in the order they appear
in the menu and as buttons. An action not written is gone from the menu and
its shortcut is free. `open` is always kept (added first, on `Enter`, if you
leave it out).

| Attribute of `<action>` | Meaning |
| --- | --- |
| `do` | one of `open`, `run-as-admin`, `show-in-folder`, `copy-path`, `reveal`, `edit`, `add-to-menu`, `forget` |
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
`keywords="…"` to an item to make it findable under other words.

---

## Not there yet

Written in the spec, parsed from the file where it makes sense, but not doing
anything yet:

- the **sections** of the empty box (`<menus>` and the others): below Recent it
  lists nothing yet;
- **`<start-menu>`**: Start menu apps are not searched yet;
- **prefixes** and the hotkey check;
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
