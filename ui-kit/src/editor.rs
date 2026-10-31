//! The inline editor shell: a [`panel`] titled with what's being edited,
//! Cancel/Done in its header, the editing UI as its body.
//!
//! Inline page content, never a popup: a `Flyout`'s width follows its
//! anchor, and `ContentDialog` has a ~548px max width from its native style
//! that this crate gives no way to override (`docs/windows-reactor.md` §6).
//! When something does need a popup, [`modal`] draws one over the page.
//! An editor normally opens as its row's [`crate::table::Row::detail`],
//! right under the thing it edits.
//!
//! Cancel is a revert, not a discard: edits commit to the model as they
//! happen (so the rest of the page — the table row, the live preview —
//! updates live), and the app restores a snapshot taken when the editor
//! opened. See the app's own `editing` state.

use crate::controls::button;
use crate::layout::{Stack, scroll_y};
use crate::surfaces::{CARD_INSET, card, panel};
use crate::text;
use crate::tokens::space;
use crate::viewport::Viewport;
use windows_reactor::*;

pub fn editor(
    title: impl Into<String>,
    body: impl Into<View>,
    on_cancel: Callback<()>,
    on_done: Callback<()>,
) -> View {
    editor_checked(title, body, true, on_cancel, on_done)
}

/// [`editor`] whose Done can be disabled while what is being edited is not
/// acceptable (`done_enabled: false`). Cancel always works.
pub fn editor_checked(
    title: impl Into<String>,
    body: impl Into<View>,
    done_enabled: bool,
    on_cancel: Callback<()>,
    on_done: Callback<()>,
) -> View {
    let actions = Stack::horizontal(space::S)
        .child("cancel", button("Cancel", false, true, on_cancel))
        .child("done", button("Done", true, done_enabled, on_done));
    panel(title, Some(actions.into()), body)
}

/// Widest a [`modal`] gets, however big the window is.
pub const MODAL_MAX_WIDTH: f64 = 860.0;

/// A dialog drawn over the whole page: a dimmed scrim and a centered card
/// titled `title`, with `body` and Cancel / OK under it. Put it in a one-cell
/// `Grid` on top of the page it covers.
///
/// It exists because `ContentDialog` can't be wider than ~548px (see the top
/// of this file). The cost: it is modal to the pointer only — the scrim eats
/// clicks, but Tab can still reach the controls behind it.
///
/// `window` is the width of the area it covers; `body` is given the width its
/// own content gets, to lay itself out in. Edits are the app's business, as
/// with [`editor`]: OK keeps them, Cancel (the app) reverts.
pub fn modal(
    window: Viewport,
    title: impl Into<String>,
    body: impl FnOnce(Viewport) -> View,
    ok_enabled: bool,
    on_cancel: Callback<()>,
    on_ok: Callback<()>,
) -> View {
    let width = (window.width - 2.0 * space::XL).clamp(0.0, MODAL_MAX_WIDTH);
    let inner = Viewport::new(width).inset(CARD_INSET);

    let buttons = Stack::horizontal(space::S)
        .child("cancel", button("Cancel", false, true, on_cancel))
        .child("ok", button("OK", true, ok_enabled, on_ok));
    let content = Stack::vertical(space::L)
        .child("title", text::strong(title))
        .child("body", body(inner))
        .child(
            "buttons",
            Border::new()
                .horizontal_alignment(HorizontalAlignment::Right)
                .content(buttons),
        );
    // `card()`'s own background is translucent (it lets the window's Mica show
    // through); over other content it must be opaque to be readable.
    let card = card()
        .background(ThemeBrush::SolidBackground)
        .width(width)
        .margin(Thickness::new(0.0, space::XL, 0.0, space::XL))
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Top)
        .content(content);

    Grid::new().keyed_children([
        (
            "scrim",
            Border::new()
                .background(ThemeBrush::SolidBackground)
                .opacity(0.7)
                .into(),
        ),
        ("dialog", scroll_y(card)),
    ])
}
