# Minimize-button interactions and the action overlay — implementation plan

Drag and resize (`mouse/gesture.rs`) and the minimize-button quick popup's
happy path (`min_overlay.rs`) work. This is what is left before y-snap
matches HulK's feature set: the minimize button's second click behavior, a
dwell delay before its popup appears, the full keyboard-driven action
overlay (`Action::Overlay`, currently `Status::Pending`), and everything
that action overlay needs from virtual desktops. Written to be read
top-to-bottom as a work queue — each part names the files it touches and
what in the existing code it has to fit around.

## Where things stand today

Already dispatched for real, in `actions.rs`: `minimize`, `maximize`,
`close`, `always-on-top`, `borderless`, `transparency`, `shade`,
`hide-taskbar`, `center-cursor`. Everything else — `overlay`, `iconize`,
`traynize`, `uniconize`, and all five `desktop-*` actions plus
`pin-toggle` — falls through to `Status::Pending` and only produces a
balloon saying so.

The minimize-button popup (`min_overlay.rs`) shows immediately on
`WM_MIN_HOVER`, i.e. the instant the pointer's ancestor window changes
under `hover::update`'s per-`WM_MOUSEMOVE` check — there is no delay and
no separate right-click path today. `hover.rs` only ever posts one
message, `WM_MIN_HOVER`, carrying the target `HWND` or null.

The big overlay's *data* is already fully parsed — `config::Overlay`,
`Palette`, `Group`, `Item` — and the `Item.desktop` / `Binding.desktop`
fields exist and are marked `#[allow(dead_code, reason = "read once the
desktop actions are written")]`. None of it is drawn or driven yet; there
is no overlay window, no key routing, and no code path that even
constructs one. `window.rs` similarly has `is_minimized` /
`is_borderless` / `is_faded` / `is_shaded` / `is_hidden_from_taskbar`
already written and marked dead-code "read by the overlay, which is not
written yet" — these are the toggle-state queries the overlay's tile
painting will call.

## Part 1 — minimize-button: dwell delay + right-click action

Two behaviors on the same button, kept apart the way `gesture` and `hover`
already are in `mouse/mod.rs`'s doc comment:

### 1a. Dwell before showing the popup

"After ~600ms–1s of mouse *inactivity*", not "600ms after entering the
button" — the popup should not appear while the pointer is still travelling
across the button on its way to the maximize/close buttons next to it, only
once it has actually stopped. That means the timer has to reset on every
`WM_MOUSEMOVE` still inside the button rect, not just arm once on entry.

Per `mouse/mod.rs`'s own doc comment, nothing that can wait belongs inside
the hook callback — so the dwell timer itself has to live on the message
loop's side, the same way `REHOOK_TIMER_ID` does, not as a `SetTimer` call
from inside `mouse_hook`. Shape:

- `hover::update` keeps posting `WM_MIN_HOVER` when the hovered *window*
  changes (unchanged), but additionally posts on every move while still
  inside the same window's button rect — or, cheaper, `main.rs` restarts a
  `SetTimer`/`KillTimer` pair keyed off `WM_MIN_HOVER` itself and a new
  message for "still moving in there," `WM_MIN_HOVER_MOVE` say, posted at a
  throttled rate rather than once per pixel.
- A new timer id (next to `REHOOK_TIMER_ID`) fires the popup after the
  configured delay if no reset arrived meanwhile; leaving the button
  (`WM_MIN_HOVER` with a null target) kills the timer outright.
- `min_overlay::MinOverlay::show` itself does not change — only *when*
  `main.rs` calls it does.
- Configurable delay: a `hover-delay` (ms) attribute on `<minbutton>` in
  `config.rs`, defaulting to something in the 600–1000ms range the user
  named; falls in next to the existing `check(element, &["action",
  "label", "icon"])`-style validation already there for the section.

### 1b. Right-click default action

A plain right-click over the minimize button rect (no modifiers) should
fire one configured action directly — no popup, default "send to tray"
(`traynize`). Where this plugs in:

- `min_button_rect` and `min_button_target` already exist in `hover.rs` and
  are exactly what is needed to test "is this `WM_RBUTTONDOWN`/`UP` over a
  minimize button" — no new hit-testing.
- The natural home is `mouse_hook`'s `WM_RBUTTONDOWN`/`WM_RBUTTONUP` arms in
  `mouse/mod.rs`. Right now those unconditionally go to
  `state.gesture.on_button_down`/`on_button_up`, which only swallow the
  event when a `resize`-like gesture is actually configured and matches.
  This needs a check *before* (or interleaved with) that: if the down/up
  pair both land inside the same window's `min_button_rect` and no
  drag/resize is in progress, swallow it and post a new message —
  `WM_MIN_RIGHTCLICK`, carrying the target `HWND` — instead of letting it
  fall through to `gesture`.
