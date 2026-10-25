//! The icons the screens show for the items: what y-pop would show in its
//! menu, drawn here from real files.
//!
//! **What an item's icon is** follows y-pop (`menu::resolve_icon`): its
//! `icon="path,index"` if it has one; otherwise a folder for a group or a
//! browsed folder, the program's own icon for an app, a stock icon for
//! About / Quit / Settings, and nothing for a separator or a column break.
//! A Store app has no file to take one from, so it keeps its symbol unless its
//! `icon` is `@app`, which asks the shell for the app's own logo.
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
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// The logo of a Store app, by its app id.
    Store(String),
    /// The shell's folder icon.
    Directory,
    Info,
    Delete,
    Settings,
    /// A built-in icon (`icon="@name"`), by its catalogue name.
    Internal(&'static str),
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
        if let Some(internal) = y_pop::internal_icons::find(text) {
            return Some(IconSpec::Internal(internal.name));
        }
        // `@app` is a Store app's own logo; anywhere else it is as if no icon
        // had been given, as y-pop reads it.
        if !text.trim().eq_ignore_ascii_case(y_pop::model::APP_LOGO) {
            let (path, index) = split_icon_text(&settings.expand_text(text, builtins));
            return Some(IconSpec::File { path, index });
        }
        if let ItemKind::App(app) = &item.kind
            && let Some(app_id) = &app.app_id
        {
            return Some(IconSpec::Store(app_id.clone()));
        }
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
    // The shell's stock and folder icons fail when asked for from several
    // threads at once (seen live: About and Quit lost their icons, as they load
    // together at start). They are quick, so they simply take turns.
    static SHELL_ICON_LOADS: Mutex<()> = Mutex::new(());
    let _turn = matches!(
        spec,
        IconSpec::Directory
            | IconSpec::Info
            | IconSpec::Delete
            | IconSpec::Settings
            | IconSpec::Internal(_)
    )
    .then(|| {
        SHELL_ICON_LOADS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    });
    // The shell wants COM on the calling thread; a pool thread has none yet.
    let entered = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let png = {
        let mut cache = IconCache::new();
        let icon = match spec {
            IconSpec::File { path, index } => cache.for_file(path, *index),
            IconSpec::Store(app_id) => cache.for_store_app(app_id),
            IconSpec::Directory => cache.for_directory(),
            IconSpec::Info => cache.for_stock(StockIcon::Info),
            IconSpec::Delete => cache.for_stock(StockIcon::Delete),
            IconSpec::Settings => cache.for_stock(StockIcon::Settings),
            IconSpec::Internal(name) => y_pop::internal_icons::find(&format!("@{name}"))
                .and_then(|internal| internal.load(&mut cache)),
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

/// How long after a failed Store logo load the next try is made: the shell
/// often fails these only because it was busy resolving many packages.
const STORE_RETRY_DELAYS: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

/// Store logos loading at once. The shell answers "pending" to a screenful
/// requested together, so the rest wait their turn (without holding a pool
/// thread: they are simply offered again after the next answer).
const STORE_LOADS_AT_ONCE: usize = 3;

/// A Store load that has not answered after this long is given up on and
/// started again. The framework can drop a finished task's answer (seen live),
/// and a load that is never answered would hold one of the slots above for
/// good, until none was left to load anything.
const STORE_LOAD_TIMEOUT: Duration = Duration::from_secs(3);

/// Every icon asked for so far, and how far each got.
#[derive(Default)]
pub struct IconStore {
    states: HashMap<IconSpec, State>,
    /// Later tries already made, per Store logo.
    retried: HashMap<IconSpec, usize>,
    /// When each failed Store logo is asked for again.
    retry_at: HashMap<IconSpec, Instant>,
    /// Since when each Store logo has been loading.
    loading_since: HashMap<IconSpec, Instant>,
    /// A wake-up for the earliest deadline is on its way.
    ticking: bool,
}

impl IconStore {
    /// Marks `spec` as being loaded. `false` if it already was (or is done),
    /// so the caller starts a load only for a new one — or if it is a Store
    /// logo and enough of those are loading already: it is asked again later.
    pub fn begin(&mut self, spec: &IconSpec) -> bool {
        if self.states.contains_key(spec) {
            return false;
        }
        if matches!(spec, IconSpec::Store(_)) {
            self.expire_stalled_loads();
            if self.loading_since.len() >= STORE_LOADS_AT_ONCE {
                return false;
            }
            self.loading_since.insert(spec.clone(), Instant::now());
        }
        self.states.insert(spec.clone(), State::Loading);
        true
    }

    pub fn finish(&mut self, spec: IconSpec, png: Option<Vec<u8>>) {
        let store = matches!(spec, IconSpec::Store(_));
        if store {
            self.loading_since.remove(&spec);
        }
        let state = match png {
            Some(bytes) => State::Ready(EncodedImage::new(bytes)),
            None => State::Missing,
        };
        if store && matches!(state, State::Missing) {
            let tries = self.retried.entry(spec.clone()).or_default();
            if let Some(delay) = STORE_RETRY_DELAYS.get(*tries) {
                self.retry_at.insert(spec.clone(), Instant::now() + *delay);
            }
            *tries += 1;
        }
        self.states.insert(spec, state);
    }

    /// Forgets the Store loads that never answered, so they start again.
    fn expire_stalled_loads(&mut self) {
        let now = Instant::now();
        let stalled: Vec<IconSpec> = self
            .loading_since
            .iter()
            .filter(|(_, since)| now.duration_since(**since) >= STORE_LOAD_TIMEOUT)
            .map(|(spec, _)| spec.clone())
            .collect();
        for spec in stalled {
            self.loading_since.remove(&spec);
            if matches!(self.states.get(&spec), Some(State::Loading)) {
                self.states.remove(&spec);
            }
        }
    }

    /// How long until the next deadline — a failed Store logo due again, or a
    /// load to give up on — if a wake-up is needed (none is already on its way).
    pub fn next_wake(&mut self) -> Option<Duration> {
        if self.ticking {
            return None;
        }
        let retries = self.retry_at.values().copied();
        let timeouts = self
            .loading_since
            .values()
            .map(|since| *since + STORE_LOAD_TIMEOUT);
        let earliest = retries.chain(timeouts).min()?;
        self.ticking = true;
        Some(earliest.saturating_duration_since(Instant::now()))
    }

    /// The wake-up came: forgets the `Missing` answers that are due and the
    /// loads that never answered, so the next request loads them again.
    pub fn retry_due(&mut self) {
        self.ticking = false;
        self.expire_stalled_loads();
        let now = Instant::now();
        let due: Vec<IconSpec> = self
            .retry_at
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(spec, _)| spec.clone())
            .collect();
        for spec in due {
            self.retry_at.remove(&spec);
            if matches!(self.states.get(&spec), Some(State::Missing)) {
                self.states.remove(&spec);
            }
        }
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
    fn the_app_logo_value_is_a_store_apps_logo_and_nothing_elsewhere() {
        let found = specs(
            r#"<y-pop><group name="G">
                 <app name="store" app-id="Pub.App_x!App" icon="@app"/>
                 <app name="program" exe="a.exe" icon="@APP"/>
                 <group name="sub" icon="@app"/>
               </group></y-pop>"#,
        );
        assert_eq!(found[1], Some(IconSpec::Store("Pub.App_x!App".into())));
        // As y-pop reads it: no icon given, so the default for the kind.
        assert_eq!(
            found[2],
            Some(IconSpec::File {
                path: "a.exe".into(),
                index: None
            })
        );
        assert_eq!(found[3], Some(IconSpec::Directory));
    }

    #[test]
    fn a_built_in_icon_name_is_its_own_spec_on_any_item() {
        let found = specs(
            r#"<y-pop><group name="G" icon="@Folder">
                 <app name="a" exe="a.exe" icon="@about"/>
                 <app name="b" exe="b.exe" icon="@nonsense"/>
               </group></y-pop>"#,
        );
        assert_eq!(found[0], Some(IconSpec::Internal("folder")));
        assert_eq!(found[1], Some(IconSpec::Internal("about")));
        // Not in the catalogue: a path like any other.
        assert!(matches!(&found[2], Some(IconSpec::File { path, .. }) if path == "@nonsense"));
    }

    #[test]
    fn every_built_in_icon_loads() {
        for internal in y_pop::internal_icons::ALL {
            assert!(
                load_png(&IconSpec::Internal(internal.name)).is_some(),
                "{}",
                internal.name
            );
        }
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
    fn a_missing_store_logo_is_asked_for_again_when_due_and_only_a_few_times() {
        let spec = IconSpec::Store("Pub.App_x!App".into());
        let mut store = IconStore::default();
        for round in 0..=STORE_RETRY_DELAYS.len() {
            assert!(store.begin(&spec), "round {round}");
            assert!(!store.begin(&spec));
            store.finish(spec.clone(), None);
            let scheduled = store.retry_at.contains_key(&spec);
            assert_eq!(scheduled, round < STORE_RETRY_DELAYS.len(), "round {round}");
            if !scheduled {
                break;
            }
            // Not due yet: still missing, but a wake-up is wanted (once).
            assert!(store.next_wake().is_some());
            assert!(store.next_wake().is_none());
            store.retry_due();
            assert!(!store.begin(&spec));
            // Due now.
            store.retry_at.insert(spec.clone(), Instant::now());
            assert!(store.next_wake().is_some());
            store.retry_due();
        }
        // Out of retries: it stays missing for good.
        assert!(!store.begin(&spec));
        assert!(store.next_wake().is_none());
    }

    #[test]
    fn only_a_few_store_logos_load_at_once() {
        let mut store = IconStore::default();
        let specs: Vec<_> = (0..5).map(|n| IconSpec::Store(format!("A{n}"))).collect();
        let started = specs.iter().filter(|spec| store.begin(spec)).count();
        assert_eq!(started, STORE_LOADS_AT_ONCE);
        store.finish(specs[0].clone(), Some(Vec::new()));
        assert!(store.begin(&specs[3]));
        assert!(store.begin(&IconSpec::Directory));
    }

    /// Diagnostic, not a check: which installed Store apps have no logo even
    /// when loaded one at a time. Run with `-- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn which_store_apps_have_no_logo() {
        let apps = y_core::apps_folder::store_apps();
        let mut failed = 0;
        for app in &apps {
            let started = std::time::Instant::now();
            let png = load_png(&IconSpec::Store(app.app_id.clone()));
            if png.is_none() {
                failed += 1;
                println!("NO LOGO {:?} ({:?})", app.app_id, started.elapsed());
            }
        }
        println!("{failed} of {} without a logo", apps.len());
    }

    #[test]
    fn stock_and_folder_icons_load_from_several_threads_at_once() {
        let threads: Vec<_> = [
            IconSpec::Directory,
            IconSpec::Info,
            IconSpec::Delete,
            IconSpec::Settings,
        ]
        .into_iter()
        .map(|spec| std::thread::spawn(move || (load_png(&spec).is_some(), spec)))
        .collect();
        for thread in threads {
            let (loaded, spec) = thread.join().unwrap();
            assert!(loaded, "{spec:?}");
        }
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
        assert!(load_png(&IconSpec::Info).is_some(), "info");
        assert!(load_png(&IconSpec::Delete).is_some(), "delete");
        assert!(load_png(&IconSpec::Settings).is_some(), "settings");
    }
}
