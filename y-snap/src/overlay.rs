//! The action overlay: one tile per configured `<overlay>` action, chosen
//! with a single key press or a click, for whichever window it was opened
//! against.
//!
//! [`min_overlay`](crate::min_overlay) is the template this follows —
//! lazy [`Overlay`] creation, a [`WindowHandler`] that turns a click into a
//! posted message, a `paint` that redraws from configuration on every
//! show — but this one is bigger in several ways that module does not need:
//! it lays out a grid instead of one row, it shows a toggle's on/off state,
//! a hover state, and a border around the target window, and — since it is
//! driven by the keyboard as much as the mouse — its own [`Overlay`] is
//! [`OverlaySpec::focusable`], unlike `min_overlay`'s.
//!
//! That is the same choice HulK's `ShowActionOverlay` makes
//! (`SetForegroundWindow`/`SetFocus` on its popup, right after showing it):
//! a real, focusable window gets `WM_KEYDOWN` delivered to it directly, so
//! there is no need for a scoped `WH_KEYBOARD_LL` hook the way
//! [`crate::mouse::modifier_guard`]'s does — [`ClickHandler`] just matches
//! `VK_ESCAPE` and each tile's bound key inline, the same handler that
//! already turns a click into a posted message. It also gets "click outside
//! closes it" for free: losing focus (`WM_KILLFOCUS`) *is* the signal that
//! the user has moved on, with no separate `WH_MOUSE_LL` hook required to
//! notice a click elsewhere.
//!
//! The hover state does not need a hook either — unlike `min_overlay`'s own
//! button-hover detection ([`crate::mouse::hover`]), which watches windows it
//! does not own from inside a low-level mouse hook, this popup is a normal,
//! interactive window: `WM_MOUSEMOVE`/`WM_MOUSELEAVE` arrive at
//! [`ClickHandler`] the ordinary way, and it posts [`WM_OVERLAY_HOVER`]
//! whenever the hit tile changes, the same shape it already used for a
//! click.
//!
//! # `desktop-list`
//!
//! The one config `<action>` that is not itself a tile: [`layout`] expands
//! it into one [`Action::DesktopMove`] tile per live virtual desktop,
//! numbered from 1, keyed `1`-`9` in order (mouse-click only past the
//! ninth — nothing here promises a grid that wide anyway) and placed in
//! whichever `<group>` the config's own `desktop-list` line named. Desktop
//! count can change while the overlay is closed (a desktop created or
//! removed), so [`ActionOverlay::show`] re-runs [`layout`] against the
//! live count every time it opens, not just once at construction — a
//! changed tile count recreates the popup window outright, the same way
//! [`ActionOverlay::show_border`] already recreates on a size change.
//!
//! A tile whose action is not yet dispatched for real (see
//! [`actions::is_implemented`]) still shows and is still clickable, so the
//! "not implemented yet" balloon [`crate::YSnap::perform`] already shows
//! still fires — it is just drawn disabled, rather than pretending it works.
//!
//! One generated tile is different: whichever one is [`is_current_desktop`]
//! — the desktop the overlay's owner window is already on — reads as
//! genuinely disabled, labelled "Current", and [`ActionOverlay::action_at`]
//! refuses to activate it at all, rather than merely showing it that way.
//! Moving a window to the desktop it is already on is not a real action to
//! offer, unlike the "not written yet" ones above, which are real actions
//! that just are not implemented *yet*.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_1, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, PostMessageW, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_SYSKEYDOWN,
};
use y_core::{
    Result,
    hotkey::key_name,
    ids::WM_APP_FIRST,
    monitor::{self, Monitor},
    overlay::{Canvas, Color, ColorFont, Font, Overlay, OverlaySpec, TextAlign},
    virtual_desktop,
    window::WindowHandler,
};

/// Not in the `windows` crate's `WindowsAndMessaging` module at this
/// version, unlike its sibling `WM_NCMOUSELEAVE`.
const WM_MOUSELEAVE: u32 = 0x02A3;

use crate::action::Action;
use crate::actions;
use crate::config::{Item, Overlay as OverlayConfig, Palette};
use crate::mouse;

/// Posted to the owner window when a tile is activated, by click or by key.
/// `wParam` is its index into [`ActionOverlay`]'s own tile list.
pub const WM_OVERLAY_ACTION: u32 = WM_APP_FIRST + 4;

/// Posted to the owner window on `Escape` — close without acting.
pub const WM_OVERLAY_CLOSE: u32 = WM_APP_FIRST + 5;

/// Posted to the owner window whenever the pointer moves onto, off, or
/// between tiles. `wParam` is the tile index, or `usize::MAX` for "no tile" —
/// there is no realistic grid large enough for that to collide with a real
/// index.
pub const WM_OVERLAY_HOVER: u32 = WM_APP_FIRST + 6;

const CLASS: &str = "YpoLite.YSnap.Overlay";
const BORDER_CLASS: &str = "YpoLite.YSnap.OverlayBorder";

