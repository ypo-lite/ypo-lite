//! `WM_MEASUREITEM`/`WM_DRAWITEM` for an owner-drawn [`super::PopupMenu`] —
//! everything that turns one item's [`super::ItemData`] into pixels.
//! Construction/lifetime (`PopupMenu` itself, appending items, reclaiming
//! `dwItemData` on drop) lives one level up, in `menu`'s own module doc.
//!
//! [`handle_draw_item`] draws three nested rects, outside in:
//!
//! 1. `rcItem`, Windows' own hit-test rect for this row.
//! 2. The **cell** — `rcItem` inset by [`super::MenuDesign::padding`], the
//!    gap *between* this row and its neighbours (CSS would call this
//!    `margin`; see [`super::MenuDesign::padding`]'s own doc comment for the
//!    naming). The strip left outside the cell is never painted by this row
//!    at all — it shows through whatever `OnInitMenuPopup`'s
//!    `MIM_BACKGROUND` brush already painted for the whole popup, which is
//!    what makes adjacent rows read as separated rather than leaving a
//!    hole. A selected, non-disabled row's cell additionally routes through
//!    [`paint_selected_row`] for the border/round/gradient treatment —
//!    scoped to the selected row only, matching PortAL's own `OnDrawItem`
//!    (confirmed by reading it: border, rounding and gradient all live
//!    inside its `ODS_SELECTED` branch) — every other row's cell is a plain
//!    flat fill.
//! 3. The **content rect** — the cell inset again by `border.size +
//!    `[`super::MenuDesign::margin`], the gap *within* the row between its
//!    border (or its cell edge, if there is no border) and its own
//!    icon/text/arrow (CSS would call this `padding`). Reserved for every
//!    row alike, selected or not, so content never shifts position when a
//!    row's hover state toggles — [`draw_normal_item`]/[`draw_separator`]
//!    position everything relative to this rect, never the cell or
//!    `rcItem` directly.

use windows::Win32::Foundation::{LPARAM, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BF_BOTTOM, BF_LEFT, BITMAP, BLENDFUNCTION, BeginPath,
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateRectRgnIndirect, CreateSolidBrush,
    DFC_MENU, DFCS_MENUARROW, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteDC,
    DeleteObject, DrawEdge, DrawFrameControl, DrawTextW, EDGE_ETCHED, EndPath, ExcludeClipRect,
    FillRect, GRADIENT_FILL_RECT_H, GRADIENT_FILL_RECT_V, GRADIENT_RECT, GetClipBox, GetDC,
    GetObjectW, GetTextExtentPoint32W, GradientFill, HBITMAP, HDC, HGDIOBJ, RGN_COPY, ReleaseDC,
    RoundRect, SRCAND, SRCINVERT, SelectClipPath, SelectClipRgn, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT, TRIVERTEX,
};
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED, ODT_MENU,
};

use crate::overlay::Color;

use super::{
    EdgeRule, Fill, Gradient, GradientDirection, ItemData, ItemKind, MenuDesign, SeparatorSpan,
};

/// The icon size every themed row's icon draws at, and the width reserved
/// for it on *every* `Normal` row regardless of whether that particular
/// item has an icon — matching a native (non-owner-draw) menu, which
/// reserves the same fixed icon/check gutter for every item so a mix of
/// icon and no-icon rows still lines its text up in one column instead of
/// each row starting wherever its own icon (or lack of one) happened to
/// end. `y-pop`'s own `Icon::to_bitmap` call sites use this directly rather
/// than keeping a second copy of the same number, so the two can never
/// drift apart.
pub const ICON_SIZE: i32 = 16;
/// Padding either side of an item's text.
const TEXT_PAD: i32 = 8;
/// Gap between the icon column and the text that follows it. `16`, not a
/// tighter value, to match PortAL's own spacing: `menu.cpp::OnDrawItem`
/// starts text at `rect.left + iconSize.cx + 24` while its icon itself
/// starts at `rect.left + 8` — so PortAL's icon-to-text gap is `24 -
/// TEXT_PAD(8) = 16`, confirmed against a real screenshot comparison
/// (`y-pop/docs/portal-analysis.md`'s pixel-numbers section), not a value
/// chosen by eye.
const ICON_GAP: i32 = 16;
/// Reserved on a submenu item's right edge for its own themed arrow glyph —
/// see [`ItemKind::Normal`]'s doc comment for why one is drawn at all.
const ARROW_WIDTH: i32 = 16;
/// An untitled separator's row height — PortAL's own fixed `10` for
/// `MENU_SEP` (`menu.cpp::OnMeasureItem`), carried over as-is: thinner than
/// a real item on purpose, the same visual cue a native `MF_SEPARATOR` row
/// already gives for free.
const SEPARATOR_HEIGHT: i32 = 10;
/// Never shorter than this even for a single-line item with no icon — a
/// themed row that's *too* tight to comfortably click is a worse trade than
/// a few extra pixels of height.
const MIN_ITEM_HEIGHT: i32 = 22;
/// Gap reserved between the label column and a shortcut hint's own text —
/// PortAL's own fixed `15` (`menu.cpp::OnMeasureItem`'s `tSize.cx + 15`),
/// carried over as-is.
const SHORTCUT_GAP: i32 = 15;

