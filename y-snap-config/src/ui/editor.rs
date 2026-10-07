//! The inline editor shell: a [`panel`] titled with what's being edited,
//! Cancel/Done in its header, the editing UI as its body.
//!
//! Inline page content, never a popup: a `Flyout`'s width follows its
//! anchor, and `ContentDialog` has a ~548px max width from its native style
//! that this crate gives no way to override (`docs/windows-reactor.md` §6).
//! An editor normally opens as its row's [`crate::ui::table::Row::detail`],
//! right under the thing it edits.
//!
//! Cancel is a revert, not a discard: edits commit to the model as they
//! happen (so the rest of the page — the table row, the live preview —
//! updates live), and the app restores a snapshot taken when the editor
//! opened. See `ConfigApp::editing` in `main.rs`.

use crate::ui::controls::button;
use crate::ui::layout::Stack;
use crate::ui::surfaces::panel;
use crate::ui::tokens::space;
use windows_reactor::*;

pub fn editor(title: impl Into<String>, body: impl Into<View>, on_cancel: Callback<()>, on_done: Callback<()>) -> View {
    let actions = Stack::horizontal(space::S)
        .child("cancel", button("Cancel", false, true, on_cancel))
        .child("done", button("Done", true, true, on_done));
    panel(title, Some(actions.into()), body)
}
