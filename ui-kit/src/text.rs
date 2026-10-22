//! Text styles — the type ramp from [`crate::tokens`] as named roles,
//! so no screen picks a font size, weight or opacity itself.
//!
//! Each returns the concrete [`TextBlock`] builder, not a `View`, so a
//! caller can still chain `.grid_row()`, `.margin()`, `.foreground()`…

use crate::tokens::{emphasis, font};
use windows_reactor::*;

/// A page title.
pub fn title(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::SUBTITLE)
        .font_weight(FontWeight::SEMI_BOLD)
}

/// The sentence under a page title. Wraps.
pub fn lede(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::BODY)
        .opacity(emphasis::MUTED)
        .text_wrapping(TextWrapping::WrapWholeWords)
}

/// Ordinary text. Wraps.
pub fn body(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::BODY)
        .text_wrapping(TextWrapping::WrapWholeWords)
}

/// Emphasised body text — a card's title.
pub fn strong(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::BODY)
        .font_weight(FontWeight::SEMI_BOLD)
}

/// A field label, column header or secondary note. Wraps on whole words
/// rather than trimming: a header like "Steal from shell" over a narrow
/// column reads better on two lines than as "Steal fr…".
pub fn caption(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::CAPTION)
        .opacity(emphasis::MUTED)
        .text_wrapping(TextWrapping::WrapWholeWords)
}

/// A section heading inside a page ("Layout", "Palette").
pub fn section_title(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::CAPTION)
        .font_weight(FontWeight::SEMI_BOLD)
        .opacity(emphasis::MUTED)
}

/// Single-line text that ends in "…" rather than wrapping or overflowing —
/// for status lines and labels inside fixed-height things.
pub fn single_line(text: impl Into<String>) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font::BODY)
        .text_wrapping(TextWrapping::NoWrap)
        .text_trimming(TextTrimming::CharacterEllipsis)
}

/// The "nothing here" placeholder ("—") for a cell that doesn't apply.
pub fn placeholder() -> TextBlock {
    TextBlock::new()
        .text("—")
        .font_size(font::BODY)
        .opacity(emphasis::SUBTLE)
        .vertical_alignment(VerticalAlignment::Center)
}
