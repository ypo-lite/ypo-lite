//! y-snap Config — a WinUI 3 settings app for `y-snap.xml`, built on
//! `windows-reactor`.
//!
//! Module map:
//! - [`ui`] — the app-agnostic toolkit every screen is built from (tokens,
//!   [`ui::viewport::Viewport`], layout primitives, controls, table,
//!   editor shell). **Read `ui-kit/docs/ui-toolkit.md` before
//!   adding or changing a screen.**
//! - [`shell`] — the window frame (nav pane, page scroller, Save bar) and
//!   the content [`ui::viewport::Viewport`] it leaves for screens.
//! - [`keys`] / [`key_picker`] — the key catalog and the picker built on it;
//!   [`icons`] / [`icon_picker`] — the same for tile icons.
//! - one module per screen: [`hotkeys`], [`mouse`], [`overlay`], [`min_button`].
//!
//! State lives here, in one place: `ConfigApp` owns the loaded `Settings`
//! plus page-local UI state, every screen is a pure function of that state
//! and a `Viewport`, and every [`Msg`] is settled in [`ConfigApp::update`]
//! — one match arm per message, next to the one `Settings` they all edit.
//!
//! **Editing model.** Edits commit to `settings` as they happen, so the
//! table row and the live preview update while an editor is still open.
//! At most one inline editor is open at a time ([`Editing`]); opening one
//! snapshots what it edits, Cancel restores that snapshot, Done keeps the
//! edits. Opening another editor, deleting a row, or reloading closes the
//! current one first (keeping its edits) — row indices can't go stale
//! under an open editor.

#![windows_subsystem = "windows"]

mod hotkeys;
mod icon_picker;
mod icons;
mod key_picker;
mod min_button;
mod mouse;
mod overlay;
mod reorder;

use ypo_ui as ui;
use ui::shell;
use ypo_ui::keys;

use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, VIRTUAL_KEY};
use windows_reactor::*;
use y_core::hotkey::Hotkey;
use y_core::overlay::Color as CoreColor;
use y_core::single_instance;
use y_snap::action::{Action, Target};
use y_snap::config::{Binding, Button as MouseButton, Gesture, Group, Item, LoadError, QuickAction, Settings, Theme};
use y_snap::ipc;

use icon_picker::IconPickerUi;
use icons::IconCategory;
use key_picker::KeyPickerUi;
use keys::KeyCategory;
use mouse::GestureKind;
use overlay::OverlaySection;
use overlay::palette::PaletteField;
use ui::prelude::*;

fn main() {
    // One editor at a time: two windows over one file would overwrite each
    // other's saves. A second launch (the tray's "Settings…" clicked again)
    // brings the open one forward instead. Held until the process exits.
    let _instance = match single_instance::SingleInstance::acquire(ipc::CONFIG_INSTANCE) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            single_instance::bring_to_front(ipc::CONFIG_TITLE);
            return;
        }
        // Couldn't even create the mutex: open anyway — losing the
        // one-window guarantee beats not opening at all.
        Err(_) => None,
    };
    App::run_component::<ConfigApp>(()).unwrap();
}

struct ConfigApp {
    settings: Result<Settings, LoadError>,
    section: Section,
    /// Whether anything has changed since the last load or save.
    dirty: bool,
    save_error: Option<String>,
    /// Whether the user closed the load-warnings banner. Reset by Reload;
    /// the warnings themselves live in `Settings::warnings`.
    warnings_dismissed: bool,
    /// After the last successful save: whether a running y-snap was asked to
    /// reload it (`Some(true)`), or none was running (`Some(false)`).
    applied: Option<bool>,
    /// The list whose rows currently show ↑/↓ instead of their actions.
    reordering: Option<ListKind>,
    /// The one open inline editor, if any, with its Cancel snapshot.
    editing: Option<Editing>,
    /// The key picker's own state (category tab, search text) — shared by
    /// whichever editor is showing a picker, since only one ever is.
    key_picker: KeyPickerUi,
    /// The icon picker inside the open tile editor (Overlay or Minimize
    /// button) — closed whenever a new editor opens.
    icon_picker: IconPickerUi,
    overlay_section: OverlaySection,
    /// Renders what is copied while the window is alive — see
    /// `y_core::shell::ClipboardFlusher`. Held only for its `Drop`.
    _clipboard: Option<y_core::shell::ClipboardFlusher>,
    /// The window's client width in DIPs, from `on_window_size`. Starts at
    /// the initial window width; the first delivery corrects it.
    window_width: f64,
}

