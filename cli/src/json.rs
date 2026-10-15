//! JSON: a list is an array, a quoted string is a string, an atom is a number when its text is
//! a JSON number, `null` when empty and `{"atom": "..."}` otherwise. The text of numbers is kept
//! as is, so the mapping is lossless. In the plain mapping atoms other than numbers and empty
//! ones are strings: easier to read, but it cannot be imported back. Unpaired surrogates are
//! written as `\uXXXX` escapes and read back.

use std::io::{self, Write};

use anyhow::{Result, anyhow, bail};
use one_brackets::{Node, Text, Value};

use crate::tree::{self, Location, Syntax};

/// Writes `node` and a line break.
pub fn write(node: &Node, out: &mut dyn Write, compact: bool, plain: bool) -> io::Result<()> {
    tree::write_nested(&Json { compact, plain }, node, out, compact, 0)?;
    out.write_all(b"\n")
}

struct Json {
    compact: bool,
    plain: bool,
}

impl Syntax for Json {
    const SEPARATOR: &'static str = ",";

    fn open(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"[")
    }

    fn close(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"]")
    }

    fn empty(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"[]")
    }

    fn leaf(&self, node: &Node, out: &mut dyn Write) -> io::Result<()> {
        match node.value() {
            Value::Str(s) => write_string(s, out),
            Value::Atom(s) if s.is_empty() => out.write_all(b"null"),
            Value::Atom(s) if s.as_str().is_some_and(is_number) => out.write_all(s.as_wtf8()),
            Value::Atom(s) if self.plain => write_string(s, out),
            Value::Atom(s) => {
                let key: &[u8] = if self.compact {
                    b"{\"atom\":"
                } else {
                    b"{\"atom\": "
                };
                out.write_all(key)?;
                write_string(s, out)?;
                out.write_all(b"}")
            }
            Value::List(_) => unreachable!("lists are not leaves"),
        }
    }
}

/// Whether `s` follows the JSON number grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
fn is_number(s: &str) -> bool {
    fn digits(s: &[u8]) -> usize {
        s.iter().take_while(|b| b.is_ascii_digit()).count()
    }
    let mut s = s.as_bytes();
    if let [b'-', rest @ ..] = s {
        s = rest;
    }
    let int = digits(s);
    if int == 0 || int > 1 && s[0] == b'0' {
        return false;
    }
    s = &s[int..];
    if let [b'.', rest @ ..] = s {
        let frac = digits(rest);
        if frac == 0 {
            return false;
        }
        s = &rest[frac..];
    }
    if let [b'e' | b'E', rest @ ..] = s {
        let rest = rest
            .strip_prefix(b"+")
            .or(rest.strip_prefix(b"-"))
            .unwrap_or(rest);
        let exp = digits(rest);
        if exp == 0 {
            return false;
        }
        s = &rest[exp..];
    }
    s.is_empty()
}

fn write_string(text: &Text, out: &mut dyn Write) -> io::Result<()> {
    out.write_all(b"\"")?;
    match text.as_str() {
        Some(s) => write_escaped(s, out)?,
        None => {
            for c in text.code_points() {
                match char::from_u32(c) {
                    Some(c) => write_escaped(c.encode_utf8(&mut [0; 4]), out)?,
                    None => write!(out, "\\u{c:04x}")?,
                }
            }
        }
    }
    out.write_all(b"\"")
}

fn write_escaped(s: &str, out: &mut dyn Write) -> io::Result<()> {
    let mut rest = s;
    while let Some(i) = rest.find(|c: char| c < ' ' || c == '"' || c == '\\') {
        out.write_all(&rest.as_bytes()[..i])?;
        let c = rest.as_bytes()[i];
        match c {
            b'"' => out.write_all(b"\\\"")?,
            b'\\' => out.write_all(b"\\\\")?,
            b'\n' => out.write_all(b"\\n")?,
            b'\r' => out.write_all(b"\\r")?,
            b'\t' => out.write_all(b"\\t")?,
            _ => write!(out, "\\u{c:04x}")?,
        }
        rest = &rest[i + 1..];
    }
    out.write_all(rest.as_bytes())
}

