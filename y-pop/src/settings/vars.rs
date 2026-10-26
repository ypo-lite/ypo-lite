//! What `<variables>` means to the rest of the file: who uses a variable,
//! what it resolves to, and renaming one without breaking its users.
//!
//! The rules are the reader's ([`crate::variables::expand`]): `%name%` is a
//! reference; a value sees the builtins and only the variables **above** it;
//! when two variables share a name the later one wins for whatever comes
//! after it. A reference to a name nothing defines is left as written, so it
//! is not a use of anything here. Menu items see every variable, resolved to
//! the last definition.

use super::{
    AppFields, Entry, Item, ItemId, ItemKind, SearchEntry, SearchSettings, Settings, Var, VarEntry,
};
use crate::variables::{self, Lookup, Variable};

/// Where a variable is referenced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UsedIn {
    /// An attribute of a menu item (`"exe"`, `"name"`, …).
    Item { id: ItemId, field: &'static str },
    /// The value of the variable at this index of [`Settings::variables`].
    Variable { index: usize },
}

/// Why a variable can't be given a name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameError {
    Empty,
    /// A `%` or surrounding blanks: it couldn't be written as `%name%`.
    Invalid,
    /// Another variable already has it, which would change what the
    /// references to either one mean.
    Taken,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "A variable needs a name.",
            Self::Invalid => "A name can't contain % or start or end with a space.",
            Self::Taken => "Another variable already has this name.",
        })
    }
}

impl std::error::Error for NameError {}

/// What a `%…%` scan found: text, or a reference (with its `%`s).
enum Piece<'a> {
    Text(&'a str),
    Reference { name: &'a str },
}

/// Splits `text` the way [`variables::expand`] reads it: a `%` pairs with
/// the next `%`; an unpaired one is plain text.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else { break };
        pieces.push(Piece::Text(&rest[..start]));
        pieces.push(Piece::Reference {
            name: &after[..end],
        });
        rest = &after[end + 1..];
    }
    pieces.push(Piece::Text(rest));
    pieces
}

/// The names `text` refers to, in order, repeats included. `%clipboard%`
/// and friends are in there too: whether a name is defined is for the
/// caller to say.
pub fn references(text: &str) -> Vec<&str> {
    pieces(text)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Reference { name } if !name.is_empty() => Some(name),
            _ => None,
        })
        .collect()
}

/// `text` with every `%old%` written `%new%`, or `None` if it had none.
pub fn rename_references(text: &str, old: &str, new: &str) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Reference { name } if name == old => {
                changed = true;
                out.push('%');
                out.push_str(new);
                out.push('%');
            }
            Piece::Reference { name } => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
    }
    changed.then_some(out)
}

/// Every attribute of `item` the reader expands variables in.
fn texts<'a>(item: &'a Item) -> Vec<(&'static str, &'a str)> {
    let mut fields: Vec<(&'static str, &'a Option<String>)> = vec![
        ("name", &item.name),
        ("icon", &item.icon),
        ("tooltip", &item.tooltip),
    ];
    if let ItemKind::App(app) = &item.kind {
        fields.extend([
            ("exe", &app.exe),
            ("app-id", &app.app_id),
            ("param", &app.param),
            ("path", &app.path),
            ("filter", &app.filter),
        ]);
    }
    fields
        .into_iter()
        .filter_map(|(field, value)| value.as_deref().map(|value| (field, value)))
        .collect()
}

fn texts_mut(item: &mut Item) -> Vec<&mut String> {
    let mut out: Vec<&mut String> = Vec::new();
    out.extend(item.name.as_mut());
    out.extend(item.icon.as_mut());
    out.extend(item.tooltip.as_mut());
    if let ItemKind::App(app) = &mut item.kind {
        out.extend(app.exe.as_mut());
        out.extend(app.app_id.as_mut());
        out.extend(app.param.as_mut());
        out.extend(app.path.as_mut());
        out.extend(app.filter.as_mut());
    }
    out
}

