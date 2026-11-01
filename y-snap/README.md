# y-snap

Window management by hotkey, mouse gesture and overlay. The Rust rewrite of
[HulK](http://glatigny.github.io/guipro/).

PowerToys' "Grab and Move" does a part of this. y-snap is the rest: minimize,
maximize, close, roll up, fade, strip the frame, hide from the taskbar, hide
behind a tray icon, pin to every virtual desktop or send to another one — from
the keyboard, or from one popup, on whichever window the pointer happens to be
over.

This page is the reference for `y-snap.xml`. For how to use y-snap, with the
defaults, common changes and troubleshooting, see the
[user manual](docs/user-manual.md).

## State

Working: the configuration file, the tray icon and menu, hotkey registration
(including reclaiming a combination the shell owns), the actions that act
directly on a window — minimize, maximize, close, always-on-top, borderless,
transparency, shade, hide-from-taskbar, center-cursor — the mouse hook (drag,
resize, mouse shortcuts on a button, side button or wheel tilt), the minimize-button popup, the action overlay (grid layout,
hover, click, key, `Escape`, click-outside), and the virtual desktop actions
(`desktop-next`/`-prev`/`-new`/`-move`, `pin-toggle`, `desktop-list`'s
per-desktop tiles) — see [`y-core`'s own `virtual_desktop`
module](../core/src/virtual_desktop.rs) for how the undocumented COM
interface behind them is handled across Windows versions.

Also working: hiding windows. `iconize` gives a window an icon of its own in
the notification area, `traynize` lists it in y-snap's tray menu; see
[Hidden windows](#hidden-windows) below. The tray menu also has an *About*
balloon (version, what is loaded, memory), and clicking that balloon opens the
settings app.

## The configuration file

y-snap reads `y-snap.xml`, in the first of these that exists:

1. the path after `-config` on the command line, `%VARIABLES%` expanded;
2. `y-snap.xml` next to `y-snap.exe` — a portable copy stays portable;
3. `%APPDATA%\Ypo-Lite\y-snap.xml`.

A malformed document stops the load and is reported with its line number.
Anything else — an unknown action, a key combination that does not parse, an
attribute nobody recognises — is a warning in a balloon, and the rest of the
file still loads. `rsc/y-snap.xml` is the default, and is a complete example.

### Keys

One attribute, written the way it reads: `key="Win+Alt+Left"`. Modifiers are
`Win`, `Ctrl`, `Alt`, `Shift`, in any order and any case. Key names are
letters and digits, `F1`-`F24`, `Numpad0`-`Numpad9`, `NumAdd`/`NumSub`/…,
`Esc`, `Space`, `Tab`, `Enter`, `Backspace`, `Insert`, `Delete`, `Home`,
`End`, `PageUp`, `PageDown`, the arrows, `PrintScreen`, `Pause`, `Menu`,
`CapsLock`, `ScrollLock`, `NumLock`, the punctuation keys as `OEM1`-`OEM8`
(or `Semicolon`, `Slash`, `Quote`, … on a US layout), the volume, media and
browser keys, and `0x…` for a raw virtual-key code.

### Actions

The same names work on a hotkey and on an overlay tile.

| | |
| --- | --- |
| `overlay` | show the action overlay for the target window |
| `reload` `quit` | y-snap itself |
| `minimize` `maximize` `close` | the obvious ones; `maximize` restores a maximized window |
| `always-on-top` `borderless` `transparency` `shade` `hide-taskbar` | toggles |
| `center-cursor` | put the pointer in the middle of the window |
| `iconize` `traynize` | hide the window behind an icon of its own, or in the tray menu |
| `uniconize` `uniconize-all` | bring back the window hidden last, or all of them |
| `desktop-next` `desktop-prev` `desktop-new` `pin-toggle` | virtual desktops: switch to the next or previous one (wrapping), create one, pin the window to all |
| `desktop-move` `desktop-move-next` `desktop-move-prev` | move the window to another desktop **and switch to it**. `desktop-move` goes to the `desktop` number; `-next`/`-prev` go one along from the current desktop, wrapping from the last to the first (and back), and take no number |
| `desktop-send` `desktop-send-next` `desktop-send-prev` | the same three, but **stay on the current desktop**: the window is sent away |
| `desktop-list` `desktop-send-list` | overlay only: expand into one tile per desktop, numbered from 1 and keyed `1`-`9`. `desktop-list` makes `desktop-move` tiles (they switch to that desktop), `desktop-send-list` makes `desktop-send` tiles (they stay put). The tile for the desktop you are on is drawn disabled |

### `<hotkeys>`

```xml
<hotkey key="Win+Escape" action="close" target="pointed"/>
```

| Attribute | |
| --- | --- |
| `key` | required |
| `action` | required |
| `target` | `focused` (default), or `pointed` for the window under the pointer, falling back to the focused one |
| `desktop` | for `desktop-move` and `desktop-send` only, 1-based |
| `steal-from-shell` | `true` takes a combination Windows already owns, with a low-level keyboard hook |

`steal-from-shell` is off by default: a combination belonging to another
application belongs to that application. It is also the only setting here with
a cost — the hook runs on every keystroke on the machine.

### `<mouse>`

```xml
<drag button="left" mod="Alt"/>
```

`<drag>` and `<resize>`, each with a `button` (`left`, `right` or
`middle`) and a `mod` (the modifier half of a hotkey, or `none`;
`Alt` if left out).

Both are driven by one `WH_MOUSE_LL` hook, installed on y-snap's own
thread rather than injected into other processes — see `mouse/`. Drag and
resize grab and release with the same button; the window under the pointer at
the moment of the press is the one they act on, same as `target="pointed"`.

#### `<shortcut>`

A modifier plus a mouse trigger fires one action, the way a hotkey does:

```xml
<shortcut button="middle"      mod="Win"  action="overlay"/>
<shortcut button="wheel-right" mod="none" action="pin-toggle"/>
<shortcut button="x1"          mod="Alt"  action="desktop-move" desktop="2"/>
```

| Attribute | |
| --- | --- |
| `button` | required: `left`, `middle`, `right`, `wheel-left`, `wheel-right` (the wheel tilted), `x1` (back), `x2` (forward) |
| `mod` | required, unlike `<drag>`: `Win+Alt`…, or `none` |
| `action` | required, the same vocabulary as a hotkey, except `desktop-list` and `desktop-send-list` |
| `target` | `pointed` (the default, unlike a hotkey) or `focused` |
| `desktop` | for `desktop-move` and `desktop-send` only, 1-based |

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
([`docs/design/switcher-plan.md`](docs/design/switcher-plan.md)). The settings
app has a Mouse shortcuts page for all of this. Design notes:
[`docs/design/mouse-shortcuts-plan.md`](docs/design/mouse-shortcuts-plan.md).

### `<minbutton>`

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

The button's own location is estimated from the window's caption metrics
rather than hit-tested against the window — see `mouse::min_button_rect`'s
doc comment for why: hit-testing another process's window from inside a
low-level hook risks the 300 ms timeout Windows enforces on one.

### `<overlay>`

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

## Hidden windows

`iconize` and `traynize` take the target window off the screen (minimized,
then hidden — so a maximized window comes back maximized — and its taskbar
button goes with it; other windows of the same program keep theirs).

| | |
| --- | --- |
| `iconize` | the window gets an icon of its own in the notification area: its own icon, its title as the tooltip. Click to bring it back; right-click for *Open* / *Close*, under the program's name |
| `traynize` | no icon of its own: the window is listed under **Hidden windows** at the top of y-snap's tray menu, with its icon. Click an item to bring it back |
| `uniconize` | bring back the window hidden last, of either kind |
| `uniconize-all` | bring back every hidden window, the latest first |

The tray menu also has **Restore all hidden windows**. At most 48 windows can
be hidden at once; one more is refused with a balloon. A window of an
elevated program is refused too, unless y-snap is elevated as well — Windows
would ignore the request without saying so.

y-snap checks every five seconds, only while something is hidden, and forgets
a window that was closed, or that its program showed again by itself. Quitting
y-snap brings every hidden window back. If y-snap is *killed*, hidden windows
stay hidden; `docs/design/iconize-traynize-plan.md` describes the opt-in cache file
planned for that.

## Layout

| | |
| --- | --- |
| `main.rs` | the tray app: the message loop, hotkey registration, dispatch |
| `hidden.rs` | the windows `iconize`/`traynize` hid: the registry, the tray icons, the purge |
| `about.rs` | the text of the *About* balloon, built to the balloon's size limit |
| `action.rs` | the action vocabulary, shared by hotkeys and the overlay |
| `config.rs` | reading `y-snap.xml` |
| `actions.rs` | action → Win32 call, in one place |
| `target.rs` | choosing the window, and refusing the shell's |
| `window.rs` | what gets done to a window, and how to undo it |
| `mouse/` | the low-level mouse hook: drag, resize (`gesture`), minimize-button hover (`hover`), mouse shortcuts (`shortcut`, a pure state machine) |
| `min_overlay.rs` | the minimize-button popup, and turning a click into an action |
| `overlay.rs` | the action overlay: tile grid, hover, click, key, focus-driven close |

Everything Win32 that is not specific to window management — the hidden
message window, the tray icon, menus, hotkey registration, the XML reader,
the overlay surface — is in [`y-core`](../core/README.md).
