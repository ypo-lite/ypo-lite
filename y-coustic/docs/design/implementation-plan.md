# y-coustic — implementation plan

How to build what [`y-coustic-plan.md`](y-coustic-plan.md) designs, one phase
at a time. The design doc says *what* and *why*; this file says *in which
order, in which files, and when a phase is done*. Phase numbers are the design
doc's (1, 2, 2b, 3 … 9), so its references to "Phase 2b" and the like still
hold.

Each phase ends with something that builds, passes
`cargo clippy --workspace --all-targets` with no warnings and
`cargo test --workspace`, and can be merged on its own. The user commits;
nothing here is committed by the session that does the work.

This file is the running progress record. Update a task's status in the same
change that does the work.

Status: ⬜ todo · 🔨 in progress · ✅ done · 👁 done, verify live (the user
runs it and reports; sessions don't drive the desktop themselves)

## Where we are, and how to resume

| Phase | State |
| --- | --- |
| 0 Probe | ✅ (one check left for the record, see below) |
| 1 Master volume, hotkeys, HUD v1 | ✅ built, live-checked (partly, 2026-11-06) |
| 2 Microphone | 👁 built (2.1–2.6), live checks pending |
| 2b Watchers | 👁 built, live checks pending |
| 3 `y_core::hotkey`, media keys and layers | 👁 built, live checks pending |
| 4 Mouse | ✅ built, live-checked (2026-11-06) |
| 5 Application volume | ✅ built, live-checked (2026-11-06) |
| 6 Panel | ✅ built, live-checked (2026-11-06); tray click debounced, scrollbar added |
| 7 Interactive HUD and tray hover | 👁 built (7.1–7.5), live checks pending |
| 8 Overlay customization | ⬜ |
| 9 `y-coustic-config` | ⬜ (gets its own plan) |

**Resume here:** the look of 8.3 and the fade of 8.4 (`<colors>`, and the mixer sharing font and radius), the live checks of 7b.2 and 7b.3 (the wheel over the icon; `<tray click double-click middle-click>`, with and without a `double-click`), of Phase 7 (hover on the tray icon with the icon on the taskbar; wheel, click, drag and glyph on the bar; a click elsewhere on it opens the panel; `interactive="false"`), then Phase 8, Phase 7b tasks 7b.2 to 7b.4 (tray clicks, wheel on the icon), then 8.3 (8.1 done, 8.2 built: its live check is the five styles, the positions and follow-cursor).

Phase 0's last check (a call application's capture session) is for the record
only and blocks nothing. `y-coustic/examples/phase0.rs` stays until the user
has run it during a call; then record the answer in the design doc's Phase 0
and delete the file (it is auto-discovered, there is no `[[example]]` entry to
remove).

## What exists already (2026-11-04)

Most of the plumbing is in `y_core` or `y-snap`. Reuse, don't copy.

| Need | Where it is | State |
| --- | --- | --- |
| App shape, message loop, single instance | `y_core::app::start`, `TrayApp`; `y-term/src/main.rs` is the minimal shape | ready |
| Tray icon, menu, balloon | `y_core::tray::TrayIcon` (`set_icon`, `rect`, `notify`), `PopupMenu` | ready; hover needs changes (Phase 7) |
| Hotkeys, shell-owned combos | `y_core::hotkey::{Hotkey, HotkeyManager, Fallback}`; `VolumeUp/Down/Mute` already parse | lookup bug and hook-only route (Phase 3) |
| XML config, warnings split | `y_core::xml`; `y-snap/src/config.rs` is the model (`load`, `path()`, `to_xml`, `save`) | ready |
| Layered overlays | `y_core::overlay::{Overlay, OverlaySpec, Canvas, Font, Color}`, `show_at_size`, `activate`, `set_click_through` | ready; global alpha missing (Phase 8) |
| Monitor, DPI | `y_core::monitor::Monitor` (`from_cursor`, `from_window`, `from_foreground`, `scaled`, `keep_inside`) | ready |
| Theme | `y_core::theme::is_dark` | ready |
| Process info | `y_core::process::{window_process_id, executable_path, can_drive}` | parent pid and UWP host missing (Phase 5) |
| App icon and name | `y_core::icon::IconCache::for_file`, `y_core::file_info::file_description` | ready |
| Startup toggle, working-set trim | `y_core::startup`, `y_core::memory::trim_working_set` | ready |
| Target resolution | `y-snap/src/target.rs` (`resolve`, `pointed`, `focused`, `BANISHED`) | moves to `y_core` (Phase 5) |
| Alt-tap guard | `y-snap/src/mouse/modifier_guard.rs` | moves to `y_core` (Phase 4) |
| Core Audio calls | `y-coustic/examples/phase0.rs`: endpoint, mic roles, session listing, set on another process's session | proven; rewrite into `audio.rs`, don't paste |
| `windows` features | root `Cargo.toml`: `Win32_Media_Audio`, `_Endpoints`, `Com_StructuredStorage`, `Variant` are in | ready |

