//! The Overlay screen's live preview, drawn from the same [`Overlay`] the
//! real popup uses (`y-snap/src/overlay.rs`).
//!
//! **It fits the width it's given.** Tile sizes are the config's own pixel
//! sizes (88×64 by default), so four columns plus a group of nine is
//! easily wider than a preview pane; the old preview drew them at full
//! size and overflowed off the window. Now the whole preview is scaled by
//! one factor — tiles, gaps, fonts and radii alike — so it keeps the real
//! popup's proportions at whatever size fits, and is drawn 1:1 when there
//! is room.
//!
//! Never shows hover/active states, or a `desktop-list` tile's real
//! per-desktop count: nothing here has a live pointer or real desktops to
//! drive them. A `desktop-list` item shows a few numbered placeholders.

use super::palette::to_ui_color;
use crate::keys;
use crate::ui::prelude::*;
use windows_reactor::*;
use y_snap::config::{Item, Overlay};

/// One tile: a real [`Item`], or a glyph standing in for one of the tiles
/// `desktop-list` expands into at runtime.
enum Tile<'a> {
    Item(&'a Item),
    Glyph(&'static str),
}

/// One grid of the popup — the main grid, or a group's row.
struct Section<'a> {
    title: Option<&'a str>,
    tiles: Vec<Tile<'a>>,
    columns: u32,
    tile: (u32, u32),
}

impl Section<'_> {
    fn columns(&self) -> usize {
        (self.columns.max(1) as usize).min(self.tiles.len().max(1))
    }

    /// Width at scale 1.
    fn natural_width(&self) -> f64 {
        let columns = self.columns() as f64;
        columns * self.tile.0 as f64 + (columns - 1.0) * TILE_GAP
    }
}

/// The popup's own tile gap and key-letter inset, in its pixels.
const TILE_GAP: f64 = 6.0;
const KEY_INSET: (f64, f64) = (4.0, 2.0);
const STAGE_PADDING: f64 = space::L;
/// Below this, tiles stop being legible — the [`super::PREVIEW_MIN_WIDTH`]
/// split threshold keeps the pane wide enough that this rarely binds.
const MIN_SCALE: f64 = 0.4;

pub fn overlay_preview(overlay: &Overlay, viewport: Viewport) -> View {
    let sections = sections(overlay);
    let natural = sections
        .iter()
        .map(Section::natural_width)
        .fold(0.0, f64::max);
    let available = viewport
        .inset(CARD_INSET)
        .inset(STAGE_PADDING + size::STROKE)
        .width;
    let scale = if natural > 0.0 {
        (available / natural).clamp(MIN_SCALE, 1.0)
    } else {
        1.0
    };

    let stage =
        Stack::vertical(space::S).children(sections.iter().enumerate().map(|(index, section)| {
            let grid = preview_grid(section, overlay, scale);
            let view: View = match section.title {
                Some(title) => Stack::vertical(space::XS)
                    .child(
                        "title",
                        text::caption(title.to_owned())
                            .foreground(to_ui_color(overlay.palette.text)),
                    )
                    .child("grid", grid)
                    .into(),
                None => grid,
            };
            (index, view)
        }));

    let stage: View = Border::new()
        .padding(Thickness::uniform(STAGE_PADDING))
        .corner_radius(CornerRadius::uniform(overlay.corner_radius as f64 * scale))
        .border_thickness(Thickness::uniform(size::STROKE))
        .border_brush(to_ui_color(overlay.palette.panel_border))
        .background(to_ui_color(with_alpha(overlay.palette.back, overlay.alpha)))
        .content(stage);

    let live: View = Stack::horizontal(space::XS)
        .child("dot", dot(ThemeBrush::Accent))
        .child(
            "text",
            text::caption(if scale < 1.0 {
                format!("Live · {:.0}%", scale * 100.0)
            } else {
                "Live".to_owned()
            }),
        )
        .into();

    panel("Preview", Some(live), stage)
}

fn with_alpha(color: y_core::overlay::Color, alpha: u8) -> y_core::overlay::Color {
    y_core::overlay::Color { a: alpha, ..color }
}

