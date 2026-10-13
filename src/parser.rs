//! Streaming reader of the bracket format from any [`Read`].

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::str::FromStr;

use crate::{Node, ParseError, Position, Value};

type Result<T> = std::result::Result<T, ParseError>;

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const BASE64: &[u8] = b"#base64:";

impl Node {
    /// Parses a single top-level value from any source.
    ///
    /// A leading BOM and surrounding whitespace are allowed. Reads byte by byte through a buffer
    /// without loading the whole input into memory.
    pub fn from_reader<R: Read>(reader: R) -> Result<Node> {
        parse(BufReader::new(reader))
    }
}

impl TryFrom<&str> for Node {
    type Error = ParseError;

    fn try_from(text: &str) -> Result<Node> {
        parse(text.as_bytes())
    }
}

impl TryFrom<String> for Node {
    type Error = ParseError;

    fn try_from(text: String) -> Result<Node> {
        parse(text.as_bytes())
    }
}

impl TryFrom<&[u8]> for Node {
    type Error = ParseError;

    fn try_from(bytes: &[u8]) -> Result<Node> {
        parse(bytes)
    }
}

impl TryFrom<Vec<u8>> for Node {
    type Error = ParseError;

    fn try_from(bytes: Vec<u8>) -> Result<Node> {
        parse(bytes.as_slice())
    }
}

impl TryFrom<File> for Node {
    type Error = ParseError;

    fn try_from(file: File) -> Result<Node> {
        Node::from_reader(file)
    }
}

impl TryFrom<&Path> for Node {
    type Error = ParseError;

    fn try_from(path: &Path) -> Result<Node> {
        let file = File::open(path).map_err(|source| ParseError::Open {
            path: path.to_owned(),
            source,
        })?;
        Node::from_reader(file)
    }
}

impl FromStr for Node {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Node> {
        parse(text.as_bytes())
    }
}

fn parse<R: BufRead>(reader: R) -> Result<Node> {
    let mut parser = Parser {
        input: Input::new(reader),
    };
    parser.input.skip_bom()?;
    parser.input.skip_ws()?;
    if parser.input.peek()?.is_none() {
        return Err(ParseError::EmptyInput);
    }
    let node = parser.value()?;
    parser.input.skip_ws()?;
    let at = parser.input.pos;
    match parser.input.peek()? {
        None => Ok(node),
        Some(b'}') => Err(ParseError::UnmatchedClose { at }),
        Some(_) => Err(ParseError::TrailingData { at }),
    }
}

/// Comma-separated top-level values read one at a time, e.g. records of an event log `.lgp`.
///
/// The input is streamed: only the current value is kept in memory. Text lines before
/// the values (a file header) can be read with [`read_line`](Values::read_line);
/// positions in errors and nodes still count from the start of the input.
/// A trailing comma is allowed: a log being written ends with one.
/// The iterator stops after the first error.
///
/// ```
/// use one_brackets::Values;
///
/// # fn main() -> Result<(), one_brackets::ParseError> {
/// let mut values = Values::new("HEADER\r\n{1,a},\r\n{2,b},".as_bytes());
/// assert_eq!(values.read_line()?.as_deref(), Some("HEADER"));
/// let records = values.collect::<Result<Vec<_>, _>>()?;
/// assert_eq!(records.len(), 2);
/// assert_eq!(records[1].position().unwrap().line, 3);
/// # Ok(())
/// # }
/// ```
pub struct Values<R> {
    parser: Parser<BufReader<R>>,
    state: State,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// The BOM has not been checked yet.
    Fresh,
    BeforeValue,
    AfterValue,
    Done,
}

impl<R: Read> Values<R> {
    pub fn new(reader: R) -> Self {
        Values {
            parser: Parser {
                input: Input::new(BufReader::new(reader)),
            },
            state: State::Fresh,
        }
    }

    /// Reads one text line before the values; `None` at the end of input.
    ///
    /// The line ending (`\n` or `\r\n`) is not included.
    pub fn read_line(&mut self) -> Result<Option<String>> {
        self.start()?;
        let input = &mut self.parser.input;
        let at = input.pos;
        if input.peek()?.is_none() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        if input.scan(|b| b == b'\n', Some(&mut bytes))?.is_some() {
            input.bump()?;
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| ParseError::InvalidUtf8 {
                context: "line",
                at,
            })
    }

    /// Position of the next unread byte.
    pub fn position(&self) -> Position {
        self.parser.input.pos
    }

    fn start(&mut self) -> Result<()> {
        if self.state == State::Fresh {
            self.parser.input.skip_bom()?;
            self.state = State::BeforeValue;
        }
        Ok(())
    }

