//! [`Viewport`]: the width a piece of UI has to lay itself out in, passed
//! *down* the view tree the way CSS container queries work — not read from
//! the window by whoever happens to need it.
//!
//! `windows-reactor` has no breakpoints, no container queries and no
//! layout callback (`docs/windows-reactor.md` §5): the one signal there is,
//! is the window's client size from `ViewContext::on_window_size`. A screen
//! comparing that to a threshold is how the old `NARROW_WIDTH` broke — the
//! window was "wide" while the Overlay workspace, after the 200px nav pane,
//! page margins and a 55/45 split, had a third of that to work with.
//!
//! So: the shell turns the window width into the width the page content
//! actually gets (`shell::content_viewport` in the app), and every container
//! that divides space hands each child *its* share — [`Viewport::inset`]
//! for padding, [`Viewport::column`] for a grid cell,
//! [`crate::layout::Split`] for a two-pane layout. A component deciding
//! "how many columns", "table or cards", "side by side or stacked" asks the
//! `Viewport` it was given, and gets an answer about its own box.
//!
//! **The numbers only drive discrete decisions.** Actual sizing is still
//! WinUI's: equal `Star` columns and `Stretch` alignment fill whatever space
//! really exists. A `Viewport` that is off by a few pixels (an unaccounted
//! border, a scrollbar) only moves a threshold slightly — which is why
//! every decision compares against honest *minimum* widths with some slack,
//! never an exact fit.

/// A box's usable width in DIPs. Copy it around freely; it's one `f64`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub width: f64,
}

/// Coarse size classes, for decisions that are about the overall density of
/// a screen (page padding, pane mode) rather than one component's fit —
/// a component that knows its own minimum width should compare against that
/// with [`Viewport::fits`] instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeClass {
    /// Under 560 DIPs: one column of everything.
    Compact,
    /// 560–900 DIPs.
    Medium,
    /// 900 DIPs and up.
    Expanded,
}

impl Viewport {
    pub const fn new(width: f64) -> Self {
        Self { width }
    }

    pub fn size_class(self) -> SizeClass {
        if self.width < 560.0 {
            SizeClass::Compact
        } else if self.width < 900.0 {
            SizeClass::Medium
        } else {
            SizeClass::Expanded
        }
    }

    /// Whether `minimum` DIPs of content fit.
    pub fn fits(self, minimum: f64) -> bool {
        self.width >= minimum
    }

    /// The viewport inside `horizontal` DIPs of padding/border on *each*
    /// side — what a card hands its content.
    pub fn inset(self, horizontal: f64) -> Self {
        Self::new((self.width - 2.0 * horizontal).max(0.0))
    }

    /// How many equal columns of at least `min_item` DIPs, separated by
    /// `gap`, fit — clamped to `1..=max_columns`. One column always "fits":
    /// on a window narrower than one item, that item gets the whole width
    /// and its own content has to cope (wrap, trim, or stack further).
    pub fn columns(self, min_item: f64, gap: f64, max_columns: usize) -> usize {
        let fit = ((self.width + gap) / (min_item + gap)).floor();
        (fit.max(1.0) as usize).min(max_columns.max(1))
    }

    /// The viewport of one of `columns` equal columns separated by `gap`.
    pub fn column(self, columns: usize, gap: f64) -> Self {
        let columns = columns.max(1) as f64;
        Self::new(((self.width - gap * (columns - 1.0)) / columns).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_clamped_to_at_least_one_and_at_most_the_maximum() {
        assert_eq!(Viewport::new(50.0).columns(100.0, 10.0, 4), 1);
        assert_eq!(Viewport::new(210.0).columns(100.0, 10.0, 4), 2);
        assert_eq!(Viewport::new(209.0).columns(100.0, 10.0, 4), 1);
        assert_eq!(Viewport::new(5000.0).columns(100.0, 10.0, 4), 4);
    }

    #[test]
    fn a_column_is_its_share_after_the_gaps() {
        assert_eq!(Viewport::new(320.0).column(3, 10.0), Viewport::new(100.0));
        assert_eq!(Viewport::new(100.0).inset(10.0), Viewport::new(80.0));
        assert_eq!(Viewport::new(10.0).inset(10.0), Viewport::new(0.0));
    }
}
