//! Where the panel's rows are, and what a point on it means. Pure: sizes come
//! in as [`Metrics`], so the arithmetic is tested without a window.
//!
//! The list is `count` rows in one column, the master and the microphone first.
//! A separator sits between row 1 (the microphone) and row 2 (the first
//! application) whenever there is a row 2. At most `max_rows` rows are shown;
//! beyond that the list scrolls and `scroll` is the first row shown.

use windows::Win32::Foundation::RECT;

/// The sizes of one panel, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub width: i32,
    /// Around the list, and left and right of a row.
    pub pad: i32,
    pub row_height: i32,
    /// The extra height of the separator.
    pub separator: i32,
    /// The icon or glyph, a square.
    pub icon: i32,
    pub gap: i32,
    /// The room for a row's name.
    pub name_width: i32,
    /// The room for the percentage.
    pub percent_width: i32,
    /// The gutter of the scrollbar, at the right of the rows, taken only while
    /// the list scrolls.
    pub scrollbar: i32,
}

/// What a point on the panel is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Part {
    /// The icon: mutes.
    Icon,
    /// The bar, with the level the point stands for, 0.0 to 1.0.
    Bar(f32),
    /// Anything else of the row: selects it.
    Row,
}

/// A hit: which row (an index into the list, not into the visible part) and
/// on what.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub row: usize,
    pub part: Part,
}

/// What a point on the scrollbar is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollHit {
    /// The thumb: dragging it scrolls.
    Thumb,
    /// The track above the thumb: a page up.
    Above,
    /// The track below the thumb: a page down.
    Below,
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub metrics: Metrics,
    pub count: usize,
    pub max_rows: usize,
    pub scroll: usize,
}

impl Layout {
    pub fn new(metrics: Metrics, count: usize, max_rows: usize, scroll: usize) -> Self {
        let max_rows = max_rows.max(1);
        Self {
            metrics,
            count,
            max_rows,
            scroll: scroll.min(count.saturating_sub(max_rows)),
        }
    }

    /// Whether there are more rows than are shown, which is when the scrollbar
    /// exists and takes its gutter.
    pub fn scrollable(&self) -> bool {
        self.count > self.max_rows
    }

    /// The right edge of what a row draws: the gutter, when there is one,
    /// stays to its right.
    fn content_right(&self) -> i32 {
        let gutter = if self.scrollable() {
            self.metrics.scrollbar
        } else {
            0
        };
        self.metrics.width - self.metrics.pad - gutter
    }

    /// The first column of the gutter: whatever is to its right belongs to the
    /// scrollbar, not to a row.
    fn gutter_left(&self) -> i32 {
        self.content_right()
    }

    /// How far the list can scroll.
    fn max_scroll(&self) -> usize {
        self.count.saturating_sub(self.max_rows)
    }

    /// The track of the scrollbar, as a rectangle, while the list scrolls: as
    /// high as the rows, in the middle of the gutter.
    pub fn track_rect(&self) -> Option<RECT> {
        if !self.scrollable() {
            return None;
        }
        let inset = self.metrics.scrollbar / 4;
        Some(RECT {
            left: self.gutter_left() + inset,
            top: self.metrics.pad,
            right: self.metrics.width - self.metrics.pad - inset,
            bottom: self.size().1 - self.metrics.pad,
        })
    }

    /// The thumb's top and bottom: as long as the shown part is of the list
    /// (never shorter than a handle), as far down as the list is scrolled.
    pub fn thumb(&self) -> Option<(i32, i32)> {
        let track = self.track_rect()?;
        let height = track.bottom - track.top;
        let length = (height * self.max_rows as i32 / self.count as i32)
            .max(self.metrics.scrollbar * 2)
            .min(height);
        let travel = height - length;
        let top = track.top + travel * self.scroll as i32 / self.max_scroll().max(1) as i32;
        Some((top, top + length))
    }

    /// What is at (`x`, `y`) of the scrollbar, `None` away from it. The whole
    /// gutter counts, out to the window's edge: a thin target is hard to hit.
    pub fn scrollbar_hit(&self, x: i32, y: i32) -> Option<ScrollHit> {
        let track = self.track_rect()?;
        if x < self.gutter_left() || x >= self.metrics.width {
            return None;
        }
        if !(track.top..track.bottom).contains(&y) {
            return None;
        }
        let (top, bottom) = self.thumb()?;
        Some(if y < top {
            ScrollHit::Above
        } else if y >= bottom {
            ScrollHit::Below
        } else {
            ScrollHit::Thumb
        })
    }

