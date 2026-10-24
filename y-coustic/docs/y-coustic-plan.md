# y-coustic — plan

`y-coustic` is the Rust rewrite of GUIPro's **VulCan**, a tray utility that
changes the system volume from a hotkey or a mouse button/wheel. Its reason
to exist is keyboards that have no volume keys (compact boards, laptops
with odd Fn layers, remote desktops), so every function must be reachable
from the **mouse** as well as from the keyboard.

Beyond the port, three things are new:

1. **Per-application volume**: change the volume of one application (the one
   under the pointer, or the focused one) instead of the whole system.
2. **Customizable overlays** in place of VulCan's tray balloon.
3. A **config schema in XML** like `y-snap.xml`, instead of `vulcan.ini`.

Status: nothing is implemented. `y-coustic/src/main.rs` is still the
`Hello, world!` placeholder. This document records what VulCan does, what
we keep, what we drop, and what is still open.

## What VulCan does (the prior art)

Source: `../guipro-git/VulCan/src`. The whole program is about 800 lines.

| File | Role |
| --- | --- |
| `main.cpp` | hidden top-level window, `WM_HOTKEY` dispatch, tray icon (left click shows the volume, right click shows the menu), `FlushMemory()` after every event |
| `volume.cpp` | `IAudioEndpointVolume` on the default render endpoint (`eRender`/`eConsole`): get/set master scalar, get/set mute. Interfaces are **created lazily and released after every hotkey** to keep the resident footprint small |
| `hotKey.cpp` | `RegisterHotKey`, plus a `WH_KEYBOARD_LL` hook that *steals* `Win+<key>` combinations Windows has already claimed (it posts `WM_HOTKEY` to itself and synthesizes a Shift tap to cancel the Start menu) |
| `mouse.cpp` | `WH_MOUSE_LL` hook: left/right/middle button and wheel, each combined with modifier keys read through `GetKeyState`. The button-down is swallowed and the action fires on button-up |
| `config.cpp` | `vulcan.ini`: `[global] step=5` (clamped to 1–30 %), then `[UP]`, `[DOWN]`, `[MUTE]` sections with `key=`, `mod=` (letters `W`/`C`/`A`/`S`) and `mouse=` |

Three actions only: volume up, volume down, mute toggle. Default bindings are
`Win+Num+`, `Win+Num-` and `Win+Num0`. Feedback is a tray balloon with the
resulting percentage.

## Goals

* **Master volume**: up, down, mute toggle, and "set to N %" (useful for
  presets bound to a key).
* **Every action bindable to a hotkey *or* a mouse input**, with modifiers.
  Mouse inputs: wheel up/down (with modifiers), and button chords
  (`Alt+MiddleClick` to mute, etc.). A bare wheel is never claimed; a
  modifier is required, and that is validated at load.
* **Application volume**: the same actions applied to one application's
  audio session. The target is chosen by `target="pointed"` (the window under
  the pointer, falling back to the focused one) or `target="focused"`, the
  same vocabulary as `y-snap`.
* **Overlays** that show what just changed: level bar, percentage, mute
  state, and for an application the app's icon and name. Look, position and
  duration are configurable.
* **Small and quiet**: no resident audio COM objects between events if it can
  be avoided, no polling, working set trimmed after each event
  (`y_core::memory`).

Non-goals (for now): a full mixer window, audio device switching, equalizer
or per-channel balance, microphone/capture devices.

## Audio layer

A new module `y-coustic/src/audio.rs`. It is the only place with COM and
`unsafe`, following the convention of `y-snap/src/mouse.rs`, with a typed,
safe surface for the rest of the app.

### Master volume

`IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender, eConsole)` →
`IAudioEndpointVolume`. `GetMasterVolumeScalar` / `SetMasterVolumeScalar` /
`GetMute` / `SetMute`, passing our own event-context GUID so that a future
change notification can recognise its own writes.

The default endpoint is **re-resolved on every action**: a user can switch
output device (headphones plugged in, Bluetooth) at any time, and VulCan's
cached `pDevice` would then keep driving the old one until it was reloaded.
Resolving per action costs a few hundred microseconds and keeps the
"nothing resident" property for free.

### Application volume

`IMMDevice::Activate(IAudioSessionManager2)` →
`GetSessionEnumerator` → per session `IAudioSessionControl2`
(`GetProcessId`, `GetSessionIdentifier`, `IsSystemSoundsSession`) and
`ISimpleAudioVolume` (`Get/SetMasterVolume`, `Get/SetMute`). This is what
the Windows volume mixer itself uses, and the values are the same as the
mixer's sliders.

Known difficulties, to be designed for rather than discovered:

* **A window is not an audio session.** The match goes window → process id →
  session. Several cases break the 1:1 assumption:
  * *Multi-process browsers* (Chrome, Edge, Firefox): the window belongs to
    the browser process, the session to an audio-service child. Match on the
    **process tree** (any session whose process is the window's process or a
    descendant) and, failing that, on the **executable name**.
  * *UWP/Store apps* hosted by `ApplicationFrameHost.exe`: the visible
    window's pid is the host, not the app. Resolve the real app through its
    core window (the child with a different pid), as `y_core::process`
    may already allow; check before writing new code.
  * *Several sessions for one app* (one per tab/stream): the action applies
    to **all** matching sessions, and the displayed level is the loudest or
    the first; to be decided (see open questions).
