# y-snap — development

How y-snap is built. To use it, see the [user manual](user-manual.md) and the
[configuration reference](configuration.md). Building the workspace is on the
[main page](../../README.md#build-from-source).

```sh
cargo run -p y-snap      # debug run
cargo test -p y-snap
cargo test -p y-snap config::tests::the_default_configuration_is_valid  # one test
```

## Layout

| | |
| --- | --- |
| `main.rs` | the tray app: the message loop, hotkey registration, dispatch |
| `hidden.rs` | the windows `iconize`/`traynize` hid: the registry, the tray icons, the purge |
| `about.rs` | the text of the *About* balloon, built to the balloon's size limit |
| `action.rs` | the action vocabulary, shared by hotkeys and the overlay |
| `config.rs` | reading `y-snap.xml` |
| `ipc.rs` | what another process can ask of a running y-snap |
| `actions.rs` | action → Win32 call, in one place |
| `y_core::target` | choosing the window, and refusing the shell's (moved to `y-core`, shared with y-coustic) |
| `window.rs` | what gets done to a window, and how to undo it |
| `mouse/` | the low-level mouse hook: drag, resize (`gesture`), minimize-button hover (`hover`), mouse shortcuts (`shortcut`, a pure state machine) |
| `min_overlay.rs` | the minimize-button popup, and turning a click into an action |
| `overlay.rs` | the action overlay: tile grid, hover, click, key, focus-driven close |

`action`, `config` and `ipc` are the `y_snap` library, so `y-snap-config` reads
and writes the same `Settings` without linking the message loop or the hooks. The
rest is the binary. Everything Win32 that is not specific to window management
(the hidden message window, the tray icon, menus, hotkey registration, the XML
reader, the overlay surface) is in [`y-core`](../../core/README.md).

## How it works

**The mouse hook.** Drag, resize, the minimize button and mouse shortcuts share
one `WH_MOUSE_LL` hook, installed on y-snap's own thread. HulK needed a DLL
injected into other processes for this; a low-level hook does not. Windows
silently removes a hook slower than `LowLevelHooksTimeout` (300 ms), so the hook
does no drawing or window moves itself: it posts them to the owner window. A
timer checks that the hook is still alive and reinstalls it if not. See the
module doc of `mouse/mod.rs`.

**Maximized windows and monitors.** With `allow-maximized` off, a press on a
maximized window starts a `Pinned` gesture (`mouse/gesture.rs`): claimed, with the
modifier guard armed, but the window does not follow. A pinned drag only watches
`handle_at(pt)` (a bare `MonitorFromPoint`, hook-safe) and, when the monitor
changes, posts `WM_GESTURE_MONITOR` with the point; the move runs on the message
loop (`apply_pending_monitor`, latest wins, a no-op if the window is already
there). `window::move_to_monitor` does the work for the four `monitor-*` actions
too. A maximized window is carried by `move_maximized`: `SetWindowPos` while still
maximized, then `SetWindowPlacement` to record where it restores to. It was chosen
by a probe over restore-then-maximize variants (B, C), which landed right but
showed the restore/maximize animation, and over a single placement call, which
failed. `rcNormalPosition` is in workspace coordinates, shifted by a taskbar on the
top or left, except for tool windows (what `hide-taskbar` makes). Design notes and
results: [`design/multi-monitor-plan.md`](design/multi-monitor-plan.md).

**The minimize button.** Its position is estimated from the window's frame and
the caption metrics (`mouse::hover::min_button_rect`), because hit-testing another
process's window from inside a low-level hook risks that same timeout. Detection
uses a looser rectangle, and one bounded check against the real window confirms
it before the popup shows.

**Choosing the window.** There is no Win32 flag meaning "not a real application
window", so `y_core::target` (`core/src/target.rs`) refuses the shell's windows by class name. Extend its
`BANISHED` list by observation, erring toward refusing.

**Virtual desktops.** Windows has no public interface for moving a window between
desktops. [`y-core`'s `virtual_desktop` module](../../core/src/virtual_desktop.rs)
wraps the undocumented COM interface and handles the differences between Windows
versions.

**Hidden windows.** A window is minimized, then hidden, so a maximized one comes
back maximized. A purge runs every five seconds, only while something is hidden.
If y-snap is killed, hidden windows stay hidden;
[`design/iconize-traynize-plan.md`](design/iconize-traynize-plan.md) describes the
opt-in cache file planned for that.

**Talking to the running app.** `y-snap-config` asks for a reload with a registered
window message broadcast to top-level windows, which is why y-snap's main window
is a hidden top-level window rather than a message-only one. See `ipc.rs`.
