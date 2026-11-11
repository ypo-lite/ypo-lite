# y-coustic — configuration reference

Every setting of `y-coustic.xml`. For the idea of it, see the
[user manual](user-manual.md).

## Where the file is

Tried in this order:

1. `-config <path>` on the command line (`%VARIABLES%` are expanded);
2. `y-coustic.xml` next to `y-coustic.exe`, for a portable copy;
3. `%APPDATA%\Ypo-Lite\y-coustic.xml`.

With no file, the built-in defaults apply. **Open configuration** in the tray menu
writes them to the place that would be used, ready to edit. The file is read at
start-up and on **Reload configuration**.

## Mistakes

A document that is not well-formed XML stops the load and is reported with its
line number. Anything else wrong is a **warning**: one line in a balloon, the
rest of the file still loads. That covers an unknown action, key or attribute, a
value out of range (it is clamped), and a second copy of a section (ignored).

A complete example, with every element and attribute and a comment on each, is
[`examples/y-coustic.xml`](examples/y-coustic.xml). The default file
(`rsc/y-coustic.xml`) is its short version.

## The file

```xml
<y-coustic>
	<volume step="5"/>
	<watch master="off" microphone="hud"/>
	<controls>
		<keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="windows"/>
		<keys up="Win+NumAdd" down="Win+NumSub" mute="Win+Numpad0"/>

		<layer mod="none"  target="master"/>
		<layer mod="Shift" target="mic"/>
	</controls>
	<hotkeys>
		<hotkey key="Win+Alt+V" action="mixer"/>
	</hotkeys>
	<mouse>
		<wheel mod="Alt" target="master"/>
		<wheel mod="Alt+Shift" target="mic"/>
		<button button="middle" mod="Alt" action="mute"/>
	</mouse>
	<tray hover-delay="600" click="mixer" double-click="none" middle-click="none" wheel="true" icon="fixed"/>
	<mixer anchor="tray" max-rows="8" system-sounds="true"/>
	<hud style="bar" position="bottom-center" timeout="1200" interactive="true"
	         scale="100" radius="10" border="0" opacity="100" font="Segoe UI" fade="0"/>
</y-coustic>
```

### `<volume>`

| Attribute | Default | Meaning |
| --- | --- | --- |
| `step` | `5` | percent of the full range one step moves, `1` to `30` |

### `<controls>`

Groups of keys, each saying what it controls. A `<keys>` group is up to three
keys, and is aimed one of two ways, written on the group itself:

| Aimed by | Written | Meaning |
| --- | --- | --- |
| a **target** | `target="mic"` | the keys control that, with no modifier held: a set of keys that does one thing |
| **layers** | `<layer>` children | the group is crossed with its own layers: the modifier held says what the keys control, and the layer's modifiers are **added** to the group's keys |

Nothing is shared between groups. A set of keys that only handles the microphone
is a group of its own with `target="mic"`; it has none of the layers of the
volume keys.

```xml
<controls>
  <!-- the keyboard's volume keys: the modifier says what they control -->
  <keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="windows">
    <layer mod="none"  target="master"/>
    <layer mod="Shift" target="mic"/>
    <layer mod="Ctrl"  target="focused"/>
  </keys>

  <!-- a mute of the microphone, and nothing else -->
  <keys mute="Win+Alt+M" target="mic"/>

  <!-- the focused application, on keys of their own -->
  <keys up="Win+Alt+PageUp" down="Win+Alt+PageDown" target="focused"/>
</controls>
```

`<keys>`: up to three keys, each optional, but at least one.

| Attribute | Meaning |
| --- | --- |
| `up`, `down`, `mute` | a key combination, as in `<hotkey key>`. A modifier already in it stays: `Win+NumAdd` under a `Shift` layer is `Win+Shift+NumAdd` |
| `plain` | `windows` (default) or `replace`, for groups with a media key, see below |
| `target` | `master`, `mic`, `focused` or `pointed`: what the keys control, with no layer |

`<layer>`, inside a `<keys>`:

| Attribute | Default | Meaning |
| --- | --- | --- |
| `mod` | `none` | the modifiers held: `none`, or `Win`, `Ctrl`, `Alt`, `Shift` joined by `+` |
| `target` | `master` | `master`, `mic`, `focused` or `pointed` |

A group with layers of its own ignores its `target` (a warning says so). A group
with neither controls nothing, and says so. A `<layer>` directly under
`<controls>`, as an older file had it, is **not read**: it is warned about and
kept in the file as a comment. To bring such a file up to date, move its layers
inside each group that used them.

