//! Where everything of the search window goes: pure geometry, no Win32.
//!
//! The window is a stack, top to bottom: the **input row** (it never moves),
//! then — if there is anything to list — a **list** of rows (section headers,
//! results, the line of prefixes) that grows downwards up to a maximum height
//! and scrolls past it, then an optional **footer** of key hints. Spec §2.3.
//!
//! Sizes are written in logical pixels (100 % scale) and multiplied by the
//! monitor's scale here, so a caller only supplies the scale and the width.
//! Everything is half-open, `[left, right) × [top, bottom)`, like `RECT`.

use std::ops::Range;

/// A rectangle in window pixels, `left`/`top` inclusive, `right`/`bottom`
/// exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        (self.left..self.right).contains(&x) && (self.top..self.bottom).contains(&y)
    }

    /// Shrunk by `horizontal` on the left and right, `vertical` top and bottom.
    pub fn inset(&self, horizontal: i32, vertical: i32) -> Self {
        Self::new(
            self.left + horizontal,
            self.top + vertical,
            self.right - horizontal,
            self.bottom - vertical,
        )
    }
}

/// What a row of the list is, for its height. (What it shows is the
/// renderer's business.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    /// A small caption: `RECENT`, `BROWSE`, `APPS`…
    Header,
    /// A result, a recent launch or a section: icon, name, second line.
    Item,
    /// The line of clickable prefixes under the sections.
    Prefixes,
}

// Logical pixels, at 100 % scale.
const MIN_WIDTH: i32 = 560;
const MAX_WIDTH: i32 = 720;
/// The share of the monitor's (logical) width the window takes, in between.
const WIDTH_SHARE: f32 = 0.4;
const INPUT_HEIGHT: i32 = 56;
const HEADER_HEIGHT: i32 = 28;
const ITEM_HEIGHT: i32 = 52;
const PREFIXES_HEIGHT: i32 = 34;
const FOOTER_HEIGHT: i32 = 34;
/// Above the first row and below the last, inside the list.
const LIST_PADDING: i32 = 6;
const PADDING: i32 = 16;
const ICON: i32 = 32;
const GAP: i32 = 12;
/// The input row's search glyph.
const INPUT_ICON: i32 = 22;

/// What a layout is made from.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// The window's width in physical pixels; see [`width_for`].
    pub width: i32,
    /// The monitor's scale, `1.0` at 100 %.
    pub scale: f32,
    /// The tallest the list may be, in physical pixels; more rows scroll.
    pub max_list_height: i32,
    /// Whether the footer of key hints is shown (not while collapsed).
    pub footer: bool,
}

/// The window's width on a monitor `monitor_width` physical pixels wide at
/// `scale`: 40 % of its logical width, kept between 560 and 720 logical
/// pixels, then scaled back to physical.
pub fn width_for(monitor_width: i32, scale: f32) -> i32 {
    let logical = monitor_width as f32 / scale;
    let width = (logical * WIDTH_SHARE).clamp(MIN_WIDTH as f32, MAX_WIDTH as f32);
    (width * scale).round() as i32
}

/// The tallest a window made from `spec` can get: the input row, a full list
/// and the footer. The size to create the overlay at, since its surface
/// cannot grow later.
pub fn max_height(spec: &Spec) -> i32 {
    let footer = if spec.footer {
        scaled(FOOTER_HEIGHT, spec.scale)
    } else {
        0
    };
    scaled(INPUT_HEIGHT, spec.scale)
        + spec.max_list_height.max(scaled(ITEM_HEIGHT, spec.scale))
        + footer
}

/// One visible row and where it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    /// Index into the rows the layout was made from.
    pub index: usize,
    pub kind: RowKind,
    pub rect: Rect,
}

/// What is under a point of the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Input,
    /// A visible row, by index. Any kind: a caller that only acts on items
    /// checks [`Layout::kind_of`].
    Row(usize),
    Footer,
    /// The window, but nothing in particular (padding, the list's margins).
    Nothing,
}

