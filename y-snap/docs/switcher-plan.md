# Window switcher — options and building blocks

The mouse-gesture `<switch>` is gone (see the commit that removed it). It was
the wrong shape for two reasons, both worth keeping in mind for what replaces
it:

* **It was not a gesture.** Drag and resize are "hold, move, release" and act
  on the window under the pointer. Switching acts on a *list*, needs a
  selection that survives several inputs, and ends with a commit. That is a
  session, not a gesture.
* **It ping-ponged and did not reliably work.** `switch_window` re-enumerated
  `EnumWindows` on every wheel notch and activated the neighbour of the
  current foreground window. Activating a window moves it to the top of the
  z-order, so the next notch's "next" is the window you just left. Beyond
  that: `EnumWindows` also returns windows of *other* virtual desktops (they
  are visible but cloaked), so a notch could target a window that cannot come
  to the foreground; and nothing skipped cloaked UWP frames, tool windows or
  windows with no title.

This document collects what the replacement can be, what it needs, and what
to settle before writing code. Nothing here is implemented yet.

## Building blocks every mode shares

These are the same whatever mode wins, so they come first.

### 1. A candidate list that is frozen for the whole session

The ping-pong fix. When a session starts, enumerate once, filter, order, and
keep that `Vec<Candidate>` untouched until the session ends. Steps move an
*index* in it; they never re-enumerate and never re-read the z-order.

Filter (a window is a candidate when all hold):

* `WS_VISIBLE`, and **not cloaked** (`DwmGetWindowAttribute(DWMWA_CLOAKED)`
  == 0) — this alone removes suspended UWP frames and most other-desktop
  windows;
* on the current desktop — `IVirtualDesktopManager::IsWindowOnCurrentVirtualDesktop`.
  That interface is **public and documented** (`shobjidl_core.h`), unlike the
  internal one `y_core::virtual_desktop` wraps, so this filter needs no
  version table;
* no owner (or owner chain ends at the shell/desktop, as `collect_window`
  did), and `WS_EX_TOOLWINDOW` excluded unless `WS_EX_APPWINDOW` is also set —
  the same rule Alt+Tab uses;
* non-empty title;
* not one of `target::BANISHED`, and not y-snap's own windows (overlay,
  hidden message window).

Minimized windows stay in (Alt+Tab shows them); committing restores them.

### 2. Ordering: z-order vs. real MRU

* **Z-order** (what `EnumWindows` gives): free, and close to MRU as long as
  nobody uses topmost windows or the user did not click-raise without
  focusing. Good enough for a first version.
* **True MRU**: keep our own most-recently-used list, fed by
  `y_core::hotkey::watch_foreground` (already exists, already used by
  `modifier_guard`). Costs a little state, but is the only way to get
  "tap = jump to the previous window" and a stable Alt+Tab-like order. The
  frozen list is then built from it.

### 3. A session state machine

```
Idle ──trigger──▶ Active { list, index } ──release──▶ Commit(index) ──▶ Idle
                      │  step(±1)  moves index           (activate that window)
                      └──Escape / click-outside──▶ Cancel ──▶ Idle
```

Everything mode-specific is a *view* of `Active` plus a *commit* action. The
state lives in the `thread_local!` next to the hook state, never behind a
lock (project rule).

### 4. Committing: activating the chosen window

Done on the message loop, never inside the hook:

1. `IsIconic` → `ShowWindow(SW_RESTORE)`.
2. `SetForegroundWindow`. It works when this process received the last input
   event, which a hotkey/hook-driven session normally satisfies. Fallback if
   Windows refuses: the usual `AttachThreadInput` dance with the current
   foreground thread, or a synthetic Alt tap (`modifier_guard` already knows
   the filler-key rules for that).
3. Verify with `GetForegroundWindow` and report a failure in the log/balloon
   instead of silently doing nothing — that silence is what made the old
   switcher look broken.

### 5. Hook-layer prerequisites (the "mouse wheel features")

What the low-level hook must provide before any mode can be built:

* **Wheel** — restore `WM_MOUSEWHEEL` (signed delta in the high word of
  `mouseData`) and add **`WM_MOUSEHWHEEL`** (tilt / horizontal wheel), which
  the old code never saw. Both only *post* a private `WM_APP` message
  (`WM_SWITCH_STEP`, `wParam` = ±notches) and return "swallowed" while a
  session is active or the trigger matches. No enumeration, no drawing in the
  hook (300 ms rule).
* **Accumulate notches** — high-resolution wheels send deltas smaller than
  120. Sum until ±120 before turning it into a step.
