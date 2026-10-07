//! The UI toolkit every screen is built from — see
//! `y-snap-config/docs/ui-toolkit.md` for the guide (principles, recipes,
//! WinUI traps). Nothing under `ui` knows about this app's `Msg` or
//! `Settings`: it takes `View`s, `Callback`s and a [`viewport::Viewport`],
//! so it can move to a shared crate once a second WinUI app needs it.
//!
//! | Module | What's in it |
//! | --- | --- |
//! | [`tokens`] | Spacing, type ramp, radii, opacities — the only literal numbers. |
//! | [`viewport`] | [`viewport::Viewport`]: the width a piece of UI gets, passed down the tree. |
//! | [`layout`] | `Fill`, `Stack`, `AutoGrid`, `Split`, `cell`, scrolling. |
//! | [`text`] | Text roles: title, lede, body, caption… |
//! | [`controls`] | Pre-aligned inputs: combo, number box, switch, chip, keycaps… |
//! | [`surfaces`] | Page, section, card, panel, note. |
//! | [`table`] | Flexing table that turns into cards when it can't fit. |
//! | [`editor`] | The inline Cancel/Done editor shell. |

pub mod controls;
pub mod editor;
pub mod layout;
pub mod surfaces;
pub mod table;
pub mod text;
pub mod tokens;
pub mod viewport;

/// Everything a screen module normally needs, in one `use`.
pub mod prelude {
    pub use super::controls::*;
    pub use super::editor::editor;
    pub use super::layout::{AutoGrid, Fill, Split, Stack, cell, scroll_y};
    pub use super::surfaces::{CARD_INSET, card, note, page, panel, section};
    pub use super::table::{Column, Row, Table};
    pub use super::text;
    pub use super::tokens::{emphasis, radius, size, space};
    pub use super::viewport::{SizeClass, Viewport};
}