* **No audio session until the app plays.** A session appears when the
  application first opens a stream, and often disappears when it closes it.
  If the pointed application has none, the overlay says so ("no audio") and
  the input is **not** swallowed (a wheel that does nothing must still
  scroll). Falling back to master would be surprising: no fallback.
* **Per-session mute vs. volume 0.** `ISimpleAudioVolume` has its own mute
  flag, independent of the level. Mute toggle must use it, so that unmuting
  returns to the previous level (VulCan's `getVolume()` reports 0 while
  muted, which is a display choice to keep; the stored level is never
  touched).

### Notifications (open)

`IAudioEndpointVolumeCallback` and `IAudioSessionEvents` would let the
overlay react when the volume is changed *elsewhere* (hardware keys, the
Windows flyout). That requires keeping COM objects alive, which fights the
low-memory goal. **Not in the first version**; the overlay only reports
changes y-coustic made itself.

## Input

### Hotkeys

`y_core::hotkey` for `RegisterHotKey`, with the same `steal-from-shell`
mechanism `y-snap` has for combinations Windows owns (VulCan did exactly
this for `Win+<key>`). The multimedia VK codes (`VOLUME_UP`, `VOLUME_DOWN`,
`VOLUME_MUTE`) are already in the key catalog, so on a keyboard that does
have them they can be rebound to per-app actions with a modifier.

### Mouse

`WH_MOUSE_LL` hook, state in `thread_local!`, same rules as `y-snap`: the
hook procedure only decides "swallow or not" and `PostMessageW`s a private
`WM_APP` message; the COM work and the overlay happen on the message loop.
`GetKeyState` for modifiers is fine inside the hook; **no COM call, no
window lookup that crosses processes** (`WindowFromPoint` is resolved in the
handler, not in the hook).

Differences from VulCan worth fixing in the port:

* VulCan only reacts when the wheel delta is **exactly** `±WHEEL_DELTA`
  (120). Free-spinning and high-resolution wheels and touchpads report
  smaller deltas and were silently ignored. y-coustic accumulates the delta
  and emits one step per 120 units (and a proportional step for precision
  devices if cheap).
* VulCan swallowed button-down and acted on button-up, but left the
  half-swallowed pair to chance if the modifier was released in between.
  Keep the "swallow the down, consume the up" shape but make the session
  state explicit (`Idle` / `Pressed { button }`) so a stray up is never
  eaten.
* Wheel *left/right* (`WM_MOUSEHWHEEL`) are supported as inputs too, since
  VulCan's own source defines them even though its ini parser never
  accepted them.

### Target resolution

Reuse `y-snap/src/target.rs`'s idea (pointed → focused fallback, refuse
shell windows). If both crates need it, move the filtering list into
`y_core` rather than copying it. The `BANISHED` list should not diverge.

## Overlays

Built on `y_core::overlay::Canvas`: borderless, topmost, non-activating,
layered, click-through. Draw order: fill panel → text → round corners.
A single overlay window is reused (re-drawn and re-armed), not recreated per
event.

Behaviour:

* Shown on volume/mute change, hidden after `timeout` ms of no further
  change; a new change **restarts** the timer instead of stacking windows.
* Optional fade-in/out (alpha steps through `UpdateLayeredWindow`, no
  per-frame animation thread; a short `SetTimer` series is enough).
* Placed on the monitor under the pointer for `target="pointed"`, the
  monitor of the focused window otherwise, or a fixed monitor.

Content, per kind:

| Kind | Shows |
| --- | --- |
| master | speaker glyph (with a muted variant), the level bar, the percentage |
| application | the application's icon, its name, the level bar, the percentage |

Looks, as a first list of what is customizable (all optional, all with a
theme-derived default via `y_core::theme`, `theme="auto"` as in y-snap):

* **style**: `bar` (horizontal), `vbar` (vertical), `ring`, `text` (percentage
  only), `compact` (icon + number);
* **position**: anchor (`top-center`, `bottom-center`, `center`, corners…),
  margin, or `follow-cursor`;
* **size**, corner radius, border, opacity, font and font size;
* **colours**: panel, bar track, bar fill, muted fill, text. The y-snap
  overlay colour attributes are the precedent;
* **timeout** and fade;
* **separate styles for master and application overlays**, so a quick glance
  tells them apart (a different accent colour is the cheap default).

