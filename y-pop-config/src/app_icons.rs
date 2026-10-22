//! The icons the screens show for the items: what y-pop would show in its
//! menu, drawn here from real files.
//!
//! **What an item's icon is** follows y-pop (`menu::resolve_icon`): its
//! `icon="path,index"` if it has one; otherwise a folder for a group or a
//! browsed folder, the program's own icon for an app, a stock icon for
//! About / Quit / Settings, and nothing for a separator or a column break.
//! A Store app has no file to take one from yet, so it keeps its symbol.
//!
//! **Loading is never done while drawing.** `ExtractIconExW` and the shell
//! can block on a network path or a sleeping disk. [`ConfigApp::request_icons`]
//! (after every message) lists the icons the tree needs that nobody has asked
//! for, and loads each on the thread pool as PNG bytes; the answer comes
//! back as [`Msg::IconLoaded`]. Until then — and for good if there is no
//! icon — the screen shows the item's symbol, so a row never changes size.
//!
//! One size is loaded, 32 px, drawn at 16 DIPs: sharp up to 200 % scaling.

use std::collections::HashMap;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use y_core::icon::{IconCache, StockIcon};
use y_pop::settings::{Item, ItemKind, Settings};
use y_pop::variables::Lookup;

use windows_reactor::*;

/// What an icon is loaded from. Equal specs share one loaded image.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum IconSpec {
    /// A program, library, icon file or shortcut, and which icon in it.
    File {
        path: String,
        index: Option<i32>,
    },
    /// The shell's folder icon.
    Directory,
    Info,
    Delete,
    Settings,
}

/// Pixels loaded; the screens draw it at [`SHOWN`] DIPs.
const LOADED: i32 = 32;
pub const SHOWN: f64 = 16.0;

/// `"path"` or `"path,index"`, the way y-pop reads `icon=` (after variables).
pub fn split_icon_text(expanded: &str) -> (String, Option<i32>) {
    if let Some((path, index)) = expanded.rsplit_once(',')
        && let Ok(index) = index.trim().parse::<i32>()
    {
        return (path.to_owned(), Some(index));
    }
    (expanded.to_owned(), None)
}

/// The icon `item` shows in y-pop, if it has one.
pub fn spec_for(settings: &Settings, item: &Item, builtins: &mut dyn Lookup) -> Option<IconSpec> {
    if let Some(text) = item.icon.as_deref().filter(|text| !text.trim().is_empty()) {
        let (path, index) = split_icon_text(&settings.expand_text(text, builtins));
        return Some(IconSpec::File { path, index });
    }
    match &item.kind {
        ItemKind::Group => Some(IconSpec::Directory),
        ItemKind::App(app) if app.app_id.is_some() => None,
        ItemKind::App(app) if app.browse => Some(IconSpec::Directory),
        ItemKind::App(app) => {
            let exe = settings.expand_text(app.exe.as_deref()?, builtins);
            let exe = exe.trim().trim_matches('"');
            (!exe.is_empty()).then(|| IconSpec::File {
                path: exe.to_owned(),
                index: None,
            })
        }
        ItemKind::About => Some(IconSpec::Info),
        ItemKind::Quit => Some(IconSpec::Delete),
        ItemKind::Settings => Some(IconSpec::Settings),
        ItemKind::Separator | ItemKind::Break => None,
    }
}

/// Loads `spec` as PNG bytes. Meant for a background thread: it may block.
pub fn load_png(spec: &IconSpec) -> Option<Vec<u8>> {
    // The shell wants COM on the calling thread; a pool thread has none yet.
    let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let png = {
        let mut cache = IconCache::new();
        let icon = match spec {
            IconSpec::File { path, index } => cache.for_file(path, *index),
            IconSpec::Directory => cache.for_directory(),
            IconSpec::Info => cache.for_stock(StockIcon::Info),
            IconSpec::Delete => cache.for_stock(StockIcon::Delete),
            IconSpec::Settings => cache.for_stock(StockIcon::Settings),
        };
        icon.and_then(|icon| icon.to_png(LOADED))
    };
    if entered {
        unsafe { CoUninitialize() };
    }
    png
}

#[derive(Clone)]
enum State {
    Loading,
    Ready(EncodedImage),
    /// Asked for and nothing came: the symbol stays.
    Missing,
}

/// Every icon asked for so far, and how far each got.
#[derive(Default)]
pub struct IconStore {
    states: HashMap<IconSpec, State>,
}

