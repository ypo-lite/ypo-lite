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
    /// A row of a column of the tray menus: small icon, name.
    Compact,
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
// The chips of the line of prefixes.
const CHIP_HEIGHT: i32 = 22;
const CHIP_GAP: i32 = 6;
// The index of an A–Z list.
const BAND_HEIGHT: i32 = 32;
const LETTER_WIDTH: i32 = 24;
const LETTER_GAP: i32 = 2;
// The Miller columns. Fixed, so that the window does not jump while walking.
const COLUMNS_HEIGHT: i32 = 320;
const CRUMB_HEIGHT: i32 = 26;
const COMPACT_HEIGHT: i32 = 30;
const COMPACT_ICON: i32 = 20;
/// Three at most: the column before, the focused one, the preview.
const COLUMNS_SHOWN: usize = 3;
/// Inside a column, and around what the details pane holds.
const PANE_PADDING: i32 = 6;
const DETAILS_PADDING: i32 = 12;
/// The icon of the details pane, for the window to load it at that size.
pub const DETAILS_ICON: i32 = 48;

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
    /// Whether the band of an A–Z list's letters tops the list. It stays put
    /// while the rows scroll under it, and comes out of `max_list_height`.
    pub band: bool,
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
    /// A chip of the line of prefixes, by index.
    Prefix(usize),
    /// An action button of the selected row, by index into its buttons.
    Button(usize),
    /// The **⋯** that ends the selected row.
    More,
    /// An item of the action menu, by index.
    Menu(usize),
    /// The action menu, between its items.
    MenuPanel,
    /// A letter of an A–Z list's index, by index.
    Letter(usize),
    /// A row of a column other than the focused one: the column, the row.
    Pane(usize, usize),
    /// A button of the details pane, by index.
    Details(usize),
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
    /// The chips of the line of prefixes, if it is visible
    /// ([`Layout::with_prefix_chips`]).
    pub prefix_chips: Vec<Rect>,
    /// The band of an A–Z list's letters ([`Spec::band`]).
    pub band: Option<Rect>,
    /// The letters in it ([`Layout::with_letters`]).
    pub letters: Vec<Rect>,
    /// The Miller columns, when they are shown ([`Layout::columns`]).
    pub columns: Option<ColumnsLayout>,
    scale: f32,
}

/// One column as far as the layout cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaneSpec {
    pub rows: usize,
    pub selected: Option<usize>,
}

/// Where the Miller columns go. [`Layout::rows`] holds the focused column's rows
/// too, so what hangs from a row works as in a list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ColumnsLayout {
    pub crumb: Rect,
    /// The panes on screen, left to right.
    pub panes: Vec<PaneLayout>,
    /// Which pane the keys are in, by index among all of them.
    pub focus: usize,
    /// The details pane's buttons ([`Layout::with_details_buttons`]).
    pub buttons: Vec<Rect>,
}

/// One pane on screen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PaneLayout {
    /// Index among all the panes: it may have scrolled off to the left.
    pub pane: usize,
    pub rect: Rect,
    /// The first row shown, then where each row shown goes.
    pub first: usize,
    pub rows: Vec<Rect>,
    /// How many rows it has in all.
    pub total: usize,
}

/// Where the pieces of a [`RowKind::Compact`] row go; `trailing` is as in
/// [`ItemParts`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CompactParts {
    pub icon: Rect,
    pub title: Rect,
    pub trailing: Rect,
}

