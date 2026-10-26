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
// The action menu.
const MENU_WIDTH: i32 = 300;
const MENU_ITEM_HEIGHT: i32 = 34;
const MENU_PADDING: i32 = 6;
/// From the window's right edge.
const MENU_MARGIN: i32 = 14;
/// How far the menu tucks under (or over) the row it belongs to.
const MENU_OVERLAP: i32 = 6;

// The buttons on the selected row.
const BUTTON_HEIGHT: i32 = 26;
const BUTTON_GAP: i32 = 4;
/// Either side of a button's label.
const BUTTON_PADDING: i32 = 10;
/// The buttons may take this share of the row's width, no more.
const BUTTONS_SHARE: f32 = 0.5;

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
    /// The **⌄** of a folded box.
    Chevron,
    /// An action button of the selected row, by index into its buttons.
    Button(usize),
    /// The **⋯** that ends the selected row.
    More,
    /// An item of the action menu, by index.
    Menu(usize),
    /// The action menu, between its items.
    MenuPanel,
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
    /// The action menu, when one is open ([`Layout::with_menu`]).
    pub menu: Option<MenuLayout>,
    /// The selected row's buttons ([`Layout::with_row_buttons`]).
    pub buttons: Option<RowButtons>,
    /// The **⌄** that unfolds a folded box, at the input row's right end
    /// ([`Layout::with_chevron`]).
    pub chevron: Option<Rect>,
    scale: f32,
}

/// Where the selected row's buttons are: its action buttons in order, then
/// the **⋯**, flush with the row's right padding.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RowButtons {
    pub row: usize,
    /// As many of the action buttons as fit, in order.
    pub items: Vec<Rect>,
    pub more: Rect,
    /// Where the strip starts: the row's texts end a gap before it.
    pub left: i32,
}