**Media keys.** `VolumeUp`, `VolumeDown` and `VolumeMute` are caught by the
keyboard hook rather than by a hotkey registration, whatever modifiers they carry
(the hook is the one route that behaves the same on every keyboard, and it takes
the key before Windows shows its flyout). Two things follow:

| `plain=` | Bare volume keys | Modifier + volume key |
| --- | --- | --- |
| `windows` (default) | Windows, with its flyout | y-coustic |
| `replace` | y-coustic, with its bar | y-coustic |

- With `plain="windows"`, the bare keys are left out of the expansion: they are
  Windows'. Taking them over is your decision, not a side effect of installing.
- The hook is installed only if there is at least one media key to catch. A file
  with no media key has no keyboard hook at all.
- It fails safely: if y-coustic is not running, or an elevated window has the
  focus, the key reaches Windows as usual. Keys that never reach a keyboard hook,
  such as a Bluetooth headset's buttons, are not supported.

**Warnings** the expansion gives, each once:

- a **collision**: the same combination used twice, by two layers of a group, a
  group and a `<hotkey>`, or two groups; the message names the group. The first use wins and the second is ignored. Single
  `<hotkey>`s come first.
- a group that **controls nothing**: no `target` and no layer.
- an **`Alt` layer**: it works, but letting go of `Alt` alone afterwards focuses
  the menu bar of the window in front.

The tray menu has **Replace the Windows volume indicator**, shown when the file
has a media group. It flips `plain` on every media group, saves the file and
reloads (see *Saving from a program* below).

### `<hotkeys>`

One `<hotkey>` per binding, for whatever the groups do not cover: a fixed level
(`set-volume`), the mixer, or a single key you would rather read in one line. A
set of up, down and mute keys for one thing is a `<keys target>` group instead. A `<hotkey>`
on a media key goes through the keyboard hook too.

| Attribute | Meaning |
| --- | --- |
| `key` | the combination: modifiers `Win`, `Ctrl`, `Alt`, `Shift` joined by `+`, then a key (`A`-`Z`, `0`-`9`, `F1`-`F24`, `Numpad0`-`Numpad9`, `NumAdd`, `NumSub`, `VolumeUp`, ...; `0x..` for a raw virtual-key code) |
| `action` | see below |
| `value` | `0` to `100`, required by `set-volume` |
| `target` | `master` (the default output; the default when omitted), `mic` (the default microphone), `focused` or `pointed` (an application, see below) |

