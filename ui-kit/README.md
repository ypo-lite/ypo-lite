# Ypo-Lite ui-kit

The WinUI toolkit shared by the Ypo-Lite settings apps
([`y-snap-config`](../y-snap-config/README.md) and
[`y-pop-config`](../y-pop-config/README.md)). It sits on top of
[`windows-reactor`](../docs/design/windows-reactor.md), and the crate is imported as
`ypo_ui`.

It exists to make the right layout the default one. Every screen gets the width it
really has (a `Viewport`, passed down the tree), controls come pre-aligned, and
tables, grids and panes reflow instead of scrolling sideways or clipping. Sizes,
fonts and opacities come from tokens, never from literals in a screen.

```rust
use ypo_ui::prelude::*;

fn screen(viewport: Viewport) -> View {
    page(
        "Hotkeys",
        "Global key combinations, each bound to one action.",
        section("Keys", Table::new(&COLUMNS).rows(rows).build(viewport)),
    )
}
```

## What's in it

| Module | |
| --- | --- |
| `tokens` | spacing, type ramp, radii and opacities: the only literal numbers |
| `viewport` | `Viewport`, the width a piece of UI gets |
| `layout` | `Stack`, `AutoGrid`, `Split`, `Fill`, `cell`, `scroll_y` |
| `text` | text roles: title, lede, body, caption… |
| `controls` | pre-aligned inputs: combo, number box, switch, chip, keycaps… |
| `surfaces` | page, section, card, panel, note, setting |
| `table` | a flexing table that turns into cards when it cannot fit |
| `editor` | the inline Cancel/Done editor, and the modal dialog |
| `shell` | the window frame: nav pane modes, page scroller, status bar, warnings banner, *Start with Windows* |
| `keys`, `key_picker` | the key catalog and the key/modifier picker |

`prelude` brings in what a screen normally needs in one `use`.

## Rules in short

- No literal size, font size, opacity or radius in a screen: use a token.
- A screen takes a `Viewport` and decides from it, never from the window size.
- Nothing scrolls sideways: use `AutoGrid`, `Split` or `Table`.
- The kit never imports an app's `Msg`. It takes `View`s and `Callback`s.

The reasons, the recipes, the WinUI traps already handled and a checklist for a new
screen are in the **[UI toolkit guide](docs/ui-toolkit.md)**. Read it before touching a
screen.

Part of [Ypo-Lite](../README.md).