/// Where the action menu and its items are.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MenuLayout {
    pub rect: Rect,
    pub items: Vec<Rect>,
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
            menu: None,
            buttons: None,
            chevron: None,
            scale,
        }
    }

    /// This layout with the **⌄** that unfolds a folded box, ending the input
    /// row; the text field stops before it.
    pub fn with_chevron(mut self) -> Self {
        let px = |logical: i32| scaled(logical, self.scale);
        let size = px(BUTTON_HEIGHT);
        let right = self.input.right - px(PADDING);
        let top = self.input.top + (self.input.height() - size) / 2;
        self.chevron = Some(Rect::new(right - size, top, right, top + size));
        self
    }

    /// This layout with buttons on the row at `row` (an index into the whole
    /// list, which must be visible): one for each of `widths` — the width of
    /// each label, in physical pixels — then the **⋯**. When they would take
    /// more than half the row, the last labelled ones are left out; they stay
    /// in the action menu.
    pub fn with_row_buttons(mut self, row: usize, widths: &[i32]) -> Self {
        let Some(rect) = self.rect_of(row) else {
            return self;
        };
        let px = |logical: i32| scaled(logical, self.scale);
        let (height, gap) = (px(BUTTON_HEIGHT), px(BUTTON_GAP));
        let more = height;
        let budget = (rect.width() as f32 * BUTTONS_SHARE) as i32;
        let widths: Vec<i32> = widths
            .iter()
            .map(|&label| label + 2 * px(BUTTON_PADDING))
            .collect();

        let mut taken = 0;
        let mut total = more;
        for &width in &widths {
            if total + gap + width > budget {
                break;
            }
            total += gap + width;
            taken += 1;
        }

        let right = rect.right - px(PADDING);
        let top = rect.top + (rect.height() - height) / 2;
        let left = right - total;
        let mut x = left;
        let items = widths[..taken]
            .iter()
            .map(|&width| {
                let button = Rect::new(x, top, x + width, top + height);
                x += width + gap;
                button
            })
            .collect();
        self.buttons = Some(RowButtons {
            row,
            items,
            more: Rect::new(x, top, x + more, top + height),
            left,
        });
        self
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
        if let Some(menu) = &self.menu
            && menu.rect.contains(x, y)
        {
            return Some(
                menu.items
                    .iter()
                    .position(|item| item.contains(x, y))
                    .map_or(Hit::MenuPanel, Hit::Menu),
            );
        }
        if self.chevron.is_some_and(|chevron| chevron.contains(x, y)) {
            return Some(Hit::Chevron);
        }
        if self.input.contains(x, y) {
            return Some(Hit::Input);
        }
        if self.footer.is_some_and(|footer| footer.contains(x, y)) {
            return Some(Hit::Footer);
        }
        if let Some(buttons) = &self.buttons {
            if buttons.more.contains(x, y) {
                return Some(Hit::More);
            }
            if let Some(n) = buttons.items.iter().position(|b| b.contains(x, y)) {
                return Some(Hit::Button(n));
            }
        }
        Some(
            self.rows
                .iter()
                .find(|placed| placed.rect.contains(x, y))
                .map_or(Hit::Nothing, |placed| Hit::Row(placed.index)),
        )
    }

    /// This layout with the action menu of `count` items open on the row at
    /// `anchor`: a panel at the window's right edge, tucked under the row, or
    /// over it if it would pass `limit` (the tallest the window can be). The
    /// window grows to hold it.
    pub fn with_menu(mut self, anchor: Rect, count: usize, limit: i32) -> Self {
        let px = |logical: i32| scaled(logical, self.scale);
        let width = px(MENU_WIDTH).min(self.size.0 - 2 * px(MENU_MARGIN));
        let height = px(MENU_ITEM_HEIGHT) * count as i32 + 2 * px(MENU_PADDING);
        let right = self.size.0 - px(MENU_MARGIN);

        let mut top = anchor.bottom - px(MENU_OVERLAP);
        if top + height > limit {
            top = anchor.top + px(MENU_OVERLAP) - height;
        }
        top = top.min(limit - height).max(self.input.bottom);

        let rect = Rect::new(right - width, top, right, top + height);
        let items = (0..count as i32)
            .map(|n| {
                let item_top = rect.top + px(MENU_PADDING) + n * px(MENU_ITEM_HEIGHT);
                Rect::new(
                    rect.left + px(MENU_PADDING),
                    item_top,
                    rect.right - px(MENU_PADDING),
                    item_top + px(MENU_ITEM_HEIGHT),
                )
            })
            .collect();
        self.size.1 = self.size.1.max(rect.bottom);
        self.menu = Some(MenuLayout { rect, items });
        self
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
                self.chevron
                    .map_or(self.input.right - padding, |chevron| chevron.left - gap),
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
    fn the_action_menu_hangs_under_its_row_at_the_right_edge() {
        let rows = [Header, Item, Item, Item];
        let spec = spec(1000);
        let layout = Layout::new(&spec, &rows, 0);
        let row = layout.rect_of(1).unwrap();
        let limit = max_height(&spec);
        let before = layout.size;

        let layout = layout.with_menu(row, 3, limit);
        let menu = layout.menu.clone().unwrap();

        assert_eq!(menu.rect.right, 640 - 14, "at the right edge");
        assert_eq!(menu.rect.width(), 300);
        assert_eq!(menu.rect.top, row.bottom - 6, "tucked under the row");
        assert_eq!(menu.rect.height(), 3 * 34 + 2 * 6);
        assert_eq!(menu.items.len(), 3);
        assert_eq!(menu.items[0].top, menu.rect.top + 6);
        assert!(
            menu.items
                .windows(2)
                .all(|pair| pair[0].bottom == pair[1].top)
        );
        assert!(
            menu.items
                .iter()
                .all(|item| item.left > menu.rect.left && item.right < menu.rect.right)
        );
        assert_eq!(layout.size.0, before.0);
        assert!(
            layout.size.1 >= menu.rect.bottom,
            "the window grows to hold it"
        );
    }

    #[test]
    fn near_the_bottom_the_menu_opens_upwards() {
        let rows = vec![Item; 12];
        let spec = spec(6 + 5 * 52 + 6);
        let limit = max_height(&spec);
        let layout = Layout::new(&spec, &rows, 0);
        let last = layout.rows.last().unwrap().rect;

        let layout = layout.with_menu(last, 6, limit);
        let menu = layout.menu.clone().unwrap();
        assert!(menu.rect.bottom <= limit, "{} > {limit}", menu.rect.bottom);
        assert!(
            menu.rect.top >= layout.input.bottom,
            "never over the input row"
        );
        assert!(
            menu.rect.bottom <= last.top + 6 + 1,
            "above its row, not over it"
        );
        assert!(layout.size.1 <= limit);
    }

    #[test]
    fn a_menu_taller_than_the_window_still_fits_inside_it() {
        let spec = spec(120);
        let limit = max_height(&spec);
        let layout = Layout::new(&spec, &[Item], 0);
        let row = layout.rows[0].rect;
        let layout = layout.with_menu(row, 12, limit);
        let menu = layout.menu.unwrap();
        assert_eq!(menu.rect.top, 56, "pinned under the input row");
    }

    #[test]
    fn hit_testing_finds_the_menu_before_what_is_under_it() {
        let rows = [Header, Item, Item, Item];
        let spec = spec(1000);
        let layout = Layout::new(&spec, &rows, 0);
        let row = layout.rect_of(1).unwrap();
        let layout = layout.with_menu(row, 3, max_height(&spec));
        let menu = layout.menu.clone().unwrap();

        let inside = |item: Rect| (item.left + 5, item.top + 5);
        for (n, item) in menu.items.iter().enumerate() {
            let (x, y) = inside(*item);
            assert_eq!(layout.hit_test(x, y), Some(Hit::Menu(n)));
        }
        // The panel's own padding is the menu, not the row beneath it.
        assert_eq!(
            layout.hit_test(menu.rect.left + 2, menu.rect.top + 2),
            Some(Hit::MenuPanel)
        );
        // Outside it, the window as before.
        assert_eq!(layout.hit_test(10, 10), Some(Hit::Input));
        let below_row_one = layout.rect_of(1).unwrap();
        assert_eq!(
            layout.hit_test(20, below_row_one.top + 2),
            Some(Hit::Row(1))
        );
    }

    #[test]
    fn the_menu_scales_with_the_monitor() {
        let spec = Spec {
            scale: 2.0,
            width: 1280,
            ..spec(2000)
        };
        let layout = Layout::new(&spec, &[Item, Item], 0);
        let row = layout.rows[0].rect;
        let menu = layout.with_menu(row, 2, max_height(&spec)).menu.unwrap();
        assert_eq!(menu.rect.width(), 600);
        assert_eq!(menu.items[0].height(), 68);
    }

    #[test]
    fn the_buttons_end_the_selected_row_with_the_more_button_last() {
        let layout = Layout::new(&spec(1000), &[Header, Item, Item], 0);
        let row = layout.rect_of(1).unwrap();
        let layout = layout.with_row_buttons(1, &[40, 60]);
        let buttons = layout.buttons.clone().unwrap();

        assert_eq!(buttons.row, 1);
        assert_eq!(buttons.items.len(), 2);
        assert_eq!(buttons.items[0].width(), 40 + 20);
        assert_eq!(buttons.items[1].width(), 60 + 20);
        assert_eq!(buttons.more.width(), 26, "a square");
        assert_eq!(buttons.more.right, row.right - 16, "flush with the padding");
        assert_eq!(buttons.items[0].left, buttons.left);
        assert_eq!(buttons.items[0].right + 4, buttons.items[1].left);
        assert_eq!(buttons.items[1].right + 4, buttons.more.left);
        assert_eq!(
            buttons.more.top - row.top,
            row.bottom - buttons.more.bottom,
            "centred vertically"
        );
    }

    #[test]
    fn a_row_with_no_labelled_button_still_gets_the_more_button() {
        let layout = Layout::new(&spec(1000), &[Item], 0).with_row_buttons(0, &[]);
        let buttons = layout.buttons.unwrap();
        assert!(buttons.items.is_empty());
        assert_eq!(buttons.left, buttons.more.left);
    }

    #[test]
    fn buttons_that_would_crowd_the_row_are_left_to_the_menu() {
        let layout = Layout::new(&spec(1000), &[Item], 0);
        // Half of 640 is 320: the ⋯ (26), then 4 + 100 each.
        let buttons = layout
            .with_row_buttons(0, &[80, 80, 80, 80])
            .buttons
            .unwrap();
        assert_eq!(buttons.items.len(), 2, "the first ones, in order");
        assert!(640 - buttons.left <= 320 + 16);
    }

    #[test]
    fn the_buttons_are_hit_before_the_row_under_them() {
        let layout = Layout::new(&spec(1000), &[Item, Item], 0).with_row_buttons(0, &[40, 40]);
        let buttons = layout.buttons.clone().unwrap();
        let middle = |r: Rect| ((r.left + r.right) / 2, (r.top + r.bottom) / 2);

        for (n, button) in buttons.items.iter().enumerate() {
            let (x, y) = middle(*button);
            assert_eq!(layout.hit_test(x, y), Some(Hit::Button(n)));
        }
        let (x, y) = middle(buttons.more);
        assert_eq!(layout.hit_test(x, y), Some(Hit::More));
        // Between two buttons, and left of the strip: the row.
        let first = buttons.items[0];
        assert_eq!(layout.hit_test(first.right + 1, y), Some(Hit::Row(0)));
        assert_eq!(layout.hit_test(buttons.left - 1, y), Some(Hit::Row(0)));
        // Only the row that has them.
        let other = layout.rect_of(1).unwrap();
        assert_eq!(layout.hit_test(x, other.top + 20), Some(Hit::Row(1)));
    }

    #[test]
    fn the_buttons_scale_with_the_monitor() {
        let spec = Spec {
            scale: 2.0,
            width: 1280,
            ..spec(2000)
        };
        let buttons = Layout::new(&spec, &[Item], 0)
            .with_row_buttons(0, &[40])
            .buttons
            .unwrap();
        assert_eq!(buttons.more.width(), 52);
        assert_eq!(
            buttons.items[0].width(),
            40 + 40,
            "labels are measured, padding scales"
        );
    }

    #[test]
    fn the_chevron_ends_the_input_row_and_the_field_stops_before_it() {
        let plain = Layout::new(&spec(1000), &[], 0);
        let layout = Layout::new(&spec(1000), &[], 0).with_chevron();
        let chevron = layout.chevron.unwrap();

        assert_eq!(chevron.right, 640 - 16);
        assert_eq!(chevron.width(), 26);
        assert_eq!(
            chevron.top - layout.input.top,
            layout.input.bottom - chevron.bottom
        );
        assert!(layout.input_parts().field.right < plain.input_parts().field.right);
        assert_eq!(layout.input_parts().field.right, chevron.left - 12);

        let (x, y) = (
            (chevron.left + chevron.right) / 2,
            (chevron.top + chevron.bottom) / 2,
        );
        assert_eq!(layout.hit_test(x, y), Some(Hit::Chevron));
        assert_eq!(layout.hit_test(100, y), Some(Hit::Input));
        assert_eq!(
            plain.hit_test(x, y),
            Some(Hit::Input),
            "none unless asked for"
        );
    }

    #[test]
    fn a_folded_window_is_the_input_row_alone() {
        let folded = Spec {
            footer: false,
            ..spec(1000)
        };
        let layout = Layout::new(&folded, &[], 0).with_chevron();
        assert_eq!(layout.size, (640, 56));
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
