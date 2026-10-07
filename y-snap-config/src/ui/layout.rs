//! Layout primitives. Each one exists because a raw `windows-reactor`
//! panel used directly kept producing the same bug:
//!
//! | Primitive | Replaces | The bug it removes |
//! | --- | --- | --- |
//! | [`Fill`] | per-control alignment | `ComboBox`/`Button`/`ToggleButton` default to `Left` (and `ComboBox` to `Top`), so they never fill their cell and rows don't line up. |
//! | [`Stack`] | `StackPanel` + hand-numbered keys | optional children shifting every later child's key. |
//! | [`AutoGrid`] | `Grid` of `STAR` columns, `VariableSizedWrapGrid` | a fixed column count squeezing on a narrow window; `VariableSizedWrapGrid`'s fixed item *height* clipping cards whose text wraps. |
//! | [`Split`] | a two-column `Grid` branched on window width | the "wide" branch being picked while the content area was narrow. |
//! | [`cell`] | `.grid_row()/.grid_column()` on a finished `View` | not compiling — placement needs a native control, a `View` isn't one. |
//! | [`scroll_y`] | `ScrollViewer` set up by hand | both axes scrolling, or neither. |
//!
//! Everything here is app-agnostic: no `Msg`, no `ConfigApp`. Callers pass
//! finished `View`s and a [`Viewport`].

use crate::ui::tokens::space;
use crate::ui::viewport::Viewport;
use windows_reactor::*;

/// Pre-alignment for a control that sits in a cell (table, form grid,
/// card). WinUI's native defaults disagree per control type —
/// `TextBox`/`NumberBox` stretch, `ComboBox`/`Button`/`ToggleButton`/
/// `ToggleSwitch` hug their content at the left — so a row mixing them is
/// ragged unless every control is told the same thing. Every control
/// constructor in [`crate::ui::controls`] already calls this; call it
/// yourself only on a raw `windows-reactor` control.
pub trait Fill: LayoutControl + Sized {
    /// Fill the cell's width; centre in its height.
    fn fill(self) -> Self {
        self.horizontal_alignment(HorizontalAlignment::Stretch).vertical_alignment(VerticalAlignment::Center)
    }
}

impl<T: LayoutControl> Fill for T {}

/// Places `view` at (`row`, `column`) of the parent `Grid`, by wrapping it
/// in a transparent `Border` — the "wrap a composed child in a native
/// control" fix `docs/windows-reactor.md` §2 describes. The `Border`
/// stretches to the whole cell; `view` fills it if `view` itself stretches
/// (anything from [`crate::ui::controls`] does).
pub fn cell(view: impl Into<View>, row: usize, column: usize) -> View {
    Border::new().grid_row(row as i32).grid_column(column as i32).content(view)
}

/// [`cell`] spanning `columns` columns.
pub fn cell_span(view: impl Into<View>, row: usize, column: usize, columns: usize) -> View {
    Border::new().grid_row(row as i32).grid_column(column as i32).grid_column_span(columns.max(1) as i32).content(view)
}

/// A vertical or horizontal run of keyed children.
///
/// Keys are the caller's, not positional: a child that only sometimes
/// exists (an open editor, an error banner) doesn't shift the keys of the
/// ones after it, so their native state (focus, a half-typed value, an
/// open dropdown) survives it appearing and disappearing.
pub struct Stack {
    orientation: Orientation,
    spacing: f64,
    children: Vec<(Key, View)>,
}

impl Stack {
    pub fn vertical(spacing: f64) -> Self {
        Self { orientation: Orientation::Vertical, spacing, children: Vec::new() }
    }

    pub fn horizontal(spacing: f64) -> Self {
        Self { orientation: Orientation::Horizontal, spacing, children: Vec::new() }
    }

    pub fn child(mut self, key: impl Into<Key>, view: impl Into<View>) -> Self {
        self.children.push((key.into(), view.into()));
        self
    }

    /// A child that is there only when `view` is `Some`.
    pub fn maybe(mut self, key: impl Into<Key>, view: Option<View>) -> Self {
        if let Some(view) = view {
            self.children.push((key.into(), view));
        }
        self
    }

    pub fn children<K: Into<Key>, V: Into<View>>(mut self, children: impl IntoIterator<Item = (K, V)>) -> Self {
        self.children.extend(children.into_iter().map(|(key, view)| (key.into(), view.into())));
        self
    }
}

impl From<Stack> for View {
    fn from(stack: Stack) -> Self {
        StackPanel::new().orientation(stack.orientation).spacing(stack.spacing).keyed_children(stack.children)
    }
}

/// Equal-width items, as many per row as the [`Viewport`] fits — the
/// replacement for every fixed `[STAR; N]` grid and for
/// `VariableSizedWrapGrid`.
///
/// Items wrap to fewer columns as space shrinks (never narrower than
/// `min_item_width`, down to a single column), and the row count follows.
/// Rows are `Auto`, so an item that grows taller (wrapped text, an expanded
/// section) pushes its row down instead of being clipped. All items in one
/// grid share one column width, so labels of different lengths never make
/// their fields different widths.
///
/// Never more columns than items: three gesture cards on a wide window are
/// three wide cards, not three narrow ones and two empty slots.
pub struct AutoGrid {
    min_item_width: f64,
    max_columns: usize,
    column_gap: f64,
    row_gap: f64,
    items: Vec<(Key, View)>,
}