Simplification vs. VulCan: the balloon is dropped entirely. The Windows
notification balloon is slow, queued by the shell and silently suppressed in
Focus Assist, none of which suits something shown on every wheel notch.
(Say so in the code, per the project's porting convention.)

## Configuration

`y-coustic.xml`, in the same two places and with the same loader as
`y-snap` (`y_core::config` + `y_core::xml`, no external dependency). It is a
clean-slate schema, not an `vulcan.ini` reader (project precedent).
Severity split as everywhere: a malformed document aborts the load with its
line number; an unknown action, a bad key combination or an unknown
attribute is a non-fatal warning and the rest of the file still loads.

A first sketch; names are not final:

```xml
<y-coustic>
	<!-- Step is a percentage of the full range, clamped to 1..30 as in VulCan. -->
	<volume step="5"/>

	<hotkeys>
		<hotkey key="Win+Num+" action="volume-up"/>
		<hotkey key="Win+Num-" action="volume-down"/>
		<hotkey key="Win+Num0" action="mute"/>
		<hotkey key="Ctrl+Win+Num+" action="app-volume-up"   target="focused"/>
		<hotkey key="Ctrl+Win+Num-" action="app-volume-down" target="focused"/>
	</hotkeys>

	<mouse>
		<wheel mod="Alt"       action="volume"/>
		<wheel mod="Alt+Shift" action="app-volume" target="pointed"/>
		<button button="middle" mod="Alt"       action="mute"/>
		<button button="middle" mod="Alt+Shift" action="app-mute" target="pointed"/>
	</mouse>

	<overlay theme="auto" style="bar" position="bottom-center" timeout="1200">
		<master/>
		<application accent="#E8A33D"/>
	</overlay>
</y-coustic>
```

Wheel bindings take the direction from the wheel itself: one `<wheel>` element
covers up and down (up raises, down lowers; an `invert="true"` attribute
flips it), instead of VulCan's separate `WHEEL_UP`/`WHEEL_DOWN` entries.

Actions (first set):

| Action | Meaning |
| --- | --- |
| `volume-up`, `volume-down`, `mute` | master |
| `volume` | master, wheel only: direction from the wheel |
| `set-volume` (`value="30"`) | master to a fixed level |
| `app-volume-up`, `app-volume-down`, `app-mute`, `app-volume` | the target application |
| `app-set-volume` (`value="…"`) | the target application to a fixed level |

## Application shape

Follows the standard Ypo-Lite layout (`skeleton/README.md`):

```
y-coustic/
  Cargo.toml
  build.rs, rsc/            icon, manifest, version info, default y-coustic.xml
  src/main.rs               App struct, y_core::app::start, message match
  src/config.rs             XML → typed Config (+ warnings), with unit tests
  src/audio.rs              the only COM code: master + per-session control
  src/input.rs              mouse hook + hotkey glue (thread_local state)
  src/overlay.rs            drawing and timing of the overlay
  docs/y-coustic-plan.md    this file
```

`audio.rs` and `config.rs` are testable without a device or a window: the
config has the usual `the_default_configuration_is_valid` test, and the
session-matching rules (process tree, executable name) are written as pure
functions over lists of `(pid, parent_pid, exe)` so they can be tested with
fixtures. The audio calls themselves can only be checked on a real machine.

## What must be added to the workspace

* `windows` features in the **root** `Cargo.toml` (single source of truth):
  `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`, `Win32_System_Com`
  (probably already present), `Win32_UI_Shell_PropertiesSystem` if the
  session display name is read from the property store. Exact list to be
  confirmed when writing `audio.rs`.
* Nothing else new is expected: hotkeys, mouse hook patterns, overlay canvas,
  theme, config, tray and memory trimming already exist in `y-core`.

## Phases

1. **Master volume, hotkeys only.** The VulCan parity milestone: config
   loader, `audio.rs` master path, three actions, a plain bar overlay,
   tray menu (reload, quit, startup toggle).
2. **Mouse.** Wheel with modifiers (accumulated delta) and button chords,
   with the explicit swallow state machine.
3. **Application volume.** Session enumeration, window → session matching,
   the "no audio" feedback, the application overlay.
4. **Overlay customization.** Styles, positions, colours, fade; both overlay
   kinds styled independently.
5. **Polish.** Optional notifications for external changes (if a cheap way
   is found), per-session level display, a `y-coustic-config` screen in the
   style of `y-snap-config` / `y-pop-config` once the schema is stable.

## Open questions

* **Several sessions for one application**: scale them all by the same
  step, or move them to a common level first? Which level does the overlay
  display? (Proposed: apply the step to each, display the maximum.)
* **Pointed target needs the pointer to be over a window of the app**, but a
  background music player is often minimized or in the tray: there is
  nothing to point at. Candidates: a named target (`target="app:spotify.exe"`
  in the hotkey), or an overlay/menu listing audio sessions to pick from.
  Probably both, named target first.
* **Should the app overlay be able to be driven by the mouse itself**
  (click or wheel on the bar)? It is click-through today; making it
  interactive costs focus handling and is deliberately out of scope for now.
* **Wheel over the tray icon**: `Shell_NotifyIcon` does not report wheel
  events, so VulCan's tray icon could never do it either. A "wheel over the
  taskbar" binding is possible with the hook but needs taskbar hit-testing;
  undecided.
* **Notifications vs. footprint** (see Audio layer): whether to keep an
  endpoint callback alive at all.
* **Sharing target resolution with y-snap**: move to `y_core`, or keep two
  filters in sync by hand.
