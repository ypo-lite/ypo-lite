//! A small XML reader, for configuration files.
//!
//! Not a general XML processor: no namespaces, no DTD, no entity definitions,
//! no validation. What it does read is the subset a settings file is written
//! in — elements, attributes, text, comments, CDATA, character references and
//! the five predefined entities — and it reports where it gave up, so a typo
//! in a configuration file produces a line number rather than a shrug.
//!
//! That subset is worth roughly two hundred lines and no dependency, which is
//! the whole argument for it. Anything beyond a settings file deserves a real
//! parser instead.
//!
//! ```
//! use y_core::xml::Document;
//!
//! let document = Document::parse(
//!     r#"<config>
//!            <!-- one binding -->
//!            <hotkey key="Win+Space" action="overlay" repeat="false"/>
//!        </config>"#,
//! )
//! .expect("valid");
//!
//! let hotkey = document.root().child("hotkey").expect("present");
//! assert_eq!(hotkey.attr("key"), Some("Win+Space"));
//! assert_eq!(hotkey.attr_bool("repeat"), Some(false));
//! assert_eq!(hotkey.attr("missing"), None);
//! ```
//!
//! # Limitations worth knowing
//!
//! * A `<!DOCTYPE …>` declaration is skipped up to its first `>`, so an
//!   internal subset containing one confuses it.
//! * Nesting is capped at [`MAX_DEPTH`]; a file crafted to recurse deeper is
//!   rejected rather than allowed to overflow the stack.
//! * Text is decoded but not normalised: whitespace is returned as written.

use std::fmt;

/// How deep elements may nest before the file is rejected.
///
/// The parser recurses once per level, and a configuration file has no honest
/// reason to go anywhere near this deep.
pub const MAX_DEPTH: usize = 64;

/// Where parsing stopped, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub message: &'static str,
    /// One-based, counted in `\n`.
    pub line: u32,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for Error {}

/// A parsed document.
///
/// Nodes live in one flat arena and are addressed by index, which keeps the
/// tree free of both reference cycles and per-node allocations for links. The
/// root is always the first node.
#[derive(Debug)]
pub struct Document {
    nodes: Vec<NodeData>,
}

#[derive(Debug)]
struct NodeData {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<usize>,
    text: String,
    line: u32,
}

impl Document {
    /// Parses a whole document. The text is expected to be UTF-8 already —
    /// an `encoding` in the XML declaration is not honoured.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Parser::new(text).document()
    }

    /// The root element.
    pub fn root(&self) -> Element<'_> {
        Element {
            document: self,
            index: 0,
        }
    }
}

/// One element of a [`Document`], cheap to copy.
#[derive(Clone, Copy)]
pub struct Element<'a> {
    document: &'a Document,
    index: usize,
}

impl<'a> Element<'a> {
    fn data(&self) -> &'a NodeData {
        &self.document.nodes[self.index]
    }

    pub fn name(&self) -> &'a str {
        &self.data().name
    }

    /// The line the element opened on — for error messages that point at the
    /// offending element rather than at the file.
    pub fn line(&self) -> u32 {
        self.data().line
    }

    /// The element's text content, entity references resolved, with the text
    /// of any child element left out.
    pub fn text(&self) -> &'a str {
        &self.data().text
    }

    /// Every child element, in document order.
    pub fn children(&self) -> impl Iterator<Item = Element<'a>> {
        let document = self.document;
        self.data()
            .children
            .iter()
            .map(move |&index| Element { document, index })
    }

    /// Every child element with this name, in document order.
    pub fn children_named(&self, name: &str) -> impl Iterator<Item = Element<'a>> {
        self.children().filter(move |child| child.name() == name)
    }

    /// The first child element with this name.
    pub fn child(&self, name: &str) -> Option<Element<'a>> {
        self.children().find(|child| child.name() == name)
    }

    /// Every attribute as written, in document order — for reporting the ones
    /// the application does not recognise, which is usually a typo the user
    /// would rather hear about.
    pub fn attributes(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.data()
            .attributes
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// The value of an attribute, or `None` if it is absent.
    pub fn attr(&self, name: &str) -> Option<&'a str> {
        self.data()
            .attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// A decimal number, or `None` if the attribute is absent *or* not a
    /// number. Callers pick the default with `unwrap_or`, and are free to
    /// tell the two cases apart with [`Element::attr`] when it matters.
    pub fn attr_u32(&self, name: &str) -> Option<u32> {
        self.attr(name)?.trim().parse().ok()
    }

    /// `true`/`yes`/`on`/`1` and `false`/`no`/`off`/`0`, in any case.
    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        match self.attr(name)?.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        }
    }
}