## Decisions

Taken for this plan. **D2, D4 and D5 are worth a second look.**

1. **A `lib.rs` from day one**, as `y-snap` has: `config` (with `to_xml` and
   `save`) and the action/target model are a library, so the tray toggle of
   Phase 3 and the config app of Phase 9 share one `Settings` type. The
   round-trip writer is written with the reader, not retrofitted.
2. **Icons.** The tray needs a normal icon and a "microphone muted" one. Until
   the user supplies `rsc/y-coustic.ico` and `rsc/y-coustic-mic-muted.ico`, the
   app uses `app_icon_or_stock` and a stock icon for the muted variant, so
   nothing blocks on the art. The HUD and panel glyphs (speaker, microphone,
   their muted forms) are drawn on the canvas, not loaded from files.
3. **One COM apartment.** `CoInitializeEx(COINIT_APARTMENTTHREADED)` once in
   `on_create`, on the message-loop thread. Every audio object lives and dies
   on that thread; no `Send` wrappers, no worker thread.
4. **A wheel on a pointed application with no audio is re-injected.** The
   design wants such a wheel to scroll as usual, but the hook has to decide to
   swallow before the handler can know (finding sessions is COM, forbidden in
   the hook). So the hook swallows; when the handler finds no session it sends
   the same wheel back with `SendInput`, tagged in `dwExtraInfo` so the hook
   lets it through. The application sees what it would have seen, a little
   later. The alternative, a cache of executables with sessions, would be stale
   without the session notifications the design rejects.
5. **Target resolution moves to `y_core`** (`y_core::target`), answering the
   design's open question: one `BANISHED` list, `y-snap` switches to it in the
   same change. It takes `y-snap`'s `Target` vocabulary (`pointed`, `focused`).
6. **Wheel deltas are coalesced in the hook's `thread_local!`.** The hook adds
   the delta to a `Cell<i32>` and posts `WM_APP_WHEEL` only when the
   accumulator was zero; the handler takes and zeroes it. No allocation in the
   hook, one COM round per message-loop turn however fast the touchpad sends.
   This works because the hook and the window share a thread.
7. **Hook-only registration is a new `Fallback` variant**, `Fallback::HookOnly`:
   skip `RegisterHotKey`, go straight to the override. One route for media keys,
   and `register` keeps a single entry point.
8. **Tray left click shows the master HUD until Phase 6**, then opens the
   panel, as designed.

## Files

```
y-coustic/
  Cargo.toml                 + [lib], build-dependencies (y-core "build")
  build.rs                   y_core::build::embed_resources
  rsc/                       y-coustic.rc (@VERSION@ tokens), manifest, y-coustic.xml (shipped default)
  src/lib.rs                 pub mod config; pub mod model;
  src/model.rs               Action, Target, Kind (master | mic | app), Level, step rules (pure)
  src/config.rs              XML ↔ Settings + warnings, to_xml/save, layer expansion (pure)
  src/main.rs                App struct, message match, dispatch
  src/audio.rs               the only COM: Endpoint, Applications, watchers
  src/audio/grouping.rs      sessions → applications, window → application (pure, tested)
  src/mouse.rs               WH_MOUSE_LL: wheel and buttons, thread_local state
  src/hud.rs                 HUD overlay: draw, timer, (Phase 7) scoped wheel hook
  src/panel.rs               panel overlay: rows, selection, hit test, draw
  src/panel/layout.rs        selection and hit testing (pure, tested)
  docs/user-manual.md, docs/configuration.md, docs/development.md, README.md
```