impl IconStore {
    /// Marks `spec` as being loaded. `false` if it already was (or is done),
    /// so the caller starts a load only for a new one.
    pub fn begin(&mut self, spec: &IconSpec) -> bool {
        if self.states.contains_key(spec) {
            return false;
        }
        self.states.insert(spec.clone(), State::Loading);
        true
    }

    pub fn finish(&mut self, spec: IconSpec, png: Option<Vec<u8>>) {
        let state = match png {
            Some(bytes) => State::Ready(EncodedImage::new(bytes)),
            None => State::Missing,
        };
        self.states.insert(spec, state);
    }

    /// The icon as a view — `size` DIPs square — or `None` while it loads
    /// or when there is none.
    pub fn image(&self, spec: Option<&IconSpec>, size: f64) -> Option<View> {
        match self.states.get(spec?)? {
            State::Ready(data) => Some(
                Image::new()
                    .source_data(data.clone())
                    .stretch(Stretch::Uniform)
                    .width(size)
                    .height(size)
                    .into(),
            ),
            State::Loading | State::Missing => None,
        }
    }
}

/// An item's icon at `size` DIPs, or its `symbol` where there is no image
/// (yet), in a box of the same size so the row does not move when it arrives.
pub fn icon_or_symbol(image: Option<View>, symbol: Symbol, size: f64) -> View {
    let inner: View = image.unwrap_or_else(|| SymbolIcon::new().symbol(symbol).into());
    Border::new()
        .width(size)
        .height(size)
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .content(inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_pop::variables::Variable;

    fn load(xml: &str) -> Settings {
        Settings::load_str(xml, &mut Vec::<Variable>::new()).expect("valid")
    }

    fn specs(xml: &str) -> Vec<Option<IconSpec>> {
        let settings = load(xml);
        let mut lookup = vec![Variable {
            name: "win".into(),
            value: r"C:\Windows".into(),
        }];
        settings
            .items()
            .map(|item| spec_for(&settings, item, &mut lookup))
            .collect()
    }

    #[test]
    fn icon_text_splits_into_a_path_and_an_optional_index() {
        assert_eq!(
            split_icon_text(r"C:\a\b.dll,-41"),
            (r"C:\a\b.dll".to_owned(), Some(-41))
        );
        assert_eq!(
            split_icon_text(r"C:\a\b.exe"),
            (r"C:\a\b.exe".to_owned(), None)
        );
        assert_eq!(
            split_icon_text(r"C:\a,b\c.ico"),
            (r"C:\a,b\c.ico".to_owned(), None)
        );
    }

    #[test]
    fn an_item_shows_what_y_pop_shows() {
        let found = specs(
            r#"<y-pop><group name="G">
                 <app name="own" exe="%win%\notepad.exe"/>
                 <app name="chosen" exe="a.exe" icon="%win%\system32\shell32.dll,3"/>
                 <app name="folder" exe="C:\Users" browse="true"/>
                 <app name="store" app-id="Pub.App_x!App"/>
                 <separator/><break/><about/><quit/>
               </group></y-pop>"#,
        );
        assert_eq!(found[0], Some(IconSpec::Directory));
        assert_eq!(
            found[1],
            Some(IconSpec::File {
                path: r"C:\Windows\notepad.exe".into(),
                index: None
            })
        );
        assert_eq!(
            found[2],
            Some(IconSpec::File {
                path: r"C:\Windows\system32\shell32.dll".into(),
                index: Some(3)
            })
        );
        assert_eq!(found[3], Some(IconSpec::Directory));
        assert_eq!(found[4], None);
        assert_eq!(found[5], None);
        assert_eq!(found[6], None);
        assert_eq!(found[7], Some(IconSpec::Info));
        assert_eq!(found[8], Some(IconSpec::Delete));
    }

    #[test]
    fn a_spec_is_loaded_once() {
        let mut store = IconStore::default();
        assert!(store.begin(&IconSpec::Directory));
        assert!(!store.begin(&IconSpec::Directory));
        store.finish(IconSpec::Directory, None);
        assert!(!store.begin(&IconSpec::Directory));
        assert!(store.image(Some(&IconSpec::Directory), 16.0).is_none());
    }

    #[test]
    fn a_real_icon_loads_to_png() {
        let path = format!(
            r"{}\notepad.exe",
            std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())
        );
        let png = load_png(&IconSpec::File { path, index: None }).expect("notepad has an icon");
        assert_eq!(&png[1..4], b"PNG");
        assert!(load_png(&IconSpec::Directory).is_some());
    }
}