    fn read_value(&mut self) -> Result<Option<Node>> {
        self.start()?;
        let input = &mut self.parser.input;
        input.skip_ws()?;
        if self.state == State::AfterValue {
            let at = input.pos;
            match input.peek()? {
                None => return Ok(None),
                Some(b',') => {
                    input.bump()?;
                    input.skip_ws()?;
                }
                Some(b'}') => return Err(ParseError::UnmatchedClose { at }),
                Some(_) => {
                    return Err(ParseError::UnexpectedChar {
                        found: self.parser.char_at_cursor()?,
                        at,
                    });
                }
            }
        }
        if self.parser.input.peek()?.is_none() {
            return Ok(None);
        }
        let node = self.parser.value()?;
        self.state = State::AfterValue;
        Ok(Some(node))
    }
}

impl<R: Read> Iterator for Values<R> {
    type Item = Result<Node>;

    fn next(&mut self) -> Option<Result<Node>> {
        if self.state == State::Done {
            return None;
        }
        let result = self.read_value();
        if !matches!(result, Ok(Some(_))) {
            self.state = State::Done;
        }
        result.transpose()
    }
}

impl<R: Read> std::iter::FusedIterator for Values<R> {}

/// Byte cursor over [`BufRead`] that tracks the position.
struct Input<R> {
    inner: R,
    /// Bytes read while checking for a BOM and pushed back.
    pending: Vec<u8>,
    pos: Position,
}