/// The popup's own outline — a subtle hairline (see `Palette::panel_border`'s
/// own doc comment), always this thin regardless of `config.border_width`,
/// which is the *target window*'s highlight border, a separate concept.
const PANEL_BORDER_WIDTH: i32 = 1;

/// Margin between the tile grid and the popup's own edge — where
/// `panel_border` actually gets to be visible. Without it the outermost
/// tiles are flush against `bounds`, so the border stroke lands right on
/// top of their own opaque pixels and reads as no border at all, correctly
/// rounded or not. Same value as HulK's `OVL_PADDING`.
const PANEL_PADDING: i32 = 8;

/// One laid-out tile: which `<action>` it draws from, and where.
struct Tile {
    /// Index into [`ActionOverlay::items`] — the resolved item list, not
    /// [`ActionOverlay::config`]'s own `items` (`desktop-list` is expanded
    /// away by the time either exists; see [`layout`]).
    item: usize,
    /// Relative to the popup's own top-left corner.
    rect: RECT,
}

/// Owns the tile grid, the focus border, and the scoped keyboard hook.
/// `config`/`tiles`/`items`/`size` — the layout itself — live for as long as
/// this struct does, rebuilt on every configuration reload since a tile's
/// key, label and position all come from it. The popup window and its pixel
/// buffer are lighter-lived than that: created lazily on
/// [`show`](ActionOverlay::show), destroyed again by
/// [`close`](ActionOverlay::close) rather than merely hidden, so the memory
/// behind an open overlay is not still committed while it sits closed,
/// which is most of the time.
pub struct ActionOverlay {
    owner: HWND,
    config: OverlayConfig,
    tiles: Vec<Tile>,
    /// `config.items` with `desktop-list` expanded into its per-desktop
    /// tiles — see [`layout`]. What [`Tile::item`] actually indexes.
    items: Vec<Item>,
    /// How many virtual desktops [`items`](Self::items) was last expanded
    /// for — [`show`](Self::show) re-lays-out only when this has changed,
    /// rather than on every call.
    desktop_count: u32,
    /// 0-based index of the desktop the overlay's owner is on, refreshed on
    /// every [`show`](Self::show) — unlike `desktop_count` this never
    /// forces a re-layout (the grid does not change shape when the *current*
    /// desktop changes, only which one tile reads as unavailable), so it is
    /// worth re-reading every time rather than only when stale.
    current_desktop: Option<u32>,
    size: (i32, i32),
    overlay: Option<Overlay>,
    border: Option<Overlay>,
    target: Option<HWND>,
    /// Which tile the pointer is currently over, if any — kept here rather
    /// than in [`ClickHandler`] because painting needs it, and painting is
    /// this struct's job, not the popup window's own handler's.
    hovered: Option<usize>,
}

impl ActionOverlay {
    /// Lays out `config`'s tiles up front, against however many virtual
    /// desktops exist right now — [`show`](Self::show) re-lays-out later if
    /// that count has since changed.
    pub fn new(owner: HWND, config: OverlayConfig) -> Self {
        let desktop_count = virtual_desktop::desktop_count().unwrap_or(0);
        let (tiles, items, size) = layout(&config, desktop_count);
        Self {
            owner,
            config,
            tiles,
            items,
            desktop_count,
            current_desktop: virtual_desktop::current_desktop_index(),
            size,
            overlay: None,
            border: None,
            target: None,
            hovered: None,
        }
    }

