# y-coustic-config — plan

The settings app for `y-coustic.xml`: the third WinUI 3 config app after
`y-snap-config` and `y-pop-config`, on the same stack (`windows-reactor`, the
`ui-kit` toolkit, `y_core`). This is Phase 9 of
[`y-coustic/docs/design/implementation-plan.md`](../../../y-coustic/docs/design/implementation-plan.md);
that file says what y-coustic does, this one says how the editor is built and in
which order. It is modelled on `y-pop-config/docs/design/plan.md`, which is
the better precedent of the two (see D1).

Each phase ends with something that builds, passes
`cargo clippy --workspace --all-targets` with no warnings and
`cargo test --workspace`, and can be merged on its own. The user commits and
runs the live checks; a session never launches the app (global rule). This file
is the running progress record: change a row's status in the same change as the
work.

Status: ⬜ todo · 🔨 in progress · ✅ done · 👁 done, verify live

## Where we are

**Phases 1 to 3 are built** (the library round trip, `ipc`, reload and *Settings…*, and the crate with its shell). Its screens are placeholders; Phase 4 (General, Tray icon, Mixer) is built, with `edit.rs` (every edit as data and one tested function, clamped to the reader's ranges, an unchanged value is not an edit). Phase 5 (Keys) is built and was reorganised (a card per group, see the keys and layers plan); Phase 6 (Mouse) is built; Phase 7 (the HUD painter in the library) is built; Phase 8 (the HUD screen with its preview) is built; Phases 1 to 10 are built apart from the live pass (10.5): every screen exists, the problem markers and the docs are done. What is left is the user's: the live checks in `ui-checklist.md`, and the two plans' open questions. The live checks are in `y-coustic-config/docs/ui-checklist.md`. Before it, the user may want to check that y-coustic still loads and saves the shipped file as before.

The rest is not built. y-coustic's schema has held still since Phase 8 (the
colours, the fade and the tray icon rule were the last additions), which was the
condition for starting. Everything an editor needs from the application is
already a library: `y_coustic::config` (`Settings`, `load_from`, `parse`,
`to_xml`, `save_to`, `warnings`), `controls`, `input`, `model`, `appearance`.

## What the editor has to cover

One screen per section of the file, in the navigation pane's order:

| Screen | Edits | Notes |
| --- | --- | --- |
| General | `<volume step>`, `<watch>` (master, microphone: off / tray / hud), start with Windows, where the file is (Open folder, Edit XML…) | the *Replace the Windows volume indicator* switch lives on Keys, where the keys are |
| Keys | `<controls>` groups (up, down, mute, `plain`) and layers (modifiers, target); `<hotkeys>` (key, action, target, value) | a read-only "what this registers" expansion (`Settings::bindings`) under the layers, since a group × layers product is hard to picture |
| Mouse | `<mouse>`: wheel and button bindings (input, modifiers, target, action, invert) | order is the file's; conflicts between two bindings are marked |
| Tray icon | `<tray>`: `hover-delay`, `click`, `double-click`, `middle-click`, `wheel`, `icon` | the three click actions as `ComboBox`es of `TrayAction`; a note that `wheel` needs the hover |
| Mixer | `<mixer>`: `anchor`, `max-rows`, `system-sounds` | small; shares a page with nothing else |
| HUD | `<hud>`: style (five cards), position (a 3 × 3 grid and *follow cursor*), timeout, interactive, scale, radius, border, opacity, font, fade | with a live preview (Phase 7) |
| Colours | `<colors>`: panel, text, track, accent, muted for everything, then for master, microphone and application | swatches with an inline `ColorPicker`, a reset each, the preview of the three kinds side by side |

Not in the first version: editing comments (they are kept, never shown, see
D1), and a "test" button that makes the running y-coustic flash the HUD (an idea
for after Phase 10).

## Decisions

Taken for this plan. **D1 and D3 are worth a second look.**

