# y-snap — About, Iconize and Traynize — spec

Status: **draft, nothing implemented.** Ports HulK's `iconize.cpp`, `menu.cpp`
and `ShowAbout`, with the changes below. Depends on
[`core/docs/notifications.md`](../../core/docs/notifications.md) (balloon
builder, icon id and balloon events in `TrayEvent`).

## 1. About

* Menu item **"About"** in the tray menu (before "Exit"). No hotkey/action.
* Presented as a **balloon** (as HulK), through the new `Notification`.
* **Title:** `Y-Snap <version>`. The version comes from the executable's
  `VERSIONINFO` (`ProductVersion`), read at run time with `GetFileVersionInfoW`
  / `VerQueryValueW` on our own path. It is already kept in sync with
  `Cargo.toml` by `y_core::build::embed_resources`. New helper:
  `y_core::resources::product_version() -> Option<String>`; falls back to
  `CARGO_PKG_VERSION` when the app embeds no resources.
* **Body** (limit 255 UTF-16 units, so short lines, one fact each):

  ```
  Window management by hotkey, mouse and overlay
  <hotkeys> hotkeys · <n> overlay actions
  Config: …\Ypo-Lite\y-snap.xml
  Memory: 4.2 MB
  Settings app: found | not installed
  ```

  * Config path is **middle-ellipsised** to fit (the tail — file name — and the
    drive/head matter, the middle does not).
  * Memory is the working set (`GetProcessMemoryInfo`), taken *after*
    `trim_working_set()` is **not** applied — report the honest number.
  * "Settings app" mirrors `open_settings`'s `is_file()` check.
  * A project URL/credit line is **dropped** from the balloon if it would
    exceed the budget — it is the first thing to go.
* Clicking the About balloon opens the settings app (`BalloonClicked` while the
  remembered balloon kind is `About`); a no-op if it is not installed.
* A balloon may be suppressed (Focus Assist). That is acceptable for About; the
  menu's disabled summary line keeps showing hotkey/overlay counts.

## 2. Hidden-window registry

New module `y-snap/src/hidden.rs` (state only; Win32 calls for hiding stay in
`window.rs`).

```rust
struct Hidden {
    hwnd: HWND,
    mode: Mode,               // Iconized | Traynized
    title: String,            // snapshot at hide time (tooltip / menu text)
    app: String,              // process exe stem (separator / menu title)
    icon: OwnedOrShared,      // see icons
    tray: Option<TrayIcon>,   // Some only for Iconized — RAII, drop = icon gone
    pid: u32,
}
struct Registry { slots: Vec<Hidden> /* ≤ CAP, in hide order */ }
```

* **Cap: 48** hidden windows total (Iconized + Traynized). Chosen inside the
  agreed 32–64 band; one `const MAX_HIDDEN`. At the cap, refuse with a warning
  balloon "Too many hidden windows (48)". Only Iconized windows cost tray
  space, so the tray-icon count is ≤ 48, never more.
* **Order** is hide order; `uniconize` restores the *last* hidden one (HulK).
* A window already in the registry is **not hidden twice**: re-triggering
  *iconize/traynize* on it is a no-op (it cannot be targeted anyway, being
  invisible — this guards the overlay/"pointed" race).
* Tray ids: `TRAY_ICON_ID` (1) stays y-snap's own icon; hidden windows take
  ids `2..2+CAP` from a free-slot bitmap. The id is the registry slot key; a
  callback's icon id (from `decode`) finds the entry.
* Menu ids: `IDM_HIDDEN_FIRST..+CAP`, above the existing `IDM_*` constants.

### Icons

