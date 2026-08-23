# win-rust-min

Minimal Windows tray application in Rust, built directly on the `windows`
crate (0.62). It creates a hidden window, puts an icon in the notification
area, and shows a popup menu with a single **Exit** entry on right click.

It is meant to be copied as a skeleton: no framework, no extra dependency,
but the structure and the Win32 idioms of a real application already in place.

## Build & run

```sh
cargo run              # debug
cargo build --release  # release, ~size-optimised profile in Cargo.toml
```

The binary is a GUI application (`#![windows_subsystem = "windows"]`), so no
console window appears. Comment out that attribute in `src/main.rs` if you
want `println!` debugging.

## Layout

| File            | Role |
| --------------- | ---- |
| `build.rs`      | Compiles the resource script into the executable |
| `rsc/*.rc`      | Icon, manifest and version info |
| `src/main.rs`   | Entry point, subsystem attribute, start-up error box |
| `src/app.rs`    | Application state and message handling — **edit this one** |
| `src/window.rs` | Window class, window procedure, message loop — reusable |
| `src/tray.rs`   | Notification area icon, RAII — reusable |
| `src/menu.rs`   | Popup menu, RAII — reusable |
| `src/hotkey.rs` | System-wide hotkeys + low-level-hook override, RAII — reusable |
| `src/memory.rs` | One-shot working set trim |
| `src/resources.rs` | Typed access to the embedded resources |
| `src/ids.rs`    | Message and command identifiers |

The split follows one rule: `unsafe` and Win32 plumbing live in the reusable
modules, `app.rs` stays plain Rust with a `match` on messages.

## Patterns worth keeping

**State behind the window procedure.** `wndproc` is a C callback and cannot
capture anything. The handler is boxed, passed through `CreateWindowExW`'s
`lpParam`, stored in `GWLP_USERDATA` on `WM_NCCREATE`, borrowed back on every
message, and dropped on `WM_NCDESTROY` (the last message a window receives).
`Box<dyn WindowHandler>` is a fat pointer, hence the double box.

**A hidden top-level window, not `HWND_MESSAGE`.** Message-only windows are
tempting for an invisible app, but they do not receive broadcast messages —
and the tray icon must be restored when Explorer restarts, which is announced
by the `TaskbarCreated` broadcast.

**RAII over shell/GDI handles.** `TrayIcon` sends `NIM_DELETE` on drop, so no
ghost icon can be left in the notification area; `PopupMenu` calls
`DestroyMenu`. Ownership, not discipline, keeps the cleanup correct.

**Notification icon version 4.** After `NIM_ADD`, `NIM_SETVERSION` opts into
the modern callback protocol: the cursor position arrives packed in `wParam`
and the event id in the low word of `lParam`, and `NIF_SHOWTIP` gives proper
tooltips. `app.rs` unpacks both.

**The tray menu dance.** `SetForegroundWindow` before `TrackPopupMenu` and a
`WM_NULL` post afterwards — without them the menu does not dismiss when the
user clicks elsewhere (MS KB135788). `TPM_RETURNCMD` returns the command id
directly, so no `WM_COMMAND` round trip is needed; `WM_COMMAND` is still
handled for menus and accelerators added later.

**Exit path.** Menu → `DestroyWindow` → `WM_DESTROY` drops the tray icon and
posts `WM_QUIT` → the message loop returns → `main` exits with 0.

## Resources

`build.rs` compiles `rsc/win-rust-min.rc` with the [`embed-resource`] crate,
which picks whichever resource compiler the active toolchain provides
(`rc.exe`, `llvm-rc` or `windres`). A hand-written `.rc` is preferred over the
`winres`/`winresource` crates, which generate the script for you: the moment
you add a menu, a dialog or a string table, you want to own that file.

The script embeds three things:

- **The icon** (`IDI_APP_ICON`, id 101) — the lowest-numbered icon resource is
  the one Explorer shows for the executable, so keep it first. `rsc/*.ico` is
  multi-resolution, and `resources::load_app_icon` asks for `SM_CXSMICON` or
  `SM_CXICON` explicitly so the shell picks the right image instead of
  rescaling a 32px one down to 16px. `LR_SHARED` means the handle is owned and
  cached by the system: never call `DestroyIcon` on it.
- **The manifest** (`rsc/win-rust-min.exe.manifest`) — enables visual styles
  (common controls v6), declares per-monitor v2 DPI awareness and UTF-8 as the
  active code page, and lists the supported Windows versions.
- **Version info** — what the "Details" tab of the file properties shows.

`build.rs` also fails the build when `FILEVERSION` in the `.rc` drifts away
from the version in `Cargo.toml`; the two live in different files and would
otherwise diverge silently. Bump both, or drop `assert_version_in_sync`.

`resources.rs` holds the id constants, which must match the `#define`s in the
script, and a `resource_id` helper — the Rust spelling of `MAKEINTRESOURCE`,
where a numeric id is passed as a pointer whose *value* is the id.

To swap in your own icon: drop the `.ico` in `rsc/`, update the file name in
the `.rc`. Nothing else changes.

[`embed-resource`]: https://crates.io/crates/embed-resource

## Hotkeys

`hotkey.rs` registers system-wide hotkeys by two routes, and the application
cannot tell them apart — both end up as a `WM_HOTKEY` on the owner window:

```rust
let mut hotkeys = HotkeyManager::new(hwnd);
hotkeys.register(IDH_SHOW_MENU, hotkey, Fallback::LowLevelHook)?;
```

- **`RegisterHotKey`** — the polite route. The shell arbitrates and reserves
  the combination. It fails when someone else already owns it, and the shell
  owns most of the interesting `Win+key` ones.