---

## Phase 1 — Master volume, hotkeys, HUD v1

VulCan parity: hotkeys step and mute the master, a bar shows the result.

| # | Task | Status |
| --- | --- | --- |
| 1.1 | **Crate shape.** `lib.rs` + `main.rs` on `y_core::app::start`, `build.rs` and `rsc/` copied from `skeleton/` (`.rc` with `@VERSION@`), `#![windows_subsystem = "windows"]`, `AppInfo::new("Y-Coustic", "YpoLite.YCoustic").single_instance()`. Tray icon with Exit only. | 👁 built, clippy clean; run once to see the tray icon (stock icon, no `.ico` yet) |
| 1.2 | **`model.rs`.** `Action { VolumeUp, VolumeDown, Mute, SetVolume(u8), Panel }`, `Target { Master, Mic, Focused, Pointed }`, and the pure level rules: `step(level, notches, step_pct) -> level` (clamped 0..1, fractional notches for Phase 4), `toggle`. Unit tests. | ✅ 9 tests; also `Kind`, `from_percent`/`to_percent` |
| 1.3 | **`config.rs`.** `<volume step>` (clamped 1–30, warning outside), `<hotkeys>` single bindings with `action`, `target`, `value`; `<overlay timeout position>` minimal. Severity split as `y-snap`. `path()` as `y-snap` (argument, portable, `%APPDATA%`). `to_xml`/`save`. Tests: `the_default_configuration_is_valid` against `include_str!("../rsc/y-coustic.xml")`, round trip, unknown action and bad combo are warnings, step clamp. Elements of later phases (`<controls>`, `<mouse>`, `<watch>`, `<panel>`, `<tray>`) are parsed only enough to warn "not supported yet", never as unknown. | ✅ 13 tests. Targets/actions of later phases (`mic`, `focused`, `pointed`, `panel`) are dropped with "not supported yet" via `target_supported`/`action_supported` in `config.rs`: widen them as each phase lands. `to_xml` omits the later sections |
| 1.4 | **`audio.rs`, endpoint path.** `Endpoint::default(flow, role) -> Result<Endpoint>`, `level()`, `set_level()`, `muted()`, `set_muted()`, every write with the app's event-context GUID (one `const GUID`). Re-resolved per action, nothing kept. `Flow` is a parameter from the start (Phase 2 only adds callers). | ✅ `audio.rs` (binary crate, not the lib); `init()` is to be called in `on_create` (1.5); one `#[ignore]` device test; the dead-code allow on `mod audio` is gone (1.5); `Flow::Capture` and `Role::Communications` keep one until Phase 2 |
| 1.5 | **Dispatch.** `WM_HOTKEY` → binding → `Action` on `Target::Master`. `volume-up`/`volume-down` strip `MOD_NOREPEAT` after `Hotkey::parse`; mute keeps it. Step up unmutes the master. `trim_working_set` after each event. | 👁 built, clippy clean; `model::apply` (tested) holds the unmute rules; a missing config file falls back to `config::DEFAULT_XML`; also fixed the tray name, which still said Y-Term |
| 1.6 | **HUD v1 (`hud.rs`).** One `Overlay`, created once, non-activating, click-through. Speaker glyph (muted variant), bar, percentage; colours from `theme::is_dark`. Bottom-centre of the monitor under the cursor. `timeout` timer restarted on each change, never a second window. Draw order fill → text → corners. | 👁 built, clippy clean, 8 tests of the pure geometry. The overlay is rebuilt when the cursor's monitor has another DPI (surface size is fixed at creation); glyph and bar are antialiased shapes painted by `hud.rs`, text by GDI. Shows "Muted" instead of a percentage when muted; no "no output device" message yet (2.3 does it for the mic) |
| 1.7 | **Tray menu.** Reload, Open configuration, Start with Windows, About, Exit; load warnings as one balloon, as `y-snap`. Left click shows the master HUD (D8). | 👁 built, clippy clean. "Open configuration" creates the file from the shipped default when there is none; the About balloon is inline in `main.rs` (version, memory, config path), no separate module; Exit uses `window::quit` as `y-term` does (nothing to restore, unlike `y-snap`) |
| 1.8 | **Docs.** `README.md`, `docs/user-manual.md`, `docs/configuration.md`, `docs/development.md` started, as for `y-snap`/`y-pop`. Note in `hud.rs` that VulCan's balloon is dropped, and why. | ✅ four files written; the note is in `hud.rs` and `development.md`. The main README table is left alone until Phase 1 is verified live |

