//! Serde support for the tree types and the helpers [`Rest`], [`Counted`] and [`atom`].
//!
//! The bracket format has no field names, only order, so the special types talk to
//! the crate's serializer and deserializer through reserved newtype and enum names.
//! Other formats see them as plain newtypes and enums.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use serde::de::{self, Deserializer, EnumAccess, VariantAccess, Visitor};
use serde::ser::{SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};

use crate::de::take_smuggled_node;
use crate::error::count_mismatch;
use crate::{Node, Value};

pub(crate) const NODE: &str = "$one_brackets::Node";
pub(crate) const VALUE: &str = "$one_brackets::Value";
pub(crate) const REST: &str = "$one_brackets::Rest";
pub(crate) const ATOM: &str = "$one_brackets::Atom";

const VARIANTS: &[&str] = &["List", "Str", "Atom"];

/// Remaining items of a list after the preceding fields, kept as is.
///
/// Lists map to structs by position, and items past the last field are ignored.
/// Put a `Rest` as the last field to keep them: it is filled from the rest of the list
/// and written back in place, so unknown items survive a read-write round trip.
/// Mark it `#[serde(default)]`: a list may have no extra items at all.
///
/// ```
/// use one_brackets::{Node, Rest};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize, Serialize)]
/// struct Version {
///     major: u32,
///     #[serde(default)]
///     rest: Rest,
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let node: Node = "{216,0,{80305,0}}".parse()?;
/// let version: Version = one_brackets::from_node(&node)?;
/// assert_eq!(version.major, 216);
/// assert_eq!(version.rest.len(), 2);
/// assert_eq!(one_brackets::to_node(&version)?, node);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Rest(pub Vec<Node>);

impl Deref for Rest {
    type Target = Vec<Node>;

    fn deref(&self) -> &Vec<Node> {
        &self.0
    }
}

impl DerefMut for Rest {
    fn deref_mut(&mut self) -> &mut Vec<Node> {
        &mut self.0
    }
}

impl From<Vec<Node>> for Rest {
    fn from(items: Vec<Node>) -> Self {
        Rest(items)
    }
}

impl Serialize for Rest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_newtype_struct(REST, &self.0)
    }
}

impl<'de> Deserialize<'de> for Rest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RestVisitor;

        impl<'de> Visitor<'de> for RestVisitor {
            type Value = Rest;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("remaining list items")
            }

            fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<Rest, D::Error> {
                Vec::deserialize(d).map(Rest)
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Rest, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Rest(items))
            }
        }

        deserializer.deserialize_newtype_struct(REST, RestVisitor)
    }
}

/// A counted list `{count,item1,item2,...}`, the common 1C shape.
///
/// The count is checked on reading and written on serialization.
/// For the [`Node`] API see [`Node::try_counted`] and [`Node::counted`].
///
/// ```
/// use one_brackets::Counted;
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize)]
/// struct Server(String, u16);
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let servers: Counted<Server> = one_brackets::from_str("{2,{\"a\",1541},{\"b\",1541}}")?;
/// assert_eq!(servers[1].0, "b");
///
/// let err = one_brackets::from_str::<Counted<Server>>("{3,{\"a\",1541}}").unwrap_err();
/// assert_eq!(err.to_string(), "list declares 3 items, but has 1 at line 1, column 1");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Counted<T>(pub Vec<T>);

impl<T> Default for Counted<T> {
    fn default() -> Self {
        Counted(Vec::new())
    }
}

impl<T> Deref for Counted<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T> DerefMut for Counted<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.0
    }
}

impl<T> From<Vec<T>> for Counted<T> {
    fn from(items: Vec<T>) -> Self {
        Counted(items)
    }
}

impl<T> FromIterator<T> for Counted<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Counted(iter.into_iter().collect())
    }
}