/// A recursive-descent parser over the source text.
///
/// It walks bytes rather than characters — every delimiter XML cares about is
/// ASCII, and a multi-byte sequence can never contain an ASCII byte, so the
/// slice boundaries it produces are always on character boundaries.
struct Parser<'a> {
    text: &'a str,
    position: usize,
    line: u32,
    nodes: Vec<NodeData>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            // A byte-order mark is not part of the document.
            text: text.strip_prefix('\u{feff}').unwrap_or(text),
            position: 0,
            line: 1,
            nodes: Vec::new(),
        }
    }

    fn document(mut self) -> Result<Document, Error> {
        self.skip_ignorable()?;

        if self.peek() != Some(b'<') {
            return Err(self.error("expected an element"));
        }
        self.element(0)?;

        self.skip_ignorable()?;
        if self.position != self.text.len() {
            return Err(self.error("trailing content after the root element"));
        }

        Ok(Document { nodes: self.nodes })
    }

    /// Parses one element, starting at its `<`, and returns its index.
    fn element(&mut self, depth: usize) -> Result<usize, Error> {
        if depth > MAX_DEPTH {
            return Err(self.error("elements nested too deeply"));
        }

        let line = self.line;
        self.advance(1); // '<'

        let name = self.name();
        if name.is_empty() {
            return Err(self.error("expected an element name"));
        }

        // Reserved before the children are parsed, so the root always ends up
        // at index 0 and a child is always pushed after its parent.
        let index = self.nodes.len();
        self.nodes.push(NodeData {
            name,
            attributes: Vec::new(),
            children: Vec::new(),
            text: String::new(),
            line,
        });

        loop {
            self.skip_whitespace();

            if self.starts_with("/>") {
                self.advance(2);
                return Ok(index); // empty element, no content to parse
            }
            if self.peek() == Some(b'>') {
                self.advance(1);
                break;
            }

            let attribute = self.attribute()?;
            self.nodes[index].attributes.push(attribute);
        }

        self.content(index, depth)?;
        Ok(index)
    }

    /// Parses everything between an element's `>` and its closing tag.
    fn content(&mut self, index: usize, depth: usize) -> Result<(), Error> {
        // An independent copy of the source, so slicing it does not borrow
        // `self` while `self.nodes` is being written to.
        let text = self.text;

        loop {
            match self.peek() {
                None => return Err(self.error("unclosed element")),

                Some(b'<') => {
                    if self.starts_with("</") {
                        self.advance(2);
                        let name = self.name();
                        self.skip_whitespace();

                        if self.peek() != Some(b'>') {
                            return Err(self.error("malformed closing tag"));
                        }
                        self.advance(1);

                        if name != self.nodes[index].name {
                            return Err(self.error("closing tag does not match its opening tag"));
                        }
                        return Ok(());
                    }

                    if self.starts_with("<!--") {
                        self.advance(4);
                        self.skip_past("-->", "unterminated comment")?;
                    } else if self.starts_with("<![CDATA[") {
                        self.advance(9);
                        let start = self.position;
                        self.skip_past("]]>", "unterminated CDATA section")?;
                        let raw = &text[start..self.position - 3];
                        self.nodes[index].text.push_str(raw);
                    } else if self.starts_with("<?") {
                        self.advance(2);
                        self.skip_past("?>", "unterminated processing instruction")?;
                    } else {
                        let child = self.element(depth + 1)?;
                        self.nodes[index].children.push(child);
                    }
                }

                Some(_) => {
                    let start = self.position;
                    let Some(offset) = text[start..].find('<') else {
                        return Err(self.error("unclosed element"));
                    };
                    self.advance(offset);

                    let decoded = decode(&text[start..self.position], self.line)?;
                    self.nodes[index].text.push_str(&decoded);
                }
            }
        }
    }

    /// Parses `name="value"`, the cursor sitting on the first name character.
    fn attribute(&mut self) -> Result<(String, String), Error> {
        let name = self.name();
        if name.is_empty() {
            return Err(self.error("expected an attribute name"));
        }

        self.skip_whitespace();
        if self.peek() != Some(b'=') {
            return Err(self.error("expected '=' after an attribute name"));
        }
        self.advance(1);
        self.skip_whitespace();

        let Some(quote @ (b'"' | b'\'')) = self.peek() else {
            return Err(self.error("expected a quoted attribute value"));
        };
        self.advance(1);

        let text = self.text;
        let start = self.position;
        let Some(offset) = text[start..].find(quote as char) else {
            return Err(self.error("unterminated attribute value"));
        };
        self.advance(offset);

        let value = decode(&text[start..self.position], self.line)?;
        self.advance(1); // closing quote

        Ok((name, value))
    }

    /// Consumes an element or attribute name. Deliberately permissive: any
    /// byte that cannot start markup is a name character.
    fn name(&mut self) -> String {
        let start = self.position;

        while let Some(byte) = self.peek() {
            match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b':' => {
                    self.advance(1)
                }
                _ if byte >= 0x80 => self.advance(1),
                _ => break,
            }
        }

        self.text[start..self.position].to_owned()
    }

    /// Whitespace, comments, processing instructions and the document type
    /// declaration — everything allowed outside the root element.
    fn skip_ignorable(&mut self) -> Result<(), Error> {
        loop {
            self.skip_whitespace();

            if self.starts_with("<!--") {
                self.advance(4);
                self.skip_past("-->", "unterminated comment")?;
            } else if self.starts_with("<!DOCTYPE") {
                self.advance(9);
                self.skip_past(">", "unterminated document type declaration")?;
            } else if self.starts_with("<?") {
                self.advance(2);
                self.skip_past("?>", "unterminated processing instruction")?;
            } else {
                return Ok(());
            }
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.advance(1);
        }
    }

    /// Advances past the next occurrence of `marker`.
    fn skip_past(&mut self, marker: &str, message: &'static str) -> Result<(), Error> {
        match self.text[self.position..].find(marker) {
            Some(offset) => {
                self.advance(offset + marker.len());
                Ok(())
            }
            None => Err(self.error(message)),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.text[self.position..].starts_with(prefix)
    }

    /// Moves forward `count` bytes, keeping the line number honest.
    fn advance(&mut self, count: usize) {
        let end = (self.position + count).min(self.text.len());
        self.line += self.text.as_bytes()[self.position..end]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count() as u32;
        self.position = end;
    }

    fn error(&self, message: &'static str) -> Error {
        Error {
            message,
            line: self.line,
        }
    }
}

