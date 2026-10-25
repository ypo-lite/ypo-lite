//! The icon picker: choose an item's `icon=` by sight, from the icons inside
//! a program, library or icon file.
//!
//! Opens as a modal (`ui_kit::editor::modal`, like the hotkey picker) from
//! the "Choose icon…" button under the Icon field. Top to bottom: the icon
//! now in use at 32 px with the value that will be written; "From target"
//! (clear the attribute); the sources — the item's own program, four system
//! files, "Other file…" — and a page of the icons in the chosen source.
//!
//! Like the hotkey picker, a choice **commits as it is made** (so the tree
//! behind it follows) and Cancel puts the old value back.
//!
//! The value written keeps variables where it can (`%system%\shell32.dll,3`,
//! not `C:\Windows\System32\…`) so a file moved to another PC still points
//! somewhere sensible. The icons are loaded like the tree's: in the
//! background, one `IconSpec::File` each, a symbol meanwhile.
//!
//! Grids of 300+ icons (`shell32.dll`) are shown a page at a time: a page is
//! [`PAGE`] icons, so the window never holds more than that many images.

use y_pop::settings::{Item, ItemId, ItemKind, Settings};
use y_pop::variables::Lookup;

use crate::app_icons::{IconSpec, IconStore, icon_or_symbol, split_icon_text};
use crate::menu_edit::{TextField, set_text};
use crate::ui::editor::modal;
use crate::ui::prelude::*;
use crate::{ConfigApp, Msg, tree};
use windows_reactor::*;

/// Icons per page of the grid.
const PAGE: usize = 96;

/// Size of an icon in the grid and in the readout, in DIPs.
const CELL: f64 = 32.0;

/// System files offered as quick sources: `(button, value as written)`.
const NAMED: [(&str, &str); 4] = [
    ("shell32.dll", r"%system%\shell32.dll"),
    ("imageres.dll", r"%system%\imageres.dll"),
    ("explorer.exe", r"%win%\explorer.exe"),
    ("ddores.dll", r"%system%\ddores.dll"),
];

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// The item's own program (an app's target).
    Own,
    /// One of [`NAMED`], by position.
    Named(usize),
    /// A file chosen with the dialog, as an absolute path.
    Other(String),
}

/// The open picker: which item, what it had, what it is looking at.
#[derive(Default)]
pub struct IconPickerUi {
    pub open: Option<ItemId>,
    /// The item's `icon=` when the picker opened, for Cancel.
    original: Option<String>,
    pub source: Option<Source>,
    /// The source as it is written into `icon=` (before `,index`).
    raw: String,
    /// The source with variables expanded: what the icons are read from.
    path: String,
    count: u32,
    page: usize,
}

impl IconPickerUi {
    fn pages(&self) -> usize {
        (self.count as usize).div_ceil(PAGE).max(1)
    }