1. **Nothing in the user's file is lost silently, and comments are kept.**
   `y-snap-config` rewrites the file and says so (comments and hand formatting
   go). y-coustic's shipped file is mostly explanation, so a Save that deleted it
   would be a poor first thing for the app to do. Follow `y-pop`: `Settings`
   keeps the comments, unknown attributes and elements it cannot read, and writes
   them back; what y-coustic would warn about is a banner above every screen.
   `y_core::xml` already has comment nodes and `xml::Writer` (y-pop's Phase 1).
   Scope: a comment directly above a section, a `<keys>`, `<layer>`, `<hotkey>`,
   `<wheel>` or `<button>` stays with it (*attached*, moves with it); any other
   comment is a free entry that keeps its place in its list; comments before the
   root are file notes. Comments are not editable in this version.
2. **`Settings` grows the round trip in the library, not in the app.** The
   writer moves from hand-built strings (`to_xml`, `controls::write_xml`) onto
   `xml::Writer`, as y-pop's did, so the tray application and the editor save the
   same bytes. A golden test: the shipped `rsc/y-coustic.xml` → `Settings` →
   `to_xml` → `load` has no warnings, the same values and the same comments, and
   saving twice gives identical bytes.
3. **The HUD preview is drawn by the real painter**, as y-pop-config's skin
   preview is: no second copy of the drawing rules. That means the painting
   (`hud::draw`, `Palette`, the glyphs) moves out of the binary into the library,
   which touches the tray application. It is the largest change outside the new
   crate (Phase 7) and is kept apart so the other screens do not wait for it.
   The fallback, if it proves too invasive, is a preview of the layout only
   (boxes, no glyphs) and a note.
4. **An `ipc` module in the library**, as `y_snap::ipc` and `y_pop::ipc`:
   `CLASS`, `CONFIG_EXE`, `CONFIG_INSTANCE`, `CONFIG_TITLE`, `reload_message()`,
   `request_reload()`, `is_running()`. Save writes the file and asks a running
   y-coustic to reload; a broken file keeps the previous configuration there and
   says so (y-coustic's `load` already behaves this way).
5. **The tray menu gets *Settings…***, launching `y-coustic-config.exe` from
   next to `y-coustic.exe` and falling back to opening the XML with a note, as
   y-snap's does. *Open configuration* stays for the file.
6. **The window has no icon**, like the other config apps, until the user
   supplies an `.ico` (`no-shipped-ico`).
7. **A crate of its own**, `y-coustic-config`, a workspace member beside
   `y-pop-config`; `y-coustic` stays free of `windows-reactor`.
8. **Screens follow `ui-kit/docs/ui-toolkit.md` to the letter**: a `Viewport`
   in, no literal sizes, `AutoGrid`/`Split`/`Table` for anything that reflows,
   keyed children, and its trap table (multi-line `TextBox`, clipboard flush,
   `NumberBox` reporting on first set) read before the first screen. A
   `ui-checklist.md` for this app from the first screen on.
9. **A mockup first?** y-pop-config had a `spec.md` and a `mockup.html`;
   y-coustic's screens are mostly forms and tables the toolkit already shapes,
   so the plan does without, and each phase describes its screen in words and in
   the table above. If the user wants a mockup for Keys, Mouse or the HUD page,
   it is made before that phase, not after.

## Phase overview

| Phase | Delivers | Depends on |
| --- | --- | --- |
| 1 | `y_coustic` library: comment-keeping `Settings`, writer, safe save, edit functions | — |
| 2 | `ipc`, reload in y-coustic, *Settings…* in its tray menu | 1 |
| 3 | The crate and its shell: window, navigation, banner, Save / Reload / Undo | 1, 2 |
| 4 | General, Tray icon and Mixer screens | 3 |
| 5 | Keys screen | 3 |
| 6 | Mouse screen | 3 |
| 7 | The HUD painter in the library; a PNG of a HUD | 1 |
| 8 | HUD screen with the preview | 3, 7 |
| 9 | Colours screen | 3, 7 |
| 10 | Polish, docs, live pass | all |

Phases 4, 5, 6 and 7 are independent once their dependencies are done; 1 → 2 →
3 → 4 is the critical path (it gives a working editor with the simple screens).

---

## Phase 1 — An editable `Settings`

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 1.1 | Comments and unknown content in the model | ✅ | `Settings` gains `prolog` (file notes), and each list (`controls.keys`, `controls.layers`, `hotkeys`, `mouse`) becomes entries that are an item or a comment, with an attached comment on the item (`lead`), as y-pop's `Entry`. The public shape the application uses stays: `bindings()` and the typed fields read the items only, so `y-coustic` and its tests are untouched. Unknown attributes are kept as `extra_attrs` per element and written back (still warned); an element the reader rejects (unknown action, bad key) becomes a *disabled* comment holding its original XML and is listed in the banner, as y-pop's 2.3. | **As built:** `kept.rs` (`Kept`, `Part`, `Note`, `Section`); the typed lists stay as they were, so the application and its tests did not change. Simplification: a comment in a list belongs to the entry after it and moves with it (no free comments that stay in place); comments after the last entry are the list's tail; the blank lines are remembered. Unknown attributes are `Part::extra`; an unusable element, an unknown root element or a second copy of a section becomes a comment of its own XML ahead of the next entry. Colour children keep no extras. |
| 1.2 | `to_xml` on `xml::Writer` | ✅ | Canonical attribute order per element, defaults omitted or written as today (decide with the golden test: the shipped file writes every attribute, so keep writing them), tabs, one blank line between sections. Replaces `to_xml`, `controls::write_xml` and `write_mouse_binding`. `escape` and `color_hex` go to one place. |
| 1.3 | Round-trip tests | ✅ | (a) the shipped file → `Settings` → `to_xml` → `load`: no warnings, equal values, equal comment count and text. (b) saving twice gives identical bytes. (c) an unknown attribute and a rejected element survive. (d) a hand-written file with comments in each list. |
| 1.4 | Safe save | ✅ | `save_to` writes a temporary file in the folder and renames it over the original; one `y-coustic.xml.bak` from before the first save of a session. |
| 1.5 | Edit functions, pure and tested | ✅ | In the library, in `edit.rs`: add, remove, move up/down, duplicate for each list; `set_key`, `toggle_modifier`, `set_action`; the duplicate-hotkey and binding-conflict readouts (what `bindings()` would collide on); `step`, the ranges and the clamping already in `config.rs` reused, not copied. Every function returns what changed so the app can offer Undo and say it. | **As built** (`edit.rs`, 8 tests): `List` (Keys, Layers, Hotkeys, Mouse); `insert_keys/layer/hotkey/mouse`, `remove_entry`, `move_entry_up/down`, `duplicate_entry`, each changing the values and their `Part`s together (a comment moves with its entry; removing an entry removes its comments); `crossing_warnings` (a combination used twice, an Alt layer), `shadowed_mouse_bindings`; `toggle_modifier` / `has_modifier`. Setting a key, an action or a target is a field write, so it has no function of its own. The ranges and clamping stay where the reader has them (`config::*_RANGE`). |

**Done when** the shipped file survives a load and a save with its comments, and
an edit made through `edit.rs` loads back in y-coustic's own reader.

## Phase 2 — Talking to y-coustic

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 2.1 | `y_coustic::ipc` | ✅ | D4. The class name is y-coustic's single-instance name; `CONFIG_TITLE` is `y-coustic — Configuration`. **As built:** `y_coustic::ipc` (`CLASS`, `STARTUP_NAME`, `CONFIG_EXE`, `CONFIG_INSTANCE`, `CONFIG_TITLE`, `reload_message`, `is_running`, `request_reload`); y-coustic's own class and start-up names now come from it. |
| 2.2 | y-coustic reloads on the message | 👁 | The handler calls the same `load` as the tray's *Reload configuration*; a broken file keeps the old settings and shows the error. Deferred while the mixer is open (as y-pop's popup). 👁 **As built:** the handler is the same `load` as the tray's *Reload configuration*, in `on_message`; a broken file keeps the old settings and shows the error. **Not deferred while the mixer is open:** the mixer takes its settings when it opens and runs hotkeys through the same index list, so a reload under it changes nothing it uses; revisit if a live check says otherwise. |
| 2.3 | *Settings…* in the tray menu | 👁 | D5. Launch from next to the executable, else open the XML with a note. A second launch of the editor brings the open one forward. 👁 **As built:** *Settings…* above *Reload configuration*; `open_settings` and `settings_app` in `main.rs`, the fallback opens the file with a note. The editor bringing its open window forward on a second launch is Phase 3. |

## Phase 3 — The crate and its shell

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 3.1 | Scaffold | ✅ | Workspace member; depends on `windows`, `windows-reactor`, `y-core`, `ui-kit`, `y-coustic` (lib). `build.rs` and `rsc/` as `y-pop-config` (version info, no icon, D6). Single instance on `ipc::CONFIG_INSTANCE`; minimum window `shell::MIN_WINDOW`. **As built:** workspace member; no `.ico` and no manifest in `rsc/` (version info only), the title bar is icon-less. The single-instance guard and `bring_to_front` are in `main`. The installer (`installer/ypo-lite.iss`) and `scripts/release.ps1` do not list the app yet: packaging is the user's call. |
| 3.2 | App state | 👁 | `ConfigApp { settings, path, screen, undo: Option<Settings>, dirty, status, picker states }`, `Screen` one variant per row of the table above. Every message settled in `update`; every screen a function of the state and a `Viewport`. **As built:** `ConfigApp` (settings, path, missing, prefs, screen, dirty, save_error, warnings_dismissed, applied, confirming_reload, undo, window width); `Screen` has seven variants with a tag, a title, a symbol and a lede. A missing file starts from `config::DEFAULT_XML`, so the first Save writes the commented default. |
| 3.3 | Shell | 👁 | `ui::shell`: a `NavigationView` with the seven screens (labelled at wide widths, icon rail below), the page frame, and `warnings_banner` listing load warnings and kept-as-comment elements. **As built:** `NavigationView` of the seven screens, labelled from 1000 px and an icon rail below, the path in the pane footer; each screen is a placeholder (`This screen is not built yet`) until its phase. |
| 3.4 | Action bar | 👁 | `shell::action_bar`: Save (accent, off when clean) → `save_to` → `ipc::request_reload` → "Saved and applied" or "Saved — y-coustic isn't running"; Reload with an inline confirm over unsaved changes; one-level Undo (a snapshot before a destructive edit); `startup_toggle`. **As built:** `shell::action_bar`: Save, Reload with the inline discard question, Undo link, *Start with Windows*. Save writes through `Settings::save_to` and asks y-coustic to reload (`ipc::request_reload`). The tests save into a temp folder and so also send that message: a running y-coustic reloads, harmlessly. |
| 3.5 | Per-user UI settings | ✅ | The last screen, in `y-coustic-config.ini` next to the XML (`y_core` `.ini`), never in the XML. **As built:** `prefs.rs`: the last screen, in `y-coustic-config.ini` next to the XML. |
| 3.6 | Test harness | ✅ | Messages tested through a `test_app` as `y-pop-config/src/main.rs` does, no window. **As built:** `test_app` and 10 tests: tags, a missing file, save then save again gives the same bytes, reload, undo, status, remembering the screen. |

**Done when** the app opens the real file, switches screens (placeholders),
saves unchanged settings byte-for-byte equal to a second save, and a running
y-coustic reloads.

## Phase 4 — General, Tray icon and Mixer

The simple screens: they prove the shell, the dirty tracking and the save path
before the large ones.

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 4.1 | General | 👁 | `step` as `range_input` (1 to 30); `<watch>` as two `segmented` choices (off, tray, hud) with the one-line reason for each default (Windows already shows its own flyout for the master); the file's location with Open folder and Edit XML… (`y_core::shell::open`). Values equal to the current one are ignored (`NumberBox` and `Slider` report on first set). **As built:** `simple::general_view`; *Volume step* is a `range_input` in a new `setting_wide` card (`ui-kit`: the control under the text at full width); the two watch rows are `segmented`, *Tray icon* disabled on the master with the reason; Open folder and Edit XML… are greyed while the folder or file does not exist. |
| 4.2 | Tray icon | 👁 | `hover-delay` with a *0 = off* note (`range_input`, 0 to 5000); `click`, `double-click`, `middle-click` as `combo`s of `TrayAction` with a line under `double-click` ("the single click then waits the double-click time"); `wheel` switch, disabled with a reason while the hover is off; `icon` as `segmented` (fixed, mic-muted). **As built:** `simple::tray_view`; the three click combos list the seven actions with a sentence each; the double-click note shows only while one is set; the wheel switch is disabled with its reason while the hover is 0; the icon rule is two choices. |
| 4.3 | Mixer | 👁 | `anchor` (tray, cursor, hud), `max-rows` (3 to 20), `system-sounds`. **As built:** `simple::mixer_view`. |

## Phase 5 — Keys

> **Reorganised.** A group of keys owns how it is aimed (a target, or layers of its own) instead of sharing every layer: see [`y-coustic/docs/design/keys-and-layers-plan.md`](../../../y-coustic/docs/design/keys-and-layers-plan.md). The rows below describe the first shape; 5.1, 5.3 and 5.4 are now a card per group, and there are no shared layers.

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 5.1 | Groups table | 👁 | `Table`: three `key_field`s (up, down, mute; ✕ to clear) and `plain` (windows / replace) shown only for a group that has a media key. Reorder, add, remove with Undo. The key picker is `ui_kit::key_picker` in `editor::modal`, with `KeyRules` allowing the media keys and the numpad that y-coustic accepts. **As built:** `keys_view::groups`; ✕ is greyed on a group's last key (`Edit::ClearGroupKey` refuses it); *Add a group* opens the dialog on its up key. The dialog is `ui_kit::editor::modal` with `key_picker_with(KeyRules::ANY)` (`key_edit.rs`, a draft until OK, 7 tests); a media key sets `hook_only` as the reader does. |
| 5.2 | *Replace the Windows volume indicator* | 👁 | The tray menu's toggle, here as a labelled switch on the media group (`Settings::has_media_group`, `replaces_windows_volume` and `set_replace_windows_volume`, already in the library), with the sentence that says what it does. **As built:** the card shows only while a group has volume keys, and is `Edit::ReplaceWindowsVolume`. |
| 5.3 | Layers table | 👁 | Modifiers as four `chip`s (Ctrl, Shift, Alt, Win; none = the bare keys) and `target` as a `combo` (master, mic, focused, pointed); a warning marker when the layer's modifier is Alt alone (the menu-bar focus quirk the shipped file warns about). **As built:** four `chip`s and a target combo; *Add a layer* adds `Ctrl` on the focused application. The Alt warning is in the readout, not beside the row (row markers are 10.2). |
| 5.4 | What it registers | 👁 | The expansion of `bindings()` as a read-only list of key → action → target, under the two tables, with duplicates marked (`edit.rs`, 1.5). **As built:** *What y-coustic registers: N*: every binding as keycaps and a sentence, the `crossing_warnings` above it. A key used twice is also said in the dialog, found by trying the draft in a copy of the settings. |
| 5.5 | Hotkeys table | 👁 | `key_field`, `action` combo (volume-up, volume-down, mute, set-volume, mixer), `target` combo (hidden for `mixer`), a `number_box` for `set-volume` (0 to 100). A hotkey used twice, or by a layer, says where. **As built:** `Does` and `On` combos, `Level %` shown only for *Set the level*, `On` hidden for the mixer; *Add a hotkey* makes a *mute* of the master to edit. |

## Phase 6 — Mouse

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 6.1 | Bindings table | 👁 | One row per `<wheel>` / `<button>`: kind, button (left, right, middle, x1, x2) for buttons, modifier chips, target, action (buttons) or `invert` (wheels). Reorder, add, remove with Undo. **As built:** `mouse_view`: *Listens to* is one combo of the seven inputs; the modifier chips, *Does* (a wheel's *Reverse direction*, or a button's action with a level box for *Set the level*), *On* (none for the mixer), ↑ ↓ ✕ with Undo. `Edit::MouseInput` and the others (`edit.rs`, 4 tests) give a binding what its new kind needs. |
| 6.2 | Rules shown | 👁 | A binding with no modifier takes a plain click away from every program: the row says so and asks for a confirming tick (the design's rule, and the reason a bare left or right button is not offered by default). Two bindings with the same input and modifiers are marked. **As built:** instead of a confirming tick, the editor **prevents** the bare case: switching the last modifier off a wheel or main button is refused, and changing to such an input puts *Alt* back. A shadowed binding says it never fires. |

## Phase 7 — The HUD painter in the library

The one phase that changes `y-coustic` itself (D3).

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 7.1 | Check what can move | ✅ | `hud/layout.rs` is pure already. `hud::draw`, `Palette`, the glyph and ring painters use `y_core::overlay::Canvas`, `Font` and `theme`, none of which needs a window; `Look` (icon and name of an application) needs the executable. List what is left in the binary and why before moving anything. **As built:** what moved: `hud/layout.rs` (pure, already), the palette and shapes, `draw`, `Skin`, and `look.rs` (it needs the executable, not a window). What stayed in the binary: the window (`Hud`, the timers, the fade, `forward.rs`), `Reading` (it carries the mixer's `RowId`) and the mixer's own window. `draw` takes the kind and the state instead of a `Reading`, and `Skin` gained `dark: Option<bool>` so a preview chooses its theme. |
| 7.2 | `y_coustic::paint` | 👁 | The painters and `Palette` move into the library; the binary calls them from there. No behaviour change: the HUD tests move with them, and a live pass of the HUD (the five styles, the fade) is the check. **As built:** `y_coustic::paint` (`paint.rs`, `paint/layout.rs`) and `y_coustic::look`; the binary and the mixer call them from there. The 23 painter and layout tests moved with the code and pass unchanged. **Live check:** the HUD in the five styles, with a border, with a fade, on an application (name and icon), and the mixer, look as they did. |
| 7.3 | `render_hud` | ✅ | `render_hud(&HudConfig, Kind, State, theme) -> Png`: a canvas in memory (a 32-bit DIB), the same `draw`, encoded with `y_core::png`. A sample application is a fixed icon and name (no executable read). Tested for size and for the pixel at the bar's end moving with the level. **As built:** `paint::render` (size, premultiplied pixels) and `paint::render_png` (straight-alpha PNG through `y_core::png`), for any `HudConfig`, kind, state, theme and dpi, with no window. Tests: each style at its size and scale, the bar's fill and colour by level and kind, the theme and `<colors>` coming from the caller, the corner cut, a PNG out. |

## Phase 8 — HUD screen

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 8.1 | Style cards | 👁 | Five cards in an `AutoGrid` (bar, vbar, ring, text, compact), each a `render_hud` picture at the current colours; the chosen one is Accent. **As built:** `hud_view::style_cards`: five `Button` cards (Accent when chosen) over a picture each from `paint::render_png` at the current look, in an `AutoGrid`; the pictures are drawn at 192 dpi and shown at half their pixels. |
| 8.2 | Position | 👁 | A 3 × 3 grid of buttons for the nine places and *follow cursor* beside it; the chosen one is Accent. **As built:** `position_grid`: three rows of three buttons with arrows (`Button` Accent when chosen, tooltip with the name) and *Follow the pointer* beside them. |
| 8.3 | Fields | 👁 | `timeout`, `fade`, `scale`, `radius`, `border`, `opacity` as `range_input`s (the ranges are the library's constants), `font` as a text box with the font's name checked (a missing family falls back silently: say so), `interactive` switch with the note that it turns the tray hover off. **As built:** `look`, `timing`, `using`: size, radius, border, opacity, stays and fade as `range_input` in `setting_wide` cards (the ranges are the library's); the font as a text box that ignores an empty box (`edit::set_font`); *interactive* as a switch, with the note about the tray hover. `Edit::Hud*` (2 tests). |
| 8.4 | Preview | 👁 | A `Split` with the fields; the preview shows the master and the microphone (live and muted) at the settings in effect, scaled to its `Viewport`, redrawn as a field changes, light and dark by a switch. Cached by settings, as `skin_preview`'s pictures are. **As built:** `HudUi` keeps the preview's theme (the system's at first) and a cache of the pictures, keyed by the settings that change a picture (not where it appears, how long it stays, the fade or whether it takes the pointer), bounded at 48. Side by side from `Split` minimums 440 and 300; stacked, the preview comes first. The samples are master, microphone live, microphone muted and a missing device; an application is not shown because it needs an executable's icon. |

## Phase 9 — Colours

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 9.1 | Swatches | 👁 | Five swatches per set (everything, master, microphone, application) that read "theme: #…" until set, a reset each; the `ColorPicker` opens inline in an editor panel as on `y-snap-config`'s Palette tab. Colours are opaque; no alpha slider. A colour message equal to the one in effect is ignored (the picker reports its initial colour). **As built:** `colours.rs` (the edits, 8 tests) and `colours_view::swatch`: four sets, five swatches each, *inherited: #…* from y-coustic's own `Palette`, an inline `ColorPicker` in `editor` under the set being edited (no alpha), ↺ per colour, *Reset all* per set with an Undo. The picker's first report is not an edit. |
| 9.2 | The microphone rule | 👁 | A line on the microphone's set: *its bar is red while live unless you set an accent here; the general accent does not reach it*. **As built:** a caption on the Everything set (it does not reach the microphone's live bar) and on the Microphone set (red while live unless an accent is set here). |
| 9.3 | Preview | 👁 | The three kinds side by side through `render_hud`, in the theme chosen, muted and live. **As built:** master, microphone live, microphone muted and application through `HudUi::picture`, beside the sets (stacked: first); the theme switch is the HUD screen's, shared. |

## Phase 10 — Polish, docs and the live pass

| # | Item | Status | Notes |
| --- | --- | --- | --- |
| 10.1 | `ui-checklist.md` | 👁 | One table per screen, every row checked by resizing a real window through each threshold down to the minimum. **As built:** `ui-checklist.md` has a table per screen (Shell, General, Tray icon, Mixer, Keys, Mouse, HUD, Colours) and one for the HUD itself; every row is for the user to check in a real window. |
| 10.2 | Problem markers | 👁 | Per row, never blocking: a key y-coustic cannot register, a duplicate, a layer that hides a bare key. **As built:** `controls::expand_origins` gives every binding its origin and every warning its place (`Origin`, `Place`, `Problem`; `Settings::expansion`); the Keys screen puts a ⚠ with the messages as tooltip on the group's header (controls nothing, or the one target's problems), on a layer's row (Alt, a key used twice) and on a hotkey's row; the readout says where each binding comes from (*group 2, layer 3*, *hotkey 1*). The Mouse screen already marked a shadowed binding. |
| 10.3 | Keyboard paths | ✅ | `windows-reactor` has no key events: every action reachable by Tab and Enter or Space; check the tables, chips and pickers. **As built:** audited, not testable here: no `Border` with a pointer handler anywhere; every action is a `Button`, `ToggleButton`, `ComboBox`, `NumberBox`, `Slider`, `ToggleSwitch`, `TextBox` or `ColorPicker`, so Tab and Enter reach it. A real Tab order is a live check. |
| 10.4 | Docs | ✅ | A *Settings* section in y-coustic's README and user manual, the screens' screenshots added to `y-coustic/docs/screenshots.md` (the HUD style cards can be rendered by `render_hud`, which gives the README's *styles* and *colours* pictures without a capture session), a `docs/development.md` for this crate. **As built:** the README, the user manual (*The settings app*), `y-coustic-config/docs/development.md`; `y-coustic/examples/hud_pictures.rs` draws five of the pictures (the HUD, the five styles, colours per kind, both themes, the microphone) with the painter, and they are in the README and the manual. The rest of `screenshots.md` is still for hand. |
| 10.5 | Live pass | ⬜ | With the user: open the real file, change a key, a layer, a mouse binding, the HUD and a colour, Save, check y-coustic reloads and the comments are intact in a text editor. **As built:** the live pass with the user: open the real file, change a key, a layer, a mouse binding, the HUD and a colour, Save, check y-coustic reloads and the comments are intact in a text editor. |
| 10.6 | Later | ⬜ | Editing comments; a *Test the HUD* button (an ipc request that makes the running y-coustic show a HUD with the settings on screen, saved or not); import of a VulCan `.ini` (the design said no, so this stays a question). |

## Changes outside the new crate

| Phase | Crate | Change |
| --- | --- | --- |
| 1 | `y-coustic` (lib) | the model keeps comments and unknown content; `to_xml` on `xml::Writer`; safe save; `edit.rs`. |
| 2 | `y-coustic` | `ipc`; reload on the message; *Settings…* in the tray menu. |
| 7 | `y-coustic` | the HUD painters and `Palette` move from the binary into the library. |
| 3 | root `Cargo.toml` | the new workspace member. |

## Risks

* **The round trip is the hard part**, not the screens: attaching comments to
  list entries and writing them back without moving or doubling them. It is
  Phase 1 for that reason, tested against the shipped file before any UI exists.
* **Moving the HUD painter** (Phase 7) could change what the tray application
  draws by a pixel. The tests move with the code and the user's live pass of the
  five styles and the fade is the gate.
* **Key rules differ between a hotkey and a media key**: `KeyRules` has to allow
  `VolumeUp` and the numpad and refuse what `RegisterHotKey` and y-coustic's hook
  cannot take; the list is in `controls.rs` and `y_core::hotkey`, and the picker's
  rules are built from it, not written twice.
* **A `windows-reactor` limit** found while building a screen (no key events, no
  hover, a brush without gradients) is noted in the code and here, as the
  porting rules in `CLAUDE.md` ask.