**Done when** (👁): `Win+NumAdd`/`NumSub`/`Numpad0` step and mute the master;
holding up keeps stepping, holding mute doesn't flap; switching the output
device mid-session drives the new one with no reload; the HUD appears on the
cursor's monitor and a burst of presses keeps one window; Reload picks up an
edited step.

## Phase 2 — Microphone

| # | Task | Status |
| --- | --- | --- |
| 2.1 | `Target::Mic` through the same `Endpoint` on `eCapture`. Mute on `eConsole` **and** `eCommunications` when they differ (compare device ids), read back after writing, report what was read. | 👁 `audio::Microphones` (console + communications when different), `main::apply_to_microphone` returns the read-back state; the mute is checked by every device being muted |
| 2.2 | Rules in `model.rs`: a step never unmutes the microphone; only `Mute` does. Test it. | ✅ `model::apply` takes the `Kind`; 3 more tests (a step and `set-volume` leave a muted microphone muted, zero does not mute) |
| 2.3 | No capture device: the action reports `Unavailable`, the HUD says "No microphone", the key stays consumed. | 👁 `Reading { kind, state: Option<State> }`; the HUD says "No microphone" (and "No audio output" for the master), the key stays consumed |
| 2.4 | HUD microphone kind: glyph and a distinct default accent. | 👁 microphone glyph (slashed when muted), red bar while live |
| 2.5 | Tray icon variant on mic mute (D2), refreshed after our own actions and on start-up. Outside changes come with 2b. | 👁 stock `IDI_ERROR` stands in until `rsc/y-coustic-mic-muted.ico` (resource 102, see the `.rc`) exists; tooltip changes too; refreshed on load and after our own mic actions |
| 2.6 | Docs. | ✅ configuration, user manual, development notes; the default XML gains a commented mic example |

**Done when** (👁): `target="mic"` hotkeys work; with a different default
communication device both get muted; the tray icon follows; with every
microphone disabled the HUD says so.

## Phase 2b — Watchers

| # | Task | Status |
| --- | --- | --- |
| 2b.1 | Check what `#[implement]` needs on `windows` 0.62.2 (`windows::core::implement`, possibly a direct `windows-core` workspace dependency). Record it in the design doc's "What must be added". | ✅ direct `windows-core = "0.62.2"` in the root `[workspace.dependencies]`, recorded in the design doc |
| 2b.2 | `EndpointWatch`: an `IAudioEndpointVolumeCallback` whose `OnNotify` compares `guidEventContext` with ours and otherwise only `PostMessageW(WM_APP_ENDPOINT_CHANGED, flow)`. Owner struct: register on create; on drop **unregister, then release**. | 👁 `audio/watch.rs`; ignored test registers and releases a real watcher |
| 2b.3 | `DeviceWatch`: `IMMNotificationClient`, posts on `OnDefaultDeviceChanged` for the watched flows; the owner keeps it alive until it unregisters. Registered only if a watcher is on. The handler moves the `EndpointWatch`. | 👁 `DeviceWatch` posts for the console role only; `Watchers::rewatch` unregisters the old watch before building the new |
| 2b.4 | `<watch master microphone>` = `off` / `tray` / `hud`, defaults `off` / `hud`. Reaction on the message loop: re-read, update the tray icon, show the HUD (restart its timer). | 👁 `config::Watch`/`WatchMode`, read and written back; master `tray` is treated as `off` (no icon variant), said in the docs; a device change refreshes the tray icon but shows no HUD |
| 2b.5 | Docs, and the results of the live checks into the design doc. | 🔨 docs done; the live checks below are the user's |

**Done when** (👁): with `microphone="hud"`, muting in the Sound control panel
shows the HUD and flips the tray icon; our own hotkey does not show it twice.
Then the design's open checks: does the laptop's mic mute key fire `OnNotify`?
The Windows volume center? A Bluetooth headset's buttons? Unplugging the
default output moves the watcher.

## Phase 3 — `y_core::hotkey`, media keys and layers

