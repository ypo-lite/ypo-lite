# y-coustic — user manual

How to use y-coustic. Every setting is in the
[configuration reference](configuration.md); how it is built is in the
[development notes](development.md).

> This manual covers what exists today: the master volume, the microphone,
> hotkeys, the mouse, applications, the mixer and the HUD. See the
> [implementation plan](design/implementation-plan.md).

## Starting

Run `y-coustic.exe`. It has no window: an icon appears in the notification area
(Windows 11 may put it in the overflow, drag it out to keep it visible). Only one
copy runs; starting a second shows the menu of the first.

To start it with Windows, tick **Start with Windows** in the icon's menu.

## Changing the volume

The default keys, VulCan's:

| Keys | Does |
| --- | --- |
| `Win` + `NumAdd` | louder |
| `Win` + `NumSub` | quieter |
| `Win` + `Numpad0` | mute / unmute |

- **Louder** also unmutes, so a press is never silently swallowed. **Quieter**
  leaves the mute switch alone.
- Holding **louder** or **quieter** keeps stepping. Holding **mute** toggles once.
- A step is 5% of the full range by default (`<volume step>`).
- The keys act on the **default output device** at the moment you press them. If
  you change it in Windows, y-coustic follows at the next press.

Change the keys, add a fixed-level key (`set-volume`) or move to a laptop-friendly
set in the [configuration](configuration.md#hotkeys).

### Your keyboard's volume keys, and layers

A modifier chooses what a set of keys controls. By default, the same three keys
(volume up, down, mute) with and without `Shift` do different things:

| Held | Volume keys, and the numpad keys with `Win` |
| --- | --- |
| nothing | the volume keys are **Windows'**, with its flyout; the numpad keys act on the master |
| `Shift` | the microphone |
| `Ctrl` | the application in the front window |
| `Ctrl` + `Shift` | the application under the mouse pointer |

So `Shift` + volume up raises the microphone, `Ctrl` + volume up raises the
application you are working in, and `Ctrl` + `Shift` + volume up the one under
the pointer. `Win` + the same modifiers do it from the numpad group.

**Taking over the bare volume keys.** If you would rather have y-coustic's HUD
than the Windows flyout, choose **Replace the Windows volume indicator** in the
tray menu. y-coustic then handles the bare volume keys, and Windows' own volume
keeps working only when y-coustic is not running or an elevated window has the
focus. Choose it again to give them back. It is a deliberate choice: y-coustic
never takes a key you rely on by itself.

The volume keys are caught by a keyboard hook, which exists only if the file has
volume keys. Buttons that never reach a keyboard hook, such as a Bluetooth
headset's, stay Windows' and show its flyout.

Layers are written as `<controls>` in the [configuration](configuration.md#controls).

## Applications

The `Ctrl` layer and the `Ctrl` + `Shift` layer (and their mouse bindings) change
**one application** instead of the whole machine, the same volume you see in the
Windows mixer.

- **The one in front** (`focused`), or **the one under the pointer** (`pointed`).
- The HUD shows the application's icon and name, its level, or *Muted*.
- An application that has not played anything yet has no volume to change: a key
  shows *No audio*, a wheel scrolls the page as usual.
- An application playing on two devices, or through a helper process like a
  browser's, is still one application: stepping moves every part, muting mutes
  every part.
- Raising an application's volume unmutes it.

## The mouse

With the default configuration, holding `Alt`:

| Mouse | Does |
| --- | --- |
| `Alt` + wheel | master volume, one step per notch |
| `Alt` + `Shift` + wheel | microphone volume |
| `Alt` + middle click | mute / unmute the master |
| `Alt` + `Shift` + middle click | mute / unmute the microphone |
| `Ctrl` + `Shift` + wheel | the application under the pointer |
| `Ctrl` + `Shift` + middle click | mute / unmute the application under the pointer |

- On a touchpad or a free-spinning wheel the change is proportional: a small
  scroll moves a fraction of a step.
- Over a window with no audio, `Ctrl` + `Shift` + wheel just scrolls: y-coustic gives
  the wheel back. A plain wheel, and every click without the modifier, work as ever: y-coustic
  never takes a bare wheel or main button.
- The middle click acts when you **release** the button.
- Letting go of `Alt` afterwards does not focus the window's menu bar.
- Like hotkeys, the mouse bindings do nothing while an elevated window (one run
  as administrator) has the focus. Windows keeps those out of reach.

Add, remove or change them in the [configuration](configuration.md#mouse).

## The microphone

Give a hotkey `target="mic"` and it acts on the default microphone instead (see
the [configuration](configuration.md#hotkeys)). Three things set it apart:

- **Only mute unmutes it.** A step key or a fixed level changes the level and
  leaves the switch alone, so a stray key never opens a live microphone.
- **A mute reaches every microphone a call could use**: the default one and the
  default *communications* one, when they are different devices. The HUD shows
  what the devices say after the change, not what was asked: *Muted* means every
  one of them is.
- **The tray icon can show it, if you ask.** By default the icon never changes. With `<tray icon="mic-muted">` it changes while the microphone is muted (a
  stock one until `y-coustic-mic-muted.ico` is supplied) and its tooltip says so.
  A change made elsewhere, in the Sound panel or by a laptop key, is noticed too
  (see *Changes made elsewhere* below).

With no microphone connected, or all of them disabled, the HUD says *No
microphone*; the key is still consumed.

## Changes made elsewhere

y-coustic can notice a volume or mute changed from outside: in the Windows Sound
panel or mixer, by a laptop's microphone key, by another program. `<watch>` says
what to do about it (see the [configuration](configuration.md#watch)):

- **Microphone, by default**: the HUD appears (and the tray icon follows if its rule says so), so a mute
  you did not make yourself is never a surprise in a call.
- **Master, off by default**: Windows already shows its own flyout. Set
  `master="hud"` to get our HUD for what its flyout misses, such as the volume
  center.

Our own keys never trigger it a second time. Switching the default device moves
the watch to the new one and updates the tray icon, without the HUD: plugging in a
headset is not a volume change.

## The HUD

After each change the HUD, a small window, appears on the monitor the pointer is on,
near the bottom by default: a speaker (with waves that follow the level, and a cross
when muted), or a microphone (slashed when muted, with a red bar while live), a bar
for the level, and the percentage, or *Muted*. It follows the Windows light or dark
theme, and goes away `timeout` milliseconds after the last change.

It never takes the focus. By default you can also use it while it is there:

- **Rest the pointer on it** and it stays, instead of going after the timeout. It
  goes a moment after the pointer leaves.
- **Wheel over it** steps the volume it shows (a fraction on a touchpad).
- **Click or drag on its bar** sets the level.
- **Click on its icon** mutes or unmutes.
- **Click anywhere else on it** opens the [mixer](#the-mixer).

It acts on what it shows: the master, the microphone or the application it names.
If you would rather it never caught a click, set `interactive="false"` on
`<hud>`: the pointer then goes right through it, and the HUD on the tray icon
is off too.

**Its look is yours to choose** with `<hud>`:

- `style`: `bar` (the one above), `compact` (smaller, no percentage), `text` (glyph and
  percentage, no bar), `vbar` (tall, for a screen edge) or `ring` (a ring that fills
  clockwise from the top). A click or a drag on a `ring` or `vbar` sets the level the
  same way; `text` has nothing to set, so only its icon mutes.
- `position`: any of the nine places on the screen (`top-left` … `bottom-right`), or
  `follow-cursor`, beside the pointer.
- `scale` (size, in percent), `radius` (corners), `border`, `opacity`, `font` and `fade` (milliseconds to fade in and out, off by default).
  `<colors>` sets the colours, per kind if wanted (the microphone stays red unless told otherwise); the mixer shares the colours, font and radius. See the configuration reference.

An application's name is only written in the `bar` style; the other styles show its icon.

## The mixer

`Win` + `Alt` + `V`, or a left click on the tray icon, opens the mixer: the master,
the microphone and every application that has audio, each with its level. Open it
again (or press `Esc`, or click anywhere else) to close it.

```
┌──────────────────────────────────────┐
│ 🔊 Master      ▓▓▓▓▓▓▓▓▓░░░░░░░   60 │
│ 🎤 Microphone  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░   80 │
│ ──────────────────────────────────── │
│ ▌  Firefox     ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░   92 │
│    Spotify     ▓▓▓▓▓░░░░░░░░░░░   35 │
└──────────────────────────────────────┘
```

It starts at the top, on the application you were in when that row is shown (else the one
under the pointer, else the master).

| Input | Does |
| --- | --- |
| `Up`, `Down` | select a row; the list follows |
| `Home`, `End`, `PageUp`, `PageDown` | first, last, a page up or down |
| `Left`, `Right` | step the selected row; with `Shift`, by 1 % |
| `M` or `Space` | mute / unmute the selected row |
| `0` to `9` | set the selected row to 0 to 90 % |
| the volume keys | act on the selected row |
| wheel over a row | step that row |
| the scrollbar (when there are more rows than shown): drag the thumb, click the track, or the wheel over it | scroll the list |
| click or drag on a bar | set the level |
| click the icon, or middle-click a row | mute / unmute |
| `Esc`, or a click elsewhere | close |

- With more rows than `max-rows`, a scrollbar appears at the right: its thumb shows how
  much of the list is in view and where. The list follows the arrow keys too.
- A muted row is dimmed and says *Muted*; its level is kept, and unmuting gives it
  back.
- An application that is paused still has a row. A player you cannot point at (no
  window, or hidden in the tray) is reached here.
- The numbers refresh once a second while it is open, so a volume changed
  elsewhere shows up.
- It keeps nothing when closed: the window and the rows are dropped.

## The tray icon

| Click | Does |
| --- | --- |
| Left | opens or closes the [mixer](#the-mixer) (`<tray click>`) |
| Double, middle | nothing until `<tray double-click>` / `<tray middle-click>` are set (mute, mute-mic, volume-up, volume-down, hud, mixer) |
| Wheel | steps the master volume (`<tray wheel>`, needs the hover on) and shows it next to the icon; not with a modifier held |
| Resting the pointer on it | shows the master volume next to the icon, after 600 ms (see below) |
| Right | the menu below |

The menu:

| Menu item | Does |
| --- | --- |
| *Reload configuration* | reads `y-coustic.xml` again and re-registers the hotkeys |
| *Open configuration* | opens the file; creates it from the defaults first if it does not exist |
| *Replace the Windows volume indicator* | shown when the file has volume keys: gives the bare volume keys to y-coustic or back to Windows, saves the file and reloads |
| *Start with Windows* | adds or removes y-coustic from the current user's start-up |
| *About* | version, memory use and the path of the configuration file in use |
| *Exit* | quits and gives the keys back |

### Resting on the icon

Leave the pointer on the icon for a moment and the HUD appears next to it,
showing the master level: the icon's tooltip, with more to say. It stays while the
pointer is on the icon, however you move it there, and goes by itself
a moment after you move away. Windows' own tooltip is off while this is on.

`<tray hover-delay>` sets the moment, and `0` turns the HUD off and gives the
Windows tooltip back. If the icon is in the overflow flyout, drag it onto the
taskbar: Windows does not report the pointer on an icon there.

## When something is wrong

- **A balloon after a reload**: the configuration had a problem that did not stop
  it loading. Each line names the line number of the file. The rest of the file
  works.
- **"Already taken by another application"**: Windows or another program owns
  that key combination. Choose another in the file.
- **A file that cannot be read at all** (a tag left open, say) is reported with
  its line number and nothing is registered until you fix it and reload.
- **The HUD says "No audio output" or "No microphone"**: there is no such device
  enabled. Plug one in or enable it in the Windows sound settings.
