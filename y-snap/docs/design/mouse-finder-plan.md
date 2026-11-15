# Mouse finder — implementation plan

Help the user find a lost pointer, mostly on multi-monitor setups: a short
animated indicator drawn around the cursor, fired by an action (hotkey, mouse
shortcut, tray menu), by tapping `Ctrl` twice like Windows does, or by shaking
the mouse.

Not built. Status is in [Progress](#progress).

## What it is, and what it is not

Windows already has a "show location of pointer when I press CTRL" effect
(`SPI_GETMOUSESONAR`). The finder is meant to be *more useful* than that, not a
clone: it also says **which monitor** the pointer is on, can be summoned
without a keyboard, and stays out of the way of games.

**In scope:** a transient overlay (beacon, monitor flash and map, crosshair,
spotlight), three triggers (action, double Ctrl, shake), a "do not disturb"
guard, configuration, `y-snap-config` screen, documentation.

**Out of scope:** changing the real system cursor (see decision 1), a
persistent cursor-trail effect, click ripples and keystroke visualisation
(PowerToys' Mouse Highlighter territory — a *toggle* halo is a Phase 7
candidate, nothing more), per-application configuration beyond an exclude list.

## Decisions

Taken for this plan. The first, the fourth and the sixth are worth a second
look.

1. **The real cursor is never resized.** Enlarging it means `SetSystemCursor`
   on every cursor role and `SPI_SETCURSORS` to restore: global state that a
   crash leaves behind, and apps that draw their own cursor ignore it. Instead
   the overlay draws the current cursor (`GetCursorInfo` → `hCursor`) scaled up
   and follows the pointer.
2. **Overlays are created lazily and destroyed after the animation.** No
   window, bitmap or timer exists while idle: the idle cost of the finder is
   the trigger detectors and nothing else. Matches "low memory".
3. **Detection stays in the hooks, drawing does not.** The shake detector and
   the Ctrl-tap detector are allocation-free and lock-free (`thread_local!`,
   fixed arrays), and only `PostMessageW` a private `WM_APP` message on a
   hit — the rule from `CLAUDE.md` for `WH_*_LL` hooks.
4. **The default is: action + double Ctrl + shake all enabled?** *No*: only the
   **action** is on by default. Double Ctrl collides with Windows' own feature
   when that is on, and shake can false-fire during fast window drags. Both are
   opt-in `<finder>` attributes; the config screen offers them and warns about
   the Windows overlap. (Revisit after live use: shake may deserve to be
   default-on once the false-positive rate is known.)
5. **Shake requires nothing held by default**, but `shake-needs="ctrl"` is
   available. A shake while a mouse button is down (a drag) never fires.
6. **Injected events are ignored by the finder's detectors** (`LLMHF_INJECTED`
   / `LLKHF_INJECTED`), the opposite of the mouse-shortcuts plan's decision 10:
   there, remapped buttons from mouse software had to count; here, remote-
   control tools and macros synthesising movement must not trigger a beacon.
7. **A trigger during an animation restarts it** rather than stacking a second
   overlay.
8. **The finder never takes focus or input.** The overlay is click-through and
   non-activating (`Overlay::set_click_through`); the keyboard hook swallows
   nothing for the Ctrl taps.
9. **It lives in y-snap**, reusing the two hooks y-snap installs anyway. The
   cost is that someone who wants only this still runs all of y-snap; a
   standalone app is not worth a third hook installer.
10. **Existing `center-cursor` is a neighbour, not a duplicate.** It warps the
    pointer to the middle of the *target window*. The finder only *shows*; an
    optional `recall="center"` makes a trigger also warp (Phase 6).

## "Do not disturb" guard

One function, `finder::allowed()`, evaluated on the message loop (never in a
hook) when a trigger lands. Any of these suppresses the finder:

| Signal | API | Catches |
| --- | --- | --- |
| Fullscreen / busy | `SHQueryUserNotificationState` → `QUNS_RUNNING_D3D_FULL_SCREEN`, `QUNS_BUSY`, `QUNS_PRESENTATION_MODE` | most games, presentations |
| Cursor hidden | `GetCursorInfo` without `CURSOR_SHOWING` | apps that swallow the pointer (raw input games, some 3D tools) |
| Cursor confined | `GetClipCursor` ≠ virtual screen | games, remote desktops, VMs |
| Foreground covers its monitor and is not the shell | `core::target` filtering | borderless-fullscreen games the first signal misses |
| Excluded process | `core::process` + `<finder><exclude>` list | anything the user names |

An explicit **action** trigger can bypass the guard (`force="true"` on the
binding) — if the user pressed a hotkey for it, they want it.

## Configuration schema

A new child of the root element, next to `<mouse>`:

