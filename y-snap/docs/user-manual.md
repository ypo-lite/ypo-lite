# y-snap — user manual

y-snap is a tray utility that manages your windows: move and resize them from
anywhere inside them, and minimize, roll up, fade, hide or pin them from the
keyboard or from a popup, on whichever window the pointer is over.

This manual describes **what works today**, and how to use it. Every attribute of
the configuration file is in the [configuration reference](configuration.md); this
page shows the ones you are likely to want. The design notes are in
[`design/`](design/).

- [First steps](#first-steps)
- [The tray icon](#the-tray-icon)
- [Which window an action applies to](#which-window-an-action-applies-to)
- [Moving and resizing](#moving-and-resizing)
- [The action overlay](#the-action-overlay)
- [The minimize button](#the-minimize-button)
- [What each action does](#what-each-action-does)
- [Hidden windows](#hidden-windows)
- [Virtual desktops](#virtual-desktops)
- [Mouse shortcuts](#mouse-shortcuts)
- [Changing the configuration](#changing-the-configuration)
- [Not there yet](#not-there-yet)
- [Troubleshooting](#troubleshooting)

---

## First steps

Start y-snap and its icon appears in the notification area. With the default
configuration, this is what you can do straight away:

| Do this | To get this |
| --- | --- |
| `Alt` + drag with the left button | move the window, from anywhere inside it |
| `Alt` + drag with the right button | resize the window, from anywhere inside it |
| `Ctrl+Space` | the [action overlay](#the-action-overlay), for the window under the pointer |
| `Win+Esc` | close the window under the pointer |
| `Win+'` | minimize the window under the pointer. This is the key Windows calls `OEM7`, so it sits elsewhere on some keyboards |
| `Ctrl+Shift+P` | put the pointer in the middle of the focused window |
| Rest the pointer on a window's minimize button | a small popup: *Traynize*, *On top* |
| Right-click a window's minimize button | hide the window in y-snap's tray menu, see [Hidden windows](#hidden-windows) |

To start y-snap with Windows, tick *Start Y-Snap with Windows* in the installer.
With the portable zip, put a shortcut to `y-snap.exe` in your Startup folder
(`Win+R`, then `shell:startup`).

---

## The tray icon

| | |
| --- | --- |
| Right click | the menu |
| Double click | the settings app |
| Left click | nothing |

The menu lists, from the top:

- **Hidden windows**, when there are any, with **Restore all hidden windows**;
- how many hotkeys and overlay actions are loaded, or what is wrong with the
  configuration when it did not load;
- **Configuration**, a submenu with **Settings…**, **Edit y-snap.xml** and
  **Reload configuration**;
- **Start with Windows**, checked when y-snap runs at logon; click to turn it on or
  off (the same setting as the installer's start-up option, and as the switch at the
  bottom of the settings app);
- **About**: a balloon with the version, what is loaded, where the file is and how
  much memory y-snap uses. Clicking the balloon opens the settings app;
- **Exit**, which brings every hidden window back first.

Starting y-snap while it is already running opens this menu at the pointer, which
is handy when Windows has tucked the icon away in the overflow area.

---

## Which window an action applies to

Each hotkey says which window it wants, with `target`:

| `target` | Acts on |
| --- | --- |
| `focused` | the focused window. The default for a hotkey |
| `pointed` | the window under the pointer, or the focused one if there is nothing there |

The default hotkeys use `pointed` so you can act on a window without clicking it
first. A [mouse shortcut](#mouse-shortcuts) also defaults to `pointed`, but has no
fallback: over the desktop or the taskbar the click just passes through.

Actions never apply to Windows itself: the desktop, the taskbar, the Start menu,
the task switcher, tooltips and menus are all refused. When that is the only thing
there, nothing happens and y-snap says nothing.

---

## Moving and resizing

Hold the modifier, press the button anywhere inside the window, and drag. You do
not need to aim for the title bar or an edge.

- **Moving** keeps the window under the pointer.
- **Resizing** moves the corner nearest to where you pressed, and the opposite
  corner stays put. Press in the lower right quarter to pull the lower right
  corner, in the upper left quarter to pull the upper left one. A window cannot get
  smaller than 32 pixels this way.

The modifiers must match exactly: `Ctrl+Alt` + drag does nothing when the gesture
is `Alt`. You may let go of the modifier and the button in either order, and doing
so will not open the window's system menu or the Start menu.

To use other buttons or modifiers, see [Mouse gestures](#changing-the-configuration)
in the settings app, or `<drag>` and `<resize>` in the
[configuration reference](configuration.md#mouse).

---

## The action overlay

`Ctrl+Space` opens a grid of tiles centred on the target window, which gets a
coloured outline so you can see which one it is.

| | |
| --- | --- |
| A tile's key | does that action and closes the overlay |
| A click on a tile | the same |
| `Esc`, or a click outside | closes it |

The tile under the pointer is highlighted. A tile whose toggle is on for this
window, such as *On top*, uses its active colours, and its label says what it would
do now: *Remove top*, *Restore*, *Unpin*.

The default tiles:

| Key | Tile | Key | Tile |
| --- | --- | --- | --- |
| `M` | Minimize | `R` | Roll up |
| `X` | Maximize | `H` | Hide from the taskbar |
| `C` | Close | `U` | Pin to every desktop |
| `T` | On top | `P` | Center cursor |
| `I` | Iconize | `N` | New desktop |
| `Y` | Traynize | `←` | Send to the previous desktop |
| `B` | Borderless | `→` | Send to the next desktop |
| `F` | Fade | `1`-`9` | Move to that desktop, and switch to it |

The row of numbered tiles follows your desktops as you add and remove them. The
one you are on says *Current* and does nothing.

---

## The minimize button

Two things happen on the minimize button of any window, with no hotkey involved:

- **Rest the pointer on it** for 0.7 seconds and a popup of tiles appears right
  under the button. Moving away or moving within the button restarts the wait, so
  it does not flash open as you pass through. Click a tile; the popup has no keys.
- **Right-click it** to do one action at once, with no popup. By default that is
  `traynize`, so **the window vanishes into y-snap's tray menu**. If that surprises
  you, see below to turn it off.

The popup's tiles come from the settings too (*Minimize button* page). The
[configuration section](#changing-the-configuration) shows how to turn either
behaviour off.

---

## What each action does

Nearly all of these work on hotkeys, overlay tiles, the minimize button's popup and
mouse shortcuts alike. Most are toggles: do it again to undo it.

| Action | Does | Good to know |
| --- | --- | --- |
| `minimize` | minimizes | |
| `maximize` | maximizes, or restores a maximized window | |
| `close` | asks the window to close | Same as the X button: the program can show a save prompt, or refuse |
| `always-on-top` | keeps the window above the others | |
| `borderless` | removes the title bar and frame | `Alt+Space` still opens the window's menu |
| `transparency` | fades the window to about 70% opaque | The level is fixed. A window that already handles its own transparency is left alone, without a message |
| `shade` | rolls the window up to its title bar | Its height is remembered for rolling back down |
| `hide-taskbar` | removes the window's taskbar button | The window stays on screen, and it leaves `Alt+Tab` too. In a program with several windows the others can lose their buttons as well; `traynize` takes only the one |
| `center-cursor` | puts the pointer in the middle of the window | Useful for finding a window on a wide desktop |
| `iconize` `traynize` | hide the window, see [Hidden windows](#hidden-windows) | |
| `uniconize` `uniconize-all` | bring back the last hidden window, or all of them | Need no target window |
| `overlay` | opens the action overlay | |
| `reload` `quit` | y-snap itself | |

Changes stay on the window, not in y-snap: exit or restart y-snap and a borderless
window stays borderless, and toggling it again later still puts its frame back. They
last until the window closes.

---

## Hidden windows

Two ways to put a window away without closing it. The window leaves the screen and
the taskbar, and comes back as it was: a maximized window comes back maximized.

| | Where it goes | How to get it back |
| --- | --- | --- |
| `iconize` | an icon of its own in the notification area, with its title as tooltip | click the icon; right-click it for *Open* and *Close* |
| `traynize` | the **Hidden windows** list at the top of y-snap's tray menu | click it in the menu |

Either way, `uniconize` brings back the last one hidden, `uniconize-all` and
**Restore all hidden windows** bring back all of them, and exiting y-snap does too.

At most 48 windows can be hidden at once. A window running as administrator is
refused, with a balloon, unless y-snap runs as administrator too.

If the program closes the window, or shows it again by itself, y-snap notices
within five seconds and drops it from the list.

**If y-snap is killed**, from Task Manager or by a crash, its hidden windows stay
hidden with nothing to bring them back. Starting a program again sometimes shows
its hidden window; if not, end the program.

---

## Virtual desktops

| Action | Does |
| --- | --- |
| `desktop-next` `desktop-prev` | switch desktop, going round at the ends |
| `desktop-new` | creates a desktop, and stays where you are |
| `desktop-move` `desktop-move-next` `desktop-move-prev` | move the window to another desktop **and go with it** |
| `desktop-send` `desktop-send-next` `desktop-send-prev` | **send** the window to another desktop and stay here |
| `pin-toggle` | shows the window on every desktop, or only on its own |

`desktop-move` and `desktop-send` go to the desktop numbered by `desktop`, counting
from 1. A number past your last desktop does nothing. The `-next` and `-prev` forms
go round at the ends. None of this is bound to a hotkey by default, so add the ones
you want:

```xml
<hotkeys>
	<hotkey key="Win+Alt+Left"  action="desktop-prev"/>
	<hotkey key="Win+Alt+Right" action="desktop-next"/>
	<hotkey key="Win+Alt+1"     action="desktop-move" desktop="1"/>
	<hotkey key="Win+Alt+2"     action="desktop-move" desktop="2"/>
</hotkeys>
```

On an overlay tile, `desktop-next` and `desktop-prev` take the window along. From a
hotkey they only switch.

Windows has no public interface for any of this, so y-snap uses a private one that
can change with a Windows update. See
[Troubleshooting](#troubleshooting) if these actions stop working.

---

## Mouse shortcuts

A modifier plus a button, a wheel tilt or a side button fires one action. None are
set by default, so a fresh install changes nothing about your mouse. Add them on
the *Mouse shortcuts* page of the settings app, or in the file:

```xml
<mouse>
	<drag   button="left"  mod="Alt"/>
	<resize button="right" mod="Alt"/>
	<shortcut button="middle"      mod="Win"  action="overlay"/>
	<shortcut button="wheel-right" mod="none" action="pin-toggle"/>
	<shortcut button="x1"          mod="Alt"  action="desktop-move" desktop="2"/>
</mouse>
```

- `mod="none"` works for the middle button, tilts and side buttons, and **replaces
  that button's normal job everywhere**. A plain left or right click cannot be taken.
- Modifiers must match exactly, as for dragging.
- The side buttons and tilt must reach Windows as such. Software that came with your
  mouse may turn them into keystrokes, which y-snap cannot see.
- When the overlay opens from a `Win` shortcut, `Win` is still held, so pressing a
  tile's letter is a `Win` combination that goes to Windows. Let go of `Win` first,
  or click the tile.

The full rules are in the [configuration reference](configuration.md#shortcut).

---

## Changing the configuration

**With the settings app.** Double-click the tray icon, or choose **Settings…**. It
has a page each for *Hotkeys*, *Mouse gestures*, *Mouse shortcuts*, *Action overlay*
and *Minimize button*, and y-snap applies what you save without being restarted.
Every page is described in the [settings app's manual](../../y-snap-config/docs/user-manual.md).

**By hand.** Choose **Edit y-snap.xml**, save, then choose **Reload configuration**.
y-snap does not notice an edit on its own.

The file is `%APPDATA%\Ypo-Lite\y-snap.xml`, or `y-snap.xml` next to `y-snap.exe` if
there is one, which is how a portable copy works. *About* shows which one is in use.

A few things people change first:

**Add a hotkey.** `key` is written the way it reads, and
[the reference](configuration.md#keys) lists the key names. Any action works:

```xml
<hotkeys>
	<hotkey key="Win+Alt+T" action="always-on-top" target="pointed"/>
	<hotkey key="Win+Alt+H" action="uniconize-all"/>
</hotkeys>
```

**Take a key Windows owns.** `steal-from-shell="true"` claims a combination that
Windows or another program holds. It watches every keystroke on the machine, so use
it only where you need it:

```xml
<hotkeys>
	<hotkey key="Win+E" action="overlay" target="pointed" steal-from-shell="true"/>
</hotkeys>
```

**Stop the minimize button's right click from hiding windows.** Keep the popup, and
turn off the right click:

```xml
<minbutton hover-delay="700" right-click-action="none">
	<action action="traynize" label="Traynize" icon="📤"/>
</minbutton>
```

**Turn the minimize button off altogether.** Delete the whole `<minbutton>` section.
An empty `<minbutton/>` is not the same: it still hides windows on a right click,
unless it says `right-click-action="none"`.

**Add an overlay tile.** Give it a key, an action and a label:

```xml
<overlay>
	<action key="Q" action="uniconize" label="Bring back" icon="↩️"/>
</overlay>
```

**When something is wrong in the file.** A mistake that is not a broken file, such
as an unknown action or a key that does not parse, is a warning: a balloon lists up
to six, and everything else still loads. y-snap has no log file, so if you missed the
balloon, choose **Reload configuration** to see it again.

Broken XML, such as a tag left open, or a missing file, is more serious. The balloon
gives the reason and the line, the tray menu keeps showing it, and until the file
loads y-snap has **no hotkeys, no gestures and no overlay**, even if it was working a
moment ago. Windows already hidden can still be brought back from the tray menu.

---

## Not there yet

- **A window switcher**, on the wheel or otherwise: there are
  [options](design/switcher-plan.md), nothing built;
- **recovering hidden windows** after y-snap is killed;
- **choosing how far `transparency` fades.**

---

## Troubleshooting

**A window disappeared.** You probably right-clicked its minimize button, or used
`iconize` or `traynize`. Look under **Hidden windows** in the tray menu, then for an
icon of its own in the notification area, including the overflow area.

**A hotkey does nothing.** Another program holds it: the start-up balloon says so
after each reload. Choose another combination, or add `steal-from-shell="true"`. If
it worked for a moment after an edit, see the end of
[Changing the configuration](#changing-the-configuration): the tray menu says when
the file did not load.

**A shortcut stopped working in another program.** y-snap holds it now, in every
program. The defaults include `Ctrl+Shift+P`, which many editors use, and `Ctrl+Space`.
Change or remove them on the *Hotkeys* page.

**Nothing happens on one window.** It may be running as administrator: Windows
keeps y-snap from acting on it unless y-snap runs as administrator too. While such a
window is in front, dragging, resizing and mouse shortcuts do not work at all, on
any window. Only `iconize` and `traynize` tell you so.

**The Fade tile does nothing.** That window handles its own transparency, and y-snap
leaves those alone.

**The minimize popup does not appear, or a right click on the button does nothing.**
y-snap works out where the button should be, then checks with the window. A window
that draws its own title bar can fail that check. The popup also needs the pointer
to keep still for the whole `hover-delay`.

**The overlay opens and closes at once, from a mouse shortcut.** Windows refused it
the keyboard focus, and without focus you could not close it, so y-snap closes it
itself. Try again, or open it from a hotkey.

**Pressing a tile's key does something else.** If a `Win` combination opened the
overlay, `Win` is still down, and `Win`+letter belongs to Windows. Let go of it
first.

**The virtual desktop actions stopped working.** Most likely a Windows update changed
the private interface they rely on. Nothing else in y-snap is affected. Please
[open an issue](https://github.com/ypo-lite/ypo-lite/issues) with your Windows build
number.

**My edits have no effect.** Choose **Reload configuration** after saving by hand.
If that changes nothing, y-snap may be reading another copy, for instance a
`y-snap.xml` next to `y-snap.exe`: *About* shows the path.
