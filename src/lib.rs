//! Reader and writer for the 1C:Enterprise bracket format: `{1,"text",{nested,list}}`.
//!
//! Input is anything implementing [`std::io::Read`]; the result is a [`Node`] tree.
//! The API is built on the standard conversion traits:
//!
//! ```
//! use one_brackets::Node;
//!
//! # fn main() -> one_brackets::Result<()> {
//! let node: Node = "\u{feff}{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,\"ок\"}".parse()?;
//! assert_eq!(node.try_get(0)?.try_i64()?, 2);
//! assert!(node[1].as_guid().is_some());
//! assert_eq!(node[2].as_quoted(), Some("ок"));
//!
//! // Writing uses the platform layout.
//! let text: String = (&node).into();
//! assert_eq!(text, "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,\"ок\"}");
//!
//! // Any `Read` source: file, socket, decompressed stream.
//! let node = Node::from_reader(std::io::Cursor::new(b"{1,{}}"))?;
//! assert_eq!(node, Node::list([Node::from(1), Node::list([])]));
//!
//! // Edit in place and write back: untouched parts keep their text.
//! let mut node: Node = "{1,\"old\",{2}}".parse()?;
//! *node.try_get_path_mut(&[2, 0])? = Node::from(3);
//! node[1] = Node::string("new");
//! assert_eq!(node.to_string(), "{1,\"new\",\r\n{3}\r\n}");
//!
//! // Errors carry a position.
//! let err = Node::try_from("{1,\n{2").unwrap_err();
//! assert_eq!(
//!     err.to_string(),
//!     "unexpected end of input at line 2, column 3: list opened at line 2, column 1 is not closed",
//! );
//! # Ok(())
//! # }
//! ```
//!
//! Map your own types with `impl TryFrom<&Node>` using the `try_*` accessors.
//! They return an [`AccessError`] that names the expected type and points at the node:
//!
//! ```
//! use one_brackets::{AccessError, Node};
//!
//! #[derive(Debug)]
//! struct FormatVersion { major: u32, build: u64 }
//!
//! impl TryFrom<&Node> for FormatVersion {
//!     type Error = AccessError;
//!
//!     fn try_from(node: &Node) -> Result<Self, AccessError> {
//!         Ok(FormatVersion {
//!             major: node.try_get_path(&[0, 0])?.try_parse()?,
//!             build: node.try_get_path(&[0, 2, 0])?.try_parse()?,
//!         })
//!     }
//! }
//!
//! let node: Node = "{\r\n{216,0,\r\n{x,0}\r\n}\r\n}".parse().unwrap();
//! let err = FormatVersion::try_from(&node).unwrap_err();
//! assert_eq!(
//!     err.to_string(),
//!     "expected u64, found value `x` at line 3, column 2: invalid digit found in string",
//! );
//! ```
//!
//! Parsing problems are reported as [`ParseError`], structure problems as [`AccessError`];
//! [`Error`] wraps both, so `?` works for either in one function.
//!
//! With the `serde` feature user types map to lists by field order:
//! see `from_node`, `to_node`, `Rest`, `Counted` and `atom`.
//!
//! # Performance
//!
//! Every string and value is a separate `String` and every list a `Vec`, so most of
//! the parsing time of large files (event logs, big forms) goes to the memory allocator.
//! The library does not pick an allocator; the application can switch to
//! [mimalloc](https://crates.io/crates/mimalloc), which works on Windows, Linux and macOS.
//! In the crate's measurements it made parsing 1.5–2.7 times faster on Windows,
//! whose system allocator is the slowest, and 1.2–1.5 times faster on Linux (glibc):
//!
//! ```toml
//! [dependencies]
//! mimalloc = "0.1"
//! ```
//!
//! ```ignore
//! #[global_allocator]
//! static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
//! ```
//!
//! For large inputs prefer [`Values`] over parsing one huge list: it keeps only
//! the current record in memory. `cargo bench` measures reading and writing speed.

#![cfg_attr(docsrs, feature(doc_cfg))]

/// Runs the README examples as doctests; some of them need the `serde` feature.
#[cfg(all(doctest, feature = "serde"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

/// The Russian README carries the same examples.
#[cfg(all(doctest, feature = "serde"))]
#[doc = include_str!("../README.ru.md")]
struct ReadmeRuDoctests;

#[cfg(feature = "serde")]
mod de;
mod error;
mod node;
mod parser;
#[cfg(feature = "serde")]
mod ser;
#[cfg(feature = "serde")]
mod serde_impls;
mod writer;

#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub use de::{from_node, from_reader, from_str};
#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub use error::SerializeError;
pub use error::{AccessError, AccessErrorKind, Error, ParseError, Position, Result};
pub use node::{Node, Value};
pub use parser::Values;
#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub use ser::{to_node, to_string};
#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub use serde_impls::{Counted, Rest, atom};
