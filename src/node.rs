//! Object model of the bracket format.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Index, IndexMut};
use std::str::FromStr;

use crate::{AccessError, AccessErrorKind, Position};

const BASE64: &str = "#base64:";

/// A node of a bracket file: a [`Value`] plus the position it was parsed from.
///
/// Quoted strings and atoms are kept distinct, so writing reproduces
/// the original platform text byte for byte. Equality and hashing ignore the position.
#[derive(Clone)]
pub struct Node {
    value: Value,
    position: Option<Position>,
}

/// Content of a [`Node`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    /// A list `{a,b,c}`.
    List(Vec<Node>),
    /// A quoted string; doubled quotes are already unescaped.
    Str(String),
    /// An unquoted value: number, GUID, literal; may be empty (`{2,x,}`).
    /// Binary data `{#base64:…}` is an atom too, line breaks included.
    Atom(String),
}

impl Node {
    pub(crate) fn parsed(value: Value, position: Position) -> Self {
        Self {
            value,
            position: Some(position),
        }
    }

    /// A list of nodes.
    pub fn list(items: impl IntoIterator<Item = Node>) -> Self {
        Value::List(items.into_iter().collect()).into()
    }

    /// An unquoted atom (GUID, literal, number).
    pub fn atom(value: impl Into<String>) -> Self {
        Value::Atom(value.into()).into()
    }

    /// A quoted string.
    ///
    /// There is no `From<&str>` for building: `Node::try_from(&str)` parses text.
    pub fn string(value: impl Into<String>) -> Self {
        Value::Str(value.into()).into()
    }

    /// A counted list `{count,item1,item2,...}`, the common 1C shape.
    pub fn counted(items: impl IntoIterator<Item = Node>) -> Self {
        let items: Vec<Node> = items.into_iter().collect();
        Node::list(std::iter::once(Node::from(items.len())).chain(items))
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }

    /// Mutable access to the content; the position stays where the node was parsed.
    pub fn value_mut(&mut self) -> &mut Value {
        &mut self.value
    }

    /// Where the node starts in the input; `None` for nodes built in code.
    ///
    /// Edits do not change it: it always refers to the original text.
    pub fn position(&self) -> Option<Position> {
        self.position
    }

    pub fn is_list(&self) -> bool {
        matches!(self.value, Value::List(_))
    }

    pub fn is_str(&self) -> bool {
        matches!(self.value, Value::Str(_))
    }

    pub fn is_atom(&self) -> bool {
        matches!(self.value, Value::Atom(_))
    }

    /// An empty atom: no value between commas (`{1,,2}`).
    pub fn is_empty_atom(&self) -> bool {
        matches!(&self.value, Value::Atom(s) if s.is_empty())
    }