/// Reads one value written in the lossless mapping. Not recursive.
pub fn read(text: &str) -> Result<Node> {
    let mut reader = Reader { text, pos: 0 };
    // Items of the open arrays, innermost last.
    let mut stack: Vec<Vec<Node>> = Vec::new();
    'value: loop {
        reader.skip_ws();
        let mut node = match reader.peek() {
            Some(b'[') => {
                reader.pos += 1;
                reader.skip_ws();
                if reader.peek() == Some(b']') {
                    reader.pos += 1;
                    Node::list([])
                } else {
                    stack.push(Vec::new());
                    continue 'value;
                }
            }
            Some(b'"') => Node::string(reader.string()?),
            Some(b'{') => reader.object()?,
            Some(b'n') => {
                reader.literal("null")?;
                Node::atom("")
            }
            Some(b'-' | b'0'..=b'9') => reader.number()?,
            Some(b't' | b'f') => {
                return Err(reader.error("booleans are not supported: write 1 or 0"));
            }
            Some(_) => return Err(reader.error("expected a value")),
            None => return Err(reader.error("unexpected end of input: expected a value")),
        };
        loop {
            let Some(items) = stack.last_mut() else {
                reader.skip_ws();
                if reader.peek().is_some() {
                    return Err(reader.error("trailing data after the value"));
                }
                return tree::root(node);
            };
            items.push(node);
            reader.skip_ws();
            match reader.peek() {
                Some(b',') => {
                    reader.pos += 1;
                    continue 'value;
                }
                Some(b']') => {
                    reader.pos += 1;
                    node = Node::from(stack.pop().expect("an array is open"));
                }
                _ => return Err(reader.error("expected ',' or ']'")),
            }
        }
    }
}