/// What the open inline editor edits, and the value to restore on Cancel.
#[derive(Clone, Debug)]
enum Editing {
    Hotkey { index: usize, original: Hotkey },
    Tile { index: usize, original: Item },
    QuickAction { index: usize, original: QuickAction },
    Palette { field: PaletteField, original: CoreColor },
}

/// A reorderable list — what [`Msg::ReorderToggled`] and [`Msg::Moved`] are
/// about. Order is meaningful in all four: tiles and groups are laid out in
/// file order in their popups; hotkeys only for reading the file, but
/// that's still the user's to arrange.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ListKind {
    Hotkeys,
    OverlayGroups,
    OverlayTiles,
    PopupTiles,
}

/// One entry in the navigation pane, in the order shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    Hotkeys,
    Mouse,
    Overlay,
    MinButton,
}

impl Section {
    const ALL: [Self; 4] = [Self::Hotkeys, Self::Mouse, Self::Overlay, Self::MinButton];

    /// `NavigationViewItem`'s tag, round-tripped by `on_selected_tag_changed`.
    fn tag(self) -> &'static str {
        match self {
            Self::Hotkeys => "hotkeys",
            Self::Mouse => "mouse",
            Self::Overlay => "overlay",
            Self::MinButton => "minbutton",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.tag() == tag)
    }

    fn title(self) -> &'static str {
        match self {
            Self::Hotkeys => "Hotkeys",
            Self::Mouse => "Mouse gestures",
            Self::Overlay => "Action overlay",
            Self::MinButton => "Minimize button",
        }
    }

    fn icon(self) -> Symbol {
        match self {
            Self::Hotkeys => Symbol::Keyboard,
            Self::Mouse => Symbol::TouchPointer,
            Self::Overlay => Symbol::ViewAll,
            Self::MinButton => Symbol::BackToWindow,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditorClose {
    /// Done: keep what was edited.
    Keep,
    /// Cancel: restore the snapshot taken on open.
    Revert,
}

enum Msg {
    SelectSection(Section),
    WindowResized(WindowSize),
    OverlaySectionSelected(OverlaySection),
    Save,
    WarningsDismissed,
    ReorderToggled(ListKind),
    /// Swap row `index` with its neighbour above (`up`) or below.
    Moved {
        list: ListKind,
        index: usize,
        up: bool,
    },
    Reload,

    // The inline editor and the key picker inside it.
    EditHotkey(usize),
    EditTile(usize),
    EditPalette(PaletteField),
    EditorClosed(EditorClose),
    KeyPicked(VIRTUAL_KEY),
    ModifierToggled(HOT_KEY_MODIFIERS),
    KeyCategorySelected(KeyCategory),
    KeySearchChanged(String),
    KeySearchChosen(String),
    IconPickerToggled,
    IconCategorySelected(IconCategory),
    IconSearchChanged(String),
    IconPicked(&'static str),

    HotkeyActionChanged(usize, Option<usize>),
    HotkeyTargetChanged(usize, Option<usize>),
    HotkeyDesktopChanged(usize, Option<f64>),
    HotkeyStealToggled(usize, bool),
    HotkeyDeleted(usize),
    HotkeyAdded,

    GestureToggled(GestureKind, bool),
    GestureButtonChanged(GestureKind, Option<usize>),
    GestureModifierToggled(GestureKind, HOT_KEY_MODIFIERS),

    OverlayColumnsChanged(Option<f64>),
    OverlayTileWidthChanged(Option<f64>),
    OverlayTileHeightChanged(Option<f64>),
    OverlayCornerRadiusChanged(Option<f64>),
    OverlayBorderWidthChanged(Option<f64>),
    OverlayAlphaChanged(Option<f64>),
    OverlayFocusBorderToggled(bool),
    OverlayPaletteColorChanged(Color),
    OverlayThemeChanged(Option<usize>),
    OverlayPaletteReset,
    OverlayGroupAdded,
    OverlayGroupDeleted(usize),
    OverlayGroupColumnsChanged(usize, Option<f64>),
    OverlayGroupTileWidthChanged(usize, Option<f64>),
    OverlayGroupTileHeightChanged(usize, Option<f64>),
    OverlayItemActionChanged(usize, Option<usize>),
    OverlayItemLabelChanged(usize, String),
    OverlayItemIconChanged(usize, String),
    OverlayItemGroupChanged(usize, Option<usize>),
    OverlayItemDesktopChanged(usize, Option<f64>),
    OverlayItemDeleted(usize),
    OverlayItemAdded,

    MinButtonDelayChanged(Option<f64>),
    MinButtonRightClickChanged(Option<usize>),
    EditQuickAction(usize),
    QuickActionActionChanged(usize, Option<usize>),
    QuickActionLabelChanged(usize, String),
    QuickActionIconChanged(usize, String),
    QuickActionDeleted(usize),
    QuickActionAdded,
}

impl Component for ConfigApp {
    type Input = ();
    type Message = Msg;

    fn create(_input: &(), _context: &ComponentContext<Self>) -> Self {
        Self {
            settings: Settings::load(),
            section: Section::Hotkeys,
            dirty: false,
            save_error: None,
            warnings_dismissed: false,
            applied: None,
            reordering: None,
            editing: None,
            key_picker: KeyPickerUi::default(),
            icon_picker: IconPickerUi::default(),
            overlay_section: OverlaySection::Layout,
            _clipboard: y_core::shell::ClipboardFlusher::start(),
            window_width: shell::INITIAL_WINDOW.0,
        }
    }

    fn update(&mut self, message: Msg, _context: &ComponentContext<Self>) {
        match message {
            Msg::SelectSection(section) => self.section = section,
            Msg::OverlaySectionSelected(section) => self.overlay_section = section,
            Msg::WindowResized(size) => self.window_width = size.width,
            Msg::KeyCategorySelected(category) => self.key_picker.category = category,
            Msg::KeySearchChanged(text) => self.key_picker.search = text,
            Msg::IconCategorySelected(category) => self.icon_picker.category = category,
            Msg::IconSearchChanged(text) => self.icon_picker.search = text,
            Msg::WarningsDismissed => self.warnings_dismissed = true,
            Msg::Reload => {
                self.editing = None;
                self.settings = Settings::load();
                self.dirty = false;
                self.save_error = None;
                self.warnings_dismissed = false;
                self.applied = None;
                self.reordering = None;
            }
            message => self.edit(message),
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title(ipc::CONFIG_TITLE);
        let (width, height) = shell::INITIAL_WINDOW;
        let (min_width, min_height) = shell::MIN_WINDOW;
        context.window_visuals(
            WindowVisuals::new().backdrop(WindowBackdrop::Mica).client_size(width, height).constraints(
                WindowConstraints { min_width: Some(min_width), min_height: Some(min_height), ..Default::default() },
            ),
        );
        context.on_window_size(context.callback(Msg::WindowResized));

        let layout = shell::layout(self.window_width);
        let settings = match &self.settings {
            Ok(settings) => settings,
            // Nothing to edit if the file can't be read — say why, plainly.
            Err(error) => {
                return shell::page_frame(page("Couldn't load y-snap.xml", &error.to_string(), View::empty()), layout);
            }
        };

        let viewport = layout.content;
        let screen = match self.section {
            Section::Hotkeys => {
                let editing = match self.editing {
                    Some(Editing::Hotkey { index, .. }) => Some(index),
                    _ => None,
                };
                let reordering = self.reordering == Some(ListKind::Hotkeys);
                hotkeys::hotkeys_view(&settings.hotkeys, editing, reordering, &self.key_picker, viewport, context)
            }
            Section::Mouse => mouse::mouse_view(&settings.mouse, viewport, context),
            Section::Overlay => {
                let (tile, palette) = match self.editing {
                    Some(Editing::Tile { index, .. }) => (Some(index), None),
                    Some(Editing::Palette { field, .. }) => (None, Some(field)),
                    _ => (None, None),
                };
                overlay::overlay_view(
                    &settings.overlay,
                    overlay::OverlayState {
                        section: self.overlay_section,
                        editing_tile: tile,
                        editing_palette: palette,
                        reordering_groups: self.reordering == Some(ListKind::OverlayGroups),
                        reordering_tiles: self.reordering == Some(ListKind::OverlayTiles),
                    },
                    &self.key_picker,
                    &self.icon_picker,
                    viewport,
                    context,
                )
            }
            Section::MinButton => {
                let editing = match self.editing {
                    Some(Editing::QuickAction { index, .. }) => Some(index),
                    _ => None,
                };
                min_button::min_button_view(
                    &settings.min_button,
                    &settings.overlay,
                    editing,
                    self.reordering == Some(ListKind::PopupTiles),
                    &self.icon_picker,
                    viewport,
                    context,
                )
            }
        };
        // Above every screen, not just one: what was skipped can be in any
        // section, and Save drops it whichever screen it's pressed from.
        let banner = (!settings.warnings.is_empty() && !self.warnings_dismissed)
            .then(|| warnings_banner(&settings.warnings, context.callback(|()| Msg::WarningsDismissed)));
        let screen: View = Stack::vertical(space::L).maybe("warnings", banner).child("screen", screen).into();

        let current = self.section;
        let items = Section::ALL.map(|section| {
            let view: View = NavigationViewItem::new().tag(section.tag()).is_selected(section == current).slots([
                SlotView::new(NavigationViewItemSlot::Icon, SymbolIcon::new().symbol(section.icon())),
                SlotView::new(NavigationViewItemSlot::Content, section.title()),
            ]);
            (section.tag(), view)
        });

        let mut slots = vec![
            SlotView::collection(NavigationViewSlot::MenuItems, items),
            SlotView::new(NavigationViewSlot::Content, shell::page_frame(screen, layout)),
        ];
        // The file path only fits a labelled pane; on the icon rail it's in
        // the status tooltip instead.
        if layout.pane_expanded {
            let footer = Stack::vertical(space::XXS)
                .child("caption", text::caption("Loaded from"))
                .child("path", text::caption(settings.path.display().to_string()).opacity(emphasis::SUBTLE));
            slots.push(SlotView::new(
                NavigationViewSlot::PaneFooter,
                Border::new().padding(Thickness::uniform(space::M)).content(footer),
            ));
        }

        let nav = shell::pane_mode(NavigationView::new().pane_title("y-snap"), layout)
            .on_selected_tag_changed(context.callback(move |tag: Option<String>| {
                Msg::SelectSection(tag.as_deref().and_then(Section::from_tag).unwrap_or(current))
            }))
            .grid_row(0)
            .slots(slots);

        let dropped = settings.warnings.len();
        let (status, healthy) = match &self.save_error {
            Some(error) => (format!("Save failed: {error}"), false),
            None if self.dirty && dropped > 0 => (
                format!(
                    "Unsaved changes — saving drops what couldn't be read ({dropped} problem{})",
                    if dropped == 1 { "" } else { "s" }
                ),
                false,
            ),
            None if self.dirty => ("Unsaved changes".to_owned(), true),
            None => match self.applied {
                Some(true) => ("Saved and applied — y-snap is reloading it".to_owned(), true),
                Some(false) => {
                    ("Saved — y-snap isn't running; it'll use these settings when it starts".to_owned(), true)
                }
                None => (summary(settings), true),
            },
        };
        let detail = format!("{status}\n{}", settings.path.display());
        let bar = shell::action_bar(
            shell::BarStatus {
                text: &status,
                detail: &detail,
                healthy,
                dirty: self.dirty,
                save_tooltip: "Rewrites y-snap.xml from these settings. Comments and hand formatting in the file \
                               aren't kept.",
                extra: None,
                tools: None,
            },
            context.callback(|()| Msg::Reload),
            context.callback(|()| Msg::Save),
        );

        // Nav takes the height the bar doesn't (`Star` over `Auto`).
        Grid::new().rows([GridLength::STAR, GridLength::Auto]).keyed_children([("nav", nav), ("bar", bar)])
    }
}

/// The load-warnings banner in y-snap's words: what was skipped isn't in
/// `Settings`, so a save leaves it out of the file.
fn warnings_banner(warnings: &[String], on_close: Callback<()>) -> View {
    let count = warnings.len();
    shell::warnings_banner(
        format!("{count} part{} of y-snap.xml couldn't be read", if count == 1 { "" } else { "s" }),
        "They were skipped, and saving will leave them out of the file — fix them in the file first if you need \
         them, then Reload.",
        warnings,
        on_close,
    )
}

/// "5 hotkeys · 16 overlay tiles" — the status line when nothing's pending.
fn summary(settings: &Settings) -> String {
    format!("Saved — {} hotkeys · {} overlay tiles", settings.hotkeys.len(), settings.overlay.items.len())
}

impl ConfigApp {
    /// Settles every message [`Component::update`] doesn't: all of them
    /// edit the loaded `Settings` (or the editor over it). Does nothing if
    /// the file failed to load.
    fn edit(&mut self, message: Msg) {
        let Ok(settings) = &mut self.settings else {
            return;
        };

        match message {
            Msg::SelectSection(_)
            | Msg::OverlaySectionSelected(_)
            | Msg::WindowResized(_)
            | Msg::KeyCategorySelected(_)
            | Msg::KeySearchChanged(_)
            | Msg::IconCategorySelected(_)
            | Msg::IconSearchChanged(_)
            | Msg::WarningsDismissed
            | Msg::Reload => unreachable!("settled by Component::update"),

            Msg::EditHotkey(index) => {
                self.reordering = None;
                let reopen = !matches!(self.editing, Some(Editing::Hotkey { index: open, .. }) if open == index);
                self.editing = None;
                if reopen {
                    let original = settings.hotkeys[index].hotkey;
                    self.key_picker = KeyPickerUi::opening_on(Some(original.key));
                    self.editing = Some(Editing::Hotkey { index, original });
                }
            }
            Msg::EditTile(index) => {
                self.reordering = None;
                let reopen = !matches!(self.editing, Some(Editing::Tile { index: open, .. }) if open == index);
                self.editing = None;
                if reopen {
                    let original = settings.overlay.items[index].clone();
                    self.key_picker = KeyPickerUi::opening_on(original.key);
                    self.icon_picker = IconPickerUi::default();
                    self.editing = Some(Editing::Tile { index, original });
                }
            }
            Msg::EditPalette(field) => {
                self.reordering = None;
                let reopen = !matches!(self.editing, Some(Editing::Palette { field: open, .. }) if open == field);
                self.editing = None;
                if reopen {
                    let original = field.get(&settings.overlay.palette);
                    self.editing = Some(Editing::Palette { field, original });
                }
            }
            Msg::EditorClosed(close) => {
                if let (Some(editing), EditorClose::Revert) = (self.editing.take(), close) {
                    match editing {
                        Editing::Hotkey { index, original } => settings.hotkeys[index].hotkey = original,
                        Editing::Tile { index, original } => settings.overlay.items[index] = original,
                        Editing::QuickAction { index, original } => settings.min_button.items[index] = original,
                        Editing::Palette { field, original } => {
                            *field.get_mut(&mut settings.overlay.palette) = original
                        }
                    }
                }
            }
            Msg::KeyPicked(key) => match self.editing {
                Some(Editing::Hotkey { index, .. }) => {
                    settings.hotkeys[index].hotkey.key = key;
                    self.dirty = true;
                }
                Some(Editing::Tile { index, .. }) => {
                    settings.overlay.items[index].key = Some(key);
                    self.dirty = true;
                }
                _ => {}
            },
            Msg::KeySearchChosen(suggestion) => {
                if let Some(key) = keys::from_suggestion(&suggestion) {
                    self.key_picker = KeyPickerUi::opening_on(Some(key));
                    self.edit(Msg::KeyPicked(key));
                }
            }
            Msg::ModifierToggled(modifier) => {
                if let Some(Editing::Hotkey { index, .. }) = self.editing {
                    settings.hotkeys[index].hotkey.modifiers.0 ^= modifier.0;
                    self.dirty = true;
                }
            }

            Msg::HotkeyActionChanged(index, Some(choice)) => {
                settings.hotkeys[index].action = hotkeys::ACTIONS[choice];
                self.dirty = true;
            }
            Msg::HotkeyTargetChanged(index, Some(choice)) => {
                settings.hotkeys[index].target = hotkeys::TARGETS[choice];
                self.dirty = true;
            }
            Msg::HotkeyDesktopChanged(index, Some(desktop)) => {
                settings.hotkeys[index].desktop = desktop.max(1.0) as u32;
                self.dirty = true;
            }
            Msg::HotkeyStealToggled(index, steal) => {
                settings.hotkeys[index].steal = steal;
                self.dirty = true;
            }
            Msg::HotkeyDeleted(index) => {
                self.editing = None;
                settings.hotkeys.remove(index);
                self.dirty = true;
            }
            Msg::HotkeyAdded => {
                self.reordering = None;
                // A harmless, unlikely-to-collide starting point, opened
                // straight into the key picker — picking the real combo is
                // the first thing anyone adding a hotkey does.
                let hotkey = Hotkey::parse("Ctrl+Alt+F13").expect("valid placeholder combo");
                settings.hotkeys.push(Binding {
                    hotkey,
                    action: Action::Minimize,
                    target: Target::Focused,
                    desktop: 0,
                    steal: false,
                });
                self.dirty = true;
                self.editing = None;
                let index = settings.hotkeys.len() - 1;
                self.edit(Msg::EditHotkey(index));
            }

            Msg::GestureToggled(kind, on) => {
                *kind.get_mut(&mut settings.mouse) =
                    on.then_some(Gesture { button: MouseButton::Left, modifiers: HOT_KEY_MODIFIERS(0) });
                self.dirty = true;
            }
            Msg::GestureButtonChanged(kind, Some(choice)) => {
                if let Some(gesture) = kind.get_mut(&mut settings.mouse) {
                    gesture.button = mouse::BUTTONS[choice];
                    self.dirty = true;
                }
            }
            Msg::GestureModifierToggled(kind, modifier) => {
                if let Some(gesture) = kind.get_mut(&mut settings.mouse) {
                    gesture.modifiers.0 ^= modifier.0;
                    self.dirty = true;
                }
            }

            // An empty `NumberBox` reports `None` while being cleared to
            // type a fresh value — "leave it", not "zero it".
            Msg::OverlayColumnsChanged(Some(columns)) => {
                settings.overlay.columns = columns.max(1.0) as u32;
                self.dirty = true;
            }
            Msg::OverlayTileWidthChanged(Some(width)) => {
                settings.overlay.tile.0 = width.max(1.0) as u32;
                self.dirty = true;
            }
            Msg::OverlayTileHeightChanged(Some(height)) => {
                settings.overlay.tile.1 = height.max(1.0) as u32;
                self.dirty = true;
            }
            Msg::OverlayCornerRadiusChanged(Some(radius)) => {
                settings.overlay.corner_radius = radius.max(0.0) as u32;
                self.dirty = true;
            }
            Msg::OverlayBorderWidthChanged(Some(width)) => {
                settings.overlay.border_width = width.max(0.0) as u32;
                self.dirty = true;
            }
            Msg::OverlayAlphaChanged(Some(alpha)) => {
                settings.overlay.alpha = alpha.clamp(0.0, 255.0) as u8;
                self.dirty = true;
            }
            Msg::OverlayFocusBorderToggled(on) => {
                settings.overlay.focus_border = on;
                self.dirty = true;
            }
            Msg::OverlayThemeChanged(Some(choice)) => {
                let theme = Theme::ALL[choice];
                // Also fires on first render; only an actual switch resets
                // the colours.
                if theme != settings.overlay.theme {
                    // A colour editor's snapshot would restore a colour of
                    // the old theme — close it rather than let Cancel do that.
                    if matches!(self.editing, Some(Editing::Palette { .. })) {
                        self.editing = None;
                    }
                    let overlay = &mut settings.overlay;
                    overlay.theme = theme;
                    overlay.theme_palette = theme.palette();
                    overlay.palette = overlay.theme_palette;
                    self.dirty = true;
                }
            }
            Msg::OverlayPaletteReset => {
                if matches!(self.editing, Some(Editing::Palette { .. })) {
                    self.editing = None;
                }
                settings.overlay.palette = settings.overlay.theme_palette;
                self.dirty = true;
            }
            Msg::OverlayPaletteColorChanged(color) => {
                if let Some(Editing::Palette { field, .. }) = self.editing {
                    *field.get_mut(&mut settings.overlay.palette) = overlay::palette::from_ui_color(color);
                    self.dirty = true;
                }
            }
            Msg::OverlayGroupAdded => {
                self.reordering = None;
                settings.overlay.groups.push(Group {
                    id: overlay::groups::next_group_id(&settings.overlay.groups),
                    columns: None,
                    tile_width: None,
                    tile_height: None,
                });
                self.dirty = true;
            }
            Msg::OverlayGroupDeleted(index) => {
                // Tiles in the deleted group fall back into the main grid
                // rather than pointing at a group that no longer exists.
                self.editing = None;
                let id = settings.overlay.groups.remove(index).id;
                for item in &mut settings.overlay.items {
                    if item.group.as_deref() == Some(id.as_str()) {
                        item.group = None;
                    }
                }
                self.dirty = true;
            }
            // `None` is meaningful here: it clears the override, so the
            // group falls back to the overlay's own value.
            Msg::OverlayGroupColumnsChanged(index, columns) => {
                settings.overlay.groups[index].columns = columns.map(|value| value.max(1.0) as u32);
                self.dirty = true;
            }
            Msg::OverlayGroupTileWidthChanged(index, width) => {
                settings.overlay.groups[index].tile_width = width.map(|value| value.max(1.0) as u32);
                self.dirty = true;
            }
            Msg::OverlayGroupTileHeightChanged(index, height) => {
                settings.overlay.groups[index].tile_height = height.map(|value| value.max(1.0) as u32);
                self.dirty = true;
            }
            Msg::OverlayItemActionChanged(index, Some(choice)) => {
                let action = overlay::OVERLAY_ACTIONS[choice];
                let item = &mut settings.overlay.items[index];
                item.icon = icons::after_action_change(item.icon.as_deref(), item.action, action);
                item.action = action;
                // `desktop-list` expands into per-desktop tiles that carry
                // their own keys; a key on the placeholder would be dead data.
                if action == Action::DesktopList {
                    item.key = None;
                }
                self.dirty = true;
            }
            Msg::OverlayItemLabelChanged(index, text) => {
                settings.overlay.items[index].label = (!text.trim().is_empty()).then_some(text);
                self.dirty = true;
            }
            Msg::OverlayItemIconChanged(index, text) => {
                settings.overlay.items[index].icon = (!text.trim().is_empty()).then_some(text);
                self.dirty = true;
            }
            Msg::OverlayItemGroupChanged(index, Some(choice)) => {
                // Choice 0 is always "(none)" — see `overlay::tiles`.
                settings.overlay.items[index].group =
                    choice.checked_sub(1).map(|group| settings.overlay.groups[group].id.clone());
                self.dirty = true;
            }
            Msg::OverlayItemDesktopChanged(index, Some(desktop)) => {
                settings.overlay.items[index].desktop = desktop.max(1.0) as u32;
                self.dirty = true;
            }
            Msg::OverlayItemDeleted(index) => {
                self.editing = None;
                settings.overlay.items.remove(index);
                self.dirty = true;
            }
            Msg::OverlayItemAdded => {
                self.reordering = None;
                settings.overlay.items.push(Item {
                    key: None,
                    action: Action::Minimize,
                    label: Some("New tile".to_owned()),
                    label_on: None,
                    icon: Some(icons::default_for(Action::Minimize).to_owned()),
                    group: None,
                    desktop: 0,
                });
                self.dirty = true;
                self.editing = None;
                let index = settings.overlay.items.len() - 1;
                self.edit(Msg::EditTile(index));
            }

            Msg::IconPickerToggled => {
                if self.icon_picker.open {
                    self.icon_picker.open = false;
                } else {
                    let current = match self.editing {
                        Some(Editing::Tile { index, .. }) => settings.overlay.items[index].icon.as_deref(),
                        Some(Editing::QuickAction { index, .. }) => settings.min_button.items[index].icon.as_deref(),
                        _ => None,
                    };
                    self.icon_picker = IconPickerUi::opening_on(current);
                }
            }
            Msg::IconPicked(icon) => {
                let target = match self.editing {
                    Some(Editing::Tile { index, .. }) => Some(&mut settings.overlay.items[index].icon),
                    Some(Editing::QuickAction { index, .. }) => Some(&mut settings.min_button.items[index].icon),
                    _ => None,
                };
                if let Some(target) = target {
                    *target = Some(icon.to_owned());
                    self.dirty = true;
                }
                self.icon_picker = IconPickerUi::default();
            }

            // WinUI's `Slider`/`NumberBox` report a value on first render,
            // not only on user input — an unchanged value isn't an edit.
            Msg::MinButtonDelayChanged(Some(delay)) => {
                let delay = delay.max(0.0).round() as u32;
                if delay != settings.min_button.hover_delay_ms {
                    settings.min_button.hover_delay_ms = delay;
                    self.dirty = true;
                }
            }
            Msg::MinButtonRightClickChanged(Some(choice)) => {
                // Choice 0 is "(off)" — see `min_button::settings`.
                let action = choice.checked_sub(1).map(|index| min_button::actions()[index]);
                if action != settings.min_button.right_click_action {
                    settings.min_button.right_click_action = action;
                    self.dirty = true;
                }
            }
            Msg::EditQuickAction(index) => {
                self.reordering = None;
                let reopen = !matches!(self.editing, Some(Editing::QuickAction { index: open, .. }) if open == index);
                self.editing = None;
                if reopen {
                    let original = settings.min_button.items[index].clone();
                    self.icon_picker = IconPickerUi::default();
                    self.editing = Some(Editing::QuickAction { index, original });
                }
            }
            Msg::QuickActionActionChanged(index, Some(choice)) => {
                let action = min_button::actions()[choice];
                let item = &mut settings.min_button.items[index];
                item.icon = icons::after_action_change(item.icon.as_deref(), item.action, action);
                item.action = action;
                self.dirty = true;
            }
            Msg::QuickActionLabelChanged(index, text) => {
                settings.min_button.items[index].label = (!text.trim().is_empty()).then_some(text);
                self.dirty = true;
            }
            Msg::QuickActionIconChanged(index, text) => {
                settings.min_button.items[index].icon = (!text.trim().is_empty()).then_some(text);
                self.dirty = true;
            }
            Msg::QuickActionDeleted(index) => {
                self.editing = None;
                settings.min_button.items.remove(index);
                self.dirty = true;
            }
            Msg::QuickActionAdded => {
                self.reordering = None;
                settings.min_button.items.push(QuickAction {
                    action: Action::Minimize,
                    label: Some("New tile".to_owned()),
                    icon: Some(icons::default_for(Action::Minimize).to_owned()),
                });
                self.dirty = true;
                self.editing = None;
                let index = settings.min_button.items.len() - 1;
                self.edit(Msg::EditQuickAction(index));
            }

            Msg::ReorderToggled(list) => {
                // An editor's index would go stale as rows move; reordering
                // and editing are separate modes.
                self.editing = None;
                self.reordering = if self.reordering == Some(list) { None } else { Some(list) };
            }
            Msg::Moved { list, index, up } => {
                let swapped = match list {
                    ListKind::Hotkeys => swap_neighbour(&mut settings.hotkeys, index, up),
                    ListKind::OverlayGroups => swap_neighbour(&mut settings.overlay.groups, index, up),
                    ListKind::OverlayTiles => swap_neighbour(&mut settings.overlay.items, index, up),
                    ListKind::PopupTiles => swap_neighbour(&mut settings.min_button.items, index, up),
                };
                if swapped {
                    self.dirty = true;
                }
            }

            Msg::Save => match settings.save() {
                Ok(()) => {
                    self.dirty = false;
                    self.save_error = None;
                    // What they described is no longer in the file.
                    settings.warnings.clear();
                    // No file watching in y-snap: tell it, so the change
                    // applies now rather than at the next tray "Reload".
                    self.applied = Some(ipc::request_reload());
                }
                Err(error) => self.save_error = Some(error.to_string()),
            },

            // A control reporting "nothing selected"/"empty" leaves the
            // value as it was.
            Msg::HotkeyActionChanged(_, None)
            | Msg::HotkeyTargetChanged(_, None)
            | Msg::HotkeyDesktopChanged(_, None)
            | Msg::GestureButtonChanged(_, None)
            | Msg::OverlayColumnsChanged(None)
            | Msg::OverlayTileWidthChanged(None)
            | Msg::OverlayTileHeightChanged(None)
            | Msg::OverlayCornerRadiusChanged(None)
            | Msg::OverlayBorderWidthChanged(None)
            | Msg::OverlayAlphaChanged(None)
            | Msg::OverlayItemActionChanged(_, None)
            | Msg::OverlayItemGroupChanged(_, None)
            | Msg::OverlayItemDesktopChanged(_, None)
            | Msg::MinButtonDelayChanged(None)
            | Msg::MinButtonRightClickChanged(None)
            | Msg::QuickActionActionChanged(_, None)
            | Msg::OverlayThemeChanged(None) => {}
        }
    }
}

/// Swaps `items[index]` with the item above (`up`) or below it. `false`,
/// and nothing changes, when there's no neighbour that way — a stale click
/// on a button that should already have been disabled.
fn swap_neighbour<T>(items: &mut [T], index: usize, up: bool) -> bool {
    let other = if up { index.checked_sub(1) } else { index.checked_add(1) };
    match other {
        Some(other) if index < items.len() && other < items.len() => {
            items.swap(index, other);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::swap_neighbour;

    #[test]
    fn swap_neighbour_moves_one_step_and_refuses_the_ends() {
        let mut items = vec!['a', 'b', 'c'];
        assert!(swap_neighbour(&mut items, 1, true));
        assert_eq!(items, ['b', 'a', 'c']);
        assert!(swap_neighbour(&mut items, 1, false));
        assert_eq!(items, ['b', 'c', 'a']);
        assert!(!swap_neighbour(&mut items, 0, true));
        assert!(!swap_neighbour(&mut items, 2, false));
        assert!(!swap_neighbour(&mut items, 7, true));
        assert_eq!(items, ['b', 'c', 'a']);
    }
}
