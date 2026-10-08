//! The window frame around every screen: navigation pane, scrolling page
//! area, and the Save/Reload bar — and the one place that knows how wide
//! each of those is, so it can tell the screens how much width is left
//! for them ([`ShellLayout::content`]).
//!
//! The pane adapts to the window: a full 220px pane with labels on a wide
//! window, WinUI's 48px icon rail (`LeftCompact`, items show their name as
//! a tooltip) below [`EXPANDED_PANE_MIN_WINDOW`] — on a 700px window, the
//! labelled pane alone was eating almost a third of the width.

use crate::ui::prelude::*;
use windows_reactor::*;

/// Window width from which the nav pane shows labels.
pub const EXPANDED_PANE_MIN_WINDOW: f64 = 1000.0;
const PANE_EXPANDED: f64 = 220.0;
/// WinUI's own `CompactPaneLength`.
const PANE_COMPACT: f64 = 48.0;
/// Room for the vertical scrollbar and `NavigationView`'s content border —
/// slack on the safe side, see [`Viewport`]'s note on approximate widths.
const CHROME_ALLOWANCE: f64 = 20.0;

/// Smallest window the content is designed for — below this the window
/// simply can't be resized. Everything must still work at this size.
pub const MIN_WINDOW: (f64, f64) = (560.0, 440.0);
pub const INITIAL_WINDOW: (f64, f64) = (1180.0, 780.0);

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShellLayout {
    pub pane_expanded: bool,
    /// Padding around the page content, each side.
    pub page_padding: f64,
    /// What a screen gets to lay itself out in.
    pub content: Viewport,
}

pub fn layout(window_width: f64) -> ShellLayout {
    let pane_expanded = window_width >= EXPANDED_PANE_MIN_WINDOW;
    let pane = if pane_expanded { PANE_EXPANDED } else { PANE_COMPACT };
    let rest = Viewport::new(window_width - pane - CHROME_ALLOWANCE);
    let page_padding = if rest.size_class() == SizeClass::Compact { space::L } else { space::XL };
    ShellLayout { pane_expanded, page_padding, content: rest.inset(page_padding) }
}

/// The pane's display-mode settings for `layout`.
pub fn pane_mode(nav: NavigationView, layout: ShellLayout) -> NavigationView {
    if layout.pane_expanded {
        nav.pane_display_mode(NavigationViewPaneDisplayMode::Left).open_pane_length(PANE_EXPANDED).is_pane_open(true)
    } else {
        nav.pane_display_mode(NavigationViewPaneDisplayMode::LeftCompact).is_pane_open(false)
    }
    // Pinned in both modes: four peer sections don't need the hamburger to
    // open an overlay pane, nor a back button with nothing to go back to.
    .is_pane_toggle_button_visible(false)
    .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
    .is_settings_visible(false)
}

/// A screen's view, padded and vertically scrolling. The horizontal axis
/// never scrolls — screens reflow to the width they're given instead.
pub fn page_frame(screen: View, layout: ShellLayout) -> View {
    scroll_y(Border::new().padding(Thickness::xy(layout.page_padding, space::XL)).content(screen))
}

/// The status / Reload / Save bar along the bottom of the window — one for
/// the whole app, so an edit on one screen can be saved from another.
pub fn action_bar(
    status: &str,
    status_detail: &str,
    healthy: bool,
    dirty: bool,
    on_reload: Callback<()>,
    on_save: Callback<()>,
) -> View {
    let status: View = Stack::horizontal(space::S)
        .child("dot", dot(if healthy { ThemeBrush::Accent } else { ThemeBrush::SystemCritical }))
        .child("text", text::single_line(status).vertical_alignment(VerticalAlignment::Center))
        .into();
    let status = status.tooltip(status_detail);

    // A Grid, not a horizontal StackPanel: the status takes what's left
    // after the buttons and trims, instead of pushing them off the window.
    let row: View = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto, GridLength::Auto])
        .column_spacing(space::S)
        .keyed_children([
            ("status", Border::new().vertical_alignment(VerticalAlignment::Center).grid_column(0).content(status)),
            ("reload", cell(button("Reload from disk", false, true, on_reload), 0, 1)),
            (
                "save",
                cell(
                    button("Save", true, dirty, on_save).tooltip(
                        "Rewrites y-snap.xml from these settings. Comments and hand formatting in the file aren't kept.",
                    ),
                    0,
                    2,
                ),
            ),
        ]);

    Border::new()
        .padding(Thickness::xy(space::L, space::S + 2.0))
        .border_thickness(Thickness::new(0.0, size::STROKE, 0.0, 0.0))
        .border_brush(ThemeBrush::CardStroke)
        .grid_row(1)
        .content(row)
}

/// The problems `Settings::load` skipped past, above whichever screen is
/// showing. Each was recorded instead of stopping the load — the rest of
/// the file loaded fine — but whatever it describes isn't in `Settings`,
/// so a save leaves it out of the file. Saying so before the first save
/// is the whole point.
pub fn warnings_banner(warnings: &[String], on_close: Callback<()>) -> View {
    const SHOWN: usize = 6;
    let mut lines: Vec<&str> = warnings.iter().take(SHOWN).map(String::as_str).collect();
    let more = warnings.len().saturating_sub(SHOWN);
    let more_line = format!("…and {more} more.");
    if more > 0 {
        lines.push(&more_line);
    }

    InfoBar::new()
        .severity(InfoBarSeverity::Warning)
        .is_open(true)
        .is_closable(true)
        .on_closed(on_close)
        .title(format!(
            "{} part{} of y-snap.xml couldn't be read",
            warnings.len(),
            if warnings.len() == 1 { "" } else { "s" }
        ))
        .message(format!(
            "They were skipped, and saving will leave them out of the file — fix them in the file first if you need \
             them, then Reload.\n{}",
            lines.join("\n")
        ))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pane_collapses_to_a_rail_on_narrow_windows() {
        assert!(layout(1200.0).pane_expanded);
        assert!(!layout(900.0).pane_expanded);
    }

    #[test]
    fn content_is_what_remains_after_pane_padding_and_chrome() {
        let wide = layout(1200.0);
        assert_eq!(wide.content.width, 1200.0 - PANE_EXPANDED - CHROME_ALLOWANCE - 2.0 * space::XL);
        let narrow = layout(MIN_WINDOW.0);
        assert_eq!(narrow.content.width, MIN_WINDOW.0 - PANE_COMPACT - CHROME_ALLOWANCE - 2.0 * space::L);
    }
}