```xml
<finder style="beacon" duration="1000" color="#FFB000"
        double-ctrl="false" shake="false" shake-needs="none"
        recall="none">
    <exclude process="game.exe" />
</finder>
```

| Attribute | Values | Default |
| --- | --- | --- |
| `style` | `beacon`, `monitor`, `crosshair`, `spotlight` | `beacon` |
| `duration` | 300–5000 ms | 1000 |
| `color` | `#RRGGBB[AA]` | accent |
| `size` | beacon radius, in dips | 160 |
| `double-ctrl` | bool; `double-ctrl-ms` tap window, 250–600 | false, 400 |
| `shake` | bool; `shake-sensitivity` 1–5 | false, 3 |
| `shake-needs` | `none`, `ctrl`, `shift` | `none` |
| `recall` | `none`, `center` | `none` |

New action `find-mouse`, bindable like any other (hotkey, mouse shortcut,
tray). Severity split per `CLAUDE.md`: a malformed document is fatal with a
line number; unknown style/attribute, out-of-range value, bad colour are
non-fatal warnings and the default is used.

## Phase 0 — probes on the real machine

Throwaway code, removed afterwards (as in the mouse-shortcuts plan). The user
runs them — the live desktop is not driven without asking.

1. **Ctrl-tap detection.** Log, from `WH_KEYBOARD_LL`, the timing of two quick
   `Ctrl` taps with nothing between them; check left/right Ctrl and that a
   `Ctrl+C` is not counted.
2. **Shake signal.** Log the move stream during a deliberate shake versus a
   fast window drag versus normal pointing; pick thresholds for reversals,
   path length and net displacement.
3. **Cursor draw.** Draw `GetCursorInfo().hCursor` scaled 4× into an `Overlay`
   canvas via `DrawIconEx`; check I-beam, wait, resize and app-owned cursors,
   and a per-monitor-DPI mix.
4. **Guard signals.** Print `SHQueryUserNotificationState`, `GetCursorInfo`
   flags and `GetClipCursor` in a game, a fullscreen video and a VM window.

Open questions the probes settle: does `DrawIconEx` handle animated and
alpha cursors; is `SHQueryUserNotificationState` fast enough to call per
trigger; does a borderless-fullscreen game show in any of the signals.

### Results, first round (2026-11-13)

* **Double Ctrl** — detection works, but fired too often: the probe had no
  limit on how long Ctrl is held or how far apart the presses are, so "press,
  hold, press again" counted. **Rules to adopt:** each press held ≤ 200 ms
  (`TAP_MAX_HOLD`); presses at most 400 ms apart, down to down
  (`DOUBLE_MAX_GAP`); a long press forgets the pending tap; any other key
  between or during taps cancels. Both limits become `<finder>` attributes if
  they need tuning.
* **Shake** — far too many detections (≥3 reversals in 600 ms, 40 px segments,
  400 ms re-arm). **Rules to try:** reversals need 60 px of travel back; mean
  speed over the window ≥ 1800 px/s (slow wiggles and normal pointing fail
  it); 1 s cool-down and the buffer cleared after a detection. In pixels, so
  DPI-scaled in the real thing.
* **Shake kinds** — the user wants the indicator to depend on *how* the shake
  is made, each kind tweakable. The probe now classifies: `horizontal` and
  `vertical` (≥4 reversals on one axis, ≤1 on the other), `circle` (signed
  turning ≥ 450°), `chaotic` (≥5 reversals total). Configuration later maps a
  kind to a style (e.g. horizontal → beacon, circle → spotlight), with the
  thresholds as advanced attributes. To be validated by the second round.
* **Cursor ghost — dropped.** A 4× copy is visibly pixelated (cursors are
  32×32 bitmaps), and a text cursor is a *monochrome AND/XOR* image
  (`0x10005`, 32×32, hotspot (8, 9), 416 visible pixels): it has no alpha to
  recover, so it renders as a poor stencil and looks off-centre. Decision 1 of
  this plan is amended: the finder draws **its own shapes only** (rings, halo,
  crosshair, spotlight, map), centred on the pointer position, never a copy of
  the cursor. Drawing a 4× cursor is out; if a bigger pointer is wanted later,
  the supported route is the system's pointer-size setting, not ours.
* **Guard** — VLC full screen showed nothing special in the log. The log was
  overwritten by a later probe, so nothing was kept to analyse; the probe now
  writes one log per mode (`finder-probe-<mode>.log` in the temp directory).
  Not concluded: needs a re-run, see below.

## Phase 1 — model, parsing, writing

- `Finder` struct and `Action::FindMouse` in `action.rs` / `config.rs`;
  `from_name`/`name` pairs, `to_xml` writer (`write_finder`, omitting defaults
  like `write_u32_if_changed` does), `<exclude>` list.