fn for_each_item_mut(entries: &mut [Entry], f: &mut impl FnMut(&mut Item)) {
    for entry in entries {
        if let Entry::Item(item) = entry {
            f(item);
            for_each_item_mut(&mut item.children, f);
        }
    }
}

/// Every item under `<search>`: its groups and folders, and what is in them.
fn search_items(search: &SearchSettings) -> Vec<&Item> {
    fn walk<'a>(entries: &'a [Entry], out: &mut Vec<&'a Item>) {
        for entry in entries {
            if let Entry::Item(item) = entry {
                out.push(item);
                walk(&item.children, out);
            }
        }
    }
    let mut out = Vec::new();
    for entry in &search.children {
        if let SearchEntry::Item(item) = entry {
            out.push(item);
            walk(&item.children, &mut out);
        }
    }
    out
}

/// The search-only groups and folders of `<search>`, and what is in them.
fn for_each_search_item_mut(search: &mut SearchSettings, f: &mut impl FnMut(&mut Item)) {
    for entry in &mut search.children {
        if let SearchEntry::Item(item) = entry {
            f(item);
            for_each_item_mut(&mut item.children, f);
        }
    }
}

impl Settings {
    /// The entry index of the last variable named `name` above `before` —
    /// the definition a reference made at `before` resolves to.
    fn definition_before(&self, name: &str, before: usize) -> Option<usize> {
        self.variables[..before.min(self.variables.len())]
            .iter()
            .rposition(|entry| matches!(entry, VarEntry::Var(var) if var.name == name))
    }

    /// Where the variable at `index` of [`Settings::variables`] is used:
    /// only references that really resolve to *this* definition (a
    /// redefinition further down takes over its later users). Empty for a
    /// comment entry.
    pub fn variable_users(&self, index: usize) -> Vec<UsedIn> {
        let Some(VarEntry::Var(var)) = self.variables.get(index) else {
            return Vec::new();
        };
        let mut users = Vec::new();

        // Menu items see the whole list.
        if self.definition_before(&var.name, self.variables.len()) == Some(index) {
            let search_items = self.search.iter().flat_map(search_items);
            for item in self.items().chain(search_items) {
                for (field, text) in texts(item) {
                    if references(text).contains(&var.name.as_str()) {
                        users.push(UsedIn::Item { id: item.id, field });
                    }
                }
            }
        }

        for (later, entry) in self.variables.iter().enumerate().skip(index + 1) {
            if let VarEntry::Var(other) = entry
                && references(&other.value).contains(&var.name.as_str())
                && self.definition_before(&var.name, later) == Some(index)
            {
                users.push(UsedIn::Variable { index: later });
            }
        }
        users
    }

    /// The value of the variable at `index` with everything above it
    /// expanded — what y-pop would substitute for `%name%`. Empty for a
    /// comment entry.
    pub fn resolved_value(&self, index: usize, builtins: &mut dyn Lookup) -> String {
        let Some(VarEntry::Var(var)) = self.variables.get(index) else {
            return String::new();
        };
        let above: Vec<Variable> = self.variables[..index]
            .iter()
            .filter_map(|entry| match entry {
                VarEntry::Var(var) => Some(Variable {
                    name: var.name.clone(),
                    value: var.value.clone(),
                }),
                VarEntry::Comment(_) => None,
            })
            .collect();
        // The reader expands each value against its predecessors' *already
        // expanded* values; expanding the raw ones in turn gives the same
        // text, one level at a time.
        let mut expanded: Vec<Variable> = Vec::with_capacity(above.len());
        for variable in above {
            let value = variables::expand(&variable.value, &expanded, builtins);
            expanded.push(Variable {
                name: variable.name,
                value,
            });
        }
        variables::expand(&var.value, &expanded, builtins)
    }

    /// `text` as y-pop would read it from an attribute: every `%name%`
    /// replaced by a variable of the file (the last of that name wins) or a
    /// builtin. `%clipboard%` and friends stay as written, as at load.
    pub fn expand_text(&self, text: &str, builtins: &mut dyn Lookup) -> String {
        let mut expanded: Vec<Variable> = Vec::new();
        for entry in &self.variables {
            if let VarEntry::Var(var) = entry {
                let value = variables::expand(&var.value, &expanded, builtins);
                expanded.push(Variable {
                    name: var.name.clone(),
                    value,
                });
            }
        }
        variables::expand(text, &expanded, builtins)
    }

