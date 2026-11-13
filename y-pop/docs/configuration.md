# y-pop — configuration reference

Every setting of `y-pop.xml` outside `<search>`: tray icons, items, hotkeys,
browsed folders, icons, variables, themes and skins. `<search>` is in the
[user manual](user-manual.md), which also explains how to use the search window.

- [The configuration file](#the-configuration-file)
- [`<y-pop>`](#y-pop)
- [Tray icons](#tray-icons)
- [Items](#items)
- [`<app>`](#app)
- [Hotkeys](#hotkeys)
- [Browsed folders](#browsed-folders)
- [Icons](#icons)
- [Variables](#variables)
- [Themes](#themes)
- [`<skin>`](#skin)
- [Not doing anything yet](#not-doing-anything-yet)

---

## The configuration file

y-pop reads `y-pop.xml`:

1. from the path after `-config` or `--config` on the command line, as written
   (`%VARIABLES%` are not expanded here, unlike in y-snap);
2. else next to `y-pop.exe` if there is one, so a portable copy stays portable;
3. else from `%APPDATA%\Ypo-Lite\y-pop.xml`.

`rsc/y-pop.xml` is the default the installer copies there. `portal.xml` files
from PortAL do not load: this is a new schema, not a compatible reader.

It is read at start-up, after a save in `y-pop-config`, and with
`>Reload configuration` in the search window. A reload that fails keeps the
running configuration, so a half-typed edit never takes your hotkeys away.
`start` and `exit` events do not fire again on a reload.

Anything recognised but wrong (an unknown attribute, a key combination that does
not parse) is a warning: a balloon lists up to six, and the rest of the file
still loads. Everything is also written to `y-pop.log`, next to `y-pop.xml`.

A file that is missing or malformed is different. y-pop keeps running with **no
tray icon and no hotkey**, and the reason, with the line number for malformed
XML, is only in `y-pop.log`. Starting y-pop again does nothing, as one is still
running: fix the file, end `y-pop.exe` in Task Manager and start it again.

## The shape of the file

```xml
<y-pop theme="auto">
	<variables>
		<var name="tools" value="%user%\tools"/>
	</variables>
	<skin hover-back-color="ff8800"/>
	<search hotkey="Alt+Space"/>

	<group name="Work">
		<hotkey key="Win+Y"/>
		<separator name="Editors"/>
		<app name="Notepad" exe="%win%\notepad.exe" tooltip="A blank page"/>
		<group name="Scripts">
			<app name="Backup" exe="%tools%\backup.bat" path="%tools%"/>
		</group>
		<app name="Documents" exe="%mydocs%" browse="true" depth="1"/>
		<separator/>
		<settings/>
		<quit/>
	</group>
</y-pop>
```

Values that are true or false take `true`, `yes`, `on`, `1`, or `false`, `no`,
`off`, `0`.

## `<y-pop>`

| Attribute | Default | |
| --- | --- | --- |
| `theme` | `native` | see [Themes](#themes) |
| `show-shortcuts` | `true` | show each item's hotkey at the right of its row |
| `shell-execute` `popup` `popup-style` | | no effect yet, see [below](#not-doing-anything-yet) |

## Tray icons

Every `<group>` and `<app>` directly under `<y-pop>` is a tray icon, and so are
`<about/>`, `<quit/>` and `<settings/>` there. `<variables>`, `<skin>` and
`<search>` are not. Keep `<separator/>` and `<break/>` inside a group: at the top
level each gets an icon that does nothing.

| | Group | Anything else |
| --- | --- | --- |
| Left click | opens its menu | does what the item does: an app launches |
| Right click | its menu, then the extras below | the extras alone |

The extras are **Open Search** when the file has a `<search>`, a **Configuration**
submenu (**Settings…**, **Edit y-pop.xml**, **Reload configuration** and **Start with
Windows**), then **About** and **Quit**, each unless that menu already holds one at
its top level (a `<settings/>` for **Configuration**). The same submenu is in the
tray menu of y-snap and y-coustic.

Every tray icon shows y-pop's own icon, whatever `icon` says; `icon` is for rows
in a menu. Its tooltip is its `name`.

## Items

Attributes all of them share, except `<break/>`:

| Attribute | |
| --- | --- |
| `name` | the text of the row. `<group>` and `<app>` warn without one; `<about/>`, `<quit/>` and `<settings/>` have their own. Write `&amp;` before a letter to make it the row's access key: `name="&amp;Notes"` |
| `icon` | see [Icons](#icons) |
| `tooltip` | shown under the row after half a second, on one line |
| `keywords` | extra words search finds the item by |

| Element | |
| --- | --- |
| `<group>` | a submenu, holding any of these. Also takes `show-shortcuts` |
| `<app>` | something to launch, see below |
| `<separator/>` | a rule, or with a `name` a title between rows. It shows no icon or tooltip |
| `<break/>` | starts a new column in the menu. Takes no attributes |
| `<about/>` | the *About* balloon |
| `<settings/>` | opens `y-pop-config`, which must sit next to `y-pop.exe`; without it, opens `y-pop.xml` |
| `<quit/>` | quits y-pop |

## `<app>`

```xml
<app name="Terminal" exe="%system%\cmd.exe" path="C:\" elevate="true"/>
<app name="Calculator" app-id="Microsoft.WindowsCalculator_8wekyb3d8bbwe!App" icon="@app"/>
<app name="Open clipboard file" exe="%win%\notepad.exe" param="%clipboard%"/>
```

| Attribute | |
| --- | --- |
| `exe` | what to run: a program, or a document, folder or address that Windows knows how to open |
| `app-id` | a Store app's id, instead of `exe`. With both, `app-id` wins and warns |
| `param` | the arguments |
| `path` | the working directory |
| `shell` | `true` always goes through the Windows shell, `false` never does, left out lets y-pop choose. See below |
| `elevate` | `true` runs as administrator, with the usual prompt |
| `events` | `start`, `exit` or `start,exit`: also run this when y-pop starts or quits |
| `show-shortcuts` | overrides the one on `<y-pop>` for this row only. On a `<group>` too, where it is not passed down to the items inside |
| `browse` `filter` `depth` `open-folder` `index` | see [Browsed folders](#browsed-folders) |

An `<app>` needs `exe` or `app-id`, or it is dropped with a warning.

### How a launch runs

What you launch from a menu, a hotkey or search goes through the Windows shell,
because that is the only way to record it in Windows's history, which search
reads. `events` launches are not recorded.

`shell="false"` starts the program directly and keeps it out of the history.
`history="false"` on `<search>` turns recording off for everything, and then
launches are direct by default too. A direct launch can only start a program, so
a document, folder or address must not have `shell="false"`, and under
`history="false"` it needs `shell="true"`.

`elevate="true"` always goes through the shell, as Windows offers it no other way.

## Hotkeys

One or more `<hotkey>` children on an item:

```xml
<app name="Notepad" exe="%win%\notepad.exe">
	<hotkey key="Win+N"/>
	<hotkey key="Ctrl+Alt+N" override="true"/>
</app>
```

| Attribute | |
| --- | --- |
| `key` | required. The same combinations as [y-snap's](../../y-snap/docs/configuration.md#keys): `Win+Alt+Left`, any order, any case |
| `override` | `true` takes the combination with a low-level keyboard hook, if another program holds it |

A hotkey on a group opens its menu at the pointer; on anything else it does what
a click on the row does. A hotkey on a browsed folder opens the folder itself.
A row shows the first hotkey only. The search window's hotkey is `hotkey` on
`<search>`, and a group inside `<search>` takes none.

A combination that is taken is reported in the start-up balloon. `override` is
off by default, because a combination belonging to another program belongs to
that program, and it is only tried after the normal registration has been
refused. `#ctrl+alt+x` in the search window tells you whether a key is free.

## Browsed folders

`browse="true"` makes an `<app>` a submenu listing the folder its `exe` names:

```xml
<app name="Scripts" exe="%mydocs%\Scripts" browse="true" filter="ps1;bat" depth="1"/>
```

| Attribute | Default | |
| --- | --- | --- |
| `filter` | every file | extensions, separated by `;` or `,`. `ps1`, `.ps1` and `*.ps1` are the same; `*.*` is everything. This is a list of extensions, so `notes-*.txt` matches nothing. Folders are not filtered |
| `depth` | `0` | how many levels of sub-folders become submenus. `0` lists files only. At most 200 |
| `open-folder` | `max-depth` | see below |
| `index` | `false` | lets search's `/` list this folder even on a network share |

`open-folder` decides how a folder is opened, rather than listed:

| | |
| --- | --- |
| `max-depth` | folders one level past `depth` are rows that open in Explorer |
| `all` | the same, and every submenu starts with an **Open folder** row |
| `never` | those folders are left out |

The folder is read when its submenu opens, so a new file shows up without a
reload, and a slow folder costs nothing until you go there. Folders come first,
then files, each alphabetically. Hidden files are skipped, and `.exe` and `.lnk`
lose their extension on the row. Each file launches with the `shell` and
`elevate` of the `<app>`.

The same folder also feeds search, through `/` and the columns of the *Tray
menus* section: [its table](user-manual.md#browsed-folders-browsetrue) says
what each setting does there.

## Icons

`icon` takes:

| | |
| --- | --- |
| `C:\path\file.ico` | a file: `.ico`, `.exe`, `.dll`, or any other file for the icon Windows shows for it |
| `file.dll,6` | the icon at that position in the file; a negative number picks it by resource id |
| `@name` | a built-in icon, below |
| `@app` | a Store app's own logo, with `app-id` |

Left out, an app shows the icon of its `exe`, and a group or browsed folder the
folder icon. About, Settings and Quit have their own.

Built-in icons, nearly all of them the shell's own, so they follow your version
of Windows: `@ypop`, `@about`, `@quit`, `@settings`, `@folder`,
`@folder-open`, `@program`, `@computer`, `@drive`, `@network-drive`, `@network`,
`@server`, `@world`, `@users`, `@find`, `@help`, `@key`, `@lock`, `@shield`,
`@warning`, `@error`, `@link`, `@share`, `@printer`, `@recycle-bin`, `@images`,
`@audio`, `@video`, `@zip`.

## Variables

`%name%` works in any text attribute, expanded once when the file loads. Your own
come first, then the built-in ones:

```xml
<variables>
	<var name="tools" value="%user%\tools"/>
	<var name="python" value="%tools%\python\python.exe"/>
</variables>
```

A `<var>` can use the built-in ones and any `<var>` above it. A later one with the
same name wins, and one named like a built-in replaces it. Names are
case-sensitive. A `%name%` that matches nothing stays as written, and Windows
environment variables are not read: `%windir%` means nothing here.

| | |
| --- | --- |
| `%pop%` | the folder of `y-pop.exe` |
| `%config%` | the path of `y-pop.xml` itself, not its folder |
| `%win%` `%system%` `%system86%` | Windows, `System32`, `SysWOW64` |
| `%programfiles%` `%programfiles86%` | |
| `%mydocs%` `%mymusic%` `%myvideo%` `%desktop%` | your folders |
| `%user%` | your profile |
| `%appdata%` | `AppData\Local` (Windows's own `%APPDATA%` is the next one) |
| `%appdata-r%` | `AppData\Roaming` |

Two are read at the moment of the launch instead, in `exe`, `param` and `path`:

| | |
| --- | --- |
| `%clipboard%` | the text on the clipboard, or the path of the first file copied |
| `%clipboards%` | the same, with every copied file, separated by spaces |

Neither adds quotes, so write `param="&quot;%clipboard%&quot;"` for a path that
may hold a space.

## Themes

`theme` on `<y-pop>` picks the look of the menus. The search window follows it
unless `<search>` has a theme or skin of its own
([details](user-manual.md#the-search-window)).

| | |
| --- | --- |
| `native` | plain Windows menus, drawn by Windows. The default |
| `default` | PortAL's own: light, with a blue gradient on the hovered row |
| `flat` | PortAL's own: light, no gradient |
| `dark` (or `black`) | PortAL's own: black, with a rule beside the icons on every row |
| `graphite` | neutral dark, a soft rounded hover |
| `midnight` | deep navy, a saturated blue hover |
| `sand` | warm paper, a golden hover |
| `auto` | `default` or `dark`, following Windows's light or dark setting each time a menu opens |

## `<skin>`

One `<skin>` directly under `<y-pop>` changes parts of the theme. What it does not
name keeps the theme's value. With no `theme`, or `theme="native"`, a skin is
painted on `auto`.

```xml
<skin back-color="1a1a1a" hover-back-color="ff8800" padding="2">
	<border size="1" round="8" color="3a3a3a"/>
	<background-gradient start="ff8800" end="cc5500" direction="v"/>
</skin>
```

| Attribute | |
| --- | --- |
| `back-color` `text-color` | idle rows |
| `hover-back-color` `hover-text-color` | the row under the pointer |
| `disabled-text-color` | rows that cannot be chosen, and shortcut hints |
| `padding` | pixels **between** rows |
| `margin` | pixels **within** a row, between its border and its content |
| `edge` | a vertical rule between the icons and the text, on `idle` rows, `hover` rows or `always` |
| `separator` | `after-icons` starts an untitled separator's line past the icons, `full` takes the whole row. A titled one always takes the row |

Colours are `RRGGBB`, `RRGGBBAA` or `RGB`, with or without `#`. `padding` and
`margin` are the reverse of their CSS meanings.

| Child | |
| --- | --- |
| `<border size="" round="" color=""/>` | the hovered row's outline and corner radius |
| `<background-gradient start="" end="" direction=""/>` | the hovered row's fill, replacing `hover-back-color` |
| `<icon-gradient start="" end="" direction=""/>` | behind the icon column, on idle rows. For one colour, make `start` and `end` the same |

Each child is all or nothing: one with an attribute missing warns and is dropped.
`direction` is optional, and takes `h` (the default) or `v`.

## Not doing anything yet

Read from the file, but doing nothing yet:

| | |
| --- | --- |
| `popup` `popup-style` | every menu is the native one. The overlay popup these choose is not built. `popup-style="runner"` and `"radial"` warn and fall back to `list`, which is not built either |
| `autoopen` on `<group>` | meant to open the menu when the pointer rests on the tray icon |
| `default` on an item | no item is treated as the default one |
| `shell-execute` on `<y-pop>` | has no effect on launches, see [How a launch runs](#how-a-launch-runs). Only the *Run* button of `y-pop-config` reads it |

Also not done: an edit made by hand is not noticed until you ask for a reload,
and icons load while the menu is built, so an icon on a slow network share holds
the menu up.
