# PowerToys GrabAndMove's hook architecture — notes for y-snap

Source analysed: [`main.cpp`](https://raw.githubusercontent.com/microsoft/PowerToys/refs/heads/main/src/modules/GrabAndMove/GrabAndMove/main.cpp)
from [microsoft/PowerToys](https://github.com/microsoft/PowerToys) (MIT).
~1960 lines; this covers the hook procedures, the modifier-absorb state
machine, and the deferred-activation logic — not the overlay rendering,
settings plumbing, or telemetry, which are out of scope for y-snap's own
work. Companion to [`altsnap-hooks-analysis.md`](altsnap-hooks-analysis.md)
in this directory — read that one first; this one assumes it.

The short version: PowerToys does not try to *disguise* the modifier's
release the way AltSnap and HulK do. It **swallows the modifier's key-down
outright, the instant it happens**, before anything else can react to it —
then, once it's known whether that press actually drove a drag/resize,
either throws the swallowed press away for good (a real gesture happened;
the shell never saw *any* part of the modifier) or **replays a synthetic
down+up pair** so everything downstream sees a normal, ordinary tap (nothing
happened; give the modifier back). Either way, nothing the shell's own
Alt/Win-tap detection watches for ever occurs, because the real down never
reached it in the first place. This is the "hard way, with precision" the
task's original ask called it — no injected filler key, no timing bet on
catching the real key-up in time; the ambiguity is resolved by not letting
the shell see the ambiguous event at all.

## 1. Both hooks are permanent, and there is a third one

```cpp
g_hhkKeyboard = SetWindowsHookExW(WH_KEYBOARD_LL, KeyboardProc, hInstance, 0);
g_hhkMouse = SetWindowsHookExW(WH_MOUSE_LL, MouseProc, hInstance, 0);

// Detect foreground switches to elevated processes (where key-up events stop arriving)
g_hWinEventHook = SetWinEventHook(
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
    nullptr, WinEventProc, 0, 0, WINEVENT_OUTOFCONTEXT);
```

Both `WH_KEYBOARD_LL` and `WH_MOUSE_LL` are installed once, for the whole
process lifetime — unlike AltSnap's mouse-hook-only-while-modifier-held
design, and unlike y-snap's current keyboard-hook-only-for-stolen-hotkeys
design. `KeyboardProc` is not "minimalist" the way AltSnap's is either: it
unconditionally intercepts every Alt/Win press on the system the moment
GrabAndMove is running, whether or not the user is about to drag anything.

The third hook, `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ...)`, is not
about mouse or keyboard input at all — it watches for foreground-window
changes, specifically to catch a failure mode the comment names outright:

```cpp
// WinEvent hook – detects foreground switch to elevated processes where
// low-level keyboard hooks stop delivering key-up events.
```

More on this in §4 — it's the most directly reusable idea here, independent
of which absorb strategy y-snap ends up choosing.

## 2. Absorb-on-keydown, not disguise-on-keyup

`KeyboardProc`'s Alt branch (the Win branch is the mirror image):

```cpp
if (wParam == WM_KEYDOWN || wParam == WM_SYSKEYDOWN)
{
    if (IsSuppressedByGameMode())
    {
        goto forward;
    }
    // If any other modifier is already held (e.g. Ctrl in Ctrl+Alt+Del),
    // or any regular key is currently held (e.g. Q held before Alt),
    // don't intercept this Alt press – let it reach the target application.
    if ((GetAsyncKeyState(VK_CONTROL) & 0x8000) ||
        (GetAsyncKeyState(VK_SHIFT) & 0x8000) ||
        (GetAsyncKeyState(VK_LWIN) & 0x8000) ||
        (GetAsyncKeyState(VK_RWIN) & 0x8000) ||
        g_heldNonAltKeyCount > 0)
    {
        goto forward;
    }
    g_altPressed = true;
    if (g_shouldAbsorbAlt)
    {
        if (!g_altAbsorbed)
        {
            g_altAbsorbed = true;
            g_dragConsumedAlt = false;
            g_pendingModifierReplayed = false;
            g_absorbedVk = kb->vkCode;
            g_absorbedScanCode = kb->scanCode;
            g_absorbedFlags = kb->flags;
        }
        return 1; // swallow the Alt keydown
    }
}
```

Every plain Alt press (no other modifier or key already down, not
suppressed by game mode) is swallowed outright — `return 1`, before the
application under the cursor, or anywhere else, ever sees it — and its
identity (`vkCode`, `scanCode`, `flags`) is stashed for later. Nothing
about a drag has happened yet; this fires purely off the key event itself,
independent of the mouse.

`ReplayAbsorbedModifier` is what turns that stash back into a real
key event when the absorbed press turns out not to have been used:

```cpp
static void ReplayAbsorbedModifier(bool alsoKeyUp)
{
    INPUT inputs[2] = {};
    inputs[0].type = INPUT_KEYBOARD;
    inputs[0].ki.wVk = static_cast<WORD>(g_absorbedVk);
    inputs[0].ki.wScan = static_cast<WORD>(g_absorbedScanCode);
    inputs[0].ki.dwFlags = (g_absorbedFlags & LLKHF_EXTENDED) ? KEYEVENTF_EXTENDEDKEY : 0;
    inputs[1] = inputs[0];
    inputs[1].ki.dwFlags |= KEYEVENTF_KEYUP;
    SendInput(alsoKeyUp ? 2 : 1, inputs, sizeof(INPUT));
}
```

Note this replays *the modifier itself* — the real `vkCode`/`scanCode`/
extended-flag it captured off the original press — not a filler key like
Right Shift or Ctrl. It isn't trying to interrupt a tap; it's reconstructing
the exact keystroke it took away, indistinguishable from the original
except for being a few milliseconds late.

## 3. Deciding what happened, at key-up

`KeyboardProc`'s Alt key-up branch does the actual bookkeeping:

```cpp
else if (wParam == WM_KEYUP || wParam == WM_SYSKEYUP)
{
    g_altPressed = false;
    g_ignoreModifier = false;
    g_activatedDuringHold = false;
    // If a press was still pending (button held, no move), deliver it now.
    FlushPendingClickOnModifierRelease();
    bool wasDragging = g_dragging;
    bool wasResizing = g_resizing;
    EndInteraction(true, true);
    if (g_altAbsorbed)
    {
        g_altAbsorbed = false;
        if (wasDragging || wasResizing || g_dragConsumedAlt)
        {
            // Alt was used for a drag/resize; swallow the keyup too
            g_dragConsumedAlt = false;
            return 1;
        }
        if (g_pendingModifierReplayed)
        {
            // The key-down was already replayed for a pending click; let
            // the real key-up reach the target.
            g_pendingModifierReplayed = false;
        }
        else
        {
            // No drag happened; replay the keydown, then let keyup through
            ReplayAbsorbedModifier(false);
        }
    }
}
```

Three outcomes, gated by state set elsewhere:

| Condition at key-up | What happens | Why |
| --- | --- | --- |
| `wasDragging \|\| wasResizing \|\| g_dragConsumedAlt` | Swallow the real key-up too. Nothing replayed. | The absorbed press was spent on a real gesture — the shell should never learn Alt was pressed at all, so there's no tap for it to mis-detect. |
| absorbed, no gesture, click not yet flushed | `ReplayAbsorbedModifier(false)` (down only), then fall through and let the *real* key-up pass. | Nothing happened with the modifier; give it back as an ordinary press-then-release, indistinguishable from the user never having been intercepted. |
| absorbed, no gesture, `g_pendingModifierReplayed` already true | Do nothing here; let the real key-up through. | The down half was already replayed earlier — by `FlushPendingClickOnModifierRelease` (§4) — so only the up is still owed. |

`g_dragConsumedAlt` (set `true` in `BeginDrag`/`BeginResize`, i.e. the
moment `on_button_down`'s equivalent actually activates a gesture) is doing
almost exactly the job AltSnap's `state.blockaltup` does — "was this
modifier press actually spent on a drag" — just consulted at a different
point in a differently-shaped state machine.

## 4. Deferred activation: a modifier+click is not a drag until it moves

This is the piece with no equivalent in AltSnap, HulK, or y-snap today, and
it's what the rest of the "pending" state (`g_pendingDrag`,
`g_pendingResize`, `g_pendingTarget`, `g_pendingDownPt`,
`g_swallowNextLButtonUp`/`g_swallowNextRButtonUp`) exists for.

`MouseProc`, on modifier+`WM_LBUTTONDOWN`:

```cpp
// Already committed to Grab and Move this hold: start dragging immediately.
if (g_activatedDuringHold)
{
    BeginDrag(hwnd, pt);
    return 1; // swallow the press; the drag is now active
}

// Defer: absorb the button-down but do nothing until the cursor moves.
g_pendingDrag = true;
g_pendingResize = false;
g_pendingTarget = hwnd;
g_pendingDownPt = pt;
return 1; // swallow the press for now; replayed on release if no move
```

The button-down is swallowed immediately, but *nothing else happens yet* —
no window is moved, no gesture is "active." Only once `WM_MOUSEMOVE`
reports the pointer has actually travelled past the system's own drag
threshold does the pending press become a real drag:

```cpp
if (wParam == WM_MOUSEMOVE && (g_pendingDrag || g_pendingResize))
{
    int ddx = ms->pt.x - g_pendingDownPt.x;
    int ddy = ms->pt.y - g_pendingDownPt.y;
    if (ddx < 0) ddx = -ddx;
    if (ddy < 0) ddy = -ddy;
    if (ddx >= GetSystemMetrics(SM_CXDRAG) || ddy >= GetSystemMetrics(SM_CYDRAG))
    {
        ...
        BeginDrag(target, downPt);
        HandleDragMove(ms->pt);
    }
    return 0; // keep the cursor moving while waiting for the threshold
}
```

`SM_CXDRAG`/`SM_CYDRAG` are the same system metrics Windows itself uses to
decide "was that a click or the start of a drag" — reused here rather than
picking an arbitrary pixel count.

If the button instead comes back up *before* the threshold is crossed —
an ordinary Alt+click, not a drag — the swallowed press is turned into a
real click and sent to the target, and the matching physical button-up
(which is still coming) is pre-marked to be swallowed so the target doesn't
see a doubled release:

```cpp
if (wParam == WM_LBUTTONUP && g_pendingDrag)
{
    g_ignoreModifier = true;
    g_pendingDrag = false;
    g_pendingTarget = nullptr;
    ReplayPendingClick(false);
    return 1; // swallow the physical up; the injected click replaces it
}
```

```cpp
static void ReplayPendingClick(bool isRight)
{
    if ((g_altAbsorbed || g_winAbsorbed) && !g_pendingModifierReplayed)
    {
        g_pendingModifierReplayed = true;
        ReplayAbsorbedModifier(false); // modifier down only
    }
    INPUT inputs[2] = {};
    inputs[0].type = INPUT_MOUSE;
    inputs[0].mi.dwFlags = isRight ? MOUSEEVENTF_RIGHTDOWN : MOUSEEVENTF_LEFTDOWN;
    inputs[1].type = INPUT_MOUSE;
    inputs[1].mi.dwFlags = isRight ? MOUSEEVENTF_RIGHTUP : MOUSEEVENTF_LEFTUP;
    SendInput(2, inputs, sizeof(INPUT));
}
```

The comment on `ReplayPendingClick` spells out the ordering contract this
depends on: the modifier's key-down is replayed here, but its key-up is
*not* — that's left for the real one to arrive later and pass through
normally (§3's "click not yet flushed" row), specifically so a user holding
Alt and clicking repeatedly without ever releasing Alt gets one replayed
down followed by real, un-intercepted clicks, not a replayed down/up pair
per click.

There's also a same-hold fast path: once one gesture has actually activated
during a modifier hold, `g_activatedDuringHold` skips the pending/threshold
dance for any further press during that same hold — click, drag, click,
drag, all with one Alt press, is not made to re-litigate "was that a click"
every time.

## 5. Recovery and edge cases worth lifting independent of the absorb strategy

* **Injected-input recognition uses the standard flags, not a private
  signature:**

  ```cpp
  if (kb->flags & LLKHF_INJECTED)
      goto forward;
  ...
  if (ms->flags & LLMHF_INJECTED)
      goto forward;
  ```

  Where HulK, AltSnap, and y-snap's `core::hotkey` all tag their own
  injected events with a private `dwExtraInfo` value and match on that
  specifically (deliberately, per `y_core::hotkey`'s own comment, so
  legitimate injected input from *other* tools — on-screen keyboards, remote
  desktop, accessibility software — isn't mistaken for the hook's own
  feedback and ignored), PowerToys takes the coarser `LLKHF_INJECTED`/
  `LLMHF_INJECTED` route: skip *all* injected input, not just its own. Worth
  keeping y-snap's private-signature approach rather than switching — the
  reasoning in `y_core::hotkey`'s doc comment for why still applies — but
  worth knowing PowerToys made the opposite trade explicitly, not by
  oversight.

* **The elevated-process key-up problem gets its own hook, not a workaround
  bolted onto the keyboard hook:**

  ```cpp
  static void CALLBACK WinEventProc(HWINEVENTHOOK, DWORD, HWND hwnd, LONG, LONG, DWORD, DWORD)
  {
      if (hwnd == g_hOverlay || hwnd == g_hMsgWnd)
          return;

      g_heldNonAltKeyCount = 0;
      memset(g_keyHeld, 0, sizeof(g_keyHeld));
      g_excludedCache.clear();

      if (!g_altPressed && !g_winPressed && !g_altAbsorbed && !g_winAbsorbed && !g_dragging && !g_resizing)
          return;

      bool modActuallyHeld = (g_modifierKey == GrabAndMoveModifier::Win)
          ? ((GetAsyncKeyState(VK_LWIN) | GetAsyncKeyState(VK_RWIN)) & 0x8000) != 0
          : ((GetAsyncKeyState(VK_LMENU) | GetAsyncKeyState(VK_RMENU)) & 0x8000) != 0;

      if (!modActuallyHeld)
      {
          g_altPressed = false;
          g_winPressed = false;
          g_winAbsorbed = false;
          g_altAbsorbed = false;
          g_dragConsumedAlt = false;
          g_ignoreModifier = false;
          g_activatedDuringHold = false;
          g_pendingDrag = false;
          g_pendingResize = false;
          g_pendingTarget = nullptr;
          g_swallowNextLButtonUp = false;
          g_swallowNextRButtonUp = false;
          g_pendingModifierReplayed = false;
          EndInteraction(true, true);
      }
  }
  ```

  This is the production answer to a risk y-snap's own hotkey module
  already documents in prose (`core/src/hotkey.rs`: *"a process without
  UIAccess cannot hook keystrokes destined for an elevated application"*)
  but has no runtime handling for. `WH_KEYBOARD_LL` reliably delivers a
  key-*down* that switches focus to an elevated window, but if the key is
  *released* while that elevated window has focus, the matching key-up can
  simply never reach a non-elevated hook — exactly the shape of bug that
  would leave y-snap's own absorb state (or a future `modifier_guard`-style
  "armed" flag) stuck forever, waiting for a key-up that isn't coming.
  `EVENT_SYSTEM_FOREGROUND` doesn't depend on the keyboard hook at all —
  it's a separate accessibility-style event PowerToys uses purely as a
  trip-wire: *any* foreground change is a moment to sanity-check tracked
  state (`g_altPressed` etc.) against ground truth (`GetAsyncKeyState`,
  which reflects real hardware state regardless of what messages did or
  didn't arrive) and forcibly reset if they disagree.

* **`g_heldNonAltKeyCount` / `g_keyHeld[256]`** track every currently-held
  non-modifier key precisely so a fresh Alt/Win press while, say, a letter
  is already held down doesn't get absorbed at all (`goto forward`) — the
  user is mid-hotkey or mid-typing, not initiating a grab. Comment:
  *"KBDLLHOOKSTRUCT::flags does not carry the KF_REPEAT bit"*, which is why
  it's tracked per-vkey rather than read off the event itself.

* **Both hooks bail per-event on an excluded foreground window
  (`IsExcluded`) and on detected fullscreen games (`IsSuppressedByGameMode`)
  before doing anything else** — not directly about the modifier-tap
  problem, but a reminder that a hook this aggressive (swallowing every
  plain Alt/Win press system-wide) needs an escape hatch for the
  applications it shouldn't touch at all, which none of y-snap, HulK, or
  AltSnap currently have as a first-class, user-configurable list.

## 6. What this suggests for y-snap, next to the AltSnap notes

The two designs are not really in competition; they're different points on
a robustness/complexity trade, and the a[ltsnap-hooks-analysis.md](altsnap-hooks-analysis.md)
recommendation (arm at drag start, install a scoped keyboard hook, fire the
absorb at the real key-up, never swallow the key-up itself) is still the
right *next* step — it fixes the timing gap with the least new state and
without a persistent keyboard hook. This document's job is to say what the
*further* step looks like if that isn't enough:

1. **If timing-the-disguise-right still isn't reliable enough, absorb-on-
   keydown is the fallback with no timing dependency at all.** It trades a
   persistent, always-on keyboard hook (every Alt/Win press on the system
   gets intercepted, not just ones that turn into a drag) for removing the
   race entirely: nothing is guessed, nothing needs to land in the few
   milliseconds before a real key-up; the real down is simply never let
   through until it's known whether it was needed.

2. **A deferred/pending-activation model is worth doing on its own merits**,
   independent of the modifier question: right now a plain `mod="none"`
   drag gesture in y-snap starts the instant the button goes down, with no
   distinction from "the user just clicked." Gating activation on the
   system's own `SM_CXDRAG`/`SM_CYDRAG` threshold — and replaying a
   swallowed press as an ordinary click if the threshold is never
   crossed — would make an accidental micro-drag on every configured click
   stop being possible, which is a real, separate usability gap y-snap has
   today regardless of Alt/Win.

3. **Adopt the `WinEventHook` recovery pattern regardless of which absorb
   strategy is chosen.** This is the one piece of this document that isn't
   really optional: any design that tracks "is a modifier currently
   absorbed/armed" as state living across a key-down and a later key-up is
   exposed to the elevated-focus gap in §5, and `y-core::hotkey`'s own doc
   comment already flags the risk in prose without addressing it at
   runtime. A single `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ...)`,
   reconciling tracked state against `GetAsyncKeyState` on every foreground
   switch, is cheap and belongs in `y_core` itself (alongside
   `absorb_modifier_tap`) rather than duplicated per application.

4. **Track held non-modifier keys before absorbing anything**, the way
   `g_heldNonAltKeyCount` does, if y-snap ever moves to keydown-time
   absorption (recommendation 1). Swallowing Alt while the user is mid
   keystroke on an unrelated hotkey is a correctness bug PowerToys clearly
   hit and fixed; better to build the check in from the start than
   rediscover it.
