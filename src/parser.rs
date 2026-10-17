//! Streaming reader of the bracket format from any [`Read`].
//!
//! Follows the reader of the platform (8.3.27): whitespace is anything ICU `u_isspace` accepts,
//! an atom runs up to the next `,` or `}`, and a quoted string unescapes doubled quotes and
//! UTF-16 code units `"\xxxx`.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::str::FromStr;

use crate::node::is_space;
use crate::text::push_code_point;
use crate::{Node, ParseError, Position, Text, Value};

type Result<T> = std::result::Result<T, ParseError>;

impl Node {
    /// Parses a single top-level value from any source.
    ///
    /// Text is UTF-8; a leading BOM and surrounding whitespace are allowed. Reads through a buffer without loading the whole input into memory.
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

impl TryFrom<&[u8]> for Node {
    type Error = ParseError;

    fn try_from(bytes: &[u8]) -> Result<Node> {
        parse(bytes)
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
    let mut parser = Parser::new(reader);
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
            parser: Parser::new(BufReader::new(reader)),
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

/// `fill_buf` that retries on `Interrupted`.
fn fill<R: BufRead>(reader: &mut R) -> io::Result<&[u8]> {
    loop {
        match reader.fill_buf() {
            // Reborrowed to end the loop's borrow: a known limitation of the borrow checker.
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    reader.fill_buf()
}

/// Byte cursor over UTF-8 input that tracks the position.
struct Input<R> {
    inner: R,
    /// Bytes taken from `inner` to look ahead.
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

    fn skip_bom(&mut self) -> Result<()> {
        const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
        for (i, expected) in BOM.into_iter().enumerate() {
            if self.peek_ahead(i)? != Some(expected) {
                return Ok(());
            }
        }
        self.pending.clear();
        self.pos.offset = BOM.len() as u64;
        Ok(())
    }

    fn peek_inner(&mut self) -> Result<Option<u8>> {
        Ok(fill(&mut self.inner)?.first().copied())
    }

    fn peek(&mut self) -> Result<Option<u8>> {
        match self.pending.first() {
            Some(&b) => Ok(Some(b)),
            None => self.peek_inner(),
        }
    }

    /// Byte `n` positions after the cursor.
    fn peek_ahead(&mut self, n: usize) -> Result<Option<u8>> {
        while self.pending.len() <= n {
            let Some(b) = self.peek_inner()? else {
                return Ok(None);
            };
            self.inner.consume(1);
            self.pending.push(b);
        }
        Ok(Some(self.pending[n]))
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
            advance_byte(&mut self.pos, b);
        }
        Ok(byte)
    }

    /// Skips whitespace as ICU `u_isspace` defines it.
    fn skip_ws(&mut self) -> Result<()> {
        loop {
            match self.space_len()? {
                0 => return Ok(()),
                1 => {
                    self.scan(|b| !is_ascii_space(b), None)?;
                }
                len => {
                    for _ in 0..len {
                        self.bump()?;
                    }
                }
            }
        }
    }

    /// Length in bytes of the whitespace character under the cursor; 0 if there is none.
    fn space_len(&mut self) -> Result<usize> {
        let Some(first) = self.peek()? else {
            return Ok(0);
        };
        let len = match first {
            b if is_ascii_space(b) => 1,
            // U+0085 and U+00A0.
            0xC2 => match self.peek_ahead(1)? {
                Some(0x85 | 0xA0) => 2,
                _ => 0,
            },
            // The others are U+1680..U+3000.
            0xE1..=0xE3 => {
                let (Some(b1), Some(b2)) = (self.peek_ahead(1)?, self.peek_ahead(2)?) else {
                    return Ok(0);
                };
                let c = u32::from(first & 0x0F) << 12
                    | u32::from(b1 & 0x3F) << 6
                    | u32::from(b2 & 0x3F);
                let valid = b1 & 0xC0 == 0x80 && b2 & 0xC0 == 0x80;
                if valid && char::from_u32(c).is_some_and(is_space) {
                    3
                } else {
                    0
                }
            }
            _ => 0,
        };
        Ok(len)
    }

    /// Moves over bytes until `stop` matches, a whole buffer at a time, copying them to `out`.
    ///
    /// Returns the stop byte, which stays unread, or `None` at the end of input.
    fn scan(
        &mut self,
        stop: impl Fn(u8) -> bool,
        mut out: Option<&mut Vec<u8>>,
    ) -> Result<Option<u8>> {
        // Bytes taken to look ahead: at most a few, one by one.
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
            let buf = fill(&mut self.inner)?;
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

    /// Reads one character, which may be an unpaired surrogate; `None` at the end of input.
    fn next_code_point(&mut self) -> Result<Option<u32>> {
        let Some(first) = self.bump()? else {
            return Ok(None);
        };
        let (len, mut c) = match first {
            0..=0x7F => return Ok(Some(u32::from(first))),
            0xC0..=0xDF => (2, u32::from(first & 0x1F)),
            0xE0..=0xEF => (3, u32::from(first & 0x0F)),
            0xF0..=0xF7 => (4, u32::from(first & 0x07)),
            _ => return Ok(Some(u32::from(char::REPLACEMENT_CHARACTER))),
        };
        for _ in 1..len {
            match self.peek()? {
                Some(b) if b & 0xC0 == 0x80 => {
                    self.bump()?;
                    c = c << 6 | u32::from(b & 0x3F);
                }
                _ => return Ok(Some(u32::from(char::REPLACEMENT_CHARACTER))),
            }
        }
        Ok(Some(c))
    }
}

fn is_ascii_space(b: u8) -> bool {
    matches!(b, b'\t'..=b'\r' | 0x1C..=b' ')
}

struct Parser<R> {
    input: Input<R>,
    /// Items of all open lists, innermost last; reused between values.
    ///
    /// A closed list takes its items out into a `Vec` of the exact size,
    /// so lists do not grow by reallocation.
    items: Vec<Node>,
}

impl<R: BufRead> Parser<R> {
    fn new(inner: R) -> Self {
        Self {
            input: Input::new(inner),
            items: Vec::new(),
        }
    }

    fn value(&mut self) -> Result<Node> {
        match self.input.peek()? {
            Some(b'{') => self.list(),
            _ => self.scalar(),
        }
    }

    /// Lists are parsed without recursion: forms can be deeply nested.
    fn list(&mut self) -> Result<Node> {
        // (position of `{`, index of its first item in `self.items`)
        let mut stack: Vec<(Position, usize)> = Vec::new();
        // Left over from a value that failed to parse.
        self.items.clear();
        loop {
            // The cursor is always on `{` here.
            let opened = self.input.pos;
            self.input.bump()?;
            self.input.skip_ws()?;
            stack.push((opened, self.items.len()));
            match self.input.peek()? {
                // Empty list `{}`; the shared closing code follows.
                Some(b'}') => {}
                Some(b'{') => continue,
                _ => {
                    let item = self.scalar()?;
                    self.items.push(item);
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
                        // Before `}` this is an empty atom: `{a,}` has two items, as the
                        // platform writes an empty value.
                        let item = self.scalar()?;
                        self.items.push(item);
                    }
                    Some(b'}') => {
                        self.input.bump()?;
                        let (opened, first) = stack.pop().expect("list stack is not empty");
                        let items = self.items.drain(first..).collect();
                        let done = Node::parsed(Value::List(items), opened);
                        if stack.is_empty() {
                            return Ok(done);
                        }
                        self.items.push(done);
                    }
                    // Only after a string or a list: an atom runs up to `,` or `}`.
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

    /// `"text"`: `""` is a quote, `"\xxxx` a UTF-16 code unit in hex, which the platform
    /// writes for each surrogate. The text goes on after an escape without a new quote:
    /// `"a"\d83d"\de00b"` is `a😀b`.
    fn string(&mut self) -> Result<Node> {
        let start = self.input.pos;
        self.input.bump()?;
        let mut bytes = Vec::new();
        loop {
            let part = bytes.len();
            if self.input.scan(|b| b == b'"', Some(&mut bytes))?.is_none() {
                return Err(ParseError::UnclosedString { start });
            }
            if std::str::from_utf8(&bytes[part..]).is_err() {
                return Err(ParseError::InvalidUtf8 {
                    context: "quoted string",
                    at: start,
                });
            }
            self.input.bump()?;
            match self.input.peek()? {
                Some(b'"') => {
                    self.input.bump()?;
                    bytes.push(b'"');
                }
                Some(b'\\') => {
                    let at = self.input.pos;
                    self.input.bump()?;
                    let mut unit = 0;
                    for _ in 0..4 {
                        let digit = self.input.next_code_point()?.and_then(hex_digit);
                        let Some(digit) = digit else {
                            return Err(ParseError::InvalidEscape { at });
                        };
                        unit = unit << 4 | digit;
                    }
                    push_code_point(&mut bytes, unit);
                }
                _ => break,
            }
        }
        Ok(Node::parsed(Value::Str(Text::from_wtf8(bytes)), start))
    }

    /// Everything up to `,`, `}` or the end of input, whitespace and line breaks included.
    fn atom(&mut self) -> Result<Node> {
        let start = self.input.pos;
        let mut bytes = Vec::new();
        self.input
            .scan(|b| matches!(b, b',' | b'}'), Some(&mut bytes))?;
        if std::str::from_utf8(&bytes).is_err() {
            return Err(ParseError::InvalidUtf8 {
                context: "value",
                at: start,
            });
        }
        Ok(Node::parsed(Value::Atom(Text::from_wtf8(bytes)), start))
    }

    /// Character under the cursor (for error messages); may span several bytes.
    fn char_at_cursor(&mut self) -> Result<char> {
        let c = self.input.next_code_point()?;
        Ok(c.and_then(char::from_u32)
            .unwrap_or(char::REPLACEMENT_CHARACTER))
    }
}

/// A hex digit of an escape, as the platform reads it: ASCII and fullwidth `0-9`, `A-F`
/// and `a-f` in any case, and any decimal digit of the Basic Multilingual Plane (`٣` is 3).
fn hex_digit(c: u32) -> Option<u32> {
    match c {
        0x30..=0x39 => Some(c - 0x30),
        0x41..=0x46 => Some(c - 0x41 + 10),
        0x61..=0x66 => Some(c - 0x61 + 10),
        0xFF21..=0xFF26 => Some(c - 0xFF21 + 10),
        0xFF41..=0xFF46 => Some(c - 0xFF41 + 10),
        _ => DIGIT_ZEROS
            .iter()
            .find(|&&zero| (zero..zero + 10).contains(&c))
            .map(|zero| c - zero),
    }
}

/// Zeros of the decimal digits (category Nd) of the Basic Multilingual Plane in Unicode 6.0,
/// the version of ICU 4.6 the platform uses. Each zero is followed by the other nine digits.
const DIGIT_ZEROS: [u32; 34] = [
    0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6, 0x0D66,
    0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50,
    0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xAA50, 0xABF0, 0xFF10,
];

/// Moves the position over one byte already read.
fn advance_byte(pos: &mut Position, byte: u8) {
    pos.offset += 1;
    if byte == b'\n' {
        pos.line += 1;
        pos.column = 1;
    } else if byte & 0xC0 != 0x80 {
        // UTF-8 continuation bytes do not advance the column.
        pos.column += 1;
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
    fn atoms_run_to_comma_or_brace() {
        let node = Node::try_from("{1 ,  2\r\n}").unwrap();
        assert_eq!(node, Node::list([atom("1 "), atom("2\r\n")]));
        assert_eq!(node[1].as_atom_trimmed(), Some("2"));
        assert_eq!(node[1].as_i64(), Some(2));
        assert_eq!(node.to_string(), "{1 ,2\r\n}");

        let node = Node::try_from("{a{b,{x\"y\",1}}").unwrap();
        assert_eq!(
            node,
            Node::list([atom("a{b"), Node::list([atom("x\"y\""), atom("1")])])
        );
        // Leading whitespace is skipped, Unicode whitespace included.
        let node = Node::try_from("{\u{3000}\"a\"\u{a0},\u{2028}{}\u{85}, b}").unwrap();
        assert_eq!(
            node,
            Node::list([Node::string("a"), Node::list([]), atom("b")])
        );
        assert_eq!(
            Node::from_reader(ByteByByte("{\u{2003}x\u{2003}}".as_bytes())).unwrap(),
            Node::list([atom("x\u{2003}")])
        );
    }

    #[test]
    fn empty_items() {
        let items = |text: &str| Node::try_from(text).unwrap().into_list().unwrap();
        assert_eq!(items("{}"), []);
        assert_eq!(items("{ \r\n}"), []);
        // The platform writes an empty value as nothing: `{a,}` has two items.
        assert_eq!(items("{,}"), [atom(""), atom("")]);
        assert_eq!(items("{a,}"), [atom("a"), atom("")]);
        assert_eq!(items("{,a}"), [atom(""), atom("a")]);
        assert_eq!(items("{,,}").len(), 3);
    }

    #[test]
    fn unescapes_utf16_code_units() {
        let node = Node::try_from(r#"{"a"\d83d"\de00b",""""\D800"""}"#).unwrap();
        assert_eq!(node[0].as_quoted(), Some("a😀b"));
        assert_eq!(node[0].to_string(), r#""a"\d83d"\de00b""#);
        // An escape follows a quote: a string that starts with one starts with `""`.
        let lone = node[1].as_text().unwrap();
        let units = [u16::from(b'"'), 0xD800, u16::from(b'"')];
        assert_eq!(lone.encode_utf16().collect::<Vec<_>>(), units);
        assert_eq!(node[1].as_str(), None);
        assert_eq!(node[1].to_string(), r#"""""\d800""""#);
        // After a doubled quote the text goes on: `"\` there is not an escape.
        assert_eq!(
            Node::try_from(r#""""\d800""#).unwrap().as_quoted(),
            Some(r#""\d800"#)
        );
        let node = Node::string(Text::from_utf16(&units[1..]));
        assert_eq!(node.to_string(), r#"""\d800""""#);
        assert_eq!(Node::try_from(node.to_string().as_str()).unwrap(), node);

        // Fullwidth letters and any decimal digits are hex digits too.
        let node = Node::try_from(r#"""\００４ｆ"\٠٠٤١""#).unwrap();
        assert_eq!(node.as_quoted(), Some("OA"));
    }

    #[test]
    fn reads_only_utf8() {
        // A UTF-16 BOM is not UTF-8.
        let err = Node::try_from(&[0xFF, 0xFE, b'{', 0, b'}', 0][..]).unwrap_err();
        assert!(matches!(err, ParseError::InvalidUtf8 { .. }), "{err}");
        // An unpaired surrogate written as UTF-8 bytes is not valid UTF-8 either.
        let err = Node::try_from(&b"{\"\xED\xA0\x80\"}"[..]).unwrap_err();
        assert!(matches!(err, ParseError::InvalidUtf8 { .. }), "{err}");
    }

    #[test]
    fn deep_nesting_without_recursion() {
        let depth = 5_000;
        let text = format!("{}{}", "{".repeat(depth), "}".repeat(depth));
        assert!(Node::try_from(text.as_str()).is_ok());
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