#[derive(Clone, Debug)]
pub struct Layout {
    /// The window, in physical pixels.
    pub size: (i32, i32),
    pub input: Rect,
    /// The area the rows sit in, margins included. `None` with no rows.
    pub list: Option<Rect>,
    pub rows: Vec<Placed>,
    pub footer: Option<Rect>,
    /// The visible rows are `first..end` of the whole list.
    pub first: usize,
    pub end: usize,
    /// Rows exist above and below what is visible, for scroll hints.
    pub more_above: bool,
    pub more_below: bool,
    scale: f32,
}

impl Layout {
    /// Lays out `rows`, starting with row `first` (the list's scroll
    /// position). At least one row is always placed, however low
    /// `max_list_height`.
    pub fn new(spec: &Spec, rows: &[RowKind], first: usize) -> Self {
        let scale = spec.scale;
        let px = |logical: i32| scaled(logical, scale);
        let width = spec.width;

        let input = Rect::new(0, 0, width, px(INPUT_HEIGHT));
        let mut bottom = input.bottom;

        let first = first.min(rows.len().saturating_sub(1));
        let mut placed = Vec::new();
        let mut list = None;
        let mut end = first;
        if !rows.is_empty() {
            let top = bottom;
            let padding = px(LIST_PADDING);
            let mut y = top + padding;
            let limit = top + spec.max_list_height.max(0) - padding;
            for (index, &kind) in rows.iter().enumerate().skip(first) {
                let height = px(row_height(kind));
                if y + height > limit && !placed.is_empty() {
                    break;
                }
                placed.push(Placed {
                    index,
                    kind,
                    rect: Rect::new(0, y, width, y + height),
                });
                y += height;
                end = index + 1;
            }
            bottom = y + padding;
            list = Some(Rect::new(0, top, width, bottom));
        }

        let footer = spec.footer.then(|| {
            let rect = Rect::new(0, bottom, width, bottom + px(FOOTER_HEIGHT));
            bottom = rect.bottom;
            rect
        });

        Self {
            size: (width, bottom),
            input,
            list,
            rows: placed,
            footer,
            first,
            end,
            more_above: first > 0,
            more_below: end < rows.len(),
            scale,
        }
    }

    /// The visible rows' range of the whole list.
    pub fn visible(&self) -> Range<usize> {
        self.first..self.end
    }

    pub fn kind_of(&self, index: usize) -> Option<RowKind> {
        self.rows
            .iter()
            .find(|placed| placed.index == index)
            .map(|placed| placed.kind)
    }

    pub fn rect_of(&self, index: usize) -> Option<Rect> {
        self.rows
            .iter()
            .find(|placed| placed.index == index)
            .map(|placed| placed.rect)
    }

    /// What is at `(x, y)`, or `None` outside the window.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<Hit> {
        if !Rect::new(0, 0, self.size.0, self.size.1).contains(x, y) {
            return None;
        }
        if self.input.contains(x, y) {
            return Some(Hit::Input);
        }
        if self.footer.is_some_and(|footer| footer.contains(x, y)) {
            return Some(Hit::Footer);
        }
        Some(
            self.rows
                .iter()
                .find(|placed| placed.rect.contains(x, y))
                .map_or(Hit::Nothing, |placed| Hit::Row(placed.index)),
        )
    }

    /// The parts of an item row.
    pub fn item_parts(&self, row: Rect) -> ItemParts {
        let px = |logical: i32| scaled(logical, self.scale);
        let (padding, gap, icon) = (px(PADDING), px(GAP), px(ICON));

        let icon_top = row.top + (row.height() - icon) / 2;
        let icon = Rect::new(
            row.left + padding,
            icon_top,
            row.left + padding + icon,
            icon_top + icon,
        );
        let text_left = icon.right + gap;
        let middle = row.top + row.height() / 2;
        // Title above the middle, the second line below it.
        ItemParts {
            icon,
            title: Rect::new(text_left, row.top + px(6), row.right - padding, middle),
            subtitle: Rect::new(text_left, middle, row.right - padding, row.bottom - px(6)),
            trailing: Rect::new(
                row.right - padding,
                row.top,
                row.right - padding,
                row.bottom,
            ),
        }
    }

    /// The parts of the input row: the search glyph, then the text field.
    pub fn input_parts(&self) -> InputParts {
        let px = |logical: i32| scaled(logical, self.scale);
        let (padding, gap, glyph) = (px(PADDING), px(GAP), px(INPUT_ICON));

        let top = self.input.top + (self.input.height() - glyph) / 2;
        let icon = Rect::new(
            self.input.left + padding,
            top,
            self.input.left + padding + glyph,
            top + glyph,
        );
        InputParts {
            icon,
            field: Rect::new(
                icon.right + gap,
                self.input.top,
                self.input.right - padding,
                self.input.bottom,
            ),
        }
    }
}

