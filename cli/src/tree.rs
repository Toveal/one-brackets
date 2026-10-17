//! Helpers shared by the formats: a non-recursive walk over a tree, checks of atoms read
//! from other formats and positions in source text.

use std::fmt;
use std::io::{self, Write};
use std::slice;

use anyhow::{Result, bail};
use one_brackets::{Node, Text, Value};

/// A step of a depth-first walk.
pub enum Event<'a> {
    /// A list starts; its items follow, then [`Event::End`].
    Start(&'a [Node]),
    /// A quoted string or an atom.
    Leaf(&'a Node),
    /// The innermost open list ends.
    End,
}

/// Depth-first walk without recursion: bracket files can be deeply nested.
pub struct Events<'a> {
    root: Option<&'a Node>,
    stack: Vec<slice::Iter<'a, Node>>,
}

impl<'a> Events<'a> {
    pub fn new(root: &'a Node) -> Self {
        Self {
            root: Some(root),
            stack: Vec::new(),
        }
    }

    fn enter(&mut self, node: &'a Node) -> Event<'a> {
        match node.value() {
            Value::List(items) => {
                self.stack.push(items.iter());
                Event::Start(items)
            }
            _ => Event::Leaf(node),
        }
    }
}

impl<'a> Iterator for Events<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        if let Some(root) = self.root.take() {
            return Some(self.enter(root));
        }
        let next = self.stack.last_mut()?.next();
        match next {
            Some(node) => Some(self.enter(node)),
            None => {
                self.stack.pop();
                Some(Event::End)
            }
        }
    }
}

/// A list of scalars and empty lists: written on one line.
pub fn is_flat(items: &[Node]) -> bool {
    !items
        .iter()
        .any(|item| item.as_list().is_some_and(|l| !l.is_empty()))
}

/// Punctuation of a format whose lists are written as nested blocks: JSON, flow YAML, XML.
pub trait Syntax {
    fn open(&self, out: &mut dyn Write) -> io::Result<()>;
    fn close(&self, out: &mut dyn Write) -> io::Result<()>;
    fn empty(&self, out: &mut dyn Write) -> io::Result<()>;
    /// Written between items, before the line break.
    const SEPARATOR: &'static str;
    fn leaf(&self, node: &Node, out: &mut dyn Write) -> io::Result<()>;
}

/// Writes `node` with lists of scalars on one line and other lists one item per line,
/// indented by two spaces per level starting from `depth`; `compact` writes everything
/// on one line. No line break is written after the value.
pub fn write_nested<S: Syntax>(
    syntax: &S,
    node: &Node,
    out: &mut dyn Write,
    compact: bool,
    depth: usize,
) -> io::Result<()> {
    struct Frame {
        inline: bool,
        empty: bool,
        first: bool,
    }
    let new_line = |out: &mut dyn Write, level: usize| -> io::Result<()> {
        out.write_all(
            b"
",
        )?;
        for _ in 0..level {
            out.write_all(b"  ")?;
        }
        Ok(())
    };
    let mut frames: Vec<Frame> = Vec::new();
    for event in Events::new(node) {
        if !matches!(event, Event::End) {
            if let Some(frame) = frames.last_mut() {
                let first = std::mem::replace(&mut frame.first, false);
                if !first {
                    out.write_all(S::SEPARATOR.as_bytes())?;
                }
                if !frame.inline {
                    new_line(out, depth + frames.len())?;
                } else if !first && !compact && !S::SEPARATOR.is_empty() {
                    out.write_all(b" ")?;
                }
            }
        }
        match event {
            Event::Start(items) => {
                if items.is_empty() {
                    syntax.empty(out)?;
                } else {
                    syntax.open(out)?;
                }
                frames.push(Frame {
                    inline: compact || is_flat(items),
                    empty: items.is_empty(),
                    first: true,
                });
            }
            Event::Leaf(leaf) => syntax.leaf(leaf, out)?,
            Event::End => {
                let frame = frames.pop().expect("a list is open");
                if !frame.empty {
                    if !frame.inline {
                        new_line(out, depth + frames.len())?;
                    }
                    syntax.close(out)?;
                }
            }
        }
    }
    Ok(())
}

