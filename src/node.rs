//! Object model of the bracket format.

use std::ops::Index;

use crate::{Error, Result};

const BASE64: &str = "#base64:";

/// A node of a bracket file.
///
/// Quoted strings and atoms are kept distinct, so writing reproduces
/// the original platform text byte for byte.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Node {
    /// A list `{a,b,c}`.
    List(Vec<Node>),
    /// A quoted string; doubled quotes are already unescaped.
    Str(String),
    /// An unquoted value: number, GUID, literal; may be empty (`{2,x,}`).
    /// Binary data `{#base64:…}` is an atom too, line breaks included.
    Atom(String),
}

impl Node {
    /// An unquoted atom (GUID, literal, number).
    pub fn atom(value: impl Into<String>) -> Self {
        Node::Atom(value.into())
    }

    /// A quoted string.
    ///
    /// There is no `From<&str>` for building: `Node::try_from(&str)` parses text.
    pub fn string(value: impl Into<String>) -> Self {
        Node::Str(value.into())
    }

    pub fn is_list(&self) -> bool {
        matches!(self, Node::List(_))
    }

    pub fn is_str(&self) -> bool {
        matches!(self, Node::Str(_))
    }

    pub fn is_atom(&self) -> bool {
        matches!(self, Node::Atom(_))
    }

    /// An empty atom: no value between commas (`{1,,2}`).
    pub fn is_empty_atom(&self) -> bool {
        matches!(self, Node::Atom(s) if s.is_empty())
    }

    pub fn as_list(&self) -> Option<&[Node]> {
        match self {
            Node::List(items) => Some(items),
            _ => None,
        }
    }

