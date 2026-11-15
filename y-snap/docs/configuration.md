# y-snap — configuration reference

Every setting of `y-snap.xml`. For how to use y-snap, with the defaults, common
changes and troubleshooting, see the [user manual](user-manual.md).

- [The configuration file](#the-configuration-file)
- [Keys](#keys)
- [Actions](#actions)
- [`<hotkeys>`](#hotkeys)
- [`<mouse>`](#mouse)
- [`<minbutton>`](#minbutton)
- [`<finder>`](#finder)
- [`<overlay>`](#overlay)

---

## The configuration file

y-snap reads `y-snap.xml`, in the first of these that exists:

1. the path after `-config` on the command line, `%VARIABLES%` expanded;
2. `y-snap.xml` next to `y-snap.exe` — a portable copy stays portable;
3. `%APPDATA%\Ypo-Lite\y-snap.xml`.

A malformed document stops the load and is reported with its line number.
Anything else — an unknown action, a key combination that does not parse, an
attribute nobody recognises — is a warning in a balloon, and the rest of the
file still loads. `rsc/y-snap.xml` is the default, and is a complete example.

## Keys

One attribute, written the way it reads: `key="Win+Alt+Left"`. Modifiers are
`Win`, `Ctrl`, `Alt`, `Shift`, in any order and any case. Key names are
letters and digits, `F1`-`F24`, `Numpad0`-`Numpad9`, `NumAdd`/`NumSub`/…,
`Esc`, `Space`, `Tab`, `Enter`, `Backspace`, `Insert`, `Delete`, `Home`,
`End`, `PageUp`, `PageDown`, the arrows, `PrintScreen`, `Pause`, `Menu`,
`CapsLock`, `ScrollLock`, `NumLock`, the punctuation keys as `OEM1`-`OEM8`
(or `Semicolon`, `Slash`, `Quote`, … on a US layout), the volume, media and
browser keys, and `0x…` for a raw virtual-key code.

## Actions

The same names work on a hotkey and on an overlay tile.

| | |
| --- | --- |
| `overlay` | show the action overlay for the target window |
| `reload` / `quit` | y-snap itself |
| `minimize` / `maximize` / `close` | the obvious ones; `maximize` restores a maximized window |
| `always-on-top` / `borderless` / `transparency` / `shade` / `hide-taskbar` | toggles |
| `center-cursor` | put the pointer in the middle of the window |
| `find-mouse` | show where the pointer is, see [`<finder>`](#finder). Needs no window; a `style` attribute picks which look |
| `iconize` / `traynize` | hide the window behind an icon of its own, or in the tray menu, see [Hidden windows](user-manual.md#hidden-windows) |
| `uniconize` / `uniconize-all` | bring back the window hidden last, or all of them, the latest first |
| `desktop-next` / `desktop-prev` / `desktop-new` / `pin-toggle` | virtual desktops: switch to the next or previous one (wrapping), create one, pin the window to all |
| `desktop-move` / `desktop-move-next` / `desktop-move-prev` | move the window to another desktop **and switch to it**. `desktop-move` goes to the `desktop` number; `-next`/`-prev` go one along from the current desktop, wrapping from the last to the first (and back), and take no number |
| `desktop-send` / `desktop-send-next` / `desktop-send-prev` | the same three, but **stay on the current desktop**: the window is sent away |
| `monitor-move-next` / `monitor-move-prev` | move the window to the next or previous monitor, **and take the pointer along** to the same relative spot. Monitors are ordered by position, left to right and then top to bottom, wrapping at the ends. Nothing happens with one monitor or a minimized window |
| `monitor-send-next` / `monitor-send-prev` | the same, but the **pointer stays where it is**. With `target="pointed"` it is then over whatever lies beneath the old place, so pressing again moves *that* window; `monitor-move-*` can be repeated to walk one window round |
| `desktop-list` / `desktop-send-list` | overlay only: expand into one tile per desktop, numbered from 1 and keyed `1`-`9`. `desktop-list` makes `desktop-move` tiles (they switch to that desktop), `desktop-send-list` makes `desktop-send` tiles (they stay put). The tile for the desktop you are on is drawn disabled |

## `<hotkeys>`

```xml
<hotkey key="Win+Escape" action="close" target="pointed"/>
```

| Attribute | |
| --- | --- |
| `key` | required |
| `action` | required |
| `target` | `focused` (default), or `pointed` for the window under the pointer, falling back to the focused one |
| `desktop` | for `desktop-move` and `desktop-send` only, 1-based |
| `style` | for `find-mouse` only: the name of the [`<finder>` style](#style) to play. Left out, or naming no style, plays the default one. On any other action it is a warning |
| `steal-from-shell` | `true` takes a combination Windows already owns, with a low-level keyboard hook |

`steal-from-shell` is off by default: a combination belonging to another
application belongs to that application. It is also the only setting here with
a cost — the hook runs on every keystroke on the machine.

## `<mouse>`

```xml
<mouse allow-maximized="true">
	<drag button="left" mod="Alt"/>
</mouse>
```

`<drag>` and `<resize>`, each with a `button` (`left`, `right` or
`middle`) and a `mod` (the modifier half of a hotkey, or `none`;
`Alt` if left out).

Drag and resize grab and release with the same button; the window under the
pointer at the moment of the press is the one they act on, same as
`target="pointed"`.

| Attribute of `<mouse>` | |
| --- | --- |
| `allow-maximized` | `true` lets drag and resize act on a **maximized** window like on any other, flag and all. `false` (the default, and not written to the file) leaves a maximized window where it is: the press is still claimed (the program gets no stray click, and the release is swallowed), a resize does nothing, and a drag that brings the pointer onto another monitor takes the window there, **still maximized**. Only the maximized flag counts: a borderless full-screen window drags as before. Anything other than `true`/`false` is a warning and means `false` |

Before this setting existed, a maximized window followed an `Alt` drag around, still
flagged as maximized. If you want that back, set `allow-maximized="true"`.

### `<shortcut>`

A modifier plus a mouse trigger fires one action, the way a hotkey does:

```xml
<shortcut button="middle"      mod="Win"  action="overlay"/>
<shortcut button="wheel-right" mod="none" action="pin-toggle"/>
<shortcut button="x1"          mod="Alt"  action="desktop-move" desktop="2"/>
<shortcut button="x1"          mod="Win"  action="monitor-move-prev"/>
<shortcut button="x2"          mod="Win"  action="monitor-move-next"/>
```

| Attribute | |
| --- | --- |
| `button` | required: `left`, `middle`, `right`, `wheel-left`, `wheel-right` (the wheel tilted), `x1` (back), `x2` (forward) |
| `mod` | required, unlike `<drag>`: `Win+Alt`…, or `none` |
| `action` | required, the same vocabulary as a hotkey, except `desktop-list` and `desktop-send-list` |
| `target` | `pointed` (the default, unlike a hotkey) or `focused` |
| `desktop` | for `desktop-move` and `desktop-send` only, 1-based |
| `style` | for `find-mouse` only, as on a hotkey |

* It fires on the press; the release is swallowed. A tilt has no release, so a
  held tilt fires once (events less than 500 ms apart are one hold) and fires
  again after a pause.
* Modifiers match exactly: a `Win` shortcut does not fire on `Win+Shift`.
* `target="pointed"` has no fallback: with nothing eligible under the pointer
  (the desktop, the taskbar) the click passes through untouched, and the
  focused window is never acted on instead.
* A plain left or right click (`mod="none"`) cannot be taken. A plain middle
  click, tilt or side button can, and replaces its normal job everywhere.
* A combination can mean only one thing: a shortcut duplicating another
  shortcut, or a `<drag>`/`<resize>`, is a warning and is dropped.
* Tilt and the side buttons must reach Windows as such: some mouse software
  remaps them to keystrokes, which no hook here can see.
* As with drag and resize, nothing reaches y-snap while an elevated window is
  in front.
* From a click, `overlay` asks Windows for the foreground with a synthetic key
  tap, and closes again if it still does not get it. `Win` is still held when
  it opens, so a tile's letter pressed before letting go is `Win`+letter, which
  belongs to the shell; clicking a tile works either way.

Wheel up/down is not a trigger; it belongs with the switcher
([`design/switcher-plan.md`](design/switcher-plan.md)). The settings app has a
Mouse shortcuts page for all of this.

## `<minbutton>`

```xml
<minbutton hover-delay="700" right-click-action="traynize">
	<action action="traynize" label="Traynize" icon="📤"/>
</minbutton>
```

Two behaviours on a window's minimize button, no hotkey needed for either:

* **Hover** shows a grid of one-click tiles right under the button once the
  pointer has sat still over it for `hover-delay` milliseconds (700 by
  default) — the delay resets on any movement still inside the button, so it
  does not flash open while the pointer is only passing through on its way
  to maximize or close. No `<overlay>` theming of its own; it reuses
  `<overlay>`'s own colours and corner radius. Each `<action>` takes
  `action`, `label` and `icon` — no `key`, no `group`; only actions that act
  on a single window are allowed (not `desktop-list`, `desktop-send-list` or `overlay` itself).
  `columns` is the number of tiles per row — `columns="2"` makes four tiles
  a 2×2 grid; left out (or `0`) puts them all on one row. The tile under the
  pointer is highlighted, and a tile whose toggle is on for the window
  (maximized, pinned, always on top…) shows the overlay's active colours.
  The popup is kept on the screen, so it moves left of the button's centre
  on a maximized window. An empty tile list turns the popup off without turning off the
  right-click action below.
* **A plain right-click** fires `right-click-action` directly — no popup.
  Defaults to `traynize`; `right-click-action="none"` turns it off.

Leave the whole section out to turn both off entirely. `<minbutton/>` with
no tiles is not the same thing: the section being present at all is what
turns `right-click-action`'s default on, so an empty one still fires
`traynize` on right-click with no popup to show — write
`right-click-action="none"` alongside it to have neither.

## `<finder>`

The mouse finder shows a short animation around the pointer, so that a lost one
can be found again, mostly on several monitors. It is played by the
[`find-mouse` action](#actions) (always available) and by **automatic
triggers**: a modifier tapped quickly, or the pointer shaken.

```xml
<finder guard="true">
	<exclude process="game"/>
	<style name="quick" shape="halo" auto-color="true" default="true">
		<tap key="ctrl"/>
		<shake mod="Ctrl"/>
	</style>
	<style name="radar" shape="beacon" size="220">
		<shake shapes="horizontal"/>
	</style>
</finder>
```

Leave the section out and only `find-mouse` works, with the built-in look. A
trigger exists by being declared: there is no `enabled` flag, and a style with no
`<tap>` or `<shake>` is played only by a hotkey, a shortcut or the tray menu.

### `<finder>` attributes

These are the defaults that every `<style>` inherits; each style may override any
of the look attributes.

| Attribute | Default | |
| --- | --- | --- |
| `shape` | `beacon` | `beacon`, `spotlight`, `crosshair`, `monitor`, `halo`, `negative` or `contrast`, see below |
| `duration` | `1000` | how long it plays, in milliseconds (300–5000) |
| `size` | `160` | the radius in device-independent pixels (40–600) |
| `color` | `#FFB000` | `#RRGGBB` |
| `auto-color` | `false` | darken the colour on a light backdrop and lighten it on a dark one, so it never melts into what is behind |
| `all-monitors` | `false` | show the indicator on every monitor, not only the pointer's, with one small overlay window per monitor while it plays. Only `monitor` honours it for now: each screen shows just the map panel (no border around the monitor, which keeps the surfaces small); the other shapes ignore it |
| `dim` | `60` | how dark `spotlight` makes the screen, in percent (0–100) |
| `recall` | `none` | `center` also moves the pointer to the middle of the focused application window before the indicator plays. With no such window the pointer stays where it is |
| `guard` | `true` | automatic triggers stay quiet while a full-screen window is on the pointer's monitor, see [When it stays quiet](#when-it-stays-quiet). `<finder>` only, not a style |

| Shape | |
| --- | --- |
| `beacon` | rings close in on the pointer, over a soft halo |
| `spotlight` | the monitor dims and a clear circle closes in on the pointer |
| `crosshair` | lines across the whole monitor, with a ring at the pointer |
| `monitor` | the pointer's monitor flashes its border, and a small map shows where all the monitors are |
| `halo` | a ring and glow that follow the pointer |
| `negative` | a lens that shows the screen behind the pointer as a negative |
| `contrast` | a ring drawn black or white, whichever stands out against what is behind it |

`halo`, `negative` and `contrast` are *persistent*: they stay four times `duration`
(ten seconds at most) and follow the pointer, instead of playing once. The effect
never shows up in a screen capture, and it is click-through: it takes no focus and
no click.

### `<exclude>`

`<exclude process="game"/>` — the executable's name, without the folder or `.exe`,
in any case. While that application has the focus the automatic triggers do
nothing, whatever the style. `find-mouse` still works. One element per
application, for all styles.

### `<style>`

| Attribute | |
| --- | --- |
| `name` | required, unique. What a `style="…"` on a hotkey or shortcut refers to |
| `default` | `true` marks the style `find-mouse` plays when it names none. The first style is the default otherwise |
| the look attributes | `shape`, `duration`, `size`, `color`, `auto-color`, `dim`, `recall`, `all-monitors`, each overriding `<finder>`'s |

A style holds any number of `<tap>` and `<shake>` elements. A second `default`, a
style without a name, a second style of the same name, and a trigger declared
twice (same tap, or same modifiers and kind of shake) are warnings: the first one
stays.

### `<tap>`

A modifier key pressed and released quickly, several times: `ctrl` twice is the
classic one. Anything else in between (another key, a mouse button, another
modifier) starts it over, and a key held down or auto-repeating is not a tap.

| Attribute | Default | |
| --- | --- | --- |
| `key` | required | `ctrl`, `shift`, `alt` or `win` |
| `side` | `any` | `left`, `right` or `any`: which of the two keys counts |
| `count` | `2` | how many taps (2–4). It fires on the release of the last one |
| `hold` | `200` | a press held longer than this (ms, 100–400) is not a tap |
| `gap` | `400` | two taps count together when their presses are at most this far apart (ms, 200–700) |

Windows has its own *Show location of pointer when I press the CTRL key* option
(Mouse settings, *Additional mouse settings*, *Pointer Options*). With it on, a
double `ctrl` tap plays Windows' effect and this one together: turn one of them
off, or tap another key.

### `<shake>`

The pointer moved quickly back and forth, or round and round.

| Attribute | Default | |
| --- | --- | --- |
| `mod` | `none` | modifiers that must be held while shaking, the same dialect as a [`<shortcut>`](#shortcut): `Ctrl`, `Win+Alt`…, or `none`. They must match exactly |
| `shapes` | `any` | `any`, or a list of `horizontal`, `vertical`, `circle` |
| `speed` | `3000`, `1200` with a `mod` | the slowest mean speed that is a shake, in pixels per second (400–8000) |
| `reversals` | `3`, `2` with a `mod` | direction changes a horizontal or vertical shake needs (1–6) |
| `cooldown` | `1000` | after a detection, nothing is detected for this long (ms, 300–3000) |
| `window` | `700` | how far back the movement is looked at (ms, 300–1500) |
| `segment` | `120` | the smallest stroke that counts as a back-and-forth (px, 40–300) |
| `dominance` | `1.5` | how much further the pointer must travel along the main axis than the other, for a shake to be horizontal or vertical (1.0–4.0) |
| `degrees` | `450` | how much smooth turning a circle needs (270–900) |
| `circle-speed` | `1500` | the slowest mean speed that is a circle (400–6000). A circle is slower than a shake. Not tuned yet |

Without a modifier the defaults are firm, so that moving the pointer quickly never
sets it off; with a modifier held you are already looking for the pointer, so a
gentler shake is enough. A shake with a mouse button down (a drag) never counts.
Distances and speeds are physical pixels, not scaled by the monitor, so a value
tuned on one screen is a starting point on another.

Events sent by another program (a macro, a remote-control tool) never trigger
anything.

### When it stays quiet

* **While the pointer is confined** to part of the desktop, which a game in play
  does (and a remote desktop or a virtual machine would), **all of y-snap is
  suspended**: no gestures, no mouse shortcuts, no hotkeys except `quit` and
  `reload`, no automatic trigger. The application owns the pointer, and nothing
  y-snap does with it would make sense. There is no setting for this.
* **A full-screen window on the pointer's monitor** keeps the automatic triggers
  quiet: a game in its menus, a full-screen video. "Full-screen" is a window that
  covers the monitor and has no title bar; the desktop and the taskbar do not
  count. A video full-screen on one monitor does not stop the finder from helping
  with a pointer lost on another. `guard="false"` turns this check off.
* **An excluded application** with the focus, see [`<exclude>`](#exclude).

An explicit `find-mouse` (a hotkey, a shortcut or the tray menu) ignores the last
two: whoever asks wants the answer. It does not ignore a confined pointer, since
y-snap is suspended then.

## `<overlay>`

One tile per action, chosen with a single key press.

| Attribute | Default | |
| --- | --- | --- |
| `theme` | `auto` | `light`, `dark`, or `auto` to follow Windows |
| `focus-border` `border-width` | `false` `2` | outline the target window while the overlay is open |
| `corner-radius` | `8` | |
| `alpha` | `235` | the popup's own background; tiles stay opaque |
| `columns` `tile-width` `tile-height` | `4` `88` `64` | the default grid |
| `key-font-size` `label-font-size` `icon-font` `icon-font-size` | `16` `9` `Segoe Fluent Icons` `22` | |
| `back-color` `text-color` `hover-*` `active-*` `disabled-*` `border-color` `panel-border-color` | from the theme | `#RGB`, `#RRGGBB` or `#RRGGBBAA` |

A theme only supplies defaults, so a file can pick one and change a single
colour rather than list all nine — ten, with `panel-border-color`, the thin
outline traced around the popup itself, separate from `border-color`, which
outlines the *target* window when `focus-border` is on.

`<group id="…" columns="…" tile-width="…" tile-height="…"/>` is a row laid out
on its own with its own grid — what puts the per-desktop tiles on one wide row
instead of in the main grid. A tile joins one with `group="…"`.

`<action key="…" action="…" label="…" label-on="…" icon="…" group="…"/>` is a
tile. `label-on` is shown in place of `label` while the toggle is on ("Pin" /
"Unpin"); `icon` is a glyph from the icon font, shown instead of the key
letter — the key itself still shows, as a small corner badge, so an iconed
tile does not hide which key fires it. Only `desktop-list` and `desktop-send-list` may omit `key`: it
is not a tile, it becomes tiles. A tile under the pointer is drawn in
`hover-back-color`/`hover-text-color`, whether reached by mouse or by the
overlay having just opened with the pointer already sitting over it.
