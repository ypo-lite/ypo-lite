//! `%token%` expansion, and the string interner that backs the memory work
//! described in `docs/y-pop-plan.md`.
//!
//! Two things live here because they run over the same data at the same
//! point in the pipeline — once per string, while `y-pop.xml` is being
//! parsed — even though they solve different problems: [`expand`] resolves
//! `%win%`/`%mydocs%`/custom `<var>` tokens once, at load time; [`Pool`]
//! makes sure two nodes that end up with the identical *resulting* string
//! (a shared `%programfiles%\...` value copy-pasted across `app` entries, an
//! `ico` reusing its `exe` path verbatim) share one allocation instead of
//! two.
//!
//! [`expand`] checks two sources, in order: `custom` — the file's own
//! `<var>`s, already fully known and cheap, so always an eager slice — then
//! `builtins`, anything implementing [`Lookup`]. That split is what lets a
//! caller plug in a *lazy* builtins source (`y-pop`'s own, wrapping
//! `y_core::known_folders`, resolves `%win%`/`%mydocs%`/… from Win32 one at
//! a time, only for a token some `%…%` actually names) without this module
//! knowing or caring that Win32 is involved at all — a fixed `Vec<Variable>`
//! implements [`Lookup`] too, which is what every test below uses, so this
//! module itself never makes a system call.
//!
//! `%clipboard%`/`%clipboards%` are deliberately not resolved here: PortAL
//! resolves them lazily, at the moment a node launches, not at load time,
//! since the clipboard's contents at load time are not what the user will
//! want pasted minutes or hours later. [`expand`] treats any unmatched token
//! — clipboard tokens included — as not present and leaves it untouched in
//! the output, so it survives into the parsed tree intact for the launcher
//! to resolve just before use.

use std::collections::HashMap;
use std::sync::Arc;

/// One `%name%` → value substitution.
#[derive(Clone, Debug)]
pub struct Variable {
    pub name: String,
    pub value: String,
}

/// A source [`expand`] can ask for a variable's value by name — see the
/// module doc for why this is a trait rather than another `&[Variable]`.
/// `&mut self` so a lazily-resolved implementation can cache what it
/// computes on the way past.
pub trait Lookup {
    fn get(&mut self, name: &str) -> Option<String>;
}

/// The trivial, eager case: every test in this module, and any caller with
/// nothing Win32 to look up, can hand `expand` a plain `Vec<Variable>` for
/// `builtins` instead of writing a one-off [`Lookup`] implementor.
impl Lookup for Vec<Variable> {
    fn get(&mut self, name: &str) -> Option<String> {
        self.iter()
            .rev()
            .find(|variable| variable.name == name)
            .map(|variable| variable.value.clone())
    }
}

/// Replaces every `%name%` in `text`. `custom` is tried first — last
/// matching entry wins, so a later `<var>` shadows an earlier one of the
/// same name — then `builtins`, for whatever `custom` doesn't define. A `%`
/// matched by neither — including `%clipboard%`/`%clipboards%`, see the
/// module doc — is left exactly as written: a stray `%` in a path is more
/// useful shown back to the user intact than silently eaten.
pub fn expand(text: &str, custom: &[Variable], builtins: &mut dyn Lookup) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }

    let mut expanded = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];

        let Some(end) = after.find('%') else {
            // No closing '%': the rest of the text, verbatim.
            expanded.push_str(&rest[start..]);
            return expanded;
        };
        let name = &after[..end];

        let value = custom
            .iter()
            .rev()
            .find(|variable| variable.name == name)
            .map(|variable| variable.value.clone())
            .or_else(|| builtins.get(name));

        match value {
            Some(value) => expanded.push_str(&value),
            None => expanded.push_str(&rest[start..=start + end + 1]),
        }
        rest = &after[end + 1..];
    }

    expanded.push_str(rest);
    expanded
}

/// Interns strings so that two identical values share one allocation — the
/// "exact-string interning" layer of the memory plan in
/// `docs/y-pop-plan.md`; the deeper prefix-sharing layer is deliberately not
/// here, kept for if profiling a real config ever shows this isn't enough.
///
/// Scoped to one config load: built fresh in [`crate::config::load_str`] and
/// dropped once the resulting [`crate::model::ConfigTree`] holds its own
/// `Arc<str>` clones, so the lookup table itself does not linger as part of
/// the tree's resident memory.
#[derive(Default)]
pub struct Pool {
    interned: HashMap<Box<str>, Arc<str>>,
}

impl Pool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, value: String) -> Arc<str> {
        if let Some(existing) = self.interned.get(value.as_str()) {
            return Arc::clone(existing);
        }
        let arc: Arc<str> = Arc::from(value.into_boxed_str());
        self.interned
            .insert(Box::from(arc.as_ref()), Arc::clone(&arc));
        arc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins() -> Vec<Variable> {
        vec![
            Variable {
                name: "win".into(),
                value: r"C:\Windows".into(),
            },
            Variable {
                name: "docs".into(),
                value: r"%win%\Documents".into(), // deliberately not pre-expanded
            },
        ]
    }

    #[test]
    fn substitutes_known_tokens() {
        assert_eq!(
            expand(r"%win%\explorer.exe", &[], &mut builtins()),
            r"C:\Windows\explorer.exe"
        );
    }

    #[test]
    fn leaves_unmatched_tokens_untouched() {
        assert_eq!(expand("%clipboard%", &[], &mut builtins()), "%clipboard%");
        assert_eq!(
            expand(r"%nope%\file.txt", &[], &mut builtins()),
            r"%nope%\file.txt"
        );
    }

    #[test]
    fn leaves_an_unmatched_percent_untouched() {
        assert_eq!(expand("100% done", &[], &mut builtins()), "100% done");
    }

    #[test]
    fn does_not_recursively_expand() {
        // %docs% expands to a string that itself contains "%win%" — that is
        // not expanded again. Simpler and safer than recursive expansion,
        // which would need cycle detection; the config schema is not asking
        // for %docs% to reference %win% today, so it is not built for it.
        assert_eq!(expand("%docs%", &[], &mut builtins()), r"%win%\Documents");
    }

    #[test]
    fn a_custom_variable_shadows_a_builtin_of_the_same_name() {
        let custom = [Variable {
            name: "win".into(),
            value: "D:\\CustomWindows".into(),
        }];
        assert_eq!(
            expand("%win%", &custom, &mut builtins()),
            "D:\\CustomWindows"
        );
    }

    #[test]
    fn pool_shares_identical_strings() {
        let mut pool = Pool::new();
        let a = pool.intern(r"C:\Program Files\x.exe".to_owned());
        let b = pool.intern(r"C:\Program Files\x.exe".to_owned());
        assert!(Arc::ptr_eq(&a, &b));

        let c = pool.intern(r"C:\Program Files\y.exe".to_owned());
        assert!(!Arc::ptr_eq(&a, &c));
    }
}
