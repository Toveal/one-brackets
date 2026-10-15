//! YAML: a list is a sequence, a quoted string is a double-quoted scalar, an atom is a plain
//! scalar. An empty atom is `~`; an atom that cannot be written plain (a leading indicator,
//! whitespace, `[`, `]` and the like) is a double-quoted scalar tagged `!atom`.
//!
//! On reading, plain scalars are atoms and quoted or block scalars are strings, whatever they
//! look like; the tags `!atom` and `!str` (or `!!str`) set the kind explicitly.

use std::borrow::Cow;
use std::io::{self, Write};
use std::slice;

use anyhow::{Result, anyhow, bail};
use one_brackets::{Node, Value};
use saphyr_parser::{Event, Parser, ScalarStyle, Tag};

use crate::tree::{self, Location, Syntax};

/// Writes `node` and a line break: in block style with lists of scalars in flow style,
/// or all in flow style on one line when `compact`.
pub fn write(node: &Node, out: &mut dyn Write, compact: bool) -> io::Result<()> {
    match node.as_list() {
        Some(items) if !compact && !tree::is_flat(items) => write_block(items, out)?,
        _ => tree::write_nested(&Flow, node, out, compact, 0)?,
    }
    out.write_all(b"\n")
}

/// Not recursive: each nested block list is an entry of the stack.
fn write_block(items: &[Node], out: &mut dyn Write) -> io::Result<()> {
    // (items left, indentation of their `- `)
    let mut stack: Vec<(slice::Iter<'_, Node>, usize)> = vec![(items.iter(), 0)];
    // The cursor is right after `- ` (or at the start): the next entry continues the line.
    let mut fresh = true;
    while let Some((items, indent)) = stack.last_mut() {
        let indent = *indent;
        let Some(item) = items.next() else {
            stack.pop();
            continue;
        };
        if !fresh {
            out.write_all(b"\n")?;
            out.write_all(" ".repeat(indent).as_bytes())?;
        }
        out.write_all(b"- ")?;
        match item.as_list() {
            Some(nested) if !tree::is_flat(nested) => {
                stack.push((nested.iter(), indent + 2));
                fresh = true;
            }
            _ => {
                tree::write_nested(&Flow, item, out, false, 0)?;
                fresh = false;
            }
        }
    }
    Ok(())
}

struct Flow;

impl Syntax for Flow {
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
            Value::Str(s) => write_quoted(s, out),
            Value::Atom(s) if s.is_empty() => out.write_all(b"~"),
            Value::Atom(s) if is_plain(s) => out.write_all(s.as_bytes()),
            Value::Atom(s) => {
                out.write_all(b"!atom ")?;
                write_quoted(s, out)
            }
            Value::List(_) => unreachable!("lists are not leaves"),
        }
    }
}

/// Whether the atom reads back unchanged as a plain scalar, in block and in flow context.
fn is_plain(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    s != "~"
        && s != "-"
        && !s.ends_with(':')
        && !s.starts_with("---")
        && !s.starts_with("...")
        && !"?:,[]{}#&*!|>'\"%@`".contains(first)
        && s.chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !"[]{},\u{feff}".contains(c))
}

fn write_quoted(s: &str, out: &mut dyn Write) -> io::Result<()> {
    let special =
        |c: char| c.is_control() || matches!(c, '"' | '\\' | '\u{2028}' | '\u{2029}' | '\u{feff}');
    out.write_all(b"\"")?;
    let mut rest = s;
    while let Some(i) = rest.find(special) {
        out.write_all(&rest.as_bytes()[..i])?;
        let c = rest[i..].chars().next().expect("found a character");
        match c {
            '"' => out.write_all(b"\\\"")?,
            '\\' => out.write_all(b"\\\\")?,
            '\0' => out.write_all(b"\\0")?,
            '\n' => out.write_all(b"\\n")?,
            '\r' => out.write_all(b"\\r")?,
            '\t' => out.write_all(b"\\t")?,
            '\u{85}' => out.write_all(b"\\N")?,
            '\u{2028}' => out.write_all(b"\\L")?,
            '\u{2029}' => out.write_all(b"\\P")?,
            '\u{feff}' => out.write_all(b"\\uFEFF")?,
            c => write!(out, "\\x{:02X}", u32::from(c))?,
        }
        rest = &rest[i + c.len_utf8()..];
    }
    out.write_all(rest.as_bytes())?;
    out.write_all(b"\"")
}

/// Reads a document of one value. Not recursive.
pub fn read(text: &str) -> Result<Node> {
    let mut stack: Vec<Vec<Node>> = Vec::new();
    let mut root = None;
    let mut documents = 0;
    for event in Parser::new_from_str(text) {
        let (event, span) = event.map_err(|e| {
            let at = e.marker();
            anyhow!(
                "{} at line {}, column {}",
                e.info(),
                at.line(),
                at.col() + 1
            )
        })?;
        let at = Location {
            text,
            offset: byte_offset(text, span.start.index()),
        };
        let node = match event {
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    bail!("a second document at {at}: only one value is allowed");
                }
                continue;
            }
            Event::SequenceStart(_, tag) => {
                if let Some(tag) = tag {
                    bail!("unsupported tag {tag} on a sequence at {at}");
                }
                stack.push(Vec::new());
                continue;
            }
            Event::SequenceEnd => Node::from(stack.pop().expect("a sequence is open")),
            Event::Scalar(value, style, _, tag) => scalar(value, style, tag.as_deref(), at)?,
            Event::MappingStart(..) => {
                bail!("a mapping at {at}: only sequences and scalars are allowed")
            }
            Event::Alias(_) => bail!("an alias at {at}: aliases are not supported"),
            _ => continue,
        };
        match stack.last_mut() {
            Some(items) => items.push(node),
            None => root = Some(node),
        }
    }
    tree::root(root.ok_or_else(|| anyhow!("input is empty: expected a value"))?)
}

