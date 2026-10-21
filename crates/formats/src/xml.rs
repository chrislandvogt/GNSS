//! A small layer over quick-xml for the XML formats: the root element, then
//! each of its children as a tree, one at a time, so a large file is never
//! held as one big tree.

use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::error::FormatError;

/// How deeply elements may nest below the root. The formats read here need a
/// handful of levels; the limit keeps hostile files from using unbounded memory.
const MAX_DEPTH: usize = 32;

/// An element: its name, attributes, text and children.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Node {
    /// The element's name, without a namespace prefix.
    pub name: String,
    /// Attributes, in order, without namespace prefixes on their names.
    pub attributes: Vec<(String, String)>,
    /// The element's text, with entities resolved and surrounding whitespace trimmed.
    pub text: String,
    /// Child elements, in order.
    pub children: Vec<Node>,
    /// The line the element starts on.
    pub line: usize,
}

impl Node {
    /// The first child with this name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// The text of the first child with this name.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }

    /// An attribute's value.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Line numbers for byte offsets that only move forward.
struct Lines<'a> {
    bytes: &'a [u8],
    offset: usize,
    line: usize,
}

impl Lines<'_> {
    fn at(&mut self, offset: u64) -> usize {
        let offset = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.bytes.len());
        if offset > self.offset {
            self.line += self.bytes[self.offset..offset]
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            self.offset = offset;
        }
        self.line
    }
}

/// An XML document being read child by child.
pub(crate) struct Document<'a> {
    file: &'a str,
    reader: Reader<&'a [u8]>,
    lines: Lines<'a>,
    /// The root element, without its children.
    pub root: Node,
    finished: bool,
}

impl<'a> Document<'a> {
    /// Reads up to and including the root element's start tag.
    pub fn open(file: &'a str, text: &'a str) -> Result<Self, FormatError> {
        let mut document = Document {
            file,
            reader: Reader::from_str(text),
            lines: Lines {
                bytes: text.as_bytes(),
                offset: 0,
                line: 1,
            },
            root: Node::default(),
            finished: false,
        };
        loop {
            let before = document.reader.buffer_position();
            match document.event()? {
                Event::Start(e) => {
                    document.root = document.node(&e, before)?;
                    return Ok(document);
                }
                Event::Empty(e) => {
                    document.root = document.node(&e, before)?;
                    document.finished = true;
                    return Ok(document);
                }
                Event::Eof => return Err(FormatError::in_file(file, "it has no XML elements")),
                Event::Text(t) if !t.xml10_content().trim().is_empty() => {
                    let line = document.lines.at(before);
                    return Err(FormatError::at(
                        file,
                        line,
                        "there's text before the first XML element: it isn't an XML file",
                    ));
                }
                _ => {}
            }
        }
    }

