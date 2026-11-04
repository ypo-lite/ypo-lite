# y-coustic — plan

`y-coustic` is the Rust rewrite of GUIPro's **VulCan**, a tray utility that
changes the system volume from a hotkey or a mouse button/wheel. Its reason
to exist is keyboards that have no volume keys (compact boards, laptops
with odd Fn layers, remote desktops), so every function must be reachable
from the **mouse** as well as from the keyboard.

The design serves two groups, and neither is the afterthought: people with no
media keys get a full set of hotkeys, the mouse and a keyboard-driven panel;
people with media keys get those keys extended, with a modifier choosing what
they control.

Beyond the port, six things are new:

1. **Per-application volume**: change the volume of one application (the one
   under the pointer, or the focused one) instead of the whole system.
2. **Microphone control**: level and mute of the capture device, with the same
   inputs and overlays as the output.
3. **An interactive panel**: one overlay to adjust and mute the master, the
   microphone and each application, by mouse, wheel or keyboard.
4. **Media key layers**: the volume keys drive the master, the microphone or an
   application depending on the modifier held, optionally replacing the
   Windows flyout with our own overlay.
5. **Customizable overlays** in place of VulCan's tray balloon.
6. A **config schema in XML** like `y-snap.xml`, instead of `vulcan.ini`.

Status: Phase 0 (the probe) is done; nothing else is implemented, and
`y-coustic/src/main.rs` is still the `Hello, world!` placeholder. This document
records what VulCan does, what we keep, what we drop, and what is still open.
The order of work, task by task, and the progress record are in
[`implementation-plan.md`](implementation-plan.md).

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
* **Application volume**: the same actions applied to one application, meaning
  all of its audio sessions. The target is chosen by `target="pointed"` (the window under
  the pointer, falling back to the focused one) or `target="focused"`, the
  same vocabulary as `y-snap`.
* **Microphone**: the same up, down, mute and "set to N %" on the default
  capture device, from every input and shown in every overlay.
* **Media keys, when the keyboard has them**: `VolumeUp`, `VolumeDown` and
  `VolumeMute` combined with a modifier reach the microphone or an application.
  Taking over the bare keys, and so the Windows flyout, is the user's choice.
* **No media keys required**: every feature is reachable without them. One
  hotkey opens the panel, whose arrow keys then do everything, on any keyboard.