- Warnings for the non-fatal cases above.
- Tests: default configuration still valid, round trip, each warning.

## Phase 2 — the action and the beacon

- `y-snap/src/finder.rs` (logic: state machine, `allowed()`) and
  `y-snap/src/finder/overlay.rs` (drawing), following `min_overlay.rs`'s shape.
- Beacon: converging rings, halo, 4× cursor ghost; ~300 px canvas at the
  cursor, per-pixel alpha, anti-aliased circles drawn by hand into the
  `Canvas`.
- A `WM_TIMER` (~16 ms) exists only while animating; frames are `refresh`ed,
  the position re-read with `GetCursorPos` each tick so the overlay follows
  the pointer.
- Clamp with `Monitor::keep_inside`; scale with `Monitor::scaled`.
- Wire `find-mouse` through `actions.rs` and the tray menu.

## Phase 3 — the guard

- `finder::allowed()` as tabled above, plus `<exclude>` matching.
- `force="true"` on a binding bypasses it.
- Tests for the pure part (exclude matching, guard combination); the Win32
  signals are checked manually.

## Phase 4 — triggers

- **Double Ctrl** in `y-core`'s keyboard hook (or a small sibling in
  y-snap if extending the hook is messier): a tiny state machine counting
  clean Ctrl down/up pairs inside `double-ctrl-ms`; any other key or mouse
  button resets it. Swallows nothing. Needs a way to arm/disarm per
  configuration, like `modifier_guard::arm`.
- **Shake** in `mouse/mod.rs`: fixed ring of recent `(x, t)` samples;
  fire on ≥3 horizontal reversals within ~600 ms with a path-to-displacement
  ratio above the sensitivity threshold, no button down, optional required
  modifier. Posts `WM_FINDER` (next `WM_APP_FIRST + n`). A cool-down avoids
  re-firing mid-animation (decision 7 restarts instead, but a continuing shake
  must not restart it forever: ignore triggers for the first ~250 ms).
- Both honour decision 6.
- The watchdog (`check_liveness`) already covers a dropped mouse hook.

## Phase 5 — more styles

- `monitor`: outline flash on the cursor's monitor and a mini-map of every
  monitor with a dot at the pointer (`Monitor::all`, `sort_spatially`). Ties
  in with [`multi-monitor-plan.md`](multi-monitor-plan.md) — share its
  monitor-numbering rather than inventing one.
- `crosshair`: full-width and full-height lines on the cursor's monitor.
- `spotlight`: dimmed monitor with a clear circle; the costliest per frame,
  so a lower frame rate and a pre-rendered mask.

## Phase 6 — `recall`

`recall="center"`: the trigger also warps the pointer to the centre of the
active window (reusing the `center-cursor` implementation) before the beacon
starts, for "just give me my mouse back".

## Phase 7 — `y-snap-config`

A "Mouse finder" screen built from `ui-kit` (read `ui-kit/docs/ui-toolkit.md`
first): style picker with a preview, duration/size/colour, the two trigger
switches with the Windows-overlap note (read `SPI_GETMOUSESONAR` and say so),
sensitivity, exclude list, and a "Test" button.

## Phase 8 — documentation

Per the house rule, each feature updates the app's docs: `y-snap/README.md`,
`docs/user-manual.md` (how to use, the Windows overlap, the game guard),
`docs/configuration.md` (the table above), `docs/development.md` (hooks,
guard signals, why the cursor is not resized).

## Risks

| Risk | Mitigation |
| --- | --- |
| Double Ctrl false positives (Ctrl-heavy typists) | strict "nothing in between" rule, adjustable window, opt-in |
| Shake false positives during drags / games | no button down, guard, sensitivity, optional modifier |
| Hook-time budget (300 ms) | detectors are arithmetic on fixed arrays; all drawing posted out |
| Overlay shows over a fullscreen game anyway | guard signals plus exclude list; verify in Phase 0 |
| Per-monitor DPI mix when the pointer crosses monitors | re-derive the monitor each tick |
| App-owned or animated cursors draw badly | fall back to the arrow cursor in the ghost |
| Both Windows' overlay and ours fire | detect `SPI_GETMOUSESONAR`, warn in the screen and docs |

## Manual checks

1. `find-mouse` hotkey on each monitor, at 100 % and 150 % scaling.
2. Double Ctrl: fires on two clean taps; not on `Ctrl+C`, not on one slow pair.
3. Shake: fires; not during a window drag; not while a button is held.
4. Silent in a fullscreen game, a video, a presentation, a VM; `force` works.
5. Excluded process: silent while it is foreground.
6. A second trigger mid-animation restarts it, leaving one overlay.
7. Idle: no extra window, timer or memory growth (check with Task Manager).
8. Reload with the section removed returns to defaults, no warnings.

## Progress

