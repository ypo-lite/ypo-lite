# Multi-monitor management — implementation plan

Three changes to how y-snap treats monitors, plus a survey of what could follow:

1. A setting that allows dragging and resizing maximized windows, off by default.
2. With that setting off, dragging a maximized window does not move it, until the
   pointer reaches another monitor: the window then goes there, still maximized.
3. Actions that send a window to the previous or next monitor.
4. Other multi-monitor features worth adding: see [Investigation](#investigation).

All phases are built; the manual checks below are the remaining work. Status is in [Progress](#progress).

## Where things stand

* **Drag and resize ignore window state.** `mouse/gesture.rs` takes the rect at the
  press and applies offsets with `SetWindowPos`. A maximized window slides around
  still flagged `WS_MAXIMIZE`, and its size does not fit the monitor it lands on.
* **Monitors are only ever looked up one at a time.** `y_core::monitor::Monitor` can
  describe the monitor at a point or under a window, but nothing lists them.
* **HulK has nothing to port.** Its only monitor code centres the overlay
  (`HulK/src/overlay.cpp:542`). AltSnap is the useful prior art.

## What AltSnap does

From `hooks.c` (GPLv3: read for behaviour, no code taken).

| AltSnap | y-snap plan |
| --- | --- |
| `BLMaximized` (default off) blocks actions on maximized windows entirely | Same idea as the default here, plus the monitor move |
| Dragging restores a maximized window at once, with one `SetWindowPlacement` (`showCmd = SW_RESTORE`, new `rcNormalPosition`) | Not by default: the window stays maximized. A candidate for later, [C8](#investigation) |
| `AutoRemaximize` (default off): at **drop**, if the monitor changed, centres `rcNormalPosition` on the target's work area and calls `SetWindowPlacement` with `SW_MAXIMIZE` | y-snap moves at the **crossing**, as requested, from a window that is still maximized. AltSnap's path starts from a restored window, so it does not prove ours works: see Phase 0 |
| After a restore on Windows 10+, re-checks the invisible borders and corrects with `SetWindowPos` | Same concern for restored windows in `move_to_monitor` |
| No next/previous monitor action found in the parts read; the monitor list is in `EnumDisplayMonitors` order | y-snap sorts by position, decision 8 |

Only part of `hooks.c` was read word for word, and `snap.c` and `zones.c` were not
fetched, so the "not found" above is not proof of absence.

## Decisions

Taken for this plan. The first four are worth a second look.

1. **One setting covers drag and resize:** `allow-maximized` on `<mouse>`. When on,
   maximized windows behave exactly as they do today, flag and all. It keeps the old
   behaviour available; it does not improve it.
2. **The default changes for existing users.** A maximized window stops following an
   `Alt` drag after this update, with no config change. The manual and release notes
   have to say so.
3. **The window moves when the pointer crosses**, not at the drop. The move is heavy
   (the application lays itself out again), so pointer wobble on a boundary is a
   known risk: see Phase 2 and the manual checks. Moving at the drop is the fallback.
4. **Four actions, not two,** matching the desktop vocabulary:

   | Action | Window | Pointer |
   | --- | --- | --- |
   | `monitor-move-next` / `monitor-move-prev` | moves | goes with it, to the same relative spot |
   | `monitor-send-next` / `monitor-send-prev` | moves | stays |

   The pair earns its place: with `target="pointed"`, a `send` leaves the pointer
   over another window, so pressing the hotkey again moves *that* one. `move` can be
   repeated to walk a window across monitors. `focused` works with either.
5. **The pointer decides the monitor**, not the window, which is maximized and would
   only change monitor once it has moved.
6. **A press on a maximized window is always claimed**, with the setting off. The
   application does not get a stray `Alt`+click, `modifier_guard` is armed as usual,
   and the release is swallowed. Resize becomes a no-op that still owns the pointer.
7. **Only `WS_MAXIMIZE` counts.** Fullscreen windows without that flag (F11, borderless
   games) drag as they do today. See [C7](#investigation).
8. **Monitor order is spatial:** sorted by left edge, then top, wrapping at both ends.
   `EnumDisplayMonitors` order is not something a user can predict.
9. **Placing a restored window:** position scales with the work areas, size stays, and
   the result is clamped inside the target's work area, shrinking if it has to. A
   window with `IsWindowArranged` true (snapped) scales position *and* size, so a left
   half stays a left half. Size differences from DPI are left to the window, which
   receives `WM_DPICHANGED`; the clamp runs on the size it ends up with.
10. **A maximized window stays maximized, and un-maximizing it must land on the new
    monitor.** This is the acceptance test for Phase 0.
11. **Nowhere to go is silent:** with one monitor the actions do nothing. A minimized
    window is left alone.

## Configuration schema

```xml
<mouse allow-maximized="true">   <!-- default false, written only when true -->
	<drag   button="left"  mod="Alt"/>
	<resize button="right" mod="Alt"/>
</mouse>

<hotkey key="Ctrl+Win+Alt+Right" action="monitor-move-next" target="focused"/>
<action key="Down" action="monitor-send-next" label="Next monitor" icon="⏭️"/>
```

The attribute goes on `<mouse>` because it applies to both gestures. A wrong value is
a warning and the default applies, as elsewhere. The actions work everywhere an action
can be named: hotkeys, shortcuts, overlay tiles, the minimize-button popup.

## Phases

### Phase 0 — Probe the maximized move ✅

The whole of item 2 rested on one unanswered question: how to take a window that *is*
maximized to another monitor so that it lands right and restores there. It needed real
windows, so it was a probe, as in the mouse shortcuts plan. Historical: the probe code
(`src/probe.rs`, the `probe` feature) is gone.

| Candidate | Sequence | Result (user's monitors) |
| --- | --- | --- |
| A. In place | `GetWindowPlacement`, move `rcNormalPosition` to the target, `SetWindowPlacement` with `SW_SHOWMAXIMIZED` | Did not work |
| B. AltSnap's shape | The same, but `SW_RESTORE` first, then a second `SetWindowPlacement` with `SW_SHOWMAXIMIZED` | Lands right, but the restore/maximize animation shows |
| C. Plain | `ShowWindow(SW_RESTORE)`, `SetWindowPos` onto the target, `ShowWindow(SW_MAXIMIZE)` | Same as B |
| B/C + DWM | B or C with `DWMWA_TRANSITIONS_FORCEDISABLED` set on the window meanwhile | B: better, less animation |
| **D. Never restore** | `SetWindowPos` while still maximized (same place on the target's work area, frame overhang included), then `SetWindowPlacement` with the moved `rcNormalPosition` and `SW_SHOWMAXIMIZED` | **Best: no visible animation. Un-maximize afterwards lands on the target. Chosen** |

D is what `window::move_maximized` does. The shell's own move
(`Win+Shift+Left/Right`) shows no animation either, which is the feel D matches.

Not recorded: a run across every application, a taskbar on the top or left, and a
`hide-taskbar` window. The code handles the workspace-versus-screen origin, but those
stay on the [Manual checks](#manual-checks) list. The foreground question was not
measured either: see the manual checks.

### Phase 1 — Monitor list and placement

The pure part, testable without a second monitor.

`core/src/monitor.rs`:
* `Monitor::all() -> Vec<Monitor>`, through `EnumDisplayMonitors`, sorted as in
  decision 8. Allocates, so message loop only.
* A `handle: HMONITOR` field on `Monitor`.
* `handle_at(point) -> HMONITOR`, a bare `MonitorFromPoint`, for the hook.

`y-snap/src/window.rs`:
* `placed_rect(rect, from, to, arranged) -> RECT`: decision 9, pure, with unit tests
  for negative coordinates, a smaller target, a stacked layout and a snapped half.
* `move_to_monitor(window, &Monitor)`: the Phase 0 winner for a maximized window,
  `placed_rect` plus `SetWindowPos` with `SWP_NOZORDER | SWP_NOACTIVATE` otherwise,
  then one correction pass for the size the window settled on.

`wrapped_neighbour` in `actions.rs` already does the wrapping.

### Phase 2 — Maximized gestures

`config.rs`:
* `Mouse::allow_maximized: bool`. `Reader::mouse` adds
  `self.check(section, &["allow-maximized"])` and reads it the way `steal-from-shell`
  is read; `write_mouse` writes it only when true. A round-trip test.

`mouse/gesture.rs`:
* `Pinned { button, window, monitor: Option<HMONITOR> }`, a third state beside `drag`
  and `resize`. `Some` for a drag, `None` for a resize. `active()` counts it.
* In `on_button_down`, after `target::at`: if `!allow_maximized` and
  `window::is_maximized(window)`, start `Pinned` instead. That check reads a style
  bit, with no message sent, so it is safe in the hook.
* `on_button_up` ends it on the matching button.
* `on_move` can no longer return `Option`: `None` means "not mine, track hover", and a
  pinned gesture must not fall through. It returns an enum instead:
  `Apply(HWND, RECT)`, `Monitor(HWND, POINT)`, `Claimed`, `Idle`.
* `Monitor` is returned once per change: `handle_at(pt)` compared with the stored
  handle, which is then updated.

`mouse/mod.rs`:
* `WM_GESTURE_MONITOR = WM_APP_FIRST + 9`, the next free id.
* A `PENDING_MONITOR` cell shaped like `PENDING_APPLY`, latest wins, carrying the
  point rather than the handle so a monitor that vanished cannot be dereferenced.
* `apply_pending_monitor()` on the message loop: checks `window::exists`, checks the
  window is not already there (this absorbs a quick wobble), calls `move_to_monitor`.

`main.rs`: one arm for the message.

If wobble proves annoying in the manual checks, add a short dwell before applying. If
that is not enough, move at the drop (decision 3).

### Phase 3 — Monitor actions

* `action.rs`: the four variants, in `from_name`, `name` and the round-trip test.
  `needs_window` is already true for them.
* `actions.rs`: the arms in `dispatch`, a `to_monitor(window, next, follow)` beside
  `relocate`, and `is_implemented`. No `toggle_state`: they have no on/off.
* Pointer follow: `SetCursorPos` to the same relative spot in the target's work area.

### Phase 4 — `y-snap-config`

* `mouse.rs`: an "Allow on maximized windows" switch under the cards, with a hint
  explaining what happens when it is off. `Msg::AllowMaximizedToggled`.
* `hotkeys.rs` `ACTIONS` (26 → 30) and `overlay/mod.rs` `OVERLAY_ACTIONS` (28 → 32).
* `icons.rs`: suggestions for the four actions. The match is exhaustive, so the
  compiler points at it.
* `mouse_shortcuts.rs`: check whether it has its own action list.

### Phase 5 — Documentation and defaults

* `docs/user-manual.md`: *Moving and resizing* gets the maximized rule, *What each
  action does* gets the actions, and the behaviour change gets a clear mention.
* `README.md`: the attribute and the actions.
* `rsc/y-snap.xml`: the attribute as a comment, a commented hotkey example, and two
  tiles on `Up` and `Down`. The combination in the schema above needs checking on a
  real machine first. `Win+Shift+Left/Right` is the shell's own and would need
  `steal-from-shell`.

## Risks

| Risk | Effect | Handling |
| --- | --- | --- |
| No candidate is clean | Flicker on every crossing, or a wrong restore position | Phase 0 before anything else; C is the fallback and is known to give the right end state |
| Pointer wobble on a boundary | Window bounces, application re-lays out each time | Latest-wins cell, then dwell, then move at the drop |
| Hook timeout | Hook dropped | Hook only calls `MonitorFromPoint` and reads a style bit; the move is posted |
| Target window busy or hung | y-snap's message loop waits | Same exposure as `apply_pending_gesture` and `window::maximize` today |
| Monitor unplugged during a drag | Stale target | Point carried, not a handle; `Monitor::all()` re-read for each action |
| Shaded window changes DPI | `y-snap.height` is in physical pixels, so unrolling gives a slightly wrong height | Accepted; noted in the doc comment |
| Elevated window | No events, or `SetWindowPos` refused | Same limit as drag and resize |

## Manual checks

- [ ] Default config: `Alt`+drag within a monitor leaves a maximized window where it is.
- [ ] Same, crossing a monitor: the window goes there at the crossing, still maximized.
- [ ] Un-maximize after that: lands on the new monitor.
- [ ] Cross and come back in one drag: ends up where the pointer ends.
- [ ] Hover back and forth on the boundary: no bouncing that gets in the way.
- [ ] `Alt`+right on a maximized window: nothing moves, no context menu.
- [ ] Release `Alt` before and after the button: no system menu or Start menu.
- [ ] No minimize-button popup appears during a pinned drag.
- [ ] `allow-maximized="true"`: drag and resize behave as before this work.
- [ ] Normal windows drag and resize exactly as before.
- [ ] Each action on normal, maximized and snapped windows, in both directions, wrapping.
- [ ] Between the scaled and unscaled screens, to a smaller monitor, and with monitors
      to the left of or above the primary (negative coordinates).
- [ ] `move` with `target="pointed"`, repeated: walks the same window round.
- [ ] One monitor: nothing happens, no error.
- [ ] Taskbar on the top or left, and a `hide-taskbar` window: restore lands correctly.
- [ ] Unplug or switch off a monitor mid-drag: nothing sticks.
- [ ] Same on another virtual desktop.

## Investigation

Item 4: what else is worth having. Ranked by fit with "do just what they need to do".

### Recommended next

| | Feature | Why | Cost |
| --- | --- | --- | --- |
| C1 | **Check the drag across DPI** | Likely an existing bug, not yet confirmed. `Move` keeps the size from the press and `apply_pending_gesture` sends it on every move, so after crossing to a screen with another scale, y-snap may fight the window's own `WM_DPICHANGED` resize. AltSnap special-cases a DPI change here. If confirmed, drags should use `SWP_NOSIZE` | One manual check, small fix |
| C2 | **`monitor-move` / `monitor-send` with `monitor="2"`, and `monitor-list`** | The exact shape of the desktop actions: one tile per monitor. Better than next/prev from three monitors up | Small; `Monitor::all()` gives the numbering, though it follows layout rather than Windows' display numbers |
| C3 | **`cursor-monitor-next` / `-prev`** | Jump the pointer to the middle of another monitor. Saves a long swipe across a wide desktop, and pairs with `center-cursor` | Very small |

### Worth considering

| | Feature | Why | Cost |
| --- | --- | --- | --- |
| C4 | Directional `monitor-left/right/up/down` | For grids and stacked screens, where next/prev is not the direction you mean | Small: nearest monitor along an axis, pure and testable |
| C5 | Focus another monitor: `focus-monitor-next` / `-prev` | Activate the topmost window over there, for keyboard users | Medium: reuses `target.rs` filters; foreground rules apply |
| C6 | Gather: `monitor-gather` | Bring every window to the current monitor, handy after undocking or for off-screen strays | Small to medium: window enumeration through `target.rs` filters |
| C7 | Treat fullscreen like maximized | F11 windows and borderless games can currently be dragged by accident (decision 7) | Small: rect equals `Monitor::bounds` with no caption. Exclusive fullscreen stays out |
| C8 | `restore` mode for maximized drags | The Aero feel: pull a maximized window and it restores under the pointer, keeping the grab point. AltSnap's default | Small to medium; would be its own attribute |

### Probably not

| | Feature | Why not |
| --- | --- | --- |
| C9 | Swap all windows between two monitors | Rare, and z-order and DPI make it fragile |
| C10 | Span a window across all monitors | Rare, and needs restore state on the window. Easy to add later with a property, like `shade` |
| C11 | Remember layouts across dock and undock | The most-wanted feature in this space, but the costliest: a snapshot store, monitor identity, `WM_DISPLAYCHANGE` timing. Windows 11 has its own "Remember window locations based on monitor connection"; try that first |
| C12 | Lock or slow the pointer at a monitor edge | Different product, and `ClipCursor` is released by other applications |
| C13 | Snap to edges or zones while dragging | Valuable, but a feature of its own, not a monitor one. Deserves its own plan |

## Progress

| Phase | Status | Notes |
| --- | --- | --- |
| 0. Probe | ✅ | Candidate D chosen: no animation, restores on the target. Probe code removed |
| 1. Monitor list and placement | ✅ | `Monitor::all`/`handle`/`handle_at`/`sort_spatially`, `placed_rect` (6 tests), `move_to_monitor` with `move_maximized` (D). No caller until Phase 3 |
| 2. Maximized gestures | ✅ | Built: `allow-maximized` (config, round-trip and warning tests), `Pinned` state and `Step` in `gesture.rs`, `WM_GESTURE_MONITOR` + `apply_pending_monitor`. Needs the live manual checks (wobble on the boundary above all) |
| 3. Monitor actions | ✅ | Four actions in `action.rs`/`actions.rs` (`to_monitor`, `scaled_point` with 2 tests); icon suggestions added in `y-snap-config/src/icons.rs` so the workspace compiles. Not yet live-checked; not yet offered in the config UI (Phase 4) |
| 4. `y-snap-config` | ✅ | Built: "Allow on maximized windows" switch on the Mouse screen, `ACTIONS` 30 and `OVERLAY_ACTIONS` 32 (the mouse-shortcuts screen reads `ACTIONS`), icons (⏩ ⏪ added to the catalog). Not yet looked at in the running app |
| 5. Documentation and defaults | ✅ | Manual (new *Several monitors* section, maximized rule and behaviour-change note), README, configuration reference, development notes, `rsc/y-snap.xml` (tiles on `Up`/`Down`, commented hotkeys and mouse shortcuts incl. the side buttons) |
