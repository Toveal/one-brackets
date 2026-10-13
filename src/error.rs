//! Errors: [`ParseError`] for broken input, [`AccessError`] for unexpected structure,
//! and [`Error`] that wraps both for code doing parsing and mapping in one go.

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// Position in the input.
///
/// `line` and `column` are 1-based; the column counts Unicode characters.
/// `offset` is the byte offset from the start of the input (including the BOM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    pub offset: u64,
    pub line: u64,
    pub column: u64,
}

impl Position {
    pub(crate) const START: Position = Position {
        offset: 0,
        line: 1,
        column: 1,
    };
}

impl Default for Position {
    fn default() -> Self {
        Self::START
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// The input could not be read or is not valid bracket text.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("I/O error while reading input: {0}")]
    Io(#[from] io::Error),

    #[error("failed to open file `{}`: {source}", path.display())]
    Open { path: PathBuf, source: io::Error },

    #[error("input is empty: expected a bracket value")]
    EmptyInput,

    #[error("unexpected end of input at {at}: list opened at {opened} is not closed")]
    UnclosedList { opened: Position, at: Position },

    #[error("unexpected end of input: string starting at {start} is not closed")]
    UnclosedString { start: Position },

    #[error("unexpected character {found:?} at {at}: expected ',' or '}}'")]
    UnexpectedChar { found: char, at: Position },

    #[error("unexpected '}}' at {at}: no list is open")]
    UnmatchedClose { at: Position },

    #[error("trailing data at {at}: only one top-level value is allowed")]
    TrailingData { at: Position },

    #[error("invalid UTF-8 in {context} starting at {at}")]
    InvalidUtf8 { context: &'static str, at: Position },
}

impl ParseError {
    /// Position in the input where the problem was found, if known.
    pub fn position(&self) -> Option<Position> {
        match self {
            ParseError::UnclosedList { at, .. }
            | ParseError::UnexpectedChar { at, .. }
            | ParseError::UnmatchedClose { at }
            | ParseError::TrailingData { at }
            | ParseError::InvalidUtf8 { at, .. } => Some(*at),
            ParseError::UnclosedString { start } => Some(*start),
            _ => None,
        }
    }
}

/// The text is valid, but a node does not have the expected shape.
///
/// Carries the position of the offending node (for parsed nodes) and,
/// for [`Node::try_get_path`](crate::Node::try_get_path), the path to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessError(Box<AccessErrorInner>);

/// Boxed to keep `Result<&Node, AccessError>` small on the happy path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AccessErrorInner {
    kind: AccessErrorKind,
    position: Option<Position>,
    path: Vec<usize>,
}

/// What exactly went wrong in an [`AccessError`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessErrorKind {
    /// The node is of another kind (list instead of value, string instead of atom...).
    UnexpectedType {
        expected: Cow<'static, str>,
        found: String,
    },
    /// The list is shorter than requested.
    IndexOutOfBounds { index: usize, len: usize },
    /// The value has the right kind but cannot be converted.
    InvalidValue {
        expected: Cow<'static, str>,
        found: String,
        reason: String,
    },
    /// A message from serde: missing items, unknown variants and the like.
    Message(String),
}

impl AccessError {
    pub(crate) fn new(kind: AccessErrorKind, position: Option<Position>) -> Self {
        Self(Box::new(AccessErrorInner {
            kind,
            position,
            path: Vec::new(),
        }))
    }

    pub(crate) fn with_path(mut self, path: &[usize]) -> Self {
        self.0.path = path.to_vec();
        self
    }

    /// Sets the position unless a more precise (inner) one is already known.
    #[cfg_attr(not(feature = "serde"), allow(dead_code))]
    pub(crate) fn or_position(mut self, position: Option<Position>) -> Self {
        if self.0.position.is_none() {
            self.0.position = position;
        }
        self
    }

    /// Adds a step to the path while the error bubbles up to the parent list.
    #[cfg_attr(not(feature = "serde"), allow(dead_code))]
    pub(crate) fn prepend_path(mut self, index: usize) -> Self {
        self.0.path.insert(0, index);
        self
    }

    pub fn kind(&self) -> &AccessErrorKind {
        &self.0.kind
    }

    /// Position of the offending node; `None` for nodes built in code.
    pub fn position(&self) -> Option<Position> {
        self.0.position
    }

    /// Path to the offending node relative to the node `try_get_path` was called on.
    pub fn path(&self) -> &[usize] {
        &self.0.path
    }
}

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = &self.0;
        match &inner.kind {
            AccessErrorKind::UnexpectedType { expected, found }
            | AccessErrorKind::InvalidValue {
                expected, found, ..
            } => write!(f, "expected {expected}, found {found}")?,
            AccessErrorKind::IndexOutOfBounds { index, len } => {
                write!(f, "index {index} is out of bounds for list of length {len}")?
            }
            AccessErrorKind::Message(message) => f.write_str(message)?,
        }
        if let Some(position) = inner.position {
            write!(f, " at {position}")?;
        }
        if !inner.path.is_empty() {
            write!(f, " (path {:?})", inner.path)?;
        }
        if let AccessErrorKind::InvalidValue { reason, .. } = &inner.kind {
            write!(f, ": {reason}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AccessError {}

#[cfg(feature = "serde")]
impl serde::de::Error for AccessError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        AccessError::new(AccessErrorKind::Message(msg.to_string()), None)
    }
}

/// A value cannot be represented in the bracket format.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SerializeError(String);

#[cfg(feature = "serde")]
impl SerializeError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[cfg(feature = "serde")]
impl serde::ser::Error for SerializeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

/// Either kind of error, for code that parses and maps in one function.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Access(#[from] AccessError),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
