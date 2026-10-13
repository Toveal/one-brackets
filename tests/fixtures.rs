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

/// `structure.txt` is `ЗначениеВСтрокуВнутр()` of
/// `Новый Структура("Число, Строка, Гуид", 1, "Привет", Новый УникальныйИдентификатор)`.
#[test]
fn reads_platform_value_representation() {
    let node = Node::try_from(std::path::Path::new("tests/fixtures/structure.txt")).unwrap();
    // `{"#",<type>,<data>}`: a value of an object type.
    assert_eq!(node[0].as_quoted(), Some("#"));
    assert!(node[1].as_guid().is_some());

    // A structure is a counted list of `{key,value}` pairs, each a tagged value.
    let pairs: Vec<String> = node[2]
        .try_counted()
        .unwrap()
        .iter()
        .map(|pair| format!("{}: {}", pair[0][1].try_str().unwrap(), pair[1]))
        .collect();
    insta::assert_snapshot!(pairs.join("\n"), @r##"
    Число: {"N",1}
    Строка: {"S","Привет"}
    Гуид: {"#",fc01b5df-97fe-449b-83d4-218a090e681e,48e3d89f-e931-4f71-b2bd-3c93c7752bdd}
    "##);
}

/// `value-*.txt` are `ЗначениеВСтрокуВнутр()` of one value each: the tag and,
/// for object types, the type GUID that comes after `#`.
#[test]
fn platform_value_tags() {
    let mut report = Vec::new();
    let mut paths: Vec<_> = fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("value-")
        })
        .collect();
    paths.sort();
    for path in paths {
        let node = Node::try_from(path.as_path()).unwrap();
        let tag = node[0].try_quoted().unwrap();
        let rest = match tag {
            "#" => format!("type {}", node[1].try_guid().unwrap()),
            _ => node
                .iter()
                .skip(1)
                .map(Node::describe)
                .collect::<Vec<_>>()
                .join(", "),
        };
        let name = path.file_stem().unwrap().to_str().unwrap();
        report.push(format!("{name}: {tag} {rest}").trim_end().to_owned());
    }
    insta::assert_snapshot!(report.join("\n"), @r#"
    value-array: # type 51e7a0d2-530b-11d4-b98a-008048da3034
    value-boolean: B value `1`
    value-date: D value `20260908233443`
    value-empty-string: S string ""
    value-map: # type 3d48feae-a9c6-4c5a-a099-9eb6477630c6
    value-null: L
    value-number: N value `-12345.678`
    value-string: S string "Кавычки \"внутри\" и\r\nперевод строки"
    value-undefined: U
    value-value-table: # type acf6192e-81ca-46ef-93a6-5a6968b78663
    "#);
}

#[test]
fn edits_keep_the_rest_byte_for_byte() {
    let bytes = fs::read("tests/fixtures/copyinfo.txt").unwrap();
    let source = String::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap().to_vec()).unwrap();
    let mut node: Node = source.parse().unwrap();

    let name = node.try_get_path_mut(&[1, 1, 3, 1]).unwrap();
    assert_eq!(name.as_quoted(), Some("ДемоОбработка"));
    *name = Node::string("Переименовано");

    let expected = source.replace("\"ДемоОбработка\"", "\"Переименовано\"");
    assert_ne!(expected, source);
    assert_eq!(node.to_string(), expected);
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
