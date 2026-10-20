# y-core

What every Ypo-Lite application would otherwise write again: the `skeleton/`
reference application, made reusable. The `unsafe` and the Win32 plumbing live
here; the applications stay plain Rust with a `match` on messages.

```rust
use y_core::prelude::*;

const APP: AppInfo = AppInfo::new("Y-Snap", "YpoLite.YSnap").single_instance();

fn main() -> ! {
    y_core::app::start(&APP, || YSnap::new().into_handler(&APP))
}
```

`app::start` owns the boring half: the single-instance mutex, the hidden
message window, the start-up error box, the working-set trim, the message
loop. Everything else is picked à la carte.

| Module | Role |
| ------ | ---- |
| `app` | start-up, and the `TrayApp` trait that handles what every tray app handles the same way |
| `window` | window classes, the window procedure, the message loop |
| `tray` | notification area icon, RAII; balloons through a `Notification` builder (custom icon, silent, quiet time) with click events — see `docs/notifications.md` |
| `process` | the process behind a window: its name, and whether UIPI lets us drive it |
| `menu` | popup menus — items, check marks, submenus — RAII |
| `hotkey` | system-wide hotkeys, low-level-hook override, `"Win+Shift+F12"` parsing |
| `overlay` | layered click-through windows drawn from a pixel buffer |
| `monitor` | monitor bounds, work areas, per-monitor DPI, cursor placement |
| `config` | `.ini` settings, portable first, comment-preserving |
| `startup` | run at logon (`HKCU\...\Run`) |
| `resources` | icons embedded by `build.rs` |
| `memory` | one-shot working set trim |
| `ids` | message and command ids reserved by the core |
| `build` | `build.rs` helper: compiles the `.rc`, keeps its version in sync |

The Win32 rationale behind each of these — why a hidden top-level window and
not `HWND_MESSAGE`, why notification icon version 4, why the tray menu needs
`SetForegroundWindow`, what the keyboard hook has to get right, why trimming
the working set is cosmetic — is in `skeleton/README.md` and in the module
docs, which are worth reading before changing anything.

## Overlays

`y-snap`, `y-pop` and `y-coustic` have no window to speak of: they show a
guide, a launcher or a volume bar and get out of the way. That is `overlay`:
a borderless, topmost, non-activating `WS_EX_LAYERED` window, pushed to the
screen with `UpdateLayeredWindow` from a buffer the application draws into.
Real per-pixel alpha, no `WM_PAINT`, no flicker — at the price of drawing
every pixel, which `Canvas` (fills, rounded corners, GDI text) makes bearable.

Draw in this order: fill the panel, draw the text, round the corners. GDI
writes zero alpha wherever it touches, so `Canvas::draw_text` repairs the
alpha of its rectangle afterwards — which only works if that rectangle was
opaque to begin with, and only if the corners are cut after the text.

## Features

* `win` (default) — everything above.
* `build` — the `build.rs` helper alone. Enable it *without* default features
  in `[build-dependencies]`, so building the build script does not also
  compile the `windows` crate for the host.

```toml
[build-dependencies]
y-core = { workspace = true, default-features = false, features = ["build"] }
```

## Resources

An application that wants its own icon, manifest and version info copies
`skeleton/rsc/` and `build.rs`, then calls `y_core::build::embed_resources`.
Until then `resources::app_icon_or_stock` falls back to the generic system
icon: an icon is cosmetic, and failing to start over one would be absurd.