    /// Value of a string or an atom.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) | Node::Atom(s) => Some(s),
            Node::List(_) => None,
        }
    }

    /// Quoted string only.
    pub fn as_quoted(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Atom only.
    pub fn as_atom(&self) -> Option<&str> {
        match self {
            Node::Atom(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.as_atom()?.parse().ok()
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_atom()?.parse().ok()
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.as_atom()?.parse().ok()
    }

    /// Atom `1` is `true`, `0` is `false`.
    pub fn as_bool(&self) -> Option<bool> {
        match self.as_atom()? {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }

    /// An atom shaped like a GUID `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub fn as_guid(&self) -> Option<&str> {
        self.as_atom().filter(|s| is_guid(s))
    }

    /// Payload of a `#base64:…` atom without the prefix and line breaks (not decoded).
    pub fn as_base64(&self) -> Option<String> {
        let data = self.as_atom()?.strip_prefix(BASE64)?;
        Some(data.chars().filter(|c| !c.is_ascii_whitespace()).collect())
    }

    /// List item by index.
    pub fn get(&self, idx: usize) -> Option<&Node> {
        self.as_list()?.get(idx)
    }

    /// Item by a path of indices from this node: `node.get_path(&[1, 0, 2])`.
    pub fn get_path(&self, path: &[usize]) -> Option<&Node> {
        path.iter().try_fold(self, |node, &idx| node.get(idx))
    }

    /// Number of list items; 0 for strings and atoms.
    pub fn len(&self) -> usize {
        self.as_list().map_or(0, <[Node]>::len)
    }

    /// List without items; strings and atoms are considered empty too.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// List items; an empty iterator for strings and atoms.
    pub fn iter(&self) -> std::slice::Iter<'_, Node> {
        self.as_list().unwrap_or(&[]).iter()
    }

    pub fn into_list(self) -> Option<Vec<Node>> {
        match self {
            Node::List(items) => Some(items),
            _ => None,
        }
    }

    /// Value of a string or an atom.
    pub fn into_string(self) -> Option<String> {
        match self {
            Node::Str(s) | Node::Atom(s) => Some(s),
            Node::List(_) => None,
        }
    }

    pub fn try_list(&self) -> Result<&[Node]> {
        self.as_list().ok_or_else(|| self.unexpected("list"))
    }

    pub fn try_str(&self) -> Result<&str> {
        self.as_str().ok_or_else(|| self.unexpected("string or value"))
    }

    pub fn try_quoted(&self) -> Result<&str> {
        self.as_quoted().ok_or_else(|| self.unexpected("quoted string"))
    }

    pub fn try_i64(&self) -> Result<i64> {
        self.as_i64().ok_or_else(|| self.unexpected("integer"))
    }

    pub fn try_u64(&self) -> Result<u64> {
        self.as_u64().ok_or_else(|| self.unexpected("unsigned integer"))
    }

    pub fn try_f64(&self) -> Result<f64> {
        self.as_f64().ok_or_else(|| self.unexpected("number"))
    }

    pub fn try_bool(&self) -> Result<bool> {
        self.as_bool().ok_or_else(|| self.unexpected("boolean (0 or 1)"))
    }

    pub fn try_guid(&self) -> Result<&str> {
        self.as_guid().ok_or_else(|| self.unexpected("GUID"))
    }

    pub fn try_get(&self, idx: usize) -> Result<&Node> {
        let items = self.try_list()?;
        items.get(idx).ok_or(Error::IndexOutOfBounds {
            index: idx,
            len: items.len(),
        })
    }

    /// Pre-order depth-first traversal with the path from the root. Not recursive: forms can be deeply nested.
    pub fn walk<'a>(&'a self, mut visit: impl FnMut(&[usize], &'a Node)) {
        // (node, depth, index in parent)
        let mut stack = vec![(self, 0usize, 0usize)];
        let mut path = Vec::new();
        while let Some((node, depth, idx)) = stack.pop() {
            if depth > 0 {
                path.truncate(depth - 1);
                path.push(idx);
            }
            visit(&path, node);
            for (i, item) in node.iter().enumerate().rev() {
                stack.push((item, depth + 1, i));
            }
        }
    }

    /// Short description of the node for error messages.
    pub fn describe(&self) -> String {
        const MAX: usize = 40;
        let clip = |s: &str| match s.char_indices().nth(MAX) {
            Some((end, _)) => format!("{}…", &s[..end]),
            None => s.to_owned(),
        };
        match self {
            Node::List(items) => match items.len() {
                0 => "empty list".to_owned(),
                1 => "list of 1 item".to_owned(),
                n => format!("list of {n} items"),
            },
            Node::Str(s) => format!("string {:?}", clip(s)),
            Node::Atom(s) if s.is_empty() => "empty value".to_owned(),
            Node::Atom(s) => format!("value `{}`", clip(s)),
        }
    }

    fn unexpected(&self, expected: &'static str) -> Error {
        Error::UnexpectedType {
            expected,
            found: self.describe(),
        }
    }
}

fn is_guid(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

impl Index<usize> for Node {
    type Output = Node;

    fn index(&self, idx: usize) -> &Node {
        match self.try_get(idx) {
            Ok(node) => node,
            Err(e) => panic!("cannot index node: {e}"),
        }
    }
}

impl<'a> IntoIterator for &'a Node {
    type Item = &'a Node;
    type IntoIter = std::slice::Iter<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<bool> for Node {
    fn from(b: bool) -> Self {
        Node::atom(if b { "1" } else { "0" })
    }
}

macro_rules! from_number {
    ($($t:ty),*) => {$(
        impl From<$t> for Node {
            fn from(n: $t) -> Self {
                Node::Atom(n.to_string())
            }
        }
    )*};
}

from_number!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

impl From<Vec<Node>> for Node {
    fn from(items: Vec<Node>) -> Self {
        Node::List(items)
    }
}

impl<T: Into<Node>> FromIterator<T> for Node {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Node::List(iter.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Node {
        Node::from(vec![
            Node::atom("12"),
            Node::from(vec![
                Node::string("text"),
                Node::atom("c2f34349-b1e0-4060-8abf-3c59f38ac69f"),
                Node::from(true),
            ]),
            Node::atom("#base64:QUJD\r\r\nREVG\r\r\n"),
            Node::atom(""),
        ])
    }

    #[test]
    fn typed_accessors() {
        let node = sample();
        assert_eq!(node[0].as_i64(), Some(12));
        assert_eq!(node[0].as_f64(), Some(12.0));
        assert_eq!(node.get_path(&[1, 0]).and_then(Node::as_quoted), Some("text"));
        assert_eq!(node.get_path(&[1, 0]).and_then(Node::as_atom), None);
        assert!(node.get_path(&[1, 1]).and_then(Node::as_guid).is_some());
        assert_eq!(node.get_path(&[1, 2]).and_then(Node::as_bool), Some(true));
        assert_eq!(node[2].as_base64().as_deref(), Some("QUJDREVG"));
        assert!(node[3].is_empty_atom());
        assert_eq!(node.get_path(&[1, 5]), None);
        assert_eq!(node.len(), 4);
        assert_eq!(node[0].iter().count(), 0);
    }

    #[test]
    fn try_accessors_explain_mismatch() {
        let node = sample();
        insta::assert_snapshot!(node.try_get(7).unwrap_err(), @"index 7 is out of bounds for list of length 4");
        insta::assert_snapshot!(node[0].try_list().unwrap_err(), @"expected list, found value `12`");
        insta::assert_snapshot!(node[1].try_i64().unwrap_err(), @"expected integer, found list of 3 items");
        insta::assert_snapshot!(node[3].try_guid().unwrap_err(), @"expected GUID, found empty value");
        insta::assert_snapshot!(node[1][0].try_bool().unwrap_err(), @r#"expected boolean (0 or 1), found string "text""#);
    }

    #[test]
    fn walks_with_paths() {
        let mut visited = Vec::new();
        let node = Node::from(vec![
            Node::atom("12"),
            Node::from(vec![Node::string("a"), Node::from(vec![Node::from(true)])]),
            Node::atom(""),
        ]);
        node.walk(|path, node| visited.push(format!("{path:?} {}", node.describe())));
        insta::assert_snapshot!(visited.join("\n"), @r#"
        [] list of 3 items
        [0] value `12`
        [1] list of 2 items
        [1, 0] string "a"
        [1, 1] list of 1 item
        [1, 1, 0] value `1`
        [2] empty value
        "#);
    }

    #[test]
    #[should_panic(expected = "cannot index node: expected list, found value `12`")]
    fn index_panics_with_message() {
        let _ = &sample()[0][0];
    }

    #[test]
    fn detects_guid() {
        assert!(is_guid("c2f34349-b1e0-4060-8abf-3c59f38ac69f"));
        assert!(!is_guid("c2f34349-b1e0-4060-8abf-3c59f38ac69"));
    }
}
