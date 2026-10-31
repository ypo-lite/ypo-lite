# Mouse shortcuts — implementation plan

A modifier plus a mouse trigger fires one action, the way a hotkey does:
`Win`+middle click opens the overlay, tilting the wheel right pins the window.

Built; status is in [Progress](#progress). Phase 0 below is the record of the
two probes that settled the open questions (their code has been removed).

## What it is, and what it is not

| | Drag / resize (`<drag>`, `<resize>`) | Mouse shortcut (`<shortcut>`) |
| --- | --- | --- |
| Shape | a session: press, move, release | one event: fire and forget |
| How many | exactly two, fixed | a list, any number |
| Bound to | a built-in behaviour | any [`Action`](../src/action.rs), like a hotkey |
| Triggers | left, right, middle | those, plus wheel tilt and side buttons |
| Has a release | yes, it ends the gesture | buttons yes, **wheel tilt no** |

So a shortcut gets its own module and its own screen, and shares only the hook
and the action vocabulary. The closest existing code is not `gesture.rs` but
`hover.rs`'s right-click: claim the press, swallow the release, post the work.

**Out of scope:** wheel up/down (`Win`+scroll), double clicks, press-and-hold,
chords of two mouse buttons, per-application shortcuts. Wheel up/down belongs
with the switcher, which needs notch accumulation and a session
([`switcher-plan.md`](switcher-plan.md) §5). The hook work below is the part of
that section both features share; the schema leaves room for `wheel-up` and
`wheel-down` later.

## Decisions

Taken for this plan. The first three are worth a second look.

1. **"Left/right buttons of the wheel" means tilt, `WM_MOUSEHWHEEL`.** Side
   buttons (`XBUTTON1/2`) come along because they cost almost nothing and the
   switcher wants them too.
2. **A held tilt fires once.** Windows sends tilt as repeating events with no
   release, so holding it would otherwise fire `close` several times. Stepping
   through desktops takes one tilt per step; an opt-in `repeat="true"` can be
   added later.
3. **A separate "Mouse shortcuts" entry in the navigation pane**, not a second
   half of the Mouse gestures page: that page is cards, this one is a table
   like Hotkeys.
4. **It fires on the press**, not the release, so the action lands on the window
   that was clicked and feels immediate. The release is always swallowed.
5. **`target` defaults to `pointed`, with no fallback.** A hotkey defaults to
   `focused`, but here the pointer is the point. If nothing eligible is under
   the pointer, the click passes through untouched, as a drag does: `Win`+middle
   on the desktop does nothing surprising, and never acts on the focused window
   instead. `target="focused"` is available.
6. **The window is chosen in the hook, at the press**, and carried in the posted
   message. Resolving it later on the message loop could pick a different window
   if the pointer has moved on.
7. **Modifiers match exactly**, as for gestures: a `Win` shortcut does not fire
   on `Win+Shift`. Left and right `Win` count as the same key.
8. **Plain left and right clicks cannot be taken.** `mod="none"` on those is a
   warning and the shortcut is dropped: it would eat every click on the machine.
   Plain middle, tilt and side buttons are allowed, since those are reasonable
   to want, but the screen says what they replace.
9. **A combination can only mean one thing.** A shortcut duplicating another
   shortcut, or a `<drag>`/`<resize>`, is a warning and is dropped. Gestures
   always win; they are fixed slots and the file reads them first.
10. **Injected events count.** Mouse software such as Logitech's often sends its
    remapped tilt through `SendInput`, so filtering on `LLMHF_INJECTED` would
    silently break exactly the mice this is for.

## Configuration schema

Inside `<mouse>`, next to the gestures:

```xml
<mouse>
	<drag     button="left"  mod="Alt"/>
	<resize   button="right" mod="Alt"/>
	<shortcut button="middle"      mod="Win"  action="overlay"/>
	<shortcut button="wheel-right" mod="none" action="pin-toggle"/>
	<shortcut button="x1"          mod="Alt"  action="desktop-move" desktop="2"/>
</mouse>
```

| Attribute | |
| --- | --- |
| `button` | required: `left` `middle` `right` `wheel-left` `wheel-right` `x1` `x2` |
| `mod` | required, unlike `<drag>`: `Win+Alt`…, or `none`. A missing value must not silently mean something, when `none` can take over a whole button |
| `action` | required, the hotkey vocabulary; not `desktop-list` |
| `target` | `pointed` (default) or `focused` |
| `desktop` | for `desktop-move`, 1-based |

Severity follows the usual split: every problem is a warning, the rest of the
file still loads. `rsc/y-snap.xml` gets the example above as a comment, so a
fresh install changes no mouse behaviour.

## Phase 0 — two probes on the real mouse

Two unknowns cannot be settled by reading code. Both need a temporary debug
build run by you, since it reads your own hardware. Throwaway code: nothing
from this phase stays.

```sh
cargo run -p y-snap --features probe
```

Historical: the probe code (`src/mouse/probe.rs`, the `probe` feature) is gone. Quit any other y-snap first: it is single-instance. The log is
`y-snap-probe.log`, next to the executable (`target/debug/`). The code is
`src/mouse/probe.rs`; to remove it, delete that file and everything marked
`feature = "probe"`, including the entry in `Cargo.toml` and `popup_hwnd` in
`overlay.rs`.

**Probe A: what does the tilt actually send?** Log every event the hook sees
besides `WM_MOUSEMOVE`: message, `mouseData`, `flags`, `time`. Tilt each way,
tap and hold, then the side buttons, with any vendor mouse software running as
you normally use it.

* Expect `WM_MOUSEHWHEEL` with a signed delta in the high word of `mouseData`,
  positive for right.
* The driver may instead send side buttons, keystrokes, or nothing at all. If
  it sends keystrokes the hook will not see them, and the fix is a setting in
  the vendor software, not code here.
* Note the gap between repeats while held, in milliseconds. It sets
  `TILT_GAP_MS` in Phase 2.

**Probe B: can the overlay take focus after a swallowed click?** This is the
biggest risk in the plan. `Overlay::activate` calls `SetForegroundWindow` and
ignores the result. A hotkey works because `WM_HOTKEY` gives y-snap the right to
take the foreground. A click the hook swallowed gives y-snap no such input, so
Windows may refuse.

If it refuses, the overlay appears without focus: its keys do nothing, and
"click outside closes it" is `WM_KILLFOCUS`, which never comes for a window that
never had focus. **The overlay would stay on screen.**

The probe fires `action_overlay.show` from a hardcoded `Win`+middle and logs
`FOCUSED` or `REFUSED`, right after and again two seconds later. A refused
overlay closes itself at that point, so a failed test cannot strand it. An
overlay hotkey logs the same lines, as the control. Try at least: a normal app
in front, the desktop in front (click it, then `Win`+middle over a window behind
it), and with y-snap idle for over a minute. Then decide:

| Result | Then |
| --- | --- |
| Always takes focus | Phase 4 is only the safety net |
| Sometimes or never | Phase 4 adds a way to obtain the foreground |

### Results so far

From the first probe run, on your mouse, 2026-10-31.

**Probe A: settled.**

* Tilt is `WM_MOUSEHWHEEL`, delta ±120, `injected=false`: no vendor remapping.
* Side buttons are `WM_XBUTTONDOWN/UP` with button 1 and 2.
* A held tilt repeats after about 400 ms (391–406), then every 93–110 ms. So
  **`TILT_GAP_MS` must be above 406**, and the 300 first assumed here would have
  fired twice on every long hold. Use 500. Two deliberate tilts less than half a
  second apart then count as one.

**Probe B: settled enough to build on.** A swallowed click does not reliably get
the foreground, so Phase 4 must obtain it. Over three runs, the first attempt after
y-snap starts was `REFUSED` all 4 times with nothing injected beforehand, and the
next attempt, after `Win` had been released and `modifier_guard` had injected its
key, was focused. The one first attempt that injected a key beforehand was focused.
One other refusal had Windows search in front.

So injected input appears to be what grants the foreground, and the `SendInput`
candidate in Phase 4 is the one to build. The positive case is a single sample,
so the manual checks must repeat it on the real path.

**Fixed: `Win`+middle reached the browser.** All 9 attempts where `target::at`
found nothing were passed on, which is what started Opera's autoscroll. The second
run showed why, and it was not about Opera. Inside the hook, `WindowFromPoint(info.pt)`
returned no window, or twice a secondary taskbar, while the same lookup from
`GetCursorPos` a moment later found Opera. So the point was wrong, not the window.

`y-snap.exe` had no manifest, so it ran DPI-unaware, although
`core/src/monitor.rs` assumes per-monitor v2. In that state `GetCursorPos` and
`WindowFromPoint` are scaled while the hook's `pt` is not, so any window on a
scaled monitor can be missed; Opera is simply what was there. `rsc/y-snap.exe.manifest`
now declares per-monitor v2, and with it no attempt was passed on: Opera was
targeted every time, on the higher-DPI screen, and drag still works.

**Still unexplained:** drag and resize already worked on that screen before the
manifest, although `Gesture::on_button_down` calls the same `target::at(info.pt)`
as the probe. The fix is confirmed, but the account of why only the click failed
is incomplete.

**To watch:** overlay sizes are not scaled in code, and Windows no longer stretches
them, so they may look smaller on the scaled screen. Separately, no other app's
`.rc` names a manifest, so `y-pop` and the rest run without DPI awareness too,
although `y-core` assumes it. No mouse hook exists outside y-snap, so this exact
fault cannot occur there.

**New: a passed-on click should still arm `modifier_guard`.** In one attempt
Windows search had taken the foreground, right after four passed-on clicks. The
likely cause is the lone `Win` release they left behind, as nothing was armed for
it. Phase 2 step 3 should arm before passing on, not after finding a window; the
probe now does, and no run since has shown it again. Decision 5 stands: the
browser was a DPI fault, not a reason to swallow clicks with no target.

## Phase 1 — model and parsing

`y-snap/src/config.rs`, no Win32 behaviour. Fully unit-tested.

* New `MouseTrigger` enum: `Left Middle Right WheelLeft WheelRight X1 X2`, with
  `from_name`/`name`. Separate from `Button`, because `<drag button="wheel-left">`
  has no meaning and must keep warning. Existing test `reads_mouse_gestures`
  stays as it is.
* New `MouseShortcut { trigger, modifiers, action, target, desktop }`, `Copy`.
* `Settings` gets a new field `mouse_shortcuts: Vec<MouseShortcut>`, rather than
  a `Vec` inside `Mouse`: `Mouse` is `Copy` and `Gesture::new` takes it by value,
  and that should not have to change.
* `Reader::mouse` accepts `shortcut` as a known child and leaves it alone; a new
  `Reader::mouse_shortcuts` reads them after, with `Reader::action`,
  `Reader::target` and `Reader::desktop` reused and `desktop-list` refused as in
  `<hotkey>`.
* `pub fn shortcut_problem(candidate, earlier, mouse) -> Option<ShortcutProblem>`
  holds decisions 8 and 9 in one place, for the reader and the settings app
  alike, the same arrangement as `is_minbutton_action`.
* `write_mouse` writes `<shortcut>`, omitting `target` when `pointed` and
  `desktop` when 0, always writing `mod`.

Tests: read and round-trip through `to_xml`; each refusal warns and keeps the
others; bad attribute, missing `mod`, `<drag button="x1">` still warns.

## Phase 2 — the hook

New `y-snap/src/mouse/shortcut.rs`, a third concern beside `gesture` and `hover`.
No new hook. Installed state stays a `thread_local!`, per the project's rule.

The hook gains `WM_XBUTTONDOWN`, `WM_XBUTTONUP` (which button is in the high word
of `mouseData`) and `WM_MOUSEHWHEEL`. `WM_MOUSEWHEEL` stays untouched, so ordinary
scrolling costs nothing new.

The decisions are a pure state machine: modifiers and time come in as arguments,
so it is testable without Windows, which `gesture.rs` is not.

```rust
pub struct Shortcuts {
    bindings: Vec<MouseShortcut>,  // copied at install, only ever read in the hook
    held: [bool; BUTTONS],         // press claimed, release still to swallow
    tilt: [Option<u32>; 2],        // `time` of the last tilt event seen
}
pub enum Press { Pass, Swallow, Fire(usize) }

fn press(&mut self, trigger, modifiers, gesture_active: bool) -> Press;
fn release(&mut self, trigger) -> bool;                 // always true once claimed
fn tilt(&mut self, trigger, modifiers, time: u32, gesture_active: bool) -> Press;
```

**Buttons.** On press, first clear `held` for that button: a release lost to a
dropped hook must not swallow the next real one. Bail out quickly if no binding
uses the trigger, so ordinary clicks pay for one slice scan and not for
`GetAsyncKeyState`.

**Tilt.** There is no release, so "held" is inferred from time. An event within
`TILT_GAP_MS` of the previous one is the same hold: `Swallow` it and refresh the
timestamp, so that the gap restarts and a long hold never fires twice. Compare
with `wrapping_sub`, as `time` wraps. Any non-zero delta counts: high-resolution
wheels send less than 120, and no accumulation is needed when one event is one
press. Use 500 ms, from Probe A's results.

**During a drag or resize**, a press matching a shortcut is `Swallow`, not `Fire`,
matching the "not now" in `Gesture::on_button_down`. Needs a small
`Gesture::active()`.

**Wiring in `mod.rs`.** A new `HookState::on_press` does what cannot be pure:

1. reads the modifiers (`modifiers_held` moves from `gesture.rs` to `mod.rs`);
2. on `Fire`, resolves the window: `target::at(pt)` for `pointed`,
   `target::focused()` for `focused`, skipped when `!action.needs_window()`;
3. no window for an action that needs one: `Pass`, and clear `held`;
4. otherwise calls `modifier_guard::arm(modifiers)`, without which `Win` or
   `Alt` would open Start or the system menu on release, then posts
   `WM_MOUSE_SHORTCUT` with `wParam` the index and `lParam` the `HWND`.

`WM_MOUSE_SHORTCUT = WM_APP_FIRST + 8`; +0 to +7 are taken. `modifier_guard`'s doc
comment gets updated, as it only speaks of gestures today.

**Order in the hook:** shortcut, then hover, then gesture, for both press and
release. A shortcut on the right button always has a modifier (decision 8), so
it can only take a modified click from the minimize-button action, and exact
duplicates of gestures are gone at parse time (decision 9). `shortcut.release`
runs first so that a claimed release cannot reach `gesture.on_button_up`.

**To check while here:** whether `target::at` refuses y-snap's own overlay. A
shortcut clicked over the open overlay must not target the overlay itself; if it
does not refuse, handle it here.

Tests: fire and swallow the release; release after modifiers are let go;
modifier mismatch passes; press clears a stale `held`; tilt hold fires once;
tilt fires again after the gap; two directions are independent; `time` wrap;
swallow during a gesture.

## Phase 3 — dispatch

`y-snap/src/main.rs`.

* Split `on_hotkey`'s body: resolving the binding stays, and the match becomes
  `fn fire(&mut self, hwnd, action, window: Option<HWND>, desktop)`, so `Quit`,
  `Reload` and `Overlay` behave identically from both entry points. For hotkeys
  `target::resolve` moves up into `on_hotkey`, unchanged in behaviour.
* New arm for `mouse::WM_MOUSE_SHORTCUT`. Look the index up with `get`, as a
  reload may have shortened the list between the post and the handling. Drop the
  message if the `HWND` is no longer a window (`IsWindow`): `close` can race the
  next click.
* `MouseHook::install` takes `&settings.mouse_shortcuts`.

## Phase 4 — the overlay from a click

Whatever Probe B says, add the safety net: in `ActionOverlay::show`, after
`activate`, if `GetForegroundWindow()` is not the overlay, close it again rather
than leave it stranded. A shortcut that does nothing is a much better failure
than a popup that cannot be dismissed. This also protects any hotkey path that
loses the same race.

Probe B showed refusals, so `ActionOverlay::show` must also obtain the foreground:
`absorb_modifier_tap` just before `Overlay::activate`, which is what the probe
tested, on the message loop. Keep this in y-snap's shortcut path rather than in
`activate`, which `y-pop`'s search also calls and which does not need it. Only if
the manual checks still show refusals, try `AttachThreadInput` with the foreground
window's thread.

**Known limit to document, not fix:** `Win` is still down when the overlay opens.
Pressing a tile's letter before letting go of `Win` is `Win+letter`, which belongs
to the shell. Clicking a tile works either way. The filler key `absorb_modifier_tap`
sends is Right Shift, which the overlay ignores unless a tile is bound to Shift.

## Phase 5 — `y-snap-config`

New `y-snap-config/src/mouse_shortcuts.rs`, and `Section::MouseShortcuts` placed
after `Mouse` in the pane. Read `ui-kit/docs/ui-toolkit.md` first. No new ui-kit
control should be needed; the screen is the Hotkeys table with a different picker.

```
Mouse shortcuts
A modifier and a mouse button, each bound to one action.

 Trigger                Action            Target     Desktop
 [Win] [Middle]         overlay      v    pointed v     —      🗑
 [Wheel right]          pin-toggle   v    pointed v     —      🗑
 ┌ Trigger for pin-toggle ─────────────────────────────────┐
 │ Modifiers   [Win] [Ctrl] [Alt] [Shift]                  │
 │ Button      [Left] [Middle] [Right]                     │
 │ Wheel tilt  [Left] [Right]                              │
 │ Side        [Back] [Forward]                            │
 │ ⚠ Win+Left is already used by Drag.                     │
 │                                         [Cancel] [Done] │
 └─────────────────────────────────────────────────────────┘
 [Alt] [Side back]      desktop-move v    pointed v   [ 2 ]    🗑

 + Add shortcut
 ⓘ Tilt and side buttons depend on your mouse: its own software
   may need them left at their default.
```

* **Table:** `Table`, `Row::detail`, `combo`, `number_box`, `add_button` and
  `keycaps`, as in `hotkeys.rs`. Trigger cell is a button opening the editor;
  `keycaps(&combo_labels(modifiers, None) + trigger label)`.
* **Editor:** `editor(...)` with modifier `chip`s, as on the gesture cards, and
  one single-choice group of `chip`s per kind of trigger. Labels: `x1`/`x2` show
  as Back/Forward, what most mice print on them.
* **Conflicts:** `shortcut_problem` from Phase 1, shown live in the editor. Done
  must be disabled while there is a problem, so `editor` needs the `ok_enabled`
  parameter `modal` already has. Cancel restores the original.
* **Plain triggers:** picking none of the modifiers on Middle, tilt or side shows a
  one-line note of what is taken over, such as "Replaces middle click everywhere."
* **State in `main.rs`:** `Editing::MouseShortcut { index, original }`;
  `Msg::EditMouseShortcut`, `MouseShortcutAdded`, `MouseShortcutDeleted`,
  `…TriggerChanged`, `…ModifierToggled`, `…ActionChanged`, `…TargetChanged`,
  `…DesktopChanged`.
* **Adding:** starts with the first free combination, trying `Win`+Middle first,
  and opens straight into the editor, as Hotkeys does.
* **Actions:** the existing `hotkeys::ACTIONS`. No reorder: order means nothing
  here, as duplicates are refused.
* `y-snap-config/docs/ui-checklist.md` gets a table for this screen; `ui-mockup.md`
  gets a line. Must work at `shell::MIN_WINDOW`, with the table's card layout.

## Phase 6 — documentation

* `y-snap/README.md`: a `<shortcut>` section under `<mouse>`, the state line, the
  Layout table.
* `rsc/y-snap.xml`: the commented example.
* `switcher-plan.md` §5: note that tilt and `XBUTTON` are done.

## Risks

| Risk | Effect | Handling |
| --- | --- | --- |
| Foreground refused after a swallowed click | Overlay stuck without focus | Probe B, Phase 4 |
| Tilt remapped by the driver | Tilt shortcuts never fire | Probe A; the note on the screen; decision 10 |
| Repeat rate differs by mouse | One hold fires twice, or a quick second tilt is lost | `TILT_GAP_MS` measured, not guessed; a config attribute only if one value proves not to fit |
| Elevated window in front | No event reaches a non-elevated hook, so nothing fires | Same limit as drag and resize; README mentions it |
| `Reload` fired from a press | The hook reinstalls before the release, so one stray button-up reaches the window | Accepted: harmless, and rare |
| Hook dropped mid-press | Release lost | Next press clears `held`; the watchdog reinstalls the hook |
| Hot path | Hook timeout | No allocation, no window work: a slice scan, then `PostMessageW` |

## Manual checks

Input timing cannot be unit-tested. Once phases 2–4 are in:

- [ ] `Win`+middle on an app: overlay opens on that window, no Start menu when `Win` is let go.
- [ ] Same, letting go of `Win` before the mouse button, and after.
- [ ] First shortcut right after starting y-snap, three restarts in a row: the overlay takes focus each time.
- [ ] Overlay from a shortcut: a tile's key works, click outside closes, `Escape` closes.
- [ ] Shortcuts, drag and the overlay on each screen, including the scaled one.
- [ ] `Alt`+middle: no system menu on release.
- [ ] Shortcut on the desktop and on the taskbar: click passes through, nothing fires.
- [ ] Wrong modifiers: the middle click reaches the application normally.
- [ ] Plain middle click with only a `Win` shortcut configured: closes a browser tab as usual.
- [ ] Tilt, tap: one action. Tilt, hold three seconds: still one. Tilt again after: fires.
- [ ] Tilt with no shortcut on it: horizontal scrolling still works.
- [ ] Side button shortcut: the browser does not also go back.
- [ ] `close` from a shortcut, then a quick click on the window behind: the click arrives.
- [ ] During an `Alt` drag, a matching shortcut neither fires nor reaches the application.
- [ ] `Alt`+drag and a shortcut sharing the left button with different modifiers: both work.
- [ ] Reload twice while shortcuts are configured: still work, nothing doubled.
- [ ] Lock the machine, come back: shortcuts still work.
- [ ] Elevated window in front: nothing happens, and nothing sticks.

## Progress

| Phase | Status | Notes |
| --- | --- | --- |
| 0. Probes | ✅ | Tilt confirmed, `TILT_GAP_MS` is 500, Phase 4 needs the foreground grant. Probe code removed |
| 1. Model and parsing | ✅ | `MouseTrigger`, `MouseShortcut`, `ShortcutProblem`, `shortcut_problem`, `Settings::mouse_shortcuts`; 3 new tests in `config.rs`. `MouseTrigger::index()` and `ALL` are there for Phase 2's tables |
| 2. Hook | ✅ | `mouse/shortcut.rs` (13 unit tests), `HookState::on_press`/`on_tilt`/`fire`, `WM_MOUSE_SHORTCUT`, `Gesture::active`, `modifiers_held` moved to `mouse/mod.rs`. Own overlay classes added to `target::BANISHED`. Checked live for the overlay path; the rest of the manual checks below are still open |
| 3. Dispatch | ✅ | `on_hotkey` split into resolve + `fire`; `on_mouse_shortcut` checks the index and `IsWindow`. Working feature, except `overlay` is not reliable until Phase 4 |
| 4. Overlay focus | ✅ | `ActionOverlay::show(window, grant_foreground)` sends `absorb_modifier_tap` before `activate` for a mouse shortcut, and closes the popup if it still is not the foreground. Checked live, three restarts in a row; probe code removed |
| 5. `y-snap-config` | ✅ | `mouse_shortcuts.rs`, `Section::MouseShortcuts`, `ui::editor_checked` (Done disabled on a problem), `Editing::MouseShortcut`. Layout is by reading only: the 👁 rows of `ui-checklist.md` need a live look |
| 6. Documentation | ✅ | README `<shortcut>` section, commented example in `rsc/y-snap.xml`, `switcher-plan.md` §5 note |