fn scalar(
    value: Cow<'_, str>,
    style: ScalarStyle,
    tag: Option<&Tag>,
    at: Location<'_>,
) -> Result<Node> {
    let value = value.into_owned();
    match tag {
        Some(tag) if tag.handle == "!" && tag.suffix == "atom" => tree::atom(value, at),
        Some(tag) if tag.suffix == "str" && (tag.handle == "!" || tag.is_yaml_core_schema()) => {
            Ok(Node::string(value))
        }
        Some(tag) => bail!("unsupported tag {tag} at {at}: use !atom or !str"),
        None if style == ScalarStyle::Plain && value == "~" => Ok(Node::atom("")),
        None if style == ScalarStyle::Plain => tree::atom(value, at),
        None => Ok(Node::string(value)),
    }
}

/// The parser counts positions in characters.
fn byte_offset(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(text: &str, compact: bool) -> String {
        let node = Node::try_from(text).unwrap();
        let mut out = Vec::new();
        write(&node, &mut out, compact).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn writes_block_style() {
        let text = "{27,{18,{{1,1,{\"ru\",\"Форма\"}},25,4294967295},\
                    09ccdc77-ea1a-4a6d-ab1c-3435eada2433},{},,N,\"\"}";
        insta::assert_snapshot!(export(text, false), @r#"
        - 27
        - - 18
          - - - 1
              - 1
              - ["ru", "Форма"]
            - 25
            - 4294967295
          - 09ccdc77-ea1a-4a6d-ab1c-3435eada2433
        - []
        - ~
        - N
        - ""
        "#);
        insta::assert_snapshot!(export(text, true), @r#"[27,[18,[[1,1,["ru","Форма"]],25,4294967295],09ccdc77-ea1a-4a6d-ab1c-3435eada2433],[],~,N,""]"#);
        insta::assert_snapshot!(export("{1,{2,3}}", false), @r"
        - 1
        - [2, 3]
        ");
        insta::assert_snapshot!(export("{1,2,{}}", false), @"[1, 2, []]");
        insta::assert_snapshot!(export("\"a\"", false), @r#""a""#);
    }

    #[test]
    fn tags_atoms_that_are_not_plain() {
        let text = "{-22,-,~,a:,a:b,#base64:QUJD\r\r\nREVG\r\r\n,[x],!x,---,*,10%}";
        insta::assert_snapshot!(export(text, false), @r##"[-22, !atom "-", !atom "~", !atom "a:", a:b, !atom "#base64:QUJD\r\r\nREVG\r\r\n", !atom "[x]", !atom "!x", !atom "---", !atom "*", 10%]"##);
    }

    #[test]
    fn round_trips() {
        let text = "{27,{\"ru\",\"a\r\n\"\"b\"\"\\\t\u{1}\u{85}\u{2028}\u{feff}😀\"},{1,{}},,N,-0.5e3,007,\
                    {#base64:QUJD\r\r\nREVG\r\r\n},\"\",\"~\",\"1\",-,~,a:,[x],null,true,\
                    {{{}}},{{1},{2,{3}}}}";
        let node = Node::try_from(text).unwrap();
        for compact in [false, true] {
            let mut out = Vec::new();
            write(&node, &mut out, compact).unwrap();
            let yaml = String::from_utf8(out).unwrap();
            assert_eq!(read(&yaml).unwrap(), node, "{yaml}");
        }
    }

    #[test]
    fn reads_scalar_styles_and_tags() {
        let yaml = "- plain\n- 'single'\n- \"double\"\n- |\n  block\n- !atom \"x\"\n- !str y\n- !!str z\n-\n- ~\n";
        assert_eq!(
            read(yaml).unwrap().to_string(),
            "{plain,\"single\",\"double\",\"block\n\",x,\"y\",\"z\",,}"
        );
    }

    #[test]
    fn deep_nesting_in_block_style() {
        // Block style indents each level: the size grows as the square of the depth.
        let depth = 3_000;
        let text = format!("{}1{}", "{".repeat(depth), "}".repeat(depth));
        let node = Node::try_from(text.as_str()).unwrap();
        let mut out = Vec::new();
        write(&node, &mut out, false).unwrap();
        assert_eq!(read(std::str::from_utf8(&out).unwrap()).unwrap(), node);
    }

    #[test]
    fn flow_style_is_limited_by_the_parser() {
        let flow = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        assert!(read(&flow(255)).is_ok());
        assert_eq!(
            read(&flow(256)).unwrap_err().to_string(),
            "recursion limit exceeded at line 1, column 256"
        );
    }

    #[test]
    fn reports_errors() {
        let error = |text: &str| read(text).unwrap_err().to_string();
        assert_eq!(
            error("- 1\n- a: 2\n"),
            "a mapping at line 2, column 3: only sequences and scalars are allowed"
        );
        assert_eq!(
            error("- 1\n- !int 2\n"),
            "unsupported tag !int at line 2, column 8: use !atom or !str"
        );
        assert_eq!(
            error("- &a 1\n- *a\n"),
            "an alias at line 2, column 3: aliases are not supported"
        );
        assert_eq!(
            error("- 1\n---\n- 2\n"),
            "a second document at line 2, column 1: only one value is allowed"
        );
        assert_eq!(
            error("- 1\n- a b\n"),
            "atom \"a b\" at line 2, column 3 contains whitespace: use a quoted string instead"
        );
        assert_eq!(error(""), "input is empty: expected a value");
        assert_eq!(
            error("- [1\n"),
            "while parsing a flow sequence, expected ',' or ']' at line 2, column 1"
        );
    }
}