* **A HUD and a panel**: the HUD shows what just changed (level bar, percentage,
  mute state, and for an application the app's icon and name) and the panel
  adjusts everything at once. Look, position and duration are configurable.
* **Noticing outside changes**: a volume or mute changed by Windows, by another
  program or by a laptop key can bring up the HUD, as configured (see Keeping up
  with outside changes).
* **Small and quiet**: no polling while the panel is closed, working set trimmed
  after each event (`y_core::memory`). Between events, the only resident audio
  objects are the watchers the configuration asks for.

Non-goals (for now): a resizable mixer application, audio device switching,
equalizer or per-channel balance, per-application microphone control (see
Microphone), and the wheel over the tray icon: hovering the icon brings up the
HUD, which takes the wheel (see Hovering the tray icon). The panel is a
transient overlay, not a window the user manages.

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

### Microphone

The same `IAudioEndpointVolume` calls on `eCapture`, so `audio.rs` has one
endpoint path with the data flow as a parameter rather than a second copy.

* **The level belongs to the device, not to an application.** Changing it
  changes what every application hears from that microphone, so there is one
  microphone row and `target="mic"` has no per-application form. Windows offers
  no per-application input level: its mixer covers render endpoints only
  ("does not display the volume controls for audio-capture endpoint devices"),
  and the volume of a capture session does not survive a restart, unlike a
  render session's. `IAudioSessionManager2` does activate and enumerate on a
  capture endpoint (checked on the development machine, 2026-11-02), but no
  application was capturing, so whether another application's capture session
  appears, or reacts to `ISimpleAudioVolume`, is unproven. Phase 0 settles that
  for the record. Even if it reacts, it stays out of scope: the setting would
  be invisible in Windows, lost on restart, and fought by the automatic gain of
  call applications.
* **Two defaults can differ.** The Sound control panel has a default device and
  a default *communication* device: a call application may be using a different
  microphone from the one `eConsole` returns. Mute must not leave a live
  microphone behind, so it applies to the `eConsole` and `eCommunications`
  capture endpoints when they differ. The displayed level is the `eConsole`
  one.
* **Muted must mean muted.** A wrong mute state is worse here than for the
  output: someone believes they are silent in a call. Each mute is read back
  after writing, and the HUD shows what was read, not what was requested.
* **No microphone** (none plugged in, or all disabled): the row is disabled and
  the HUD says so. A hotkey still consumes its key, as it does for any bound
  combination.
* **Showing the state outside the HUD.** The tray icon has a "microphone muted"
  variant (`TrayIcon::set_icon`), the cheap always-visible signal. Keeping it
  accurate when another program changes the mute takes a watcher, see Keeping
  up with outside changes.
* **A laptop's own mic mute key** is one of those outside changes. On the
  development laptop (Phase 0, 2026-11-02) the key flips the capture endpoint's
  mute, as `GetMute` shows, but logs no key in the probe's special-key set and
  no shell `WM_APPCOMMAND`, so it is not something y-coustic can bind. It needs
  no binding: Windows has the true state, so `GetMute` read-back holds, and
  `y-coustic` mutes the same flag. Its LED follows `SetMute` too, so the LED and
  the HUD agree. That is one laptop; another may mute below
  Windows, where `GetMute` would not show it. The HUD appears for this key only
  through a watcher, and whether the key reaches one is still to be checked.

### Application volume

`EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)` → per endpoint
`IMMDevice::Activate(IAudioSessionManager2)` → `GetSessionEnumerator` → per
session `IAudioSessionControl2` (`GetProcessId`, `IsSystemSoundsSession`) and
`ISimpleAudioVolume` (`Get/SetMasterVolume`, `Get/SetMute`).

**The user sees an application, the API sees sessions.** Windows has no
application volume object. The API works on sessions, and the mixer shows one
row per application only because of how sessions are normally formed. What a
listing on the development machine showed (2026-11-02, five applications on
three render endpoints):

* Every application used the default session, so all the streams of a process
  are one session. A browser does not get one per tab.
* The session identifier has no pid in it, only the executable path and a
  session GUID, and the GUID was null everywhere. The pid is only in the
  instance identifier. Microsoft says a render session's volume persists, so
  this is why a level in practice belongs to the executable. That link is our
  reading, not a documented rule, and the identifier's format is undocumented:
  it is not parsed.
* A session belongs to **one endpoint**. Chrome and Windows Terminal each had
  two, same pid, one per device, so the level is really per application *and*
  per device.
* The sessions outlive the sound: all of them were inactive and still listed.

The documented exceptions remain: an application can put streams in several
sessions, or share one across processes, and each running instance is a
session instance of its own.

So y-coustic's unit is the application, and its identity is the **executable
path** (`y_core::process::executable_path` of the session's pid):

| Operation | On all the application's sessions, on every active render endpoint |
| --- | --- |
| Step | Each session moves by the step, so a difference between them is kept |
| Set (`set-volume`, click, drag) | Each session is set to the value |
| Mute | If any is unmuted, all are muted; otherwise all are unmuted |
| Displayed level | The highest of them |
| Displayed mute | Muted only if all are muted |

Working only on the default endpoint would miss an application sent to another
device in the Windows per-application output settings: it would have no row.

Sessions sharing a non-null grouping parameter are meant to move as one control
(Microsoft's rule for a volume control program), but that needs no separate
code here: the listing shows one grouping parameter per application across
devices, where Microsoft documents `GUID_NULL` as the default, so the parameter
is ignored and the executable path decides. System sounds have their own row
(`IsSystemSoundsSession`, pid 0). A cross-process session has no single pid
(`AUDCLNT_S_NO_SINGLE_PROCESS`), so it is a row of its own, named from
`GetDisplayName`.

Writing to another application's session is what this feature rests on, and
Microsoft's documentation says an application "cannot change the volume levels
of sessions that belong to other, unrelated applications". Practice disagrees:
Windhawk's *Taskbar Volume Control Per-App* mod (1.1.4) enumerates the sessions
of other processes and calls `ISimpleAudioVolume::SetMasterVolume` on them with
plain COM, and the Phase 0 probe did the same from its own process.

That mod is the closest prior art, and we differ from it on purpose:

| | The mod | y-coustic |
| --- | --- | --- |
| Endpoints | Default only | Every active render endpoint |
| Matching | Pid, then a Chromium audio child found by reading its command line from the PEB | Executable path, no reading of another process's memory |
| Several sessions | Each flipped from its own mute state, so they can diverge | One mute state for the application |
| No session | The wheel is swallowed | The wheel passes through |
| Reaching the target | Hooks into undocumented taskbar classes, by injection | The window under the pointer, or the panel |

Known difficulties, to be designed for rather than discovered:

* **A window is not an audio session.** The match goes window → process id →
  executable path → sessions, in this order:
  1. **Same executable path.** This covers multi-process browsers. Chrome,
     Opera and Discord each held their session in a `--utility-sub-type=
     audio.mojom.AudioService` child, not the process that owns the window,
     but the child runs the same executable. Firefox held it in its main
     process, which matches the same way.
  2. **The process tree**, for an application whose audio runs in a helper with
     a different executable: any session whose process descends from the
     window's process.
  * *UWP/Store apps* hosted by `ApplicationFrameHost.exe`: the visible
    window's pid is the host, not the app. Resolve the real app through its
    core window (the child with a different pid) first. `y_core::process` has
    no such helper today, so it is new code.
* **No audio session until the app plays.** A session appears when the
  application first opens a stream.
  If the pointed application has none, the overlay says so ("no audio") and
  the input is **not** swallowed (a wheel that does nothing must still
  scroll). Falling back to master would be surprising: no fallback. The hook
  must decide before COM can tell, so for the wheel "not swallowed" means
  swallowed and then re-injected, tagged so the hook lets it pass
  (`implementation-plan.md`, decision 4).
* **Per-session mute vs. volume 0.** `ISimpleAudioVolume` has its own mute
  flag, independent of the level. Mute toggle must use it, so that unmuting
  returns to the previous level (VulCan's `getVolume()` reports 0 while
  muted, which is a display choice to keep; the stored level is never
  touched).
* **A step up unmutes, except on the microphone.** Otherwise pressing volume up
  on a muted application appears to do nothing. Windows' own volume keys behave
  this way, and both Windhawk mods do it for any non-zero level. The
  microphone is the exception: only an explicit mute action unmutes it, so a
  stray wheel notch cannot open a live microphone. Reaching 0 % does not mute;
  the two states stay separate, unlike in the mods.

### Keeping up with outside changes

Two things need to know about changes made elsewhere: the panel, which shows
live values, and the user's configured reaction, which works with the panel
closed. Neither needs a hook. Core Audio tells registered clients through COM
callbacks.

**The panel:**

* **While the panel is open: a 1 s timer** re-reads the endpoints and sessions.
  The timer dies with the panel, so "no polling" still holds for the idle app.
* **Session notifications are not used for this.** Microsoft documents that
  `IAudioSessionManager2::RegisterSessionNotification` delivers nothing unless
  COM is initialized MTA on a non-UI thread. Our message-loop thread creates
  windows, so it stays STA, and a dedicated thread is not worth it for a panel
  open a few seconds at a time.

**Watchers, with the panel closed.** `<watch master="off" microphone="hud"/>`
says what to do when that endpoint changes from outside:

| Value | Reaction |
| --- | --- |
| `off` | Nothing, and nothing resident for this endpoint |
| `tray` | Keep the tray icon's state true (the muted variant) |
| `hud` | The same, and show the HUD |

The defaults follow from what Windows already does. It shows a flyout for the
master when a volume key reaches it, so a HUD there would double it unless
`plain="replace"` is set. It shows nothing for the microphone, where a wrong
belief costs the most, and a laptop's mic mute key would otherwise pass unseen.

**Our overlay always**, for the user who wants it, is a matter of configuration:
`plain="replace"` takes the keys, and `master="hud"` catches what they do not
cover, such as the Windows volume center, which shows no flyout of its own
(seen in Phase 0). What is left overlaps: a key the hook cannot see, with an
elevated window in front, and a Bluetooth headset's buttons, which the hook
never sees. Windows shows its flyout, and the watcher adds the HUD if the change
reaches it (to check in Phase 2b).

Reactions only show things for now. What the user can configure per action is
still to be decided. One that writes, such as a volume cap or a held mute, must
pass our event-context GUID, which keeps it from reacting to itself.

* **`IAudioEndpointVolumeCallback`** on each watched endpoint, registered with
  `RegisterControlChangeNotify`. Microsoft documents that it covers level and
  mute, in shared and exclusive mode, and goes to all registered clients.
* **Our own changes are skipped.** Each write passes our event-context GUID,
  and `OnNotify` compares it with `guidEventContext`. The action that made the
  change has already updated the HUD.
* **`IMMNotificationClient`** follows the default device. A watcher is tied to
  one endpoint, so when `OnDefaultDeviceChanged` arrives, the watcher moves.
  It is registered only if at least one watcher is on.
* **The callbacks only post.** Microsoft requires them to be non-blocking, and
  forbids unregistering from inside one or dropping the last reference to an
  audio object there. So they call `PostMessageW` and return, and the message
  loop re-reads the state, shows the HUD and moves the watcher. Several
  notifications in a row just restart the HUD's timer.
* **Teardown order is explicit**: a `Watch` owner unregisters, then releases.
  Unregistering from the callback object's own `Drop` is the leak Microsoft
  warns about, because the endpoint's reference keeps it from ever dropping.
  `IMMNotificationClient` is the reverse: the system holds no reference, so the
  owner must keep it alive until it unregisters.

**Unverified: a hardware change.** Microsoft lists the `IAudioEndpointVolume`
methods as the trigger and does not say whether a hardware button causes a
notification. So whether the laptop's mic mute key fires `OnNotify` is the first
thing Phase 2b checks. If it does not, `GetMute` still shows the true state, but
that key's HUD would need a low-rate timer or be dropped.

**Applications are not watched.** It would take `IAudioSessionEvents` on every
session of every endpoint, plus `RegisterSessionNotification` to learn of new
sessions, which needs the dedicated MTA thread rejected above. Microsoft does
not say either whether the events fire for a change by another client, or
register at all on another process's session. The panel's timer covers
applications while it is open, and nothing else does.

## Input

### Hotkeys

`y_core::hotkey` for `RegisterHotKey`, with the same `steal-from-shell`
mechanism `y-snap` has for combinations Windows owns (VulCan did exactly
this for `Win+<key>`).

Combinations are written as `y_core::hotkey::Hotkey::parse` reads them.
`+` separates the parts, so a key cannot be spelled with one: the keypad keys
are `NumAdd`, `NumSub` and `Numpad0`.

`Hotkey::parse` adds `MOD_NOREPEAT`, but a held volume key has to keep stepping.
`volume-up` and `volume-down` bindings strip it after parsing; mute and panel
bindings keep it, so a held key cannot flap.

### Media keys and the Windows flyout

**The flyout cannot be overridden through any supported API.** There is no
documented setting to disable, move or replace it. The known tools hide the
shell's window from the outside (HideVolumeOSD, ModernFlyouts). That is
undocumented, the hide does not survive a reboot on its own, and reports say it
stopped working on the newer Windows 11 flyout. **y-coustic does not touch the
shell's window.**

**Taking the key instead works.** Confirmed in Phase 0 on the development
laptop (2026-11-02): with the key swallowed and the volume set through
`IAudioEndpointVolume`, no flyout appeared. The flyout appears to come from
the shell handling the volume key, not from the volume changing: the API
documentation mentions only callbacks, and a dial on a USB device has been
reported to change the volume with no flyout. Windhawk's *Taskbar Volume
Control* mod (1.3.1) points the same way: to get the Windows 11 flyout it has to
post `HSHELL_APPCOMMAND` to the taskbar, and with its indicator set to `none` it
only changes the volume through `IAudioEndpointVolume`. So swallowing the key in
`WH_KEYBOARD_LL` and setting the volume ourselves should leave Windows nothing
to show. None of the sources states this outright, which is why Phase 0 tested
it before anything was built on it.

It also fails safely, which the hide does not:

| Situation | What happens |
| --- | --- |
| y-coustic is not running, or crashed | The key reaches Windows: normal volume, normal flyout |
| An elevated window has focus | The hook does not see the key: the same, Windows handles it |
| The keyboard sends something the hook does not see | The same, the key keeps its normal function |
| A Bluetooth headset's volume buttons | The same: Phase 0 logged neither a key nor a shell `WM_APPCOMMAND`, and Windows showed its flyout |

The last two rows are real: reports disagree on whether a low-level hook sees media
keys, because some keyboards send `VK_VOLUME_*` and others go straight to
`WM_APPCOMMAND`. `WM_APPCOMMAND` only reaches the foreground window, and a
global `WH_SHELL` hook would need an injected DLL, which the suite does not do.
Such keys are unsupported, not worked around. The hotkeys and the panel remain.

Three settings, on each `<keys>` group's `plain` attribute and as a tray menu
toggle:

| `plain=` | Bare volume keys | Modifier + volume key | Hook installed |
| --- | --- | --- | --- |
| `windows` (default) | Windows, with its flyout | y-coustic layers | yes |
| `replace` | y-coustic, with its HUD | y-coustic layers | yes |

Removing the `<keys>` group is the third setting: no media keys at all, no hook.
Default to `windows`: replacing a key the user relies on is their decision, not
a side effect of installing the app.

### Layers

A modifier chooses what a set of keys controls. One `<keys>` group plus one
`<layer>` list expands at load into plain bindings, so the same layers work on
media keys and on any other three keys.

| Held | Target |
| --- | --- |
| nothing | master (for media keys, only with `plain="replace"`) |
| `Ctrl` | the focused application |
| `Shift` | the microphone |
| `Ctrl+Shift` | the application under the pointer |

`Alt` is deliberately not a default layer: swallowing `Alt+<key>` leaves the
system seeing a lone Alt tap, which moves focus to the application's menu bar.
`y_core::hotkey::absorb_modifier_tap` is only called for `Win` in the hook path.
A user who configures an `Alt` layer gets that until the hook handles it, so a
warning at load says so. An expansion that collides with another binding is a
non-fatal warning too.

While the panel is open, the bare volume keys act on the **selected row**, so
the media keys and the panel work together. This applies in both `plain` modes.

### Changes needed in `y_core::hotkey`

Layers do not work on today's override path:

* **Lookup is by key only.** `HookState::on_key` takes the first override with a
  matching key, then checks its modifiers, so with `VolumeUp` and
  `Ctrl+VolumeUp` both registered the second can never fire. It must search on
  key *and* held modifiers, and the key-up must find the entry marked `pressed`
  rather than the first with that key. This is a fix for `y-snap` too, and
  takes unit tests.
* **No hook-only route.** `HotkeyManager::register` tries `RegisterHotKey` first
  and uses the hook only if that fails. Media keys need one predictable route,
  so add a way to register straight on the hook. Its behaviour with volume keys
  is unverified, and it would add a second code path to test.

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
  smaller deltas and were silently ignored. y-coustic makes the change
  proportional, `delta × step / 120`, as both Windhawk mods do: a normal notch is
  one step and a fine scroll is a fraction of one, with no accumulator to
  reset. The hook posts deltas and the handler adds up what is queued, so a
  precision touchpad does not cost one COM round per message.
* VulCan swallowed button-down and acted on button-up, but left the
  half-swallowed pair to chance if the modifier was released in between.
  Keep the "swallow the down, consume the up" shape but make the session
  state explicit (`Idle` / `Pressed { button }`) so a stray up is never
  eaten.
* Wheel *left/right* (`WM_MOUSEHWHEEL`) are supported as inputs too, since
  VulCan's own source defines them even though its ini parser never
  accepted them.
* VulCan never dealt with the Alt tap: `Alt+Wheel` swallows the wheel, then the
  Alt release alone focuses the menu bar. `y-snap/src/mouse/modifier_guard.rs`
  solves exactly this. Move what is shared into `y_core` rather than copying it.

The bare wheel is claimed in one case only: over a visible HUD (see Overlays),
through a hook that exists only while the HUD does.

The hook sees nothing while an elevated window has the foreground, the same
limit as for the keys and the one the Windhawk mod documents for its own hook.

**No wheel over taskbar buttons.** Scrolling on a taskbar button to change that
application is the Windhawk per-app mod's feature, but finding out which
application a button stands for takes code injected into `explorer.exe` and
hooks on undocumented, version-dependent taskbar classes. The suite does not
inject, so `pointed` keeps meaning a window, and the taskbar stays refused
by target resolution.

### Target resolution

Reuse `y-snap/src/target.rs`'s idea (pointed → focused fallback, refuse
shell windows). It moves to `y_core::target` in Phase 5 and `y-snap` uses it
from there, so there is one `BANISHED` list.

## Overlays

Two overlays with two jobs, each built on `y_core::overlay::Canvas`. Draw order:
fill panel → text → round corners. Each window is reused (re-drawn and
re-armed), not recreated per event.

They are separate windows because `WS_EX_NOACTIVATE` is fixed per overlay at
creation, and the two need opposite answers: the HUD must never take focus, the
panel must. Both already exist in `y_core`: `OverlaySpec::interactive`,
`focusable`, `Overlay::activate`, `set_click_through` and `show_at_size`, with
the `y-snap` action popup as the precedent.

### HUD

Non-activating. Answers "what did that key just do?".

* Shown on volume/mute change, hidden after `timeout` ms of no further
  change; a new change **restarts** the timer instead of stacking windows.
* The pointer entering it pauses the timer, and it reacts while it is there:
  wheel steps the level, click or drag sets it, clicking the glyph mutes, and
  any other click opens the panel.
* The wheel needs a hook. A window that was never activated only gets wheel
  messages if "Scroll inactive windows" is on, a user setting, so a scoped
  `WH_MOUSE_LL` runs only while the HUD is visible and takes the wheel inside
  its rectangle. Nothing outside the rectangle is touched.
* `interactive="false"` keeps it fully click-through, for anyone who would
  rather never have it intercept a click. It also turns off the hover below,
  which would otherwise offer an overlay that cannot be used.
* Optional fade-in/out (a short `SetTimer` series, no animation thread).
  `Overlay::update` hard-codes `SourceConstantAlpha` to 255, so this needs a
  small `y_core::overlay` addition.
* Placed on the monitor under the pointer for `target="pointed"`, the
  monitor of the focused window otherwise, or a fixed monitor.

### Hovering the tray icon

Resting the pointer on the tray icon for `<tray hover-delay="600"/>` ms shows the
HUD for the master, placed against the icon whatever the HUD's configured
`position`, so the pointer is a short move away from it. There the HUD works as
usual: wheel, click to set, click the glyph to mute, any other click for the
panel. `hover-delay="0"` turns this off. This replaces any wheel on the icon
itself, which `Shell_NotifyIcon` does not report anyway.

It opens the HUD and not the panel because the panel takes focus: an overlay
that steals the foreground window whenever the pointer crosses the taskbar is
worse than no feature. The click stays the way to the panel.

**Detection uses the shell's pop-up messages.** Under version 4, an icon
registered *without* `NIF_SHOWTIP` gets `NIN_POPUPOPEN` when the pointer enters
it and `NIN_POPUPCLOSE` when it leaves. Phase 0 (2026-11-03, Windows 11
26200) saw both, contrary to a 2021 report:

* `NIN_POPUPOPEN` arrives with the first `WM_MOUSEMOVE`, so it marks the entry
  and carries no delay of its own. `hover-delay` counts from it.
* `NIN_POPUPCLOSE` arrives as the pointer leaves, including into a neighbouring
  icon.
* Its `wParam` coordinates were not screen positions (`50,56`, `28,34` for an
  icon at about `1590,1050`), so they are not used. The HUD is placed against
  `TrayIcon::rect()`, which the shell gave every time.
* An icon *with* `NIF_SHOWTIP` gets only `WM_MOUSEMOVE`, many of them, with no
  message for leaving, so it would need polling to tell when the pointer is
  gone.

So: `NIN_POPUPOPEN` arms a one-shot timer of `hover-delay` ms, `NIN_POPUPCLOSE`
kills it, and the timer firing opens the HUD. There is no polling, and the
timer exists only while the pointer is on the icon.

This needs two things in `y_core::tray`: a way to register without
`NIF_SHOWTIP`, and `TrayEvent::PopupOpen` / `PopupClose` (today they fall into
`Other`).

Without `NIF_SHOWTIP` the shell shows no tooltip (confirmed in Phase 0), as
Microsoft documents. So with hover on, the HUD is the icon's tooltip, and it
says more than the text did. With `hover-delay="0"` y-coustic keeps
`NIF_SHOWTIP`: the standard tooltip comes back, and the icon behaves as now.

Unverified: an icon in the overflow area. It was not tried, so hover there may
simply do nothing; the click still works.

### Panel

Focusable, opened on demand. Answers "let me set this up".

Opened by the `panel` action (a hotkey or a mouse binding), by left-clicking the
tray icon, or from the HUD. Opening it again closes it.

```
┌──────────────────────────────────────┐
│ 🔊 Master      ▓▓▓▓▓▓▓▓▓░░░░░░░   60 │
│ 🎤 Microphone  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░   80 │
│ ──────────────────────────────────── │
│ ▌  Firefox     ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░   92 │  ← selected: the focused application
│    Spotify     ▓▓▓▓▓░░░░░░░░░░░   35 │
│ 🔇 Discord     ▓▓▓▓▓▓▓▓░░░░░░░░   50 │  ← muted: level kept, drawn dimmed
└──────────────────────────────────────┘
```

| Input | Effect |
| --- | --- |
| Wheel over a row | Step that row |
| Click or drag on a bar | Set the level |
| Click the icon, or middle-click a row | Mute toggle |
| `Up`, `Down` | Select a row |
| `Left`, `Right` | Step the selected row |
| `Shift+Left`, `Shift+Right` | Step by 1 % |
| `M` or `Space` | Mute toggle |
| `0`–`9` | Set the selected row to 0–90 % |
| `Esc`, or clicking elsewhere | Close |
| `VolumeUp`, `VolumeDown`, `VolumeMute` | Act on the selected row |

Details that have to be right:

* **Capture the target before activating.** Opening the panel changes the
  foreground window, so the focused and pointed applications are read first.
  That application's row, when it has one, is the one selected on opening;
  otherwise it is Master.
* **Keyboard and dismissal come from focus**, as in `y-snap`'s popup: keys
  arrive as `WM_KEYDOWN`, and `WM_KILLFOCUS` means "clicked elsewhere, close".
  No keyboard hook is needed, and it stays open as long as it has focus.
* **Height**: the surface is fixed at creation, so it is created for the maximum
  row count and shown with `show_at_size`. Beyond `max-rows` the list scrolls
  with the selection.
* **Rows**: one row per application, whatever its number of sessions and
  devices, with the rules of Application volume. Inactive sessions keep their
  row, so a paused player can still be adjusted. Expired sessions are dropped. Windows system sounds are a
  last row that `<panel system-sounds="false"/>` hides.
* **Bar drag** uses `SetCapture` and applies the value from `WM_MOUSEMOVE`.
* **Audio objects live with the panel**: they are held while it is open and
  released when it closes, returning to the nothing-resident state.

This also answers the background-player problem (see Open questions): an
application with no window to point at still has a row.

Content of the HUD, per kind:

| Kind | Shows |
| --- | --- |
| master | speaker glyph (with a muted variant), the level bar, the percentage |
| microphone | microphone glyph (with a muted variant), the level bar, the percentage |
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
* **separate styles for master, microphone and application**, so a quick glance
  tells them apart (a different accent colour is the cheap default). This
  matters most for the microphone, where the wrong assumption is the costly one.

The panel shares the colours, font and radius. Styles and positions apply to
the HUD; the panel has its own anchor (`tray`, `cursor`, or the HUD's).

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

	<!-- What to do on a change from outside: off, tray or hud. -->
	<watch master="off" microphone="hud"/>

	<!-- Each <keys> group combines with every <layer>. -->
	<controls>
		<keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="windows"/>
		<keys up="Win+NumAdd" down="Win+NumSub" mute="Win+Numpad0"/>

		<layer mod="none"       target="master"/>
		<layer mod="Ctrl"       target="focused"/>
		<layer mod="Shift"      target="mic"/>
		<layer mod="Ctrl+Shift" target="pointed"/>
	</controls>

	<!-- Single bindings, for whatever the layers do not cover. -->
	<hotkeys>
		<hotkey key="Win+Alt+V"  action="panel"/>
		<hotkey key="Win+Alt+M"  action="mute" target="mic"/>
		<hotkey key="Win+Alt+F9" action="set-volume" target="master" value="30"/>
	</hotkeys>

	<mouse>
		<wheel mod="Alt"       target="master"/>
		<wheel mod="Alt+Shift" target="pointed"/>
		<button button="middle" mod="Alt"       action="mute"/>
		<button button="middle" mod="Alt+Shift" action="mute" target="pointed"/>
	</mouse>

	<!-- Ms the pointer rests on the tray icon before the HUD opens; 0 is off. -->
	<tray hover-delay="600"/>

	<panel anchor="tray" max-rows="8" system-sounds="true"/>

	<overlay theme="auto" style="bar" position="bottom-center" timeout="1200" interactive="true">
		<master/>
		<microphone accent="#D9534F"/>
		<application accent="#E8A33D"/>
	</overlay>
</y-coustic>
```

The defaults use no key a compact keyboard lacks for the main path: `Win+Alt+V`
then the arrows work everywhere, and the wheel works with no keyboard shortcut
at all. The numpad group is VulCan's defaults, kept for those who have one.
The `Win+Alt+…` picks still have to be checked against what Windows owns.
`Win+Ctrl+V` is taken by the Windows 11 sound settings.

Wheel bindings take the direction from the wheel itself: one `<wheel>` element
covers up and down (up raises, down lowers; an `invert="true"` attribute
flips it), instead of VulCan's separate `WHEEL_UP`/`WHEEL_DOWN` entries.

Actions and targets are independent, which replaces the earlier `app-…` action
names and means the microphone needs no action of its own:

| Action | Meaning |
| --- | --- |
| `volume-up`, `volume-down`, `mute` | step or mute the target |
| `volume` | wheel only, the implied action of `<wheel>`: direction from the wheel |
| `set-volume` (`value="30"`) | the target to a fixed level |
| `panel` | open or close the panel; takes no target |

| Target | Meaning |
| --- | --- |
| `master` | the default render endpoint; the default when `target` is omitted |
| `mic` | the default capture endpoint(s) |
| `focused`, `pointed` | an application, as in `y-snap` |

## Application shape

Follows the standard Ypo-Lite layout (`skeleton/README.md`):

```
y-coustic/
  Cargo.toml
  build.rs, rsc/            icon, manifest, version info, default y-coustic.xml
  src/main.rs               App struct, y_core::app::start, message match
  src/config.rs             XML → typed Config (+ warnings), with unit tests
  src/audio.rs              the only COM code: endpoints (render, capture) + sessions
  src/input.rs              mouse hook + hotkey glue (thread_local state)
  src/hud.rs                drawing, timing and scoped wheel hook of the HUD
  src/panel.rs              the panel: rows, selection, hit testing, drawing
  docs/design/y-coustic-plan.md    this file
  docs/design/implementation-plan.md   the order of work and progress
```

The implementation plan refines this list: a `lib.rs` (config and model) as in
`y-snap`, `model.rs` for the pure level rules, and `mouse.rs` for what this
list calls `input.rs`.

`audio.rs` and `config.rs` are testable without a device or a window: the
config has the usual `the_default_configuration_is_valid` test, and the
session rules (executable path, process tree, the mute and level of an
application with several sessions) are written as pure functions over lists of
`(pid, parent_pid, exe)` and session values so they can be tested with fixtures. The audio calls themselves can only be checked on a real machine.
The same goes for the layer expansion (a table of `<keys>` and `<layer>` to
bindings, with collisions) and for the panel's selection, hit testing and row
grouping, which are pure functions over plain data.

## What must be added to the workspace

* `windows` features in the **root** `Cargo.toml` (single source of truth):
  `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`,
  `Win32_System_Com_StructuredStorage` and `Win32_System_Variant` are in,
  added for the Phase 0 probe (`IMMDevice::Activate` needs the last two).
  `Win32_UI_Shell_PropertiesSystem` is still to come if the session display name
  is read from the property store. The watchers implement COM interfaces, so
  they also need whatever `#[implement]` takes in this `windows` version.
  **Checked (Phase 2b, 2026-11-05):** `windows::core::implement` is re-exported,
  but the code it generates names `::windows_core::` directly, so a **direct
  `windows-core = "0.62.2"` dependency** is needed (the version `windows` 0.62.2
  already uses, so nothing new is fetched). It is in the root
  `[workspace.dependencies]`. `PROPERTYKEY`, which `IMMNotificationClient` uses,
  is in `Foundation`, already enabled; no `PropertiesSystem` feature needed.
* `y_core::hotkey`: search overrides by key and modifiers, and a hook-only
  registration (see Changes needed in `y_core::hotkey`).
* `y_core::overlay`: a global alpha for the fade.
* `y_core::tray`: registering without `NIF_SHOWTIP`, and `PopupOpen` /
  `PopupClose` events (see Hovering the tray icon).
* `y_core`: the modifier-tap guard shared with `y-snap`, and the shell-window
  filter if target resolution moves there.

## Phases

The outline. Tasks, files, decisions and status per phase are in
[`implementation-plan.md`](implementation-plan.md), which uses the same numbers.

0. **Prove the media key assumption.** A throwaway build with a `WH_KEYBOARD_LL`
   hook that logs and swallows the three volume keys and sets the master
   through `IAudioEndpointVolume`. This is `y-coustic/examples/phase0.rs`, run
   with `cargo run -p y-coustic --example phase0`; delete it once the answers
   are recorded here. It is a live check on the user's machine, so the user
   runs it. It must answer:
   * Does swallowing a bare volume key suppress the flyout on this Windows 11?
   * Does a programmatic change leave the flyout hidden?
   * Which of the user's devices does the hook see (keyboards, laptop Fn keys,
     dials, headset buttons)?
   * Do `Ctrl` and `Shift` reach the hook with the key on an Fn layer?
   * Does `SetMasterVolume` on another application's render session take
     effect from our own process, as it does from the Windhawk mod in
     `explorer.exe`, despite what the documentation says?
   * During a call, does the application's capture session appear on the
     capture endpoint, and does `SetMasterVolume` on it change what the other
     side hears? For the record only (see Microphone).
   * Does hovering the tray icon deliver `WM_MOUSEMOVE` under version 4 on this
     build, and does leaving out `NIF_SHOWTIP` suppress the tooltip?

   If the first answer is no, `plain="replace"` is dropped and both overlays
   can appear together. The modifier layers are not affected. If writing to
   another application's session fails, application volume has to be rethought
   before Phase 5.

   Results so far (2026-11-02, development laptop):
   * Flyout: none, so `plain="replace"` stays.
   * Sessions are listed with their volumes from our own process, and
     `SetMasterVolume` on another application's session works, despite what
     the documentation says.
   * The Windows volume center changes the master with no flyout either.
   * The laptop's mic mute key flips the endpoint mute and logs no key; its LED
     follows `SetMute` (see Microphone).
   * `Ctrl` and `Shift` reach the hook with a volume key on the Fn layer, so
     the layers work on the laptop keyboard.
   * A Bluetooth headset's volume buttons are not seen at all (see Media keys).
   * Tray hover: `NIN_POPUPOPEN` / `NIN_POPUPCLOSE` arrive without
     `NIF_SHOWTIP` (see Hovering the tray icon).
   * Still open, for the record only: the capture session during a call. The
     probe stays until then.
1. **Master volume, hotkeys only.** The VulCan parity milestone: config
   loader, `audio.rs` endpoint path, step and mute actions, a plain bar HUD,
   tray menu (reload, quit, startup toggle).
2. **Microphone.** The capture data flow, the both-defaults mute, read-back,
   `target="mic"`, the HUD's microphone kind, the tray icon variant.

   2b. **Watchers.** `<watch>`, the endpoint callback, following the default
   device, skipping our own changes, teardown. First checks: do the laptop's
   mic mute key, the Windows volume center and the Bluetooth headset fire
   `OnNotify`?
3. **Media keys and layers.** The `y_core::hotkey` changes, `<controls>`
   expansion, `plain`, the tray toggle.
4. **Mouse.** Wheel with modifiers (accumulated delta) and button chords,
   with the explicit swallow state machine and the Alt tap guard.
5. **Application volume.** Session enumeration on every render endpoint, the
   application as the unit over its sessions, window → application matching,
   the "no audio" feedback, the application HUD.
6. **Panel.** Rows, mouse and keyboard, the live timer, tray left click, volume
   keys on the selected row.
7. **Interactive HUD.** Hover, wheel with its scoped hook, click to set, click
   through to the panel, and opening it by hovering the tray icon.
8. **Overlay customization.** Styles, positions, colours, fade; each kind
   styled independently.
9. **Polish.** A `y-coustic-config` screen in the style of `y-snap-config` /
   `y-pop-config` once the schema is stable.

## Open questions

* **What the user configures per action**: which reactions exist beyond showing
  the state, and how they attach to actions. To decide later.
* **Named targets**: the panel gives a minimized player a row, so the mouse and
  keyboard case is solved. `target="app:spotify.exe"` is still useful for a
  dedicated hotkey; decide once the panel is in use.

## References

* Hiding the flyout, and its limits:
  [HideVolumeOSD on Windows 11](https://answers.microsoft.com/en-us/windows/forum/all/hide-or-move-on-screen-volume-control/53faeb50-d96e-438a-93b8-1f013ec4c63c),
  [ahk-hide-volume-osd](https://github.com/sophice/ahk-hide-volume-osd),
  [SuppressVolumeOSD](https://github.com/cobbertine/SuppressVolumeOSD).
* Swallowing the keys, and which keyboards allow it:
  [PureBasic, volume keys in a low-level hook](https://www.purebasic.fr/english/viewtopic.php?p=535623),
  [Stack Overflow, media keys and `WM_APPCOMMAND`](https://kiwix.lema.org/stackoverflow.com_en_all_2023-11/questions/16509120/setwindowshookex-does-not-react-on-media-keys).
* No flyout from a USB dial:
  [ModernFlyouts issue 4](https://github.com/ModernFlyouts-Community/ModernFlyouts/issues/4).
* Core Audio:
  [`SetMasterVolumeLevelScalar`](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nf-endpointvolume-iaudioendpointvolume-setmastervolumelevelscalar),
  [`RegisterSessionNotification`](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessionmanager2-registersessionnotification),
  [`GetDefaultAudioEndpoint`](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdeviceenumerator-getdefaultaudioendpoint).
* Watchers:
  [`IAudioEndpointVolumeCallback`](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nn-endpointvolume-iaudioendpointvolumecallback),
  [`RegisterControlChangeNotify`](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nf-endpointvolume-iaudioendpointvolume-registercontrolchangenotify),
  [`AUDIO_VOLUME_NOTIFICATION_DATA`](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/ns-endpointvolume-audio_volume_notification_data),
  [`IMMNotificationClient`](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient),
  [`RegisterEndpointNotificationCallback`](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdeviceenumerator-registerendpointnotificationcallback),
  [`IAudioSessionEvents`](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nn-audiopolicy-iaudiosessionevents).
* Sessions and their volume:
  [Audio Sessions](https://learn.microsoft.com/en-us/windows/win32/coreaudio/audio-sessions),
  [Volume Controls](https://learn.microsoft.com/en-us/windows/win32/coreaudio/volume-controls),
  [Grouping Parameters](https://learn.microsoft.com/en-us/windows/win32/coreaudio/grouping-parameters),
  [`GetSessionInstanceIdentifier`](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nf-audiopolicy-iaudiosessioncontrol2-getsessioninstanceidentifier).
* Windhawk mods by m417z, prior art for the wheel, the audio calls and the flyout
  (both read at the versions quoted above, not ported):
  [Taskbar Volume Control](https://github.com/ramensoftware/windhawk-mods/blob/main/mods/taskbar-volume-control.wh.cpp),
  [Taskbar Volume Control Per-App](https://github.com/ramensoftware/windhawk-mods/blob/main/mods/taskbar-volume-control-per-app.wh.cpp).
* Tray hover:
  [`Shell_NotifyIcon`](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw),
  [`NOTIFYICONDATAW`](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw),
  [`NIN_POPUPOPEN` missing on Windows 11](https://learn.microsoft.com/en-us/answers/questions/588939/(windows-11)-vm-popupopen-message-is-not-generated).
