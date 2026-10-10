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
//!
//! # Comments
//!
//! Comments are parsed and kept, but stay out of the way: [`Element::children`]
//! returns elements only, so a reader that does not care never sees them. An
//! editor that must write a file back reads them with
//! [`Element::children_with_comments`] and [`Document::prolog_comments`], and
//! writes them again with [`Writer`].

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
    root: usize,
    /// Comments before and after the root element, in document order.
    prolog: Vec<usize>,
    epilog: Vec<usize>,
}

/// An element or a comment. For a comment, `text` is the comment's text and
/// the other fields are empty.
#[derive(Debug)]
struct NodeData {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<usize>,
    text: String,
    line: u32,
    comment: bool,
    /// An empty line separates this node from the sibling before it.
    blank_before: bool,
}

impl NodeData {
    fn element(name: String, line: u32) -> Self {
        Self {
            name,
            attributes: Vec::new(),
            children: Vec::new(),
            text: String::new(),
            line,
            comment: false,
            blank_before: false,
        }
    }
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
            index: self.root,
        }
    }

    /// The comments before the root element, in order.
    pub fn prolog_comments(&self) -> impl Iterator<Item = Comment<'_>> {
        self.comments(&self.prolog)
    }

    /// The comments after the root element, in order.
    pub fn epilog_comments(&self) -> impl Iterator<Item = Comment<'_>> {
        self.comments(&self.epilog)
    }

    fn comments<'a>(&'a self, indices: &'a [usize]) -> impl Iterator<Item = Comment<'a>> {
        indices.iter().map(move |&index| Comment {
            document: self,
            index,
        })
    }
}

/// One child of an element, as [`Element::children_with_comments`] yields it.
#[derive(Clone, Copy)]
pub enum Node<'a> {
    Element(Element<'a>),
    Comment(Comment<'a>),
}

/// A comment, cheap to copy.
#[derive(Clone, Copy)]
pub struct Comment<'a> {
    document: &'a Document,
    index: usize,
}