/// Handles `WM_MEASUREITEM`. Call from the owner window's message handler
/// with the message's `lParam` (cast, as Windows sends it, to
/// `*mut MEASUREITEMSTRUCT` via `LPARAM`); returns `true` if this was an
/// owner-drawn *menu* item this module recognised (Windows also sends
/// `WM_MEASUREITEM` for owner-draw list boxes/combo boxes/buttons — the
/// `CtlType` check is what rules those out), `false` otherwise, in which
/// case the caller should fall through to `DefWindowProc`.
pub fn handle_measure_item(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let item = unsafe { &mut *(lparam.0 as *mut MEASUREITEMSTRUCT) };
    if item.CtlType != ODT_MENU || item.itemData == 0 {
        return false;
    }
    let data = unsafe { &*(item.itemData as *const ItemData) };

    let hdc = unsafe { GetDC(None) };
    let mut size = SIZE::default();
    // An empty string measures as zero width, which would starve an
    // untitled separator of the padding it still needs on either side.
    let text: Vec<u16> = match data.kind {
        _ if data.text.is_empty() => vec![b' ' as u16],
        // `DrawText` swallows the mnemonic markers of a label — measure it
        // the way it will be drawn.
        ItemKind::Normal => super::without_mnemonic_markers(&data.text),
        ItemKind::Separator => data.text.clone(),
    };
    let _ = unsafe { GetTextExtentPoint32W(hdc, &text, &mut size) };

    // Same measurement, for the shortcut hint if this item has one — see
    // `menu::ItemData::shortcut`'s doc comment. Matches PortAL's own
    // `OnMeasureItem` (`tSize.cx + 15`, gated on `MENU_SUB | MENU_SHOWSC`,
    // confirmed by reading it), just widened to "has a shortcut" rather
    // than that specific flag pair, since `y-pop` computes the same
    // presence/absence itself before this ever sees the item.
    let shortcut_width = match &data.shortcut {
        Some(shortcut) => {
            let mut sc_size = SIZE::default();
            let _ = unsafe { GetTextExtentPoint32W(hdc, shortcut, &mut sc_size) };
            sc_size.cx + SHORTCUT_GAP
        }
        None => 0,
    };

    unsafe { ReleaseDC(None, hdc) };

    // Reserved for every `Normal` row alike, icon or not — see `ICON_SIZE`'s
    // own doc comment for why a fixed reservation, not the real bitmap's
    // width, is what keeps every row's text aligned in one column.
    let icon_width = match data.kind {
        ItemKind::Normal => ICON_SIZE + ICON_GAP,
        ItemKind::Separator => 0,
    };
    let arrow_width = if data.submenu {
        ARROW_WIDTH + TEXT_PAD
    } else {
        0
    };

    // Two independent extras, `Normal`-row only — a separator keeps its
    // rule spanning the row's unpadded, unmargined width/height (see
    // `padding`'s and `margin`'s own doc comments: growing either here
    // would only shrink that rule for no benefit). `padding` (`2×`, one
    // side each) grows the row to make room for the gap between it and its
    // neighbours; `margin` (`2×` again) grows it further to make room for
    // the extra inset its own icon/text/arrow content gets pushed in by —
    // see `handle_draw_item`'s matching `content_rect` computation.
    let (padding, margin) = match data.kind {
        ItemKind::Normal => (data.design.padding * 2, data.design.margin * 2),
        ItemKind::Separator => (0, 0),
    };
    let extra = padding + margin;

    item.itemWidth =
        (size.cx + TEXT_PAD * 2 + icon_width + arrow_width + shortcut_width) as u32 + extra;
    item.itemHeight = match &data.kind {
        ItemKind::Separator if data.text.is_empty() => SEPARATOR_HEIGHT as u32,
        _ => (size.cy + 8).max(MIN_ITEM_HEIGHT) as u32 + extra,
    };
    true
}

/// Handles `WM_DRAWITEM`. Call from the owner window's message handler with
/// the message's `lParam` cast to `*const DRAWITEMSTRUCT`, same shape and
/// same `CtlType` filtering as [`handle_measure_item`]. See the module doc
/// for the three nested rects this draws.
pub fn handle_draw_item(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
    if item.CtlType != ODT_MENU || item.itemData == 0 {
        return false;
    }
    let data = unsafe { &*(item.itemData as *const ItemData) };

    let hdc = item.hDC;
    let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
    let disabled = item.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0) != 0;

    // A separator ignores both padding and margin — its etched rule is
    // meant to span the row's full, unpadded, unmargined width (see
    // `handle_measure_item`'s own match, and `draw_separator`'s doc
    // comment).
    let (padding, margin) = match data.kind {
        ItemKind::Normal => (data.design.padding as i32, data.design.margin as i32),
        ItemKind::Separator => (0, 0),
    };
    let cell = inset_rect(item.rcItem, padding);
    let content_rect = inset_rect(cell, data.design.border.size as i32 + margin);

    let (base, text_color) = match (disabled, selected) {
        (true, _) => (data.design.base.back, data.design.disabled_text),
        (false, true) => (data.design.selected.back, data.design.selected.text),
        (false, false) => (data.design.base.back, data.design.base.text),
    };

    // A disabled row never reads as "selected" for either of these — see
    // `EdgeRule`'s own doc comment (`edge`) and `MenuDesign::icon_backdrop`'s
    // (idle-only, gated on the same flag PortAL's own `!ODS_SELECTED` check
    // reads, but folded together with `disabled` here the same way the
    // `base`/`text_color` match above already does for the row's fill).
    let idle = !selected || disabled;

    // Where a normal row's icon column ends. Worked out from the design, not
    // this row's `content_rect`: a separator has no padding or margin, and
    // its line has to meet the column the normal rows around it draw.
    let design = &data.design;
    let column_end = icon_column_end(
        item.rcItem.left + (design.padding + design.border.size + design.margin) as i32,
    );
    // The icon column doesn't run through a separator whose line spans the
    // row: every titled one, and an untitled one if the design says so.
    let has_column = match data.kind {
        ItemKind::Normal => true,
        ItemKind::Separator => {
            data.text.is_empty() && data.design.separator == SeparatorSpan::AfterIcons
        }
    };

    if selected && !disabled {
        paint_selected_row(hdc, cell, &data.design);
    } else {
        fill_solid(hdc, cell, base);
        if idle
            && has_column
            && let Some(icon_backdrop) = data.design.icon_backdrop
        {
            paint_icon_backdrop(hdc, cell, column_end, icon_backdrop);
        }
    }

    match data.kind {
        ItemKind::Separator => {
            let column = has_column.then_some(column_end);
            draw_separator(hdc, content_rect, data, text_color, column, idle)
        }
        ItemKind::Normal => {
            draw_normal_item(hdc, content_rect, item.rcItem, data, text_color, idle)
        }
    }

    true
}

