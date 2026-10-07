//! [`Table`]: rows of aligned cells under a header — or, when its columns
//! can't fit the [`Viewport`], the same rows as a stack of label/value
//! cards. Either way, no horizontal scrolling and nothing off-window.
//!
//! What it fixes, relative to the old table (fixed-pixel columns in a
//! sideways `ScrollViewer`):
//! - **Columns flex.** [`Column::flex`] columns share the width left after
//!   [`Column::fixed`] ones, in proportion to their minimums, so the table
//!   always fills the page instead of being its fixed pixel sum.
//! - **It reflows below its minimum.** Under [`Table::min_width`] each row
//!   becomes a card listing `header: cell` pairs — the table's narrow
//!   layout, not a clipped wide one.
//! - **Cells line up.** Every cell is wrapped in a vertically centred
//!   `Border` spanning its column, so a 32px `ComboBox`, a 20px
//!   `ToggleSwitch` and a line of text sit on one centre line, and controls
//!   from [`crate::ui::controls`] fill their column.
//! - **Rows can open in place.** [`Row::detail`] spans the whole row
//!   beneath it — where an editor for that row goes, instead of at the top
//!   of the page far from the row it edits.
//! - **Keys are the table's job.** Cells are keyed `"{row}/{column}"` from
//!   the row's own key; no `row * 8 + column` arithmetic per screen.

use crate::ui::layout::{Stack, cell_span};
use crate::ui::surfaces::{CARD_INSET, card};
use crate::ui::text;
use crate::ui::tokens::{size, space};
use crate::ui::viewport::Viewport;
use windows_reactor::*;

/// How a column sizes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Width {
    Fixed(f64),
    /// Shares the leftover width in proportion to `min`; never below it.
    Flex(f64),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Column {
    header: &'static str,
    width: Width,
}

impl Column {
    /// Exactly `width` DIPs — icons, switches, short numbers.
    pub const fn fixed(header: &'static str, width: f64) -> Self {
        Self { header, width: Width::Fixed(width) }
    }

    /// At least `min` DIPs, growing with the table — anything with text.
    pub const fn flex(header: &'static str, min: f64) -> Self {
        Self { header, width: Width::Flex(min) }
    }

    /// A headerless trailing column for row actions (delete, edit). In card
    /// layout its cell moves to the card's top-right corner.
    pub const fn actions(width: f64) -> Self {
        Self::fixed("", width)
    }

    fn min(self) -> f64 {
        match self.width {
            Width::Fixed(width) | Width::Flex(width) => width,
        }
    }

    fn track(self) -> GridLength {
        match self.width {
            Width::Fixed(width) => GridLength::Pixel(width),
            Width::Flex(min) => GridLength::Star(min),
        }
    }

    fn is_actions(self) -> bool {
        self.header.is_empty()
    }
}

/// One row: exactly one cell per [`Column`], in column order, plus an
/// optional full-width detail view under it.
pub struct Row {
    key: String,
    cells: Vec<View>,
    detail: Option<View>,
}

impl Row {
    pub fn new(key: impl Into<String>, cells: Vec<View>) -> Self {
        Self { key: key.into(), cells, detail: None }
    }

    /// A view spanning the whole row, directly beneath it — an open editor.
    pub fn detail(mut self, detail: Option<View>) -> Self {
        self.detail = detail;
        self
    }
}

pub struct Table<'a> {
    columns: &'a [Column],
    rows: Vec<Row>,
    column_gap: f64,
    empty: &'static str,
}

impl<'a> Table<'a> {
    pub fn new(columns: &'a [Column]) -> Self {
        Self { columns, rows: Vec::new(), column_gap: space::M, empty: "Nothing here yet." }
    }

    pub fn rows(mut self, rows: impl IntoIterator<Item = Row>) -> Self {
        self.rows.extend(rows);
        self
    }

    /// What to say instead of an empty table.
    pub fn empty_message(mut self, message: &'static str) -> Self {
        self.empty = message;
        self
    }

    /// The narrowest viewport the grid layout fits in; below it, cards.
    /// Flex columns grow in proportion to their minimums, so they all reach
    /// their minimum at the same moment — the sum is exact.
    pub fn min_width(&self) -> f64 {
        let columns: f64 = self.columns.iter().map(|column| column.min()).sum();
        columns + self.column_gap * (self.columns.len().saturating_sub(1)) as f64
    }