/// Resolves entity and character references in text or an attribute value.
fn decode(raw: &str, line: u32) -> Result<String, Error> {
    if !raw.contains('&') {
        return Ok(raw.to_owned());
    }

    let fail = |message| Error { message, line };
    let mut decoded = String::with_capacity(raw.len());
    let mut rest = raw;

    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);

        let after = &rest[start + 1..];
        let Some(end) = after.find(';') else {
            return Err(fail("unterminated entity reference"));
        };

        match &after[..end] {
            "amp" => decoded.push('&'),
            "lt" => decoded.push('<'),
            "gt" => decoded.push('>'),
            "quot" => decoded.push('"'),
            "apos" => decoded.push('\''),

            // Only the five predefined entities exist without a DTD, so
            // anything else has to be a character reference.
            reference => {
                let Some(digits) = reference.strip_prefix('#') else {
                    return Err(fail("unknown entity reference"));
                };
                let code = match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16),
                    None => digits.parse(),
                }
                .map_err(|_| fail("malformed character reference"))?;

                let Some(character) = char::from_u32(code) else {
                    return Err(fail("character reference out of range"));
                };
                decoded.push(character);
            }
        }

        rest = &after[end + 1..];
    }

    decoded.push_str(rest);
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- a settings file -->
<y-snap>
	<hotkeys>
		<hotkey key="Win+Space" action="overlay" target="pointed"/>
		<hotkey key='Win+Escape' action='close'/>
	</hotkeys>
	<overlay columns="4" focus-border="yes">
		<action key="M" action="minimize" label="Minimize &amp; hide"/>
	</overlay>