/// The selected row's own cell — border ring, optional rounding, then a
/// solid or gradient fill inside it. Exactly PortAL's own two-pass
/// `RoundRect`+`SelectClipPath` trick (`menu.cpp::OnDrawItem`, confirmed by
/// reading it a second time for this design): draw the *unrounded* border
/// colour first, then clip to a smaller, inset-by-`border.size` rounded
/// rect before painting the fill on top — the only way GDI offers to get a
/// ring around a variable-radius rounded rect without a dedicated stroke
/// API. `round == 0` skips the clip-path dance entirely and just becomes
/// two ordinary square `FillRect`s (or one, with `border.size == 0`) — the
/// same shape `handle_draw_item` used before this design existed.
fn paint_selected_row(hdc: HDC, cell: RECT, design: &MenuDesign) {
    let round = design.border.round as i32;
    // Only backed up (and only restored) when a round clip is actually set —
    // an unrounded row draws under whatever clip Windows already gave this
    // `hDC`, same as it always has.
    let clip_backup = (round > 0).then(|| {
        set_round_clip(hdc, cell, round);
        get_clip_rect(hdc)
    });

    let fill_rect = if design.border.size > 0 {
        fill_solid(hdc, cell, design.border.color);
        let inner = inset_rect(cell, design.border.size as i32);
        if round > 0 {
            set_round_clip(hdc, inner, round);
        }
        inner
    } else {
        cell
    };

    match design.fill {
        Fill::Solid => fill_solid(hdc, fill_rect, design.selected.back),
        Fill::Gradient(gradient) => fill_gradient(hdc, fill_rect, gradient),
    }

    // Icon/text/arrow drawing after this point must not stay clipped to the
    // rounded rect, or their own square-cornered content would itself get
    // clipped at the corners — restore the clip this row found on entry.
    if let Some(rect) = clip_backup {
        restore_clip(hdc, rect);
    }
}

/// `rect` shrunk by `amount` on all four sides, never crossing past its own
/// centre — a margin/border inset larger than the row itself collapses to a
/// zero-size rect at the centre rather than an inverted (negative-size) one,
/// which every GDI fill call below would otherwise silently no-op or
/// misbehave on.
fn inset_rect(rect: RECT, amount: i32) -> RECT {
    let mid_x = (rect.left + rect.right) / 2;
    let mid_y = (rect.top + rect.bottom) / 2;
    RECT {
        left: (rect.left + amount).min(mid_x),
        top: (rect.top + amount).min(mid_y),
        right: (rect.right - amount).max(mid_x),
        bottom: (rect.bottom - amount).max(mid_y),
    }
}

fn fill_solid(hdc: HDC, rect: RECT, color: Color) {
    unsafe {
        let brush = CreateSolidBrush(color.colorref());
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush.into());
    }
}

/// `GradientFill` across `rect`'s two opposite corners — PortAL's own
/// `_getGradiant` (`menu.cpp`), confirmed by reading it: top-left vertex at
/// `gradient.start`, bottom-right at `gradient.end`, direction picks
/// `GRADIENT_FILL_RECT_H`/`_V`. `Alpha` is left `0`: `GradientFill` ignores
/// it entirely (documented behaviour, also PortAL's own comment on the
/// equivalent field).
fn fill_gradient(hdc: HDC, rect: RECT, gradient: Gradient) {
    let channel = |c: u8| (c as u16) << 8;
    let vertex = [
        TRIVERTEX {
            x: rect.left,
            y: rect.top,
            Red: channel(gradient.start.r),
            Green: channel(gradient.start.g),
            Blue: channel(gradient.start.b),
            Alpha: 0,
        },
        TRIVERTEX {
            x: rect.right,
            y: rect.bottom,
            Red: channel(gradient.end.r),
            Green: channel(gradient.end.g),
            Blue: channel(gradient.end.b),
            Alpha: 0,
        },
    ];
    let mesh = GRADIENT_RECT {
        UpperLeft: 0,
        LowerRight: 1,
    };
    let mode = match gradient.direction {
        GradientDirection::Horizontal => GRADIENT_FILL_RECT_H,
        GradientDirection::Vertical => GRADIENT_FILL_RECT_V,
    };
    unsafe {
        let _ = GradientFill(hdc, &vertex, (&raw const mesh).cast(), 1, mode);
    }
}

/// Whether [`MenuDesign::edge`]'s rule draws on a row with the given
/// idle/selected state — pulled out of [`draw_normal_item`] as a pure
/// function (no `HDC`, no rect) so the row-state logic itself is directly
/// unit-testable without a live GDI draw.
fn edge_rule_draws(rule: EdgeRule, idle: bool) -> bool {
    match rule {
        EdgeRule::Idle => idle,
        EdgeRule::Hover => !idle,
        EdgeRule::Always => true,
    }
}

/// [`MenuDesign::icon_backdrop`]'s extra fill, idle rows only — PortAL's own
/// `icon_gradiant` (`menu.cpp::OnDrawItem`): a rect from the row's own left
/// edge (`cell`, the same rect the plain base fill it paints over just
/// used) out to just past the icon/text boundary — [`icon_column_end`],
/// `+ 2` more to bleed past it the same amount PortAL's own
/// `iconSize.cx + 18` does (`+16` for the boundary itself, `+2` further) —
/// confirmed by reading `menu.cpp`, not guessed. Drawn *before*
/// [`draw_normal_item`] so the icon/text/arrow it goes on to draw land on
/// top, same order PortAL's own call site uses.
fn paint_icon_backdrop(hdc: HDC, cell: RECT, column_end: i32, gradient: Gradient) {
    let rect = RECT {
        left: cell.left,
        top: cell.top,
        right: (column_end + 2).min(cell.right),
        bottom: cell.bottom,
    };
    fill_gradient(hdc, rect, gradient);
}

