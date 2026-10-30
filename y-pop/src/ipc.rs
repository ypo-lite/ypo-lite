//! What another process may say to a running y-pop, and how to find out
//! whether one is running at all. In the library so the tray app and
//! `y-pop-config` derive every name from the same constants. Same design as
//! `y_snap::ipc` — see its module doc for why a registered window message is
//! enough, and for the elevated-sender caveat.
//!
//! Two things so far: "reload `y-pop.xml`", sent by `y-pop-config` after a
//! successful save, and a [`Request`] to `y-pop-config` — "show this item",
//! "add this app" — which a second launch of it hands to the open one.

use std::path::PathBuf;

use y_core::single_instance;

use crate::config;
use crate::model::{ConfigTree, NodeId};

/// y-pop's main window class — also its single-instance mutex name
/// (`AppInfo::single_instance` derives one from the other).
pub const CLASS: &str = "YpoLite.YPop";

/// The name y-pop is registered under to start with Windows (`HKCU...Run`).
pub const STARTUP_NAME: &str = "Y-Pop";

/// The settings app's executable, looked for next to `y-pop.exe` by the
/// `<settings/>` menu item.
pub const CONFIG_EXE: &str = "y-pop-config.exe";

/// The settings app's single-instance mutex name.
pub const CONFIG_INSTANCE: &str = "YpoLite.YPopConfig";

/// The settings app's window title — what a second launch finds the open
/// one by (a WinUI window's class name is the framework's, not ours).
pub const CONFIG_TITLE: &str = "y-pop — Configuration";

/// The registered message that asks a running y-pop to reload its
/// configuration. Same id in every process that asks for it.
pub fn reload_message() -> u32 {
    single_instance::broadcast_message(&format!("{CLASS}.Reload"))
}

/// Whether y-pop is running in this logon session.
pub fn is_running() -> bool {
    single_instance::is_running(CLASS)
}

/// Asks a running y-pop to reload `y-pop.xml`. Returns whether there was one
/// to ask — not whether the reload succeeded: y-pop keeps its previous
/// configuration and reports a broken file itself.
pub fn request_reload() -> bool {
    if !is_running() {
        return false;
    }
    single_instance::notify_running_instance(reload_message());
    true
}

/// What the settings app is asked to do, on its command line
/// (`--select <path>`, `--new-app --name ...`) or, when it is already open,
/// through [`post_request`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Request {
    /// Show an item of a tray menu: see [`path_of`] for what the text is.
    Select(String),
    /// Add an app to a tray menu, with these fields filled in.
    NewApp(NewApp),
}

/// The fields `--new-app` can prefill; all raw text, as typed in the editor.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct NewApp {
    pub name: Option<String>,
    /// A program, file or folder.
    pub target: Option<String>,
    pub args: Option<String>,
    /// `path` or `path,index`, as in the `icon` attribute.
    pub icon: Option<String>,
    /// The working folder.
    pub dir: Option<String>,
}

impl Request {
    /// The request a command line makes, or `None` for a plain start. An
    /// unknown flag is ignored, like `--config <file>`, which is not ours.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut args = args.into_iter();
        let mut select = None;
        let mut new_app = None::<NewApp>;
        while let Some(flag) = args.next() {
            let flag = flag.to_ascii_lowercase();
            let mut value = || args.next().filter(|value| !value.is_empty());
            match flag.as_str() {
                "--select" => select = value(),
                "--new-app" => {
                    new_app.get_or_insert_with(NewApp::default);
                }
                "--name" => new_app.get_or_insert_with(NewApp::default).name = value(),
                "--target" => new_app.get_or_insert_with(NewApp::default).target = value(),
                "--args" => new_app.get_or_insert_with(NewApp::default).args = value(),
                "--icon" => new_app.get_or_insert_with(NewApp::default).icon = value(),
                "--dir" => new_app.get_or_insert_with(NewApp::default).dir = value(),
                _ => {}
            }
        }
        match (select, new_app) {
            (_, Some(app)) => Some(Self::NewApp(app)),
            (Some(path), None) => Some(Self::Select(path)),
            (None, None) => None,
        }
    }

    /// The command line that makes this request: what [`Request::from_args`]
    /// reads back.
    pub fn to_args(&self) -> Vec<String> {
        match self {
            Self::Select(path) => vec!["--select".to_owned(), path.clone()],
            Self::NewApp(app) => {
                let mut args = vec!["--new-app".to_owned()];
                for (flag, value) in [
                    ("--name", &app.name),
                    ("--target", &app.target),
                    ("--args", &app.args),
                    ("--icon", &app.icon),
                    ("--dir", &app.dir),
                ] {
                    if let Some(value) = value {
                        args.push(flag.to_owned());
                        args.push(value.clone());
                    }
                }
                args
            }
        }
    }
}

/// Where a request waits for the open settings app to take it: next to
/// `y-pop.xml`, like its preferences.
pub fn request_path() -> PathBuf {
    config::path().with_file_name("y-pop-config.request")
}