    /// The scroll position of a thumb dragged to `y`, which it was taken
    /// `grab` pixels below its top.
    pub fn scroll_for_thumb(&self, y: i32, grab: i32) -> usize {
        let (Some(track), Some((top, bottom))) = (self.track_rect(), self.thumb()) else {
            return 0;
        };
        let travel = (track.bottom - track.top) - (bottom - top);
        if travel <= 0 {
            return 0;
        }
        let ratio = ((y - grab - track.top) as f32 / travel as f32).clamp(0.0, 1.0);
        (ratio * self.max_scroll() as f32).round() as usize
    }

    /// `scroll` kept within what the list can scroll.    pub fn paged_to(&self, scroll: usize) -> usize {        scroll.min(self.max_scroll())    }
    /// `scroll` kept within what the list can scroll.
    pub fn paged_to(&self, scroll: usize) -> usize {
        scroll.min(self.max_scroll())
    }

    /// The scroll position a page up or down gives.
    pub fn paged(&self, direction: isize) -> usize {
        self.scroll
            .saturating_add_signed(direction * self.max_rows as isize)
            .min(self.max_scroll())
    }

    /// How many rows are shown.
    pub fn visible(&self) -> usize {
        (self.count - self.scroll).min(self.max_rows)
    }

    /// Whether the separator is drawn at all: there is an application row and
    /// the shown part reaches it from above.
    fn has_separator(&self) -> bool {
        self.count > 2 && self.scroll <= 1 && self.scroll + self.visible() > 2
    }

    /// The window's size for the rows shown now. A list that scrolls keeps one
    /// height, the separator's room included whether it is in view or not: a
    /// window that changed height as the separator scrolled out would move the
    /// scrollbar's track under a thumb being dragged.
    pub fn size(&self) -> (i32, i32) {
        let m = &self.metrics;
        let separator = if self.has_separator() || self.scrollable() {
            m.separator
        } else {
            0
        };
        (
            m.width,
            m.pad * 2 + self.visible() as i32 * m.row_height + separator,
        )
    }

    /// The tallest the panel gets, for the surface made once: `max_rows` rows
    /// and the separator.
    pub fn max_size(metrics: Metrics, max_rows: usize) -> (i32, i32) {
        (
            metrics.width,
            metrics.pad * 2 + max_rows.max(1) as i32 * metrics.row_height + metrics.separator,
        )
    }

    /// The top of row `row`, if it is shown.
    fn top(&self, row: usize) -> Option<i32> {
        if row < self.scroll || row >= self.scroll + self.visible() {
            return None;
        }
        let separator = if row >= 2 && self.has_separator() {
            self.metrics.separator
        } else {
            0
        };
        Some(self.metrics.pad + (row - self.scroll) as i32 * self.metrics.row_height + separator)
    }

    /// Row `row` as a rectangle, across the whole width, if it is shown.
    pub fn row_rect(&self, row: usize) -> Option<RECT> {
        let top = self.top(row)?;
        Some(RECT {
            left: 0,
            top,
            right: self.metrics.width,
            bottom: top + self.metrics.row_height,
        })
    }

    /// The icon of a row.
    pub fn icon_rect(&self, row: usize) -> Option<RECT> {
        let rect = self.row_rect(row)?;
        let m = &self.metrics;
        let top = rect.top + (m.row_height - m.icon) / 2;
        Some(RECT {
            left: m.pad,
            top,
            right: m.pad + m.icon,
            bottom: top + m.icon,
        })
    }

    /// The room of a row's name, as a full-height rectangle.
    pub fn name_rect(&self, row: usize) -> Option<RECT> {
        let rect = self.row_rect(row)?;
        let m = &self.metrics;
        let left = m.pad + m.icon + m.gap;
        Some(RECT {
            left,
            right: left + m.name_width,
            ..rect
        })
    }

    /// The span of the bar, left and right edges, the same on every row.
    pub fn bar_span(&self) -> (i32, i32) {
        let m = &self.metrics;
        let left = m.pad + m.icon + m.gap + m.name_width + m.gap;
        let right = self.content_right() - m.percent_width - m.gap;
        (left, right.max(left + 1))
    }