| # | Task | Status |
| --- | --- | --- |
| 3.1 | **Fix the override lookup** in `HookState::on_key`: match key *and* held modifiers on the down; on the up, find the entry marked `pressed` for that key. Make it testable by passing the held modifiers in (`GetAsyncKeyState` stays in the caller). Tests: `VolumeUp` and `Ctrl+VolumeUp` both registered, each fires; release after letting go of `Ctrl` first. | ✅ `HookState::decide` (pure) with `Held` passed in; 9 tests in `core/src/hotkey.rs`, including the Ctrl-released-first and mid-repeat cases (a release clears every marked entry of the key) |
| 3.2 | **`Fallback::HookOnly`** (D7). Doc comment on why. | ✅ doc comment says why |
| 3.3 | **Regression on `y-snap`**: its overrides go through the changed lookup. Tests pass; 👁 the stolen `Win+…` hotkeys still work. | 👁 workspace clippy and tests pass; live check of `y-snap`'s stolen `Win+…` hotkeys is yours |
| 3.4 | **`<controls>` expansion** in `config.rs`, pure: each `<keys>` × each `<layer>` → bindings. Warnings: collision with another binding, an `Alt` layer (the menu-bar tap). Table tests. | ✅ `y-coustic/src/controls.rs` (own module, 14 tests); `Settings` keeps `controls` and `hotkeys` as written, `bindings()` is the expansion; collisions are warned at load, first use wins |
| 3.5 | **`plain`**: `windows` registers only the modifier layers of a media `<keys>` group; `replace` adds the bare keys. All through `HookOnly`. No `<keys>` with media keys → no keyboard hook. | 👁 `hook_only` on a `Binding`, `Fallback::HookOnly` in `main.rs`; the default file now has a media group (`plain="windows"`) and a numpad group, layers none → master and Shift → mic |
| 3.6 | **Tray toggle** "Replace the Windows volume indicator": flips `plain` on the media group, saves through `to_xml`, re-registers. | 👁 shown only when the file has a media group; saves through `to_xml` (comments lost, noted in the docs) and reloads |
| 3.7 | Layers with an application target warn "not supported yet" until Phase 5; `focused`/`pointed` targets then just work. Docs. | ✅ a layer on `focused`/`pointed` is skipped once with "not supported yet"; docs written |

**Done when** (👁): with `plain="windows"`, bare volume keys show the Windows
flyout and `Shift+VolumeUp` steps the microphone with our HUD; with `replace`,
no flyout appears and our HUD does; killing the app gives the keys back.

## Phase 4 — Mouse

| # | Task | Status |
| --- | --- | --- |
| 4.1 | **Move `modifier_guard` to `y_core`** (`y_core::modifier_guard`), `y-snap` switches to it. Tests move with it; 👁 `y-snap`'s `Alt+drag` still leaves no menu-bar focus. | 👁 moved to `core/src/modifier_guard.rs`, `y-snap` uses `y_core::modifier_guard`; clippy clean; `y-snap` Alt/Win drag checked live (before the move, re-check once) |
| 4.2 | **`mouse.rs`**: `WH_MOUSE_LL`, `thread_local!` state, installed only if `<mouse>` has bindings. Wheel and `WM_MOUSEHWHEEL` with exact modifiers; bare wheel never claimed (validated at load). Delta coalescing (D6). Injected events pass only when tagged by us (D4); other injected events count. | 👁 `src/mouse.rs` (hook, glue) and `src/input.rs` (pure `Machine`, 16 tests); wheel and tilt, exact modifiers, bare wheel refused at load, delta coalesced per binding. No re-injection tag yet (Phase 5, 5.5): injected events count |
| 4.3 | **Buttons**: `Idle` / `Pressed { button }`, swallow the down, consume the matching up, fire on the up; a stray up is never eaten. Unit-test the state machine. | 👁 per-button press table in `input::Machine`, tested (stray up passes, stale press cleared) |
| 4.4 | **Alt guard** armed whenever a binding with `Alt` swallows. | 👁 every claim arms `y_core::modifier_guard` (Alt and Win) |
| 4.5 | `<mouse>` in config: `<wheel mod target invert>`, `<button button mod action target>`. Proportional change: `delta × step / 120` through `model::step`. | 👁 `<wheel mod target invert axis>`, `<button button mod action target value>`, round-trip; `model::apply_scroll`; default file has Alt layers |
| 4.6 | Docs. | ✅ configuration, user manual, development notes |