| Action | Does |
| --- | --- |
| `volume-up` | one step up, and unmutes |
| `volume-down` | one step down |
| `mute` | toggles the mute switch |
| `set-volume` | sets the level to `value`, and unmutes |
| `mixer` | opens or closes the [mixer](#mixer); takes no target |

`volume-up` and `volume-down` repeat while the key is held; the others fire once
per press.

**The microphone differs from the master in one way**: only `mute` unmutes it.
`volume-up` and `set-volume` change the level and leave the mute switch as it
is, so a stray key cannot open a live microphone. And a `mute` reaches the
default *communications* microphone too when that is another device, since a
call application may be listening to it. Reaching 0 % does not mute.

**The applications** (`target="focused"` and `target="pointed"`) change the volume
of one application in the Windows mixer, on whichever output device it plays:

- `focused` is the application of the window in front, `pointed` the one of the
  window under the mouse pointer (no fallback to the focused one). The taskbar,
  the desktop, menus and y-coustic's own windows are never applications.
- The window's process is not always the one playing: a browser plays from a
  helper, a Store app's window belongs to `ApplicationFrameHost`. y-coustic
  matches by **executable**, so Chrome's audio helper and its main process are one
  application, then by the process tree for a helper with another executable.
  A Store app is resolved through its core window first.
- An application with several sessions (several devices, several helpers) is one
  row: each session steps on its own, so their relative levels stay; **mute** is
  one decision for all (mute everything if anything is audible, else unmute).
  The HUD shows the loudest level, and "muted" only if all are.
- Like the master, raising the volume unmutes the application; the mute switch is
  the application's own and its level is never touched by it.
- **No audio** (the window's application has not played anything yet, or the
  window is not eligible): a key or a click shows *No audio* and does nothing. A
  **wheel** scrolls as usual: it was taken before y-coustic could know, and is
  sent back to the window, with the same modifiers, a moment later.
- The HUD names the application (the executable's *file description*, else its
  file name) and shows its icon.
- Windows keeps another process's session volume out of reach in principle; in
  practice setting it works, and y-coustic relies on that. An elevated
  application's executable cannot be read from here, so it is found by process
  alone.

### `<watch>`

What to do when the volume or the mute changes from **outside** y-coustic: in the
Windows Sound panel or volume mixer, from a laptop's microphone key, from another
program. A change y-coustic makes itself is never reported back.

| Attribute | Default | Meaning |
| --- | --- | --- |
| `master` | `off` | the default output |
| `microphone` | `hud` | the default microphone |

| Value | Reaction |
| --- | --- |
| `off` | nothing, and nothing resident for that endpoint |
| `tray` | keep the tray icon true to its `<tray icon>` rule (nothing happens with the default `fixed`) |
| `hud` | the same, and show the HUD |

The defaults follow from what Windows already does: it shows its own flyout for
the master when a volume key reaches it, so a bar of ours would double it; it
shows nothing for the microphone, where believing you are muted when you are not
costs the most. For the master, `tray` has no icon variant to keep true, so it
behaves like `off`.

Only the default device of each flow is watched, and the watch follows the
default when it changes. A communications microphone that differs from the
default one is muted along with it by y-coustic, but is not watched.

### `<mixer>`

The mixer window the `mixer` action and a click on the tray icon open.

| Attribute | Default | Meaning |
| --- | --- | --- |
| `anchor` | `tray` | where it opens: `tray` (next to the icon, above it for a bottom taskbar), `cursor` (at the pointer) or `hud` (where the HUD appears) |
| `max-rows` | `8` | rows shown before the list scrolls, `3` to `20`; a scrollbar then appears |
| `system-sounds` | `true` | `false` hides the last row, Windows' own system sounds |

The rows are the master, the microphone, then one row per application that has an
audio session, in the order Windows lists them. An application that is paused
keeps its row, so it can still be adjusted; one with no session yet has none.

### `<tray>`

What resting the pointer on the tray icon does.

| Attribute | Default | Meaning |
| --- | --- | --- |
| `hover-delay` | `600` | milliseconds the pointer rests on the icon before the HUD shows next to it, `0` to `5000`; `0` turns the hover off |
| `click` | `mixer` | a left click: `mixer`, `hud`, `mute`, `mute-mic`, `volume-up`, `volume-down` or `none` |
| `double-click` | `none` | a double left click, same values; with one set, the single `click` waits the system double-click time to be sure it is alone |
| `middle-click` | `none` | a middle click, same values |
| `wheel` | `true` | the wheel over the icon steps the master volume and shows it on the HUD; needs the hover (`hover-delay` above `0`), since Windows reports the pointer on an icon only when it has no tooltip |
| `icon` | `fixed` | how the icon follows the devices: `fixed` (always the same icon) or `mic-muted` (the muted variant while the microphone is muted). More rules (by volume level) are meant to come |

With the hover on, the HUD **is** the icon's tooltip: Windows shows none (it
reports the pointer entering and leaving only for an icon without one), so the
text of the tooltip (*microphone muted*) is not shown; the icon itself still
changes. With `hover-delay="0"` the standard tooltip comes back and nothing else
changes.

The HUD shows the master volume, above the icon for a bottom taskbar and below it
otherwise, whatever `<hud position>` says. It stays while the pointer is on the icon
(the shell tells when it enters and leaves), then goes after `timeout` as ever.
It opens the HUD and not the mixer because the mixer takes the focus: a window
that took the foreground whenever the pointer crossed the taskbar would be worse
than no feature. The click opens the mixer. For an icon hidden in the overflow
flyout Windows does not say where the icon is, and may not report the pointer at
all: drag the icon onto the taskbar to have the hover.

### `<hud>`

The HUD: the small window that shows the volume when it changes, and when the pointer rests on the tray icon.