    /// The room of the percentage, as a full-height rectangle.
    pub fn percent_rect(&self, row: usize) -> Option<RECT> {
        let rect = self.row_rect(row)?;
        let m = &self.metrics;
        Some(RECT {
            left: self.content_right() - m.percent_width,
            right: self.content_right(),
            ..rect
        })
    }

    /// What is at (`x`, `y`) of the window, `None` outside every row.
    pub fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        let row = (self.scroll..self.scroll + self.visible()).find(|&row| {
            self.row_rect(row)
                .is_some_and(|rect| (rect.top..rect.bottom).contains(&y))
        })?;
        // The gutter belongs to the scrollbar, not to the row it lies beside.
        let right = if self.scrollable() {
            self.gutter_left()
        } else {
            self.metrics.width
        };
        if !(0..right).contains(&x) {
            return None;
        }

        let icon = self.icon_rect(row)?;
        let (bar_left, bar_right) = self.bar_span();
        let part = if (icon.left..icon.right).contains(&x) {
            Part::Icon
        } else if (bar_left..=bar_right).contains(&x) {
            Part::Bar(level_at(x, bar_left, bar_right))
        } else {
            Part::Row
        };
        Some(Hit { row, part })
    }

    /// The level a pointer at `x` stands for while a bar is dragged: the pointer
    /// may have left the bar, the level stays within 0 to 1.
    pub fn drag_level(&self, x: i32) -> f32 {
        let (left, right) = self.bar_span();
        level_at(x, left, right)
    }

    /// The scroll position that keeps `selected` shown, moving no more than it
    /// must.
    pub fn scroll_to_show(&self, selected: usize) -> usize {
        let selected = selected.min(self.count.saturating_sub(1));
        let scroll = if selected < self.scroll {
            selected
        } else if selected >= self.scroll + self.max_rows {
            selected + 1 - self.max_rows
        } else {
            self.scroll
        };
        scroll.min(self.count.saturating_sub(self.max_rows))
    }
}

fn level_at(x: i32, left: i32, right: i32) -> f32 {
    ((x - left) as f32 / (right - left) as f32).clamp(0.0, 1.0)
}