**Done when** (👁): `Alt+wheel` steps the master by one step per notch and a
fraction on a touchpad; releasing `Alt` focuses no menu bar; `Alt+middle`
mutes; a plain wheel scrolls everywhere as before.

## Phase 5 — Application volume

| # | Task | Status |
| --- | --- | --- |
| 5.1 | **`y_core::target`** (D5): `resolve`, `pointed`, `focused`, `at`, `BANISHED` moved from `y-snap`, which re-uses it. | ✅ `core/src/target.rs` (with `Target`, moved with it); banishes every `YpoLite.*` class by prefix; y-snap uses it; clippy clean. 👁 y-snap closing/moving by hotkey on focused and pointed still works |
| 5.2 | **`y_core::process`**: `parent_pids()` (one Toolhelp32 snapshot → `pid → parent`), and `uwp_app_pid(window)` (the `ApplicationFrameHost` child core window with a different pid). | ✅ `y_core::process::{parent_pids, uwp_app_pid}`; the `Win32_System_Diagnostics_ToolHelp` feature added to the root `Cargo.toml`. 👁 `uwp_app_pid` on a Store app (Calculator, Spotify store build) |
| 5.3 | **`audio.rs` sessions**: every active render endpoint → `IAudioSessionManager2` → sessions → `SessionInfo { pid, exe, is_system, display_name, level, muted, expired }` plus the `ISimpleAudioVolume` to write through. Held only for one action. | 👁 `audio/sessions.rs` (`Sessions::snapshot`, `write`); a session of no single process is told by pid 0, not by its success code |
| 5.4 | **`grouping.rs`** (pure): sessions → `Application` by executable path; system sounds one row; no-single-process sessions their own row; window pid → application (same exe, then descendants via `parent_pids`). Aggregates: level is the max, muted only if all muted. Operations: step each, set each, mute all if any unmuted. Fixture tests, including the Chrome audio child and the two-endpoint Terminal from the design. | ✅ `audio/grouping.rs`, 16 tests, incl. the Chrome helper, descent, pid-loop guard and the two-endpoint Terminal |
| 5.5 | **Dispatch** for `focused`/`pointed` from hotkeys, layers and the wheel. No session: HUD "No audio" for hotkeys; re-inject the wheel (D4). Step up unmutes the application. | 👁 `apply_to_application` in `main.rs`; hotkeys, layers, buttons and wheel; `mouse::reinject` tagged `REINJECTED`; `config::target_supported` removed. Default file gains the `Ctrl` and `Ctrl+Shift` layers and `Ctrl+Shift` wheel/middle |
| 5.6 | **HUD application kind**: the exe's icon (`IconCache::for_file`), name from `file_description`, falling back to the exe stem. | 👁 `Reading::application`, `Hud::look` caches name and icon; a two-row bar for an application |
| 5.7 | Docs. | ✅ configuration, user manual, development notes |

**Done when** (👁): `Ctrl+Shift+wheel` over Chrome, Firefox and a Store app
changes only that application in the Windows mixer, on whichever device it
plays; over a window with no audio the page scrolls; an app routed to the
second output device is reached.

## Phase 6 — Panel