struct Reader<'a> {
    text: &'a str,
    pos: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn here(&self) -> Location<'_> {
        Location {
            text: self.text,
            offset: self.pos,
        }
    }

    fn error(&self, message: &str) -> anyhow::Error {
        anyhow!("{message} at {}", self.here())
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        self.skip_ws();
        if self.peek() != Some(byte) {
            return Err(self.error(&format!("expected '{}'", byte as char)));
        }
        self.pos += 1;
        Ok(())
    }

    fn literal(&mut self, word: &str) -> Result<()> {
        if !self.text[self.pos..].starts_with(word) {
            return Err(self.error("expected a value"));
        }
        self.pos += word.len();
        Ok(())
    }

    fn number(&mut self) -> Result<Node> {
        let start = self.pos;
        let len = self.text[start..]
            .find(|c: char| !matches!(c, '0'..='9' | '-' | '+' | '.' | 'e' | 'E'))
            .unwrap_or(self.text.len() - start);
        let number = &self.text[start..start + len];
        if !is_number(number) {
            return Err(self.error(&format!("invalid number `{number}`")));
        }
        self.pos += len;
        Ok(Node::atom(number))
    }

    /// `{"atom": "..."}` or `{"str": "..."}`.
    fn object(&mut self) -> Result<Node> {
        let start = self.pos;
        self.pos += 1;
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return Err(self.error("expected `\"atom\"` or `\"str\"`"));
        }
        let key = self.string()?;
        self.expect(b':')?;
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return Err(self.error("expected a string"));
        }
        let value = self.string()?;
        self.expect(b'}')?;
        let at = Location {
            text: self.text,
            offset: start,
        };
        match key.as_str().unwrap_or_default() {
            "atom" => tree::atom(value, at),
            "str" => Ok(Node::string(value)),
            _ => bail!("unknown key {key:?} at {at}: expected \"atom\" or \"str\""),
        }
    }

    /// A string as UTF-16 code units: `\u` escapes may give unpaired surrogates.
    fn string(&mut self) -> Result<Text> {
        self.pos += 1;
        let mut value: Vec<u16> = Vec::new();
        loop {
            let rest = &self.text[self.pos..];
            let Some(i) = rest.find(|c: char| c < ' ' || c == '"' || c == '\\') else {
                return Err(self.error("unclosed string"));
            };
            value.extend(rest[..i].encode_utf16());
            self.pos += i;
            match self.peek() {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(Text::from_utf16(&value));
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escaped = match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            value.push(self.hex4()?);
                            continue;
                        }
                        _ => return Err(self.error("invalid escape")),
                    };
                    self.pos += 1;
                    value.push(escaped as u16);
                }
                _ => return Err(self.error("control character in a string: escape it")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u16> {
        let hex = self.text.get(self.pos..self.pos + 4).unwrap_or("");
        match u16::from_str_radix(hex, 16) {
            Ok(code) if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                self.pos += 4;
                Ok(code)
            }
            _ => Err(self.error("expected four hex digits after \\u")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(text: &str, compact: bool, plain: bool) -> String {
        let node = Node::try_from(text).unwrap();
        let mut out = Vec::new();
        write(&node, &mut out, compact, plain).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn writes_lists_of_scalars_on_one_line() {
        let text = "{27,{\"ru\",\"Форма \"\"А\"\"\"},{1,{}},,N,-0.5e3,007}";
        insta::assert_snapshot!(export(text, false, false), @r#"
        [
          27,
          ["ru", "Форма \"А\""],
          [1, []],
          null,
          {"atom": "N"},
          -0.5e3,
          {"atom": "007"}
        ]
        "#);
        insta::assert_snapshot!(export(text, true, false), @r#"[27,["ru","Форма \"А\""],[1,[]],null,{"atom":"N"},-0.5e3,{"atom":"007"}]"#);
        insta::assert_snapshot!(export(text, true, true), @r#"[27,["ru","Форма \"А\""],[1,[]],null,"N",-0.5e3,"007"]"#);
    }

    #[test]
    fn escapes_strings() {
        let node = Node::list([Node::string("a\r\n\t\"\\\u{1}/")]);
        let mut out = Vec::new();
        write(&node, &mut out, true, false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[\"a\\r\\n\\t\\\"\\\\\\u0001/\"]\n"
        );
    }

    #[test]
    fn round_trips() {
        let text = "{27,{\"ru\",\"a\r\n\"\"b\"\"\"},{1,{}},,N,-0.5e3,007,\
                    {#base64:QUJD\r\r\nREVG\r\r\n},\"\",\"😀\"}";
        let node = Node::try_from(text).unwrap();
        for compact in [false, true] {
            let mut out = Vec::new();
            write(&node, &mut out, compact, false).unwrap();
            assert_eq!(read(std::str::from_utf8(&out).unwrap()).unwrap(), node);
        }
    }

    #[test]
    fn reads_escapes_and_explicit_kinds() {
        let node = read(r#"[{"str": "1"}, {"atom": "1"}, "\u00e9\ud83d\ude00\/", null]"#).unwrap();
        assert_eq!(node.to_string(), r#"{"1",1,"é"\d83d"\de00/",}"#);
    }

    #[test]
    fn deep_nesting_without_recursion() {
        let depth = 100_000;
        let text = format!("{}{}", "{".repeat(depth), "}".repeat(depth));
        let node = Node::try_from(text.as_str()).unwrap();
        let mut out = Vec::new();
        write(&node, &mut out, true, false).unwrap();
        assert_eq!(read(std::str::from_utf8(&out).unwrap()).unwrap(), node);
    }

    #[test]
    fn reports_positions() {
        let error = |text: &str| read(text).unwrap_err().to_string();
        assert_eq!(
            error("[1,\n  2 3]"),
            "expected ',' or ']' at line 2, column 5"
        );
        assert_eq!(
            error("[1,\n  [2,"),
            "unexpected end of input: expected a value at line 2, column 6"
        );
        assert_eq!(
            error("[true]"),
            "booleans are not supported: write 1 or 0 at line 1, column 2"
        );
        assert_eq!(error("[01]"), "invalid number `01` at line 1, column 2");
        assert_eq!(
            error("[\n{\"atom\": \" a\"}]"),
            "atom \" a\" at line 2, column 1 starts with whitespace: use a quoted string instead"
        );
        assert_eq!(
            error("[{\"x\": \"1\"}]"),
            "unknown key \"x\" at line 1, column 2: expected \"atom\" or \"str\""
        );
        assert_eq!(error("null"), "an empty atom cannot be the top-level value");
        assert_eq!(
            error("[] 1"),
            "trailing data after the value at line 1, column 4"
        );
    }

    #[test]
    fn keeps_unpaired_surrogates() {
        let node = Node::list([Node::string(Text::from_utf16(&[0x61, 0xDE00, 0xD800]))]);
        let mut out = Vec::new();
        write(&node, &mut out, true, false).unwrap();
        let json = String::from_utf8(out).unwrap();
        assert_eq!(json, "[\"a\\ude00\\ud800\"]\n");
        assert_eq!(read(&json).unwrap(), node);
        // A pair written as escapes is one character.
        assert_eq!(
            read(r#"["😀"]"#).unwrap().to_string(),
            r#"{""\d83d"\de00"}"#
        );
    }
}
