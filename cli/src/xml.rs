//! XML: `<list>`, `<str>` and `<atom>` elements. `\r` and control characters are written as
//! character references, so text is read back unchanged; whitespace between elements is
//! ignored.

use std::io::{self, Write};

use anyhow::{Context, Result, anyhow, bail};
use one_brackets::{Node, Value};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::tree::{self, Location, Syntax};

const DECLARATION: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// Writes the XML declaration, `node` and a line break.
pub fn write(node: &Node, out: &mut dyn Write, compact: bool) -> io::Result<()> {
    out.write_all(DECLARATION)?;
    tree::write_nested(&Xml, node, out, compact, 0)?;
    out.write_all(b"\n")
}

/// Writes values one by one inside a `<values>` element.
pub struct ValuesWriter<'a> {
    out: &'a mut dyn Write,
    compact: bool,
}

impl<'a> ValuesWriter<'a> {
    pub fn start(out: &'a mut dyn Write, compact: bool) -> io::Result<Self> {
        out.write_all(DECLARATION)?;
        out.write_all(b"<values>")?;
        Ok(Self { out, compact })
    }

    pub fn value(&mut self, node: &Node) -> io::Result<()> {
        if !self.compact {
            self.out.write_all(b"\n  ")?;
        }
        tree::write_nested(&Xml, node, self.out, self.compact, 1)
    }

    pub fn finish(self) -> io::Result<()> {
        if !self.compact {
            self.out.write_all(b"\n")?;
        }
        self.out.write_all(b"</values>\n")
    }
}

struct Xml;

impl Syntax for Xml {
    const SEPARATOR: &'static str = "";

    fn open(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"<list>")
    }

    fn close(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"</list>")
    }

    fn empty(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"<list/>")
    }

    fn leaf(&self, node: &Node, out: &mut dyn Write) -> io::Result<()> {
        let (name, text) = match node.value() {
            Value::Str(s) => ("str", s),
            Value::Atom(s) => ("atom", s),
            Value::List(_) => unreachable!("lists are not leaves"),
        };
        if text.is_empty() {
            return write!(out, "<{name}/>");
        }
        write!(out, "<{name}>")?;
        write_text(text, out)?;
        write!(out, "</{name}>")
    }
}

fn write_text(s: &str, out: &mut dyn Write) -> io::Result<()> {
    let special =
        |c: char| matches!(c, '&' | '<' | '>') || c.is_ascii_control() && c != '\n' && c != '\t';
    let mut rest = s;
    while let Some(i) = rest.find(special) {
        out.write_all(&rest.as_bytes()[..i])?;
        match rest.as_bytes()[i] {
            b'&' => out.write_all(b"&amp;")?,
            b'<' => out.write_all(b"&lt;")?,
            b'>' => out.write_all(b"&gt;")?,
            c => write!(out, "&#{c};")?,
        }
        rest = &rest[i + 1..];
    }
    out.write_all(rest.as_bytes())
}