/// The icon/text boundary of a row whose content starts at `content_left`:
/// where [`MenuDesign::edge`]'s rule goes, the midpoint of the icon/text gap.
fn icon_column_end(content_left: i32) -> i32 {
    content_left + TEXT_PAD + ICON_SIZE + ICON_GAP / 2
}

/// [`MenuDesign::edge`]'s rule at `x`, over `rect`'s height.
fn draw_edge_rule(hdc: HDC, x: i32, rect: RECT) {
    let mut bar = RECT { left: x, ..rect };
    unsafe {
        let _ = DrawEdge(hdc, &mut bar, EDGE_ETCHED, BF_LEFT);
    }
}

/// The clip region's bounding rect, backed up before [`set_round_clip`]
/// narrows it — PortAL's own `fullRect` backup (`OnDrawItem`'s
/// `GetClipBox` call), reused here for [`restore_clip`] to reconstruct a
/// plain rectangular region from.
fn get_clip_rect(hdc: HDC) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        GetClipBox(hdc, &mut rect);
    }
    rect
}

/// Clips all subsequent painting on `hdc` to `rect`'s rounded-corner shape —
/// `round` is passed straight through to `RoundRect`'s own `width`/`height`
/// ellipse parameters, same as PortAL's own call (`border_round` for both,
/// not a separately computed radius).
fn set_round_clip(hdc: HDC, rect: RECT, round: i32) {
    unsafe {
        let _ = BeginPath(hdc);
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            round,
            round,
        );
        let _ = EndPath(hdc);
        let _ = SelectClipPath(hdc, RGN_COPY);
    }
}

/// Restores `hdc`'s clip region to the plain rectangle [`get_clip_rect`]
/// captured earlier — `SelectClipRgn` needs a region handle, not a `RECT`,
/// so this builds a throwaway one and frees it immediately after.
fn restore_clip(hdc: HDC, rect: RECT) {
    unsafe {
        let region = CreateRectRgnIndirect(&rect);
        let _ = SelectClipRgn(hdc, Some(region));
        let _ = DeleteObject(region.into());
    }
}

/// A leaf or submenu row. No arrow drawn here even for a submenu — see
/// [`ItemKind::Normal`]'s doc comment for why that would only double the
/// one Windows already draws in its own gutter. `rect` is the row's
/// *content* rect — [`handle_draw_item`]'s already inset it past padding,
/// the border, and margin, so everything here draws relative to it
/// directly, never `rcItem` or the cell — except the `ExcludeClipRect` call
/// below, which needs the real, uninset `item_rect` (`rcItem`): Windows
/// lays its own native arrow out relative to the whole row, not our
/// content rect, so an exclusion bounded by `rect` leaves a sliver of
/// padding/border/margin near the row's outer edge unprotected, letting
/// the native arrow's fixed system colour peek through there (seen as a
/// faint fixed-colour sliver on a light/flat theme, or a few stray pixels
/// on hover on a dark one). `idle` is this row's own idle/selected state —
/// see [`MenuDesign::edge`]'s own doc comment for why its rule (drawn here,
/// unlike [`MenuDesign::icon_backdrop`], since it needs the same icon/text
/// boundary this function already computes) is gated on it.
fn draw_normal_item(
    hdc: HDC,
    rect: RECT,
    item_rect: RECT,
    data: &ItemData,
    text_color: Color,
    idle: bool,
) {
    let mut text_rect = rect;
    text_rect.left += TEXT_PAD;
    text_rect.right -= TEXT_PAD;

    // Drawn flush-left in the icon column when this item has one — but the
    // column itself, and so `text_rect`'s advance past it, is reserved
    // whether or not it does (see `ICON_SIZE`'s doc comment): an item with
    // no icon must still start its text at the same x as one that has one.
    if let Some((w, h)) = data.bitmap.and_then(bitmap_size)
        && let Some(bitmap) = data.bitmap
    {
        let y = rect.top + ((rect.bottom - rect.top) - h) / 2;
        blit_icon(hdc, text_rect.left, y, w, h, bitmap);
    }
    text_rect.left += ICON_SIZE + ICON_GAP;

    // A vertical rule between the icon column and the text — see
    // `MenuDesign::edge`'s own doc comment for which rows this draws on,
    // and why the config-facing names (`Idle`/`Hover`/`Always`) aren't
    // PortAL's own `_SEL`/`_UNSEL`. Positioned at the midpoint of the
    // icon/text gap, same as PortAL's own `iconSize.cx + 16` (`menu.cpp`,
    // confirmed by reading it).
    if let Some(rule) = data.design.edge
        && edge_rule_draws(rule, idle)
    {
        draw_edge_rule(hdc, icon_column_end(rect.left), rect);
    }

    if data.submenu {
        let column_left = (rect.right - TEXT_PAD - ARROW_WIDTH).max(text_rect.left);
        text_rect.right = column_left;

        // A fixed-size square glyph, vertically centred in the row — same
        // placement convention as `blit_icon`'s icon, not stretched across
        // the row's full (text-baseline-driven, not square) height the way
        // the column reserved for it is.
        let arrow_top = rect.top + ((rect.bottom - rect.top) - ARROW_WIDTH) / 2;
        let arrow_rect = RECT {
            left: column_left,
            top: arrow_top,
            right: column_left + ARROW_WIDTH,
            bottom: arrow_top + ARROW_WIDTH,
        };
        draw_arrow(hdc, arrow_rect, text_color);

        // Windows still draws its own arrow for this `MF_POPUP` item after
        // `WM_DRAWITEM` returns, in a fixed system colour that goes
        // invisible against a dark theme — see `ItemKind::Normal`'s doc
        // comment. Excluding the whole reserved column (not just the
        // smaller square glyph drawn above) from the clip region is
        // PortAL's own fix for exactly this (`menu.cpp::OnDrawItem`,
        // "in order to not redraw the system arrow" is their own comment
        // on the equivalent call), not invented here. The exclusion goes
        // all the way out to `item_rect`'s own right/top/bottom, not
        // `rect`'s (content rect, inset by padding/border/margin) — see
        // this function's own doc comment for why stopping at `rect`
        // leaves a sliver of the native arrow uncovered.
        unsafe {
            let _ = ExcludeClipRect(
                hdc,
                column_left,
                item_rect.top,
                item_rect.right,
                item_rect.bottom,
            );
        }
    }

    // Right-aligned shortcut hint, e.g. "Win+Y" — PortAL's own `sctext`
    // (`menu.cpp::OnDrawItem`), drawn in its own colour after the label so a
    // long label's clipped tail never bleeds into it. `text_rect`'s right
    // edge already stops before the arrow column when this item is a
    // submenu (set above), so no extra allowance for that is needed here
    // the way PortAL's own `isArrow` branch needed one — a second `-= 10`
    // on top of its already-shared `tRectText`. PortAL draws this in a flat
    // `COLOR_GRAYTEXT`, unthemed even under a themed skin — a divergence
    // this deliberately doesn't carry over (unreadable dark-on-dark under
    // the `dark` skin); [`MenuDesign::disabled_text`] gives the same "muted
    // hint" read while actually tracking the active theme.
    if let Some(shortcut) = &data.shortcut {
        let mut shortcut_rect = text_rect;
        shortcut_rect.right -= TEXT_PAD;
        draw_text(
            hdc,
            shortcut.clone(),
            shortcut_rect,
            data.design.disabled_text,
            DT_RIGHT | DT_NOPREFIX,
        );
    }

    // Without `DT_NOPREFIX`: the label's `&` marks its mnemonic, which
    // `DrawText` underlines (and `&&` draws as a literal ampersand).
    draw_text(hdc, data.text.clone(), text_rect, text_color, DT_LEFT);
}