/// Builds an atom read from another format, checking that the bracket parser
/// would read the written text back as the same atom.
///
/// An atom runs up to the next `,` or `}`; a leading `{`, quote or whitespace would start
/// a list, start a string or be skipped.
pub fn atom(text: Text, at: impl fmt::Display) -> Result<Node> {
    let Some(s) = text.as_str() else {
        bail!("atom {text:?} at {at} holds an unpaired surrogate: use a quoted string instead");
    };
    let problem = if s.contains([',', '}']) {
        Some("contains ',' or '}'")
    } else if s.starts_with(['{', '"']) {
        Some("starts with '{' or a quote")
    } else if !s.is_empty()
        && Node::try_from(format!("{{{s}}}").as_str()).ok() != Some(Node::list([Node::atom(s)]))
    {
        // Whitespace as the platform understands it, U+00A0 and the like included.
        Some("starts with whitespace")
    } else {
        None
    };
    match problem {
        Some(problem) => bail!("atom {s:?} at {at} {problem}: use a quoted string instead"),
        None => Ok(Node::atom(text)),
    }
}

/// Text of a string or an atom for a format that holds only Unicode: YAML, XML.
pub fn unicode<'a>(text: &'a Text, format: &str) -> io::Result<&'a str> {
    text.as_str().ok_or_else(|| {
        io::Error::other(format!(
            "text {text:?} holds an unpaired surrogate, which {format} cannot keep: export to JSON"
        ))
    })
}

/// Checks the top-level value: an empty atom alone would be written as an empty file.
pub fn root(node: Node) -> Result<Node> {
    if node.as_atom() == Some("") {
        bail!("an empty atom cannot be the top-level value");
    }
    Ok(node)
}

/// Line and column (both 1-based, the column in characters) of a byte offset in `text`.
pub struct Location<'a> {
    pub text: &'a str,
    pub offset: usize,
}

impl fmt::Display for Location<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut offset = self.offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let before = &self.text[..offset];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        write!(f, "line {line}, column {column}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_in_order() {
        let node: Node = "{1,{2,{}},\"a\"}".parse().unwrap();
        let mut trace = String::new();
        for event in Events::new(&node) {
            match event {
                Event::Start(_) => trace.push('['),
                Event::Leaf(leaf) => trace.push_str(leaf.as_str().unwrap()),
                Event::End => trace.push(']'),
            }
        }
        assert_eq!(trace, "[1[2[]]a]");
    }

    #[test]
    fn checks_atoms() {
        for good in [
            "c2f34349-b1e0-4060-8abf-3c59f38ac69f",
            "",
            "#base64:QUJD\r\r\nREVG\r\r\n",
            "a b",
            "a\n",
            "a{b\"",
        ] {
            assert!(atom(good.into(), "x").is_ok(), "{good:?}");
        }
        for bad in ["a,b", "a}", "{", "\"q", " a", "\u{a0}a", "\r\na"] {
            assert!(atom(bad.into(), "x").is_err(), "{bad:?}");
        }
        assert!(atom(Text::from_utf16(&[0xD800]), "x").is_err());
        assert_eq!(
            atom(" a".into(), "line 1, column 2")
                .unwrap_err()
                .to_string(),
            "atom \" a\" at line 1, column 2 starts with whitespace: use a quoted string instead"
        );
    }

    #[test]
    fn locates_offsets() {
        let text = "ab\nвгд\ne";
        assert_eq!(Location { text, offset: 0 }.to_string(), "line 1, column 1");
        assert_eq!(Location { text, offset: 7 }.to_string(), "line 2, column 3");
        assert_eq!(
            Location { text, offset: 100 }.to_string(),
            "line 3, column 2"
        );
    }
}