    /// Opens the overlay against `window` — there is only ever one target at
    /// a time, so a call while it is already showing just redirects it,
    /// in place, rather than hiding and re-showing. That matters now that
    /// the popup is [`OverlaySpec::focusable`]: hiding a focused window
    /// fires `WM_KILLFOCUS` synchronously (see [`ClickHandler`]'s handling
    /// of it), and that would post a [`WM_OVERLAY_CLOSE`] which — arriving
    /// only after this call returns — would close the *new* target's
    /// overlay right back down again.
    ///
    /// `grant_foreground` is for a caller that has no input to its name — a
    /// mouse shortcut, whose click the hook swallowed. `WM_HOTKEY` lets a
    /// process take the foreground; a swallowed click does not, and Probe B
    /// (`docs/mouse-shortcuts-plan.md`) saw Windows refuse. A synthetic key
    /// tap just before `activate` is what got it granted.
    ///
    /// Whatever the caller, an overlay that still did not get the foreground
    /// is closed again: without focus its keys do nothing and
    /// `WM_KILLFOCUS` never comes, so it could not be dismissed.
    pub fn show(&mut self, window: HWND, grant_foreground: bool) -> Result<()> {
        // Re-lay-out only when the live desktop count actually moved since
        // the last time — a desktop created or removed while the overlay
        // was closed. Dropping `self.overlay` forces the "build the popup"
        // branch below to run again, with the new tile count/keys baked
        // into a fresh `ClickHandler` — cheaper than trying to mutate one
        // that already exists behind `Overlay`'s type-erased handler.
        let desktop_count = virtual_desktop::desktop_count().unwrap_or(0);
        if desktop_count != self.desktop_count {
            let (tiles, items, size) = layout(&self.config, desktop_count);
            self.tiles = tiles;
            self.items = items;
            self.size = size;
            self.desktop_count = desktop_count;
            self.overlay = None;
        }
        // Cheap, and worth refreshing every time — see the field's own doc
        // comment for why this does not gate a re-layout the way
        // `desktop_count` above does.
        self.current_desktop = virtual_desktop::current_desktop_index();

        if self.tiles.is_empty() {
            return Ok(());
        }

        if self.overlay.is_none() {
            let keys = self
                .tiles
                .iter()
                .enumerate()
                .filter_map(|(index, tile)| Some((self.items[tile.item].key?, index)))
                .collect();
            let handler = ClickHandler {
                owner: self.owner,
                tiles: self.tiles.iter().map(|tile| tile.rect).collect(),
                keys,
                hovered: None,
            };
            let spec = OverlaySpec::new(CLASS, self.size)
                .interactive()
                .focusable()
                .with_owner(self.owner);
            self.overlay = Some(Overlay::with_handler(&spec, Box::new(handler))?);
        }
        self.target = Some(window);

        let position =
            Monitor::from_window(window).keep_inside(centre_on(window, self.size), self.size);

        // Seed the hover state from wherever the pointer already is: Windows
        // only sends WM_MOUSEMOVE on real movement, not because a window just
        // appeared underneath it, so without this a tile the pointer happens
        // to already be sitting over would not look hovered until it moves.
        self.hovered = monitor::cursor_position().ok().and_then(|pt| {
            let pt = POINT {
                x: pt.x - position.x,
                y: pt.y - position.y,
            };
            self.tiles.iter().position(|tile| point_in(tile.rect, pt))
        });

        self.paint()?;
        let overlay = self.overlay.as_ref().expect("created above");
        overlay.show_at(position)?;
        // WM_KEYDOWN and click-outside (WM_KILLFOCUS) both need this window
        // to actually hold the keyboard focus — see this module's doc
        // comment on why, and [`Overlay::activate`] for why show_at alone
        // does not do it.
        if grant_foreground {
            y_core::hotkey::absorb_modifier_tap();
        }
        overlay.activate();

        // The safety net: a popup nobody can dismiss is worse than a
        // shortcut that did nothing.
        let focused = unsafe { GetForegroundWindow() } == overlay.hwnd();
        if !focused {
            self.close();
            return Ok(());
        }

        if self.config.focus_border {
            self.show_border(window)?;
        }

        Ok(())
    }

    /// Closes the overlay without acting — `Escape`, a click outside, a
    /// stale click, or about to show a new target. Safe to call when already
    /// closed.
    ///
    /// Actually destroys the popup (and the focus border), rather than just
    /// hiding it: a `y_core::overlay::Canvas`'s pixel buffer is one DIB
    /// section sized to the whole grid, and that stays committed for as long
    /// as the `Overlay` object holding it does — hiding a window does not
    /// release a single byte of it. The overlay sits closed far longer than
    /// it sits open, so freeing that memory here and paying a cheap
    /// window-plus-DIB-section recreation cost on the next
    /// [`show`](Self::show) (the same path `show` already takes when the
    /// tile count itself changes) is the right side to be cheap on, in
    /// keeping with this project's own low-memory goal.
    ///
    /// Direct2D and DirectWrite go for the same reason, here rather than in
    /// the caller so that [`show`](Self::show)'s own close counts too: every
    /// icon is kept as pixels by now, so a next show needs neither.
    pub fn close(&mut self) {
        self.target = None;
        self.hovered = None;
        self.overlay = None;
        self.border = None;
        y_core::overlay::release_color_text();
    }

    /// The action, target and (for `desktop-move`) desktop number for tile
    /// `index`, if the overlay is currently showing one and the tile is not
    /// [`Self::is_current_desktop`] — moving a window to the desktop it is
    /// already on is not a real activation, so this reads the same as a
    /// stale one: `None`, silently.
    pub fn action_at(&self, index: usize) -> Option<(Action, HWND, u32)> {
        let tile = self.tiles.get(index)?;
        let item = &self.items[tile.item];
        if is_current_desktop(self.current_desktop, item) {
            return None;
        }
        Some((item.action, self.target?, item.desktop))
    }

    /// The pointer moved onto, off, or between tiles — `None` for "off every
    /// tile". Called from [`crate::YSnap`] on [`WM_OVERLAY_HOVER`], which
    /// [`ClickHandler`] posts from inside the popup window's own message
    /// handling. A no-op once already showing this state, so a stray repeat
    /// (there is one at most, from [`show`](Self::show)'s own seeding versus
    /// the popup's independent tracking) does not repaint for nothing.
    pub fn set_hover(&mut self, index: Option<usize>) -> Result<()> {
        if self.hovered == index || self.target.is_none() {
            return Ok(());
        }
        self.hovered = index;
        self.paint()?;
        if let Some(overlay) = &self.overlay {
            overlay.refresh()?;
        }
        Ok(())
    }