Phase 0 done; the results are in the sections below. Phase 1 built: see the end of this file.

### Results, second round (2026-11-13)

* **Horizontal and vertical never fired**: they required ≤ 1 reversal on the
  other axis, which a human hand never achieves. **Rule now:** `horizontal` =
  ≥ 3 reversals on x *and* x travel ≥ 1.5× y travel (y may wobble freely).
  `vertical` is the mirror and may well prove impractical with a mouse: keep
  it only if it ever fires in practice, otherwise drop it.
* **Circle fired on non-circles** (ellipses, back-and-forth scribbles): the
  turning angle summed the ±180° flips of each reversal, which accumulate
  into a fake rotation. **Rule now:** only *smooth* corners (< 120°) add to
  the turning, and a circle needs ≥ 450° of it with at most one sharp corner.

### Direction after round two (2026-11-13)

* **Everything is a tweakable value.** The probe takes `name=value` arguments
  (`window`, `segment`, `speed`, `reversals`, `dominance`, `circle`,
  `cooldown`, `ctrl-speed`, `ctrl-reversals`) and prints the effective set at
  start, so thresholds are tuned live without recompiling. In the real
  feature these become `<finder>` attributes (an "advanced" group in the
  config screen) with the tuned values as defaults.
* **A second, easier trigger: shake while Ctrl is held.** Holding Ctrl already
  says "I am looking for the pointer", so the shake bar is lower (defaults
  `ctrl-speed=900`, `ctrl-reversals=2`). Ctrl is only *read* (the keyboard
  hook swallows nothing, so there is no key to cancel and no side effect on
  the focused app). This is `shake-needs="ctrl"` of the schema, refined: it
  becomes a profile (relaxed while Ctrl is held) rather than a requirement,
  so a plain strong shake still works if `shake` is on.

### Tuned values (2026-11-14) — the defaults, and what the config screen exposes

Read from the user's last shake run (`finder-probe-shake.log`):
`window=700 segment=120 speed=3000 reversals=3 dominance=1.5 circle=450
cooldown=1000 ctrl-speed=1200 ctrl-reversals=2`. In that run 11 shakes fired
(all `horizontal`, 3 of them with Ctrl held) and the `near miss` lines were
slow wiggles (1800–2900 px/s) that the speed gate rightly refused. Double
Ctrl keeps `TAP_MAX_HOLD=200`, `DOUBLE_MAX_GAP=400`.

| Setting | Default | Range | Screen | Note |
| --- | --- | --- | --- | --- |
| Double Ctrl on/off | off | | basic | |
| Shake on/off | off | | basic | |
| Shake sensitivity | medium | low / medium / high | basic | one slider driving `speed` and `segment` below; the raw values are the advanced group |
| `speed` | 3000 px/s | 1000–8000 | advanced | the main false-positive knob |
| `segment` | 120 px | 40–300 | advanced | **tuned on one machine in physical pixels: store it in dips and scale by the monitor** |
| `reversals` | 3 | 2–6 | advanced | |
| `dominance` | 1.5 | 1–4 | advanced | horizontal needs x travel ≥ this × y travel |
| `window` | 700 ms | 300–1500 | advanced | |
| `cooldown` | 1000 ms | 300–3000 | advanced | |
| `ctrl-speed` | 1200 px/s | 400–4000 | advanced | relaxed profile while Ctrl is held |
| `ctrl-reversals` | 2 | 1–4 | advanced | |
| `circle` | 450° | 270–900 | advanced | |
| `tap-max-hold` | 200 ms | 100–400 | advanced | double Ctrl |
| `double-max-gap` | 400 ms | 200–700 | advanced | double Ctrl |

Ideas for the screen: a **Test area** (the "Test" button of Phase 7) that
shows the live detection — kind, reversals, speed against the thresholds — so
the sliders can be tuned the way the probe's log was; and a **sensitivity
preset** row (low / medium / high) over the advanced values.

Open points: no `circle` ever fired in the last run (its smooth-turn reached
−645° at 2231 px/s, under the 3000 px/s gate) — a circle is naturally slower
than a shake, so it likely wants **its own speed threshold**; `vertical` never
fired, and `chaotic` is untested. The `guard` probe has still not been
re-run (no log kept), so the VLC question is open.

### Decision: no `chaotic` kind (2026-11-14)

`chaotic` (≥ reversals+2 across both axes, no dominant axis) is dropped from
the probe and from the design: the user does not like it, and it blurs the
kinds. Shake kinds are therefore `horizontal` and `circle`, with `vertical`
kept only if it ever proves reachable. One observation to keep: moving the
mouse *very* rapidly in all directions still triggered other kinds in the
probe — the speed gate is what lets that through, and since every threshold is
a user setting (see the table above), the answer is the settings and the
sensitivity preset, not more built-in rules. A "scribble" that is none of the
kinds should simply do nothing.

