# Alt/Win modifier swallowing — implementation plan

Synthesizes [`altsnap-hooks-analysis.md`](altsnap-hooks-analysis.md) and
[`powertoys-grabandmove-analysis.md`](powertoys-grabandmove-analysis.md)
into concrete next steps for y-snap. Read those two first — this is the
"now do this" that follows from them, not a third independent analysis.

## Where this stands right now

`core/src/hotkey.rs::absorb_modifier_tap` and its call sites in
`y-snap/src/mouse/gesture.rs` are **mid-experiment**, not a finished design:
`absorb_modifier_tap` currently injects `Ctrl-down, [modifier]-up,
Ctrl-up`, called from `Gesture::on_button_down`/`on_button_up` — i.e. still
at *mouse*-event time, the thing both analysis docs point at as the actual
gap, not the filler key or its flags. `gesture.rs` also still posts
`mouse::WM_ABSORB_MODIFIER` to run a second, held-duration variant
(`absorb_modifier_tap_held`) plus a `type_test_string("ypo")` sanity check
from the ordinary message loop — scaffolding for comparing "inject from
inside the hook" against "inject from the message loop," explicitly marked
temporary in its own doc comments.

None of that is wrong to have tried — it's how the two real findings below
got found. But it's not the design to build on; the phases below replace it
rather than extend it.

## The two things that actually matter, restated

1. **Timing, not the filler key.** AltSnap and HulK use different filler
   keys with different flags and both work; y-snap tried several
   combinations of both and none reliably did. The one thing y-snap hasn't
   done that both reference implementations do: fire the correction
   **synchronously inside the keyboard-hook callback that observes the
   modifier's real key-up**, immediately before letting that key-up
   continue down the chain — not at some earlier or unrelated point
   (mouse-button time).
2. **A drag/resize needs a keyboard hook that outlives the mouse gesture.**
   The user can let go of Alt/Win seconds after releasing the mouse button.
   Nothing keyed off `WM_LBUTTONUP`/`WM_RBUTTONUP` can be present at the
   moment that matters. This is *why* (1) hasn't been achievable from
   `gesture.rs` alone so far, not a separate problem.

## Phase 1 — scoped keyboard hook, fire at the real key-up

The core change. Small, and the direct answer to the user's own framing of
this task ("hook/unhook the keyboard while start drag/resize, so we can
simulate the keyup on the alt/win key and catch the real keyup later").

**Shape:** revive the `modifier_guard`-style module that was tried and
reverted, but fix the actual mistake in that attempt — it swallowed the
real key-up outright and stayed permanently armed once mis-tuned, which is
what broke system-wide Alt/Win. This version does neither:

* `y-snap/src/mouse/modifier_guard.rs` (new): owns a `WH_KEYBOARD_LL` hook
  installed **only while at least one Alt/Win-modified gesture is currently
  in progress** — armed in `Gesture::on_button_down`'s activation branches
  (where `notify_absorb`/`absorb_if_modified` are called today), torn down
  once resolved. Not installed for the process's whole lifetime the way
  PowerToys' is (see Phase 3 for when that trade becomes worth it).
* The hook's *only* job: on `WM_SYSKEYUP`/`WM_KEYUP` for the armed
  modifier family (`VK_LMENU`/`VK_RMENU`/`VK_MENU` for Alt,
  `VK_LWIN`/`VK_RWIN` for Win — remember `KBDLLHOOKSTRUCT::vkCode` reports
  the side-specific code, the bug the last attempt shipped with briefly),
  call `absorb_modifier_tap()` **and then let the real key-up continue**
  (`CallNextHookEx`, no `return 1`). This is the AltSnap `HotkeyUp()` shape
  exactly: the filler tap precedes the real release in the same callback;
  the real release is never itself swallowed, so nothing downstream is left
  thinking Alt/Win is still held.
* `absorb_modifier_tap` goes back to doing one thing: inject a filler tap
  for whichever modifier is passed in, called from exactly one place (the
  new hook, at the real key-up). Whether the filler is Right Shift
  (HulK), Ctrl (AltSnap), or the modifier's own down+up replayed
  (PowerToys) is now genuinely a one-line choice with a known-working
  reference for each — pick one, and stop trying to combine them (the
  current `Ctrl, [modifier]-up, Ctrl` sandwich in the experimental code is
  worth reverting to a single well-understood choice before layering
  anything else on top of it).
* Delete the `WM_ABSORB_MODIFIER` message, its handler in `main.rs`, and
  `absorb_modifier_tap_held`/`type_test_string` in `core/src/hotkey.rs` —
  they did their job establishing that timing was the missing variable;
  keeping them past that point is just two more places for the real fix to
  drift out of sync with.

**Explicitly not doing (yet):** PowerToys' absorb-on-keydown model. That
needs the keyboard hook to be permanent and to intercept *every* plain
Alt/Win press on the system, which is a materially bigger commitment and
was the direct cause of last round's regression when tried without the
rest of PowerToys' surrounding machinery (deferred activation, held-key
tracking, the `WinEventHook` recovery net). It's the documented fallback if
Phase 1 turns out not to be reliable enough in practice — see "If Phase 1
isn't enough" below — not the default plan.