/// PortAL's own separator look (`menu.cpp::OnDrawItem`'s `MENU_SEP`/
/// `MENU_TITLE` branches, confirmed by reading them): an `EDGE_ETCHED`
/// groove — the same two-tone line a plain, non-owner-draw `MF_SEPARATOR`
/// gets from Windows for free. With a `column` (where the icon column ends) it
/// starts just past it, PortAL's `iconSize.cx + 17`, and that column's rule
/// carries on through it. Without, it spans the row, split either side of the
/// centred title if there is one.
fn draw_separator(
    hdc: HDC,
    rect: RECT,
    data: &ItemData,
    text_color: Color,
    column: Option<i32>,
    idle: bool,
) {
    if data.text.is_empty() {
        if let Some(column_end) = column
            && let Some(rule) = data.design.edge
            && edge_rule_draws(rule, idle)
        {
            draw_edge_rule(hdc, column_end, rect);
        }
        let mut bar = RECT {
            left: column.map_or(rect.left + TEXT_PAD, |column_end| column_end + 1),
            right: rect.right - TEXT_PAD,
            top: (rect.top + rect.bottom) / 2,
            bottom: (rect.top + rect.bottom) / 2,
        };
        unsafe {
            let _ = DrawEdge(hdc, &mut bar, EDGE_ETCHED, BF_BOTTOM);
        }
        return;
    }

    let mut text = data.text.clone();
    let mut size = SIZE::default();
    let _ = unsafe { GetTextExtentPoint32W(hdc, &text, &mut size) };

    let mid_x = (rect.left + rect.right) / 2;
    let mut text_rect = rect;
    text_rect.left = mid_x - size.cx / 2;
    text_rect.right = text_rect.left + size.cx;

    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, text_color.colorref());
        DrawTextW(
            hdc,
            &mut text,
            &mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }

    let mid_y = (rect.top + rect.bottom) / 2;
    let mut left_bar = RECT {
        left: rect.left + TEXT_PAD,
        right: (text_rect.left - ICON_GAP).max(rect.left + TEXT_PAD),
        top: mid_y,
        bottom: mid_y,
    };
    let mut right_bar = RECT {
        left: (text_rect.right + ICON_GAP).min(rect.right - TEXT_PAD),
        right: rect.right - TEXT_PAD,
        top: mid_y,
        bottom: mid_y,
    };
    unsafe {
        let _ = DrawEdge(hdc, &mut left_bar, EDGE_ETCHED, BF_BOTTOM);
        let _ = DrawEdge(hdc, &mut right_bar, EDGE_ETCHED, BF_BOTTOM);
    }
}

