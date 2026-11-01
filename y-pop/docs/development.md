# y-pop — development

How y-pop is built. To use it, see the [user manual](user-manual.md) and the
[configuration reference](configuration.md). Building the workspace is on the
[main page](../../README.md#build-from-source).

```sh
cargo run -p y-pop       # debug run
cargo test -p y-pop
```

## Layout

| | |
| --- | --- |
| `main.rs` | the tray app: the message loop, tray icons, hotkeys, reload, what each pick does |
| `menu.rs` | the native popup, built from the tree: icons, submenus, lazy folders, Reveal |
| `tooltip.rs` | item tooltips, on a small overlay |
| `about.rs` | the text of the *About* balloon, built to the balloon's size limit |
| `search_window.rs` | the search window: reads the keyboard and mouse, and paints |
| `config.rs`, `config/reader.rs` | finding `y-pop.xml`, reading it into the tree |
| `model.rs` | the configuration tree, a flat arena addressed by `NodeId` |
| `variables.rs`, `builtins.rs` | `%name%` expansion and string interning; the built-in values |
| `launch.rs` | the routes a launch can take, and the clipboard variables |
| `browse.rs` | listing a browsed folder |
| `theme.rs`, `skins.rs` | `theme` and `<skin>` as read; the built-in skins, and resolving both into a design |
| `internal_icons.rs` | the `@name` icons |
| `search/` | search itself, pure Rust but for painting: see its module doc |
| `settings.rs`, `settings/` | the editable form of `y-pop.xml`, which `y-pop-config` loads and saves |
| `ipc.rs` | what another process can ask of a running y-pop |

The first five are the `y-pop` binary, the rest is the `y_pop` library that
`y-pop-config` shares. Everything Win32 that is not specific to a launcher (the
hidden message window, the tray icon, menus, hotkey registration, the XML reader,
the overlay surface) is in [`y-core`](../../core/README.md). The settings app is
[`y-pop-config`](../../y-pop-config/docs/design/plan.md).

## How it works

**Launching.** `launch.rs` has three routes: `CreateProcessW`, `ShellExecuteExW`
when `shell` or `elevate` asks for it, and `IApplicationActivationManager` for a
Store app. What the user launches goes through the shell by default, because
that is what records it in the Windows history search reads.

**Search.** Everything but `look` and `preview` is plain Rust with no window, so
it runs in tests. The index is one flat list built from the tree at load and
reload, never per keystroke, with its text folded once. The matcher and the
hotkey type take no y-pop type, so they can move to `y-core` when y-term needs
them. The history and the `/` files are read on threads of their own.

**Reveal.** It opens the tray menu and presses arrow keys for the user, counting
the rows above the target. That is why it is best effort: see
[Reveal in menu](user-manual.md#reveal-in-menu).

**Talking to the running app.** `ipc.rs` holds the names both sides derive from:
a registered window message asks y-pop to reload after `y-pop-config` saves, and
a request hands "show this item" or "add this app" to an open `y-pop-config`.
The design is y-snap's, see `y-snap/src/ipc.rs`.