    /// `text` rewritten to start with a `%variable%` when it begins with the
    /// folder one stands for (`C:\Program Files\App\a.exe` →
    /// `%programfiles%\App\a.exe`), or `None` when nothing fits or the text
    /// already starts with a token. The longest folder wins; at equal length
    /// the file's own variable wins over a builtin (the later of two own
    /// variables over the earlier). Matching is
    /// case-insensitive and only at a folder boundary (`C:\Win` does not
    /// claim `C:\Windows`). Only absolute paths count as candidates.
    pub fn tokenize(&self, text: &str, builtins: &mut dyn Lookup) -> Option<String> {
        if text.trim_start().starts_with('%') {
            return None;
        }
        // (name, folder) — the last definition of a name is the one in force.
        let mut candidates: Vec<(String, String)> = Vec::new();
        for index in 0..self.variables.len() {
            if let VarEntry::Var(var) = &self.variables[index] {
                let value = self.resolved_value(index, builtins);
                candidates.retain(|(name, _)| *name != var.name);
                candidates.push((var.name.clone(), value));
            }
        }
        let own = candidates.len();
        for name in crate::builtins::NAMES {
            if candidates.iter().any(|(taken, _)| taken == name) {
                continue; // shadowed by the file's variable
            }
            if let Some(value) = builtins.get(name) {
                candidates.push(((*name).to_owned(), value));
            }
        }

        let mut best: Option<(usize, usize, &str)> = None; // (length, own?, name)
        for (position, (name, value)) in candidates.iter().enumerate() {
            let folder = value.trim_end_matches('\\');
            let absolute = folder.contains(':') || folder.starts_with(r"\\");
            if !absolute || folder.contains('%') {
                continue;
            }
            let Some(head) = text.get(..folder.len()) else {
                continue;
            };
            let boundary =
                text[folder.len()..].is_empty() || text[folder.len()..].starts_with('\\');
            if !head.eq_ignore_ascii_case(folder) || !boundary {
                continue;
            }
            let rank = usize::from(position < own);
            if best.is_none_or(|(length, held, _)| (folder.len(), rank) >= (length, held)) {
                best = Some((folder.len(), rank, name));
            }
        }
        let (length, _, name) = best?;
        Some(format!("%{name}%{}", &text[length..]))
    }

    /// `fields` as y-pop would launch it: every attribute expanded, the shell
    /// choice settled against the file's default. What "Run now" hands to
    /// `y_pop::launch::launch`. A folder is opened, not listed.
    pub fn launch_app(&self, fields: &AppFields, builtins: &mut dyn Lookup) -> crate::model::App {
        use std::sync::Arc;
        let mut expand = |text: &Option<String>| {
            text.as_deref()
                .map(|text| Arc::<str>::from(self.expand_text(text, builtins)))
        };
        let mut events = crate::model::EventMask::default();
        if fields.on_start {
            events.insert(crate::model::EventMask::START);
        }
        if fields.on_exit {
            events.insert(crate::model::EventMask::EXIT);
        }
        crate::model::App {
            exe: expand(&fields.exe),
            app_id: fields.app_id.as_deref().map(Arc::<str>::from),
            param: expand(&fields.param),
            path: expand(&fields.path),
            shell: fields.shell.or(self.globals.shell_execute).unwrap_or(false),
            elevate: fields.elevate,
            events,
            browse: None,
        }
    }

    /// Checks `name` for the variable at `index` (which may keep its own).
    pub fn check_variable_name(&self, index: usize, name: &str) -> Result<(), NameError> {
        if name.trim().is_empty() {
            return Err(NameError::Empty);
        }
        if name.contains('%') || name != name.trim() {
            return Err(NameError::Invalid);
        }
        let taken = self.variables.iter().enumerate().any(|(other, entry)| {
            other != index && matches!(entry, VarEntry::Var(var) if var.name == name)
        });
        if taken { Err(NameError::Taken) } else { Ok(()) }
    }