fn draw_text(
    hdc: HDC,
    mut text: Vec<u16>,
    mut rect: RECT,
    color: Color,
    align: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    // `DrawTextW` reads through an empty `Vec`'s dangling pointer.
    if text.is_empty() {
        return;
    }
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color.colorref());
        DrawTextW(
            hdc,
            &mut text,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

/// The `(width, height)` a bitmap was created at, via `GetObjectW` — every
/// icon this module draws came from [`crate::icon::Icon::to_bitmap`], which
/// always makes a square bitmap, but reading its real size back rather than
/// assuming one keeps this module decoupled from that size choice.
fn bitmap_size(bitmap: HBITMAP) -> Option<(i32, i32)> {
    let mut info: BITMAP = unsafe { std::mem::zeroed() };
    let written = unsafe {
        GetObjectW(
            HGDIOBJ::from(bitmap),
            size_of::<BITMAP>() as i32,
            Some((&raw mut info).cast()),
        )
    };
    (written != 0).then_some((info.bmWidth, info.bmHeight))
}

/// Blits a premultiplied-alpha icon bitmap (see [`crate::icon::Icon::
/// to_bitmap`]) onto `hdc` at `(x, y)` with per-pixel alpha, via a
/// throwaway memory DC — `AlphaBlend` needs the source selected into a DC
/// of its own, there is no "blend straight from a bitmap handle" call.
fn blit_icon(hdc: HDC, x: i32, y: i32, w: i32, h: i32, bitmap: HBITMAP) {
    unsafe {
        let mem_dc = CreateCompatibleDC(Some(hdc));
        if mem_dc.is_invalid() {
            return;
        }
        let previous = SelectObject(mem_dc, bitmap.into());
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = AlphaBlend(hdc, x, y, w, h, mem_dc, 0, 0, w, h, blend);
        SelectObject(mem_dc, previous);
        let _ = DeleteDC(mem_dc);
    }
}

/// Draws a right-pointing submenu arrow into `rect` (expected square, see
/// [`draw_normal_item`]'s call site), recoloured to `color` — the OS's own
/// arrow shape (`DFC_MENU`/`DFCS_MENUARROW`, the same primitive Windows
/// itself uses for a native, non-owner-draw `MF_POPUP` item's arrow) rather
/// than a font glyph, matching PortAL's own choice
/// (`menu.cpp::OnDrawItem`'s `DrawFrameControl(DFC_MENU, DFCS_MENUARROW)`
/// call, confirmed by reading it — not a font-rendered `▶`, which an
/// earlier version of this function used and which came out visibly
/// smaller/thinner than PortAL's arrow in a side-by-side screenshot
/// comparison, since a glyph's rendered size follows the menu font's
/// metrics rather than a fixed box).
///
/// `DrawFrameControl` itself only ever paints in a fixed system colour —
/// there is no colour parameter — so getting a themed arrow out of it needs
/// the same three-`BitBlt` colour-substitution trick PortAL's own
/// `OnDrawItem` uses: draw the glyph once (black arrow on white) into
/// `arrow_dc`, fill a same-size `fill_dc` solid `color`, then
/// XOR/AND/XOR the destination against `fill_dc`/`arrow_dc`/`fill_dc` in
/// that order. Where the glyph is white (background), the two XORs cancel
/// out and the destination pixel is left exactly as it was (so the row's
/// own fill still shows through around the arrow); where the glyph is
/// black (the arrow strokes), the AND zeroes the pixel and the final XOR
/// leaves it at exactly `color`.
fn draw_arrow(hdc: HDC, rect: RECT, color: Color) {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 {
        return;
    }

    unsafe {
        let arrow_dc = CreateCompatibleDC(Some(hdc));
        let fill_dc = CreateCompatibleDC(Some(hdc));
        if arrow_dc.is_invalid() || fill_dc.is_invalid() {
            let _ = DeleteDC(arrow_dc);
            let _ = DeleteDC(fill_dc);
            return;
        }

        let arrow_bitmap = CreateCompatibleBitmap(hdc, w, h);
        let fill_bitmap = CreateCompatibleBitmap(hdc, w, h);
        let old_arrow = SelectObject(arrow_dc, arrow_bitmap.into());
        let old_fill = SelectObject(fill_dc, fill_bitmap.into());

        let mut glyph_rect = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        let _ = DrawFrameControl(arrow_dc, &mut glyph_rect, DFC_MENU, DFCS_MENUARROW);

        let brush = CreateSolidBrush(color.colorref());
        FillRect(fill_dc, &glyph_rect, brush);
        let _ = DeleteObject(brush.into());

        let _ = BitBlt(
            hdc,
            rect.left,
            rect.top,
            w,
            h,
            Some(fill_dc),
            0,
            0,
            SRCINVERT,
        );
        let _ = BitBlt(hdc, rect.left, rect.top, w, h, Some(arrow_dc), 0, 0, SRCAND);
        let _ = BitBlt(
            hdc,
            rect.left,
            rect.top,
            w,
            h,
            Some(fill_dc),
            0,
            0,
            SRCINVERT,
        );

        SelectObject(arrow_dc, old_arrow);
        SelectObject(fill_dc, old_fill);
        let _ = DeleteObject(arrow_bitmap.into());
        let _ = DeleteObject(fill_bitmap.into());
        let _ = DeleteDC(arrow_dc);
        let _ = DeleteDC(fill_dc);
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemInfoW, MENUITEMINFOW, MIIM_DATA};

    use super::super::{Border, ColorPair, PopupMenu};
    use super::*;

    /// Same shape as `menu`'s own `test_design` — duplicated rather than
    /// shared, so the two test modules stay independent of each other.
    fn test_design() -> MenuDesign {
        MenuDesign {
            base: ColorPair {
                text: Color::rgb(255, 255, 255),
                back: Color::rgb(32, 32, 32),
            },
            selected: ColorPair {
                text: Color::rgb(255, 255, 255),
                back: Color::rgb(0, 120, 215),
            },
            disabled_text: Color::rgb(120, 120, 120),
            border: Border {
                size: 0,
                round: 0,
                color: Color::rgb(0, 120, 215),
            },
            fill: Fill::Solid,
            padding: 0,
            margin: 0,
            edge: None,
            separator: SeparatorSpan::AfterIcons,
            icon_backdrop: None,
        }
    }

    /// Same shape as `menu`'s own `item_data` — duplicated for the same
    /// reason `test_design` is.
    fn item_data(menu: windows::Win32::UI::WindowsAndMessaging::HMENU, position: u32) -> usize {
        let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<MENUITEMINFOW>() as u32;
        info.fMask = MIIM_DATA;
        unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }.expect("GetMenuItemInfoW");
        info.dwItemData
    }

    fn measured_width(menu: windows::Win32::UI::WindowsAndMessaging::HMENU, position: u32) -> u32 {
        let mut measure = MEASUREITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemWidth: 0,
            itemHeight: 0,
            itemData: item_data(menu, position),
        };
        let handled = handle_measure_item(LPARAM(&raw mut measure as isize));
        assert!(
            handled,
            "a themed item's own WM_MEASUREITEM must be recognised"
        );
        measure.itemWidth
    }

    #[test]
    fn a_titled_separator_measures_wider_than_an_untitled_one_on_a_themed_menu() {
        let untitled = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        untitled.add_separator().expect("add_separator");
        let untitled_width = measured_width(untitled.handle(), 0);

        let titled = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        titled
            .add_titled_separator("A rather long section title")
            .expect("add_titled_separator");
        let titled_width = measured_width(titled.handle(), 0);

        assert!(titled_width > untitled_width);
    }

    #[test]
    fn a_submenu_item_reserves_extra_width_for_its_own_arrow() {
        let mut menu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        menu.add_item(1, "Leaf").expect("add_item");
        let leaf_width = measured_width(menu.handle(), 0);

        let submenu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        menu.add_submenu("Leaf", submenu).expect("add_submenu");
        let submenu_width = measured_width(menu.handle(), 1);

        // Same text either way — the only possible difference is the
        // reserved arrow column (see `ItemKind::Normal`'s doc comment for
        // why one is reserved at all: Windows' own arrow is drawn in a
        // fixed system colour and must be clipped out, not relied on).
        assert!(submenu_width > leaf_width);
    }

    #[test]
    fn handle_measure_and_draw_item_ignore_messages_that_are_not_theirs() {
        // Zero itemData: a themed item always has a non-zero one (see
        // `menu`'s own owner-drawn test), so this is indistinguishable from
        // "not one of ours" and must be declined, not dereferenced.
        let mut measure = MEASUREITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemWidth: 0,
            itemHeight: 0,
            itemData: 0,
        };
        assert!(!handle_measure_item(LPARAM(&raw mut measure as isize)));
        assert!(!handle_measure_item(LPARAM(0)));

        let draw = DRAWITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemAction: unsafe { std::mem::zeroed() },
            itemState: unsafe { std::mem::zeroed() },
            hwndItem: HWND::default(),
            hDC: HDC::default(),
            rcItem: RECT::default(),
            itemData: 0,
        };
        assert!(!handle_draw_item(LPARAM(&raw const draw as isize)));
        assert!(!handle_draw_item(LPARAM(0)));
    }

    #[test]
    fn a_themed_leaf_and_submenu_item_both_draw_without_a_live_popup() {
        let mut menu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        menu.add_item(1, "Leaf").expect("add_item");
        let submenu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        menu.add_submenu("Group", submenu).expect("add_submenu");
        menu.add_separator().expect("add_separator");

        let hdc = unsafe { GetDC(None) };
        for position in 0..menu.item_count() {
            let mut draw = DRAWITEMSTRUCT {
                CtlType: ODT_MENU,
                CtlID: 0,
                itemID: 0,
                itemAction: unsafe { std::mem::zeroed() },
                itemState: unsafe { std::mem::zeroed() },
                hwndItem: HWND(menu.handle().0 as *mut _),
                hDC: hdc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 120,
                    bottom: 22,
                },
                itemData: item_data(menu.handle(), position),
            };
            assert!(handle_draw_item(LPARAM(&raw mut draw as isize)));
        }
        unsafe { ReleaseDC(None, hdc) };
    }

    /// Same coverage as the leaf/submenu draw test above, with a shortcut
    /// hint on both rows this time — `draw_normal_item`'s
    /// own shortcut branch (`data.shortcut`) must not panic just because
    /// the label and hint are drawn into related-but-different rects.
    #[test]
    fn a_themed_item_with_a_shortcut_draws_without_a_live_popup() {
        let mut menu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        menu.add_item_with_shortcut(1, "Notepad", Some("Win+N"))
            .expect("add_item_with_shortcut");
        let submenu = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        menu.add_submenu_with_shortcut("y-pop", submenu, Some("Win+Y"))
            .expect("add_submenu_with_shortcut");

        let hdc = unsafe { GetDC(None) };
        for position in 0..menu.item_count() {
            let mut draw = DRAWITEMSTRUCT {
                CtlType: ODT_MENU,
                CtlID: 0,
                itemID: 0,
                itemAction: unsafe { std::mem::zeroed() },
                itemState: unsafe { std::mem::zeroed() },
                hwndItem: HWND(menu.handle().0 as *mut _),
                hDC: hdc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 160,
                    bottom: 22,
                },
                itemData: item_data(menu.handle(), position),
            };
            assert!(handle_draw_item(LPARAM(&raw mut draw as isize)));
        }
        unsafe { ReleaseDC(None, hdc) };
    }

    /// [`edge_rule_draws`]'s row-state logic, direct — no `HDC` needed. Pins
    /// down the [`EdgeRule`] naming's whole point: `Idle` fires on an idle
    /// row, `Hover` on a selected one, `Always` on both — the opposite of
    /// what porting PortAL's own `_SEL`/`_UNSEL` bit names literally would
    /// have given (see `EdgeRule`'s own doc comment for that history).
    #[test]
    fn edge_rule_draws_on_the_row_states_its_name_says() {
        assert!(edge_rule_draws(EdgeRule::Idle, true));
        assert!(!edge_rule_draws(EdgeRule::Idle, false));

        assert!(!edge_rule_draws(EdgeRule::Hover, true));
        assert!(edge_rule_draws(EdgeRule::Hover, false));

        assert!(edge_rule_draws(EdgeRule::Always, true));
        assert!(edge_rule_draws(EdgeRule::Always, false));
    }

    /// Same "draws without a live popup" coverage as the tests above, with
    /// `edge`/`icon_backdrop` both set this time — `handle_draw_item`'s own
    /// icon-backdrop branch and `draw_normal_item`'s edge-rule branch must
    /// not panic on a real (if tiny) row.
    #[test]
    fn a_themed_item_with_an_edge_rule_and_icon_backdrop_draws_without_a_live_popup() {
        let design = MenuDesign {
            edge: Some(EdgeRule::Always),
            separator: SeparatorSpan::AfterIcons,
            icon_backdrop: Some(Gradient {
                direction: GradientDirection::Horizontal,
                start: Color::rgb(255, 217, 150),
                end: Color::rgb(0, 0, 0),
            }),
            ..test_design()
        };
        let mut menu = PopupMenu::themed(design).expect("CreatePopupMenu");
        menu.add_item(1, "Leaf").expect("add_item");
        let submenu = PopupMenu::themed(design).expect("CreatePopupMenu");
        menu.add_submenu("Group", submenu).expect("add_submenu");

        let hdc = unsafe { GetDC(None) };
        for position in 0..menu.item_count() {
            for selected in [false, true] {
                let mut draw = DRAWITEMSTRUCT {
                    CtlType: ODT_MENU,
                    CtlID: 0,
                    itemID: 0,
                    itemAction: unsafe { std::mem::zeroed() },
                    itemState: if selected {
                        ODS_SELECTED
                    } else {
                        unsafe { std::mem::zeroed() }
                    },
                    hwndItem: HWND(menu.handle().0 as *mut _),
                    hDC: hdc,
                    rcItem: RECT {
                        left: 0,
                        top: 0,
                        right: 120,
                        bottom: 22,
                    },
                    itemData: item_data(menu.handle(), position),
                };
                assert!(handle_draw_item(LPARAM(&raw mut draw as isize)));
            }
        }
        unsafe { ReleaseDC(None, hdc) };
    }

    #[test]
    fn padding_grows_the_measured_size_of_a_normal_row_but_not_a_separator() {
        let padded_design = MenuDesign {
            padding: 4,
            ..test_design()
        };
        let plain = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        plain.add_item(1, "Item").expect("add_item");
        let plain_width = measured_width(plain.handle(), 0);

        let padded = PopupMenu::themed(padded_design).expect("CreatePopupMenu");
        padded.add_item(1, "Item").expect("add_item");
        assert_eq!(
            measured_width(padded.handle(), 0),
            plain_width + 4 * 2,
            "2 * padding added to the row's own layout rect — the gap between rows"
        );

        // A separator ignores padding entirely — see `handle_measure_item`'s
        // own doc comment for why.
        let plain_sep = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        plain_sep.add_separator().expect("add_separator");
        let padded_sep = PopupMenu::themed(padded_design).expect("CreatePopupMenu");
        padded_sep.add_separator().expect("add_separator");
        assert_eq!(
            measured_width(plain_sep.handle(), 0),
            measured_width(padded_sep.handle(), 0),
        );
    }

    #[test]
    fn margin_also_grows_the_measured_size_of_a_normal_row() {
        // Same shape as the padding test above — `margin` (the gap *within*
        // a row, between its border and its content) is a wholly separate
        // knob from `padding` (the gap *between* rows) but grows the row's
        // own layout rect the same way, so `handle_draw_item`'s
        // `content_rect` always has room to actually shrink into.
        let margined_design = MenuDesign {
            margin: 5,
            ..test_design()
        };
        let plain = PopupMenu::themed(test_design()).expect("CreatePopupMenu");
        plain.add_item(1, "Item").expect("add_item");
        let plain_width = measured_width(plain.handle(), 0);

        let margined = PopupMenu::themed(margined_design).expect("CreatePopupMenu");
        margined.add_item(1, "Item").expect("add_item");
        assert_eq!(
            measured_width(margined.handle(), 0),
            plain_width + 5 * 2,
            "2 * margin added to the row's own layout rect"
        );
    }

    #[test]
    fn a_rounded_gradient_bordered_selected_row_draws_without_a_live_popup() {
        let design = MenuDesign {
            padding: 3,
            margin: 2,
            border: Border {
                size: 1,
                round: 8,
                color: Color::rgb(60, 60, 60),
            },
            fill: Fill::Gradient(Gradient {
                direction: GradientDirection::Vertical,
                start: Color::rgb(43, 43, 43),
                end: Color::rgb(26, 26, 26),
            }),
            ..test_design()
        };
        let menu = PopupMenu::themed(design).expect("CreatePopupMenu");
        menu.add_item(1, "Leaf").expect("add_item");

        let hdc = unsafe { GetDC(None) };
        let mut draw = DRAWITEMSTRUCT {
            CtlType: ODT_MENU,
            CtlID: 0,
            itemID: 0,
            itemAction: unsafe { std::mem::zeroed() },
            // ODS_SELECTED — the only state that routes through
            // `paint_selected_row`'s round/gradient/border pipeline.
            itemState: ODS_SELECTED,
            hwndItem: HWND(menu.handle().0 as *mut _),
            hDC: hdc,
            rcItem: RECT {
                left: 0,
                top: 0,
                right: 120,
                bottom: 22,
            },
            itemData: item_data(menu.handle(), 0),
        };
        assert!(handle_draw_item(LPARAM(&raw mut draw as isize)));
        unsafe { ReleaseDC(None, hdc) };
    }

    #[test]
    fn inset_rect_never_inverts_past_the_centre() {
        let rect = RECT {
            left: 0,
            top: 0,
            right: 10,
            bottom: 4,
        };
        let inset = inset_rect(rect, 100);
        assert!(inset.left <= inset.right);
        assert!(inset.top <= inset.bottom);
    }
}