| # | Task | Status |
| --- | --- | --- |
| 6.1 | `panel.rs`: focusable overlay created for `max-rows`, shown with `show_at_size`. Rows: Master, Microphone, separator, applications, system sounds (`<panel system-sounds>`). | 👁 `src/panel.rs` (focusable overlay per opening, created for `max-rows`, shown at the height in use; the master, the microphone, applications, system sounds last) |
| 6.2 | `layout.rs` (pure): row rects, hit test (glyph, bar, row), selection moves and scrolling past `max-rows`. Tests. | ✅ `src/panel/layout.rs`, 12 tests |
| 6.3 | Opening: read the focused and pointed applications **before** `activate`; select that application's row, else Master. Toggle on a second `panel` action. Tray left click now opens it (D8). | 👁 `mixer::row_of_window` (focused, else pointed) read before opening; tray left click and the `panel` action toggle; a tray click right after a focus-loss close is ignored |
| 6.4 | Input: keys from `WM_KEYDOWN` per the design's table, wheel per row, click/drag with `SetCapture`, middle-click or glyph to mute; `WM_KILLFOCUS` and `Esc` close. | 👁 keys, `WM_APPCOMMAND` volume keys, wheel, click and drag with `SetCapture`, middle click, `WM_KILLFOCUS`; focus-refused fallback as in y-pop |
| 6.5 | Volume hotkeys and layers act on the selected row while the panel is open. | 👁 `run_binding`: `volume-up`/`-down`/`mute` from a hotkey, layer or mouse button go to the selected row while open |
| 6.6 | 1 s refresh timer, alive only with the panel; audio objects released on close, then `trim_working_set`. | 👁 1 s `WM_TIMER`, held while a bar is dragged; close drops everything and trims the working set. Simplification: rows are re-read per refresh, not kept as live audio objects |
| 6.7 | `<panel anchor max-rows system-sounds>`. Docs. | ✅ `<panel anchor max-rows system-sounds>`, `panel` as an action everywhere; the default file has `Win+Alt+V`; docs |

**Done when** (👁): `Win+Alt+V`, arrows and digits adjust everything without
the mouse; a paused background player has a row; clicking elsewhere closes it
and the working set goes back down.

## Phase 7 — Interactive HUD and tray hover

| # | Task | Status |
| --- | --- | --- |
| 7.1 | `y_core::tray`: an option to register without `NIF_SHOWTIP`; `TrayEvent::PopupOpen` / `PopupClose` decoded (and the decode test updated). | ✅ `TrayIcon::set_standard_tooltip(show)` (toggles `NIF_SHOWTIP`, kept across `restore`), `TrayEvent::PopupOpen`/`PopupClose`, decode test extended |
| 7.2 | `<tray hover-delay>`: `PopupOpen` arms a one-shot timer, `PopupClose` kills it, firing shows the master HUD against `TrayIcon::rect()`. `0` keeps `NIF_SHOWTIP` and the tooltip. | 👁 `<tray hover-delay>` (0 to 5000, default 600, 0 = off and keeps the tooltip); `PopupOpen` arms `HOVER_TIMER_ID`, `PopupClose` kills it, firing shows the master HUD through `Hud::show_against(rect)`; falls back to the configured place when the shell gives no rect; docs. The module doc and `Sections for later` are gone, no section is reserved now; held while the pointer stays on the icon (`Hud::holding`, `release` on `PopupClose`) |
| 7.3 | HUD `interactive="true"`: no longer click-through; pointer inside pauses the timer (`TrackMouseEvent` for leave); click/drag sets, glyph mutes, other click opens the panel. | 👁 `<overlay interactive>` (default true), `hud/forward.rs` handler, `Geometry::hit`, `Hud::on_message`/`redraw`, `Reading::row`; clicks act on what the bar shows; hover pauses the timer; the bar rebuilds when `interactive` changes |
| 7.4 | Scoped `WH_MOUSE_LL` while the HUD is visible, taking only the wheel inside its rect. Removed when the HUD hides. | 👁 `WheelHook` installed by `try_show`, dropped by `hide`; bare wheel inside the rect only; `interactive="false"` also turns the tray hover off |
| 7.5 | Docs. | ✅ configuration, user manual, development notes |

**Done when** (👁): resting on the tray icon brings the HUD next to it; wheel
over it adjusts the master; moving away lets it time out; with
`interactive="false"` it never catches a click and hover is off.

## Phase 7b — Naming, and the tray icon's actions

Done: the vocabulary is **HUD** (the small window: on a change, on the tray
hover) and **mixer** (the window listing every volume). Settings: `<hud>` (was
`<overlay>`), `<mixer>` (was `<panel>`), action `mixer` (was `panel`); no
compatibility with the old names, nothing being released. In the source the
mixer window is still `panel.rs` and `Action::Mixer`/`MixerConfig`/`HudConfig`
carry the new words; `mixer.rs` is the audio side.