### Round three: the indicator styles (2026-11-14)

`finder_probe styles` prototypes the five candidates so they can be judged on
real screens: `beacon`, `spotlight`, `crosshair`, `monitor` (border + map),
`halo` (persistent). Tweakable: `size` (dips), `duration`, `color`, `dim`,
`delay`, `halo-seconds`. It prints per-effect frame statistics (frames,
average and worst ms) and the guard snapshot at each trigger, which also
re-tests the VLC question. Small effects (beacon, halo) use a pointer-sized
overlay that moves with the pointer; the others use a monitor-sized overlay on
the pointer's monitor. Note the cost: a 4K monitor-sized layered surface is
~33 MB — acceptable for a 1 s effect that is destroyed afterwards (decision 2),
not for anything persistent. Waiting for the user's verdict.

### Round three feedback and round four (2026-11-14)

* Verdict: a good start. **Keep `halo`**, with a colour setting.
* **Bug found:** monitor-sized effects did not follow the pointer to another
  monitor. The surface is bound to one monitor, so the finder must rebuild it
  when the pointer's monitor (or its scale) changes. The probe now does, and
  reports how many switches happened.
* **Visibility on any backdrop** (white screens swallow a coloured halo). Ideas
  now in the probe, all based on a `BitBlt` of the screen behind the effect —
  our overlay is a layered window (not part of a plain screen `BitBlt`) and
  opts out of capture (`WDA_EXCLUDEFROMCAPTURE`), so the grab is the backdrop:
  * `color=auto` for any style: the configured colour is darkened on a light
    backdrop and lightened on a dark one, smoothed over a few frames;
  * `6 autohalo`: the same, plus a thin outline of the opposite tone;
  * `7 negative`: a lens that shows the backdrop inverted (a mid-grey backdrop
    inverts to itself, hence a thin outline ring too);
  * `8 contrast`: a ring painted black or white pixel by pixel, against what
    is behind it.
  Costs to judge: one screen grab per frame (the size of the small overlay), and
  the effect samples a backdrop that may itself change under it.
* To decide after the live test: whether `auto` should be the default colour
  mode, and whether `negative` earns its place beside `autohalo`.

### Phase 1 built (2026-11-14)

`y-snap/src/config/finder.rs` (child of `config.rs`): `Finder` with `Shake`,
`DoubleCtrl`, `FinderStyle`, `Recall`; the reader, the writer (only non-default
values, and no section at all when everything is default), and 9 tests. The
default configuration still loads clean and round-trips. `Action::FindMouse`
(`find-mouse`) exists but is **not implemented yet** (dispatch reports
`Pending`, so a tile for it renders disabled): that is Phase 2.
`y-snap-config` got the action in its hotkey list and an icon (compile
requirement, not the Phase 7 screen). A commented `<finder>` example is in
`rsc/y-snap.xml`.

Deviations from the schema above, decided while writing it:

* **`auto-color` is its own attribute**, next to `color="#RRGGBB"`: the
  configured colour is the base the automatic mode darkens or lightens.
* **An explicit `find-mouse` action always bypasses the guard**, so there is
  no `force` attribute on bindings; `guard="false"` turns the guard off for the
  automatic triggers (the default is on).
* **Per-kind styles**: `horizontal-style` and `circle-style` (fall back to
  `style`), the config form of "the indicator depends on the shake".
* **`circle-speed`** is its own threshold (default 1500 px/s, **untuned**);
  `shake-dominance` is a decimal, the other numbers are integers; every
  number has a range (`config::finder_range`) and out of range is a warning.
* The sensitivity preset (low/medium/high) is a screen affordance writing the
  raw values, not a file attribute.
* `<exclude process="Game.EXE"/>` is stored lower-case without `.exe`.

Docs (`configuration.md`, manual) wait for Phase 8, when the feature does
something. Known unrelated failure: `y-coustic`'s test target does not compile
(`paint/layout.rs:480`, `Monitor` literal missing `handle`).

## Rework of the configuration: styles and triggers (2026-11-14)

The flat `<finder>` of Phase 1 could hold one look and one of each trigger.
Replaced (never released) by named styles, each with its own triggers:

```xml
<finder guard="true" shape="beacon" size="160" ...>   <!-- defaults every style inherits -->
  <exclude process="game"/>                            <!-- global: all styles -->
  <style name="quick" shape="halo" auto-color="true" default="true">
    <tap key="ctrl" side="any" count="2" hold="200" gap="400"/>
    <shake mod="Ctrl" shapes="any"/>
  </style>
  <style name="radar" shape="beacon">
    <shake shapes="horizontal,vertical" speed="2500"/>
  </style>
</finder>
```

