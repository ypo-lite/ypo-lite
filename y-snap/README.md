# y-snap

**Your windows, handled from wherever your pointer is.** Move and resize any
window from anywhere inside it, and minimize, roll up, fade, hide or pin it from
the keyboard or from one popup, with no need to aim for a title bar.

<!-- Add once recorded, see docs/design/media-shots.md:
![Moving and resizing with Alt](docs/images/y-snap_drag_01.gif)
-->

PowerToys' "Grab and Move" does a part of this. y-snap is the rest: minimize,
maximize, close, roll up, fade, strip the frame, hide from the taskbar, hide
behind a tray icon, pin to every virtual desktop or send to another one, on
whichever window the pointer happens to be over.

## Why y-snap

- **Light.** One native executable of about 300 KB. No runtime, no background
  service, and nothing injected into your other programs.
- **Works on the window you point at.** No clicking it into focus first.
- **Ready at once.** The defaults are useful from the first start, and every one
  of them can be changed.
- **Portable.** Drop `y-snap.xml` next to `y-snap.exe` and take it with you.

## Move and resize from anywhere

Hold `Alt` and drag with the left button to move a window, with the right button
to resize it, from anywhere inside it. Resizing pulls the corner nearest to where
you pressed. The buttons and modifiers are yours to change.

## One popup, every action

`Ctrl+Space` opens a grid of tiles over the window, which gets a coloured outline
so you can see which one it is. One key press or one click does the action.
Tiles show what is on: *Restore*, *Unpin*, *Remove top*.

<!-- Add once recorded:
![The action overlay](docs/images/y-snap_overlay_01.gif)
-->

| | |
| --- | --- |
| **Roll up** | shrink a window to its title bar, and back |
| **On top** | keep a window above the others |
| **Fade** | make a window see-through |
| **Borderless** | strip the title bar and frame |
| **Hide from the taskbar** | keep a window on screen, without its button |
| **Pin** | show a window on every virtual desktop |

Tiles, keys, labels, icons, colours and layout are all configurable, with light,
dark and automatic themes.

## The minimize button does more

Rest the pointer on any window's minimize button and a small popup of tiles
appears under it. Right-click the button and the window goes straight to y-snap's
tray menu. No hotkey to remember.

<!-- Add once recorded:
![The minimize button](docs/images/y-snap_minbutton_01.gif)
-->

## Put windows away

Clear the taskbar without closing anything. `iconize` turns a window into an icon
of its own in the notification area, `traynize` lists it in y-snap's tray menu.
It comes back exactly as it was, and quitting y-snap brings everything back.

## Virtual desktops, one key away

Switch desktops, create one, pin a window to all of them, or send a window
to another desktop with or without following it. The overlay shows one numbered
tile per desktop, so *`Ctrl+Space`, `3`* moves the window you are pointing at.

<!-- Add once recorded:
![Moving a window to another desktop](docs/images/y-snap_desktops_01.gif)
-->

## Your mouse, your shortcuts

A modifier plus a button, a side button or a wheel tilt can fire any action: tilt
the wheel to pin a window, or press `Win` and the middle button for the overlay.
None are set by default.

## Set it up your way

Use **y-snap-config**, the settings app, and y-snap applies what you save at once.
Or edit `y-snap.xml`, where a key reads the way you press it:

```xml
<hotkey key="Win+Alt+T" action="always-on-top" target="pointed"/>
```

<!-- Add once taken:
![y-snap-config](docs/images/y-snap_settings_01.png)
-->

An unknown action or a bad key combination costs you that one setting: y-snap warns
about it and loads the rest.

## Get it

y-snap runs on Windows 10 and 11 (x64). The
[main page](../README.md#install) has the installer and the portable zip.

## Documentation

| | |
| --- | --- |
| [User manual](docs/user-manual.md) | how to use y-snap: the defaults, common changes, troubleshooting |
| [Configuration reference](docs/configuration.md) | every setting of `y-snap.xml` |
| [Development](docs/development.md) | how the code is organised, and the design notes |

y-snap is the Rust rewrite of [HulK](http://glatigny.github.io/guipro/), part of
[Ypo-Lite](../README.md).