    /// Renames the variable at `index`, and every reference that resolves to
    /// it, in one edit. Returns how many references were rewritten. Nothing
    /// changes on an error.
    pub fn rename_variable(&mut self, index: usize, new_name: &str) -> Result<usize, NameError> {
        self.check_variable_name(index, new_name)?;
        let Some(VarEntry::Var(var)) = self.variables.get(index) else {
            return Ok(0);
        };
        let old = var.name.clone();
        if old == new_name {
            return Ok(0);
        }

        let mut rewritten = 0;

        if self.definition_before(&old, self.variables.len()) == Some(index) {
            let mut rename = |item: &mut Item| {
                for text in texts_mut(item) {
                    if let Some(changed) = rename_references(text, &old, new_name) {
                        *text = changed;
                        rewritten += 1;
                    }
                }
            };
            for_each_item_mut(&mut self.roots, &mut rename);
            if let Some(search) = &mut self.search {
                for_each_search_item_mut(search, &mut rename);
            }
        }
        for later in index + 1..self.variables.len() {
            if self.definition_before(&old, later) != Some(index) {
                continue;
            }
            if let VarEntry::Var(other) = &mut self.variables[later]
                && let Some(changed) = rename_references(&other.value, &old, new_name)
            {
                other.value = changed;
                rewritten += 1;
            }
        }

        if let VarEntry::Var(var) = &mut self.variables[index] {
            var.name = new_name.to_owned();
        }
        Ok(rewritten)
    }

    /// A name no variable has: `variable`, `variable2`, …
    pub fn unused_variable_name(&self) -> String {
        let taken = |name: &str| {
            self.variables
                .iter()
                .any(|entry| matches!(entry, VarEntry::Var(var) if var.name == name))
        };
        (1..)
            .map(|n| {
                if n == 1 {
                    "variable".to_owned()
                } else {
                    format!("variable{n}")
                }
            })
            .find(|name| !taken(name))
            .unwrap_or_default()
    }

    /// Appends an empty variable with a fresh name and returns its index.
    pub fn add_variable(&mut self) -> usize {
        let name = self.unused_variable_name();
        self.variables.push(VarEntry::Var(Var {
            name,
            value: String::new(),
            blank_before: false,
            extra_attrs: Vec::new(),
        }));
        self.variables.len() - 1
    }