    /// The root's next child element, complete, or `None` after the root's end tag.
    pub fn next_child(&mut self) -> Result<Option<Node>, FormatError> {
        if self.finished {
            return Ok(None);
        }
        let mut stack: Vec<Node> = Vec::new();
        loop {
            let before = self.reader.buffer_position();
            match self.event()? {
                Event::Start(e) => {
                    let node = self.node(&e, before)?;
                    if stack.len() >= MAX_DEPTH {
                        return Err(FormatError::at(
                            self.file,
                            node.line,
                            format!("elements nest more than {MAX_DEPTH} deep"),
                        ));
                    }
                    stack.push(node);
                }
                Event::Empty(e) => {
                    let node = self.node(&e, before)?;
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => return Ok(Some(node)),
                    }
                }
                Event::End(_) => match stack.pop() {
                    Some(mut node) => {
                        let trimmed = node.text.trim();
                        if trimmed.len() != node.text.len() {
                            node.text = trimmed.to_owned();
                        }
                        match stack.last_mut() {
                            Some(parent) => parent.children.push(node),
                            None => return Ok(Some(node)),
                        }
                    }
                    None => {
                        self.finished = true;
                        return Ok(None);
                    }
                },
                Event::Text(t) => {
                    if let Some(node) = stack.last_mut() {
                        node.text.push_str(&t.xml10_content());
                    }
                }
                Event::CData(c) => {
                    if let Some(node) = stack.last_mut() {
                        node.text.push_str(&c.xml10_content());
                    }
                }
                Event::GeneralRef(r) => {
                    let resolved = match r.resolve_char_ref() {
                        Ok(Some(c)) => c.to_string(),
                        Ok(None) => {
                            let name = r.xml10_content();
                            match resolve_predefined_entity(&name) {
                                Some(text) => text.to_owned(),
                                None => {
                                    let line = self.lines.at(before);
                                    return Err(FormatError::at(
                                        self.file,
                                        line,
                                        format!("unknown entity &{name};"),
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            let line = self.lines.at(before);
                            return Err(FormatError::at(self.file, line, e.to_string()));
                        }
                    };
                    if let Some(node) = stack.last_mut() {
                        node.text.push_str(&resolved);
                    }
                }
                Event::Eof => {
                    let line = self.lines.at(before);
                    let inside = stack.last().map_or(self.root.name.as_str(), |n| &n.name);
                    return Err(FormatError::at(
                        self.file,
                        line,
                        format!("the file ends inside <{inside}>: it's incomplete"),
                    ));
                }
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            }
        }
    }

    fn event(&mut self) -> Result<Event<'a>, FormatError> {
        self.reader.read_event().map_err(|e| {
            let line = self.lines.at(self.reader.error_position());
            FormatError::at(self.file, line, format!("it isn't well-formed XML: {e}"))
        })
    }

    fn node(&mut self, start: &BytesStart<'_>, at: u64) -> Result<Node, FormatError> {
        let line = self.lines.at(at);
        let name = start.local_name().as_ref().to_owned();
        let mut attributes = Vec::new();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|e| {
                FormatError::at(self.file, line, format!("bad attribute in <{name}>: {e}"))
            })?;
            let key = attribute.key.local_name().as_ref().to_owned();
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|e| FormatError::at(self.file, line, format!("bad attribute {key}: {e}")))?
                .into_owned();
            attributes.push((key, value));
        }
        Ok(Node {
            name,
            attributes,
            text: String::new(),
            children: Vec::new(),
            line,
        })
    }
}

/// `text` with `&`, `<`, `>` and `"` escaped, for element text and attribute values.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn reads_children_one_at_a_time() {
        let text = "<?xml version=\"1.0\"?>\n<!-- c -->\n<Root a=\"1 &amp; 2\" xmlns:x=\"u\">\n  <A>\n    <B> x &lt; y &#65; </B>\n    <C/>\n  </A>\n  <x:D><![CDATA[<raw>]]></x:D>\n</Root>\n";
        let mut d = Document::open("t.xml", text).unwrap();
        assert_eq!(d.root.name, "Root");
        assert_eq!(d.root.attribute("a"), Some("1 & 2"));
        let a = d.next_child().unwrap().unwrap();
        assert_eq!((a.name.as_str(), a.line), ("A", 4));
        assert_eq!(a.text_of("B"), Some("x < y A"));
        assert_eq!(a.child("C").map(|c| c.line), Some(6));
        let dd = d.next_child().unwrap().unwrap();
        assert_eq!((dd.name.as_str(), dd.text.as_str()), ("D", "<raw>"));
        assert!(d.next_child().unwrap().is_none());
        assert!(d.next_child().unwrap().is_none());
    }

    #[test]
    fn reports_where_the_xml_is_broken() {
        let err = |text: &str| {
            let mut d = match Document::open("t.xml", text) {
                Ok(d) => d,
                Err(e) => return e.to_string(),
            };
            loop {
                match d.next_child() {
                    Ok(Some(_)) => {}
                    Ok(None) => return String::from("no error"),
                    Err(e) => return e.to_string(),
                }
            }
        };
        assert_eq!(err(""), "t.xml: it has no XML elements");
        assert!(err("hello").contains("isn't an XML file"));
        assert!(err("<R>\n<A>\n</B>\n</R>").starts_with("t.xml, line 3: it isn't well-formed XML"));
        assert_eq!(
            err("<R>\n<A>\n"),
            "t.xml, line 3: the file ends inside <A>: it's incomplete"
        );
        assert!(err("<R><A>&nope;</A></R>").contains("unknown entity &nope;"));
        let deep = format!("<R>{}{}</R>", "<a>".repeat(40), "</a>".repeat(40));
        assert!(err(&deep).contains("nest more than 32 deep"));
        assert_eq!(err("<R/>"), "no error");
    }

    #[test]
    fn escapes_text() {
        assert_eq!(escape("a<b & \"c\">"), "a&lt;b &amp; &quot;c&quot;&gt;");
    }
}