/// Reads one value. Not recursive.
pub fn read(text: &str) -> Result<Node> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    // Items of the open lists, innermost last.
    let mut stack: Vec<Vec<Node>> = Vec::new();
    // Name and text of the open `<atom>` or `<str>` with the offset of its start.
    let mut leaf: Option<(&str, String, usize)> = None;
    let mut root = None;
    loop {
        let start = reader.buffer_position() as usize;
        let at = Location {
            text,
            offset: start,
        };
        let event = reader.read_event().map_err(|e| {
            let offset = reader.error_position() as usize;
            anyhow!("{e} at {}", Location { text, offset })
        })?;
        let node = match event {
            Event::Start(element) | Event::Empty(element) if leaf.is_some() => {
                bail!(
                    "element <{}> at {at} inside a scalar",
                    String::from_utf8_lossy(element.name().into_inner())
                )
            }
            Event::Start(element) => {
                match element.name().into_inner() {
                    b"list" => stack.push(Vec::new()),
                    b"atom" => leaf = Some(("atom", String::new(), start)),
                    b"str" => leaf = Some(("str", String::new(), start)),
                    other => bail!(
                        "unknown element <{}> at {at}: expected <list>, <atom> or <str>",
                        String::from_utf8_lossy(other)
                    ),
                }
                continue;
            }
            Event::Empty(element) => match element.name().into_inner() {
                b"list" => Node::list([]),
                b"atom" => Node::atom(""),
                b"str" => Node::string(""),
                other => bail!(
                    "unknown element <{}> at {at}: expected <list>, <atom> or <str>",
                    String::from_utf8_lossy(other)
                ),
            },
            Event::End(_) => match leaf.take() {
                Some(("atom", value, start)) => tree::atom(
                    value,
                    Location {
                        text,
                        offset: start,
                    },
                )?,
                Some((_, value, _)) => Node::string(value),
                None => Node::from(stack.pop().context("closing tag without an open one")?),
            },
            Event::Text(content) => {
                let content = content.xml10_content()?;
                match &mut leaf {
                    Some((_, value, _)) => value.push_str(&content),
                    None if content.trim().is_empty() => {}
                    None => bail!("text outside <atom> and <str> at {at}"),
                }
                continue;
            }
            Event::CData(content) => {
                match &mut leaf {
                    Some((_, value, _)) => value.push_str(&content.xml10_content()?),
                    None => bail!("text outside <atom> and <str> at {at}"),
                }
                continue;
            }
            Event::GeneralRef(reference) => {
                let Some((_, value, _)) = &mut leaf else {
                    bail!("text outside <atom> and <str> at {at}");
                };
                let c = match reference.resolve_char_ref()? {
                    Some(c) => c,
                    None => match &*reference {
                        b"amp" => '&',
                        b"lt" => '<',
                        b"gt" => '>',
                        b"quot" => '"',
                        b"apos" => '\'',
                        other => bail!(
                            "unknown entity &{}; at {at}",
                            String::from_utf8_lossy(other)
                        ),
                    },
                };
                value.push(c);
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        match stack.last_mut() {
            Some(items) => items.push(node),
            None if root.is_none() => root = Some(node),
            None => bail!("a second top-level element at {at}: only one value is allowed"),
        }
    }
    if !stack.is_empty() || leaf.is_some() {
        bail!("unexpected end of input: an element is not closed");
    }
    tree::root(root.ok_or_else(|| anyhow!("input is empty: expected a value"))?)
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
    fn writes_elements() {
        let text = "{27,{\"ru\",\"<a & b>\"},{1,{2}},,{},\"\"}";
        insta::assert_snapshot!(export(text, false), @r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <list>
          <atom>27</atom>
          <list><str>ru</str><str>&lt;a &amp; b&gt;</str></list>
          <list>
            <atom>1</atom>
            <list><atom>2</atom></list>
          </list>
          <atom/>
          <list/>
          <str/>
        </list>
        "#);
        insta::assert_snapshot!(export(text, true), @r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <list><atom>27</atom><list><str>ru</str><str>&lt;a &amp; b&gt;</str></list><list><atom>1</atom><list><atom>2</atom></list></list><atom/><list/><str/></list>
        "#);
    }

    #[test]
    fn writes_values() {
        let mut out = Vec::new();
        let mut values = ValuesWriter::start(&mut out, false).unwrap();
        values
            .value(&Node::try_from("{1,{2,{3}}}").unwrap())
            .unwrap();
        values.value(&Node::try_from("{4}").unwrap()).unwrap();
        values.finish().unwrap();
        insta::assert_snapshot!(String::from_utf8(out).unwrap(), @r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <values>
          <list>
            <atom>1</atom>
            <list>
              <atom>2</atom>
              <list><atom>3</atom></list>
            </list>
          </list>
          <list><atom>4</atom></list>
        </values>
        "#);
    }

    #[test]
    fn round_trips() {
        let text = "{27,{\"ru\",\"a\r\n\"\"b\"\" <&>'\t\u{1}😀\"},{1,{}},,N,\
                    {#base64:QUJD\r\r\nREVG\r\r\n},\"\",\"  \",{{{}}},{{1},{2,{3}}}}";
        let node = Node::try_from(text).unwrap();
        for compact in [false, true] {
            let mut out = Vec::new();
            write(&node, &mut out, compact).unwrap();
            let xml = String::from_utf8(out).unwrap();
            assert_eq!(read(&xml).unwrap(), node, "{xml}");
        }
    }

    #[test]
    fn reads_references_cdata_and_comments() {
        let xml = "<!-- c --><list>\r\n <str>a&#x41;&#66;&quot;&apos;<![CDATA[<x>]]></str><str>1\r\n2</str></list>";
        assert_eq!(read(xml).unwrap().to_string(), "{\"aAB\"\"'<x>\",\"1\n2\"}");
    }

    #[test]
    fn deep_nesting_without_recursion() {
        let depth = 100_000;
        let text = format!("{}{}", "{".repeat(depth), "}".repeat(depth));
        let node = Node::try_from(text.as_str()).unwrap();
        let mut out = Vec::new();
        write(&node, &mut out, true).unwrap();
        assert_eq!(read(std::str::from_utf8(&out).unwrap()).unwrap(), node);
    }

    #[test]
    fn reports_errors() {
        let error = |text: &str| read(text).unwrap_err().to_string();
        assert_eq!(
            error("<list>\n  <item/></list>"),
            "unknown element <item> at line 2, column 3: expected <list>, <atom> or <str>"
        );
        assert_eq!(
            error("<list>\n  x</list>"),
            "text outside <atom> and <str> at line 1, column 7"
        );
        assert_eq!(
            error("<list><atom>a<str/></atom></list>"),
            "element <str> at line 1, column 14 inside a scalar"
        );
        assert_eq!(
            error("<list>\n<atom>a b</atom></list>"),
            "atom \"a b\" at line 2, column 1 contains whitespace: use a quoted string instead"
        );
        assert_eq!(
            error("<list/><list/>"),
            "a second top-level element at line 1, column 8: only one value is allowed"
        );
        assert_eq!(
            error("<list>"),
            "unexpected end of input: an element is not closed"
        );
        assert_eq!(error(""), "input is empty: expected a value");
        assert!(error("<list></str>").contains("at line 1, column 7"));
    }
}
