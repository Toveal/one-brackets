//! Deserialization of a [`Node`] tree into user types.
//!
//! Mapping:
//! - struct, tuple → list, field by position; extra items are ignored (see [`Tail`](crate::Tail));
//! - `Vec<T>` → list; map → list of `{key,value}` pairs;
//! - numbers, `bool` (`0`/`1`) → atoms; `String` → string or atom;
//! - `Option<T>` → `None` for an empty atom;
//! - enum → atom with the variant index or name, or a list `{variant,data...}`.
//!
//! Errors carry the position of the offending node and the path to it.

use std::cell::RefCell;
use std::io::Read;

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, Visitor};
use serde::{Deserialize, forward_to_deserialize_any};

use crate::serde_impls::{NODE, TAIL, VALUE};
use crate::{AccessError, Node, Value};

/// Deserializes a user type from a node; `&str` fields can borrow from the node.
///
/// ```
/// use one_brackets::Node;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, PartialEq, Deserialize, Serialize)]
/// struct Root {
///     version: u32,
///     #[serde(with = "one_brackets::atom")]
///     object: String,
///     extra: Option<u32>,
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let node: Node = "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,}".parse()?;
/// let root: Root = one_brackets::from_node(&node)?;
/// assert_eq!(root.version, 2);
/// assert_eq!(root.extra, None);
/// assert_eq!(one_brackets::to_string(&root)?, "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,}");
///
/// let err = one_brackets::from_str::<Root>("{2,\r\n{}}").unwrap_err();
/// assert_eq!(err.to_string(), "expected string or value, found empty list at line 2, column 1 (path [1])");
/// # Ok(())
/// # }
/// ```
pub fn from_node<'de, T: Deserialize<'de>>(node: &'de Node) -> Result<T, AccessError> {
    T::deserialize(NodeDeserializer { node })
}

/// Parses bracket text and deserializes a user type from it.
pub fn from_str<T: DeserializeOwned>(text: &str) -> crate::Result<T> {
    let node: Node = text.parse()?;
    Ok(from_node(&node)?)
}

/// Reads bracket text from any [`Read`] and deserializes a user type from it.
pub fn from_reader<T: DeserializeOwned, R: Read>(reader: R) -> crate::Result<T> {
    let node = Node::from_reader(reader)?;
    Ok(from_node(&node)?)
}

thread_local! {
    /// Hands a node to `Node::deserialize` as is, positions included:
    /// the serde data model has no way to carry it.
    static SMUGGLED: RefCell<Option<Node>> = const { RefCell::new(None) };
}

pub(crate) fn take_smuggled_node() -> Option<Node> {
    SMUGGLED.with(|slot| slot.borrow_mut().take())
}

#[derive(Clone, Copy)]
struct NodeDeserializer<'de> {
    node: &'de Node,
}

impl NodeDeserializer<'_> {
    /// Points errors raised by visitors at this node.
    fn locate<T>(&self, result: Result<T, AccessError>) -> Result<T, AccessError> {
        result.map_err(|e| e.or_position(self.node.position()))
    }
}

macro_rules! deserialize_parsed {
    ($($method:ident => $visit:ident),* $(,)?) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
            let value = self.node.try_parse()?;
            self.locate(visitor.$visit(value))
        }
    )*};
}