    /// Owned (`OverlaySpec::with_owner`) by the same `self.owner` the tile
    /// grid's own popup is — two independent, owner-less `WS_EX_TOPMOST`
    /// popups have no Z-order relationship for Windows to maintain between
    /// them, so in practice only whichever one's most recent `SetWindowPos`
    /// "won" stayed visible, the other invisible underneath it. Sharing an
    /// owner is what HulK does too (`overlay.cpp`'s `ShowFocusBorder` and
    /// `ShowActionOverlay` both parent their popup to `g_hwndMain`), and
    /// it's what actually fixes this rather than being cosmetic.
    fn show_border(&mut self, window: HWND) -> Result<()> {
        let rect = mouse::visual_window_rect(window).unwrap_or_default();
        let size = (
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
        );

        if self
            .border
            .as_ref()
            .is_none_or(|border| border.size() != size)
        {
            let spec = OverlaySpec::new(BORDER_CLASS, size).with_owner(self.owner);
            self.border = Some(Overlay::new(&spec)?);
        }
        let border = self.border.as_mut().expect("created above");

        let canvas = border.canvas_mut();
        canvas.clear(Color::TRANSPARENT);
        let width = self.config.border_width.max(1) as i32;
        for inset in 0..width {
            canvas.stroke_rect(
                RECT {
                    left: inset,
                    top: inset,
                    right: size.0 - inset,
                    bottom: size.1 - inset,
                },
                self.config.palette.border,
            );
        }

        border.show_at(POINT {
            x: rect.left,
            y: rect.top,
        })
    }

    fn paint(&mut self) -> Result<()> {
        let Some(target) = self.target else {
            return Ok(());
        };
        let current_desktop = self.current_desktop;
        let Some(overlay) = &mut self.overlay else {
            return Ok(());
        };
        let palette = &self.config.palette;
        let bounds = overlay.bounds();

        let canvas = overlay.canvas_mut();
        canvas.clear(palette.back.with_alpha(self.config.alpha));

        let fonts = TileFonts::new(&self.config)?;
        // Capped well below the panel's own corner-radius: at a tile's size
        // (tens of pixels, not the whole popup's hundred-plus), the same
        // radius as the panel reads as bubbly rather than crisp. `corner-
        // radius` still scales both together below that cap, for a file
        // that sets it small.
        let tile_radius = (self.config.corner_radius as i32).min(6);
        let panel_back = palette.back.with_alpha(self.config.alpha);

        for (index, tile) in self.tiles.iter().enumerate() {
            let item = &self.items[tile.item];
            let current = is_current_desktop(current_desktop, item);
            let implemented = actions::is_implemented(item.action) && !current;
            let active = implemented && actions::toggle_state(item.action, target).unwrap_or(false);
            let hovered = implemented && self.hovered == Some(index);
            let (back, text) = tile_colors(palette, implemented, hovered, active);

            draw_tile(
                canvas,
                tile,
                item,
                &fonts,
                TileState {
                    current,
                    active,
                    back,
                    text,
                },
                tile_radius,
                panel_back,
            )?;
        }

        draw_panel_border(canvas, bounds, &self.config);
        Ok(())
    }
}

/// The four fonts a tile can draw with, built once per [`ActionOverlay::
/// paint`] call rather than once per tile — every tile in a popup shares
/// the same config, so there is nothing tile-specific to rebuild.
struct TileFonts {
    key: Font,
    label: Font,
    icon: ColorFont,
    /// A small corner badge on icon tiles, so the key that fires them is
    /// still visible even though the icon has taken over the spot the key
    /// letter would otherwise be drawn in. Sized off the same
    /// `key_font_size` as a standalone key rather than a fixed constant, so
    /// raising that setting also grows the badge; still visibly smaller
    /// than a standalone key, since it is the secondary hint on an
    /// icon-led tile, not the tile's main identity.
    badge: Font,
}

impl TileFonts {
    fn new(config: &OverlayConfig) -> Result<Self> {
        Ok(Self {
            key: Font::new("Segoe UI", config.key_font_size as i32, true)?,
            label: Font::new("Segoe UI", config.label_font_size as i32, false)?,
            icon: ColorFont::new(&config.icon_font, config.icon_font_size as i32, false)?,
            badge: Font::new(
                "Segoe UI",
                (config.key_font_size * 3 / 4).max(11) as i32,
                true,
            )?,
        })
    }
}

/// A tile's background/text colour pair for its current implemented/hovered/
/// active combination — pure lookup, no drawing, so [`paint`](ActionOverlay::
/// paint)'s own loop stays about *which* tile, not which colour that
/// implies.
fn tile_colors(
    palette: &Palette,
    implemented: bool,
    hovered: bool,
    active: bool,
) -> (Color, Color) {
    match (implemented, hovered, active) {
        (false, _, _) => (palette.disabled_back, palette.disabled_text),
        (_, true, _) => (palette.hover_back, palette.hover_text),
        (_, _, true) => (palette.active_back, palette.active_text),
        (_, false, false) => (palette.back, palette.text),
    }
}

