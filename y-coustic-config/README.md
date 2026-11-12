# Y-Coustic-config

The settings app of [Y-Coustic](../y-coustic/README.md): a WinUI 3 editor for
`y-coustic.xml`. One screen each for the general options, the keys and their
layers, the mouse, the tray icon, the mixer, the HUD (with a preview drawn by
y-coustic itself) and the colours. When you save, a running y-coustic applies the
change at once.

<!-- Add once taken: ![y-coustic-config](docs/images/y-coustic-cfg_keys_01.png) -->

## Use it

Choose **Settings…** in the y-coustic tray menu. `y-coustic-config.exe` sits next
to `y-coustic.exe`; only one window opens at a time, and a second click brings the
open one forward. It also starts on its own, with or without y-coustic running.

Saving keeps the comments of `y-coustic.xml`, the attributes it does not know and
the entries it cannot use (as comments, with a warning that says why), and the
first save of a run keeps the old file as `y-coustic.xml.bak`. With no
`y-coustic.xml` yet, the first Save writes the shipped one, explanations included.

## Documentation

| | |
| --- | --- |
| [User manual](docs/user-manual.md) | every screen of the app |
| [Configuration reference](../y-coustic/docs/configuration.md) | every setting of `y-coustic.xml` |
| [Development notes](docs/development.md) | how the app is built, and the limits found |
| [`design/`](docs/design/plan.md) | the plan, with what was decided and why |
| [UI checklist](docs/ui-checklist.md) | what to look at in a real window |
| [UI toolkit](../ui-kit/docs/ui-toolkit.md) | the shared `ui-kit` the screens are built from |

## Build

```sh
cargo run -p y-coustic-config
```

Needs the [Windows App Runtime](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads)
2.4 or later; if it is missing, the app offers to open that page. Built on
[`windows-reactor`](../docs/design/windows-reactor.md) and `ui-kit`, part of
[Ypo-Lite](../README.md).