    pub fn as_list(&self) -> Option<&[Node]> {
        match &self.value {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    /// Value of a string or an atom.
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) | Value::Atom(s) => Some(s),
            Value::List(_) => None,
        }
    }

    /// Quoted string only.
    pub fn as_quoted(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Atom only.
    pub fn as_atom(&self) -> Option<&str> {
        match &self.value {
            Value::Atom(s) => Some(s),
            _ => None,
        }
    }

    /// Atom parsed into any [`FromStr`] type: `node.as_parsed::<u32>()`.
    pub fn as_parsed<T: FromStr>(&self) -> Option<T> {
        self.as_atom()?.parse().ok()
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.as_parsed()
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_parsed()
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.as_parsed()
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
        match self.value {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    /// Value of a string or an atom.
    pub fn into_string(self) -> Option<String> {
        match self.value {
            Value::Str(s) | Value::Atom(s) => Some(s),
            Value::List(_) => None,
        }
    }

    pub fn try_list(&self) -> Result<&[Node], AccessError> {
        self.as_list().ok_or_else(|| self.unexpected("list"))
    }

    pub fn try_str(&self) -> Result<&str, AccessError> {
        self.as_str()
            .ok_or_else(|| self.unexpected("string or value"))
    }

    pub fn try_quoted(&self) -> Result<&str, AccessError> {
        self.as_quoted()
            .ok_or_else(|| self.unexpected("quoted string"))
    }

    pub fn try_atom(&self) -> Result<&str, AccessError> {
        self.as_atom().ok_or_else(|| self.unexpected("value"))
    }

    /// Atom parsed into any [`FromStr`] type: `node.try_parse::<u32>()?`.
    ///
    /// The error names the target type and keeps the reason from [`FromStr::Err`].
    pub fn try_parse<T>(&self) -> Result<T, AccessError>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let expected = type_label::<T>();
        let atom = self
            .as_atom()
            .ok_or_else(|| self.unexpected(expected.clone()))?;
        atom.parse().map_err(|e: T::Err| {
            AccessError::new(
                AccessErrorKind::InvalidValue {
                    expected,
                    found: self.describe(),
                    reason: e.to_string(),
                },
                self.position,
            )
        })
    }

    pub fn try_i64(&self) -> Result<i64, AccessError> {
        self.try_parse()
    }

    pub fn try_u64(&self) -> Result<u64, AccessError> {
        self.try_parse()
    }

    pub fn try_f64(&self) -> Result<f64, AccessError> {
        self.try_parse()
    }

    pub fn try_bool(&self) -> Result<bool, AccessError> {
        self.as_bool()
            .ok_or_else(|| self.unexpected("boolean (0 or 1)"))
    }

    pub fn try_guid(&self) -> Result<&str, AccessError> {
        self.as_guid().ok_or_else(|| self.unexpected("GUID"))
    }

    pub fn try_get(&self, idx: usize) -> Result<&Node, AccessError> {
        let items = self.try_list()?;
        items.get(idx).ok_or_else(|| {
            AccessError::new(
                AccessErrorKind::IndexOutOfBounds {
                    index: idx,
                    len: items.len(),
                },
                self.position,
            )
        })
    }

    /// Item by a path of indices: `node.try_get_path(&[0, 2, 0])?`.
    ///
    /// On failure the error carries the path to the node that could not be indexed.
    pub fn try_get_path(&self, path: &[usize]) -> Result<&Node, AccessError> {
        let mut node = self;
        for (depth, &idx) in path.iter().enumerate() {
            node = node.try_get(idx).map_err(|e| e.with_path(&path[..depth]))?;
        }
        Ok(node)
    }

    /// List items for editing: replace, push, insert or remove nodes.
    pub fn as_list_mut(&mut self) -> Option<&mut Vec<Node>> {
        match &mut self.value {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut Node> {
        self.as_list_mut()?.get_mut(idx)
    }

    pub fn get_path_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        path.iter().try_fold(self, |node, &idx| node.get_mut(idx))
    }

    /// List items; an empty iterator for strings and atoms.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Node> {
        match &mut self.value {
            Value::List(items) => items.iter_mut(),
            _ => [].iter_mut(),
        }
    }

    pub fn try_list_mut(&mut self) -> Result<&mut Vec<Node>, AccessError> {
        if !self.is_list() {
            return Err(self.unexpected("list"));
        }
        Ok(self.as_list_mut().expect("checked above"))
    }

    pub fn try_get_mut(&mut self, idx: usize) -> Result<&mut Node, AccessError> {
        let position = self.position;
        let items = self.try_list_mut()?;
        let len = items.len();
        items.get_mut(idx).ok_or_else(|| {
            AccessError::new(
                AccessErrorKind::IndexOutOfBounds { index: idx, len },
                position,
            )
        })
    }

    /// Item by a path of indices for editing: `*node.try_get_path_mut(&[1, 0])? = Node::from(2)`.
    pub fn try_get_path_mut(&mut self, path: &[usize]) -> Result<&mut Node, AccessError> {
        let mut node = self;
        for (depth, &idx) in path.iter().enumerate() {
            node = node
                .try_get_mut(idx)
                .map_err(|e| e.with_path(&path[..depth]))?;
        }
        Ok(node)
    }

    /// Items of a counted list `{count,item1,item2,...}`: `{2,a,b}` gives `[a, b]`.
    ///
    /// Fails if the count is not a number or does not match the number of items.
    pub fn try_counted(&self) -> Result<&[Node], AccessError> {
        let declared: usize = self
            .try_get(0)?
            .try_parse()
            .map_err(|e| e.with_path(&[0]))?;
        let items = &self.try_list()?[1..];
        if items.len() != declared {
            return Err(AccessError::new(
                AccessErrorKind::CountMismatch {
                    declared,
                    actual: items.len(),
                },
                self.position,
            ));
        }
        Ok(items)
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
        match &self.value {
            Value::List(items) => match items.len() {
                0 => "empty list".to_owned(),
                1 => "list of 1 item".to_owned(),
                n => format!("list of {n} items"),
            },
            Value::Str(s) => format!("string {:?}", clip(s)),
            Value::Atom(s) if s.is_empty() => "empty value".to_owned(),
            Value::Atom(s) => format!("value `{}`", clip(s)),
        }
    }

    pub(crate) fn unexpected(&self, expected: impl Into<Cow<'static, str>>) -> AccessError {
        AccessError::new(
            AccessErrorKind::UnexpectedType {
                expected: expected.into(),
                found: self.describe(),
            },
            self.position,
        )
    }
}

/// `u32` for primitives, `Foo` instead of `my_crate::model::Foo` for user types.
pub(crate) fn type_label<T>() -> Cow<'static, str> {
    let name = std::any::type_name::<T>();
    if name.contains('<') {
        Cow::Borrowed(name)
    } else {
        Cow::Borrowed(name.rsplit("::").next().unwrap_or(name))
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

impl fmt::Debug for Node {
    /// Shows only the value: positions would drown the tree in noise.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for Node {}

impl Hash for Node {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
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

impl IndexMut<usize> for Node {
    fn index_mut(&mut self, idx: usize) -> &mut Node {
        match self.try_get_mut(idx) {
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

impl<'a> IntoIterator for &'a mut Node {
    type Item = &'a mut Node;
    type IntoIter = std::slice::IterMut<'a, Node>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl From<Value> for Node {
    fn from(value: Value) -> Self {
        Self {
            value,
            position: None,
        }
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
                Node::atom(n.to_string())
            }
        }
    )*};
}

from_number!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

impl From<Vec<Node>> for Node {
    fn from(items: Vec<Node>) -> Self {
        Value::List(items).into()
    }
}

impl<T: Into<Node>> FromIterator<T> for Node {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Node::list(iter.into_iter().map(Into::into))
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
        assert_eq!(node[0].as_parsed::<u8>(), Some(12));
        assert_eq!(
            node.get_path(&[1, 0]).and_then(Node::as_quoted),
            Some("text")
        );
        assert_eq!(node.get_path(&[1, 0]).and_then(Node::as_atom), None);
        assert!(node.get_path(&[1, 1]).and_then(Node::as_guid).is_some());
        assert_eq!(node.get_path(&[1, 2]).and_then(Node::as_bool), Some(true));
        assert_eq!(node[2].as_base64().as_deref(), Some("QUJDREVG"));
        assert!(node[3].is_empty_atom());
        assert_eq!(node.get_path(&[1, 5]), None);
        assert_eq!(node.len(), 4);
        assert_eq!(node[0].iter().count(), 0);
        assert_eq!(node[0].position(), None);
    }

    #[test]
    fn try_accessors_explain_mismatch() {
        let node = sample();
        insta::assert_snapshot!(node.try_get(7).unwrap_err(), @"index 7 is out of bounds for list of length 4");
        insta::assert_snapshot!(node[0].try_list().unwrap_err(), @"expected list, found value `12`");
        insta::assert_snapshot!(node[1].try_i64().unwrap_err(), @"expected i64, found list of 3 items");
        insta::assert_snapshot!(node[3].try_guid().unwrap_err(), @"expected GUID, found empty value");
        insta::assert_snapshot!(node[1][0].try_bool().unwrap_err(), @r#"expected boolean (0 or 1), found string "text""#);
        insta::assert_snapshot!(node[1][1].try_parse::<u32>().unwrap_err(), @"expected u32, found value `c2f34349-b1e0-4060-8abf-3c59f38ac69f`: invalid digit found in string");
        insta::assert_snapshot!(node.try_get_path(&[1, 0, 3]).unwrap_err(), @r#"expected list, found string "text" (path [1, 0])"#);
        insta::assert_snapshot!(node.try_get_path(&[1, 9]).unwrap_err(), @"index 9 is out of bounds for list of length 3 (path [1])");
    }

    #[test]
    fn parsed_nodes_report_positions() {
        let node: Node = "{1,\r\n{x,\"s\"}}".parse().unwrap();
        insta::assert_snapshot!(node.try_get_path(&[1, 0]).unwrap().try_parse::<u8>().unwrap_err(), @"expected u8, found value `x` at line 2, column 2: invalid digit found in string");
        insta::assert_snapshot!(node.try_get_path(&[1, 1, 0]).unwrap_err(), @r#"expected list, found string "s" at line 2, column 4 (path [1, 1])"#);
        insta::assert_snapshot!(node[1].try_get(5).unwrap_err(), @"index 5 is out of bounds for list of length 2 at line 2, column 1");
    }

    #[test]
    fn equality_ignores_positions() {
        let parsed: Node = "{1,\"a\"}".parse().unwrap();
        assert_eq!(parsed, Node::list([Node::from(1), Node::string("a")]));
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
    fn type_labels_are_short() {
        struct Local;
        assert_eq!(type_label::<u32>(), "u32");
        assert_eq!(type_label::<Local>(), "Local");
        assert_eq!(type_label::<String>(), "String");
    }

    #[test]
    fn edits_in_place() {
        let mut node: Node = "{1,\r\n{a,\"b\"}}".parse().unwrap();
        node[0] = Node::from(2);
        *node.try_get_path_mut(&[1, 1]).unwrap().value_mut() = Value::Str("c".into());
        node.get_mut(1)
            .unwrap()
            .try_list_mut()
            .unwrap()
            .push(Node::atom("d"));
        for item in &mut node[1] {
            if item.as_atom() == Some("a") {
                *item = Node::atom("z");
            }
        }
        assert_eq!(node.to_string(), "{2,\r\n{z,\"c\",d}\r\n}");
        // The edited value keeps the position of the original node.
        assert_eq!(
            node.get_path(&[1, 1]).unwrap().position().unwrap().column,
            4
        );
        assert!(node.get_path_mut(&[0, 0]).is_none());
        assert_eq!(node[0].iter_mut().count(), 0);

        insta::assert_snapshot!(node.try_get_path_mut(&[1, 7]).unwrap_err(), @"index 7 is out of bounds for list of length 3 at line 2, column 1 (path [1])");
        insta::assert_snapshot!(node[0].try_list_mut().unwrap_err(), @"expected list, found value `2`");
    }

    #[test]
    #[should_panic(expected = "cannot index node: index 5 is out of bounds for list of length 2")]
    fn index_mut_panics_with_message() {
        let mut node = Node::list([Node::from(1), Node::from(2)]);
        node[5] = Node::from(3);
    }

    #[test]
    fn counted_lists() {
        let node: Node = "{2,a,\r\n{b}}".parse().unwrap();
        assert_eq!(
            node.try_counted().unwrap(),
            [Node::atom("a"), Node::list([Node::atom("b")])]
        );
        assert_eq!(Node::counted(node.try_counted().unwrap().to_vec()), node);
        assert_eq!(Node::counted([]).to_string(), "{0}");

        let bad: Node = "{\r\n{x},\r\n{3,a},\r\n{}}".parse().unwrap();
        insta::assert_snapshot!(bad[0].try_counted().unwrap_err(), @"expected usize, found value `x` at line 2, column 2 (path [0]): invalid digit found in string");
        insta::assert_snapshot!(bad[1].try_counted().unwrap_err(), @"list declares 3 items, but has 1 at line 3, column 1");
        insta::assert_snapshot!(bad[2].try_counted().unwrap_err(), @"index 0 is out of bounds for list of length 0 at line 4, column 1");
    }

    #[test]
    fn detects_guid() {
        assert!(is_guid("c2f34349-b1e0-4060-8abf-3c59f38ac69f"));
        assert!(!is_guid("c2f34349-b1e0-4060-8abf-3c59f38ac69"));
    }
}