impl<T> IntoIterator for Counted<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Counted<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: Serialize> Serialize for Counted<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len() + 1))?;
        seq.serialize_element(&self.0.len())?;
        for item in &self.0 {
            seq.serialize_element(item)?;
        }
        seq.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Counted<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CountedVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for CountedVisitor<T> {
            type Value = Counted<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a counted list {count,items...}")
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Counted<T>, A::Error> {
                let declared: usize = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let mut items = Vec::with_capacity(declared.min(4096));
                while items.len() < declared {
                    match seq.next_element()? {
                        Some(item) => items.push(item),
                        None => return Err(mismatch(declared, items.len())),
                    }
                }
                let mut actual = declared;
                while seq.next_element::<de::IgnoredAny>()?.is_some() {
                    actual += 1;
                }
                if actual != declared {
                    return Err(mismatch(declared, actual));
                }
                Ok(Counted(items))
            }
        }

        fn mismatch<E: de::Error>(declared: usize, actual: usize) -> E {
            E::custom(count_mismatch(declared, actual))
        }

        deserializer.deserialize_seq(CountedVisitor(PhantomData))
    }
}

/// Field helper for `#[serde(with = "one_brackets::atom")]`.
///
/// Strings are written quoted by default. With this helper a field is written
/// as an unquoted atom (GUID, number, literal) via [`Display`](std::fmt::Display)
/// and read via [`FromStr`](std::str::FromStr).
pub mod atom {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::node::type_label;

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Display + ?Sized,
        S: Serializer,
    {
        serializer.serialize_newtype_struct(super::ATOM, &value.to_string())
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(|e| {
            D::Error::custom(format_args!(
                "expected {}, found value `{text}` ({e})",
                type_label::<T>()
            ))
        })
    }
}

/// Writes an atom: a plain string in other formats.
struct AtomStr<'a>(&'a str);

impl Serialize for AtomStr<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_newtype_struct(ATOM, self.0)
    }
}

/// Other formats see `{"List": [...]}`, `{"Str": "..."}` or `{"Atom": "..."}`.
impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::List(items) => serializer.serialize_newtype_variant(VALUE, 0, "List", items),
            Value::Str(s) => serializer.serialize_newtype_variant(VALUE, 1, "Str", s),
            Value::Atom(s) => serializer.serialize_newtype_variant(VALUE, 2, "Atom", &AtomStr(s)),
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_enum(VALUE, VARIANTS, ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a bracket value: List, Str or Atom")
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Value, A::Error> {
        let (kind, variant) = data.variant()?;
        match kind {
            Kind::List => variant.newtype_variant().map(Value::List),
            Kind::Str => variant.newtype_variant().map(Value::Str),
            Kind::Atom => variant.newtype_variant().map(Value::Atom),
        }
    }
}

enum Kind {
    List,
    Str,
    Atom,
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KindVisitor;

        impl Visitor<'_> for KindVisitor {
            type Value = Kind;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("List, Str or Atom")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Kind, E> {
                match v {
                    0 => Ok(Kind::List),
                    1 => Ok(Kind::Str),
                    2 => Ok(Kind::Atom),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(v), &self)),
                }
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Kind, E> {
                match v {
                    "List" => Ok(Kind::List),
                    "Str" => Ok(Kind::Str),
                    "Atom" => Ok(Kind::Atom),
                    _ => Err(E::unknown_variant(v, VARIANTS)),
                }
            }
        }

        deserializer.deserialize_identifier(KindVisitor)
    }
}

/// Positions are not written: they belong to the parsed text.
impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value().serialize(serializer)
    }
}

/// From this crate's deserializer a node comes out as is, positions included.
impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bracket node")
            }

            fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<Node, D::Error> {
                match take_smuggled_node() {
                    Some(node) => Ok(node),
                    None => Value::deserialize(d).map(Node::from),
                }
            }

            fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Node, A::Error> {
                ValueVisitor.visit_enum(data).map(Node::from)
            }
        }

        deserializer.deserialize_newtype_struct(NODE, NodeVisitor)
    }
}