- **Look** (`shape`, `duration`, `size`, `color`, `auto-color`, `dim`, `recall`)
  cascades: `Look::default()` → `<finder>` → `<style>`. `FinderStyle` became
  `Shape`; `Style` is a name + look + triggers.
- **`<tap>`**: `key` ctrl|shift|alt|win, `side` any|left|right, `count` 2–4,
  `hold`, `gap`. Replaces `double-ctrl`.
- **`<shake>`**: `mod` (same dialect as `<shortcut mod>`), `shapes`
  (`any` or a list of horizontal/vertical/circle), `window`, `segment`,
  `speed`, `reversals`, `dominance`, `degrees`, `circle-speed`, `cooldown`.
  Defaults depend on `mod`: none → speed 3000 / 3 reversals; any modifier
  → the relaxed Ctrl profile (1200 / 2). Replaces `shake*`, `shake-ctrl-*`,
  `horizontal-style`, `circle-style`.
- A trigger exists by being declared: no `enabled` flags. No style = no
  automatic trigger; `find-mouse` then plays `Look` from `<finder>`
  (`Finder::style(None)` always returns something).
- Duplicates (same tap, same modifiers + shape, same style name, second
  `default`) are warnings; the first one wins.
- `shake-speed` range widened to 400–8000, reversals 1–6 (one range for both
  profiles).
- Still to do in Phase 2: an optional `style="name"` on `<hotkey>` and
  `<shortcut>` (today their unknown-attribute check would warn), and the
  tray entry plays the default style.

## Phase 2 built (2026-11-14)

`finder.rs` (`Player`: the animation state, one `WM_TIMER` id 4 at 16 ms that
exists only while playing, overlay created per animation and per monitor
change) and `finder/overlay.rs` (pure drawing + `Capturer`). All seven shapes
were ported from the probe at once (the plan's Phase 5 "more styles" is
therefore done), including `auto-color`, backdrop sensing and
`WDA_EXCLUDEFROMCAPTURE`.

- `find-mouse` is implemented: hotkey, `<shortcut>`, overlay/minbutton tile and
  a new tray entry "Find the mouse pointer" all play the default style (or the
  one named by the binding's `style="name"`, stored as an index into
  `finder.styles`; unknown name or `style` on another action = warning).
  `<finder>` is read before the bindings for that reason.
- Persistent shapes (halo, negative, contrast) play 4× `duration`, capped at
  10 s, with a 0.3 s fade in and out (nothing can dismiss them early yet).
- `recall` is read but not acted on (Phase 6); the guard is not applied yet
  (Phase 3, automatic triggers only).
- Checked: build, clippy, tests. **Not checked live**: the user should try the
  tray entry and a `find-mouse` hotkey with a few shapes, across monitors and
  on white/dark backdrops.

## Order change (2026-11-14)

**The guard (Phase 3) is postponed to just after Phase 4.** Until it exists,
automatic triggers ignore games and full-screen video, so they stay off unless
a style declares them. The guard must be validated live with a **game** (and
the still-unanswered VLC question) using `examples/finder_probe.rs` (`guard`
mode, or `styles` with `g`) *before* `allowed()` is written: the probe stays
in the tree until then, and must not be deleted early.

## Phase 4 built (2026-11-14)

- `finder/detect.rs`: pure, tested detectors. `Taps` (hold ≤ `hold`, down-to-down
  ≤ `gap`, `count` taps, any other key/mouse button/modifier or a different
  side resets, fires on the release of the last tap so a held key never
  fires, auto-repeat ignored, a press older than 1 s with no release is
  treated as a lost release). `Shakes` (fixed 320-sample ring, samples merged
  within 5 ms, judged at most every 10 ms, no allocation in the hook; the probe's
  reversal/smooth-turn/dominance rules per trigger window; exact modifier match;
  a button down clears the history; per-trigger cooldown).
- `finder/input.rs`: a `WH_KEYBOARD_LL` hook installed only if a `<tap>` is
  declared; shakes are fed from the existing mouse hook (`on_move`/`on_button`).
  Injected events ignored. Posts `WM_FINDER_TRIGGER` (`WM_APP_FIRST + 10`,
  `wParam` = style index), handled by `find_mouse`.
- Deviations: distances/speeds are **physical pixels** (no DPI scaling yet; the
  tuned values were physical too). The keyboard hook has no watchdog.
- No guard yet (see "Order change"): automatic triggers also fire in games and
  full-screen video until Phase 3 is done. **Not checked live.**

## Phase 3 built: the guard (2026-11-14)

Probe results (`guard` mode, game = an Unreal Engine title, video = VLC):

