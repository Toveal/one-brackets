//! Serialization of user types into a [`Node`] tree, the mirror of [`de`](crate::de):
//! fields become list items in declaration order, strings are quoted
//! (see [`atom`](crate::atom) for unquoted ones), `None` and `()` are empty atoms,
//! enum variants are written by index.

use serde::ser::{self, Serialize};

use crate::serde_impls::{ATOM, REST, VALUE};
use crate::{Node, SerializeError, Value};

/// Serializes a user type into a node tree.
pub fn to_node<T: Serialize + ?Sized>(value: &T) -> Result<Node, SerializeError> {
    Ok(match value.serialize(NodeSerializer)? {
        Item::One(node) => node,
        Item::Splice(items) => Node::list(items),
    })
}

/// Serializes a user type into bracket text in the platform layout.
pub fn to_string<T: Serialize + ?Sized>(value: &T) -> Result<String, SerializeError> {
    to_node(value).map(|node| node.to_string())
}

/// A [`Rest`](crate::Rest) is spliced into the enclosing list instead of nesting.
enum Item {
    One(Node),
    Splice(Vec<Node>),
}

fn one(node: Node) -> Result<Item, SerializeError> {
    Ok(Item::One(node))
}

struct NodeSerializer;

macro_rules! serialize_display {
    ($($method:ident: $t:ty),* $(,)?) => {$(
        fn $method(self, v: $t) -> Result<Item, SerializeError> {
            one(Node::atom(v.to_string()))
        }
    )*};
}

impl ser::Serializer for NodeSerializer {
    type Ok = Item;
    type Error = SerializeError;
    type SerializeSeq = List;
    type SerializeTuple = List;
    type SerializeTupleStruct = List;
    type SerializeTupleVariant = List;
    type SerializeMap = Map;
    type SerializeStruct = List;
    type SerializeStructVariant = List;

    fn serialize_bool(self, v: bool) -> Result<Item, SerializeError> {
        one(Node::from(v))
    }

    serialize_display! {
        serialize_i8: i8,
        serialize_i16: i16,
        serialize_i32: i32,
        serialize_i64: i64,
        serialize_i128: i128,
        serialize_u8: u8,
        serialize_u16: u16,
        serialize_u32: u32,
        serialize_u64: u64,
        serialize_u128: u128,
        serialize_f32: f32,
        serialize_f64: f64,
    }

    fn serialize_char(self, v: char) -> Result<Item, SerializeError> {
        one(Node::string(v))
    }

    fn serialize_str(self, v: &str) -> Result<Item, SerializeError> {
        one(Node::string(v))
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Item, SerializeError> {
        Err(SerializeError::new(
            "byte arrays are not supported: write binary data as a `#base64:` atom",
        ))
    }

    fn serialize_none(self) -> Result<Item, SerializeError> {
        one(Node::atom(""))
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Item, SerializeError> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Item, SerializeError> {
        one(Node::atom(""))
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Item, SerializeError> {
        one(Node::atom(""))
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
    ) -> Result<Item, SerializeError> {
        one(Node::from(index))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Item, SerializeError> {
        match name {
            REST => match to_node(value)?.into_list() {
                Some(items) => Ok(Item::Splice(items)),
                None => Err(SerializeError::new("a rest must serialize as a sequence")),
            },
            ATOM => match to_node(value)?.into_value() {
                Value::Str(s) | Value::Atom(s) => one(Node::atom(s)),
                Value::List(_) => Err(SerializeError::new("an atom must serialize as a string")),
            },
            _ => value.serialize(self),
        }
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<Item, SerializeError> {
        if name == VALUE {
            return value.serialize(self);
        }
        one(Node::list([Node::from(index), to_node(value)?]))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<List, SerializeError> {
        Ok(List::with_capacity(len.unwrap_or(0)))
    }

    fn serialize_tuple(self, len: usize) -> Result<List, SerializeError> {
        Ok(List::with_capacity(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<List, SerializeError> {
        Ok(List::with_capacity(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
        len: usize,
    ) -> Result<List, SerializeError> {
        Ok(List::tagged(index, len))
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Map, SerializeError> {
        Ok(Map {
            items: Vec::with_capacity(len.unwrap_or(0)),
            key: None,
        })
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<List, SerializeError> {
        Ok(List::with_capacity(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
        len: usize,
    ) -> Result<List, SerializeError> {
        Ok(List::tagged(index, len))
    }
}

struct List {
    items: Vec<Node>,
}

impl List {
    fn with_capacity(len: usize) -> Self {
        List {
            items: Vec::with_capacity(len),
        }
    }

    fn tagged(index: u32, len: usize) -> Self {
        let mut list = List::with_capacity(len + 1);
        list.items.push(Node::from(index));
        list
    }

    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerializeError> {
        match value.serialize(NodeSerializer)? {
            Item::One(node) => self.items.push(node),
            Item::Splice(items) => self.items.extend(items),
        }
        Ok(())
    }

    fn finish(self) -> Result<Item, SerializeError> {
        one(Node::list(self.items))
    }
}

macro_rules! impl_list {
    ($($trait:ident :: $method:ident($($key:ident: $key_ty:ty)?)),* $(,)?) => {$(
        impl ser::$trait for List {
            type Ok = Item;
            type Error = SerializeError;

            fn $method<T: Serialize + ?Sized>(
                &mut self,
                $($key: $key_ty,)?
                value: &T,
            ) -> Result<(), SerializeError> {
                self.push(value)
            }

            fn end(self) -> Result<Item, SerializeError> {
                self.finish()
            }
        }
    )*};
}

impl_list! {
    SerializeSeq::serialize_element(),
    SerializeTuple::serialize_element(),
    SerializeTupleStruct::serialize_field(),
    SerializeTupleVariant::serialize_field(),
}

impl ser::SerializeStruct for List {
    type Ok = Item;
    type Error = SerializeError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), SerializeError> {
        self.push(value)
    }

    /// `skip_serializing_if` must not shift the following fields: an empty atom keeps the slot.
    fn skip_field(&mut self, _key: &'static str) -> Result<(), SerializeError> {
        self.items.push(Node::atom(""));
        Ok(())
    }

    fn end(self) -> Result<Item, SerializeError> {
        self.finish()
    }
}

impl ser::SerializeStructVariant for List {
    type Ok = Item;
    type Error = SerializeError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), SerializeError> {
        self.push(value)
    }

    fn skip_field(&mut self, _key: &'static str) -> Result<(), SerializeError> {
        self.items.push(Node::atom(""));
        Ok(())
    }

    fn end(self) -> Result<Item, SerializeError> {
        self.finish()
    }
}

/// Written as a list of `{key,value}` pairs.
struct Map {
    items: Vec<Node>,
    key: Option<Node>,
}

impl ser::SerializeMap for Map {
    type Ok = Item;
    type Error = SerializeError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), SerializeError> {
        self.key = Some(to_node(key)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), SerializeError> {
        let key = self
            .key
            .take()
            .ok_or_else(|| SerializeError::new("map value written before its key"))?;
        self.items.push(Node::list([key, to_node(value)?]));
        Ok(())
    }

    fn end(self) -> Result<Item, SerializeError> {
        one(Node::list(self.items))
    }
}
