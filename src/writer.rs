//! Writing in the platform layout: a nested list starts on a new line,
//! and the closing bracket moves to a new line if the last item is a list. No BOM is written.

use std::fmt;
use std::io;

use crate::{Node, Value};

impl Node {
    /// Writes the bracket file text to any [`io::Write`].
    pub fn write_to<W: io::Write>(&self, writer: W) -> io::Result<()> {
        let mut adapter = IoAdapter {
            inner: writer,
            error: None,
        };
        match write(self, &mut adapter) {
            Ok(()) => Ok(()),
            Err(fmt::Error) => Err(adapter
                .error
                .unwrap_or_else(|| io::Error::other("formatter error"))),
        }
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write(self, f)
    }
}

impl From<&Node> for String {
    fn from(node: &Node) -> Self {
        node.to_string()
    }
}

impl From<&Node> for Vec<u8> {
    fn from(node: &Node) -> Self {
        node.to_string().into_bytes()
    }
}

/// Not recursive: forms can be deeply nested.
fn write(root: &Node, out: &mut impl fmt::Write) -> fmt::Result {
    // (list items, index of the next item)
    let mut stack: Vec<(&[Node], usize)> = Vec::new();
    open(root, out, &mut stack)?;
    while let Some(top) = stack.last_mut() {
        let (items, next) = *top;
        top.1 += 1;
        if let Some(item) = items.get(next) {
            if next > 0 {
                out.write_char(',')?;
            }
            if item.is_list() {
                out.write_str("\r\n")?;
            }
            open(item, out, &mut stack)?;
        } else {
            if items.last().is_some_and(Node::is_list) {
                out.write_str("\r\n")?;
            }
            out.write_char('}')?;
            stack.pop();
        }
    }
    Ok(())
}

/// Writes a scalar entirely, or opens a list and pushes it onto the stack.
fn open<'a>(
    node: &'a Node,
    out: &mut impl fmt::Write,
    stack: &mut Vec<(&'a [Node], usize)>,
) -> fmt::Result {
    match node.value() {
        Value::List(items) => {
            out.write_char('{')?;
            stack.push((items, 0));
        }
        Value::Str(s) => {
            out.write_char('"')?;
            for (i, part) in s.split('"').enumerate() {
                if i > 0 {
                    out.write_str("\"\"")?;
                }
                out.write_str(part)?;
            }
            out.write_char('"')?;
        }
        Value::Atom(s) => out.write_str(s)?,
    }
    Ok(())
}

struct IoAdapter<W> {
    inner: W,
    error: Option<io::Error>,
}

impl<W: io::Write> fmt::Write for IoAdapter<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.inner.write_all(s.as_bytes()).map_err(|e| {
            self.error = Some(e);
            fmt::Error
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_platform_layout() {
        let text = "{27,\r\n{18,\r\n{\r\n{1,1,\r\n{\"ru\",\"Форма\"}\r\n},25,4294967295},\
                    \r\n{0}\r\n},\"\",-1,\r\n{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,}\r\n}";
        assert_eq!(Node::try_from(text).unwrap().to_string(), text);
    }

    #[test]
    fn keeps_base64_with_line_breaks() {
        let text = "{1,\r\n{#base64:QUJD\r\r\nREVG\r\r\n},2}";
        assert_eq!(Node::try_from(text).unwrap().to_string(), text);
    }

    #[test]
    fn builds_and_writes() {
        let node = Node::from(vec![
            Node::from(1),
            Node::string("say \"hi\""),
            Node::from(vec![Node::from(true), Node::atom("")]),
        ]);
        insta::assert_snapshot!(node.to_string().replace("\r\n", "⏎"), @r#"{1,"say ""hi""",⏎{1,}⏎}"#);

        let mut bytes = Vec::new();
        node.write_to(&mut bytes).unwrap();
        assert_eq!(bytes, Vec::<u8>::from(&node));
        assert_eq!(Node::try_from(bytes.as_slice()).unwrap(), node);
    }

    #[test]
    fn deep_nesting_without_recursion() {
        let depth = 5_000;
        let text = format!("{}{}", "{".repeat(depth), "}".repeat(depth));
        let node = Node::try_from(text.as_str()).unwrap();
        assert_eq!(node.to_string().replace("\r\n", ""), text);
    }
}