- Track this as its own small piece of state (armed-on-down, fired-on-up,
  cancelled if the pointer leaves the rect before up) — the same "click,
  not just press" shape `min_overlay::ClickHandler` already uses for the
  popup's own tiles, just at the hook level instead of a window handler.
- Config: a `right-click-action` attribute on `<minbutton>`, parsed with
  `Reader::action`-style lookup (defaulting to `Action::Traynize`); `main.rs`
  gets an `on_min_rightclick(target)` alongside `on_min_action`, calling
  `self.perform(action, Some(target))` the same way.
- Interaction with 1a: a right-click should probably cancel any pending
  dwell timer for that button (or the popup should just be a no-op replay
  if it fires a moment later) — pick one and note it in the commit rather
  than leaving it to fall out by accident.

## Part 2 — the action overlay (`Action::Overlay`) — done, minus step 3

Implemented in `y-snap/src/overlay.rs`, wired into `main.rs::on_hotkey`
alongside `Quit`/`Reload`. Steps 1, 2, 4, 5, 6, 7 below are all in; step 3
(`desktop-list` expansion) is still skipped on purpose — a `desktop-list`
tile is simply left out of the grid entirely rather than shown broken,
until Part 3 exists to expand it for real.

**Step 6 shipped differently than planned below.** Rather than a scoped
`WH_KEYBOARD_LL` hook, the popup's `Overlay` is `OverlaySpec::focusable`
(dropping `WS_EX_NOACTIVATE`) and takes real focus via
`Overlay::activate()` right after showing — the same thing HulK's
`ShowActionOverlay` does (`SetForegroundWindow`/`SetFocus`). `WM_KEYDOWN`
then arrives at the popup's own `ClickHandler` directly, no hook needed,
and it is also what implements "click outside the overlay closes it" —
losing focus (`WM_KILLFOCUS`) closes it, with no `WH_MOUSE_LL` involvement
at all. See `overlay.rs`'s own module doc comment for the detail.

The larger piece. `Action::Overlay` needs its own module — call it
`overlay.rs` in `y-snap/src/` (the name `min_overlay.rs` is a good template
to read first: lazy `Overlay` creation, `WindowHandler` for clicks, a
`paint` that redraws from `config::Overlay` on every show).

Sub-tasks, roughly in build order:

1. **Trigger wiring.** `Action::Overlay` cannot go through
   `actions::dispatch` unchanged — like `Quit`/`Reload`, it needs
   application state (`settings.overlay`, the owner window), not just a
   target `HWND`. `main.rs::on_hotkey` special-cases it next to
   `Action::Quit`/`Action::Reload`.
2. **Grid layout.** Items with no `group` fill `columns`-wide rows in
   declaration order, `tile` size each. Items with a `group` are laid out
   on their own row(s) using that group's `columns`/`tile_width`/
   `tile_height` overrides where set (`config::Group`, already parsed).
   Total window size is the union of both.
3. **`desktop-list` expansion.** At *open* time (not load time — desktops
   can be created/removed between opens) the single `desktop-list` item
   expands into one tile per current virtual desktop, each carrying its own
   generated key (numbers, or letters if there are more desktops than
   digits) and `desktop` index, inert for whichever desktop the target
   window is already on. This is the one place the overlay directly depends
   on Part 3's desktop enumeration.
4. **Rendering.** Per tile: background from `Palette` (`back`/`hover_back`/
   `active_back`/`disabled_back`), `icon` glyph (icon font) or key letter,
   `label` vs. `label_on` chosen by querying the matching `window::is_*`
   state function for the target window (`is_maximized`, `is_borderless`,
   `is_faded`, `is_shaded`, `is_hidden_from_taskbar`, plus a new
   `is_pinned`/`is_on_top` pair for `pin-toggle`/`always-on-top`) — this is
   exactly what those functions' "read by the overlay" dead-code notes are
   waiting for.
5. **Focus border.** `overlay.focus_border`/`border_width`: a second,
   click-through `Overlay` (or `Canvas::stroke_rect` on a window-sized
   layered surface) traced around the target window's rect while the tile
   overlay is open, torn down together with it.
6. **Keyboard routing.** The overlay is driven by a single key press per
   `config.rs`'s own doc comment ("chosen with a single key press"), but
   `Overlay`/`WindowSpec` sets `WS_EX_NOACTIVATE` — it never gets keyboard
   focus, so `WM_KEYDOWN` on its own `hwnd` will not fire. Needs a scoped
   `WH_KEYBOARD_LL` hook, installed only while the overlay is shown and torn
   down when it closes — architecturally the same shape as
   `mouse/modifier_guard.rs`'s arm/disarm hook, not a new pattern. Maps
   `Item.key` (already a parsed `VIRTUAL_KEY`) to a tile, `Escape` and a
   click outside the overlay both close it without acting.
