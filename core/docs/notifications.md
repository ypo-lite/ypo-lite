# Notifications in `y-core` — spec

Status: **implemented in `core/src/tray.rs`** (deviations from the proposal
below are listed under "As built"). Agreed direction: route **A** — keep
`NIF_INFO` balloons (which Windows 10/11 already shows as toasts and files in
the notification center) and make them richer and actionable. No WinRT
`ToastNotificationManager`: it needs an AUMID plus a Start-menu shortcut for an
unpackaged app and pulls WinRT into the binary, against "low memory, fast".

## As built

* `TrayEvent::decode` keeps its `(event, position)` signature. The icon id is
  a separate `TrayEvent::icon_id(lparam)`, so single-icon apps (`y-pop`,
  `y-term`) were not touched.
* `Notify::User(HICON)` carries the icon; the builder has `.level()`,
  `.large_icon()`, `.silent()`, `.respect_quiet_time()`.
* `TrayEvent::BalloonClosed` covers both `NIN_BALLOONTIMEOUT` and
  `NIN_BALLOONHIDE`.
* Truncation lives in `resources::copy_to_buffer_ellipsis` (`…`, never splits
  a surrogate pair); the tooltip still uses the silent `copy_to_buffer`.
* `skeleton` is **not** updated: it has no `notify` at all, so it did not
  diverge, only lags. Port the builder there if/when it grows one.
* Tests: `tray::tests` (decode, icon id) and `resources::ellipsis_tests`.
* Only call site: `y-snap`'s `notify` helper.

## Today (before the change)

`TrayIcon::notify(title, message, Notify)` in `core/src/tray.rs`: one-shot
`NIF_INFO` on the single icon, four stock icons, and no feedback.
`TrayEvent::decode` folds `NIN_BALLOON*` into `TrayEvent::Other`, so an app
cannot tell that a balloon was clicked. It also returns no icon id, which
becomes a problem as soon as an app owns more than one icon (see
`y-snap/docs/iconize-traynize-plan.md`).

## Goals

1. A notification can carry **a custom icon**, can **respect quiet time**,
   and can **be silent**.
2. The app learns when a notification was **clicked** (and, optionally, shown /
   timed out), and can attach an action to it.
3. Callers stop passing positional `(title, message, level)`.
4. Callbacks identify **which icon** they come from.

## API proposal

```rust
pub struct Notification<'a> {
    title: &'a str,          // truncated to szInfoTitle (63 chars + NUL)
    body: &'a str,           // truncated to szInfo (255 chars + NUL)
    level: Notify,           // None | Info | Warning | Error | User(HICON)
    large_icon: bool,        // NIIF_LARGE_ICON, only meaningful with User
    silent: bool,            // NIIF_NOSOUND
    respect_quiet_time: bool // NIIF_RESPECT_QUIET_TIME
}

impl TrayIcon {
    pub fn notify(&mut self, n: &Notification) -> Result<()>; // replaces the old signature
}
```

* Builder-style constructors: `Notification::new(title, body).warning().silent()`.
* Truncation is **explicit and on a char boundary** with a trailing `…`; today
  `copy_to_buffer` truncates silently. Callers that build multi-line bodies
  (y-snap's configuration warnings) must still budget their own lines — 255
  UTF-16 units is about five short lines.
* `Notify::User(HICON)` sets `NIIF_USER` and `hBalloonIcon`; the handle is
  borrowed (the shell copies it), like the tray icon's own.
* The old `notify(title, message, level)` is removed, not kept as a shim: it has
  few call sites (y-snap `main.rs`, `y-pop`, skeleton) and the workspace is
  ours.

### Events

`TrayEvent` gains:

| variant | message | meaning |
| --- | --- | --- |
| `BalloonClicked` | `NIN_BALLOONUSERCLICK` | the user clicked the notification body |
| `BalloonShown` | `NIN_BALLOONSHOW` | the shell displayed it (not sent if suppressed — do not rely on it) |
| `BalloonClosed` | `NIN_BALLOONTIMEOUT` | it timed out or was dismissed |

`decode` returns the **icon id** too (high word of `lParam` under version 4):

```rust
pub fn decode(wparam, lparam) -> (TrayEvent, POINT, /*icon id*/ u32)
```

An app that wants "click the balloon → do X" remembers *what the last balloon
was for* (a small enum in the app, cleared on `BalloonClosed`/`BalloonClicked`).
`core` stays out of that: it only delivers the events. Only one balloon is
live per icon at a time, so one remembered value is enough.

## Limits we must live with

* The shell may drop a balloon (Focus Assist, per-app notifications off).
  Nothing may depend on it being seen — keep the existing rule; anything
  important is also shown in the menu (y-snap's `problem` line already does so).
* Windows rate-limits notifications from one icon; do not fire balloons in a
  loop (e.g. one per rejected hotkey).
* `NIIF_USER` icon requires the icon be a real `HICON` that outlives the call.

## `skeleton`

`skeleton/` mirrors `tray.rs` and must keep building standalone. Port the
`Notification` type there too, or leave `skeleton` on the old API and say so in
its README — **decision for implementation time**, but it must not be left
silently diverged.

## Work list

1. `core/src/tray.rs`: `Notification`, `Notify::User`, new `TrayEvent`
   variants, icon id in `decode`.
2. Update call sites (`y-snap`, `y-pop`, `y-term`, `y-coustic`, `skeleton`).
3. Unit tests for `decode` (pure function — no Win32 needed) and for the
   truncation helper.
4. `core/README.md`: replace the balloon paragraph.
