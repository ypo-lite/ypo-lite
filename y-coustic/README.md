# y-coustic

**The volume, one keystroke away.** y-coustic lives in the notification area and
steps or mutes the sound with the hotkeys you choose, then tells you where the
level went with the HUD, a small bar on the screen you are looking at.

This is the first phase of the rewrite of *VulCan*: the master volume, hotkeys
and the HUD. Microphone, applications, the mouse wheel and a mixer window follow;
the [implementation plan](docs/design/implementation-plan.md) says in which
order, and the [design](docs/design/y-coustic-plan.md) says why.

## What it does today

- **Hotkeys for the master volume**: louder, quieter, mute, or a fixed level.
  Holding a step key keeps stepping; holding mute does not flap.
- **The HUD on the monitor under the pointer**: a speaker, the level and the
  percentage, in the system's light or dark theme. A burst of presses keeps one
  HUD on screen, which goes away a moment after the last.
- **Follows your output device**: switch speakers for headphones and the keys
  drive the new one, with no reload.
- **Light.** One native executable, no runtime, no service. It gives its memory
  back after each event.
- **Portable.** Drop `y-coustic.xml` next to `y-coustic.exe`. With no file at
  all, the defaults apply.

## Quick start

1. Run `y-coustic.exe`. An icon appears in the notification area.
2. Press `Win` + `NumAdd` or `NumSub` to change the volume, `Win` + `Numpad0` to
   mute.
3. Right-click the icon for the menu: **Reload configuration**, **Open
   configuration** (creates the file if there is none), **Start with Windows**,
   **About**, **Exit**. A left click shows the current level.

## Documentation

- [User manual](docs/user-manual.md)
- [Configuration reference](docs/configuration.md)
- [Development notes](docs/development.md)
- [Design](docs/design/y-coustic-plan.md) and
  [implementation plan](docs/design/implementation-plan.md)

Building the workspace is on the [main page](../README.md#build-from-source).