/// The parts of a tile's state [`draw_tile`] needs but can't derive from
/// `Tile`/`Item` alone — computed once per tile in [`ActionOverlay::paint`]'s
/// own loop, since it already has `target`/`current_desktop`/`hovered` in
/// scope there.
struct TileState {
    /// Whether this tile represents the desktop the target window is
    /// already on — shows "Current" instead of the item's own label.
    current: bool,
    active: bool,
    back: Color,
    text: Color,
}

/// Draws one tile: its fill, icon-or-key glyph (plus the key's corner badge
/// when both an icon and a key are set), label, and rounded corners — see
/// [`ActionOverlay::paint`]'s own doc comment on `round_rect_corners`'s
/// ordering for why the rounding happens last.
fn draw_tile(
    canvas: &mut Canvas,
    tile: &Tile,
    item: &Item,
    fonts: &TileFonts,
    state: TileState,
    tile_radius: i32,
    panel_back: Color,
) -> Result<()> {
    let TileState {
        current,
        active,
        back,
        text,
    } = state;
    canvas.fill_rect(tile.rect, back);

    let glyph_height = (tile.rect.bottom - tile.rect.top) * 6 / 10;
    let glyph_rect = RECT {
        bottom: tile.rect.top + glyph_height,
        ..tile.rect
    };
    let label_rect = RECT {
        top: glyph_rect.bottom,
        ..tile.rect
    };

    match item.icon.as_deref() {
        Some(icon) => {
            canvas.draw_text_color(icon, glyph_rect, &fonts.icon, text, TextAlign::Centre)?;
            if let Some(key) = item.key {
                // A muted blend of `text` toward this tile's own `back`, not
                // a reduced alpha: draw_text stamps the alpha of its *whole*
                // rect to the colour it drew with, so anything less than
                // fully opaque here would punch a translucent hole in an
                // otherwise solid tile. Blending the hue instead keeps the
                // badge fully opaque while still reading as a secondary hint
                // next to the icon's full-strength colour — the colour
                // difference this module's own doc comment on the corner
                // badge motivates.
                let badge_color = muted(text, back);
                let badge_margin = 4;
                let badge_rect = RECT {
                    left: tile.rect.left + badge_margin,
                    top: tile.rect.top + badge_margin,
                    right: tile.rect.left + (tile.rect.right - tile.rect.left) * 2 / 5,
                    bottom: tile.rect.top + (tile.rect.bottom - tile.rect.top) * 2 / 5,
                };
                canvas.draw_text(
                    &key_name(key),
                    badge_rect,
                    &fonts.badge,
                    badge_color,
                    TextAlign::Left,
                )?;
            }
        }
        None => {
            let key = item.key.map(key_name).unwrap_or_default();
            canvas.draw_text(&key, glyph_rect, &fonts.key, text, TextAlign::Centre)?;
        }
    }

    let label = if current {
        Some("Current")
    } else if active {
        item.label_on.as_deref()
    } else {
        None
    }
    .or(item.label.as_deref());
    if let Some(label) = label {
        canvas.draw_text(label, label_rect, &fonts.label, text, TextAlign::Centre)?;
    }

    // Last, now that every GDI call touching this tile is done — same
    // ordering requirement as the panel's own `round_corners` in
    // `draw_panel_border`, and for the same reason (text/icon drawing can
    // bleed opaque pixels into the corner region a small radius carves
    // out). Blends toward the panel's own background rather than toward
    // transparent: a tile sits on the popup, not the desktop.
    canvas.round_rect_corners(tile.rect, tile_radius, panel_back);
    Ok(())
}

/// A thin outline around the whole popup, distinct from the accent-coloured
/// `focus_border` around the *target* window — just enough definition to
/// read as a floating panel against whatever is behind it. Drawn last,
/// before rounding, same as every tile: rounding has to see the finished
/// image to cut a clean corner out of it.
///
/// `stroke_rect` draws it straight, corners included; the corner fix-up is
/// `round_corners_with_border`'s job, not a plain `round_corners` — see
/// that method's own doc comment for why a straight stroke rounded off
/// after the fact does not actually produce a border that follows the
/// curve.
fn draw_panel_border(canvas: &mut Canvas, bounds: RECT, config: &OverlayConfig) {
    canvas.stroke_rect(bounds, config.palette.panel_border);
    canvas.round_corners_with_border(
        config.corner_radius as i32,
        PANEL_BORDER_WIDTH,
        config.palette.panel_border,
    );
}

impl Drop for ActionOverlay {
    fn drop(&mut self) {
        self.close();
    }
}

/// A 60/40 blend of `text` toward `back`, fully opaque — the key badge's
/// own colour, distinct in hue from `text` at a glance without touching
/// alpha (see the badge's own call site for why alpha is off the table
/// here).
fn muted(text: Color, back: Color) -> Color {
    let mix = |t: u8, b: u8| ((t as u16 * 3 + b as u16 * 2) / 5) as u8;
    Color::rgb(
        mix(text.r, back.r),
        mix(text.g, back.g),
        mix(text.b, back.b),
    )
}