| # | Task | Status |
| --- | --- | --- |
| 7b.1 | The rename above, tests, default file, example, docs. | ✅ |
| 7b.2 | `<tray click double-click middle-click>`: `mixer`, `hud`, `mute`, `mute-mic`, `volume-up`, `volume-down`, `none`. With a `double-click` set, the single click waits the system double-click time; otherwise it is immediate. Reworks `TRAY_DEBOUNCE`. | 👁 `TrayAction` in `config.rs`, `TrayEvent::MiddleClick` in `y_core::tray`, `on_tray_select` / `on_click_timer` in `main.rs`; the debounce stays for the no-double-click case |
| 7b.3 | `<tray wheel>`: the scoped `WheelHook` installed on `PopupOpen` with the icon's rectangle kept from then, dropped on `PopupClose`; steps the master and shows the HUD. Needs the hover (icon on the taskbar, `hover-delay` above 0). | 👁 `WheelHook::install_tray` (a second, independent scoped hook in `hud/forward.rs`, posts `WM_TRAY_WHEEL`), `<tray wheel>` (default true), `on_tray_wheel` in `main.rs` |
| 7b.4 | Docs. | ✅ with 7b.2 and 7b.3 |
| 7c | Tray icon rules. Done: the icon is fixed by default; `<tray icon>` picks a rule (`fixed` or `mic-muted`); `appearance.rs` (pure `Rule`, `Inputs`, `Appearance`, `decide`, tested) is where further rules (by master or microphone level) are added, `refresh_tray_icon` only reads what a rule needs. Not done: level-based rules, per-level icons. | 👁 |

## Phase 8 — Overlay customization

| # | Task | Status |
| --- | --- | --- |
| 8.1 | `y_core::overlay`: a global alpha on `Overlay::update` (today `SourceConstantAlpha` is hard-coded to 255). Done: `Overlay::set_opacity(u8)` (persists, applied by the next show or refresh) and `alpha_of_percent`; the default stays 255, so no existing overlay changes. | ✅ |
| 8.2 | Styles `bar`, `vbar`, `ring`, `text`, `compact`; positions and `follow-cursor`; size, radius, border, opacity, font. Done, 👁: `hud/layout.rs` (pure `Geometry` per style, `place` for nine positions, `follow`, 11 tests); `config::Overlay` gains `style`, `scale`, `radius`, `border`, `opacity`, `font` and 10 positions; the window is rebuilt for a new style, scale, font or `interactive`, the rest is only drawn; border uses `palette.track` until 8.3. | 👁 |
| 8.3 | Per-kind `<master>`, `<microphone>`, `<application>` with colours and accent; the panel shares colours, font, radius. | 👁 `<colors>` (panel, text, track, accent, muted; `<master>`, `<microphone>`, `<application>` children) in `HudConfig.colors`; `Palette::current(kind, muted, &Colors)`; the mixer takes colours, font and radius from the HUD's settings (`OpenWith::look`); a general accent spares the microphone's red |
| 8.4 | Fade in/out as a short `SetTimer` series. | 👁 `<hud fade>` (0 to 1000 ms, default 0 = off); `Fade` and the pure `fade_alpha` in `hud.rs`, a 16 ms `FADE_TIMER_ID` alive only during a fade, `Overlay::set_opacity` per step; a change during a fade out resumes from the current opacity, the pointer on the HUD cancels a fade out |
| 8.5 | Docs, with screenshots. | ⬜ |

## Phase 9 — `y-coustic-config`

A settings app on `ui-kit`, in the style of `y-snap-config` / `y-pop-config`,
once the schema has held still through Phase 8. It gets its own plan; the
`lib.rs` of D1 is what it builds on. An `ipc` reload message, as `y-snap`'s,
comes with it.

## Risks

* **The override lookup fix touches `y-snap`'s hotkeys** (3.1). Covered by
  tests and a live check, and done before anything in y-coustic depends on it.
* **Re-injecting a wheel** (D4) could loop if the tag check is wrong, or land
  on a different window if the pointer moved. The tag is checked first in the
  hook; the injected wheel goes to the original point.
* **`#[implement]` on 0.62** (2b.1) is unverified; if it needs a new
  dependency, that goes in the root `Cargo.toml`.
* **Hardware mic mute** may not fire `OnNotify` (2b). Fallback per the design:
  a low-rate timer only while `microphone="hud"`, or drop the HUD for that key.

## Progress

| Phase | Status | Notes |
| --- | --- | --- |
| 0. Probe | ✅ | Results in the design doc; capture session during a call still open, for the record |