impl<'de> de::Deserializer<'de> for NodeDeserializer<'de> {
    type Error = AccessError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let result = match self.node.value() {
            Value::List(items) => visitor.visit_seq(Seq::new(items)),
            Value::Atom(s) if s.is_empty() => visitor.visit_unit(),
            Value::Str(s) | Value::Atom(s) => visitor.visit_borrowed_str(s),
        };
        self.locate(result)
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let value = self.node.try_bool()?;
        self.locate(visitor.visit_bool(value))
    }

    deserialize_parsed! {
        deserialize_i8 => visit_i8,
        deserialize_i16 => visit_i16,
        deserialize_i32 => visit_i32,
        deserialize_i64 => visit_i64,
        deserialize_i128 => visit_i128,
        deserialize_u8 => visit_u8,
        deserialize_u16 => visit_u16,
        deserialize_u32 => visit_u32,
        deserialize_u64 => visit_u64,
        deserialize_u128 => visit_u128,
        deserialize_f32 => visit_f32,
        deserialize_f64 => visit_f64,
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let value = self.node.try_str()?;
        self.locate(visitor.visit_borrowed_str(value))
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let value = self.node.try_str()?;
        self.locate(visitor.visit_borrowed_bytes(value.as_bytes()))
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let result = if self.node.is_empty_atom() {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        };
        self.locate(result)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        if !self.node.is_empty_atom() {
            return Err(self.node.unexpected("empty value"));
        }
        self.locate(visitor.visit_unit())
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        if name == NODE {
            SMUGGLED.with(|slot| *slot.borrow_mut() = Some(self.node.clone()));
            let result = visitor.visit_newtype_struct(self);
            take_smuggled_node();
            return self.locate(result);
        }
        self.locate(visitor.visit_newtype_struct(self))
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let items = self.node.try_list()?;
        self.locate(visitor.visit_seq(Seq::new(items)))
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        let items = self.node.try_list()?;
        self.locate(visitor.visit_seq(Seq::new(items).limit(len)))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        let items = self.node.try_list()?;
        self.locate(visitor.visit_map(Map {
            items,
            next: 0,
            value: None,
        }))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        self.deserialize_tuple(fields.len(), visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        if name == VALUE {
            return self.locate(visitor.visit_enum(ValueAccess(self.node)));
        }
        let variant = match self.node.value() {
            Value::List(items) => {
                let (tag, data) = items
                    .split_first()
                    .ok_or_else(|| self.node.unexpected("enum variant"))?;
                Variant {
                    tag,
                    data,
                    in_list: true,
                }
            }
            _ => Variant {
                tag: self.node,
                data: &[],
                in_list: false,
            },
        };
        self.locate(visitor.visit_enum(variant))
    }

    /// Numeric atoms select a variant or a field by index, others by name.
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        match self.node.as_atom() {
            Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                let index = self.node.try_parse()?;
                self.locate(visitor.visit_u64(index))
            }
            _ => self.deserialize_str(visitor),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        self.locate(visitor.visit_unit())
    }
}

/// List items in order. `end` limits a struct or tuple to its own fields.
struct Seq<'de> {
    items: &'de [Node],
    next: usize,
    end: usize,
    /// Index of `items[0]` in the parent list, for error paths.
    offset: usize,
    /// Item taken by the current element; `None` when a [`Tail`](crate::Tail) took the rest.
    taken: Option<usize>,
}

impl<'de> Seq<'de> {
    fn new(items: &'de [Node]) -> Self {
        Seq {
            items,
            next: 0,
            end: items.len(),
            offset: 0,
            taken: None,
        }
    }

    fn limit(mut self, len: usize) -> Self {
        self.end = self.end.min(len);
        self
    }

    fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}

impl<'de> de::SeqAccess<'de> for Seq<'de> {
    type Error = AccessError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, AccessError> {
        if self.next >= self.end {
            return Ok(None);
        }
        self.taken = None;
        seed.deserialize(Element { seq: self })
            .map(Some)
            .map_err(|e| match self.taken {
                Some(i) => e
                    .or_position(self.items[i].position())
                    .prepend_path(self.offset + i),
                None => e,
            })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.end.saturating_sub(self.next))
    }
}

/// The next list item; takes it only when its type is known, so a `Tail` can take the rest.
struct Element<'a, 'de> {
    seq: &'a mut Seq<'de>,
}

impl<'de> Element<'_, 'de> {
    fn take(self) -> NodeDeserializer<'de> {
        let items = self.seq.items;
        let index = self.seq.next;
        self.seq.next += 1;
        self.seq.taken = Some(index);
        NodeDeserializer {
            node: &items[index],
        }
    }
}