    /// Swaps the entry at `index` with its neighbour above (`up`) or below.
    /// `false`, and nothing changes, when there's none that way.
    pub fn move_variable_entry(&mut self, index: usize, up: bool) -> bool {
        let other = if up {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        match other {
            Some(other) if index < self.variables.len() && other < self.variables.len() => {
                self.variables.swap(index, other);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> Settings {
        Settings::load_str(text, &mut Vec::<Variable>::new()).expect("valid")
    }

    fn builtins() -> Vec<Variable> {
        vec![Variable {
            name: "win".into(),
            value: r"C:\Windows".into(),
        }]
    }

    fn var(settings: &Settings, index: usize) -> &Var {
        match &settings.variables[index] {
            VarEntry::Var(var) => var,
            VarEntry::Comment(_) => panic!("a comment"),
        }
    }

    const FILE: &str = r#"<y-pop>
        <variables>
            <var name="docs" value="%mydocs%"/>
            <var name="notes" value="%docs%\notes"/>
            <var name="again" value="%docs%\again;%docs%"/>
        </variables>
        <group name="%docs% group">
            <app name="Notes" exe="%notes%\n.exe" param="%docs%"/>
            <app name="Unknown" exe="%nothing%\x.exe"/>
        </group>
    </y-pop>"#;

    fn with_vars(vars: &str) -> Settings {
        load(&format!(
            "<y-pop><variables>{vars}</variables><group name=\"g\"/></y-pop>"
        ))
    }

    #[test]
    fn tokenize_uses_a_builtin_at_a_folder_boundary() {
        let settings = with_vars("");
        assert_eq!(
            settings.tokenize(r"c:\windows\system32\a.exe", &mut builtins()),
            Some(r"%win%\system32\a.exe".to_owned())
        );
        assert_eq!(
            settings.tokenize(r"C:\Windows", &mut builtins()),
            Some("%win%".to_owned())
        );
        assert_eq!(
            settings.tokenize(r"C:\WindowsApps\a.exe", &mut builtins()),
            None
        );
        assert_eq!(settings.tokenize(r"D:\x\a.exe", &mut builtins()), None);
    }

    #[test]
    fn tokenize_prefers_the_longest_folder_then_the_files_own_variable() {
        let settings = with_vars(
            r#"<var name="tools" value="%win%\System32"/><var name="sys" value="C:\Windows\System32"/>"#,
        );
        // Same folder twice: the file's own variable (the later one here).
        assert_eq!(
            settings.tokenize(r"C:\Windows\System32\cmd.exe", &mut builtins()),
            Some(r"%sys%\cmd.exe".to_owned())
        );
        // Only the shorter folder applies.
        assert_eq!(
            settings.tokenize(r"C:\Windows\Fonts", &mut builtins()),
            Some(r"%win%\Fonts".to_owned())
        );
    }

    #[test]
    fn tokenize_leaves_tokens_and_relative_values_alone() {
        let settings = with_vars(r#"<var name="short" value="x"/>"#);
        assert_eq!(settings.tokenize(r"%win%\a.exe", &mut builtins()), None);
        assert_eq!(settings.tokenize(r"x\a.exe", &mut builtins()), None);
        assert_eq!(settings.tokenize("", &mut builtins()), None);
    }

    #[test]
    fn references_follow_the_readers_pairing() {
        assert_eq!(references(r"%win%\a\%docs%"), ["win", "docs"]);
        // Two stray percent signs still pair up — exactly as `expand` reads them.
        assert_eq!(references("100% sure, 50%"), [" sure, 50"]);
        assert_eq!(references("only 50%"), Vec::<&str>::new());
        assert_eq!(references("%%"), Vec::<&str>::new());
        assert_eq!(references("no tokens"), Vec::<&str>::new());
        assert_eq!(references("%a%%b%"), ["a", "b"]);
    }

    #[test]
    fn renaming_a_reference_rewrites_only_that_name() {
        assert_eq!(
            rename_references(r"%docs%\x;%docsx%", "docs", "d").as_deref(),
            Some(r"%d%\x;%docsx%")
        );
        assert_eq!(rename_references("100% %docs", "docs", "d"), None);
        assert_eq!(rename_references("none", "docs", "d"), None);
    }

    #[test]
    fn users_are_items_and_later_variables() {
        let settings = load(FILE);
        let users = settings.variable_users(0);
        // `notes`' and `again`' values, the group's name, the app's param.
        assert_eq!(users.len(), 4, "{users:?}");
        assert!(users.contains(&UsedIn::Variable { index: 1 }));
        assert!(users.contains(&UsedIn::Variable { index: 2 }));
        assert!(
            users
                .iter()
                .any(|u| matches!(u, UsedIn::Item { field: "name", .. }))
        );
        assert!(
            users
                .iter()
                .any(|u| matches!(u, UsedIn::Item { field: "param", .. }))
        );

        assert_eq!(settings.variable_users(1).len(), 1);
        assert!(settings.variable_users(2).is_empty());
    }

    #[test]
    fn a_redefinition_takes_over_the_later_users() {
        let settings = load(
            r#"<y-pop><variables>
                <var name="x" value="1"/>
                <var name="y" value="%x%"/>
                <var name="x" value="2"/>
                <var name="z" value="%x%"/>
            </variables><app name="A" exe="%x%"/></y-pop>"#,
        );
        // Only `y` still sees the first `x`; `z` and the item see the second.
        assert_eq!(settings.variable_users(0), [UsedIn::Variable { index: 1 }]);
        let second = settings.variable_users(2);
        assert_eq!(second.len(), 2);
        assert!(second.contains(&UsedIn::Variable { index: 3 }));
    }

    #[test]
    fn a_value_is_resolved_against_the_builtins_and_what_is_above() {
        let settings = load(FILE);
        let mut builtins = vec![Variable {
            name: "mydocs".into(),
            value: r"C:\Users\me\Documents".into(),
        }];
        assert_eq!(
            settings.resolved_value(0, &mut builtins),
            r"C:\Users\me\Documents"
        );
        assert_eq!(
            settings.resolved_value(1, &mut builtins),
            r"C:\Users\me\Documents\notes"
        );
        assert_eq!(
            settings.resolved_value(2, &mut builtins),
            r"C:\Users\me\Documents\again;C:\Users\me\Documents"
        );
    }

    #[test]
    fn a_reference_to_something_below_is_left_as_written() {
        let settings = load(
            r#"<y-pop><variables><var name="a" value="%b%"/><var name="b" value="x"/></variables></y-pop>"#,
        );
        assert_eq!(settings.resolved_value(0, &mut builtins()), "%b%");
        assert!(settings.variable_users(1).is_empty());
    }

    #[test]
    fn renaming_rewrites_every_user_and_nothing_else() {
        let mut settings = load(FILE);
        let rewritten = settings.rename_variable(0, "documents").expect("fine");
        assert_eq!(rewritten, 4);
        assert_eq!(var(&settings, 0).name, "documents");
        assert_eq!(var(&settings, 1).value, r"%documents%\notes");

        let written = settings.to_xml();
        assert!(written.contains(r#"name="%documents% group""#), "{written}");
        assert!(written.contains(r#"param="%documents%""#), "{written}");
        // `%notes%` and the unknown token are not `docs`.
        assert!(written.contains(r#"exe="%notes%\n.exe""#));
        assert!(written.contains(r#"exe="%nothing%\x.exe""#));
        assert!(!written.contains("%docs%"));

        // y-pop reads the renamed file to the same menus.
        let before = crate::config::load_str(
            FILE,
            &mut vec![Variable {
                name: "mydocs".into(),
                value: "D".into(),
            }],
        )
        .expect("ok");
        let after = crate::config::load_str(
            &written,
            &mut vec![Variable {
                name: "mydocs".into(),
                value: "D".into(),
            }],
        )
        .expect("ok");
        let dump = |tree: &crate::model::ConfigTree| -> Vec<String> {
            tree.all_nodes()
                .map(|id| format!("{:?}", tree.node(id)))
                .collect()
        };
        assert_eq!(dump(&before), dump(&after));
    }

    #[test]
    fn a_bad_name_changes_nothing() {
        let mut settings = load(FILE);
        let before = settings.to_xml();
        assert_eq!(settings.rename_variable(0, ""), Err(NameError::Empty));
        assert_eq!(settings.rename_variable(0, "  "), Err(NameError::Empty));
        assert_eq!(settings.rename_variable(0, "a%b"), Err(NameError::Invalid));
        assert_eq!(settings.rename_variable(0, " a"), Err(NameError::Invalid));
        assert_eq!(settings.rename_variable(0, "notes"), Err(NameError::Taken));
        assert_eq!(
            settings.rename_variable(0, "docs"),
            Ok(0),
            "its own name is fine"
        );
        assert_eq!(settings.to_xml(), before);
    }

    #[test]
    fn new_variables_get_fresh_names() {
        let mut settings = load(FILE);
        let first = settings.add_variable();
        let second = settings.add_variable();
        assert_eq!((first, second), (3, 4));
        assert_eq!(var(&settings, 3).name, "variable");
        assert_eq!(var(&settings, 4).name, "variable2");
    }

    #[test]
    fn entries_move_one_step_and_refuse_the_ends() {
        let mut settings = load(FILE);
        assert!(settings.move_variable_entry(1, true));
        assert_eq!(var(&settings, 0).name, "notes");
        assert!(!settings.move_variable_entry(0, true));
        assert!(!settings.move_variable_entry(2, false));
        assert!(!settings.move_variable_entry(9, true));
    }
}
