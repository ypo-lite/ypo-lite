# Finder patterns: modifier sequences and held keys (plan)

Status: **plan, 2026-11-15. Nothing built.** Extends the finder's `<tap>`
trigger (a modifier tapped N times) to ordered sequences of modifiers and to a
held modifier. Code today: `config/finder.rs` (`Tap`, `TapKey`, `Side`),
`finder/detect.rs` (`Taps`), `finder/input.rs` (keyboard hook),
`y-snap-config/src/finder.rs` (the trigger cards).

## Goal

Recognise, as automatic finder triggers:

- `ctrl, shift, ctrl, shift`, "more or less fast";
- `win` held while `ctrl, ctrl` is tapped;
- any other short sequence of modifiers, optionally with held modifiers.

## Model

A **pattern** replaces `Tap`'s `key` + `count`:

- `steps`: 1 to `MAX_STEPS` (6) of `(TapKey, Side)`, in order.
- `held`: a set of modifiers that must be down before the first step and stay
  down until the last release (empty by default).
- `max_hold_ms`, `max_gap_ms`: as today (200 / 400).

The old `key` + `taps` is sugar for `steps` = the key repeated `taps` times.

## Recognition (`finder/detect.rs`)

Same philosophy as `Taps`: pure logic, fixed size, nothing allocated in the
hook, wrapping millisecond arithmetic.

- Per pattern, a chain: step index reached, key currently down and since when,
  spoiled flag, time of the last step's press.
- A step matches when the released modifier equals `steps[index]` (side
  honoured) and was held no longer than `max_hold_ms`; the next press must come
  within `max_gap_ms` of the previous press.
- A **different** modifier restarts the pattern; if it equals `steps[0]` it
  counts as step 1 (so `ctrl ctrl shift` is not lost to a stray `ctrl`).
- Any non-modifier key or mouse button resets (as today: `Ctrl+C` is no tap).
- **Held set:** modifiers in `held` are exempt from "another key spoils", but
  must be down when step 1 goes down and still down at the final release; if
  one is released early, the chain resets. Other held modifiers spoil.
- Fires on the final release, so a key held down can never fire.
- Wrapped in the same `STALE_MS` guard for a lost release.

## Configuration (`config/finder.rs`, `y-snap.xml`)

```xml
<tap key="ctrl" taps="2"/>                          <!-- unchanged -->
<tap keys="ctrl shift ctrl shift" hold="200" gap="400"/>
<tap keys="ctrl ctrl" held="win"/>
<tap keys="lctrl rshift"/>                          <!-- side per step -->
```

- Step names: `ctrl shift alt win`, optionally prefixed `l`/`r`; the existing
  aliases (`control`, `windows`, `super`, `meta`) keep working.
- `key`/`taps` and `keys` are mutually exclusive; both present is a warning
  and `keys` wins.
- Severity split as everywhere: a malformed document aborts, anything
  recognised-but-wrong is a warning and the rest loads. Warnings: unknown
  step, more than `MAX_STEPS`, a held modifier that is also a step, empty
  pattern, **a pattern that is a prefix of another** (the shorter one would
  always fire first), duplicates.
- Writing back (`write_finder`) emits the short form when the pattern is one
  key repeated, `keys` otherwise.

## Settings app (`y-snap-config/src/finder.rs`)

The `<tap>` card becomes a pattern builder (built from `ui-kit`; read
`ui-kit/docs/ui-toolkit.md` first):

- A row of **key chips**: "+" opens a menu (Ctrl, Shift, Alt, Win × Any, Left,
  Right); delete and reorder per chip.
- **Keep held**: four toggles.
- **Speed**: Fast / Normal / Relaxed / Custom (the two numbers).
- **Record pattern**: the user performs it, the chips fill in and the speed is
  inferred from their rhythm (limits: Win is intercepted by the shell, so
  recording it is unreliable; fall back to the "+" menu).
- Inline **warnings**: prefix conflict, held-equals-step, and the layout-switch
  one below.

## Risks, to settle by live checks

1. **Input-language hotkey.** Ctrl+Shift and Alt+Shift switch layouts on a
   machine with several. A pattern alternating those keys may trigger it.
   Check whether it can be prevented from the hook; otherwise warn.
2. **Start menu.** A lone `Win` release opens Start. With `win` held and
   `ctrl` tapped it should not, but must be tested; `y_core::modifier_guard`
   is the fallback.
3. **False positives** of long sequences in everyday typing; tune the default
   hold/gap on a real keyboard.
4. Keyboard hook cost: the same call per key as today, a slightly longer scan.

## Phases

0. **Decisions** (open questions below).
1. **Model and parser**: `Pattern` type, `keys`/`held` parsing, `key`+`taps`
   sugar, write-back, warnings; unit tests incl. round trip.
2. **Recognition**: new `Taps`, tests with scripted event sequences (fast, too
   slow, interrupted by a key, wrong modifier restart, held set kept and
   released early, prefix case).
3. **Hook wiring**: `finder::input` unchanged in shape; check `STALE_MS`, the
   held-set bookkeeping and `other()` still behave.
4. **Settings app**: chips, held toggles, speed presets, warnings.
5. **Record mode** in the settings app.
6. **Docs**: user manual, configuration reference, README, development notes
   (each feature updates all four).
7. **Live checks** (risks 1 to 3), run by the user.

## Open questions

1. Allow `held` on any modifier, or only Win/Alt/Ctrl/Shift as listed (all
   four)? (Proposed: all four.)
2. Maximum steps: 6? (Proposed.)
3. Prefix conflicts: warn and keep both, or refuse the shorter? (Proposed:
   warn, keep both, the shorter fires.)
4. Is Record mode worth it in the first delivery, or after phase 4?
   (Proposed: after.)
5. Should sequences allow non-modifier keys (e.g. `ctrl, F`)? (Proposed: no;
   keeps the hook cheap and avoids shadowing real shortcuts.)