/// Whether `pt` falls inside `rect` — used to hit-test tiles for both a
/// click and a hover.
fn point_in(rect: RECT, pt: POINT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

/// Whether `item` is the `desktop-move` tile for `current_desktop` (0-based,
/// [`ActionOverlay::current_desktop`]) — the one `desktop-list` tile that
/// would be pointless to click, drawn disabled and unclickable rather than
/// left to pretend a move to it would do anything. A free function, not a
/// method: [`ActionOverlay::paint`] needs it while `self.overlay` is already
/// borrowed mutably (for the canvas it is drawing into), so it takes the one
/// field it needs by value instead of borrowing all of `self` again.
fn is_current_desktop(current_desktop: Option<u32>, item: &Item) -> bool {
    item.action.takes_desktop() && current_desktop.is_some_and(|index| index + 1 == item.desktop)
}

/// Centres `size` on `window`'s own bounds — the action overlay opens over
/// the window it acts on, not at the cursor.
fn centre_on(window: HWND, size: (i32, i32)) -> POINT {
    let rect = mouse::visual_window_rect(window).unwrap_or_default();
    POINT {
        x: rect.left + (rect.right - rect.left - size.0) / 2,
        y: rect.top + (rect.bottom - rect.top - size.1) / 2,
    }
}

/// Lays out every tile, against `desktop_count` live virtual desktops —
/// `desktop-list` (see this module's doc comment) expands into that many
/// [`Action::DesktopMove`] tiles first, everything else passes through
/// unchanged. Ungrouped items fill `config.columns`-wide rows in
/// declaration order; each `<group>` gets its own row(s) below that, using
/// its own overrides where set. Called from [`ActionOverlay::new`] and
/// again from [`ActionOverlay::show`] whenever `desktop_count` has changed
/// since the last call.
fn layout(config: &OverlayConfig, desktop_count: u32) -> (Vec<Tile>, Vec<Item>, (i32, i32)) {
    let default_tile = (config.tile.0 as i32, config.tile.1 as i32);
    let default_columns = config.columns.max(1) as i32;

    let resolved: Vec<Item> = config
        .items
        .iter()
        .flat_map(|item| {
            if item.action.is_desktop_list() {
                expand_desktop_list(item, desktop_count)
            } else {
                vec![item.clone()]
            }
        })
        .collect();

    let items: Vec<(usize, &Item)> = resolved.iter().enumerate().collect();

    let has_group = |item: &Item| {
        item.group
            .as_deref()
            .is_some_and(|id| config.groups.iter().any(|group| group.id == id))
    };

    let mut tiles = Vec::new();
    let mut width = 0;
    let mut cursor_y = 0;

    let ungrouped: Vec<_> = items.iter().filter(|(_, item)| !has_group(item)).collect();
    if !ungrouped.is_empty() {
        place(&ungrouped, default_columns, default_tile, 0, &mut tiles);
        let rows = (ungrouped.len() as i32 - 1) / default_columns + 1;
        width = width.max(default_columns * default_tile.0);
        cursor_y = rows * default_tile.1;
    }

    for group in &config.groups {
        let members: Vec<_> = items
            .iter()
            .filter(|(_, item)| item.group.as_deref() == Some(group.id.as_str()))
            .collect();
        if members.is_empty() {
            continue;
        }

        let columns = group.columns.map_or(default_columns, |c| c.max(1) as i32);
        let tile = (
            group.tile_width.map_or(default_tile.0, |w| w as i32),
            group.tile_height.map_or(default_tile.1, |h| h as i32),
        );

        place(&members, columns, tile, cursor_y, &mut tiles);
        let rows = (members.len() as i32 - 1) / columns + 1;
        width = width.max(columns * tile.0);
        cursor_y += rows * tile.1;
    }

    // Shift the whole grid, laid out above from (0, 0), by PANEL_PADDING —
    // simpler than threading an offset through `place` and every width/
    // cursor_y update above it, and just as correct since every tile moves
    // by the same amount.
    for tile in &mut tiles {
        tile.rect.left += PANEL_PADDING;
        tile.rect.right += PANEL_PADDING;
        tile.rect.top += PANEL_PADDING;
        tile.rect.bottom += PANEL_PADDING;
    }

    (
        tiles,
        resolved,
        (
            width.max(1) + PANEL_PADDING * 2,
            cursor_y.max(1) + PANEL_PADDING * 2,
        ),
    )
}

/// `source`'s expansion into one tile per desktop, numbered from 1 —
/// [`layout`]'s replacement for the single `desktop-list` line in the
/// configuration. `label`/`group`/`icon` are what a generated tile shows
/// and where it sits; `key` follows the same numbers, `1`-`9`, `None` past
/// the ninth (mouse-click only — matching every other "ran out of a fixed
/// resource" fallback in this codebase, and there is no realistic grid this
/// wide to begin with).
fn expand_desktop_list(source: &Item, desktop_count: u32) -> Vec<Item> {
    // `desktop-list` tiles follow the window to its desktop, `desktop-send-list`
    // ones leave you where you are.
    let action = if source.action == Action::DesktopSendList {
        Action::DesktopSend
    } else {
        Action::DesktopMove
    };
    (1..=desktop_count)
        .map(|desktop| Item {
            key: desktop_key(desktop),
            action,
            label: Some(desktop.to_string()),
            label_on: None,
            icon: source.icon.clone(),
            group: source.group.clone(),
            desktop,
        })
        .collect()
}

/// `VK_1`..`VK_9` for desktop numbers 1-9, `None` past that.
fn desktop_key(desktop: u32) -> Option<VIRTUAL_KEY> {
    let offset = u16::try_from(desktop.checked_sub(1)?).ok()?;
    (offset < 9).then(|| VIRTUAL_KEY(VK_1.0 + offset))
}

/// Fills `tiles` with one entry per `(index, _)` in `items`, `columns` wide,
/// `tile`-sized, starting `top` pixels down.
fn place(
    items: &[&(usize, &Item)],
    columns: i32,
    tile: (i32, i32),
    top: i32,
    tiles: &mut Vec<Tile>,
) {
    for (position, (index, _)) in items.iter().enumerate() {
        let position = position as i32;
        let (col, row) = (position % columns, position / columns);
        tiles.push(Tile {
            item: *index,
            rect: RECT {
                left: col * tile.0,
                top: top + row * tile.1,
                right: col * tile.0 + tile.0,
                bottom: top + row * tile.1 + tile.1,
            },
        });
    }
}

/// The popup's own window handler: turns a click on a tile into a message
/// the owner window can act on — the same shape as
/// [`crate::min_overlay::ClickHandler`], just with a real grid of rects to
/// hit-test instead of one evenly divided row — and additionally tracks
/// which tile the pointer is over, so the tiles can react to a hover the
/// same way a click does.
struct ClickHandler {
    owner: HWND,
    /// Tile bounds, indexed the same way [`ActionOverlay::tiles`] is.
    tiles: Vec<RECT>,
    /// `(key, tile index)`, only for tiles that have one — built once in
    /// [`ActionOverlay::show`] from the same tile list `tiles` above mirrors.
    keys: Vec<(VIRTUAL_KEY, usize)>,
    /// This handler's own idea of which tile is hovered, purely to decide
    /// when a [`WM_OVERLAY_HOVER`] is worth posting — [`ActionOverlay::
    /// hovered`] is the copy that actually drives painting, and the two are
    /// allowed to disagree briefly (`show`'s own seeding from the cursor
    /// position races this one's WM_MOUSEMOVE tracking); the worst that
    /// happens is one redundant post, which [`ActionOverlay::set_hover`]
    /// already no-ops on.
    hovered: Option<usize>,
}

impl WindowHandler for ClickHandler {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let key = VIRTUAL_KEY(wparam.0 as u16);
                if key == VK_ESCAPE {
                    self.post(WM_OVERLAY_CLOSE, 0);
                } else if let Some(&(_, index)) = self.keys.iter().find(|(k, _)| *k == key) {
                    self.post(WM_OVERLAY_ACTION, index);
                }
                Some(LRESULT(0))
            }

            // The window lost keyboard focus — a click outside the overlay,
            // Alt+Tab, another window taking the foreground. Same meaning as
            // Escape: close without acting. This is what makes "click
            // outside closes it" work without a mouse hook — see this
            // module's doc comment.
            WM_KILLFOCUS => {
                self.post(WM_OVERLAY_CLOSE, 0);
                Some(LRESULT(0))
            }

            WM_LBUTTONUP => {
                let pt = point_from_lparam(lparam);
                if let Some(index) = self.tiles.iter().position(|rect| point_in(*rect, pt)) {
                    self.post(WM_OVERLAY_ACTION, index);
                }
                Some(LRESULT(0))
            }

            WM_MOUSEMOVE => {
                let pt = point_from_lparam(lparam);
                let hit = self.tiles.iter().position(|rect| point_in(*rect, pt));
                if hit != self.hovered {
                    self.hovered = hit;
                    self.post_hover();
                }
                // Armed on every move, not just the first: TRACKMOUSEEVENT's
                // leave notification is one-shot, so it has to be re-armed
                // each time or only the first WM_MOUSELEAVE after the popup
                // shows would ever fire.
                track_leave(hwnd);
                Some(LRESULT(0))
            }

            WM_MOUSELEAVE => {
                if self.hovered.is_some() {
                    self.hovered = None;
                    self.post_hover();
                }
                Some(LRESULT(0))
            }

            _ => None,
        }
    }
}

