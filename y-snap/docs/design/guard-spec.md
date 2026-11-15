# Guard: when y-snap stays out of the way (spec)

Status: **draft, 2026-11-15.** Specifies P2 of the notes in
`mouse-finder-plan.md` ("the guard's per-event cost") and adds game-mode
detection. Nothing here is built. Code: `y-snap/src/guard.rs`.

## Signals

| Signal | Source | Cost | Allowed in a hook |
| --- | --- | --- | --- |
| Confined pointer | `GetClipCursor` vs the virtual screen | one call + four metrics | yes, but see below |
| Full-screen window on the pointer's monitor | foreground window geometry, class, style | several calls | no |
| **Game mode** (new) | `SHQueryUserNotificationState() == QUNS_RUNNING_D3D_FULL_SCREEN` | a shell round-trip, may block | **no** |

Game mode is the signal PowerToys uses. Here it is added next to the other two,
not instead of them. The probe recorded in `guard.rs` found that the *other*
shell states are unusable ("busy" for a whole game, menus and seconds after it
included; "app" for the Start menu), so **only
`QUNS_RUNNING_D3D_FULL_SCREEN` counts** and every other value reads as "no".
It only covers exclusive Direct3D full-screen: borderless games, videos and
remote desktops are still caught by the other two signals.

## What each signal suspends

- **Confined pointer: all of y-snap** (hotkeys, mouse gestures, shortcuts,
  min-button hover, finder), as today, except `Quit` and `Reload`, and except a
  drag or resize that already started, which may finish.
- **Full-screen window and game mode: the automatic finder triggers only**
  (taps, shakes). Hotkeys, gestures and shortcuts keep working, as the user
  asked for them explicitly. An explicit `find-mouse` ignores both, as today.
- The `guard` switch (`<finder guard>`) governs full-screen window **and**
  game mode together. The exclude list and a confined pointer always count.
  *(Open: see question 1.)*

## Where each check runs

The rule is **no guard work in the mouse hook until an event would act.**

- A plain `WM_MOUSEMOVE` that no feature consumes does no guard call.
  The hook asks the guard only at the moment of acting: a gesture button
  press, a shortcut press or tilt, a shake detected, a hover hit test that
  found a minimize button.
- The check there is `confined_cached(event_time)`: one system call at most
  every 250 ms (unchanged), a cell read otherwise. *(Open: question 2.)*
- Full-screen and game mode are never read in a hook. They are read on the
  message loop, when a finder trigger post is handled (`main.rs`, via
  `guard::read`), which already costs a foreground-process lookup.
- Hotkeys call the uncached `guard::confined()` (rare, so cost is irrelevant),
  through the same documented function so there is one rule.

## Changes

1. `guard.rs`: add `Signals::game_mode: bool`, `Reason::GameMode`, and a
   `game_mode()` reader around `SHQueryUserNotificationState`. `blocks` returns
   `GameMode` after `Confined`/`Excluded`, under the `fullscreen_check` flag.
2. `Cargo.toml` (root): add the `Win32_UI_Shell` feature to
   `[workspace.dependencies.windows]` if it is not there.
3. `mouse/mod.rs`: move the `confined_cached` call from the top of
   `mouse_hook` to the acting points above; plain moves return first.
   Keep the `LAST_POINT` update for the watchdog on every move.
4. Tests: `blocks` returns `GameMode` only with the flag on; a non-D3D state
   maps to `false` (the mapping is a pure function, tested without the shell).
5. Docs: the configuration reference and user manual (what `guard` covers).

## Open questions

1. Should game mode have its own switch, or share `guard`? (Proposed: share.)
2. Staleness of the cached confined answer (250 ms). Proposed: keep 250 ms for
   moves, but **re-read on every button press and tilt**, the rare events that
   can swallow a game's click.
3. Read the game-mode signal once per finder trigger, or cache it a second or
   two? (Proposed: once per trigger; triggers are rare.)

## Live checks

Exclusive full-screen D3D game: the finder is quiet, hotkeys still work.
Borderless game and video: unchanged. Empty config: no hook (P1) and no guard
call anywhere.
