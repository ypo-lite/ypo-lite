# y-coustic-config — user manual

y-coustic-config is the settings app of [y-coustic](../../y-coustic/README.md). It
edits `y-coustic.xml` for you: the volume step, the keys and their layers, the
mouse, the tray icon, the mixer, the HUD and the colours. When you save, a running
y-coustic applies the change at once.

Everything the app edits is also in the file, and the
[configuration reference](../../y-coustic/docs/configuration.md) describes it
attribute by attribute. This manual is about the app.

- [Opening it](#opening-it)
- [The window](#the-window)
- [General](#general)
- [Keys](#keys)
- [Mouse](#mouse)
- [Tray icon](#tray-icon)
- [Mixer](#mixer)
- [HUD](#hud)
- [Colours](#colours)
- [Saving, and what is kept](#saving-and-what-is-kept)
- [Troubleshooting](#troubleshooting)

---

## Opening it

Choose **Settings…** in y-coustic's tray menu. The app is `y-coustic-config.exe`,
in the same folder as `y-coustic.exe`; it also starts on its own, with or without
y-coustic running. Without it next to y-coustic, **Settings…** opens the XML file
instead.

Only one window is ever open: starting it again brings the open one forward. The
app reopens on the screen you left it on (remembered in `y-coustic-config.ini`,
next to the XML, never inside it).

---

## The window

<!-- Add once taken: ![The window](images/y-coustic-cfg_window_01.png) -->

- **The list on the left** has the seven screens. On a narrow window (under about
  1000 px) it shrinks to icons; hover one for its name. Below the list, *Loaded
  from* shows which `y-coustic.xml` is being edited.
- **The bar at the bottom** says whether the file is saved. Nothing pending, it
  counts the hotkeys and mouse bindings; with changes, it says *Unsaved changes*.
  Its buttons:
  - **Start with Windows** registers y-coustic to start when you sign in. It acts
    at once, does not wait for Save, and matches the tray menu's item.
  - A message about the last thing you did, such as *Deleted a hotkey*, with
    **Undo**. Undo goes back one step.
  - **Reload from disk** reads `y-coustic.xml` again. With unsaved changes it asks
    *Discard unsaved changes?* first.
  - **Save**, available once something has changed.
- **Warnings.** If the file has something y-coustic would warn about, a banner above
  every screen lists it, and the entries it concerns are marked. Close the banner
  and it stays closed until the next Reload.

---

## General

The volume step, what to do when something else changes the volume, and the file.

| Setting | |
| --- | --- |
| **Volume step** | How many percent one press of a volume key, or one notch of the wheel, moves the level. A slider and a box, 1 to 30. |
| **Speakers and headphones** | What y-coustic does when something *other* than y-coustic changes the master volume: show it on the **HUD**, keep the **Tray icon** true, or nothing (**Off**). Off by default, since Windows already shows its own flyout. *Tray icon* is greyed out for the master, which has no icon variant. |
| **Microphone** | The same, for the microphone: the Sound panel, a laptop's microphone key. On by default, because believing you are muted when you are not costs the most. |
| **Configuration file** | The path of `y-coustic.xml`, with **Open folder** and **Edit XML…**, both greyed out until the file exists. |

*Off* watches nothing, and nothing stays resident for it.

---

## Keys

Volume keys in **groups**, then single hotkeys.

**Replace the Windows volume indicator.** On, the bare volume keys are y-coustic's
and show its HUD instead of the Windows flyout. Off, Windows keeps them and
y-coustic takes only the layers with a modifier, such as `Shift` for the
microphone. The card shows only when a group has keyboard volume keys.

### A group

Each group is a card titled *Group 1*, *Group 2*..., with ↑ ↓ to reorder,
**Duplicate** and **Delete** in its header. It has three keys, *Up*, *Down* and
*Mute*, shown as keycaps (an empty one is a dash, and the ✕ removes a key, except
the last). **Add a group** starts a new one, aimed at the master volume.

**Choosing a key.** Click a keycap and a dialog opens over the window, on that
key's category. **OK** is greyed out until a key is chosen; **Cancel** changes
nothing. If the combination is already used, the dialog warns, and still lets you
keep it.

**Bare volume keys** (only for a group with a keyboard volume key): *Windows keeps
them* or *y-coustic takes them*.

**Controls** says what the group's keys act on:

- **One target**: one choice, *Master volume*, *Microphone*, *Focused application*
  or *Application under the pointer*.
- **Layers by modifier**: a table, one layer per row. Each layer has four chips,
  `Win`, `Ctrl`, `Alt`, `Shift`, for the modifiers held, and a target. *None* held
  is the bare keys. **Add a layer** appends one; each row has ↑ ↓ and ✕.

Switching from layers to one target keeps the first layer's target; switching back
gives one layer with no modifier. Both offer **Undo** when something is dropped.
When other groups have layers, **Copy the layers of group N** copies them here.

A microphone-only mute is a group with one key and *One target: Microphone*.

### Single hotkeys

One key for one thing, for whatever the groups do not cover. Each row has the key,
the modifiers held, what it **does** (*Volume up*, *Volume down*, *Mute*, *Set the
level*, *Open the mixer*), the target it acts **on**, and, for *Set the level*, a
**Level %**. **Add a hotkey** appends one.

### The warning markers

Beside a key or a layer, a glyph says that a combination is used twice (the readout
names the group), that a layer uses `Alt`, or that a group controls nothing. They
never stop you. A line at the bottom, *What y-coustic registers: N*, counts the
keys that will be registered. The app does not know whether Windows will accept a
key another program already owns: y-coustic reports that when it starts.

---

## Mouse

<!-- Add once taken: ![The Mouse screen](images/y-coustic-cfg_mouse_01.png) -->

One row per binding, in the file's order, in a table that becomes cards on a
narrow window.

| Column | |
| --- | --- |
| **Listens to** | *Wheel*, *Wheel, tilted sideways*, or one of the five buttons (left, right, middle, *Back button (X1)*, *Forward button (X2)*). |
| **Modifiers held** | The four chips. A wheel or a main button needs at least one (the last cannot be switched off); a side button may have none. |
| **Does** | A wheel shows *Reverse direction* and steps the level; a button shows its action: *Volume up*, *Volume down*, *Mute*, *Set the level*, *Open the mixer*. |
| **On** | The target, or none for *Open the mixer*. |

Changing a row between a wheel and a button gives it what the new kind needs, and
a main button or a wheel with no modifier gets `Alt`. If an earlier row has the same
input and modifiers, the later one says it never fires.

**Add a wheel binding** appends *Alt + wheel*; **Add a button binding** appends
*Alt + middle button, mute*. Each row has ↑ ↓ and ✕ (with **Undo**).

---

## Tray icon

What resting on, clicking and turning the wheel over the tray icon do.

- **Hover.** *Rest the pointer on the icon* sets, in milliseconds (0 to 5000), how
  long before the HUD appears. `0` turns it off and gives the Windows tooltip back.
  If the icon is in the overflow flyout, drag it onto the taskbar: Windows does not
  report the pointer on an icon there.
- **Clicks.** *Click*, *Double-click* and *Middle click* each take an action:
  *Open or close the mixer*, *Show the master level*, *Mute or unmute the master*,
  *Mute or unmute the microphone*, *Master volume up one step*, *Master volume
  down one step*, or *Nothing*. Leave the double-click at *Nothing* to keep the
  single click immediate; a note says so while one is set.
- **Wheel.** Turn the wheel over the icon to step the master volume. It needs the
  hover setting above, and is greyed out at `0`.
- **The icon itself.** What the icon shows, for instance a muted microphone.

---

## Mixer

The window that lists every volume: the master, the microphone and each
application.

| Setting | |
| --- | --- |
| **Where it opens** | *Next to the tray icon*, *At the pointer* or *Where the HUD shows*. |
| **Rows before it scrolls** | 3 to 20. With more, it scrolls with the selection. |
| **System sounds** | List Windows' own sounds (notifications, alerts) as the last row. |

---

## HUD

<!-- Add once taken: ![The HUD screen](images/y-coustic-cfg_hud_01.png) -->

The small window that shows the level after a change, and when the pointer rests on
the tray icon. A **Preview** is drawn by y-coustic's own painter, so it is the
real thing: the master volume, the microphone live (red bar) and muted, and a
device that is not there. Its **Dark** switch shows both themes whatever Windows is
set to, and is shared with the Colours screen. On a wide window the preview sits
beside the fields; on a narrow one it comes first.

- **Style.** Five cards, each a picture of that style at your current look: *Bar*,
  *Compact*, *Text*, *Tall bar*, *Ring*.
- **Where it appears.** A grid of the nine places on the screen, or **Follow the
  pointer**.
- **Size and look.** *Font* (a name Windows does not have falls back to its default
  font; clearing the box keeps the last one), *Size* in percent, *Corner radius*,
  *Border* width (0 is none), and *Opacity*.
- **How long it stays.** *Stays*, in milliseconds after the last change, and *Fade*,
  the milliseconds to fade in and out (0 shows and hides it at once).
- **Using it.** *Use it with the pointer*: the wheel, a click or a drag on its bar,
  a click on its icon to mute. Off, it is click-through, and the tray hover needs it.

---

## Colours

<!-- Add once taken: ![The Colours screen](images/y-coustic-cfg_colours_01.png) -->

The colours of the HUD and of the mixer, in four sets: **Everything**, **Master
volume**, **Microphone** and **Application**. Each has five swatches:
*Background*, *Text*, *Empty bar*, *Bar* and *Bar, muted*.

A set swatch shows its own colour. An unset one reads *inherited: #…*, the colour
y-coustic actually draws: a kind's setting wins over *Everything*, which wins over
the theme's own. Note that the general *Bar* reaches the master and applications,
but not the microphone's live bar, whose accent is its own.

- **Click a swatch** to open the picker under its set (there is no alpha slider);
  click it again, or **Cancel**, to close it, and Cancel puts the colour back.
- **↺** under a set swatch goes back to what it inherits.
- **Reset all** clears the set, and offers **Undo**.
- The preview follows every change at once, and **Dark** switches the theme of both
  the swatches and the pictures.

---

## Saving, and what is kept

**Save** rewrites `y-coustic.xml` from the settings and asks a running y-coustic to
reload it. The status then says *Saved and applied: y-coustic is reloading it*, or
*Saved. y-coustic isn't running; it'll use these settings when it starts*.

**Nothing in your file is lost.** Comments, attributes the app does not know and
entries y-coustic cannot use are kept (the last as comments, with a warning that
says why), and the first save of a run keeps the old file as `y-coustic.xml.bak`.
A file that still has a top-level `<layer>`, from before layers belonged to
groups, shows a warning and Save keeps it as a comment.

**Starting from nothing.** With no `y-coustic.xml` the app starts from the shipped
default, and the first Save writes it, explanations included.

---

## Troubleshooting

- **The window says *Couldn't load y-coustic.xml*.** The file cannot be read or is
  not XML (a tag left open, say); the message gives the reason and the line.
  Fix the file, then **Reload from disk**.
- **The app asks for the Windows App Runtime.** The settings apps need
  [version 2.4 or later](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads).
  The installer fetches it when needed; with the zip, install it once. y-coustic
  itself does not need it.
- **Save failed.** The bar shows the reason, such as a full disk or a read-only
  file. Nothing is changed in the app; fix the cause and save again.
- **Saved, but nothing changed.** *y-coustic isn't running* means it was not asked
  to reload; start it and it uses the file. If it is running, check its balloon for
  a warning about a line of the file.
- **A key does nothing.** Windows or another program may own that combination;
  y-coustic says *Already taken by another application* when it starts. Choose
  another key.