/// Leaves `request` for the settings app that is already open. One at a time:
/// a newer request replaces one not yet taken. A registered message can't
/// carry the text, and a file keeps this free of window-procedure plumbing on
/// a window the UI framework owns.
pub fn post_request(request: &Request) -> std::io::Result<()> {
    // One argument per line: no argument holds a line break.
    let text: Vec<String> = request
        .to_args()
        .into_iter()
        .map(|arg| arg.replace(['\r', '\n'], " "))
        .collect();
    std::fs::write(request_path(), text.join("\n"))
}

/// Takes the request [`post_request`] left, if any, removing it.
pub fn take_request() -> Option<Request> {
    let path = request_path();
    let text = std::fs::read_to_string(&path).ok()?;
    std::fs::remove_file(&path).ok()?;
    Request::from_args(text.lines().map(str::to_owned))
}

/// An item as `--select` names it: the position of each node among its
/// siblings, from a tray menu down, joined by `/` (`2/0/5`: the sixth entry
/// of the first group of the third menu). By position, not by name: names
/// repeat, carry `&`, and are written with `%variables%` that only one side
/// has expanded. An item that only the search knows (a search-only group or
/// folder, or what is in one) starts with `s/`: `s/1/0` is the first entry of
/// the second of them.
///
/// Only as good as the file is unchanged since y-pop read it — which is the
/// case when asked from a running y-pop, and a wrong item is not a harm.
pub fn path_of(tree: &ConfigTree, node: NodeId) -> Option<String> {
    fn find(tree: &ConfigTree, siblings: &[NodeId], node: NodeId, path: &mut Vec<usize>) -> bool {
        for (index, &sibling) in siblings.iter().enumerate() {
            path.push(index);
            if sibling == node {
                return true;
            }
            let below: Vec<NodeId> = tree.children(sibling).collect();
            if find(tree, &below, node, path) {
                return true;
            }
            path.pop();
        }
        false
    }

    let join = |path: &[usize]| {
        path.iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("/")
    };

    let mut path = Vec::new();
    if find(tree, &tree.roots, node, &mut path) {
        return Some(join(&path));
    }
    // A search-only group or folder: `s/`, then its place among those.
    let only: Vec<NodeId> = tree.search.as_ref()?.nodes().collect();
    path.clear();
    find(tree, &only, node, &mut path).then(|| format!("s/{}", join(&path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variables::Variable;

    fn args(text: &[&str]) -> Vec<String> {
        text.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn a_command_line_is_a_select_or_a_new_app_or_nothing() {
        assert_eq!(Request::from_args(args(&[])), None);
        assert_eq!(Request::from_args(args(&["--config", "x.xml"])), None);
        assert_eq!(
            Request::from_args(args(&["--select", "1/2"])),
            Some(Request::Select("1/2".into()))
        );
        assert_eq!(
            Request::from_args(args(&[
                "--new-app",
                "--name",
                "Notepad",
                "--target",
                "n.exe"
            ])),
            Some(Request::NewApp(NewApp {
                name: Some("Notepad".into()),
                target: Some("n.exe".into()),
                ..NewApp::default()
            }))
        );
        // A bare flag is an empty app.
        assert!(matches!(
            Request::from_args(args(&["--new-app"])),
            Some(Request::NewApp(app)) if app == NewApp::default()
        ));
    }

    #[test]
    fn a_request_survives_its_command_line() {
        let request = Request::NewApp(NewApp {
            name: Some("A b".into()),
            target: Some(r"C:\x y\a.exe".into()),
            args: Some("-q".into()),
            icon: Some("a.exe,2".into()),
            dir: Some(r"C:\x y".into()),
        });
        assert_eq!(Request::from_args(request.to_args()), Some(request));
        let select = Request::Select("0/1".into());
        assert_eq!(Request::from_args(select.to_args()), Some(select));
    }

    #[test]
    fn a_node_is_named_by_the_positions_down_to_it() {
        let tree = crate::config::load_str(
            r#"<y-pop><group name="A"><app name="a" exe="a.exe"/><group name="B"><app name="b" exe="b.exe"/></group></group><app name="c" exe="c.exe"/></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let a = tree.roots[0];
        let b = tree.children(a).nth(1).expect("group B");
        let inner = tree.children(b).next().expect("app b");
        assert_eq!(path_of(&tree, a).as_deref(), Some("0"));
        assert_eq!(path_of(&tree, inner).as_deref(), Some("0/1/0"));
        assert_eq!(path_of(&tree, tree.roots[1]).as_deref(), Some("1"));
    }

    #[test]
    fn a_search_only_node_is_named_from_s() {
        let tree = crate::config::load_str(
            r#"<y-pop><search><group name="G"><app name="a" exe="a.exe"/></group><app name="F" exe="f" browse="true"/></search><group name="T"/></y-pop>"#,
            &mut Vec::<Variable>::new(),
        )
        .expect("valid");
        let only: Vec<NodeId> = tree.search.as_ref().expect("search").nodes().collect();
        let inner = tree.children(only[0]).next().expect("app a");
        assert_eq!(path_of(&tree, only[0]).as_deref(), Some("s/0"));
        assert_eq!(path_of(&tree, inner).as_deref(), Some("s/0/0"));
        assert_eq!(path_of(&tree, only[1]).as_deref(), Some("s/1"));
        assert_eq!(path_of(&tree, tree.roots[0]).as_deref(), Some("0"));
    }
}