/// `selected` after moving `delta` rows, stopping at the ends (no wrap: a
/// held arrow reaching the end should stay there).
pub fn move_selection(selected: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    selected.saturating_add_signed(delta).min(count - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Metrics {
        Metrics {
            width: 400,
            pad: 8,
            row_height: 40,
            separator: 10,
            icon: 24,
            gap: 8,
            name_width: 100,
            percent_width: 40,
            scrollbar: 12,
        }
    }

    fn layout(count: usize, max_rows: usize, scroll: usize) -> Layout {
        Layout::new(metrics(), count, max_rows, scroll)
    }

    #[test]
    fn the_size_follows_the_rows_shown_and_the_separator() {
        // Master and microphone only: no separator.
        assert_eq!(layout(2, 8, 0).size(), (400, 16 + 80));
        // An application row brings the separator.
        assert_eq!(layout(3, 8, 0).size(), (400, 16 + 120 + 10));
        assert_eq!(layout(8, 8, 0).size(), (400, 16 + 320 + 10));
        // The surface is made for the tallest.
        assert_eq!(Layout::max_size(metrics(), 8), (400, 16 + 320 + 10));
    }

    #[test]
    fn rows_stack_with_the_separator_after_the_microphone() {
        let l = layout(4, 8, 0);
        assert_eq!(l.row_rect(0).unwrap().top, 8);
        assert_eq!(l.row_rect(1).unwrap().top, 48);
        assert_eq!(l.row_rect(2).unwrap().top, 8 + 80 + 10);
        assert_eq!(l.row_rect(3).unwrap().top, 8 + 120 + 10);
        assert!(l.row_rect(4).is_none());
    }

    #[test]
    fn a_scrolled_list_shows_a_window_of_rows() {
        let l = layout(12, 5, 4);
        assert_eq!(l.visible(), 5);
        assert!(l.row_rect(3).is_none());
        assert_eq!(l.row_rect(4).unwrap().top, 8, "first shown row is on top");
        assert_eq!(l.row_rect(8).unwrap().top, 8 + 4 * 40);
        assert!(l.row_rect(9).is_none());
        // Past the separator none is drawn, but a scrolling list keeps its height.
        assert_eq!(l.size(), (400, 16 + 200 + 10));
    }

    #[test]
    fn scrolling_is_clamped_to_the_end() {
        assert_eq!(layout(10, 5, 99).scroll, 5);
        assert_eq!(layout(3, 5, 2).scroll, 0, "everything fits");
        assert_eq!(layout(10, 5, 5).visible(), 5);
    }

    #[test]
    fn a_hit_says_which_row_and_which_part() {
        let l = layout(4, 8, 0);
        let (bar_left, bar_right) = l.bar_span();

        let icon = l.hit(20, 20).unwrap();
        assert_eq!((icon.row, icon.part), (0, Part::Icon));

        let on_bar = l.hit((bar_left + bar_right) / 2, 60).unwrap();
        assert_eq!(on_bar.row, 1);
        match on_bar.part {
            Part::Bar(level) => assert!((level - 0.5).abs() < 0.01),
            other => panic!("{other:?}"),
        }

        // On the name, between the icon and the bar.
        let name = l.hit(60, 20).unwrap();
        assert_eq!(name.part, Part::Row);
        // On the percentage.
        assert_eq!(l.hit(390 - 5, 20).unwrap().part, Part::Row);
    }

    #[test]
    fn the_separator_and_the_outside_hit_nothing() {
        let l = layout(4, 8, 0);
        // Between row 1 (ends at 88) and row 2 (starts at 98).
        assert!(l.hit(100, 92).is_none());
        assert!(l.hit(100, 2).is_none(), "the top padding");
        assert!(l.hit(-1, 20).is_none());
        assert!(l.hit(400, 20).is_none());
        assert!(l.hit(100, 9999).is_none());
    }

    #[test]
    fn a_hit_on_a_scrolled_list_names_the_row_of_the_list() {
        let l = layout(12, 5, 4);
        assert_eq!(l.hit(100, 10).unwrap().row, 4);
        assert_eq!(l.hit(100, 8 + 4 * 40 + 5).unwrap().row, 8);
    }

    #[test]
    fn dragging_clamps_the_level_outside_the_bar() {
        let l = layout(3, 8, 0);
        let (left, right) = l.bar_span();
        assert_eq!(l.drag_level(left - 50), 0.0);
        assert_eq!(l.drag_level(right + 50), 1.0);
        assert!((l.drag_level((left + right) / 2) - 0.5).abs() < 0.01);
    }

    #[test]
    fn the_bar_span_is_never_empty() {
        let narrow = Metrics {
            width: 100,
            ..metrics()
        };
        let (left, right) = Layout::new(narrow, 3, 8, 0).bar_span();
        assert!(right > left);
    }

    #[test]
    fn the_scrollbar_exists_only_when_the_list_does_not_fit() {
        assert!(!layout(5, 5, 0).scrollable());
        assert!(layout(6, 5, 0).scrollable());
        assert!(layout(5, 5, 0).track_rect().is_none());
        assert!(layout(5, 5, 0).thumb().is_none());
    }

    #[test]
    fn the_gutter_is_taken_from_the_rows() {
        let plain = layout(5, 5, 0);
        let scrolling = layout(12, 5, 0);
        assert_eq!(plain.bar_span().1 - scrolling.bar_span().1, 12);
        assert_eq!(
            plain.percent_rect(0).unwrap().right - scrolling.percent_rect(0).unwrap().right,
            12
        );
        // The scrollbar stays clear of the rows' content.
        let track = scrolling.track_rect().unwrap();
        assert!(track.left >= scrolling.percent_rect(0).unwrap().right);
        assert!(track.right <= 400 - 8);
    }

    #[test]
    fn the_thumb_shows_how_much_is_shown_and_where() {
        let top = layout(10, 5, 0);
        let track = top.track_rect().unwrap();
        let height = track.bottom - track.top;
        let (t0, b0) = top.thumb().unwrap();
        assert_eq!(t0, track.top, "at the top when not scrolled");
        assert_eq!(b0 - t0, height / 2, "half the list is shown");

        let end = layout(10, 5, 5);
        let (t1, b1) = end.thumb().unwrap();
        assert_eq!(b1, track.bottom, "at the bottom when fully scrolled");
        assert_eq!(b1 - t1, b0 - t0);
    }

    #[test]
    fn a_thumb_is_never_smaller_than_a_handle() {
        let l = layout(200, 5, 0);
        let (top, bottom) = l.thumb().unwrap();
        assert_eq!(bottom - top, 24, "twice the gutter");
    }

    #[test]
    fn the_scrollbar_is_hit_in_the_gutter_only() {
        let l = layout(12, 5, 0);
        let (thumb_top, thumb_bottom) = l.thumb().unwrap();
        let gutter = 400 - 8 - 12;
        assert_eq!(
            l.scrollbar_hit(gutter + 5, thumb_top + 2),
            Some(ScrollHit::Thumb)
        );
        assert_eq!(
            l.scrollbar_hit(399, thumb_bottom + 5),
            Some(ScrollHit::Below),
            "out to the window's edge"
        );
        assert!(l.scrollbar_hit(gutter - 1, thumb_top + 2).is_none());
        assert!(l.scrollbar_hit(gutter + 5, 2).is_none(), "above the track");

        let scrolled = layout(12, 5, 7);
        let (top, _) = scrolled.thumb().unwrap();
        assert_eq!(
            scrolled.scrollbar_hit(gutter + 5, top - 3),
            Some(ScrollHit::Above)
        );
        assert!(layout(5, 5, 0).scrollbar_hit(395, 50).is_none());
    }

    #[test]
    fn a_row_is_not_hit_in_the_gutter() {
        let scrolling = layout(12, 5, 0);
        assert!(scrolling.hit(400 - 8 - 5, 20).is_none());
        assert!(scrolling.hit(400 - 8 - 12 - 5, 20).is_some());
        // Without the scrollbar the same column is the row's.
        assert!(layout(5, 5, 0).hit(400 - 8 - 5, 20).is_some());
    }

    #[test]
    fn dragging_the_thumb_maps_the_pointer_to_a_scroll_position() {
        let l = layout(12, 5, 0);
        let track = l.track_rect().unwrap();
        let (top, bottom) = l.thumb().unwrap();
        let travel = (track.bottom - track.top) - (bottom - top);

        assert_eq!(l.scroll_for_thumb(track.top - 50, 0), 0, "above: the top");
        assert_eq!(
            l.scroll_for_thumb(track.bottom + 50, 0),
            7,
            "below: the end"
        );
        // Halfway down the travel is halfway down the scroll.
        let halfway = l.scroll_for_thumb(track.top + travel / 2, 0);
        assert!((3..=4).contains(&halfway), "{halfway}");
        // The grab point is where the thumb was taken, not its top.
        assert_eq!(
            l.scroll_for_thumb(track.top + travel + 6, 6),
            7,
            "grabbed 6 px down, dragged to the end"
        );
    }

    #[test]
    fn a_page_moves_by_the_rows_shown_and_stops_at_the_ends() {
        let l = layout(12, 5, 0);
        assert_eq!(l.paged(1), 5);
        assert_eq!(l.paged(-1), 0);
        assert_eq!(layout(12, 5, 5).paged(1), 7);
        assert_eq!(layout(12, 5, 7).paged(1), 7);
        assert_eq!(layout(12, 5, 3).paged(-1), 0);
    }
    #[test]
    fn selection_moves_and_stops_at_the_ends() {
        assert_eq!(move_selection(0, 1, 5), 1);
        assert_eq!(move_selection(0, -1, 5), 0);
        assert_eq!(move_selection(4, 1, 5), 4);
        assert_eq!(move_selection(2, -1, 5), 1);
        assert_eq!(move_selection(0, 0, 0), 0);
    }

    #[test]
    fn the_selection_stays_in_view() {
        let l = layout(12, 5, 0);
        assert_eq!(l.scroll_to_show(3), 0, "already shown");
        assert_eq!(
            l.scroll_to_show(5),
            1,
            "just enough to show it at the bottom"
        );
        assert_eq!(l.scroll_to_show(11), 7);

        let scrolled = layout(12, 5, 7);
        assert_eq!(
            scrolled.scroll_to_show(6),
            6,
            "just enough to show it on top"
        );
        assert_eq!(scrolled.scroll_to_show(0), 0);
        assert_eq!(scrolled.scroll_to_show(9), 7);
    }
}
