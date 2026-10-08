//! The Action overlay screen: four tabs (Layout/Palette/Groups/Tiles)
//! beside a live preview of the popup.
//!
//! The editor and preview are a [`Split`]: side by side when *both* get
//! their minimum width, stacked (preview below) otherwise. The editor's
//! minimum is the active tab's own — a Tiles list needs more room than
//! six number fields — so the decision follows what's actually on screen,
//! not a window-width threshold that ignored the nav pane (what the old
//! `NARROW_WIDTH = 720` did, leaving a ~250px editor column on a "wide"
//! window).
//!
//! Tabs rather than the mock's single stacked column: stacked,
//! Layout + Palette + Groups + Tiles ran far taller than the window and
//! pushed the tiles out of reach.

pub mod groups;
pub mod layout;
pub mod palette;
pub mod preview;
pub mod tiles;

use crate::icon_picker::IconPickerUi;
use crate::key_picker::KeyPickerUi;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg};
use palette::PaletteField;
use windows_reactor::*;
use y_snap::action::Action;
use y_snap::config::Overlay;

/// Every [`Action`] an overlay tile can be bound to — [`crate::hotkeys::
/// ACTIONS`] plus [`Action::DesktopList`], which only means something on a
/// tile (it expands into one tile per virtual desktop when the overlay
/// opens).
pub const OVERLAY_ACTIONS: [Action; 21] = [
    Action::Overlay,
    Action::Reload,
    Action::Quit,
    Action::Minimize,
    Action::Maximize,
    Action::Close,
    Action::AlwaysOnTop,
    Action::Borderless,
    Action::Transparency,
    Action::Shade,
    Action::HideTaskbar,
    Action::CenterCursor,
    Action::Iconize,
    Action::Traynize,
    Action::Uniconize,
    Action::DesktopNext,
    Action::DesktopPrev,
    Action::DesktopNew,
    Action::DesktopMove,
    Action::PinToggle,
    Action::DesktopList,
];

/// One of this screen's four tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlaySection {
    Layout,
    Palette,
    Groups,
    Tiles,
}

impl OverlaySection {
    pub const ALL: [Self; 4] = [Self::Layout, Self::Palette, Self::Groups, Self::Tiles];

    /// [`SelectorBarItem`]'s text, round-tripped by `on_selected_text_changed`.
    pub fn title(self) -> &'static str {
        match self {
            Self::Layout => "Layout",
            Self::Palette => "Palette",
            Self::Groups => "Groups",
            Self::Tiles => "Tiles",
        }
    }

    pub fn from_title(title: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.title() == title)
    }

    /// The narrowest the editor pane can be with this tab showing.
    fn min_width(self) -> f64 {
        match self {
            // Three fields abreast at their minimum, or the `ColorPicker`'s
            // own natural width.
            Self::Layout | Self::Palette => 360.0,
            Self::Groups => groups::min_width(),
            Self::Tiles => tiles::min_width(),
        }
    }
}

/// What this screen shows besides the settings themselves.
#[derive(Clone, Copy, Debug)]
pub struct OverlayState {
    pub section: OverlaySection,
    pub editing_tile: Option<usize>,
    pub editing_palette: Option<PaletteField>,
}

/// Smallest the preview gets beside the editor before it moves below it —
/// it scales its tiles down to fit, but not without limit.
const PREVIEW_MIN_WIDTH: f64 = 300.0;

pub fn overlay_view(
    overlay: &Overlay,
    state: OverlayState,
    picker: &KeyPickerUi,
    icons: &IconPickerUi,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    let split = Split::new(viewport).min_widths(state.section.min_width(), PREVIEW_MIN_WIDTH).weights(1.2, 1.0);
    let editor_viewport = split.primary();

    let tab = match state.section {
        OverlaySection::Layout => layout::layout_tab(overlay, editor_viewport, context),
        OverlaySection::Palette => palette::palette_tab(overlay, state.editing_palette, editor_viewport, context),
        OverlaySection::Groups => groups::groups_tab(overlay, editor_viewport, context),
        OverlaySection::Tiles => tiles::tiles_tab(overlay, state.editing_tile, picker, icons, editor_viewport, context),
    };

    let editor = Stack::vertical(space::L)
        .child("tabs", tabs(state.section, context))
        // Keyed by tab, so switching tabs replaces the content rather than
        // trying to morph one tab's controls into another's.
        .child(state.section.title(), tab);

    let preview = preview::overlay_preview(overlay, split.secondary());

    page(
        "Action overlay",
        "The tile grid shown on the overlay hotkey. Edit it on the left; the preview mirrors the popup's own \
         layout and colours as you go.",
        split.build(editor.into(), preview),
    )
}

fn tabs(current: OverlaySection, context: &ViewContext<ConfigApp>) -> View {
    let items = OverlaySection::ALL.map(|section| {
        let view: View = SelectorBarItem::new().text(section.title()).is_selected(section == current).into();
        (section.title(), view)
    });

    SelectorBar::new()
        .on_selected_text_changed(context.callback(move |text: Option<String>| {
            Msg::OverlaySectionSelected(text.as_deref().and_then(OverlaySection::from_title).unwrap_or(current))
        }))
        .slots([SlotView::collection(SelectorBarSlot::Items, items)])
}