## Phase 2 — the `WinEventHook` recovery net

Independent of Phase 1's design, and worth doing regardless: port
PowerToys' `EVENT_SYSTEM_FOREGROUND` watchdog
(`powertoys-grabandmove-analysis.md` §5) into `y_core`, next to
`absorb_modifier_tap`. Any design that tracks "a modifier is currently
armed/absorbed, waiting for its key-up" is exposed to the same failure
`core/src/hotkey.rs`'s own doc comment already names but doesn't handle:
`WH_KEYBOARD_LL` can lose a key-up specifically when focus crosses an
elevation boundary while the key is held. Without this net, that leaves
Phase 1's armed state stuck until the next gesture happens to reuse and
clear it.

* `y_core::hotkey`: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, ...)`,
  installed alongside the existing keyboard-hook machinery.
* On every foreground switch, reconcile whatever "armed" state Phase 1
  exposes against `GetAsyncKeyState` ground truth (which reflects real
  hardware state independent of whether messages arrived) and force-clear
  it on mismatch.
* Cheap: one event per foreground switch, not per keystroke.

## Phase 3 — mouse-hook watchdog (separate bug, same root cause)

Not about Alt/Win at all, but the same underlying fact — Windows silently
drops a low-level hook that misses `LowLevelHooksTimeout` — is what caused
the "drag continues after mouse-up" symptom from earlier in this
investigation. AltSnap's fix
(`altsnap-hooks-analysis.md` §4, `REHOOK_TIMER`) is a five-second polling
timer that reinstalls the mouse hook if the cursor has moved without a
matching `WM_MOUSEMOVE` reaching it.

* `y-snap/src/mouse/mod.rs`: a low-frequency timer alongside `MouseHook`
  that checks `GetCursorPos()` against the last point `on_move` recorded,
  and reinstalls the `WH_MOUSE_LL` hook if they disagree.
* Deliberately not the `GetAsyncKeyState`-per-move self-heal tried and
  reverted earlier — that fired on every mouse move (cheap but a race right
  after activation) and only covered "this gesture's button," not "the hook
  itself is gone." A several-second watchdog checking hook liveness
  directly is both cheaper and covers the actual failure mode.
* No dependency on Phases 1/2 — can land independently, any time.

## Phase 4 — deferred activation (feature-level, optional, do last)

PowerToys' click-vs-drag threshold
(`powertoys-grabandmove-analysis.md` §4) is a real usability gap in
y-snap today, independent of Alt/Win: a configured gesture currently
activates the instant the button goes down, with no way to tell "a click"
from "the start of a drag." Worth doing eventually — gate activation on
`GetSystemMetrics(SM_CXDRAG/SM_CYDRAG)`, replay a swallowed press as an
ordinary click if the threshold is never crossed — but it's a bigger
change to `Gesture`'s state machine than anything above, touches every
gesture (not just Alt/Win-modified ones), and isn't needed to fix the bug
this whole investigation started from. Sequence it after Phases 1–3 are
verified working, as its own piece of work.

## If Phase 1 isn't enough

Escalate to PowerToys' absorb-on-keydown design in full, not piecemeal:

* The keyboard hook becomes permanent (installed whenever any configured
  gesture uses Alt/Win, for the process's run — not scoped to "while a
  gesture is active").
* Every plain Alt/Win keydown is swallowed unconditionally and its
  identity stashed, gated the way PowerToys gates it — no other modifier
  or ordinary key already held (`g_heldNonAltKeyCount` equivalent), not
  suppressed by an exclusion list — *before* any of this is attempted, or
  every keystroke on the machine pays a hook round-trip for a feature only
  relevant during a drag.
* At key-up, replay (nothing was consumed) or drop for good (a gesture
  consumed it) — never disguise-and-let-through.
* This is a bigger, riskier change and should not be attempted without
  Phase 2 already in place — a permanently-armed absorb state is exactly
  the shape of thing the elevated-focus gap can strand.

## Testing checklist (for whichever phase lands)

Manual — none of this is meaningfully unit-testable, it depends on real
Windows input timing:

- [ ] Alt-drag; release the mouse button, *then* release Alt — no menu.
- [ ] Alt-drag; release Alt, *then* release the mouse button — no menu.
- [ ] Win-drag, both orderings above — no Start menu.
- [ ] Alt-resize, both orderings — no menu.
- [ ] Plain Alt tap with no drag at all (not touching the mouse) — Alt
      still works normally (opens/highlights the menu as expected — this
      is the case Phase 1's scoping is supposed to leave alone entirely).
- [ ] Two gestures back-to-back without releasing Alt in between (e.g.
      Alt+drag immediately followed by Alt+drag again) — second one still
      works, no stuck state from the first.
- [ ] Left-click and right-click chorded (one down while the other is
      already held) — neither gets silently dropped (the specific
      regression reported earlier from the keyboard-hook attempt).
- [ ] Drag a window belonging to an elevated process, or switch focus to
      one mid-drag if reachable — confirms Phase 2 actually recovers
      rather than just being untested code.
- [ ] Leave the machine locked/sleeping for a while, then drag — confirms
      Phase 3's watchdog, once it exists.
