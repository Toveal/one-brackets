//! Real bracket files from an exported data processor.

use std::fs;
use std::io::{self, Read};

use one_brackets::Node;

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
fn parses_fixtures() {
    insta::glob!("fixtures/*.txt", |path| {
        let node = Node::try_from(path).unwrap();
        insta::assert_debug_snapshot!(node);
    });
}

#[test]
fn writes_fixtures_back_byte_for_byte() {
    insta::glob!("fixtures/*.txt", |path| {
        let bytes = fs::read(path).unwrap();
        let source = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        let node = Node::try_from(bytes.as_slice()).unwrap();
        assert_eq!(
            String::from(&node),
            String::from_utf8_lossy(source),
            "{}",
            path.display()
        );
    });
}

#[test]
fn streaming_matches_buffered() {
    insta::glob!("fixtures/*.txt", |path| {
        let bytes = fs::read(path).unwrap();
        let streamed = Node::from_reader(ByteByByte(&bytes)).unwrap();
        assert_eq!(
            streamed,
            Node::try_from(bytes).unwrap(),
            "{}",
            path.display()
        );
    });
}

/// Example of a user model on top of `Node`.
#[derive(Debug)]
struct Root {
    version: u32,
    object: String,
}

impl TryFrom<&Node> for Root {
    type Error = one_brackets::AccessError;

    fn try_from(node: &Node) -> Result<Self, Self::Error> {
        Ok(Root {
            version: node.try_get(0)?.try_parse()?,
            object: node.try_get(1)?.try_guid()?.to_owned(),
        })
    }
}

/// Parsing and mapping in one step: `one_brackets::Error` accepts both error kinds.
impl std::str::FromStr for Root {
    type Err = one_brackets::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Ok(Root::try_from(&text.parse::<Node>()?)?)
    }
}

#[test]
fn maps_to_user_type() {
    let node = Node::from_reader(fs::File::open("tests/fixtures/root.txt").unwrap()).unwrap();
    let root = Root::try_from(&node).unwrap();
    assert_eq!(root.version, 2);
    assert_eq!(root.object, "c2f34349-b1e0-4060-8abf-3c59f38ac69f");

    let version = Node::try_from(std::path::Path::new("tests/fixtures/version.txt")).unwrap();
    insta::assert_snapshot!(Root::try_from(&version).unwrap_err(), @"expected u32, found list of 3 items at line 2, column 1");

    let root: Root = fs::read_to_string("tests/fixtures/root.txt")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(root.version, 2);
    insta::assert_snapshot!("{2,".parse::<Root>().unwrap_err(), @"unexpected end of input at line 1, column 4: list opened at line 1, column 1 is not closed");
    insta::assert_snapshot!("{2,\"x\"}".parse::<Root>().unwrap_err(), @r#"expected GUID, found string "x" at line 1, column 4"#);
}