/// Where the pieces of the details pane go. The lines of text follow `title`,
/// one `line` apart, up to the buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DetailsParts {
    pub icon: Rect,
    pub title: Rect,
    pub first_line: Rect,
    pub line: i32,
    /// Where the text must end.
    pub bottom: i32,
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
        let mut band = None;
        let mut end = first;
        if !rows.is_empty() {
            let top = bottom;
            let padding = px(LIST_PADDING);
            let rows_top = if spec.band {
                let rect = Rect::new(0, top, width, top + px(BAND_HEIGHT));
                band = Some(rect);
                rect.bottom
            } else {
                top
            };
            let mut y = rows_top + padding;
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
            prefix_chips: Vec::new(),
            band,
            letters: Vec::new(),
            columns: None,
            scale,
        }
    }

    /// The Miller columns: a breadcrumb over the last three of `panes`, of which
    /// `focus` is where the keys are and `scroll` the first row it shows. Each
    /// other pane shows its selection. The height does not depend on the rows.
    pub fn columns(spec: &Spec, panes: &[PaneSpec], focus: usize, scroll: usize) -> Self {
        let scale = spec.scale;
        let px = |logical: i32| scaled(logical, scale);
        let mut layout = Self::new(
            &Spec {
                band: false,
                ..*spec
            },
            &[],
            0,
        );
        let width = spec.width;
        let top = layout.input.bottom;
        let list = Rect::new(0, top, width, top + columns_height(spec));
        let crumb = Rect::new(0, top, width, top + px(CRUMB_HEIGHT));
        let fit = column_fit(spec);

        let hidden = panes.len().saturating_sub(COLUMNS_SHOWN);
        let third = width / COLUMNS_SHOWN as i32;
        let mut placed = Vec::new();
        for (slot, (pane, spec)) in panes.iter().enumerate().skip(hidden).enumerate() {
            let slot = slot as i32;
            let right = if slot == COLUMNS_SHOWN as i32 - 1 {
                width
            } else {
                third * (slot + 1)
            };
            let rect = Rect::new(third * slot, crumb.bottom, right, list.bottom);
            let wanted = if pane == focus {
                scroll
            } else {
                spec.selected.map_or(0, |row| (row + 1).saturating_sub(fit))
            };
            let first = wanted.min(spec.rows.saturating_sub(fit));
            let (inset, height) = (px(PANE_PADDING), px(COMPACT_HEIGHT));
            let shown = fit.min(spec.rows - first) as i32;
            let rows = (0..shown)
                .map(|n| {
                    let row_top = rect.top + inset + n * height;
                    Rect::new(rect.left, row_top, rect.right, row_top + height)
                })
                .collect();
            placed.push(PaneLayout {
                pane,
                rect,
                first,
                rows,
                total: spec.rows,
            });
        }

        if let Some(pane) = placed.iter().find(|pane| pane.pane == focus) {
            layout.rows = pane
                .rows
                .iter()
                .enumerate()
                .map(|(n, &rect)| Placed {
                    index: pane.first + n,
                    kind: RowKind::Compact,
                    rect,
                })
                .collect();
            layout.first = pane.first;
            layout.end = pane.first + pane.rows.len();
            layout.more_above = pane.first > 0;
            layout.more_below = layout.end < pane.total;
        }
        layout.columns = Some(ColumnsLayout {
            crumb,
            panes: placed,
            focus,
            buttons: Vec::new(),
        });
        layout.list = Some(list);
        let mut bottom = list.bottom;
        layout.footer = spec.footer.then(|| {
            let rect = Rect::new(0, bottom, width, bottom + px(FOOTER_HEIGHT));
            bottom = rect.bottom;
            rect
        });
        layout.size = (width, bottom);
        layout
    }

    /// This layout with the buttons of the details pane, the last pane: one for
    /// each of `widths` — the width of each label, in physical pixels — along its
    /// bottom, as many as fit.
    pub fn with_details_buttons(mut self, widths: &[i32]) -> Self {
        let px = |logical: i32| scaled(logical, self.scale);
        let Some(columns) = self.columns.as_mut() else {
            return self;
        };
        let Some(pane) = columns.panes.last().map(|pane| pane.rect) else {
            return self;
        };
        let (height, gap, inner) = (px(BUTTON_HEIGHT), px(BUTTON_GAP), px(BUTTON_PADDING));
        let inset = px(DETAILS_PADDING);
        let top = pane.bottom - inset - height;
        let mut x = pane.left + inset;
        for &label in widths {
            let width = label + 2 * inner;
            if x + width > pane.right - inset {
                break;
            }
            columns
                .buttons
                .push(Rect::new(x, top, x + width, top + height));
            x += width + gap;
        }
        self
    }

    /// The parts of a row of a column.
    pub fn compact_parts(&self, row: Rect) -> CompactParts {
        let px = |logical: i32| scaled(logical, self.scale);
        let (inset, icon) = (px(PANE_PADDING) + px(8), px(COMPACT_ICON));
        let top = row.top + (row.height() - icon) / 2;
        let icon = Rect::new(row.left + inset, top, row.left + inset + icon, top + icon);
        let right = row.right - inset;
        CompactParts {
            icon,
            title: Rect::new(icon.right + px(8), row.top, right, row.bottom),
            trailing: Rect::new(right, row.top, right, row.bottom),
        }
    }

    /// The parts of the details pane `pane`.
    pub fn details_parts(&self, pane: Rect) -> DetailsParts {
        let px = |logical: i32| scaled(logical, self.scale);
        let inset = px(DETAILS_PADDING);
        let (left, right) = (pane.left + inset, pane.right - inset);
        let icon = Rect::new(
            left,
            pane.top + inset,
            left + px(DETAILS_ICON),
            pane.top + inset + px(DETAILS_ICON),
        );
        let title = Rect::new(left, icon.bottom + px(6), right, icon.bottom + px(30));
        let line = px(20);
        DetailsParts {
            icon,
            title,
            first_line: Rect::new(left, title.bottom, right, title.bottom + line),
            line,
            bottom: pane.bottom - inset - px(BUTTON_HEIGHT) - px(6),
        }
    }

    /// This layout with the `count` letters of an A–Z list's index across its
    /// band, narrower if they would not otherwise fit.
    pub fn with_letters(mut self, count: usize) -> Self {
        let Some(band) = self.band.filter(|_| count > 0) else {
            return self;
        };
        let px = |logical: i32| scaled(logical, self.scale);
        let (height, gap) = (px(CHIP_HEIGHT), px(LETTER_GAP));
        let left = band.left + px(PADDING);
        let room = band.right - px(PADDING) - left;
        let width = px(LETTER_WIDTH).min((room + gap) / count as i32 - gap);
        let top = band.top + (band.height() - height) / 2;
        self.letters = (0..count as i32)
            .map(|n| {
                let x = left + n * (width + gap);
                Rect::new(x, top, x + width, top + height)
            })
            .collect();
        self
    }

    /// This layout with the chips of the line of prefixes, if that row is on
    /// screen: one for each of `widths` — the width of each label, in physical
    /// pixels — from the left padding on, as many as fit.
    pub fn with_prefix_chips(mut self, widths: &[i32]) -> Self {
        let Some(row) = self
            .rows
            .iter()
            .find(|placed| placed.kind == RowKind::Prefixes)
            .map(|placed| placed.rect)
        else {
            return self;
        };
        let px = |logical: i32| scaled(logical, self.scale);
        let (height, gap, inner) = (px(CHIP_HEIGHT), px(CHIP_GAP), px(BUTTON_PADDING));
        let top = row.top + (row.height() - height) / 2;
        let limit = row.right - px(PADDING);
        let mut x = row.left + px(PADDING);
        for &label in widths {
            let width = label + 2 * inner;
            if x + width > limit {
                break;
            }
            self.prefix_chips
                .push(Rect::new(x, top, x + width, top + height));
            x += width + gap;
        }
        self
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
        if let Some(n) = self
            .prefix_chips
            .iter()
            .position(|chip| chip.contains(x, y))
        {
            return Some(Hit::Prefix(n));
        }
        if let Some(n) = self.letters.iter().position(|letter| letter.contains(x, y)) {
            return Some(Hit::Letter(n));
        }
        if let Some(columns) = &self.columns {
            if let Some(n) = columns.buttons.iter().position(|b| b.contains(x, y)) {
                return Some(Hit::Details(n));
            }
            for pane in columns.panes.iter().filter(|p| p.pane != columns.focus) {
                if let Some(n) = pane.rows.iter().position(|row| row.contains(x, y)) {
                    return Some(Hit::Pane(pane.pane, pane.first + n));
                }
            }
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
    /// window grows to hold it, and the footer, if the menu reaches it, goes
    /// down to the new bottom so that it stays the last thing in the window.
    pub fn with_menu(mut self, anchor: Rect, count: usize, limit: i32) -> Self {
        let px = |logical: i32| scaled(logical, self.scale);
        let footer_height = self.footer.map_or(0, |footer| footer.height());
        let limit = limit - footer_height;
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
        match self.footer {
            Some(footer) if rect.bottom > footer.top => {
                self.size.1 = self.size.1.max(rect.bottom + footer_height);
                self.footer = Some(Rect::new(
                    0,
                    self.size.1 - footer_height,
                    self.size.0,
                    self.size.1,
                ));
            }
            _ => self.size.1 = self.size.1.max(rect.bottom),
        }
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

/// The first row to show so that the list starts at row `row`, or as near as it
/// can with the window still full: a jump to the last letter should not leave a
/// few rows in a window that has shrunk around them.
pub fn scroll_to_top(spec: &Spec, rows: &[RowKind], row: usize) -> usize {
    let mut first = row.min(rows.len().saturating_sub(1));
    while first > 0 && Layout::new(spec, rows, first - 1).end == rows.len() {
        first -= 1;
    }
    first
}

/// The first row of the focused column so that row `selected` shows, changing
/// `first` as little as possible.
pub fn scroll_column(spec: &Spec, first: usize, selected: usize) -> usize {
    let fit = column_fit(spec);
    first.min(selected).max((selected + 1).saturating_sub(fit))
}

/// The height of the columns and their breadcrumb: fixed, less on a monitor too
/// small for it.
fn columns_height(spec: &Spec) -> i32 {
    let px = |logical: i32| scaled(logical, spec.scale);
    px(CRUMB_HEIGHT + COLUMNS_HEIGHT)
        .min(spec.max_list_height)
        .max(px(CRUMB_HEIGHT + COMPACT_HEIGHT + 2 * PANE_PADDING))
}

/// How many rows a column shows.
fn column_fit(spec: &Spec) -> usize {
    let px = |logical: i32| scaled(logical, spec.scale);
    let room = columns_height(spec) - px(CRUMB_HEIGHT) - 2 * px(PANE_PADDING);
    (room / px(COMPACT_HEIGHT)).max(1) as usize
}

fn row_height(kind: RowKind) -> i32 {
    match kind {
        RowKind::Header => HEADER_HEIGHT,
        RowKind::Item => ITEM_HEIGHT,
        RowKind::Prefixes => PREFIXES_HEIGHT,
        RowKind::Compact => COMPACT_HEIGHT,
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
            band: false,
        }
    }

    fn pane(rows: usize, selected: Option<usize>) -> PaneSpec {
        PaneSpec { rows, selected }
    }

    #[test]
    fn the_columns_keep_their_height_whatever_they_hold() {
        let few = Layout::columns(&spec(1000), &[pane(2, Some(0)), pane(0, None)], 0, 0);
        let many = Layout::columns(&spec(1000), &[pane(90, Some(0)), pane(90, None)], 0, 0);
        assert_eq!(few.size, many.size);
        assert_eq!(few.size, (640, 56 + 26 + 320 + 34));
        assert_eq!(few.list, Some(Rect::new(0, 56, 640, 56 + 346)));
        assert_eq!(few.footer, Some(Rect::new(0, 402, 640, 436)));
    }

    #[test]
    fn the_last_three_panes_show_in_three_thirds() {
        let panes = [
            pane(3, Some(0)),
            pane(3, Some(1)),
            pane(3, Some(2)),
            pane(0, None),
        ];
        let layout = Layout::columns(&spec(1000), &panes, 2, 0);
        let columns = layout.columns.as_ref().unwrap();
        let shown: Vec<(usize, i32, i32)> = columns
            .panes
            .iter()
            .map(|pane| (pane.pane, pane.rect.left, pane.rect.right))
            .collect();
        assert_eq!(shown, [(1, 0, 213), (2, 213, 426), (3, 426, 640)]);
    }

    #[test]
    fn the_rows_of_the_focused_column_are_the_layouts_rows_and_the_others_are_panes() {
        let panes = [pane(3, Some(1)), pane(4, Some(0)), pane(2, None)];
        let layout = Layout::columns(&spec(1000), &panes, 1, 0);
        assert_eq!(layout.rows.len(), 4);
        assert!(layout.rows.iter().all(|row| row.kind == RowKind::Compact));

        let columns = layout.columns.as_ref().unwrap();
        let row = layout.rows[2].rect;
        assert_eq!(
            layout.hit_test(row.left + 5, row.top + 5),
            Some(Hit::Row(2))
        );
        let left = columns.panes[0].rows[1];
        assert_eq!(
            layout.hit_test(left.left + 5, left.top + 5),
            Some(Hit::Pane(0, 1))
        );
        let preview = columns.panes[2].rows[1];
        assert_eq!(
            layout.hit_test(preview.left + 5, preview.top + 5),
            Some(Hit::Pane(2, 1))
        );
    }

    #[test]
    fn a_column_shows_its_selection_and_the_focused_one_its_scroll() {
        let fit = column_fit(&spec(1000));
        assert_eq!(fit, 10, "(320 - 12) / 30");
        let panes = [pane(50, Some(30)), pane(50, Some(12))];
        let layout = Layout::columns(&spec(1000), &panes, 1, 7);
        let columns = layout.columns.as_ref().unwrap();
        assert_eq!(columns.panes[0].first, 21, "its selection on the last row");
        assert_eq!(columns.panes[1].first, 7);
        assert_eq!(layout.first..layout.end, 7..17);

        assert_eq!(scroll_column(&spec(1000), 7, 3), 3, "up");
        assert_eq!(scroll_column(&spec(1000), 7, 8), 7, "still in view");
        assert_eq!(scroll_column(&spec(1000), 7, 20), 11, "down");

        let last = Layout::columns(&spec(1000), &panes, 1, 49);
        assert_eq!(last.first, 40, "no further than a full column");
    }

    #[test]
    fn the_columns_shrink_on_a_small_monitor_down_to_one_row() {
        let height = |max| {
            let layout = Layout::columns(&spec(max), &[pane(5, Some(0))], 0, 0);
            layout.list.unwrap().height()
        };
        assert_eq!(height(200), 200);
        assert_eq!(height(10), 26 + 30 + 12);
        assert_eq!(column_fit(&spec(10)), 1);
    }

    #[test]
    fn the_details_buttons_sit_along_the_bottom_of_the_last_pane_as_many_as_fit() {
        let panes = [pane(3, Some(0)), pane(0, None)];
        let layout =
            Layout::columns(&spec(1000), &panes, 0, 0).with_details_buttons(&[40, 40, 40, 40, 40]);
        let columns = layout.columns.as_ref().unwrap();
        let last = columns.panes.last().unwrap().rect;
        assert_eq!(columns.buttons.len(), 3, "three of 60 fit in 213 less 24");
        assert!(columns.buttons.iter().all(|b| b.right <= last.right - 12));
        assert_eq!(columns.buttons[0].bottom, last.bottom - 12);

        let button = columns.buttons[1];
        assert_eq!(
            layout.hit_test(button.left + 2, button.top + 2),
            Some(Hit::Details(1))
        );
        let parts = layout.details_parts(last);
        assert!(parts.bottom <= columns.buttons[0].top);
    }

    #[test]
    fn the_letter_band_stays_on_top_and_takes_its_height_from_the_list() {
        let rows = [Header, Item, Item, Item];
        let plain = spec(190);
        let banded = Spec {
            band: true,
            ..plain
        };
        let layout = Layout::new(&banded, &rows, 2).with_letters(3);
        let band = layout.band.unwrap();
        assert_eq!(band, Rect::new(0, 56, 640, 88));
        assert_eq!(layout.rows[0].rect.top, 88 + 6, "the rows start under it");
        assert!(layout.size.1 - 34 <= 56 + 190, "and the list is no taller");

        let letter = layout.letters[2];
        assert_eq!(
            layout.hit_test(letter.left + 1, letter.top + 1),
            Some(Hit::Letter(2))
        );
        assert_eq!(Layout::new(&plain, &rows, 0).band, None);
        assert!(
            Layout::new(&plain, &rows, 0)
                .with_letters(3)
                .letters
                .is_empty()
        );
    }

    #[test]
    fn twenty_seven_letters_fit_the_narrowest_window() {
        let spec = Spec {
            width: 560,
            band: true,
            ..spec(400)
        };
        let layout = Layout::new(&spec, &[Header, Item], 0).with_letters(27);
        assert_eq!(layout.letters.len(), 27);
        assert!(layout.letters[26].right <= 560 - 16);
    }

    #[test]
    fn a_jump_puts_the_row_on_top_unless_the_window_would_no_longer_be_full() {
        let rows = vec![Item; 40];
        let spec = spec(232); // 4 rows.
        assert_eq!(scroll_to_top(&spec, &rows, 10), 10);
        assert_eq!(scroll_to_top(&spec, &rows, 38), 36);
        assert_eq!(scroll_to_top(&spec, &rows, 36), 36);
        let few = vec![Item; 2];
        assert_eq!(scroll_to_top(&spec, &few, 1), 0, "everything fits");
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
    fn a_menu_that_reaches_the_footer_pushes_it_to_the_new_bottom() {
        let rows = [Header, Item, Item];
        let spec = spec(1000);
        let layout = Layout::new(&spec, &rows, 0);
        let row = layout.rect_of(2).unwrap();
        let footer_before = layout.footer.unwrap();

        let layout = layout.with_menu(row, 4, max_height(&spec));
        let menu = layout.menu.clone().unwrap();
        let footer = layout.footer.unwrap();

        assert!(menu.rect.bottom > footer_before.top, "it does reach it");
        assert_eq!(footer.bottom, layout.size.1, "the footer is the last thing");
        assert_eq!(footer.height(), footer_before.height());
        assert!(
            menu.rect.bottom <= footer.top,
            "and the menu does not cover it"
        );
        assert!(layout.size.1 <= max_height(&spec));
    }

    #[test]
    fn a_menu_that_fits_above_the_footer_leaves_it_alone() {
        let rows = [Header, Item, Item, Item, Item, Item];
        let spec = spec(1000);
        let layout = Layout::new(&spec, &rows, 0);
        let row = layout.rect_of(1).unwrap();
        let (footer, size) = (layout.footer, layout.size);
        let layout = layout.with_menu(row, 2, max_height(&spec));
        assert_eq!((layout.footer, layout.size), (footer, size));
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
    fn the_line_of_prefixes_has_a_clickable_chip_for_each_prefix() {
        let rows = [Header, Item, Prefixes];
        let layout = Layout::new(&spec(1000), &rows, 0).with_prefix_chips(&[60, 50, 40]);
        let row = layout.rect_of(2).unwrap();

        assert_eq!(layout.prefix_chips.len(), 3);
        assert_eq!(
            layout.prefix_chips[0].left,
            row.left + 16,
            "from the padding"
        );
        assert_eq!(layout.prefix_chips[0].width(), 60 + 20);
        assert_eq!(
            layout.prefix_chips[0].right + 6,
            layout.prefix_chips[1].left
        );
        for (n, chip) in layout.prefix_chips.iter().enumerate() {
            let (x, y) = ((chip.left + chip.right) / 2, (chip.top + chip.bottom) / 2);
            assert_eq!(layout.hit_test(x, y), Some(Hit::Prefix(n)));
        }
        let gap = layout.prefix_chips[0].right + 2;
        assert_eq!(layout.hit_test(gap, row.top + 17), Some(Hit::Row(2)));
    }

    #[test]
    fn without_that_line_on_screen_there_are_no_chips() {
        let layout = Layout::new(&spec(1000), &[Header, Item], 0).with_prefix_chips(&[60]);
        assert!(layout.prefix_chips.is_empty());
        // Chips that would run off the row are left out.
        let layout = Layout::new(&spec(1000), &[Prefixes], 0).with_prefix_chips(&[300, 300, 300]);
        assert_eq!(layout.prefix_chips.len(), 1);
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