- **`WH_KEYBOARD_LL`** — the fallback, requested with `Fallback::LowLevelHook`.
  We see every keystroke before the system does and swallow the ones we want,
  which is how an application reclaims a combination the shell reserved.
  `Fallback::None` is the right default: a combination owned by another
  application belongs to that application.

`register` returns whether it got `Registration::System` or
`Registration::Override`, which is worth surfacing in a settings UI. Dropping
the `HotkeyManager` releases everything and unhooks.

Things the hook has to get right, all of them load-bearing:

- **It runs for every keystroke on the machine**, on the message-loop thread.
  Windows silently discards hooks slower than `LowLevelHooksTimeout` (300 ms),
  so the procedure allocates nothing and takes no lock another thread could
  hold. State lives in a `thread_local`, not a `Mutex`, because the procedure
  only ever runs on the thread that installed the hook — so there is no lock a
  keystroke could ever block on. The hook is installed on first need and
  dropped as soon as no override wants it.
- **Swallowing the key-up too.** Having eaten the press, the release must go
  as well, or applications see half a keystroke.
- **Cancelling the Start menu.** Eating `Win+X` leaves the shell seeing a Win
  press followed by a Win release with nothing between them — the gesture that
  opens the Start menu. Injecting a keystroke the shell ignores (right shift)
  breaks the pair. This is the trick from PortAL's `hotKey.cpp`, with
  `SendInput` in place of the deprecated `keybd_event`.
- **Recognising its own injection.** Those synthetic keystrokes come straight
  back through the hook. They are tagged with a signature in `dwExtraInfo`
  rather than filtered on `LLKHF_INJECTED`, because plenty of *legitimate*
  input is injected — on-screen keyboards, remote desktop, macro tools, some
  laptop Fn keys — and filtering all of it gives you a hotkey that
  mysteriously fails for those users.
- **Exact modifier matching**, like `RegisterHotKey`: with `Win+X` overridden,
  `Win+Ctrl+X` stays somebody else's business. `MOD_NOREPEAT` is honoured on
  the fallback path too, by tracking the key's down state.

Limits worth knowing: without UIAccess the hook never sees keystrokes headed
for an elevated application, and it is inactive on the secure desktop (UAC
prompt, lock screen). Neither route can be made to work there from a normal
process.

Turning `"Win+Shift+F12"` from a config file into a `Hotkey` is not in here —
that is `getHotKeyCode`/`getModifier` in PortAL's `hotKey.cpp`, and it belongs
with whatever configuration format the application ends up using.

## Memory

Measured on the release build, idle in the notification area:

| | Working set | Private bytes |
| --- | ---: | ---: |
| Without trimming | 9 420 K | 1 504 K |
| After `trim_working_set()` | 412 K | 1 520 K |

The two columns measure different things, and only one of them is real.

**Private bytes — 1.5 MB — is already the floor.** That is the memory the
process actually costs the system: a few heap allocations, the stacks, the
loader's per-process data. There is nothing in it to optimise away.

**Working set is mostly not ours.** The process maps 24 DLLs (`user32`,
`gdi32full`, `shell32`, `combase`, `uxtheme`, …); those pages are shared with
every other GUI process on the machine and counted again in each of them. A
C++ tray application lands in the same range for the same reason.

**A different global allocator does not help.** Rust on the MSVC target
already routes `alloc` to the process heap (`HeapAlloc`), with no arena of its
own. `mimalloc` or `jemalloc` would *add* preallocated arenas and raise
private bytes; this application performs a few dozen small allocations in its
whole lifetime, so there is nothing for a faster allocator to win.

**Trimming is cosmetic, and `memory.rs` treats it as such.** `EmptyWorkingSet`
moves pages to the standby list; the commit charge does not change (see the
table: private bytes went *up* by the size of the call's own bookkeeping).
Everything touched afterwards — opening the menu, for instance — faults back
in, cheaply from standby, expensively from the pagefile if the machine is
under pressure. So the skeleton calls it exactly once, after start-up, and
never on a timer: CRT init, DLL relocation and icon loading touch pages that
are genuinely never read again, and the process then idles for hours.

Prefer `EmptyWorkingSet` over `SetProcessWorkingSetSize(h, 102400, 409600)`:
those arguments set a *quota*, not a one-off trim, and a process pinned under
a 100 KB minimum faults continuously. The "trim now" spelling of that API is
`SetProcessWorkingSetSize(h, usize::MAX, usize::MAX)`, which is what
`EmptyWorkingSet` does with less room for error.

If the idle number does not matter to you, delete the call in `app.rs::run` —
Windows already trims idle processes on its own when memory gets tight.

## Extending

- **New menu entry**: add an id in `ids.rs`, an `add_item` call in
  `App::show_context_menu`, an arm in `App::on_command`.
- **New hotkey**: add an id in `ids.rs`, a `register` call in `App::on_create`,
  an arm in `App::on_hotkey`. The skeleton ships one demo,
  `Win+Shift+F12`, which opens the tray menu at the cursor.
- **Left click / double click**: add the `WM_LBUTTONUP` / `WM_LBUTTONDBLCLK`
  cases in `App::on_tray_event`.
- **A new resource** (dialog, string table, accelerators): add it to the `.rc`
  and its id to `resources.rs`.
- **A visible window**: `window.rs` already creates a normal
  `WS_OVERLAPPEDWINDOW`; call `ShowWindow` and handle `WM_PAINT` in `app.rs`.
- **Single instance**: create a named mutex in `main` before `app::run`.