| situation | confined | covers monitor | QUNS | cursor shown |
|---|---|---|---|---|
| game in play | **true** | true | 2 (busy) | false |
| game in its menus | false | true | 2 | true |
| game just left (explorer) | false | false | 2 for ~3 s, then 5 | true |
| VLC full-screen video | false | **true** | 5 | true |
| VLC windowed / normal apps | false | false | 5 | true |
| Start menu / search | false | false | 7 (app) | true |

Decisions (the user's, from these):

- **Confined pointer ⇒ all of y-snap is suspended**: the mouse hook passes
  everything through (cached `GetClipCursor`, at most every 250 ms; a drag
  already under way is let finish; the watchdog's `LAST_POINT` is still
  fed), hotkeys are ignored (except `quit`/`reload`), automatic finder
  triggers do nothing.
- **A full-screen window on the pointer's monitor ⇒ the automatic finder is
  quiet** (`guard="true"`, the default). It is the *pointer's* monitor that
  counts, so a video on monitor A does not stop the finder helping with a
  pointer lost on B. "Full-screen" = covers the monitor, **no title bar**
  (a maximised window with an auto-hidden taskbar also covers it), not the
  desktop/taskbar classes.
- `<exclude>` (foreground process) always applies to automatic triggers;
  an explicit `find-mouse` ignores it and the full-screen check.
- **Not used**: `SHQueryUserNotificationState` (busy for the whole game and
  seconds after, "app" for the Start menu) and "cursor hidden" (it flips with
  text fields too).
- Code: `guard.rs` (pure `blocks`/`covers_screen` + tests, Win32 `read`,
  `confined`, `confined_cached`); `main.rs::on_finder_trigger`.
- Possible later setting: switching the confined suspension off.
- **Not checked live** (running with the game: automatic triggers quiet, hotkeys
  ignored in play, VLC full-screen quiet, VLC on another monitor not).
- `examples/finder_probe.rs` was removed (2026-11-15); the measurements are above.

## Notes for later: idle cost of the mouse hook (2026-11-15)

Raised while checking that a disabled finder costs nothing. The finder itself
is fine (no `<tap>`/`<shake>` means no keyboard hook, `on_move` returns at
once, no overlay or timer). Two points remain on the **mouse hook**, which
y-snap installs whatever the configuration. Not specified or started.

- **P1: no hook when nothing needs it.** *Built 2026-11-15:*
  `MouseHook::needed` (drag, resize, shortcuts, min-button tiles or
  right-click, finder shakes) gates the install in `YSnap::load`, which also
  covers reload; the hover hit test is skipped per move when neither min-button
  feature is on (`Hover::enabled`). Live check pending: an empty `y-snap.xml`
  leaves no `WH_MOUSE_LL` hook and no timer 1.
  - If there is no finder trigger (`<shake>`) and no min-button (min overlay)
    — and, by the same reasoning, no mouse gesture and no mouse shortcut —
    the `WH_MOUSE_LL` hook (and its watchdog timer) should not be installed.
  - **Reload config** must be handled: it can enable or disable any of those
    features, so the hook has to be installed or removed to match the new
    settings (`load` already tears down and rebuilds, so the decision belongs
    there). Do not forget the watchdog timer (`REHOOK_TIMER_ID`) and the
    thread-local state when removing it.
  - Today `MouseHook::install` documents that it installs even with every
    gesture unset, because min-button hover works alone; that comment and
    behaviour change.
- **P2: the guard's per-event cost.**
  - `guard::confined_cached` runs in the mouse hook (`mouse/mod.rs`) for every
    event, even with no finder: `GetClipCursor` plus a few `GetSystemMetrics`
    at most every 250 ms, otherwise a cached read. Small, but paid by someone
    who uses none of it.
  - To improve, once P1 settles which features need the hook. **A short
    specification must be written first** (what is suspended when the pointer
    is confined, when the check runs, whether it stays in the hook), so that
    it is decided, not guessed.

## Phase 7 built: the "Mouse finder" screen (2026-11-15)

`y-snap-config/src/finder.rs`, a new section after Mouse shortcuts.

- **General:** the full-screen guard switch, the excluded applications (typed
  name, normalised as y-snap compares them), and an "Edit" for the default look
  (`<finder>`'s own attributes: find-mouse plays it with no style, a new style
  starts from it).
- **Styles:** a `Table` (name, look, trigger summary, a Default button, delete)
  with an inline editor: name, look (shape, duration, size, dimming, adapt the
  colour, colour picker) and trigger cards (`<tap>`: key, side, taps, longest
  press, longest gap; `<shake>`: keys held, kinds of shake, and all eight
  tuning values).
- **Model:** every change is a pure function (`apply_look/tap/shake`,
  `add_style`, `delete_style`, `add_exclude`, `style_problem`) that says whether
  it changed anything, with tests. Same editing model as the other screens
  (snapshot, Cancel reverts, Done keeps).
- **Not built, on purpose:** (the Test button came after, see below) (no way to ask a running y-snap to
  play an unsaved style; the tray entry plays the saved default), a shake
  sensitivity preset, the `recall` choice (Phase 6),
  a warning for
  two clashing triggers (y-snap reports it on load), reordering styles.
- **Checked:** clippy clean; `y-snap-config` 19 tests pass. **Not looked at
  live** (see the Mouse finder section of `y-snap-config/docs/design/ui-checklist.md`).

Follow-up (2026-11-15): the colour is a swatch button that opens the picker on
demand; a `find-mouse` hotkey or mouse shortcut picks its style in the Target
cell (header "Target / style", `finder::style_combo`: "(default style)" then the
styles; disabled with no style). Changing such a row's action to another clears
its style, since y-snap warns about a style anywhere else.

**Test button (2026-11-15).** Each look editor (default look and styles) has a
Test button that plays the look as it is on screen, saved or not, in the
running y-snap, around the pointer (which is on the settings window).

- A second registered message, `ipc::finder_test_message()`, broadcast like the
  reload one. The look travels in the two message parameters (`ipc::pack_look`:
  shape, auto colour, dimming, duration, size in `wParam`, the colour in
  `lParam`; `recall` is left out, a test never moves the pointer). No shared
  memory, no pipe. `y_core::single_instance::notify_running_instance_with` is
  the broadcast with parameters.
- y-snap plays it as `Style::new("test", look)` through the same `Player`. It
  does not go through the guard: the user asked, like an explicit `find-mouse`.
- Any process can post this message, so `ipc::unpack_look` refuses an unknown
  shape or a number outside the ranges the file itself obeys (tested, with the
  round trip).
- The button is disabled, with a line saying why, when y-snap is not running
  (checked when the screen is drawn).
- **Not looked at live.** To try: with y-snap running, open a style, change the
  shape or colour without saving, press Test.

## Phase 6 built: `recall` (2026-11-15)

`recall="center"` on `<finder>` or a `<style>`: before the indicator plays,
the pointer is moved to the middle of the focused application window.

- In `main.rs::find_mouse`, so every way of playing a style gets it: the
  `find-mouse` action (hotkey, shortcut, tray) and the automatic triggers.
  The guard has already had its say by then.
- The window is `y_core::target::focused()` (the same filter as a hotkey's
  `focused` target: no taskbar, tooltip or shell surface) and the move is
  `window::center_cursor`, the `center-cursor` action's own code. With no such
  window the pointer stays where it is and the indicator plays there.
- The Test button never moves the pointer (the look it sends has no recall).
- Screen: a "When it plays" dropdown in each look editor ("Only show where it
  is" / "Bring it to the active window").
- **Not looked at live.** To try: `recall="center"` on a style, lose the pointer
  on another monitor, trigger it. Worth watching: a shake that warps the pointer
  must not set another trigger off (the trigger's cooldown should cover it).

**Reordering styles (2026-11-15).** The styles table has the Reorder toggle and
the ↑/↓ rows of the other lists (`reorder::list_footer` / `row_actions`,
`ListKind::FinderStyles`). The order is the file's, and it matters: the first
style is the default unless one says `default="true"`, and a duplicate trigger
is won by the earlier style. A hotkey's or shortcut's `style` is a position, so
`finder::move_style` swaps the two styles and remaps the default and every
binding's `style` with them (tested): a hotkey keeps playing the same style.

## Phase 8 built: documentation (2026-11-15)

- `y-snap/docs/configuration.md`: the `<finder>` reference (attributes and
  ranges, shapes, `<exclude>`, `<style>`, `<tap>`, `<shake>`, when it stays
  quiet), the `find-mouse` action, `style` on `<hotkey>` and `<shortcut>`.
- `y-snap/docs/user-manual.md`: "Finding the pointer" (asking, tapping, shaking,
  looks, the guard, `recall`, the Windows overlap), the tray entry, the action
  table, the settings pages list, two troubleshooting entries.
- `y-snap/README.md`: a "Find a lost pointer" section.
- `y-snap/docs/development.md`: the six new files, and how the finder works
  (hooks, idle cost, capture exclusion, why the cursor is not resized, the guard
  signals and what was rejected, the Test message).
- `y-snap-config/docs/user-manual.md`: the Mouse finder page, the Target / style
  column.
- Not done: screenshots of the new page (there is none yet), and the Windows
  Mouse Sonar overlap is only documented, not detected (the plan's risk table
  suggested reading `SPI_GETMOUSESONAR` to warn in the screen).

`examples/finder_probe.rs` was removed (2026-11-15) once the guard, the shapes and
the triggers were built from it. Its measurements are in this plan (Phase 0, the
guard table); the code is in git history, in the commit that added it.