For a window: `WM_GETICON(ICON_SMALL)` via `SendMessageTimeoutW`
(`SMTO_ABORTIFHUNG`, 250 ms) → `ICON_SMALL2`/`ICON_BIG` → class
`GCLP_HICONSM` → y-snap's own icon. HulK's rules kept: the class icon is a
**shared** handle and is never destroyed; y-snap's own fallback is a shared
resource icon. `WM_GETICON` returns a handle owned by the other process —
used immediately for `NIM_ADD` / menu bitmap and **not retained** beyond that
(the menu bitmap path converts to an owned `HBITMAP` at menu-build time instead
of holding the `HICON`; the tray icon stores the `HICON` the shell copies).
These calls run on the action path, not in a hook, so the 300 ms
`LowLevelHooksTimeout` rule does not apply, but the 250 ms timeout still keeps a
hung target from freezing the message loop.

## 3. Iconize

`iconize` on a target window:

1. Refuse if the window is **elevated and we are not** (UIPI would make
   `ShowWindow` silently fail): warning balloon "Cannot hide an elevated
   window".
2. `ShowWindow(SW_HIDE)` (HulK first sent `SC_MINIMIZE`; kept — see below).
3. Create a `TrayIcon` for it: the window's icon, tooltip = **window title**
   (truncated to 127 chars, the `szTip` limit).
4. Register the entry.

Hiding the taskbar button is **not** done (a multi-window app would lose
buttons for windows that are still visible) — `SW_HIDE` only.

On `SC_MINIMIZE` first: HulK minimized so that a later `SC_RESTORE` returns the
window to its previous (normal/maximized) state. Keep: send `SC_MINIMIZE`, then
`SW_HIDE`.

**Interaction on the window's tray icon:**

| event | action |
| --- | --- |
| `Select` (left click) / `DoubleClick` | restore (**uniconize**) |
| `ContextMenu` | menu: *(title row)* **App name** · separator · **Open** · **Close** |

The title row shows the **application name** drawn as a *titled separator*
(`PopupMenu::add_titled_separator`), as requested. **Close** restores, then
sends `SC_CLOSE` (HulK: a minimized/hidden window may not process `WM_CLOSE`).

## 4. Traynize

Same hide mechanism as Iconize, but **no tray icon of its own**: the window is
listed in y-snap's tray menu.

* Tray menu gets a **"Hidden windows"** section at the top (above the
  summary), only when the registry has Traynized entries: one item per entry,
  text = window title (truncated for the menu, ≈ 50 chars), icon = the window's
  icon, in hide order. Click = restore (**uniconize**). The section is
  separated from the rest.
* Default left-click on y-snap's own icon stays "nothing" (double-click still
  opens settings). **Open point:** a left-click that pops the same menu (or
  *only* the hidden-windows list) is a natural follow-up; the user said this
  area "might evolve", so the section is built as one function
  (`build_hidden_section(menu)`) that can later be reused by a dedicated popup.
* Traynize is the default `right-click-action` of the minimize button
  (already configured that way in `config.rs`).

## 5. Restoring

Actions (configurable like any other, same names in hotkeys and overlay):

| action | does |
| --- | --- |
| `uniconize` | restore the **last** hidden window (existing name, HulK semantics) |
| `uniconize-all` | **new**: restore every hidden window, in reverse hide order |

Restore = `ShowWindow(SW_SHOW)` then `SC_RESTORE`, then bring it to the
foreground (we hold foreground rights: the trigger is a click or a hotkey
delivered to us). Drop the registry entry and its tray icon (RAII).
`uniconize*` need no target window — add them to `Action::needs_window`'s
"false" side. `uniconize-all` is added to `action.rs`'s name table, the
overlay catalog (`ui-kit` key catalog / `y-snap-config` action list), the
README table and `rsc/y-snap.xml` comments.

## 6. Housekeeping and safety (item 8)

