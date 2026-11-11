//! What a save must give back besides the values: the comments of the file,
//! the attributes y-coustic does not read, and the elements it could not use.
//!
//! The typed settings (`Settings::hotkeys`, `controls`, `mouse`, the section
//! structs) stay exactly what the application reads. Everything else lives in a
//! [`Kept`] beside them, so a program that only runs y-coustic never looks at
//! it, and the editor writes it all back.
//!
//! **Simplification, deliberate:** a comment in a list belongs to the entry
//! that follows it and travels with it when the entry moves; there is no
//! separate "free" comment that stays in place. One blank line before a
//! comment, or between a comment and its entry, is remembered and written
//! back, so the grouping of the file survives. Comments after the last entry
//! of a list are the list's *tail*.
//!
//! The lists of `Kept` are parallel to the typed ones (`Kept::hotkeys[i]` is
//! about `Settings::hotkeys[i]`; `Kept::keys[g].layers[l]` about the layer `l` of
//! the group `g`). Every edit that inserts, removes or moves an
//! entry does the same to its [`Part`] (see `crate::edit`); a part that is
//! missing reads as empty, so a list edited without them loses nothing but the
//! comments of the entries it touched.
//!
//! An element y-coustic cannot use (an unknown action, a bad key, an
//! unknown element, a second copy of a section) becomes a comment holding its
//! own XML, ahead of the entry that follows, and is reported once as a
//! warning. A comment is read as a comment on the next load, so the file
//! settles after the first save and the warning does not come back.

use y_core::xml::{Element, Writer};

/// One comment: its text, and whether an empty line came before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub text: String,
    pub blank_before: bool,
}

impl Note {
    pub fn new(text: impl Into<String>, blank_before: bool) -> Self {
        Self {
            text: text.into(),
            blank_before,
        }
    }
}

/// What goes with one element: the comments above it, whether an empty line
/// separates it from them (or from the sibling above, with no comments), and
/// the attributes y-coustic does not read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Part {
    pub lead: Vec<Note>,
    pub blank_before: bool,
    pub extra: Vec<(String, String)>,
}

impl Part {
    pub fn is_empty(&self) -> bool {
        self.lead.is_empty() && !self.blank_before && self.extra.is_empty()
    }
}

/// A `<keys>` group: its own [`Part`], the [`Part`]s of its `<layer>` children
/// (parallel to the group's layers, when it has any) and the comments after the
/// last of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupPart {
    pub part: Part,
    pub layers: Vec<Part>,
    pub tail: Vec<Note>,
}

impl GroupPart {
    pub fn is_empty(&self) -> bool {
        self.part.is_empty() && self.layers.is_empty() && self.tail.is_empty()
    }
}

/// One section element: its [`Part`], and the comments inside it after its
/// last entry (all of its comments, for a section with no entries).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub part: Part,
    pub tail: Vec<Note>,
}

/// The names of the section elements, in the order they are written.
pub const SECTIONS: &[&str] = &[
    "volume", "watch", "controls", "hotkeys", "mouse", "tray", "mixer", "hud", "colors",
];

/// The sections whose children are entries of their own.
pub const LISTS: &[&str] = &["controls", "hotkeys", "mouse"];

/// Everything a save gives back that is not a value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    /// Comments before the root element, and after it.
    pub prolog: Vec<Note>,
    pub epilog: Vec<Note>,
    /// Comments (and unusable elements) after the last section.
    pub root_tail: Vec<Note>,
    /// By element name, see [`SECTIONS`]. A section the file does not have is
    /// not here.
    pub sections: Vec<(String, Section)>,
    /// Parallel to `Controls::keys`, `Settings::hotkeys` and `Settings::mouse`.
    pub keys: Vec<GroupPart>,
    pub hotkeys: Vec<Part>,
    pub mouse: Vec<Part>,
}

impl Kept {
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, section)| section)
    }

    pub fn section_mut(&mut self, name: &str) -> &mut Section {
        let at = match self.sections.iter().position(|(known, _)| known == name) {
            Some(at) => at,
            None => {
                self.sections.push((name.to_owned(), Section::default()));
                self.sections.len() - 1
            }
        };
        &mut self.sections[at].1
    }

    /// How many comments there are in all: the number a round trip must keep.
    pub fn comment_count(&self) -> usize {
        let parts = |parts: &[Part]| parts.iter().map(|part| part.lead.len()).sum::<usize>();
        self.prolog.len()
            + self.epilog.len()
            + self.root_tail.len()
            + self
                .sections
                .iter()
                .map(|(_, section)| section.part.lead.len() + section.tail.len())
                .sum::<usize>()
            + self
                .keys
                .iter()
                .map(|group| group.part.lead.len() + parts(&group.layers) + group.tail.len())
                .sum::<usize>()
            + parts(&self.hotkeys)
            + parts(&self.mouse)
    }
}

/// The attributes of `element` that are not in `known`.
pub fn unknown_attributes(element: Element<'_>, known: &[&str]) -> Vec<(String, String)> {
    element
        .attributes()
        .filter(|(name, _)| !known.contains(name))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// `element` as the XML it was written as, for a comment: its attributes and
/// its child elements, one line each. Text and comments inside it are not kept.
pub fn element_xml(element: Element<'_>) -> String {
    let mut writer = Writer::new();
    write_element(&mut writer, element);
    writer.finish().trim().to_owned()
}

fn write_element(writer: &mut Writer, element: Element<'_>) {
    writer.open(element.name());
    for (name, value) in element.attributes() {
        writer.attr(name, value);
    }
    for child in element.children() {
        write_element(writer, child);
    }
    writer.close();
}

/// `notes` at the writer's depth.
pub fn write_notes(writer: &mut Writer, notes: &[Note]) {
    for note in notes {
        if note.blank_before {
            writer.blank_line();
        }
        writer.comment(&note.text);
    }
}

/// The start of an entry: its comments, the empty line it carried, its tag.
pub fn open_entry(writer: &mut Writer, name: &str, part: Option<&Part>) {
    if let Some(part) = part {
        write_notes(writer, &part.lead);
        if part.blank_before {
            writer.blank_line();
        }
    }
    writer.open(name);
}

/// The attributes y-coustic does not read, at the end of the start tag.
pub fn write_extra(writer: &mut Writer, part: Option<&Part>) {
    if let Some(part) = part {
        for (name, value) in &part.extra {
            writer.attr(name, value);
        }
    }
}