fn sections(overlay: &Overlay) -> Vec<Section<'_>> {
    let mut sections = vec![Section {
        title: None,
        tiles: overlay
            .items
            .iter()
            .filter(|item| item.group.is_none())
            .map(Tile::Item)
            .collect(),
        columns: overlay.columns,
        tile: overlay.tile,
    }];

    for group in &overlay.groups {
        let mut tiles = Vec::new();
        for item in overlay
            .items
            .iter()
            .filter(|item| item.group.as_deref() == Some(group.id.as_str()))
        {
            if item.action.is_desktop_list() {
                tiles.extend(["1", "2", "3", "+", "◀", "▶"].map(Tile::Glyph));
            } else {
                tiles.push(Tile::Item(item));
            }
        }
        if !tiles.is_empty() {
            sections.push(Section {
                title: Some(&group.id),
                tiles,
                columns: group.columns.unwrap_or(overlay.columns),
                tile: (
                    group.tile_width.unwrap_or(overlay.tile.0),
                    group.tile_height.unwrap_or(overlay.tile.1),
                ),
            });
        }
    }
    sections
}

/// A section's tiles at `scale`, in fixed-pixel columns — the same grid
/// the popup draws, just smaller. Left-aligned in the stage, like the
/// popup's own groups.
fn preview_grid(section: &Section<'_>, overlay: &Overlay, scale: f64) -> View {
    let columns = section.columns();
    let rows = section.tiles.len().div_ceil(columns).max(1);
    let (width, height) = (section.tile.0 as f64 * scale, section.tile.1 as f64 * scale);

    let cells = section.tiles.iter().enumerate().map(|(index, tile)| {
        (
            index,
            cell(
                preview_tile(tile, overlay, width, height, scale),
                index / columns,
                index % columns,
            ),
        )
    });

    Grid::new()
        .columns(vec![GridLength::Pixel(width); columns])
        .rows(vec![GridLength::Pixel(height); rows])
        .column_spacing(TILE_GAP * scale)
        .row_spacing(TILE_GAP * scale)
        .horizontal_alignment(HorizontalAlignment::Left)
        .keyed_children(cells.collect::<Vec<_>>())
}

/// One tile: the key in the top-left corner, icon and label centred — the
/// two layers overlap in one `Grid` cell, the way the popup draws key text
/// over the tile rather than budgeting space for it.
fn preview_tile(tile: &Tile<'_>, overlay: &Overlay, width: f64, height: f64, scale: f64) -> View {
    let (key, glyph, label) = match tile {
        Tile::Item(item) => (
            item.key.map(keys::label).unwrap_or_default(),
            item.icon.clone().unwrap_or_default(),
            item.label.clone().unwrap_or_default(),
        ),
        Tile::Glyph(glyph) => (String::new(), (*glyph).to_owned(), String::new()),
    };
    let palette = &overlay.palette;
    let font = |size: f64| (size * scale).max(6.0);

    let content: View = Grid::new().keyed_children([
        (
            "key",
            TextBlock::new()
                .text(key)
                .font_size(font((overlay.key_font_size as f64).min(12.0)))
                .font_weight(FontWeight::BOLD)
                .foreground(to_ui_color(palette.text))
                .opacity(emphasis::MUTED)
                .horizontal_alignment(HorizontalAlignment::Left)
                .vertical_alignment(VerticalAlignment::Top)
                .margin(Thickness::new(
                    KEY_INSET.0 * scale,
                    KEY_INSET.1 * scale,
                    0.0,
                    0.0,
                ))
                .into(),
        ),
        (
            "face",
            // A `Stack` is a `View` with no alignment of its own; a
            // centred `Border` around it is what centres it in the tile.
            Border::new()
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center)
                .content(
                    Stack::vertical(0.0)
                        .child(
                            "glyph",
                            TextBlock::new()
                                .text(glyph)
                                .font_size(font((overlay.icon_font_size as f64 * 0.65).max(10.0)))
                                .foreground(to_ui_color(palette.text))
                                .horizontal_alignment(HorizontalAlignment::Center),
                        )
                        .child(
                            "label",
                            TextBlock::new()
                                .text(label)
                                .font_size(font(overlay.label_font_size as f64))
                                .foreground(to_ui_color(palette.text))
                                .text_trimming(TextTrimming::CharacterEllipsis)
                                .horizontal_alignment(HorizontalAlignment::Center),
                        ),
                ),
        ),
    ]);

    // No stroke, and the radius capped at 6 — both as `ActionOverlay::paint`
    // draws a tile. (`border-width`/`palette.border` are the target-window
    // focus outline, not tile borders.)
    Border::new()
        .width(width)
        .height(height)
        .corner_radius(CornerRadius::uniform(
            overlay.corner_radius.min(6) as f64 * scale,
        ))
        .background(to_ui_color(palette.back))
        .content(content)
}