</y-snap>
"#;

    #[test]
    fn reads_a_settings_file() {
        let document = Document::parse(SAMPLE).expect("valid");
        let root = document.root();
        assert_eq!(root.name(), "y-snap");

        let hotkeys: Vec<_> = root
            .child("hotkeys")
            .expect("present")
            .children_named("hotkey")
            .collect();
        assert_eq!(hotkeys.len(), 2);
        assert_eq!(hotkeys[0].attr("key"), Some("Win+Space"));
        assert_eq!(hotkeys[0].attr("target"), Some("pointed"));
        assert_eq!(hotkeys[1].attr("action"), Some("close"));
        assert_eq!(hotkeys[1].attr("target"), None);

        let overlay = root.child("overlay").expect("present");
        assert_eq!(overlay.attr_u32("columns"), Some(4));
        assert_eq!(overlay.attr_bool("focus-border"), Some(true));

        let action = overlay.child("action").expect("present");
        assert_eq!(action.attr("label"), Some("Minimize & hide"));
    }

    #[test]
    fn reports_the_line_of_the_mistake() {
        let error = Document::parse("<a>\n  <b>\n  <c/>\n</a>").expect_err("mismatched");
        assert_eq!(error.line, 4);
        assert!(error.to_string().starts_with("line 4:"));
    }

    #[test]
    fn keeps_attributes_in_order_for_reporting() {
        let document = Document::parse(r#"<a one="1" two="2"/>"#).expect("valid");
        let names: Vec<_> = document.root().attributes().map(|(name, _)| name).collect();
        assert_eq!(names, ["one", "two"]);
    }

    #[test]
    fn reads_text_comments_and_cdata() {
        let document =
            Document::parse("<a>one <!-- ignored --><![CDATA[& <two>]]> three</a>").expect("valid");
        assert_eq!(document.root().text(), "one & <two> three");
    }

    #[test]
    fn resolves_character_references() {
        let document =
            Document::parse(r#"<a v="&#65;&#x42;&lt;&gt;&quot;&apos;"/>"#).expect("valid");
        assert_eq!(document.root().attr("v"), Some("AB<>\"'"));
    }

    #[test]
    fn rejects_malformed_documents() {
        for text in [
            "",
            "<a>",
            "<a></b>",
            "<a/><b/>",
            "<a x/>",
            "<a x=1/>",
            r#"<a x="unterminated/>"#,
            "<a>&nope;</a>",
            "<!-- unterminated",
        ] {
            assert!(Document::parse(text).is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn refuses_to_recurse_without_end() {
        let deep = "<a>".repeat(MAX_DEPTH + 2);
        let error = Document::parse(&deep).expect_err("too deep");
        assert_eq!(error.message, "elements nested too deeply");
    }
}
