//! The UI toolkit every WinUI screen in the workspace is built from — see
//! `ui-kit/docs/ui-toolkit.md` for the guide (principles, recipes, WinUI
//! traps). Nothing here knows an app's `Msg` or settings: it takes `View`s,
//! `Callback`s and a [`viewport::Viewport`], which is what let it move out of
//! `y-snap-config` for `y-pop-config` to share.
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
//! | [`shell`] | The window frame: nav pane modes, page scroller, status bar, warnings banner. |
//! | [`keys`] | The key catalog behind the key picker. |
//! | [`key_picker`] | The key/modifier picker, driven by [`key_picker::KeyPickerEvents`] callbacks. |

pub mod controls;
pub mod editor;
pub mod key_picker;
pub mod keys;
pub mod layout;
pub mod shell;
pub mod surfaces;
pub mod table;
pub mod text;
pub mod tokens;
pub mod viewport;

/// Everything a screen module normally needs, in one `use`.
pub mod prelude {
    pub use super::controls::*;
    pub use super::editor::{editor, editor_checked};
    pub use super::layout::{AutoGrid, Fill, Split, Stack, cell, scroll_y};
    pub use super::surfaces::{CARD_INSET, card, note, page, panel, section, setting, setting_wide};
    pub use super::table::{Column, Row, Table};
    pub use super::text;
    pub use super::tokens::{emphasis, radius, size, space};
    pub use super::viewport::{SizeClass, Viewport};
}
