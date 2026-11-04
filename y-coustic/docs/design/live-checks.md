# y-coustic — live checks, and how to resume

Phases 1, 2, 2b and 3 are built, and `cargo clippy --workspace --all-targets` and
`cargo test --workspace` pass. Nothing below has been run on a real desktop: the
sessions that did the work do not drive the desktop themselves. This file is the
list of what to try, what you should see, and where to write down what you saw.

It is also where to pick the work up again: see [Resuming](#resuming).

Status per check: ⬜ not tried · ✅ as expected · ❌ not as expected (write what
happened under it).

## 0. Setup

### Build and run

```sh
cargo run -p y-coustic
```

Run it from a terminal in the repository root. It has no window: look for its icon
in the notification area (Windows 11 may put it in the overflow, drag it out).
Quit with the tray menu's **Exit**. Only one copy runs; starting a second shows the
menu of the first.

### A test configuration

Your existing `y-coustic.xml` was written before `<watch>` and `<controls>`
existed. The app fills in defaults for what is missing, but for these checks it is
simpler to have one file that has everything.

1. Tray icon → **Open configuration** (it shows you which file is in use; **About**
   also prints the path).
2. Replace the whole content with the block below. The complete example,
   [`../examples/y-coustic.full.xml`](../examples/y-coustic.full.xml), is a fuller
   file you can use instead: it has every option with comments, but its hotkeys and
   `<watch master>` differ from this block, so the check tables assume the block
   below.

```xml
<?xml version="1.0" encoding="utf-8"?>
<y-coustic>
	<volume step="5"/>

	<!-- master="hud" is for check 2b.3, put it back to "off" afterwards. -->
	<watch master="hud" microphone="hud"/>

	<controls>
		<keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="windows"/>
		<keys up="Win+NumAdd" down="Win+NumSub" mute="Win+Numpad0"/>

		<layer mod="none"  target="master"/>
		<layer mod="Shift" target="mic"/>
	</controls>

	<hotkeys>
		<hotkey key="Win+Alt+M"  action="mute" target="mic"/>
		<hotkey key="Win+Alt+F9" action="set-volume" value="30"/>
	</hotkeys>

	<overlay position="bottom-center" timeout="1200"/>
</y-coustic>
```

3. Save, then tray icon → **Reload configuration**.

**No balloon should appear.** A balloon after a reload lists warnings, one line
each: copy them into the report, they are the first thing to look at. A balloon
saying a key is *"Already taken by another application"* means that combination is
owned by Windows or another program: note which, and pick another in the file.

If you have no numpad, the numpad group is simply unused; use the hotkeys of the
`<hotkeys>` section instead.

### Before you start

- Note the **current volume** of the speakers and unmute them, so you can tell a
  change from nothing.
- Know which **microphone** is your default (Sound settings → Recording).
- Have the Windows **Sound control panel** at hand: `Win+R`, `mmsys.cpl`.

---

## 1. Phase 1: the master volume, the keys and the bar

| # | Do | Expect | Status |
| --- | --- | --- | --- |
| 1.1 | Press `Win+NumAdd` once | The volume rises by 5 points, a bar appears at the bottom centre of the monitor the pointer is on: a speaker with waves, a bar, the percentage | ⬜ |
| 1.2 | Hold `Win+NumAdd` | The volume keeps rising while held | ⬜ |
| 1.3 | Press `Win+NumSub` | It drops by 5 | ⬜ |
| 1.4 | Press `Win+Numpad0` once | Muted: the bar says *Muted*, the speaker is crossed, the bar is grey | ⬜ |
| 1.5 | Hold `Win+Numpad0` | It toggles **once**, it does not flap | ⬜ |
| 1.6 | With the speakers muted, press `Win+NumAdd` | It **unmutes** and raises the level | ⬜ |
| 1.7 | With the speakers muted, press `Win+NumSub` | It stays muted, the level drops | ⬜ |
| 1.8 | Press `Win+NumAdd` ten times quickly | **One** bar on screen, not ten, which goes away about 1.2 s after the last press | ⬜ |
| 1.9 | Change the output device (Windows volume menu → another output) then press `Win+NumAdd` | The new device changes, with no reload | ⬜ |
| 1.10 | Edit `<volume step="5"/>` to `step="15"`, **Reload configuration**, press `Win+NumAdd` | It rises by 15 | ⬜ |
| 1.11 | Left-click the tray icon | The bar shows the current level, nothing changes | ⬜ |
| 1.12 | Tray menu → **Open configuration** | Opens the file in use. (With no file, it is created from the defaults first.) | ⬜ |
| 1.13 | Tray menu → **Start with Windows**, then look at it again | It is ticked, and ticks off again if chosen once more. Leave it off | ⬜ |
| 1.14 | Tray menu → **About** | A balloon with the version, the memory (should be a few MB), the config path | ⬜ |
| 1.15 | **Look at the bar** | Proportions, colours (follow the Windows light/dark setting), glyph shapes are right | ⬜ |
| 1.16 | If you have a second monitor with another scale, press a key with the pointer on it | The bar is on that monitor and the right size | ⬜ |

Things worth reporting under 1.15 even if "fine": the bar being too small, too
close to the taskbar, text cut off, the speaker glyph looking odd. The glyphs and
bar are drawn by code, so they are cheap to adjust.

---

## 2. Phase 2: the microphone

| # | Do | Expect | Status |
| --- | --- | --- | --- |
| 2.1 | Press `Win+Alt+M` | The bar shows a **microphone** glyph (slashed), *Muted*, a grey bar. The **tray icon turns into a red stock icon**, its tooltip says *Y-Coustic: microphone muted* | ⬜ |
| 2.2 | Press `Win+Alt+M` again | Unmuted: microphone glyph, a **red** bar and the percentage. The tray icon goes back to normal | ⬜ |
| 2.3 | Open `mmsys.cpl` → **Recording** → your default microphone → *Properties* → *Levels*; look at the mute button while pressing `Win+Alt+M` | The Windows mute state follows each press | ⬜ |
| 2.4 | **Two defaults.** In the Recording tab, set a *different* device as *Default Communications Device* than the *Default Device* (right click → *Set as Default Communication Device*). Mute with `Win+Alt+M` | **Both** devices end up muted (check each one's Levels tab). Skip this check if you only have one microphone | ⬜ |
| 2.5 | Mute the microphone, then press `Shift+VolumeUp` (the `Shift` layer, see check 4) | The level changes and the microphone **stays muted** | ⬜ |
| 2.6 | Unmuted microphone: `Shift+VolumeDown` repeatedly down to 0 % | It reaches 0 % and does **not** mute by itself | ⬜ |
| 2.7 | **Disable every microphone** (Recording tab → right click → *Disable*), press `Win+Alt+M` | The bar says *No microphone*. Re-enable them afterwards | ⬜ |
| 2.8 | Mute the microphone, then **Reload configuration** and quit and restart y-coustic | After the restart the tray icon shows the muted state at once | ⬜ |

---

## 3. Phase 2b: watchers (changes made outside y-coustic)

The microphone is watched by default (`microphone="hud"`); the file above also
turns the master on (`master="hud"`).

| # | Do | Expect | Status |
| --- | --- | --- | --- |
| 3.1 | In `mmsys.cpl` → Recording → default microphone → Levels, **click its mute button** | The bar appears with *Muted* **and** the tray icon turns red | ⬜ |
| 3.2 | Do 3.1 again to unmute | The bar appears, the tray icon goes back to normal | ⬜ |
| 3.3 | Change the **speaker** volume in the Windows volume mixer or volume flyout | Our bar appears (needs `master="hud"`) | ⬜ |
| 3.4 | Press `Win+Alt+M` (our own key) | The bar shows **once**, not twice | ⬜ |
| 3.5 | Press `Win+NumAdd` | Once, not twice | ⬜ |
| 3.6 | Change the default **output** device, then change the volume in the Windows mixer | The bar still appears: the watch followed the new device | ⬜ |
| 3.7 | Change the default **microphone** to another one, then mute it from `mmsys.cpl` | The tray icon and the bar follow the new default | ⬜ |
| 3.8 | Edit `microphone="hud"` to `microphone="off"`, reload, mute from `mmsys.cpl` | **Nothing** happens in y-coustic | ⬜ |
| 3.9 | Edit it to `microphone="tray"`, reload, mute from `mmsys.cpl` | The **tray icon** changes, **no bar** | ⬜ |

### Questions the design is waiting on

These are open in the [design](y-coustic-plan.md); answer them in your report and
they get recorded there.

| # | Question | Your answer |
| --- | --- | --- |
| Q1 | Does your **laptop's microphone mute key** (if it has one) show the bar and flip the tray icon? | |
| Q2 | Does the **Windows volume center** (the quick-settings slider) show our bar, with `master="hud"`? | |
| Q3 | Do a **Bluetooth headset's buttons** show our bar? | |
| Q4 | **Unplugging the default output** while y-coustic runs: does the bar still follow the new default afterwards? | |

---

## 4. Phase 3: volume keys and layers

Put `<watch master="off" microphone="hud"/>` back first, so the master watcher
does not add a bar to what Windows already shows.

| # | Do | Expect | Status |
| --- | --- | --- | --- |
| 4.1 | With `plain="windows"`, press your keyboard's **volume up** (bare) | **Windows' own flyout**, no bar of ours | ⬜ |
| 4.2 | Hold `Shift` and press **volume up** | Our bar, a microphone glyph, the microphone level rises, and **no Windows flyout** | ⬜ |
| 4.3 | `Shift` + **volume down**, `Shift` + **volume mute** | The microphone level drops; the microphone mutes and unmutes | ⬜ |
| 4.4 | Tray menu → **Replace the Windows volume indicator** | Ticked. (It saved the file and reloaded: open the file, `plain` is now `replace`, **your comments are gone**, that is expected) | ⬜ |
| 4.5 | Now press the bare **volume up** | **Our bar**, a speaker glyph, **no Windows flyout** | ⬜ |
| 4.6 | Hold the bare volume up | The volume keeps rising while held | ⬜ |
| 4.7 | Hold the bare volume mute | It toggles once, no flapping | ⬜ |
| 4.8 | Tray menu → **Replace the Windows volume indicator** again | Unticked, bare keys give Windows' flyout again | ⬜ |
| 4.9 | Put it back to `replace`, then **Exit** y-coustic from the tray and press volume up | **Windows' own volume and flyout are back**, with y-coustic not running | ⬜ |
| 4.10 | Put `<layer mod="Ctrl" target="focused"/>` inside `<controls>`, reload | A balloon says that layer is *not supported yet* (once). Remove the line afterwards | ⬜ |
| 4.11 | Put `<layer mod="Alt" target="master"/>` inside `<controls>`, reload | A balloon warns about `Alt` and the menu bar. Try `Alt` + volume up, then let go of `Alt`: does a menu bar get the focus? Remove the line afterwards | ⬜ |
| 4.12 | Add a second `<hotkey key="Shift+VolumeUp" action="mute"/>` to `<hotkeys>`, reload | A balloon says `Shift+VolumeUp` is used twice. Remove the line afterwards | ⬜ |
| 4.13 | With an **elevated** window in front (an admin terminal), press `Shift+VolumeUp` | Nothing from y-coustic: the hook does not see keys going to an elevated window. Known limit | ⬜ |

---

## 5. Regression: `y-snap` (the hook lookup changed)

Phase 3 changed `y_core::hotkey`, which `y-snap` uses for the hotkeys it steals from
the shell. Please check this one **before** the next phase, which changes `y-snap`
further.

| # | Do | Expect | Status |
| --- | --- | --- | --- |
| 5.1 | Quit y-coustic. Run `cargo run -p y-snap` with its usual configuration | No balloon about taken keys that was not there before | ⬜ |
| 5.2 | Use **each hotkey that has `steal-from-shell="true"`** in your `y-snap.xml` (the `Win+…` ones) | Each does what it did before, and the **Start menu does not open** when you let go of `Win` | ⬜ |
| 5.3 | Hold one of them if it repeats | Same behaviour as before | ⬜ |
| 5.4 | If you have two stolen hotkeys on the **same key with different modifiers** (for example `Win+X` and `Win+Shift+X`) | Each fires on its own combination. (This is the case the fix is about; before it, one could never fire) | ⬜ |

---

## What to send back

Copy the tables and the questions, with a status on each line. For anything ❌ or
odd, three things help most:

1. What you did exactly (keys, menu items, the relevant lines of your file).
2. What you saw instead, including any balloon text word for word.
3. Whether it is new, or the same in the previous step.

The first thing to look at, if several things fail: **a balloon after Reload**.

## Resuming

### Where things stand

| Phase | State |
| --- | --- |
| 0 Probe | ✅ (a call application's capture session, for the record only) |
| 1 Master volume, hotkeys, HUD | 👁 built, checks in section 1 |
| 2 Microphone | 👁 built, checks in section 2 |
| 2b Watchers | 👁 built, checks in section 3 |
| 3 Media keys and layers | 👁 built, checks in sections 4 and 5 |
| 4 Mouse | ⬜ not started |
| 5 Application volume | ⬜ |
| 6 Panel | ⬜ |
| 7 Interactive HUD and tray hover | ⬜ |
| 8 Overlay customization | ⬜ |
| 9 `y-coustic-config` | ⬜ (its own plan) |

The plan with every task is [`implementation-plan.md`](implementation-plan.md); the
reasons are in [`y-coustic-plan.md`](y-coustic-plan.md). Both have a *Resume here*
line, kept up to date.

### To resume

1. Send the results of this file (or the failures), and the answers to Q1 to Q4.
2. Fixes to anything ❌ come first. The answers to Q1–Q4 go into the design doc's
   Phase 2b notes.
3. **Next task: Phase 4, task 4.1**: move `modifier_guard` from `y-snap` to
   `y_core::modifier_guard`, `y-snap` switching to it. It touches `y-snap`, which is
   why section 5 comes first, and the live check for it is that `Alt+drag` in
   `y-snap` still leaves no menu-bar focus.
4. Then 4.2 to 4.6: the mouse hook (`Alt+wheel`, buttons), delta coalescing, the
   `<mouse>` section.

### Decisions waiting for you

| Question | Today |
| --- | --- |
| Master `tray` in `<watch>` | Behaves like `off` (the master has no tray icon variant). The alternative is to reject it with a warning |
| Default layers | `none` → master and `Shift` → microphone. `Ctrl` → focused application and `Ctrl+Shift` → pointed application are kept for Phase 5, as the design table says |
| Tray icon for a muted microphone | The stock red error icon until `rsc/y-coustic-mic-muted.ico` exists (resource 102, see `rsc/y-coustic.rc`) |
| The application icon | None yet: the generic system icon, until `rsc/y-coustic.ico` exists |
| Main README | Does not list y-coustic yet; add it once these checks pass |

### Known limits, so they are not mistaken for bugs

- A mute made outside y-coustic on the **communications** microphone is not
  noticed (only the default one is watched).
- `<mouse>`, `<panel>` and `<tray>` are recognised and answered *not supported yet*.
  Layers and hotkeys on `focused` or `pointed` likewise.
- Saving from the tray regenerates the file: comments are lost and sections of
  later phases are not written back.
- Nothing is committed: the working tree holds everything, and `git status` shows
  it. The changes outside `y-coustic/` are `core/src/hotkey.rs` (the lookup fix and
  `Fallback::HookOnly`) and the root `Cargo.toml` (`windows-core`).

### Commands

```sh
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets     # keep clean
cargo test --workspace
cargo test -p y-coustic -- --ignored       # the tests that need an audio device
cargo run -p y-coustic
```

If a build looks stale ("Finished" in a fraction of a second after an edit),
`touch` the changed `.rs` files and run it again.