* **XBUTTON1/2** (`WM_XBUTTONDOWN/UP`) — side buttons are a natural trigger
  and a cheap one; the hook does not handle them today.
* **Session end detection** — "commit on release of Win/Alt". Two options:
  * reuse `modifier_guard`'s `WH_KEYBOARD_LL` hook (sees the real key-up
    immediately), or
  * poll `GetAsyncKeyState` from a ~30 ms `WM_TIMER` on the message loop.
    Simpler, can't be starved by a hook timeout, and immune to the
    "key-up lost when focus moves to an elevated window" problem described in
    `y_core::hotkey`. Costs up to 30 ms of latency on commit. **Recommended**,
    with the keyboard hook only as an optimisation if it ever feels laggy.
* **Swallowing the modifier** — `Win`+wheel then releasing `Win` would open
  the Start menu, `Alt` alone would focus the menu bar. `modifier_guard::arm`
  /`absorb_modifier_tap` already solves exactly this for drag/resize; the
  switcher session arms it the same way.
* **Keyboard while active** — arrows / Tab / Shift+Tab / Enter / Escape need
  to reach the session, so the overlay is either focus-taking (simple, but
  steals focus from the app you are leaving, which breaks "restore previous
  focus on cancel" unless remembered) or non-activating with those keys
  intercepted by the keyboard hook while a session is active. Non-activating
  matches the existing overlays; recommended.

## The modes

Ordered from least to most work; each builds on the previous.

### A. Quick cycle (no UI)

Trigger+wheel steps through the frozen list and **activates each window as you
go**, like the old feature but without the ping-pong. Flashes windows, but has
no overlay to build. Tap with no step could mean "go to previous window"
(needs MRU).

* Good: minimal code — blocks 1–4 and the wheel plumbing, nothing else.
* Bad: windows visibly shuffle; the final z-order is changed by every
  intermediate step; commit-on-release is not needed, so the "release" part of
  the session is just cleanup.

### B. Cycle with preview outline (commit on release)

Same as A, but stepping only moves a **highlight border** around the candidate
(the action overlay's `focus_border` already draws this); the actual
activation happens once, on release. No list.

* Good: no flashing, nothing reordered unless you commit, cancellable.
* Bad: with no list you cannot see what is coming; a window hidden behind
  others only gets an outline, and a minimized one has no rect to outline
  (show its taskbar button position, or just skip the outline for it).

### C. Simple list overlay — "Win+wheel, release to switch"

What was asked first. A centred (on the pointer's monitor) layered overlay,
`y_core::overlay::Canvas` like the others:

```
┌──────────────────────────────────┐
│ 🌐  Firefox — Rust docs          │
│ ▶ 🗒  Notepad — notes.txt        │  ◀ selected
│ 💻  Windows Terminal             │
└──────────────────────────────────┘
```

* Entries: icon (the code path `min_overlay`/`icon` work already has), title,
  optionally the process name; minimized ones dimmed.
* Wheel / ↑ ↓ / Tab move the selection; release of the trigger modifier
  commits; Escape or a click outside cancels; a click on an entry commits it
  immediately.
* Layout options (config): vertical list, horizontal strip of icons with the
  title of the selected one underneath (Alt+Tab style), or a wrapped grid when
  there are many windows.
* Extra actions on the selection, cheap once the list exists: middle-click or
  `Delete` closes that window; `M` minimizes it.
* Reuses `ActionOverlay`'s drawing/theme config (`<overlay>` palette) so it
  does not need its own colour settings, or gets a small `<switcher>` palette
  override.

### D. Desktop-aware overlay

Same overlay, but **windows are grouped by virtual desktop** and the selection
can cross desktops:

```
 Desktop 1        Desktop 2 ◀            Desktop 3
 ─────────        ────────────           ─────────
 Firefox          ▶ Terminal             (empty)
 Notepad            VS Code
```

* Wheel moves inside the current desktop's column; a second axis
  (Shift+wheel, tilt wheel, or ← →) moves between desktops.
* Commit on a window of another desktop = **switch to that desktop, then
  activate the window**. `IVirtualDesktopManager::GetWindowDesktopId` (public)
  gives each window's desktop GUID, and the internal manager already in
  `y_core::virtual_desktop` gives the ordered desktop list; the missing piece
  is a `switch_to(index)` there (it only has `Next`/`Prev` today) and
  "enumerate all desktops' windows" (the filter in block 1 is dropped, the
  cloaked check is replaced by "cloaked *by the shell for being on another
  desktop*", i.e. `DWM_CLOAKED_SHELL`).
* Desktop-level actions on the selected column: new desktop, move the selected
  window to another desktop (`M` + ← →), delete an empty desktop. These map
  one-to-one onto functions `y_core::virtual_desktop` already exports
  (`create_desktop`, `move_window_to_desktop`).
* A **desktops only** variant — a strip of desktop tiles, no windows — is the
  same view with the window rows removed, and replaces `desktop-next/prev`
  with something you can see.
* Degradation: on a Windows build where `y_core::virtual_desktop::acquire`
  finds no known interface, the overlay falls back to mode C (single desktop,
  public-API filter only) and says so, as the plan for the `desktop-*`
  actions already prescribes.

### E. Thumbnail grid (stretch)

Task-View-like grid with live previews via `DwmRegisterThumbnail`. Large
payoff, large cost: DWM thumbnails do not composite through an
`UpdateLayeredWindow` bitmap surface, so this mode needs a different overlay
window type than everything else in the workspace. Listed so it is a conscious
"not now", not an oversight.

## Triggers

Independent of the mode; each can be configured per mode.

| Trigger | Notes |
| --- | --- |
| `modifier` + wheel | The request. `Win`, `Alt`, `Ctrl`+`Alt`… Session lives while the modifier is held. |
| `modifier` + tilt wheel | Frees the plain wheel for stepping desktop-axis in mode D. |
| Mouse side button (XBUTTON1/2) | Press to open, wheel to step, release to commit. Needs the new hook case. |
| Wheel over the taskbar / screen edge | No modifier at all: scroll on the taskbar to cycle. Needs the taskbar rect and a "hover region" check done in the hook (geometric, no cross-process call). |
| Hotkey (`Alt+Tab` replacement) | `y_core::hotkey` `steal` could take `Alt+Tab` from the shell; riskier (the shell owns it), but it is the only way to get a keyboard-only session. |
| Tap | Press-and-release with no step jumps to the **previous** window (needs MRU). The cleanest anti-ping-pong feature of all: two taps toggle between two windows by definition. |

## Further ideas

* **Same-application cycling** — a variant of the filter: only windows of the
  foreground window's process (Alt+`). One filter predicate, no new UI.
* **Current monitor only** — filter by the pointer's or foreground window's
  monitor (`y_core::monitor` already exists).
* **Type to filter** in the list overlay (title / process substring).
* **Per-desktop MRU** so each desktop remembers its own last window.
* **Exclusion lists** in config (class names / process names) beyond
  `BANISHED`, for windows the user never wants offered.
* **Include or skip minimized windows**, as a setting.
* **Preview-on-hover**: hovering an entry peeks the window (`DwmpActivateLivePreview`
  is undocumented — skip) or outlines it (mode B's border).

## Suggested order

1. Hook-layer prerequisites (§5): wheel + tilt + XBUTTON posting private
   messages, notch accumulation, session-end polling. Testable alone with a
   debug log of the messages.
2. Blocks 1–4: the frozen candidate list, the session state machine, and the
   commit with verification. At this point **mode A works** and the ping-pong
   is gone.
3. Mode C's overlay (and mode B's outline falls out of the same view code if
   wanted as a "no list" option).
4. MRU tracking + the tap-to-previous trigger.
5. Mode D, after `switch_to(index)` and per-window desktop lookup are added to
   `y_core::virtual_desktop`.
6. Config: a `<switcher>` section and a dedicated screen in `y-snap-config`
   (mode, trigger, order, filters, layout). Schema stays clean-slate, with
   the usual severity split (malformed document aborts, unknown attribute is
   a warning).

## Decisions needed before coding

1. **Which modes ship?** My suggestion: C first (it contains A's machinery),
   then D; B only if you want a list-free option; E not now.
2. **One mode at a time in config, or several bindable at once?** (e.g. `Win`+wheel
   = list, `Alt`+wheel = cycle). Several triggers means several sessions'
   worth of config but the same code.
3. **Z-order or true MRU** for the list order (MRU also unlocks "tap = previous
   window").
4. **Overlay focus model:** non-activating with keys intercepted by the
   keyboard hook (my recommendation, consistent with the other overlays) or a
   focus-taking window.
5. **Session end:** timer polling of the modifier (recommended) vs. the
   keyboard hook.
6. **Where the desktop work lives:** extend `y_core::virtual_desktop`
   (`switch_to`, per-window desktop id) — the obvious place, since other apps
   may want it.
