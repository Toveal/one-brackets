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
//! assert_eq!(node, Node::from(vec![Node::from(1), Node::List(vec![])]));
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
//! Map your own types with `impl TryFrom<&Node>` using the `try_*` accessors,
//! which return a descriptive [`Error::UnexpectedType`].

mod error;
mod node;
mod parser;
mod writer;

pub use error::{Error, Position, Result};
pub use node::Node;