    /// The viewport a [`Row::detail`] gets, for a detail whose own layout
    /// is responsive (the key picker). Ask before building rows.
    pub fn detail_viewport(&self, viewport: Viewport) -> Viewport {
        if viewport.fits(self.min_width()) { viewport } else { viewport.inset(CARD_INSET) }
    }

    pub fn build(self, viewport: Viewport) -> View {
        if self.rows.is_empty() {
            return text::caption(self.empty).into();
        }
        if viewport.fits(self.min_width()) { self.grid() } else { self.cards() }
    }

    fn grid(self) -> View {
        let column_count = self.columns.len();
        let mut tracks = vec![GridLength::Auto];
        let mut children: Vec<(String, View)> = Vec::new();

        children.push(("header/rule".to_owned(), rule(0, 1, column_count)));
        for (column, spec) in self.columns.iter().enumerate() {
            let header = text::caption(spec.header).margin(Thickness::new(0.0, 0.0, 0.0, space::S));
            children.push((format!("header/{column}"), cell_span(header, 0, column, 1)));
        }

        for row in self.rows {
            let line = tracks.len();
            let spans = 1 + usize::from(row.detail.is_some());
            tracks.extend(std::iter::repeat_n(GridLength::Auto, spans));

            // The rule goes in first so it paints under the cells.
            children.push((format!("{}/rule", row.key), rule(line, spans, column_count)));
            debug_assert_eq!(row.cells.len(), column_count, "one cell per column");
            for (column, view) in row.cells.into_iter().enumerate() {
                children.push((format!("{}/{column}", row.key), centred_cell(view, line, column)));
            }
            if let Some(detail) = row.detail {
                let detail = Border::new()
                    .grid_row(line as i32 + 1)
                    .grid_column(0)
                    .grid_column_span(column_count as i32)
                    .padding(Thickness::new(0.0, 0.0, 0.0, space::M))
                    .content(detail);
                children.push((format!("{}/detail", row.key), detail));
            }
        }

        let columns: Vec<GridLength> = self.columns.iter().map(|column| column.track()).collect();
        Grid::new().columns(columns).rows(tracks).column_spacing(self.column_gap).keyed_children(children)
    }

    fn cards(self) -> View {
        let columns = self.columns;
        Stack::vertical(space::S)
            .children(self.rows.into_iter().map(|row| {
                let key = row.key.clone();
                (key, row_card(columns, row))
            }))
            .into()
    }
}

/// A row's cell, centred on the row's centre line and filling its column.
fn centred_cell(view: View, row: usize, column: usize) -> View {
    Border::new()
        .grid_row(row as i32)
        .grid_column(column as i32)
        .vertical_alignment(VerticalAlignment::Center)
        .padding(Thickness::xy(0.0, space::S - 2.0))
        .content(view)
}

/// A hairline along the bottom of `spans` rows starting at `row`, under
/// every column — the row separator.
fn rule(row: usize, spans: usize, columns: usize) -> View {
    Border::new()
        .grid_row(row as i32)
        .grid_row_span(spans as i32)
        .grid_column(0)
        .grid_column_span(columns as i32)
        .border_thickness(Thickness::new(0.0, 0.0, 0.0, size::STROKE))
        .border_brush(ThemeBrush::CardStroke)
        .into()
}

/// One row as a card: a `header | value` grid, the actions cell in the top
/// right, the detail underneath.
fn row_card(columns: &[Column], row: Row) -> View {
    let mut children: Vec<(String, View)> = Vec::new();
    let mut line = 0;
    let mut actions = None;

    for (column, view) in columns.iter().zip(row.cells) {
        if column.is_actions() {
            actions = Some(view);
            continue;
        }
        let label = text::caption(column.header).vertical_alignment(VerticalAlignment::Center);
        children.push((format!("{line}/label"), cell_span(label, line, 0, 1)));
        // The first value shares its line with the actions; the rest span
        // both value columns.
        let span = if line == 0 { 1 } else { 2 };
        children.push((format!("{line}/value"), cell_span(view, line, 1, span)));
        line += 1;
    }
    if let Some(actions) = actions {
        children.push(("actions".to_owned(), cell_span(actions, 0, 2, 1)));
    }
    if let Some(detail) = row.detail {
        children.push(("detail".to_owned(), cell_span(detail, line, 0, 3)));
        line += 1;
    }

    let body: View = Grid::new()
        .columns([GridLength::Auto, GridLength::STAR, GridLength::Auto])
        .rows(vec![GridLength::Auto; line.max(1)])
        .column_spacing(space::M)
        .row_spacing(space::S)
        .keyed_children(children);
    card().content(body)
}