/// Where the pieces of an item row go. `trailing` starts out empty at the
/// right padding; the renderer widens it leftwards to whatever it draws there
/// (a hotkey, a time, a ›), and ends the title and subtitle before it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemParts {
    pub icon: Rect,
    pub title: Rect,
    pub subtitle: Rect,
    pub trailing: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InputParts {
    pub icon: Rect,
    pub field: Rect,
}

/// The first row to show so that row `selected` is visible, changing
/// `first` as little as possible. Scrolling up to a row that has a header
/// directly above it shows the header too.
pub fn scroll_to_show(spec: &Spec, rows: &[RowKind], first: usize, selected: usize) -> usize {
    let selected = selected.min(rows.len().saturating_sub(1));
    if selected < first {
        return if selected > 0 && rows[selected - 1] == RowKind::Header {
            selected - 1
        } else {
            selected
        };
    }
    let mut first = first;
    while first < selected && Layout::new(spec, rows, first).end <= selected {
        first += 1;
    }
    first
}

fn row_height(kind: RowKind) -> i32 {
    match kind {
        RowKind::Header => HEADER_HEIGHT,
        RowKind::Item => ITEM_HEIGHT,
        RowKind::Prefixes => PREFIXES_HEIGHT,
    }
}

fn scaled(logical: i32, scale: f32) -> i32 {
    (logical as f32 * scale).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use RowKind::{Header, Item, Prefixes};

    fn spec(max_list_height: i32) -> Spec {
        Spec {
            width: 640,
            scale: 1.0,
            max_list_height,
            footer: true,
        }
    }

    #[test]
    fn the_width_follows_the_monitor_within_limits() {
        assert_eq!(width_for(1366, 1.0), 560, "a small screen: the minimum");
        assert_eq!(width_for(1600, 1.0), 640);
        assert_eq!(width_for(1920, 1.0), 720, "a big one: the maximum");
        assert_eq!(width_for(3840, 1.0), 720);
        // 4K at 200 %: the same window, twice the pixels.
        assert_eq!(width_for(3840, 2.0), 1440);
        assert_eq!(width_for(2560, 1.5), 1024, "1707 logical wide: 683 logical");
    }

    #[test]
    fn with_nothing_to_list_the_window_is_the_input_row_and_the_footer() {
        let layout = Layout::new(&spec(400), &[], 0);
        assert_eq!(layout.list, None);
        assert_eq!(layout.input, Rect::new(0, 0, 640, 56));
        assert_eq!(layout.footer, Some(Rect::new(0, 56, 640, 90)));
        assert_eq!(layout.size, (640, 90));

        let collapsed = Layout::new(
            &Spec {
                footer: false,
                ..spec(400)
            },
            &[],
            0,
        );
        assert_eq!(collapsed.size, (640, 56), "one row high");
        assert_eq!(collapsed.footer, None);
    }

    #[test]
    fn rows_stack_without_gaps_between_the_input_and_the_footer() {
        let rows = [Header, Item, Item, Prefixes];
        let layout = Layout::new(&spec(1000), &rows, 0);

        let list = layout.list.unwrap();
        assert_eq!(list.top, layout.input.bottom);
        let mut y = list.top + 6;
        for (placed, kind) in layout.rows.iter().zip(rows) {
            assert_eq!(placed.kind, kind);
            assert_eq!(placed.rect.top, y);
            assert_eq!((placed.rect.left, placed.rect.right), (0, 640));
            y = placed.rect.bottom;
        }
        assert_eq!(y, 56 + 6 + 28 + 52 + 52 + 34);
        assert_eq!(list.bottom, y + 6);
        assert_eq!(layout.footer.unwrap().top, list.bottom);
        assert_eq!(layout.size.1, list.bottom + 34);
        assert!(!layout.more_above && !layout.more_below);
        assert_eq!(layout.visible(), 0..4);
    }

    #[test]
    fn a_long_list_is_capped_and_says_there_is_more() {
        let rows = vec![Item; 20];
        // Room for six items: 6 padding + 6 × 52 + 6 padding.
        let layout = Layout::new(&spec(6 + 6 * 52 + 6), &rows, 0);
        assert_eq!(layout.rows.len(), 6);
        assert_eq!(layout.list.unwrap().height(), 6 + 6 * 52 + 6);
        assert!(layout.more_below && !layout.more_above);

        let scrolled = Layout::new(&spec(6 + 6 * 52 + 6), &rows, 10);
        assert_eq!(scrolled.visible(), 10..16);
        assert!(scrolled.more_above && scrolled.more_below);

        let last = Layout::new(&spec(6 + 6 * 52 + 6), &rows, 99);
        assert_eq!(
            last.first, 19,
            "scrolling past the end stops at the last row"
        );
        assert_eq!(last.rows.len(), 1);
    }

    #[test]
    fn no_layout_is_taller_than_the_maximum() {
        let spec = spec(6 + 6 * 52 + 6);
        let tallest = max_height(&spec);
        for rows in [
            vec![],
            vec![Item],
            vec![Item; 3],
            vec![Header, Item, Item, Prefixes],
            vec![Item; 50],
            vec![Header; 50],
        ] {
            for first in [0, 1, 7] {
                let height = Layout::new(&spec, &rows, first).size.1;
                assert!(
                    height <= tallest,
                    "{height} > {tallest} for {} rows",
                    rows.len()
                );
            }
        }
        // The full list reaches it, to the pixel.
        assert_eq!(Layout::new(&spec, &[Item; 50], 0).size.1, tallest);

        let collapsed = Spec {
            footer: false,
            ..spec
        };
        assert_eq!(max_height(&collapsed), tallest - 34);
    }

    #[test]
    fn at_least_one_row_is_placed_however_little_room() {
        let layout = Layout::new(&spec(0), &[Item, Item], 0);
        assert_eq!(layout.rows.len(), 1);
    }

    #[test]
    fn scaling_multiplies_every_size() {
        let at = |scale: f32| {
            let spec = Spec {
                scale,
                width: (640.0 * scale) as i32,
                ..spec(10_000)
            };
            Layout::new(&spec, &[Header, Item], 0)
        };
        let (one, two) = (at(1.0), at(2.0));
        assert_eq!(two.input.height(), 2 * one.input.height());
        assert_eq!(two.size.1, 2 * one.size.1);
        let odd = at(1.25);
        assert_eq!(odd.input.height(), 70);
        assert_eq!(odd.rows[0].rect.height(), 35);
    }

    #[test]
    fn hit_testing_finds_the_part_under_a_point() {
        let rows = [Header, Item, Item];
        let layout = Layout::new(&spec(1000), &rows, 0);
        let item = layout.rect_of(1).unwrap();

        assert_eq!(layout.hit_test(10, 10), Some(Hit::Input));
        assert_eq!(
            layout.hit_test(10, layout.input.bottom - 1),
            Some(Hit::Input)
        );
        assert_eq!(
            layout.hit_test(10, layout.input.bottom),
            Some(Hit::Nothing),
            "list margin"
        );
        assert_eq!(layout.hit_test(100, item.top), Some(Hit::Row(1)));
        assert_eq!(layout.hit_test(100, item.bottom - 1), Some(Hit::Row(1)));
        assert_eq!(
            layout.hit_test(100, item.bottom),
            Some(Hit::Row(2)),
            "half open"
        );
        assert_eq!(layout.kind_of(0), Some(Header));
        assert_eq!(
            layout.hit_test(100, layout.rect_of(0).unwrap().top),
            Some(Hit::Row(0))
        );
        assert_eq!(
            layout.hit_test(5, layout.footer.unwrap().top + 1),
            Some(Hit::Footer)
        );

        assert_eq!(layout.hit_test(-1, 10), None);
        assert_eq!(layout.hit_test(640, 10), None);
        assert_eq!(layout.hit_test(10, layout.size.1), None);
    }

    #[test]
    fn a_scrolled_row_is_found_by_its_list_index() {
        let rows = vec![Item; 10];
        let layout = Layout::new(&spec(6 + 3 * 52 + 6), &rows, 4);
        let top = layout.rows[0].rect;
        assert_eq!(layout.hit_test(50, top.top + 1), Some(Hit::Row(4)));
        assert_eq!(layout.rect_of(0), None, "scrolled out");
    }

    #[test]
    fn an_item_row_has_its_icon_then_two_lines_of_text() {
        let layout = Layout::new(&spec(1000), &[Item], 0);
        let row = layout.rows[0].rect;
        let parts = layout.item_parts(row);

        assert_eq!(parts.icon.width(), 32);
        assert_eq!(parts.icon.height(), 32);
        assert_eq!(parts.icon.left, 16);
        assert_eq!(
            parts.icon.top - row.top,
            row.bottom - parts.icon.bottom,
            "centred vertically"
        );
        assert!(parts.title.left > parts.icon.right);
        assert_eq!(parts.title.left, parts.subtitle.left);
        assert!(parts.title.bottom <= parts.subtitle.top);
        assert!(parts.subtitle.bottom <= row.bottom);
        assert_eq!(parts.title.right, 640 - 16);
        assert_eq!(parts.trailing.width(), 0);
    }

    #[test]
    fn the_input_row_has_a_glyph_then_a_field() {
        let layout = Layout::new(&spec(1000), &[], 0);
        let parts = layout.input_parts();
        assert!(parts.icon.right < parts.field.left);
        assert_eq!(parts.field.top, 0);
        assert_eq!(parts.field.bottom, 56);
        assert_eq!(parts.field.right, 640 - 16);
        assert_eq!(parts.icon.top, 56 - parts.icon.bottom, "centred in the row");
    }

    #[test]
    fn scrolling_to_a_row_moves_the_list_as_little_as_possible() {
        let mut rows = vec![Header];
        rows.extend([Item; 9]);
        rows.push(Header);
        rows.extend([Item; 5]);
        let spec = spec(6 + 4 * 52 + 6);

        // Already visible: unchanged.
        assert_eq!(scroll_to_show(&spec, &rows, 0, 2), 0);
        // Just below the window: one step brings it in at the bottom.
        let shown = Layout::new(&spec, &rows, 0).end;
        let first = scroll_to_show(&spec, &rows, 0, shown);
        assert!(first > 0);
        let layout = Layout::new(&spec, &rows, first);
        assert!(layout.visible().contains(&shown));
        assert!(
            !Layout::new(&spec, &rows, first - 1)
                .visible()
                .contains(&shown),
            "and no further than needed"
        );
        // Far down and back up.
        let down = scroll_to_show(&spec, &rows, 0, rows.len() - 1);
        assert!(
            Layout::new(&spec, &rows, down)
                .visible()
                .contains(&(rows.len() - 1))
        );
        // Up to a row under a header: the header comes with it.
        assert_eq!(
            scroll_to_show(&spec, &rows, 12, 11),
            10,
            "row 10 is a header"
        );
        assert_eq!(scroll_to_show(&spec, &rows, 5, 3), 3);
    }
}