| Attribute | Default | Meaning |
| --- | --- | --- |
| `style` | `bar` | the shape: `bar` (glyph, bar, percentage), `compact` (smaller, no percentage), `text` (glyph and percentage, no bar), `vbar` (tall: glyph, vertical bar, percentage), `ring` (a ring filling clockwise from the top, glyph and percentage inside) |
| `position` | `bottom-center` | `top-left`, `top-center`, `top-right`, `center-left`, `center`, `center-right`, `bottom-left`, `bottom-center`, `bottom-right`, or `follow-cursor` (beside the pointer when it shows), on the monitor under the pointer |
| `scale` | `100` | size in percent of the design size, `50` to `300` (the screen's scaling applies on top) |
| `radius` | `10` | corner radius, `0` to `40` |
| `border` | `0` | border width, `0` to `8`, `0` for none |
| `opacity` | `100` | how opaque the whole bar is, in percent, `10` to `100` |
| `font` | `Segoe UI` | the font family of the text |
| `timeout` | `1200` | milliseconds the HUD stays after the last change, `100` to `10000` |
| `fade` | `0` | milliseconds the HUD takes to fade in and to fade out, `0` to `1000`; `0` shows and hides at once. A change while it fades out brings it back, and the pointer on it keeps it |
| `interactive` | `true` | `true` lets the pointer use the HUD (below), `false` keeps it click-through and turns off the hover on the tray icon |

**Using the HUD.** With `interactive="true"` the pointer can use the HUD while it
is on screen:

| Pointer | Does |
| --- | --- |
| over it | the HUD stays: its timer is paused until the pointer leaves, then starts again |
| wheel over it | steps what it shows, as the mouse bindings do, in fractions on a touchpad |
| click or drag on the bar | sets the level (the drag may leave the bar, the level stays within 0 to 100) |
| click on the icon or glyph | mutes or unmutes |
| any other click | opens the [mixer](#mixer) |

It acts on the row it shows: the master, the microphone, or the application it
names. A wheel with a modifier held over the HUD is still a mouse binding's. The
bar never takes the focus; clicking it leaves the window in front where it was.
Where the HUD covers a window, it also covers its clicks, which is why
`interactive="false"` exists.

### `<colors>`

The colours of the HUD and of the [mixer](#mixer), which share them (and the
`<hud>` `font` and `radius`). Every attribute is optional and a colour is
`#RRGGBB` (or `#RGB`); what is left out follows the Windows theme, light or dark.

| Attribute | Meaning |
| --- | --- |
| `panel` | the window's background |
| `text` | text and the glyphs |
| `track` | the empty part of a bar |
| `accent` | the filled part of a bar while sound is on |
| `muted` | the filled part while muted |

The attributes on `<colors>` apply to everything. A `<master>`, `<microphone>` or
`<application>` child takes the same attributes for that kind alone and wins, so
a glance tells them apart:

```xml
<colors accent="#3FB950">
	<microphone accent="#D9534F"/>
	<application accent="#E8A33D" muted="#777777"/>
</colors>
```

The microphone's bar is red while it is live whatever the theme: a general
`accent` does not reach it, only `<microphone accent>` does, because taking a live
microphone for the master volume is the mistake that costs.

### `<mouse>`

Wheel and button bindings, taken by a mouse hook that exists only if this
section has bindings. Its modifiers are matched **exactly**: `Alt` does not
answer `Alt+Shift`.

| Element | Attribute | Meaning |
| --- | --- | --- |
| `<wheel>` | `mod` | required: `Win`, `Ctrl`, `Alt`, `Shift` joined by `+` |
| | `target` | `master` (default), `mic`, `focused` or `pointed` |
| | `invert` | `true` to lower going up and raise going down |
| | `axis` | `vertical` (default), or `horizontal` for the sideways tilt: right raises |
| `<button>` | `button` | `left`, `right`, `middle`, `x1` or `x2` (the side buttons) |
| | `mod` | as above; may be left out for `x1` and `x2` only |
| | `action` | as for [`<hotkey>`](#hotkeys); `volume` is not an action here, a wheel is its own |
| | `value`, `target` | as for `<hotkey>` |

- **A wheel is proportional.** A notch is one step; a touchpad's or a free-spinning
  wheel's smaller deltas move a fraction of one (`delta / 120` steps). Raising
  unmutes, lowering does not, and the microphone is never unmuted by a wheel.
- **A button acts on release.** The press is swallowed, the release is consumed,
  and the action runs when the button comes up, with the modifiers that were held
  at the press. A release whose press was not taken is never eaten.
- **Never bare.** A wheel, `left`, `right` or `middle` without a modifier would
  take over scrolling and clicking everywhere: the binding is dropped with a
  warning. `x1` and `x2` may be bare.
- **Alt and Win are safe.** Letting go of `Alt` alone after a claimed event would
  focus the window's menu bar, and `Win` would open Start; y-coustic corrects both
  at the key-up, as `y-snap` does.
- Two bindings on the same input and modifiers: the first wins, the other is
  dropped with a warning.
- Like the keys, the hook sees nothing while an elevated window has the
  foreground.

## Saving from a program

A program that writes the file (the tray's *Replace the Windows volume
indicator*, and the settings app) writes it back with what it found: the
comments, the attributes it does not read, and the elements it cannot use (kept
as comments holding their own XML, and reported once). Sections come in a fixed
order and the attributes of each in a fixed order; the empty lines of the file
are kept where they were. A comment belongs to the entry that follows it and
moves with it, and a comment after the last entry of a list stays at the end of
it. The `<controls>` are written as groups and layers, not as the bindings they
expand to. The file is written to a temporary file and renamed over the
original, and the first save of a run keeps what was there as `y-coustic.xml.bak`.
