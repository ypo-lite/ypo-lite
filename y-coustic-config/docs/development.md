# y-coustic-config: development

How the settings app is built. To use it, see the
[user manual](../../y-coustic/docs/user-manual.md#the-settings-app); the plan, with
what was decided and why, is [`design/plan.md`](design/plan.md); what to look at in
a real window is [`ui-checklist.md`](ui-checklist.md). Read
[`ui-kit/docs/ui-toolkit.md`](../../ui-kit/docs/ui-toolkit.md) before adding or
changing a screen.

```sh
cargo run -p y-coustic-config      # the app, on the real y-coustic.xml
cargo test -p y-coustic-config     # the screens' logic, no window needed
```

## Where things are

| File | Role |
| --- | --- |
| `src/main.rs` | `ConfigApp` (the loaded `Settings` and the page-local state), `Msg`, `update`, the shell: navigation, banner, the bar (Save, Reload, Undo) |
| `src/edit.rs` | every edit the screens make, as plain data (`Edit`) and one function (`apply`), clamped to the reader's ranges; an unchanged value is not an edit |
| `src/simple.rs` | General, Tray icon and Mixer |
| `src/keys_view.rs`, `src/key_edit.rs` | Keys: a card per group, single hotkeys, the readout with its warning markers; the dialog that chooses a key (a draft until OK) |
| `src/mouse_view.rs` | Mouse |
| `src/hud_view.rs` | HUD: the style cards, the fields, the preview and its cache |
| `src/colours.rs`, `src/colours_view.rs` | Colours: the edits (tested) and the swatches, picker and preview |
| `src/prefs.rs` | the last screen, in `y-coustic-config.ini` next to the XML |

Everything that is not window code is in the `y-coustic` library: the settings and
their round trip (`config`, `kept`, `edit`), the key groups and their expansion
(`controls`), the ipc names (`ipc`) and the HUD's painter (`paint`, which
`hud_view` and `colours_view` call through `render_png`).

## Rules that held

- **A screen is a function of the settings, its own state and a `Viewport`.** No
  literal sizes; `AutoGrid`, `Split` and `Table` for anything that reflows.
- **Controls report an `Edit`; `update` applies it.** WinUI reports a value when it
  is first set, so `apply` returns whether anything changed and only then is the
  file dirty.
- **Logic first, view second.** What can be tested without a window is: the edits,
  the key dialog, the colour rules, the origin and problem lookups.
- **Nothing is lost silently.** The library keeps comments and what it cannot use;
  the banner lists what y-coustic would warn about; removing something offers Undo.
- **Every action is a real control** (`Button`, `ToggleButton`, `ComboBox`,
  `NumberBox`, `Slider`, `ToggleSwitch`, `TextBox`, `ColorPicker`). Nothing is
  clickable only through a pointer handler, so every one can be reached with Tab
  and used with Enter or Space: `windows-reactor` has no key events, and this is
  how the app stays usable without a mouse.
- **A combo box that would keep showing its last choice is a button.** *Copy the
  layers of group N* is one button each.

## Limits found

- `windows-reactor` has no key events, no hover states and a brush without
  gradients; none is needed by these screens, which is why they stay plain.
- The preview is drawn at twice the layout's dpi and shown at half, so it is sharp
  on a scaled display. It cannot draw an application's icon and name without an
  executable, so there is no application sample with an icon.
- The warning markers say what `expand` found; they do not check that a key can be
  registered by Windows (a key another program owns is found only when y-coustic
  runs).