impl<R: BufRead> Input<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            pending: Vec::new(),
            pos: Position::START,
        }
    }

    fn peek_inner(&mut self) -> Result<Option<u8>> {
        loop {
            match self.inner.fill_buf() {
                Ok(buf) => return Ok(buf.first().copied()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn peek(&mut self) -> Result<Option<u8>> {
        match self.pending.first() {
            Some(&b) => Ok(Some(b)),
            None => self.peek_inner(),
        }
    }

    fn bump(&mut self) -> Result<Option<u8>> {
        let byte = if self.pending.is_empty() {
            let byte = self.peek_inner()?;
            if byte.is_some() {
                self.inner.consume(1);
            }
            byte
        } else {
            Some(self.pending.remove(0))
        };
        if let Some(b) = byte {
            advance(&mut self.pos, &[b]);
        }
        Ok(byte)
    }

    fn skip_bom(&mut self) -> Result<()> {
        let mut seen = Vec::new();
        for expected in BOM {
            match self.peek_inner()? {
                Some(b) if b == expected => {
                    self.inner.consume(1);
                    seen.push(b);
                }
                _ => break,
            }
        }
        if seen.len() == BOM.len() {
            self.pos.offset += BOM.len() as u64;
        } else {
            self.pending = seen;
        }
        Ok(())
    }

    fn skip_ws(&mut self) -> Result<()> {
        self.scan(|b| !b.is_ascii_whitespace(), None)?;
        Ok(())
    }

    /// Moves over bytes until `stop` matches, a whole buffer at a time, copying them to `out`.
    ///
    /// Returns the stop byte, which stays unread, or `None` at the end of input.
    fn scan(
        &mut self,
        stop: impl Fn(u8) -> bool,
        mut out: Option<&mut Vec<u8>>,
    ) -> Result<Option<u8>> {
        // Pushed-back bytes of a partial BOM: at most two, one by one.
        while let Some(&b) = self.pending.first() {
            if stop(b) {
                return Ok(Some(b));
            }
            self.bump()?;
            if let Some(out) = out.as_deref_mut() {
                out.push(b);
            }
        }
        loop {
            let buf = loop {
                match self.inner.fill_buf() {
                    Ok(buf) => break buf,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e.into()),
                }
            };
            if buf.is_empty() {
                return Ok(None);
            }
            let (len, found) = match buf.iter().position(|&b| stop(b)) {
                Some(i) => (i, Some(buf[i])),
                None => (buf.len(), None),
            };
            let chunk = &buf[..len];
            if let Some(out) = out.as_deref_mut() {
                out.extend_from_slice(chunk);
            }
            advance(&mut self.pos, chunk);
            self.inner.consume(len);
            if found.is_some() {
                return Ok(found);
            }
        }
    }
}

struct Parser<R> {
    input: Input<R>,
}

impl<R: BufRead> Parser<R> {
    fn value(&mut self) -> Result<Node> {
        match self.input.peek()? {
            Some(b'{') => self.list(),
            _ => self.scalar(),
        }
    }

    /// Lists are parsed without recursion: forms can be deeply nested.
    fn list(&mut self) -> Result<Node> {
        // (position of `{`, collected items)
        let mut stack: Vec<(Position, Vec<Node>)> = Vec::new();
        loop {
            // The cursor is always on `{` here.
            let opened = self.input.pos;
            self.input.bump()?;
            self.input.skip_ws()?;
            stack.push((opened, Vec::new()));
            match self.input.peek()? {
                // Empty list `{}`; the shared closing code follows.
                Some(b'}') => {}
                Some(b'{') => continue,
                _ => {
                    let item = self.scalar()?;
                    push(&mut stack, item);
                }
            }
            // Separators and closing brackets until a new nested list starts.
            loop {
                self.input.skip_ws()?;
                let at = self.input.pos;
                match self.input.peek()? {
                    Some(b',') => {
                        self.input.bump()?;
                        self.input.skip_ws()?;
                        if self.input.peek()? == Some(b'{') {
                            break;
                        }
                        let item = self.scalar()?;
                        push(&mut stack, item);
                    }
                    Some(b'}') => {
                        self.input.bump()?;
                        let (opened, items) = stack.pop().expect("list stack is not empty");
                        let done = Node::parsed(Value::List(items), opened);
                        match stack.last_mut() {
                            Some((_, parent)) => parent.push(done),
                            None => return Ok(done),
                        }
                    }
                    Some(_) => {
                        return Err(ParseError::UnexpectedChar {
                            found: self.char_at_cursor()?,
                            at,
                        });
                    }
                    None => {
                        let (opened, _) = stack.last().expect("list stack is not empty");
                        return Err(ParseError::UnclosedList {
                            opened: *opened,
                            at,
                        });
                    }
                }
            }
        }
    }

    fn scalar(&mut self) -> Result<Node> {
        if self.input.peek()? == Some(b'"') {
            self.string()
        } else {
            self.atom()
        }
    }

    fn string(&mut self) -> Result<Node> {
        let start = self.input.pos;
        self.input.bump()?;
        let mut bytes = Vec::new();
        loop {
            if self.input.scan(|b| b == b'"', Some(&mut bytes))?.is_none() {
                return Err(ParseError::UnclosedString { start });
            }
            self.input.bump()?;
            // A doubled quote is an escaped one; a single quote ends the string.
            if self.input.peek()? != Some(b'"') {
                break;
            }
            self.input.bump()?;
            bytes.push(b'"');
        }
        let value = String::from_utf8(bytes).map_err(|_| ParseError::InvalidUtf8 {
            context: "quoted string",
            at: start,
        })?;
        Ok(Node::parsed(Value::Str(value), start))
    }

    fn atom(&mut self) -> Result<Node> {
        let start = self.input.pos;
        let mut bytes = Vec::new();
        let delimiter = |b: u8| matches!(b, b',' | b'{' | b'}');
        let stop = self.input.scan(
            |b| delimiter(b) || b.is_ascii_whitespace(),
            Some(&mut bytes),
        )?;
        // Base64 is wrapped with `\r\r\n`: whitespace is part of the value.
        if stop.is_some_and(|b| b.is_ascii_whitespace()) && bytes.starts_with(BASE64) {
            self.input.scan(delimiter, Some(&mut bytes))?;
        }
        let value = String::from_utf8(bytes).map_err(|_| ParseError::InvalidUtf8 {
            context: "value",
            at: start,
        })?;
        Ok(Node::parsed(Value::Atom(value), start))
    }

    /// Character under the cursor (for error messages); may span several bytes.
    fn char_at_cursor(&mut self) -> Result<char> {
        let mut bytes = Vec::with_capacity(4);
        while let Some(b) = self.input.peek()? {
            if !bytes.is_empty() && b & 0xC0 != 0x80 || bytes.len() == 4 {
                break;
            }
            bytes.push(b);
            self.input.bump()?;
        }
        Ok(std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.chars().next())
            .unwrap_or(char::REPLACEMENT_CHARACTER))
    }
}

/// Moves the position over bytes already read.
fn advance(pos: &mut Position, bytes: &[u8]) {
    // UTF-8 continuation bytes do not advance the column.
    let chars = |bytes: &[u8]| bytes.iter().filter(|&&b| b & 0xC0 != 0x80).count() as u64;
    pos.offset += bytes.len() as u64;
    match bytes.iter().rposition(|&b| b == b'\n') {
        Some(last) => {
            pos.line += bytes.iter().filter(|&&b| b == b'\n').count() as u64;
            pos.column = 1 + chars(&bytes[last + 1..]);
        }
        None => pos.column += chars(bytes),
    }
}

fn push(stack: &mut [(Position, Vec<Node>)], item: Node) {
    stack
        .last_mut()
        .expect("list stack is not empty")
        .1
        .push(item);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(s: &str) -> Node {
        Node::atom(s)
    }

    /// Source that yields one byte per `read` call.
    struct ByteByByte<'a>(&'a [u8]);

    impl Read for ByteByByte<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match (self.0.split_first(), buf.first_mut()) {
                (Some((&b, rest)), Some(slot)) => {
                    *slot = b;
                    self.0 = rest;
                    Ok(1)
                }
                _ => Ok(0),
            }
        }
    }

    #[test]
    fn parses_values() {
        let node: Node = "\u{feff}{1,{\"a \"\"b\"\"\",x},{},}".parse().unwrap();
        assert_eq!(
            node,
            Node::list([
                atom("1"),
                Node::list([Node::string("a \"b\""), atom("x")]),
                Node::list([]),
                atom(""),
            ])
        );
    }

    #[test]
    fn keeps_base64_with_line_breaks() {
        let node = Node::try_from("{1,\r\n{#base64:QUJD\r\r\nREVG\r\r\n},2}").unwrap();
        assert_eq!(
            node.get_path(&[1, 0]).and_then(Node::as_str),
            Some("#base64:QUJD\r\r\nREVG\r\r\n")
        );
        assert_eq!(node[2].as_i64(), Some(2));
    }

    #[test]
    fn deep_nesting_without_recursion() {
        let depth = 5_000;
        let text = format!("{}{}", "{".repeat(depth), "}".repeat(depth));
        assert!(Node::try_from(text).is_ok());
    }

    #[test]
    fn reads_byte_by_byte() {
        let text = "\u{feff}{1,\r\n{\"ру\"\"с\",#base64:AB\r\r\nC},Ж,\"\"}";
        let expected = Node::try_from(text).unwrap();
        let node = Node::from_reader(ByteByByte(text.as_bytes())).unwrap();
        assert_eq!(node, expected);
    }

    #[test]
    fn partial_bom_is_part_of_value() {
        // U+F000 starts with byte 0xEF like a BOM: the bytes read are pushed back.
        let node = Node::try_from("\u{f000}").unwrap();
        assert_eq!(node, atom("\u{f000}"));
    }

    #[test]
    fn propagates_io_errors() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk is on fire"))
            }
        }
        let err = Node::from_reader(Failing).unwrap_err();
        assert!(matches!(err, ParseError::Io(_)));
        insta::assert_snapshot!(err, @"I/O error while reading input: disk is on fire");
    }

    #[test]
    fn rejects_broken_input() {
        assert!(Node::try_from("{1,2").is_err());
        assert!(Node::try_from("{\"abc}").is_err());
        assert!(Node::try_from("{1}}").is_err());
    }

    #[test]
    fn reads_value_sequences() {
        let text = "\u{feff}HEAD\r\n\r\n{1},\r\n{2,\"x\"},\r\nz,\r\n";
        let mut values = Values::new(ByteByByte(text.as_bytes()));
        assert_eq!(values.read_line().unwrap().as_deref(), Some("HEAD"));
        assert_eq!(values.read_line().unwrap().as_deref(), Some(""));
        let nodes: Vec<Node> = values.map(Result::unwrap).collect();
        assert_eq!(
            nodes,
            [
                Node::list([atom("1")]),
                Node::list([atom("2"), Node::string("x")]),
                atom("z")
            ]
        );
        assert_eq!(nodes[1].position().unwrap().line, 4);

        assert_eq!(Values::new("".as_bytes()).count(), 0);
        let mut values = Values::new("{1}\r\n{2}".as_bytes());
        assert!(values.next().unwrap().is_ok());
        insta::assert_snapshot!(values.next().unwrap().unwrap_err(), @"unexpected character '{' at line 2, column 1: expected ',' or '}'");
        assert!(values.next().is_none());
        let err = Values::new("{1},{2".as_bytes())
            .nth(1)
            .unwrap()
            .unwrap_err();
        insta::assert_snapshot!(err, @"unexpected end of input at line 1, column 7: list opened at line 1, column 5 is not closed");
    }

    #[test]
    fn reports_positions() {
        let err = Node::try_from("{1,\r\n{2,\r\n  3").unwrap_err();
        insta::assert_snapshot!(err, @"unexpected end of input at line 3, column 4: list opened at line 2, column 1 is not closed");
        let err = Node::try_from("{\"ёж\" x}").unwrap_err();
        insta::assert_snapshot!(err, @"unexpected character 'x' at line 1, column 7: expected ',' or '}'");
        assert_eq!(err.position().map(|p| p.offset), Some(8));
    }
}