impl ClickHandler {
    /// Posts `msg` to the owner window with `param` as its `wParam` —
    /// [`WM_OVERLAY_ACTION`] and [`WM_OVERLAY_CLOSE`] both just need an
    /// index (0 for close) delivered this way.
    fn post(&self, msg: u32, param: usize) {
        let _ = unsafe { PostMessageW(Some(self.owner), msg, WPARAM(param), LPARAM(0)) };
    }

    fn post_hover(&self) {
        let index = self.hovered.map_or(usize::MAX, |index| index);
        self.post(WM_OVERLAY_HOVER, index);
    }
}

/// Screen — really client — coordinates packed into a mouse message's
/// `lParam`, the same decoding [`WM_LBUTTONUP`] already relied on.
fn point_from_lparam(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: (lparam.0 >> 16 & 0xFFFF) as i16 as i32,
    }
}

/// Arms a one-shot [`WM_MOUSELEAVE`] for `hwnd` — see [`ClickHandler`]'s
/// `WM_MOUSEMOVE` handling for why this is called on every move rather than
/// once.
fn track_leave(hwnd: HWND) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    let _ = unsafe { TrackMouseEvent(&mut event) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Group;

    fn desktop_list_item() -> Item {
        Item {
            key: None,
            action: Action::DesktopList,
            label: None,
            label_on: None,
            icon: Some("\u{e8a9}".to_owned()),
            group: Some("desktops".to_owned()),
            desktop: 0,
        }
    }

    #[test]
    fn desktop_list_expands_into_one_tile_per_desktop() {
        let config = OverlayConfig {
            groups: vec![Group {
                id: "desktops".to_owned(),
                columns: Some(9),
                tile_width: None,
                tile_height: Some(48),
            }],
            items: vec![desktop_list_item()],
            ..OverlayConfig::default()
        };

        let (tiles, items, _size) = layout(&config, 3);

        assert_eq!(tiles.len(), 3);
        let mut moves: Vec<&Item> = tiles.iter().map(|tile| &items[tile.item]).collect();
        moves.sort_by_key(|item| item.desktop);

        for (index, item) in moves.iter().enumerate() {
            let desktop = index as u32 + 1;
            assert_eq!(item.action, Action::DesktopMove);
            assert_eq!(item.desktop, desktop);
            assert_eq!(item.label.as_deref(), Some(desktop.to_string().as_str()));
            assert_eq!(item.group.as_deref(), Some("desktops"));
            assert_eq!(item.icon.as_deref(), Some("\u{e8a9}"));
        }
    }

    #[test]
    fn desktop_send_list_expands_into_send_tiles() {
        let mut source = desktop_list_item();
        source.action = Action::DesktopSendList;
        let config = OverlayConfig {
            items: vec![source],
            ..OverlayConfig::default()
        };

        let (_tiles, items, _size) = layout(&config, 3);

        assert_eq!(items.len(), 3);
        for (index, item) in items.iter().enumerate() {
            assert_eq!(item.action, Action::DesktopSend);
            assert_eq!(item.desktop, index as u32 + 1);
        }
        // Sending to the desktop you are on is as pointless as moving to it.
        assert!(is_current_desktop(Some(1), &items[1]));
    }

    #[test]
    fn desktop_list_tiles_are_keyed_1_through_9_then_unkeyed() {
        let config = OverlayConfig {
            groups: vec![Group {
                id: "desktops".to_owned(),
                columns: None,
                tile_width: None,
                tile_height: None,
            }],
            items: vec![desktop_list_item()],
            ..OverlayConfig::default()
        };

        let (tiles, items, _size) = layout(&config, 11);
        let mut moves: Vec<&Item> = tiles.iter().map(|tile| &items[tile.item]).collect();
        moves.sort_by_key(|item| item.desktop);

        for (index, item) in moves.iter().enumerate() {
            let expected = (index < 9).then(|| VIRTUAL_KEY(VK_1.0 + index as u16));
            assert_eq!(item.key, expected, "desktop {}", index + 1);
        }
    }

    #[test]
    fn zero_desktops_expands_to_no_tiles() {
        let config = OverlayConfig {
            items: vec![desktop_list_item()],
            ..OverlayConfig::default()
        };
        let (tiles, items, _size) = layout(&config, 0);
        assert!(tiles.is_empty());
        assert!(items.is_empty());
    }

    #[test]
    fn only_the_current_desktops_own_tile_is_flagged() {
        let config = OverlayConfig {
            groups: vec![Group {
                id: "desktops".to_owned(),
                columns: None,
                tile_width: None,
                tile_height: None,
            }],
            items: vec![desktop_list_item()],
            ..OverlayConfig::default()
        };
        let (tiles, items, _size) = layout(&config, 3);
        let moves: Vec<&Item> = tiles.iter().map(|tile| &items[tile.item]).collect();

        // 0-based current index 1 == the 1-based desktop-2 tile.
        for item in &moves {
            assert_eq!(is_current_desktop(Some(1), item), item.desktop == 2);
        }

        // No live current desktop (COM unavailable): nothing is flagged.
        for item in &moves {
            assert!(!is_current_desktop(None, item));
        }

        // Only ever true for desktop-move tiles — an ordinary action sharing
        // the same 1-based number by coincidence must not be flagged.
        let unrelated = Item {
            key: None,
            action: Action::Minimize,
            label: None,
            label_on: None,
            icon: None,
            group: None,
            desktop: 2,
        };
        assert!(!is_current_desktop(Some(1), &unrelated));
    }
}