7. **Lifecycle.** An `ActionOverlay` struct owned by `YSnap` next to
   `min_overlay`, rebuilt on every `load()` (its tiles come from
   `settings.overlay` same as `min_overlay`'s come from `settings.min_button`),
   torn down in `on_destroy` in the same order comment that already calls
   out hooks having to come down before the message loop stops.

## Part 3 — virtual desktop actions

`DesktopNext`, `DesktopPrev`, `DesktopNew`, `DesktopMove`, `DesktopList`,
`PinToggle`. Windows has no *public, documented* API for switching to,
creating, or enumerating virtual desktops — only the undocumented,
per-build COM interfaces (`IVirtualDesktopManagerInternal`,
`IVirtualDesktopPinnedApps`, `IApplicationViewCollection`, …), whose GUIDs
and vtable shapes have changed release to release, sometimes gaining
features (pin support, for instance, is not there on Windows 10 / early
Windows 11) and not just renumbering existing ones.

**Own implementation, not the `winvd` crate.** HulK already carries the
GUID table and the version-by-version account of which interface members
came and went; `winvd` (`ciantic/VirtualDesktopAccessor`, `rust` branch)
is useful only as a second reference for what a Rust wrapper over the same
surface looks like — its own published build now targets Windows 11 24H2
and newer exclusively, which is narrower than what HulK already supports
and narrower than what this needs to keep. Porting HulK's own table is the
way to keep that range, and it also means the ownership of "which GUID for
which build" stays in this codebase rather than behind a third-party
crate's release cadence.

Shape this the way `altsnap-hooks-analysis.md` and
`powertoys-grabandmove-analysis.md` shaped the modifier-swallow work: a
short analysis document first, distilled from HulK's own source, then the
implementation follows it directly rather than re-deriving anything.
**Blocked on HulK's GUID/version table being available to work from** —
point me at the source (or paste the relevant table/switch) and the
analysis doc + module can follow the same day. Roughly:

1. **`virtual-desktop-com-analysis.md`** — HulK's GUID table transcribed
   per Windows build/version, which interface members exist on which
   range, and which of this plan's six actions each range can actually
   support (this is where "pin may not exist on Windows 10 / early
   Windows 11" gets a precise floor instead of a hedge).
2. **A small internal module** (`y-snap/src/desktop.rs`, or a `virtual_desktop`
   module in `y_core` if `y-coustic`/others end up wanting it too) —
   runtime build detection (`RtlGetVersion`-style, the way version-gated
   behaviour usually has to be done since `GetVersionEx` lies), the
   per-version interface definitions, and a small safe API in front of
   them shaped like the table below so the rest of `y-snap` never touches
   raw COM:

   | Action | module call |
   | --- | --- |
   | `DesktopNew` | create a desktop |
   | `DesktopNext` / `DesktopPrev` | current index + count, switch to index ± 1 |
   | `DesktopMove` | move a window to desktop N (`Binding.desktop`/`Item.desktop`, already parsed, 1-based) |
   | `DesktopList` | enumerate live desktops for the overlay's tile expansion, plus "is this the target's own desktop" |
   | `PinToggle` | read + flip pinned state, where the running build supports it at all |

3. **Graceful degradation.** On a build/interface combination the table
   doesn't recognise, or for `PinToggle` specifically on a build that has
   no pin interface at all: `config.rs`'s existing `Status::Pending`-style
   fallback is the right shape — disable the affected tiles/hotkeys and say
   why in the same balloon `main.rs::report` already uses for configuration
   problems, rather than crash or guess at a GUID that doesn't apply.

## Part 4 — iconize / traynize / uniconize

Smaller, no research spike needed, listed here mainly so it isn't lost:

- `traynize` — hide the window and give it its own icon in the
  notification area (`y_core::tray` already has the primitives `y-snap`
  uses for its own tray icon); clicking that icon restores it.
- `iconize` — hide the window behind an icon in a small list y-snap keeps
  for itself (not the system tray) — needs that list's own UI, however
  minimal.
- `uniconize` — bring back the most recently hidden window, from whichever
  of the two lists above it went into; `Action::needs_window()` already
  excludes it (it acts on y-snap's own state, not a target), matching
  `Reload`/`Quit`/`DesktopNext`/`DesktopPrev`/`DesktopNew`.

## Suggested order

1. Part 1a (dwell) and 1b (right-click) — small, self-contained, no new
   research, immediately useful on their own.
2. Part 2 minus step 3 (`desktop-list` expansion) — the overlay is useful
   the moment it can show and drive every non-desktop tile, and none of it
   waits on Part 3.
3. Part 3's analysis doc, as soon as HulK's GUID/version table is
   available to transcribe from — the actual blocker on this whole part.
4. Part 3's module, once the analysis doc exists — unblocks
   `desktop-next`/`-prev`/`-new`/`-move`/`pin-toggle`, the corresponding
   hotkeys already commented out in `rsc/y-snap.xml`, and closes the
   `desktop-list` gap left in Part 2.
5. Part 4, whenever — independent of everything above.