impl<'a> Comment<'a> {
    fn data(&self) -> &'a NodeData {
        &self.document.nodes[self.index]
    }

    /// The comment's text without its delimiters. Surrounding blank space is
    /// trimmed, and the indentation shared by the lines after the first is
    /// removed, so the text reads the same wherever the comment was nested.
    pub fn text(&self) -> &'a str {
        &self.data().text
    }

    /// The line the comment opened on.
    pub fn line(&self) -> u32 {
        self.data().line
    }

    /// Whether an empty line separates it from the node before it.
    pub fn blank_before(&self) -> bool {
        self.data().blank_before
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
            .filter(move |&&index| !document.nodes[index].comment)
            .map(move |&index| Element { document, index })
    }

    /// Every child, elements and comments alike, in document order.
    pub fn children_with_comments(&self) -> impl Iterator<Item = Node<'a>> {
        let document = self.document;
        self.data().children.iter().map(move |&index| {
            if document.nodes[index].comment {
                Node::Comment(Comment { document, index })
            } else {
                Node::Element(Element { document, index })
            }
        })
    }

    /// Whether an empty line separates the element from the node before it
    /// (an element or a comment). `false` for a first child.
    pub fn blank_before(&self) -> bool {
        self.data().blank_before
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
        let mut prolog = Vec::new();
        let blank = self.skip_ignorable(&mut prolog, false)?;

        if self.peek() != Some(b'<') {
            return Err(self.error("expected an element"));
        }
        let root = self.element(0)?;
        self.nodes[root].blank_before = blank && !prolog.is_empty();

        let mut epilog = Vec::new();
        self.skip_ignorable(&mut epilog, true)?;
        if self.position != self.text.len() {
            return Err(self.error("trailing content after the root element"));
        }

        Ok(Document {
            nodes: self.nodes,
            root,
            prolog,
            epilog,
        })
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
        self.nodes.push(NodeData::element(name, line));

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
        // Whether the last run of text ended in an empty line, for the next
        // element or comment to pick up.
        let mut blank = false;

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

                    let has_previous = !self.nodes[index].children.is_empty();

                    if self.starts_with("<!--") {
                        let comment = self.comment(blank && has_previous)?;
                        self.nodes[index].children.push(comment);
                        blank = false;
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
                        self.nodes[child].blank_before = blank && has_previous;
                        self.nodes[index].children.push(child);
                        blank = false;
                    }
                }

                Some(_) => {
                    let start = self.position;
                    let Some(offset) = text[start..].find('<') else {
                        return Err(self.error("unclosed element"));
                    };
                    self.advance(offset);

                    blank = ends_in_empty_line(&text[start..self.position]);
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

    /// Parses a comment, the cursor on its `<!--`, and returns its index.
    fn comment(&mut self, blank_before: bool) -> Result<usize, Error> {
        let text = self.text;
        let line = self.line;

        self.advance(4);
        let start = self.position;
        self.skip_past("-->", "unterminated comment")?;

        let index = self.nodes.len();
        self.nodes.push(NodeData {
            text: tidy_comment(&text[start..self.position - 3]),
            comment: true,
            blank_before,
            ..NodeData::element(String::new(), line)
        });
        Ok(index)
    }

    /// Whitespace, comments, processing instructions and the document type
    /// declaration — everything allowed outside the root element. Comments
    /// are collected into `comments`.
    ///
    /// Returns whether an empty line sits between whatever came last and the
    /// thing the cursor stopped at. `has_previous` says whether the root
    /// element already precedes it.
    fn skip_ignorable(
        &mut self,
        comments: &mut Vec<usize>,
        has_previous: bool,
    ) -> Result<bool, Error> {
        loop {
            let before = self.position;
            self.skip_whitespace();
            let blank = ends_in_empty_line(&self.text[before..self.position]);

            if self.starts_with("<!--") {
                let blank_before = blank && (has_previous || !comments.is_empty());
                comments.push(self.comment(blank_before)?);
            } else if self.starts_with("<!DOCTYPE") {
                self.advance(9);
                self.skip_past(">", "unterminated document type declaration")?;
            } else if self.starts_with("<?") {
                self.advance(2);
                self.skip_past("?>", "unterminated processing instruction")?;
            } else {
                return Ok(blank);
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

/// Whether trailing whitespace holds an empty line: two line breaks with
/// nothing but blanks between them.
fn ends_in_empty_line(gap: &str) -> bool {
    let trimmed = gap.trim_end_matches([' ', '\t', '\r', '\n']);
    gap[trimmed.len()..]
        .bytes()
        .filter(|&byte| byte == b'\n')
        .count()
        >= 2
}

/// A comment's text as the editor wants it: trimmed, line breaks as `\n`, and
/// the indentation shared by the lines after the first removed.
fn tidy_comment(raw: &str) -> String {
    let raw = raw.replace("\r\n", "\n");
    let mut lines = raw.trim().split('\n');
    let first = lines.next().unwrap_or("");
    let rest: Vec<&str> = lines.collect();

    // The longest run of blanks every non-empty later line starts with.
    let mut indent: Option<&str> = None;
    for line in rest.iter().filter(|line| !line.trim().is_empty()) {
        let own = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        indent = Some(match indent {
            None => own,
            Some(shared) => {
                let length = shared
                    .bytes()
                    .zip(own.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                &shared[..length]
            }
        });
    }
    let indent = indent.unwrap_or("");

    let mut tidy = first.trim_end().to_owned();
    for line in rest {
        tidy.push('\n');
        tidy.push_str(line.strip_prefix(indent).unwrap_or(line.trim_start()).trim_end());
    }
    tidy
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

/// Writes the subset of XML [`Document`] reads: tab-indented elements,
/// escaped attributes, and comments.
///
/// ```
/// use y_core::xml::Writer;
///
/// let mut writer = Writer::new();
/// writer.open("config");
/// writer.comment("one binding");
/// writer.open("hotkey");
/// writer.attr("key", "Win+Space");
/// writer.close();
/// writer.close();
/// assert_eq!(
///     writer.finish(),
///     "<config>\n\t<!-- one binding -->\n\t<hotkey key=\"Win+Space\"/>\n</config>\n",
/// );
/// ```
///
/// An element is a start tag that stays open for [`attr`](Writer::attr) until
/// the next thing is written; closing an element that got no content makes it
/// self-closing. Calls that don't fit (an attribute after content, more
/// closes than opens) are bugs in the caller and panic in debug builds.
#[derive(Debug, Default)]
pub struct Writer {
    out: String,
    /// Names of the elements still open.
    open: Vec<String>,
    /// The innermost start tag has not been finished with `>` yet.
    tag_open: bool,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// `<?xml version="1.0" encoding="utf-8"?>`, for the top of a file.
    pub fn declaration(&mut self) {
        self.out
            .push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    }

    /// Starts an element; set its attributes with [`attr`](Writer::attr).
    pub fn open(&mut self, name: &str) {
        self.finish_tag();
        self.indent(self.open.len());
        self.out.push('<');
        self.out.push_str(name);
        self.open.push(name.to_owned());
        self.tag_open = true;
    }

    /// Adds an attribute to the element just opened.
    pub fn attr(&mut self, name: &str, value: &str) {
        debug_assert!(self.tag_open, "attribute after the element's content");
        self.out.push(' ');
        self.out.push_str(name);
        self.out.push_str("=\"");
        for character in value.chars() {
            match character {
                '&' => self.out.push_str("&amp;"),
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '"' => self.out.push_str("&quot;"),
                // Kept as references, so they survive being read back.
                '\n' => self.out.push_str("&#10;"),
                '\r' => self.out.push_str("&#13;"),
                '\t' => self.out.push_str("&#9;"),
                other => self.out.push(other),
            }
        }
        self.out.push('"');
    }

    /// Closes the innermost element: `/>` if it has no content, else its end
    /// tag.
    pub fn close(&mut self) {
        let name = self.open.pop();
        debug_assert!(name.is_some(), "close without a matching open");
        let Some(name) = name else { return };

        if self.tag_open {
            self.out.push_str("/>\n");
            self.tag_open = false;
        } else {
            self.indent(self.open.len());
            self.out.push_str("</");
            self.out.push_str(&name);
            self.out.push_str(">\n");
        }
    }

    /// An element with no content, closed at once: `open` + `close`.
    pub fn empty(&mut self) {
        debug_assert!(self.tag_open, "empty() after the element's content");
        self.close();
    }

    /// A comment at the current depth.
    ///
    /// `--` isn't allowed inside a comment, so it is written `- -`. A text of
    /// one line stays on one line; a longer one gets the delimiters on lines
    /// of their own and every line indented to the depth.
    pub fn comment(&mut self, text: &str) {
        self.finish_tag();
        let depth = self.open.len();

        let mut safe = text.replace("\r\n", "\n");
        while safe.contains("--") {
            safe = safe.replace("--", "- -");
        }
        let safe = safe.trim();

        self.indent(depth);
        if safe.contains('\n') {
            self.out.push_str("<!--\n");
            for line in safe.split('\n') {
                let line = line.trim_end();
                if !line.is_empty() {
                    self.indent(depth + 1);
                    self.out.push_str(line);
                }
                self.out.push('\n');
            }
            self.indent(depth);
            self.out.push_str("-->\n");
        } else {
            self.out.push_str("<!-- ");
            self.out.push_str(safe);
            self.out.push_str(" -->\n");
        }
    }

    /// An empty line, unless the output already ends in one or is empty.
    pub fn blank_line(&mut self) {
        self.finish_tag();
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    /// The document written so far.
    pub fn finish(self) -> String {
        debug_assert!(self.open.is_empty(), "unclosed elements: {:?}", self.open);
        self.out
    }

    /// Ends a pending start tag because content follows it.
    fn finish_tag(&mut self) {
        if self.tag_open {
            self.out.push_str(">\n");
            self.tag_open = false;
        }
    }

    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push('\t');
        }
    }
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

    fn texts<'a>(comments: impl Iterator<Item = Comment<'a>>) -> Vec<&'a str> {
        comments.map(|comment| comment.text()).collect()
    }

    #[test]
    fn keeps_comments_around_the_root() {
        let document =
            Document::parse("<?xml version=\"1.0\"?>\n<!-- one -->\n\n<!-- two -->\n<a/>\n<!-- after -->")
                .expect("valid");
        assert_eq!(texts(document.prolog_comments()), ["one", "two"]);
        assert_eq!(texts(document.epilog_comments()), ["after"]);

        let blanks: Vec<_> = document.prolog_comments().map(|c| c.blank_before()).collect();
        assert_eq!(blanks, [false, true]);
        assert!(!document.root().blank_before());
        assert_eq!(document.root().name(), "a");
    }

    #[test]
    fn a_comment_stays_out_of_the_element_view() {
        let document = Document::parse("<a><!-- x --><b/><!-- y --><c/></a>").expect("valid");
        let names: Vec<_> = document.root().children().map(|e| e.name()).collect();
        assert_eq!(names, ["b", "c"]);
        assert_eq!(document.root().children_with_comments().count(), 4);
    }

    #[test]
    fn keeps_comments_between_children_in_order() {
        let document = Document::parse(
            "<a>\n\t<!-- first -->\n\t<b/>\n\n\t<!-- second -->\n\t<c/>\n</a>",
        )
        .expect("valid");

        let mut seen = Vec::new();
        for node in document.root().children_with_comments() {
            match node {
                Node::Comment(c) => seen.push(format!("#{} {}", c.text(), c.blank_before())),
                Node::Element(e) => seen.push(format!("{} {}", e.name(), e.blank_before())),
            }
        }
        assert_eq!(
            seen,
            ["#first false", "b false", "#second true", "c false"],
        );
    }

    #[test]
    fn a_comment_can_be_the_only_content() {
        let document = Document::parse("<a>\n\t<!-- only -->\n</a>").expect("valid");
        assert_eq!(document.root().children().count(), 0);
        let Some(Node::Comment(comment)) = document.root().children_with_comments().next() else {
            panic!("expected a comment");
        };
        assert_eq!(comment.text(), "only");
        assert_eq!(comment.line(), 2);
    }

    #[test]
    fn dedents_a_multi_line_comment() {
        let document = Document::parse(
            "<a>\n\t\t<!--\n\t\t\tline one\n\t\t\t  indented\n\n\t\t\tline three\n\t\t-->\n</a>",
        )
        .expect("valid");
        let Some(Node::Comment(comment)) = document.root().children_with_comments().next() else {
            panic!("expected a comment");
        };
        assert_eq!(comment.text(), "line one\n  indented\n\nline three");
    }

    #[test]
    fn writes_what_it_is_told() {
        let mut writer = Writer::new();
        writer.declaration();
        writer.open("a");
        writer.attr("v", "x & <y> \"z\"");
        writer.comment("note");
        writer.open("b");
        writer.empty();
        writer.blank_line();
        writer.blank_line();
        writer.open("c");
        writer.open("d");
        writer.close();
        writer.close();
        writer.close();

        assert_eq!(
            writer.finish(),
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <a v=\"x &amp; &lt;y&gt; &quot;z&quot;\">\n\
             \t<!-- note -->\n\
             \t<b/>\n\
             \n\
             \t<c>\n\
             \t\t<d/>\n\
             \t</c>\n\
             </a>\n"
        );
    }

    #[test]
    fn never_writes_a_double_hyphen_in_a_comment() {
        let mut writer = Writer::new();
        writer.open("a");
        writer.comment("a -- b --- c");
        writer.comment("ends with -");
        writer.close();
        let text = writer.finish();

        let document = Document::parse(&text).expect("a well-formed document");
        let Some(Node::Comment(first)) = document.root().children_with_comments().next() else {
            panic!("expected a comment");
        };
        assert_eq!(first.text(), "a - - b - - - c");
    }

    #[test]
    fn indents_a_multi_line_comment_to_its_depth() {
        let mut writer = Writer::new();
        writer.open("a");
        writer.open("b");
        writer.comment("one\n  two\n\nthree");
        writer.close();
        writer.close();
        let text = writer.finish();
        assert_eq!(
            text,
            "<a>\n\t<b>\n\t\t<!--\n\t\t\tone\n\t\t\t  two\n\n\t\t\tthree\n\t\t-->\n\t</b>\n</a>\n"
        );

        let document = Document::parse(&text).expect("valid");
        let Some(Node::Comment(comment)) = document
            .root()
            .child("b")
            .expect("present")
            .children_with_comments()
            .next()
        else {
            panic!("expected a comment");
        };
        assert_eq!(comment.text(), "one\n  two\n\nthree");
    }

    #[test]
    fn parse_write_parse_keeps_elements_and_comments() {
        fn write(element: Element<'_>, writer: &mut Writer) {
            writer.open(element.name());
            for (name, value) in element.attributes() {
                writer.attr(name, value);
            }
            for node in element.children_with_comments() {
                match node {
                    Node::Comment(comment) => {
                        if comment.blank_before() {
                            writer.blank_line();
                        }
                        writer.comment(comment.text());
                    }
                    Node::Element(child) => {
                        if child.blank_before() {
                            writer.blank_line();
                        }
                        write(child, writer);
                    }
                }
            }
            writer.close();
        }

        let source = "<!-- head -->\n<a x=\"1 &amp; 2\">\n\t<!-- one -->\n\t<b y=\"&#10;\"/>\n\n\t<g>\n\t\t<!--\n\t\t\tdeep\n\t\t\ttext\n\t\t-->\n\t\t<c/>\n\t</g>\n</a>\n";
        let first = Document::parse(source).expect("valid");

        let mut writer = Writer::new();
        for comment in first.prolog_comments() {
            writer.comment(comment.text());
        }
        write(first.root(), &mut writer);
        let written = writer.finish();

        // Writing is stable: the second pass reproduces the first.
        let second = Document::parse(&written).expect("valid");
        let mut again = Writer::new();
        for comment in second.prolog_comments() {
            again.comment(comment.text());
        }
        write(second.root(), &mut again);
        assert_eq!(again.finish(), written);
        assert_eq!(written, source);
    }
}
