//! Parse and node access errors.

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

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
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

    #[error("expected {expected}, found {found}")]
    UnexpectedType {
        expected: &'static str,
        found: String,
    },

    #[error("index {index} is out of bounds for list of length {len}")]
    IndexOutOfBounds { index: usize, len: usize },
}

impl Error {
    /// Position in the input, if the error occurred while parsing.
    pub fn position(&self) -> Option<Position> {
        match self {
            Error::UnclosedList { at, .. }
            | Error::UnexpectedChar { at, .. }
            | Error::UnmatchedClose { at }
            | Error::TrailingData { at }
            | Error::InvalidUtf8 { at, .. } => Some(*at),
            Error::UnclosedString { start } => Some(*start),
            _ => None,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