* **On exit / `Quit` / `Reload`:** restore everything on `Quit` and on
  `on_destroy` (HulK's `UnIconizeAll`). `Reload` does **not** touch the
  registry — it only reloads configuration.
* **Elevated windows:** refused as above. *(Agreed: "Yes refuse".)*
* **Dead windows** *(you asked for details)*. A hidden window can be destroyed
  by its app while hidden; its tray icon/menu entry must then go away.
  Options:
  1. **Timer, 5 s, only while the registry is non-empty** (HulK): `IsWindow` on
     ≤ 48 handles is microseconds; the timer is killed when the list empties,
     so an idle y-snap has no extra wake-ups. Entries are also re-checked
     lazily when the menu opens and when an icon is clicked. *Recommended* —
     simplest, bounded, matches the "no cost when unused" rule.
  2. **`SetWinEventHook(EVENT_OBJECT_DESTROY)`**: immediate, but it fires for
     every destroyed object system-wide, adding a permanent cost for a rare case.
  3. Both: pointless complexity.
  Spec takes option 1. Handle reuse is a theoretical risk (a dead handle value
  reassigned to a new window within 5 s); entries also store the **pid** and
  re-validate with `GetWindowThreadProcessId` on purge and on restore.
* **Crash recovery** *(you asked for details)*. If y-snap is killed, hidden
  windows are orphaned: invisible, no icon to restore them. Mitigation:
  1. On every registry change, write a tiny state file
     `%LOCALAPPDATA%\Ypo-Lite\y-snap-hidden.txt` (one line per window:
     `hwnd pid class`), deleted when the registry is empty.
  2. At start-up, if the file exists, for each line whose `hwnd` still exists
     *and* matches `pid` + class name and is **not visible**, `ShowWindow(SW_SHOW)`
     it. Then delete the file. No UI, no prompt; one info balloon "Restored N
     windows hidden by a previous session".
  Cost: one small file write per hide/restore (not on a hot path). The class
  + pid check protects against restoring a recycled handle. **Needs your
  decision** — default proposal is *yes*, because an invisible, unrecoverable
  window is the worst failure this feature can cause.
* **Explorer restart** (`TaskbarCreated`): `on_taskbar_created` must re-add
  every Iconized window's icon (`TrayIcon::restore` per entry), not only y-snap's.
* **Balloon spam:** no balloon per hide; only refusals and the crash-recovery
  message.

## 7. Code touch points

| file | change |
| --- | --- |
| `core/src/tray.rs` | per notifications spec; `decode` returns the icon id |
| `core/src/resources.rs` | `product_version()`; window-icon fetch helper `window_icon(hwnd)` (timeout, fallbacks) — `unsafe` stays in `core` |
| `y-snap/src/action.rs` | `UniconizeAll`; `needs_window` false for both uniconize actions |
| `y-snap/src/hidden.rs` | new: registry, slots, state file, purge |
| `y-snap/src/window.rs` | `hide_window` / `show_window` (min + hide, restore + show) |
| `y-snap/src/actions.rs` | route iconize/traynize/uniconize*; they need the registry, so like `Quit`/`Reload`/`Overlay` they are handled in `main.rs` and dropped from the `_ => Pending` arm / added to `is_implemented` |
| `y-snap/src/main.rs` | menu: About + Hidden section; per-icon callbacks; `WM_TIMER` purge; start-up recovery; `on_destroy` restore-all |
| `Cargo.toml` (root) | any new `windows` features (`Win32_System_ProcessStatus`, `Win32_Storage_FileSystem` version APIs, token query for elevation) |
| `y-snap/README.md` | drop "not written yet", document `uniconize-all`, the cap, the elevated-window refusal |
| `ui-kit` / `y-snap-config` | action list knows `uniconize-all` |

## 8. Tests

Pure logic gets unit tests (no Win32): slot allocation and the cap, hide-order
restore, middle-ellipsis, balloon-body budgeting, state-file parse/format,
`Action` name round-trip incl. `uniconize-all`. Live behaviour (hiding a real
window, tray icons, balloons) needs the desktop: per the project rules I will
**ask before running y-snap**, and say exactly what it would do.

## 9. Still open

1. Crash-recovery state file: yes/no (proposal: yes).
2. Dead-window purge: timer (proposal) vs WinEvent hook.
3. Left-click on y-snap's own icon: keep "nothing" for now, or pop the hidden list?
4. Cap value inside 32–64 (proposal: 48).
