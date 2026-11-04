# y-coustic — development

How y-coustic is built. To use it, see the [user manual](user-manual.md) and the
[configuration reference](configuration.md). Building the workspace is on the
[main page](../../README.md#build-from-source). What is built, and what comes
next, is in the [implementation plan](design/implementation-plan.md); why, in the
[design](design/y-coustic-plan.md).

```sh
cargo run -p y-coustic                       # debug run
cargo test -p y-coustic
cargo test -p y-coustic -- --ignored         # also the test that needs an output device
```

## Layout

| | |
| --- | --- |
| `src/main.rs` | the tray app: message handling, load and reload, dispatch of a hotkey, the menu |
| `src/audio.rs` | the only COM: an endpoint's level and mute, and the pair of microphones |
| `src/hud.rs` | the HUD: one overlay, its drawing, its timer, an application's icon and name |
| `src/look.rs` | an application's name and icon, resolved from its executable (HUD and mixer) |
| `src/mixer.rs` | (the audio side) what a change does to the audio, and the rows of the mixer: the bridge between the pure rules and COM |
| `src/panel.rs` | the mixer window: input, drawing, the requests it sends the owner |
| `src/panel/layout.rs` | where the mixer's rows are and what a point on it is: pure |
| `src/audio/sessions.rs` | the COM of the applications: every output device's sessions, read and written |
| `src/audio/grouping.rs` | sessions to applications, window to application, what to write: pure |
| `src/mouse.rs` | the mouse hook: swallow or pass, accumulate wheel delta, post to the loop |
| `src/input.rs` | mouse inputs, bindings and the hook's decision machine: pure |
| `src/lib.rs` | the library the settings app will share |
| `src/model.rs` | actions, targets, and the level arithmetic: pure |
| `src/controls.rs` | `<controls>`: key groups, layers, and their expansion into bindings: pure |
| `src/config.rs` | reading and writing `y-coustic.xml`: pure but for the file |
| `rsc/` | the shipped default `y-coustic.xml`, the manifest and the version resource |

The binary is `main`, `audio`, `hud` and `mouse`. `model`, `controls`, `input` and `config` are the `y_coustic`
library, free of Win32 windows, hooks and COM, so a settings app can link them
and so they test without a desktop. The Win32 plumbing (message window, tray,
hotkeys, overlay surface, XML) is in [`y-core`](../../core/README.md).

## How it works

**Audio.** `audio::Endpoint` resolves the default device for a flow and role on
every action and drops it afterwards: nothing is cached, so a changed default
device needs no notification code. Everything runs in one single-threaded COM
apartment, entered in `on_create`. Every write carries `EVENT_CONTEXT`, a GUID of
our own, so the watchers can tell our changes from outside ones.

**Dispatch.** A hotkey id is the binding's index plus `IDH_FIRST`. The handler
looks the binding up (a reload may have shortened the list since the message was
posted), reads the device, applies `model::apply` and writes back only what
changed. `model::apply` holds the rules: raising the level unmutes, lowering does
not.

**Microphone.** `audio::Microphones` holds the default capture device and, when it
is another device, the default communications one. `state()` reports the console
level and "muted" only if every device is; `set_muted` writes to all. The
dispatch writes, then reads the state back and returns that, so the HUD shows
what the devices say. `model::apply` takes the `Kind` because the rule differs:
only `Mute` unmutes a microphone. The tray icon is refreshed after our own
microphone actions and on load, only when the state changed; outside changes arrive through the watchers
(below).

**Watchers.** `audio/watch.rs` has two COM callbacks, each reduced to a
`PostMessageW` to the owner window with the flow as `wParam`: Core Audio calls
them from a thread of its own, so they hold only a window handle and a flow, and
the reaction runs on the message loop. `EndpointWatch` is an
`IAudioEndpointVolumeCallback` on a default endpoint; its `OnNotify` ignores
events carrying our `EVENT_CONTEXT`, so our own writes post nothing.
`DeviceWatch` is an `IMMNotificationClient` whose `OnDefaultDeviceChanged`
(console role) moves the `EndpointWatch` onto the new device. Both owners
unregister first and release second, in `Drop`. `Watchers` is rebuilt on every
load and holds nothing for a flow set to `off`, not even the device client.
`#[implement]` needs `windows-core` as a direct dependency (the macro emits
`::windows_core::` paths), at the version `windows` re-exports, pinned once in
the root `Cargo.toml`. The messages are in the application range, from
`WM_APP_FIRST`.

**Keys and layers.** `controls::expand` crosses every `<keys>` group with every
`<layer>` into `Binding`s, after the single `<hotkey>`s, and reports collisions,
`Alt` layers and layers on targets not supported yet. `Settings` keeps the groups
and layers as written (`controls`) and the singles as written (`hotkeys`);
`Settings::bindings()` is the expansion, so `to_xml` writes back what the user
wrote and a layer stays one layer. A hotkey id is an index into the expanded list
(`YCoustic::bindings`) plus `IDH_FIRST`. A binding on a media key has `hook_only`
and registers with `Fallback::HookOnly`; with `plain="windows"` the expansion
leaves the bare media keys out, so they stay Windows'.

**The hook lookup.** `y_core::hotkey`'s hook state matches a press on the key
**and** the held modifiers, and a release on the entries marked `pressed` for the
key, clearing all of them. Before, the first entry for a key was taken whatever
the modifiers, so `VolumeUp` and `Ctrl+VolumeUp` could not both be registered
(a fix for `y-snap`'s overrides too). The decision is `HookState::decide`, pure
but for the `pressed` marks, with the held modifiers passed in (`Held`), which is
what the tests drive.

**Auto-repeat.** `Hotkey::parse` sets `MOD_NOREPEAT`. For the step actions it is
stripped before registering, so holding the key keeps stepping.

**The HUD.** One click-through, non-activating layered window. Its surface is
fixed at creation and its size depends on the monitor's DPI, so it is rebuilt
when the pointer moves to a monitor with another DPI. The mixer is filled and the
text drawn by GDI first, the glyph and bar are painted after as antialiased
shapes (4x4 supersampling), and the corners are rounded last. VulCan's balloon
notification is dropped: it is slow, queued, and absent under Focus Assist.

**The mouse.** `<mouse>` is read into `config::Settings::mouse`; `input.rs` (in
the library, pure) holds `Input`, `MouseBinding` and `Machine`, the decisions of
the hook, which the tests drive with the modifiers passed in. `src/mouse.rs` is
the glue: one `WH_MOUSE_LL` hook, installed only when there are bindings, state
in a `thread_local!` (never a `Mutex`: the hook only runs on its own thread).
The hook decides swallow or pass, reads the modifiers with `GetAsyncKeyState`,
and posts `WM_MOUSE_WHEEL` / `WM_MOUSE_ACTION` with the binding's index; COM and
the HUD run on the message loop. A wheel's delta is added to a per-binding
accumulator and the message is posted only when it was zero, so a precision
touchpad costs one COM round per loop turn (`mouse::take_wheel` empties it,
`input::notches` turns it into steps, `model::apply_scroll` applies the same
unmute rules as a key). A button is `Idle`/`Pressed` per button: the press is
swallowed, the binding picked then, and the release fires it. Every claim arms
`y_core::modifier_guard` (moved there from `y-snap`) so releasing `Alt` or `Win`
alone opens no menu. Simplifications, also noted in `mouse.rs`: no watchdog that
reinstalls a dropped hook (the hook does no work on a move); injected events
count like real ones, except the wheel y-coustic re-injects itself (see Applications).

**Applications.** `Target::Focused`/`Pointed` go through `mixer::apply_to_application`
in `main.rs`: the window (`y_core::target`, shell and Ypo-Lite windows refused),
its process (`y_core::process::window_process_id`, replaced by
`uwp_app_pid` for an `ApplicationFrameHost` frame), then `audio::sessions`, a
snapshot of every active render endpoint's sessions read into plain
`SessionInfo`s with their `ISimpleAudioVolume` kept beside (taken per action,
dropped after: nothing is cached). `audio/grouping.rs` is pure and tested:
`group` folds sessions into applications by executable path (system sounds one
row, no-single-process sessions one row each), `for_window` finds the
application of a window by executable, then by descent in `parent_pids()`
(computed only if the path matched nothing, and bounded against pid reuse),
and `apply` says what to write to each session (each steps on its own, a mute
is one decision for all). `model::Change` is the action-or-notches value all of
it shares. The wheel's no-audio case is `mouse::reinject`: the hook swallowed
before COM could say, so the same delta is sent back tagged in `dwExtraInfo`
(`REINJECTED`), which the hook passes. A session's `AUDCLNT_S_NO_SINGLE_PROCESS`
success code is not distinguished from pid 0, which `group` treats the same.
The HUD names the application from its executable's file description and
draws its icon, both cached in `Hud::look` between two readings of the same
application.

**The mixer.** (The mixer window is `panel.rs` in the source, a name from before the rename; `mixer.rs` is the audio side.) `panel.rs` is the view and the input, `mixer.rs` the audio, and the
two never meet: the window keeps the rows it was given (`mixer::Row`, plain data)
and returns what it wants as `Request::Change { row, change }` from
`Panel::on_message`; `main.rs` carries it out with `mixer::apply_row` and gives the
rows back (`Panel::update`). `mixer.rs` also owns what the bindings use
(`apply_to_master`, `apply_to_microphone`, `apply_to_application`) and
`row_of_window`, read before the mixer takes the focus, which is what it selects
first. Where a row is, and what a point on it means (icon, bar with its level,
row), is `panel/layout.rs`: pure, with the separator after the microphone, the
scroll window and the selection moves tested. The window is a focusable
`Overlay` created per opening for `max-rows` and shown at the height in use; its
handler (`Forwarder`) only posts `WM_PANEL_*` to the owner, each carrying the
opening's generation, as `y-pop`'s search window does. Dismissal is
`WM_KILLFOCUS`; when the system refuses the foreground the mouse is captured until
a click takes the focus or closes. A left press captures the mouse for a bar drag.
`WM_APPCOMMAND` volume commands become the same key message, so the volume keys
act on the selected row whether y-coustic holds them or not; with the mixer open
`run_binding` sends a `volume-up`, `volume-down` or `mute` binding to the selected
row instead of its own target. A tray click that has just taken the focus from the
mixer is not a click to open it again (`Panel::just_lost_focus`). Names and icons
are `look::Look`, cached per executable while the mixer is open (the HUD shares the
type). The owner's 1 s `WM_TIMER` re-reads the rows, except while a bar is dragged;
closing drops the window, the rows and the cache, then trims the working set.

The list starts at the top (the row to select is taken only if it is shown there). A
list with more rows than `max-rows` has a scrollbar: `Layout` computes the track,
the thumb, the hit and the drag mapping (pure, tested) and keeps one window height
whatever the scroll, so the track does not move under a dragged thumb; `Drag` holds
either a bar or the thumb. A refresh never scrolls back to the selection, only a
move of the selection does.

A mixer opened by a click on a tray icon in the overflow flyout is handed the focus
back by the flyout as it closes: a `WM_KILLFOCUS` within 800 ms of opening takes the
focus again (up to three times) instead of closing, or the mixer flashes and goes.

**Tray hover.** `y_core::tray` reports `NIN_POPUPOPEN` and `NIN_POPUPCLOSE` as
`TrayEvent::PopupOpen` / `PopupClose` for an icon registered without `NIF_SHOWTIP`
(`TrayIcon::set_standard_tooltip(false)`, which `load` applies when
`<tray hover-delay>` is above zero). Open arms a one-shot `WM_TIMER`
(`HOVER_TIMER_ID`) of `hover-delay`; close kills it; firing shows the master through
`Hud::show_against`, placed against `TrayIcon::rect()` (centred on the icon, above for
a bottom taskbar, below otherwise), or where the settings say when the shell gives no
rectangle. The popup messages' own coordinates are not screen positions and are not
used. No polling, and no timer while the pointer is elsewhere. The HUD shown that way is
*held* (`Hud::show_against` sets it): its timer restarts instead of hiding until
`PopupClose` calls `Hud::release`, since the shell sends the enter message once per
entry and nothing would bring the HUD back after a timeout. Any other showing ends the
hold, and opening the mixer hides the HUD. Not while the mixer is
open.

**The interactive HUD.** The HUD overlay is created click-through, or, with
`<hud interactive>`, as an interactive one whose handler (`hud/forward.rs`,
`Forwarder`) posts `WM_HUD_*` to the owner, as the mixer does. `Hud::on_message`
turns them into a `hud::Request` for `main.rs` to carry out through
`mixer::apply_row`, and `Hud::redraw` draws the result in place. What a click is
on is `Geometry::hit`, the same numbers the drawing uses (`Geometry` is pure and
tested): the glyph with the gap after it is a mute, the bar is a level (the whole
height of its row is the target, and a drag clamps), the rest is the mixer, opened
on release. The reading carries its `RowId` (`Reading::row`), so the bar acts on
what it shows. The pointer on the bar pauses the timer (`TrackMouseEvent` for the
leave, `WM_MOUSELEAVE` bound by value since the crate does not); a showing while
the pointer is on it leaves the timer paused. The wheel is `WheelHook`, a
`WH_MOUSE_LL` installed by `try_show` for an interactive bar and dropped by `hide`
(one hook for as long as the bar is up, never resident): it takes only a bare
wheel inside the rectangle of the bar, passes a wheel with a modifier (a binding)
and the wheel `y-coustic` re-injects, and coalesces the delta like the bindings
do. The window is rebuilt when `interactive` or the DPI changes: both are fixed at
creation.

**Configuration.** Follows `y-snap`: a malformed document aborts, everything else
is a warning. `config::target_supported` and `config::action_supported` say what
works today and are widened as each phase lands. With no file, `DEFAULT_XML` (the
shipped `rsc/y-coustic.xml`, embedded) is used.

## Icons

There is no `y-coustic.ico` yet, and the application uses the generic system icon.
When one is supplied, add it to `rsc/y-coustic.rc` (the comment there says how).
The speaker and microphone glyphs of the bar are drawn, not loaded.
