# y-coustic-config — UI checklist

What has to be looked at in a real window, screen by screen, resized through
each threshold down to the minimum (`shell::MIN_WINDOW`, 560 × 472 with the
title bar). Nothing here can be checked by reading the code: the user runs
`cargo run -p y-coustic-config` and reports.

Status: ⬜ to check · ✅ checked · ✗ wrong (say what)

## Shell

| # | Check | Status |
| --- | --- | --- |
| S1 | Opens on the screen it was closed on; a first run opens on General | ⬜ |
| S2 | The pane shows the seven screens with labels from 1000 px, and as an icon rail (names as tooltips) below | ⬜ |
| S3 | The pane footer shows the file path when labelled; on the rail it is in the status tooltip | ⬜ |
| S4 | A second launch (the tray's *Settings…* again) brings the open window forward instead of opening another | ⬜ |
| S5 | With no `y-coustic.xml` the status says *No y-coustic.xml yet: Save creates it* and Save creates a commented file | ⬜ |
| S6 | A file with problems shows the warnings banner above every screen; closing it keeps it closed until Reload | ⬜ |
| S7 | A file that is not XML shows *Couldn't load* with the reason, and Reload picks it up once fixed | ⬜ |
| S8 | Save is off while clean; after Save with y-coustic running the status says *Saved and applied* and y-coustic reloads; without it, *y-coustic isn't running* | ⬜ |
| S9 | Reload with unsaved changes asks *Discard unsaved changes?* inline (nothing to discard yet: needs a screen that edits) | ⬜ |
| S10 | *Start with Windows* in the bar acts at once and matches the tray menu's | ⬜ |
| S11 | The window has no icon in its title bar, a Mica backdrop, and cannot be sized below the minimum | ⬜ |

## General

| # | Check | Status |
| --- | --- | --- |
| G1 | *Volume step* shows the file's value; the slider and the box stay in step, and opening the screen does not mark the file *Unsaved* | ⬜ |
| G2 | Moving the slider or typing in the box changes the value (1 to 30; 0 or 99 is brought to the nearest) and marks the file unsaved | ⬜ |
| G3 | Both *When something else changes the volume* rows show the file's choice; on the master, *Tray icon* is greyed with its reason | ⬜ |
| G4 | The two settings cards keep the choice beside their text from about 460 px of card, and under it below; nothing clipped at 560 px | ⬜ |
| G5 | *Open folder* opens the folder, *Edit XML…* opens the file; both greyed when the file does not exist yet | ⬜ |

## Tray icon

| # | Check | Status |
| --- | --- | --- |
| T1 | The hover slider reads 0 to 5000; at 0 the wheel switch is greyed with its reason | ⬜ |
| T2 | The three click combos show the file's actions (none of them empty); changing one marks the file unsaved | ⬜ |
| T3 | The note about the double-click wait appears only while a double-click action is set | ⬜ |
| T4 | *The icon itself*: both choices, the chosen one accented | ⬜ |
| T5 | After Save and with y-coustic running, a click and the wheel behave as set | ⬜ |

## Mixer

| # | Check | Status |
| --- | --- | --- |
| M1 | *Where it opens* shows the file's anchor; *Rows before it scrolls* 3 to 20; *System sounds* the file's switch | ⬜ |
| M2 | After Save, the mixer opens where, and as tall as, the settings say | ⬜ |

## Keys

| # | Check | Status |
| --- | --- | --- |
| K1 | The *Replace the Windows volume indicator* card shows only when a group has volume keys, and matches the tray menu's toggle after a save | ⬜ |
| K2 | One card per group, titled *Group N*, with ↑ ↓, *Duplicate* and *Delete* in its header; the keys are keycaps, an empty one a dash, and the ✕ is greyed on a group's last key | ⬜ |
| K3 | Clicking a key opens the dialog over the whole window (bar included), starting on that key's category; OK is greyed until there is a key; Cancel changes nothing | ⬜ |
| K4 | *Bare volume keys* appears only for a group with a keyboard volume key | ⬜ |
| K5 | *Controls* is *One target* or *Layers by modifier*: a target shows one combo; layers show the table with four chips each, *none* held being the bare keys | ⬜ |
| K6 | Switching a group from layers to one target keeps the first layer's target and offers *Undo* when it drops layers; switching back gives one layer with no modifier | ⬜ |
| K7 | *Copy the layers of group N* appears only for other groups that have layers and differ; clicking it replaces this group's layers (and offers *Undo* if it had several) | ⬜ |
| K8 | A microphone-only mute is a group with one key and *One target: Microphone*; it has none of the media group's layers | ⬜ |
| K9 | Deleting a group or a layer offers *Undo*; a new group starts as one target, the master | ⬜ |
| K10 | A hotkey's *Level %* shows only for *Set the level*; *On* is a dash for *Open the mixer*; switching away and back keeps the level | ⬜ |
| K11 | A key used twice shows a warning in the dialog (and OK still works) and in the readout, which names the group | ⬜ |
| K12 | Cards and the layers table reflow below their minimum width; nothing is clipped at 560 px and the dialog fits | ⬜ |
| K13 | After Save with y-coustic running, the new keys work and the old ones no longer do; a file with a top-level `<layer>` shows the warning and Save keeps it as a comment | ⬜ |

## Mouse

| # | Check | Status |
| --- | --- | --- |
| U1 | One row per binding, in the file's order; *Listens to* shows the right option for each (wheel, wheel tilted, the five buttons) | ⬜ |
| U2 | A wheel shows *Reverse direction* under *Does*; a button shows its action; *Set the level* adds a level box; *Open the mixer* has no target | ⬜ |
| U3 | Changing a row between a wheel and a button gives it what the new kind needs (a button an action, a wheel none), and a main button or wheel that had no modifier gets *Alt* | ⬜ |
| U4 | The last modifier chip of a wheel or main button cannot be switched off; a side button (X1, X2) may have none | ⬜ |
| U5 | Two rows with the same input and modifiers: the later one says it never fires | ⬜ |
| U6 | ↑ ↓ ✕ on each row; deleting offers *Undo*; *Add a wheel binding* and *Add a button binding* append *Alt + wheel* and *Alt + middle button, mute* | ⬜ |
| U7 | The table becomes cards below its minimum width; nothing clipped at 560 px | ⬜ |
| U8 | After Save with y-coustic running, the bindings work and the removed ones no longer do | ⬜ |

## HUD

| # | Check | Status |
| --- | --- | --- |
| H1 | Five style cards, each a picture of that style at the current look; the chosen one is accented; clicking one changes the style and every picture | ⬜ |
| H2 | The position grid shows the nine places and *Follow the pointer*, the chosen one accented; the arrows read right | ⬜ |
| H3 | Size, corner radius, border, opacity, stays and fade: each slider and its box agree, stay inside their range, and the preview follows at once; opening the screen does not mark the file *Unsaved* | ⬜ |
| H4 | The preview shows master, microphone live (red bar), microphone muted and *No microphone*; *Dark* switches them between the themes whatever Windows is set to | ⬜ |
| H5 | The preview matches the real HUD: compare a card with y-coustic's own HUD in the same style, theme and size | ⬜ |
| H6 | Typing a font name changes the preview; an unknown name falls back to Windows' default font without an error; clearing the box keeps the last font | ⬜ |
| H7 | *Use it with the pointer* off shows the note that the tray hover needs it | ⬜ |
| H8 | Side by side from about 800 px of content, the preview beside the fields; below that the preview comes first; nothing clipped at 560 px, the picture of the tall bar and the ring included | ⬜ |
| H9 | Colours set on the Colours screen (once it exists) or in the file show in every picture | ⬜ |

## Colours

| # | Check | Status |
| --- | --- | --- |
| C1 | Four sets (Everything, Master volume, Microphone, Application), five swatches each; an unset swatch reads *inherited: #…* with the colour y-coustic draws; a set one reads its own | ⬜ |
| C2 | Clicking a swatch opens the picker under that set, the swatch accented; clicking it again, or *Cancel*, closes it, *Cancel* putting the colour back | ⬜ |
| C3 | Opening a swatch does not mark the file *Unsaved* until the colour really changes; the picker has no alpha slider | ⬜ |
| C4 | The preview (master, microphone live, microphone muted, application) follows each change at once and matches the swatches; *Dark* switches the theme of both the swatches and the pictures, and is shared with the HUD screen | ⬜ |
| C5 | The general *Bar* colour reaches the master and the application but not the microphone's live bar; the microphone's own does | ⬜ |
| C6 | The ↺ under a set swatch goes back to what it inherits; *Reset all* clears the set and offers *Undo* | ⬜ |
| C7 | Saving writes `<colors>` and y-coustic shows the colours after a reload, in the HUD and the mixer | ⬜ |
| C8 | Side by side from about 800 px, stacked below (preview first); five swatches reflow; nothing clipped at 560 px | ⬜ |

## The HUD itself (Phase 7: the painter moved into the library)

Not a screen of the editor: what y-coustic draws must look as it did before the move. Run `y-coustic` and check.

| # | Check | Status |
| --- | --- | --- |
| P1 | The HUD in each of the five styles (`bar`, `vbar`, `ring`, `text`, `compact`) looks as before: glyph, bar or ring, percentage | ⬜ |
| P2 | Light and dark theme, a border, a radius, an opacity below 100, and a fade still work | ⬜ |
| P3 | A microphone is red while live; *Muted* is grey; with `<colors>` set, the colours follow | ⬜ |
| P4 | An application on the HUD shows its icon and name in the `bar` style | ⬜ |
| P5 | The mixer (its bars, glyphs, colours, scrollbar) looks as before | ⬜ |