impl AutoGrid {
    pub fn new(min_item_width: f64) -> Self {
        Self { min_item_width, max_columns: 12, column_gap: space::M, row_gap: space::M, items: Vec::new() }
    }

    pub fn max_columns(mut self, columns: usize) -> Self {
        self.max_columns = columns;
        self
    }

    pub fn gap(mut self, column: f64, row: f64) -> Self {
        self.column_gap = column;
        self.row_gap = row;
        self
    }

    /// How many columns `count` items get in `viewport`.
    pub fn columns_for(&self, viewport: Viewport, count: usize) -> usize {
        viewport.columns(self.min_item_width, self.column_gap, self.max_columns).min(count.max(1))
    }

    /// The viewport each of `count` items will get — for items whose own
    /// content is responsive (a card with a grid inside). Ask before
    /// building the items, then pass the same `count` worth of items.
    pub fn item_viewport(&self, viewport: Viewport, count: usize) -> Viewport {
        viewport.column(self.columns_for(viewport, count), self.column_gap)
    }

    pub fn item(mut self, key: impl Into<Key>, view: impl Into<View>) -> Self {
        self.items.push((key.into(), view.into()));
        self
    }

    pub fn items<K: Into<Key>, V: Into<View>>(mut self, items: impl IntoIterator<Item = (K, V)>) -> Self {
        self.items.extend(items.into_iter().map(|(key, view)| (key.into(), view.into())));
        self
    }

    pub fn build(self, viewport: Viewport) -> View {
        let columns = self.columns_for(viewport, self.items.len());
        let rows = self.items.len().div_ceil(columns).max(1);
        let cells: Vec<(Key, View)> = self
            .items
            .into_iter()
            .enumerate()
            .map(|(index, (key, view))| (key, cell(view, index / columns, index % columns)))
            .collect();

        Grid::new()
            .columns(vec![GridLength::STAR; columns])
            .rows(vec![GridLength::Auto; rows])
            .column_spacing(self.column_gap)
            .row_spacing(self.row_gap)
            .keyed_children(cells)
    }
}

/// A primary/secondary two-pane layout (editor + live preview): side by
/// side when *both* panes get at least their minimum width, stacked
/// (primary first) otherwise.
///
/// Decide first, build second — the panes' content usually depends on the
/// width it gets, so ask [`Split::primary`]/[`Split::secondary`] for each
/// pane's [`Viewport`], build the panes with those, then [`Split::build`].
pub struct Split {
    viewport: Viewport,
    primary_min: f64,
    secondary_min: f64,
    primary_weight: f64,
    secondary_weight: f64,
    gap: f64,
}

impl Split {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            viewport,
            primary_min: 0.0,
            secondary_min: 0.0,
            primary_weight: 1.0,
            secondary_weight: 1.0,
            gap: space::XL,
        }
    }

    pub fn min_widths(mut self, primary: f64, secondary: f64) -> Self {
        self.primary_min = primary;
        self.secondary_min = secondary;
        self
    }

    /// The panes' share of the width when side by side (`Star` weights).
    pub fn weights(mut self, primary: f64, secondary: f64) -> Self {
        self.primary_weight = primary;
        self.secondary_weight = secondary;
        self
    }

    fn shares(&self) -> (f64, f64) {
        let available = (self.viewport.width - self.gap).max(0.0);
        let total = self.primary_weight + self.secondary_weight;
        (available * self.primary_weight / total, available * self.secondary_weight / total)
    }

    pub fn is_side_by_side(&self) -> bool {
        let (primary, secondary) = self.shares();
        primary >= self.primary_min && secondary >= self.secondary_min
    }

    pub fn primary(&self) -> Viewport {
        if self.is_side_by_side() { Viewport::new(self.shares().0) } else { self.viewport }
    }

    pub fn secondary(&self) -> Viewport {
        if self.is_side_by_side() { Viewport::new(self.shares().1) } else { self.viewport }
    }

    pub fn build(self, primary: View, secondary: View) -> View {
        // Same keys in both arrangements, so switching between them moves
        // the panes instead of recreating them (an open `ColorPicker` or a
        // focused field survives the window crossing the threshold).
        if self.is_side_by_side() {
            Grid::new()
                .columns([GridLength::Star(self.primary_weight), GridLength::Star(self.secondary_weight)])
                .column_spacing(self.gap)
                .keyed_children([("primary", cell(primary, 0, 0)), ("secondary", cell(secondary, 0, 1))])
        } else {
            Grid::new()
                .rows([GridLength::Auto, GridLength::Auto])
                .row_spacing(self.gap)
                .keyed_children([("primary", cell(primary, 0, 0)), ("secondary", cell(secondary, 1, 0))])
        }
    }
}

/// Vertical scrolling only. The horizontal axis is disabled, which is what
/// makes the content's width finite — the precondition for text wrapping,
/// `Star` columns and `Stretch` meaning anything inside it. Content that
/// can't fit that width must reflow (an [`AutoGrid`], a
/// [`crate::ui::table::Table`]'s card mode), never rely on being cut off.
pub fn scroll_y(content: impl Into<View>) -> View {
    ScrollViewer::new()
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .content(content)
}