    /// The icons of the page now shown, for the background loader.
    pub fn visible(&self) -> Vec<IconSpec> {
        if self.open.is_none() || self.path.is_empty() {
            return Vec::new();
        }
        let first = self.page * PAGE;
        let last = (first + PAGE).min(self.count as usize);
        (first..last)
            .map(|index| IconSpec::File {
                path: self.path.clone(),
                index: Some(index as i32),
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub enum IconMsg {
    Open(ItemId),
    Source(Source),
    /// The file dialog, then that file as the source.
    OtherFile,
    Page(isize),
    /// The icon at this position of the source; `None`: from the target.
    Pick(Option<u32>),
    /// A Store app's own logo (`icon="@app"`).
    AppLogo,
    Closed {
        accepted: bool,
    },
}

impl ConfigApp {
    pub(crate) fn edit_icon(&mut self, message: IconMsg) {
        match message {
            IconMsg::Open(id) => self.open_icon_picker(id),
            IconMsg::Source(source) => self.set_icon_source(source),
            IconMsg::OtherFile => {
                let options = y_core::dialog::OpenOptions {
                    title: Some("Choose a file with icons"),
                    filters: &[
                        ("Icon files", "*.exe;*.dll;*.ico;*.lnk;*.icl"),
                        ("All files", "*.*"),
                    ],
                    ..Default::default()
                };
                let owner =
                    unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
                if let Ok(picked) = y_core::dialog::open_files(Some(owner), &options)
                    && let Some(path) = picked.into_iter().next()
                {
                    self.set_icon_source(Source::Other(path.to_string_lossy().into_owned()));
                }
            }
            IconMsg::Page(by) => {
                let last = self.icon_picker.pages() - 1;
                self.icon_picker.page = self.icon_picker.page.saturating_add_signed(by).min(last);
            }
            IconMsg::Pick(index) => {
                let text = match index {
                    Some(index) => format!("{},{index}", self.icon_picker.raw),
                    None => String::new(),
                };
                self.set_icon_text(text);
            }
            IconMsg::AppLogo => self.set_icon_text(y_pop::model::APP_LOGO.to_owned()),
            IconMsg::Closed { accepted } => {
                if !accepted {
                    let original = self.icon_picker.original.clone().unwrap_or_default();
                    self.set_icon_text(original);
                }
                self.icon_picker = IconPickerUi::default();
            }
        }
    }

    fn set_icon_text(&mut self, text: String) {
        let Some(id) = self.icon_picker.open else {
            return;
        };
        let Ok(settings) = &mut self.settings else {
            return;
        };
        if let Some(item) = tree::item_mut(settings, id)
            && set_text(item, TextField::Icon, text)
        {
            self.dirty = true;
        }
    }

    fn open_icon_picker(&mut self, id: ItemId) {
        let Ok(settings) = &self.settings else { return };
        let Some(item) = tree::item(settings, id) else {
            return;
        };
        let original = item.icon.clone().filter(|icon| !icon.is_empty());

        // Open on the source the icon already comes from, else on the item's
        // own program, else on shell32.
        let current = original
            .as_deref()
            .map(|icon| split_icon_text(icon).0.to_ascii_lowercase());
        let source = NAMED
            .iter()
            .position(|(_, raw)| current.as_deref() == Some(&raw.to_ascii_lowercase()))
            .map(Source::Named)
            .or_else(|| {
                current
                    .clone()
                    .map(|path| Source::Other(split_icon_text(&path).0))
            })
            .or_else(|| own_program(item).map(|_| Source::Own))
            .unwrap_or(Source::Named(0));

        self.icon_picker = IconPickerUi {
            open: Some(id),
            original,
            ..IconPickerUi::default()
        };
        self.set_icon_source(source);
    }

    fn set_icon_source(&mut self, source: Source) {
        let Some(id) = self.icon_picker.open else {
            return;
        };
        let Ok(settings) = &self.settings else { return };
        let Some(item) = tree::item(settings, id) else {
            return;
        };
        let Some(raw) = source_raw(item, &source) else {
            return;
        };

        let path = settings.expand_text(&raw, &mut *self.builtins.borrow_mut());
        let path = path.trim().trim_matches('"').to_owned();
        let count = y_core::icon::count_in_file(&path);
        self.icon_picker = IconPickerUi {
            source: Some(source),
            raw,
            path,
            count,
            page: 0,
            ..std::mem::take(&mut self.icon_picker)
        };
    }
}

/// The item's own program, as written, if it is an app that has one.
fn own_program(item: &Item) -> Option<String> {
    match &item.kind {
        ItemKind::App(app) if app.app_id.is_none() && !app.browse => {
            app.exe.clone().filter(|exe| !exe.trim().is_empty())
        }
        _ => None,
    }
}

/// How `source` is written into `icon=` for `item`.
fn source_raw(item: &Item, source: &Source) -> Option<String> {
    match source {
        Source::Own => own_program(item),
        Source::Named(index) => NAMED.get(*index).map(|(_, raw)| (*raw).to_owned()),
        Source::Other(path) => Some(path.clone()),
    }
}

/// The picker over the whole window, while one is open.
pub fn icon_modal(
    settings: &Settings,
    picker: &IconPickerUi,
    builtins: &mut dyn Lookup,
    icons: &IconStore,
    window: Viewport,
    context: &ViewContext<ConfigApp>,
) -> Option<View> {
    let id = picker.open?;
    let item = tree::item(settings, id)?;
    let send = |message: IconMsg| context.callback(move |()| Msg::Icon(message.clone()));

    // What is in use now, and what will be written.
    let used = crate::app_icons::spec_for(settings, item, builtins);
    let current = item.icon.clone().filter(|icon| !icon.is_empty());
    let store_app = matches!(&item.kind, ItemKind::App(app) if app.app_id.is_some());
    let logo_chosen = current
        .as_deref()
        .is_some_and(|icon| icon.trim().eq_ignore_ascii_case(y_pop::model::APP_LOGO));
    let now: View = Stack::horizontal(space::M)
        .child(
            "icon",
            icon_or_symbol(icons.image(used.as_ref(), CELL), Symbol::Placeholder, CELL),
        )
        .child(
            "value",
            Stack::vertical(space::XXS)
                .child(
                    "title",
                    text::strong(if current.is_some() {
                        "Chosen icon"
                    } else {
                        "From target"
                    }),
                )
                .child(
                    "text",
                    text::caption(if logo_chosen {
                        "the app's own logo".to_owned()
                    } else {
                        current.clone().unwrap_or_else(|| default_words(item))
                    }),
                ),
        )
        .child(
            "clear",
            button(
                if store_app { "Generic" } else { "From target" },
                current.is_none(),
                current.is_some(),
                context.callback(|()| Msg::Icon(IconMsg::Pick(None))),
            ),
        )
        .maybe(
            "logo",
            store_app.then(|| {
                button(
                    "App logo",
                    logo_chosen,
                    !logo_chosen,
                    context.callback(|()| Msg::Icon(IconMsg::AppLogo)),
                )
            }),
        )
        .into();

    let selected = picker.source.as_ref();
    // Which cell is the item's current icon: its `icon=`, expanded, when it
    // names this source.
    let chosen_index = current.as_deref().and_then(|icon| {
        let (path, index) = split_icon_text(&settings.expand_text(icon, builtins));
        path.trim()
            .trim_matches('"')
            .eq_ignore_ascii_case(&picker.path)
            .then_some(index)
            .flatten()
    });

    let source_button =
        |key: String, label: &str, source: Source, enabled: bool| -> (String, View) {
            let chosen = selected == Some(&source);
            (
                key,
                button(label, chosen, enabled, send(IconMsg::Source(source))),
            )
        };
    let mut sources: Vec<(String, View)> = vec![source_button(
        "own".into(),
        "Its program",
        Source::Own,
        own_program(item).is_some(),
    )];
    for (index, (label, _)) in NAMED.iter().enumerate() {
        sources.push(source_button(
            format!("named{index}"),
            label,
            Source::Named(index),
            true,
        ));
    }
    let other_chosen = matches!(selected, Some(Source::Other(_)));
    sources.push((
        "other".into(),
        button("Other file…", other_chosen, true, send(IconMsg::OtherFile)),
    ));

    let body_width =
        Viewport::new(window.width.min(crate::ui::editor::MODAL_MAX_WIDTH) - 2.0 * space::XL)
            .inset(CARD_INSET);
    let source_row = AutoGrid::new(110.0)
        .max_columns(6)
        .gap(space::XS, space::XS)
        .items(sources)
        .build(body_width);

    Some(modal(
        window,
        format!("Icon for {}", crate::roots::label(item)),
        |viewport| {
            Stack::vertical(space::L)
                .child("now", now)
                .child("sources", source_row)
                .child("grid", grid(picker, chosen_index, icons, viewport, context))
                .into()
        },
        true,
        context.callback(|()| Msg::Icon(IconMsg::Closed { accepted: false })),
        context.callback(|()| Msg::Icon(IconMsg::Closed { accepted: true })),
    ))
}

/// Words for "no `icon=`": what the item shows instead.
fn default_words(item: &Item) -> String {
    match &item.kind {
        ItemKind::Group => "the folder icon".to_owned(),
        ItemKind::App(app) if app.browse => "the folder icon".to_owned(),
        ItemKind::App(app) if app.app_id.is_some() => "the generic icon".to_owned(),
        ItemKind::App(_) => "the program's own icon".to_owned(),
        _ => "the default icon".to_owned(),
    }
}

fn grid(
    picker: &IconPickerUi,
    chosen_index: Option<i32>,
    icons: &IconStore,
    viewport: Viewport,
    context: &ViewContext<ConfigApp>,
) -> View {
    if picker.count == 0 {
        return text::caption("No icons in this file — or it could not be read.").into();
    }

    let first = picker.page * PAGE;
    let last = (first + PAGE).min(picker.count as usize);

    let cells = (first..last).map(|index| {
        let spec = IconSpec::File {
            path: picker.path.clone(),
            index: Some(index as i32),
        };
        let face = icon_or_symbol(icons.image(Some(&spec), CELL), Symbol::Placeholder, CELL);
        let button = Button::new()
            .style(if chosen_index == Some(index as i32) {
                ButtonStyle::Accent
            } else {
                ButtonStyle::Default
            })
            .min_width(0.0)
            .fill()
            .on_click(context.callback(move |()| Msg::Icon(IconMsg::Pick(Some(index as u32)))))
            .content(face);
        let button: View = button.tooltip(format!("Icon {index}"));
        (index, button)
    });
    let grid = AutoGrid::new(CELL + 20.0)
        .max_columns(16)
        .gap(space::XS, space::XS)
        .items(cells)
        .build(viewport);

    let pages = picker.pages();
    let mut column = Stack::vertical(space::M).child(
        "caption",
        text::caption(format!(
            "{} · {} icon{}",
            picker.path,
            picker.count,
            if picker.count == 1 { "" } else { "s" }
        )),
    );
    column = column.child(format!("page{}", picker.page), grid);
    if pages > 1 {
        let pager = Stack::horizontal(space::S)
            .child(
                "prev",
                button(
                    "Previous",
                    false,
                    picker.page > 0,
                    context.callback(|()| Msg::Icon(IconMsg::Page(-1))),
                ),
            )
            .child(
                "where",
                text::caption(format!("Page {} of {pages}", picker.page + 1))
                    .vertical_alignment(VerticalAlignment::Center),
            )
            .child(
                "next",
                button(
                    "Next",
                    false,
                    picker.page + 1 < pages,
                    context.callback(|()| Msg::Icon(IconMsg::Page(1))),
                ),
            );
        column = column.child("pager", pager);
    }
    column.into()
}