macro_rules! forward_to_node {
    ($($method:ident($($arg:ident: $ty:ty),*)),* $(,)?) => {$(
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, AccessError> {
            self.take().$method($($arg,)* visitor)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for Element<'_, 'de> {
    type Error = AccessError;

    forward_to_node! {
        deserialize_any(),
        deserialize_bool(),
        deserialize_i8(),
        deserialize_i16(),
        deserialize_i32(),
        deserialize_i64(),
        deserialize_i128(),
        deserialize_u8(),
        deserialize_u16(),
        deserialize_u32(),
        deserialize_u64(),
        deserialize_u128(),
        deserialize_f32(),
        deserialize_f64(),
        deserialize_char(),
        deserialize_str(),
        deserialize_string(),
        deserialize_bytes(),
        deserialize_byte_buf(),
        deserialize_option(),
        deserialize_unit(),
        deserialize_unit_struct(name: &'static str),
        deserialize_seq(),
        deserialize_tuple(len: usize),
        deserialize_tuple_struct(name: &'static str, len: usize),
        deserialize_map(),
        deserialize_struct(name: &'static str, fields: &'static [&'static str]),
        deserialize_enum(name: &'static str, variants: &'static [&'static str]),
        deserialize_identifier(),
        deserialize_ignored_any(),
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        if name != TAIL {
            return self.take().deserialize_newtype_struct(name, visitor);
        }
        let seq = self.seq;
        let start = seq.next;
        let rest = &seq.items[start..];
        seq.next = seq.items.len();
        visitor.visit_newtype_struct(TailDeserializer(Seq::new(rest).offset(seq.offset + start)))
    }
}

/// The rest of a list, as a sequence.
struct TailDeserializer<'de>(Seq<'de>);

impl<'de> de::Deserializer<'de> for TailDeserializer<'de> {
    type Error = AccessError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, AccessError> {
        visitor.visit_seq(self.0)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

/// A list of `{key,value}` pairs.
struct Map<'de> {
    items: &'de [Node],
    next: usize,
    value: Option<&'de Node>,
}

impl<'de> de::MapAccess<'de> for Map<'de> {
    type Error = AccessError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, AccessError> {
        let Some(entry) = self.items.get(self.next) else {
            return Ok(None);
        };
        let index = self.next;
        self.next += 1;
        let locate = |e: AccessError| e.or_position(entry.position()).prepend_path(index);
        let key = entry.try_get(0).map_err(locate)?;
        self.value = Some(entry.try_get(1).map_err(locate)?);
        seed.deserialize(NodeDeserializer { node: key })
            .map(Some)
            .map_err(|e| locate(e.or_position(key.position()).prepend_path(0)))
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, AccessError> {
        let value = self
            .value
            .take()
            .ok_or_else(|| de::Error::custom("map value requested before its key"))?;
        let index = self.next - 1;
        seed.deserialize(NodeDeserializer { node: value })
            .map_err(|e| {
                e.or_position(value.position())
                    .prepend_path(1)
                    .prepend_path(index)
            })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len() - self.next)
    }
}

/// A user enum: an atom with the variant, or a list `{variant,data...}`.
struct Variant<'de> {
    tag: &'de Node,
    data: &'de [Node],
    in_list: bool,
}

impl<'de> de::EnumAccess<'de> for Variant<'de> {
    type Error = AccessError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), AccessError> {
        let value = seed
            .deserialize(NodeDeserializer { node: self.tag })
            .map_err(|e| {
                let e = e.or_position(self.tag.position());
                if self.in_list { e.prepend_path(0) } else { e }
            })?;
        Ok((value, self))
    }
}

impl<'de> de::VariantAccess<'de> for Variant<'de> {
    type Error = AccessError;

    fn unit_variant(self) -> Result<(), AccessError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, AccessError> {
        let node = self.data.first().ok_or_else(|| {
            de::Error::custom("expected a list {variant,value} for a variant with data")
        })?;
        seed.deserialize(NodeDeserializer { node })
            .map_err(|e| e.or_position(node.position()).prepend_path(1))
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        visitor.visit_seq(Seq::new(self.data).limit(len).offset(1))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, AccessError> {
        self.tuple_variant(fields.len(), visitor)
    }
}

/// [`Value`] itself: the variant follows the node kind.
struct ValueAccess<'de>(&'de Node);

impl<'de> de::EnumAccess<'de> for ValueAccess<'de> {
    type Error = AccessError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), AccessError> {
        let index: u32 = match self.0.value() {
            Value::List(_) => 0,
            Value::Str(_) => 1,
            Value::Atom(_) => 2,
        };
        let value = seed.deserialize(IntoDeserializer::<AccessError>::into_deserializer(index))?;
        Ok((value, self))
    }
}

impl<'de> de::VariantAccess<'de> for ValueAccess<'de> {
    type Error = AccessError;

    fn unit_variant(self) -> Result<(), AccessError> {
        Err(de::Error::custom("bracket values are newtype variants"))
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, AccessError> {
        seed.deserialize(NodeDeserializer { node: self.0 })
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, AccessError> {
        Err(de::Error::custom("bracket values are newtype variants"))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, AccessError> {
        Err(de::Error::custom("bracket values are newtype variants"))
    }
}
