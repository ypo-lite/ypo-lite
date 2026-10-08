# AltSnap's hook architecture — notes for y-snap

Source analysed: [`hooks.c`](https://raw.githubusercontent.com/RamonUnch/AltSnap/refs/heads/main/hooks.c)
from [RamonUnch/AltSnap](https://github.com/RamonUnch/AltSnap) (GPLv3, a maintained fork
of Stefan Sundin's AltDrag). ~6800 lines; this covers the parts relevant to
y-snap's own drag/resize and modifier-swallowing work, not the whole feature
set (snapping, per-monitor zones, the action menu, etc. are out of scope
here). Short excerpts are quoted for technical analysis under the same
GPLv3 terms as the original.

The short version, if you read nothing else: **AltSnap's keyboard hook is the
permanent one and its mouse hook is the temporary one — the opposite of
y-snap's current shape** — and it disguises the modifier's release by
injecting a Ctrl tap **synchronously inside the same hook call that sees the
real key-up**, not at some earlier or later point. That timing, not the
choice of filler key, looks like the part y-snap has been missing.

## 1. Two hooks, opposite lifetimes

y-snap today: one `WH_MOUSE_LL` hook, installed for the process's whole
run ([`y-snap/src/mouse/mod.rs`](../src/mouse/mod.rs)); no keyboard hook
except the unrelated one `y_core::hotkey` installs only for a stolen
`Win+…` hotkey.

AltSnap: it's the reverse.

* **Keyboard hook — always on.** Installed once, for the life of the
  process, and deliberately kept minimal:

  ```c
  ///////////////////////////////////////////////////////////////////////////
  // Keep this one minimalist, it is always on.
  __declspec(dllexport) LRESULT CALLBACK LowLevelKeyboardProc(int nCode, WPARAM wParam, LPARAM lParam)
  {
      if (nCode != HC_ACTION || state.ignorekey) return CallNextHookEx(NULL, nCode, wParam, lParam);
      ...
  ```

  Its whole job at rest is: notice when the configured "grab" key(s) (Alt,
  by default, but configurable — `state.alt` stores the actual vkey, not a
  bool) go down, and hand off to the mouse hook from there.

* **Mouse hook — installed only while a modifier is held.**
  `HookMouse()`/`UnhookMouse()` are called from inside the keyboard hook's
  own key-down/key-up handling, not from application start-up:

  ```c
  static void HookMouse(void)
  {
      state.moving = 0;
      if (conf.keepMousehook) {
          PostMessage(g_mainhwnd, WM_TIMER, REHOOK_TIMER, 0);
      }
      if (mousehook) return;
      mousehook = SetWindowsHookEx(WH_MOUSE_LL, LowLevelMouseProc, hinstDLL, 0);
  }
  ```

  Called from the keyboard hook's key-down branch, the moment the grab key
  is pressed:

  ```c
  if (wParam == WM_KEYDOWN || wParam == WM_SYSKEYDOWN) {
      if (!state.alt && !state.action.ac && ... && IsHotkey(vkey) && ...) {
          state.alt = vkey;
          state.blockaltup = 0;
          ...
          HookMouse();
          ...
      }
  ```

  and torn back down (`UnhookMouseOnly()`) once the grab key comes back up
  and no action is in progress.

**Why this shape, not y-snap's:** AltSnap's gesture is *modifier-first* —
hold Alt, then click. y-snap's is *button-first* — the modifier is read
with `GetAsyncKeyState` only once a configured mouse button goes down. That
difference is real and not worth un-doing (y-snap's `<mouse>` gestures
support plain, unmodified drags too, which AltSnap's model does not), but
it does mean y-snap cannot get away with a keyboard hook that's "always on
and effectively free" the way AltSnap's is: AltSnap's hook is on the modifier
key's *own* down/up, is idle for every other key already at the top of the
function (`if (nCode != HC_ACTION || state.ignorekey) return CallNextHookEx(...)`,
then a single-letter `vkey` switch), and it's *how the feature is driven* —
not extra machinery bolted on to fix a side effect. A keyboard hook that
exists solely to swallow one key-up per gesture is more defensible if it is
installed only while a gesture bound to Alt/Win is actually in progress —
see §4.

## 2. The disguise trick, and *when* it fires

The mechanism itself is the same idea HulK and PowerToys both reach for:
break the shell's "Alt/Win pressed and released with nothing in between"
detection by making sure something else happens in between. AltSnap's
version:

```c
#define Send_CTRL() if (conf.EndSendKey) { Send_KEY(conf.EndSendKey); LOG("END KEY SENT"); }
```

```c
static void Send_KEY(unsigned char vkey)
{
    KEYBDINPUT ctrl[2] = { {0, 0, 0, 0, 0}, {0, 0, KEYEVENTF_KEYUP, 0, 0} };
    ctrl[0].wVk = ctrl[1].wVk = vkey;
    ctrl[0].dwExtraInfo = ctrl[1].dwExtraInfo = GetMessageExtraInfo();
    INPUT input[2] = {0};
    input[0].type = input[1].type = INPUT_KEYBOARD;
    input[0].ki = ctrl[0]; input[1].ki = ctrl[1];
    InterlockedIncrement(&state.ignorekey);
    SendInput(2, input, sizeof(*input));
    InterlockedDecrement(&state.ignorekey);
}
```

Notable, relative to what y-snap has tried:

* **`conf.EndSendKey` defaults to `VK_CONTROL` (`0x11`)**, not Right Shift.
  It's user-configurable, which is itself a hint: AltSnap's own maintainers
  don't treat one specific filler key as reliably universal either.
* **No scan code, no `KEYEVENTF_EXTENDEDKEY`.** `KEYBDINPUT` is
  zero-initialised and only `wVk` and the up/down flag are set. This
  matches what y-snap's own experiments converged on for the current
  `absorb_modifier_tap` (`wScan: 0`, no extended flag) — AltSnap's own
  working code agrees with that, not with the "must set
  `KEYEVENTF_EXTENDEDKEY`" reading HulK's source suggested. HulK sets that
  flag; AltSnap doesn't; both apparently work in practice, which weakens
  "the flag" as *the* deciding variable and points harder at timing (below)
  as what actually matters.
* **Both events in one `SendInput` call**, an array of two — the thing
  y-snap tried and, per the last round of testing, didn't fix anything on
  its own. Doesn't rule it back in or out; just: AltSnap batches them too,
  so batching-vs-not is unlikely to be the deciding factor either.
* **`state.ignorekey`, an interlocked counter, not a `dwExtraInfo`
  signature**, is how the hook recognises and ignores its own injected
  keys — the very first line of `LowLevelKeyboardProc` bails out entirely
  while any injection is in flight (`if (nCode != HC_ACTION ||
  state.ignorekey) return CallNextHookEx(...)`). Coarser than y-snap's
  per-event signature match, but note what it implies: **the hook does not
  try to distinguish or filter the injected event by content at all — it
  just goes fully passive for the few microseconds `SendInput` takes.**
  That sidesteps any question of whether the hook's own logic might
  re-trigger on its own injected key.

None of the above is where the real difference is, though. The important
part is **where in the control flow `Send_CTRL()` actually gets called**:

```c
static void HotkeyUp(void)
{
    // Prevent the alt keyup from triggering the window menu to be selected
    // The way this works is that the alt key is "disguised" by sending
    // ctrl keydown/keyup events
    if (state.blockaltup || state.action.ac) {
        Send_CTRL();
        state.blockaltup = 0;
        if (NumKeysDown() > 1) state.blockaltup = 1;
    }
    state.alt = 0;
    state.alt1 = 0;
    ...
    if (!state.action.ac) UnhookMouse();
}
```

`HotkeyUp()` is called from exactly one place: the keyboard hook's
`WM_KEYUP`/`WM_SYSKEYUP` branch, when the key going up is the one
`state.alt` is currently tracking:

```c
} else if (wParam == WM_KEYUP || wParam == WM_SYSKEYUP) {
    if (IsHotkey(vkey)) {
        HotkeyUp();
    }
    ...
    return CallNextHookEx(NULL, nCode, wParam, lParam);
}
```

So the sequence, for a drag that used Alt, is:

1. **Mouse-down** (much earlier, in `init_movement_and_actions`, y-snap's
   equivalent of `on_button_down`'s activation): arm the flag —
   `state.blockaltup = state.alt;` — but *do not* touch the keyboard yet.
2. User drags, releases the mouse button. Nothing keyboard-related happens
   here either — `UnhookMouse()`'s job is the mouse hook, not the keyboard
   one.
3. **The real Alt key-up arrives**, at whatever point later the user
   actually lets go of Alt — could be before or after the mouse button.
   *Inside that same keyboard-hook call*, before the function returns and
   lets the real key-up continue down the chain, `Send_CTRL()` fires: Ctrl
   down, Ctrl up, both delivered by `SendInput` synchronously, *then* the
   original `CallNextHookEx(...)` call at the bottom of the function lets
   the real Alt key-up proceed.

The real Alt key-up is **never swallowed** — `return 1` doesn't appear
anywhere in this path. It's allowed through, but only *after* the Ctrl tap
has already landed immediately before it in the input stream the shell
sees. The shell gets: Alt-down (real, long ago) … Ctrl-down, Ctrl-up
(injected, just now) … Alt-up (real, right after) — never a clean,
uninterrupted Alt tap.

**This is the part y-snap hasn't done.** Both of y-snap's attempts so far
injected the filler key at *mouse* event time — at drag/resize activation
(`on_button_down`), sometimes also at deactivation (`on_button_up`) — which
is temporally decoupled from when Alt itself actually comes back up. If the
user holds Alt for a while after releasing the mouse (entirely normal — the
mouse button and the modifier don't have to come up together), the filler
key AltSnap or HulK's approach depends on already landed seconds earlier,
with nothing to guarantee it still counts as "interrupting" the tap by the
time the real release happens. AltSnap doesn't have this problem because it
never guesses when to fire the disguise — it watches for the actual event
that would trigger the menu and acts in the same callback, before letting
that event proceed.

## 3. The `blockaltup` flag: consumed vs. not

`state.blockaltup` is the same idea as the `consumed` flag the
`modifier_guard` prototype used, but AltSnap sets and clears it in more
places, which is worth internalising:

| Set to 1 when... | Set to 0 when... |
| --- | --- |
| A drag/resize actually activates: `state.blockaltup = state.alt;` in `init_movement_and_actions` | Right after `Send_CTRL()` fires and is consumed |
| A wheel/click action completes and did something: `state.blockaltup = ret && state.alt > BT_HWHEELU;` | On fresh Alt-down: `state.blockaltup = 0;` |
| Re-armed if more than one key is still down after disguising one release: `if (NumKeysDown() > 1) state.blockaltup = 1;` | |

That last one matters for chorded modifiers (Ctrl+Alt+drag, say): AltSnap
doesn't just fire-and-forget the disguise once — it re-checks whether
there's still a reason to keep disguising further releases before turning
the flag off for good.

`HotkeyUp()`'s own guard, `if (state.blockaltup || state.action.ac)`, is the
same shape as y-snap's `absorb_if_modified`, but is evaluated *at the
keyup*, using live state at that moment, rather than baked into a decision
made back when the gesture started.

## 4. The "hooks silently die" problem — AltSnap already hit this

Independent of the modifier question: AltSnap has a standing defence
against exactly the "drag doesn't stop, mouse hook seems to have gone deaf"
symptom y-snap has seen. A watchdog timer, re-armed every time the mouse
hook is installed, periodically checks whether the hook is still actually
receiving events and silently reinstalls it if not:

```c
case REHOOK_TIMER: {
    // Silently rehook hooks if they have been stopped (>= Win7 and LowLevelHooksTimeout)
    // This can often happen if locking or sleeping the computer a lot
    POINT pt;
    GetCursorPos(&pt);
    if (mousehook && !SamePt(state.prevpt, pt)) {
        UnhookWindowsHookEx(mousehook);
        mousehook = SetWindowsHookEx(WH_MOUSE_LL, LowLevelMouseProc, hinstDLL, 0);
    }
} break;
```

The comment is the important part: **this is a known, accepted-as-normal
Windows behaviour**, not a bug specific to one implementation. `>= Win7 and
LowLevelHooksTimeout` — the same 300 ms timeout y-snap's own hook module doc
already cites — silently drops a low-level hook, and the fix AltSnap ships
is not "never be slow" but "notice you went quiet and reinstall." The
check itself is cheap and coarse: has the cursor moved since the point we
last recorded, while we believe the hook is installed and should have told
us? If yes, the hook is stale.

This is worth lifting into y-snap more or less as-is: a low-frequency timer
(AltSnap uses 5 seconds:
`SetTimer(g_mainhwnd, REHOOK_TIMER, 5000, ...)`) that reinstalls
`MouseHook` if the cursor has moved without a corresponding `WM_MOUSEMOVE`
having reached it. Cheaper and more general than the `GetAsyncKeyState`
self-heal y-snap tried and reverted (which only covered "the button
matching the *current* gesture went missing," not "the hook itself is
gone" — and risked false positives from state-update races right after
activation, which is what that attempt ran into).

## 5. What this suggests for y-snap

In order of how directly each maps onto the current codebase:

1. **Move the injection from mouse-hook time to keyboard-hook time.**
   This is the one change that actually matches the mechanism AltSnap
   (and, per the earlier `GrabAndMove` read, PowerToys too, in its own more
   elaborate way) both rely on. Concretely, close to the user's own
   proposal: install a `WH_KEYBOARD_LL` hook **only while a gesture bound
   to Alt/Win is active** (armed at `on_button_down`'s activation,
   torn down once resolved — not the always-on, swallow-by-default design
   the earlier `modifier_guard` prototype used, which is what broke
   system-wide Alt/Win). That hook's only job: on seeing the real
   `WM_SYSKEYUP`/`WM_KEYUP` for the armed modifier, call
   `absorb_modifier_tap()` *synchronously, in that same callback*, before
   returning `CallNextHookEx` — never swallowing the real key-up itself,
   only preceding it with the filler tap, exactly like `HotkeyUp()` does.
   This needs the hook to exist for however long the user keeps holding
   Alt/Win after the mouse button comes up, not just for the instant of
   the button event — which is the actual reason a `WH_MOUSE_LL`-hook-only
   design structurally cannot do this correctly no matter which filler key
   or flags it uses.

2. **Don't re-litigate the filler key or the flags.** AltSnap's own
   production code uses Ctrl, no scan code, no extended flag, batched into
   one `SendInput` call — differing from HulK's Right-Shift-with-extended-flag
   on three of four points, and both are reported working. That's strong
   evidence the filler key's identity and flags are not where y-snap's bug
   lives; §1's timing gap is the more likely culprit, and is worth
   confirming before spending more cycles on flag combinations.

3. **Add a rehook watchdog for `MouseHook`**, independent of the modifier
   question — §4's `REHOOK_TIMER` pattern, ported as-is. This is a
   correctness fix on its own merits (a low-level hook silently dying is
   documented Windows behaviour, not an edge case), and cheap enough to add
   regardless of how the modifier-swallow work lands.

4. **Keep `y-snap`'s button-first gesture model.** Nothing here argues for
   copying AltSnap's modifier-first architecture wholesale — that would be
   a much larger redesign for a feature y-snap's config format doesn't
   currently need (unmodified-button gestures). The keyboard hook in (1)
   should be scoped to "while a gesture that used Alt/Win is
   outstanding," not promoted to being the thing that drives gesture
   activation.
