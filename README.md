# Ypo-Lite

*Lite productivity tools for Windows: low memory, fast, and they do just what
they need to do.*

<p align="center">
    <picture>
        <img src="./docs/ypo-lite_logo.png" />
    </picture>
</p>

> Ypo-Lite rewrites the [GUIPro](http://glatigny.github.io/guipro/) tray
utilities (PortAL, HulK, VulCan, ...) in Rust. Each tool is one small native
executable, with no DLL plugins and no background service.

## The tools

| Tool | What it does | Rewrite of | State |
| --- | --- | --- | --- |
| **[Y-Snap](y-snap/README.md)** | Window management by hotkey, mouse gesture and overlay: minimize, maximize, roll up, fade, strip the frame, hide behind a tray icon, pin to every virtual desktop or send to another one | HulK | Usable |
| **[Y-Pop](y-pop/README.md)** | A tray icon whose menu launches your programs, and a search window that finds them as you type | PortAL | Usable |
| **Y-Coustic** | Volume manager | VulCan | *Work in progress* |

Y-Snap and Y-Pop are configured through one XML file each. `y-snap-config` and
`y-pop-config` are the WinUI 3 settings apps that edit those files, if you
would rather not edit them by hand.

## Install

Ypo-Lite needs Windows 10 or 11 (x64). Download one of these from the
[latest release](https://github.com/ypo-lite/ypo-lite/releases/latest):

- **The installer**, `ypo-lite-setup-<version>.exe`. It installs for your user
  only, without administrator rights, and lets you pick the tools. It adds a
  default configuration, and downloads the Windows App Runtime (about 125 MB)
  if the settings apps need it.
- **The portable zip**, `ypo-lite-<version>-x64.zip`. Extract it to a folder of
  your own and run `y-snap.exe` or `y-pop.exe`; each one appears in the
  notification area. The configuration files stay next to the executables.

The settings apps need the
[Windows App Runtime](https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads)
2.4 or later. If you use the zip and it is missing, they offer to open that
page. Y-Snap and Y-Pop don't need it.

The executables and the installer are signed with the project's own self-signed
certificate, `CN=Ypo-Lite`, published as
[`installer/ypo-lite-signing.cer`](installer/ypo-lite-signing.cer). No public
authority has vouched for it, so Windows still reports the publisher as unknown
and SmartScreen may warn you the first time you run them. Each release lists
SHA-256 checksums, and `gh attestation verify <file> --repo ypo-lite/ypo-lite`
confirms a file was built by this repository's release workflow.

### Where the configuration lives

Each tool uses the first of these that exists:

1. the path given with `-config <file>` on the command line;
2. `y-snap.xml` or `y-pop.xml` next to the executable, which keeps a copy
   portable;
3. `%APPDATA%\Ypo-Lite\`.

## Build from source

You need [Rust](https://rustup.rs/) 1.95 or later with the MSVC toolchain, and
the Windows SDK for its resource compiler.

```sh
cargo build --release    # executables in target/release
cargo test --workspace
cargo run -p y-snap      # debug run of one tool
```

## Repository layout

| Path | Content |
| --- | --- |
| `y-snap`, `y-pop` | the tools |
| `y-snap-config`, `y-pop-config` | their WinUI 3 settings apps |
| `y-term`, `y-coustic` | placeholders for the next tools |
| [`core`](core/README.md) | shared Win32 plumbing: tray icon, hotkeys, overlays, menus, configuration |
| [`ui-kit`](ui-kit/docs/ui-toolkit.md) | the WinUI toolkit shared by the settings apps |
| [`skeleton`](skeleton/README.md) | a self-contained reference application, with the Win32 rationale behind `core` |

## License

[GPL-3.0-or-later](LICENSE).
