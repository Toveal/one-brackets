//! Positional serde mapping (`serde` feature).
#![cfg(feature = "serde")]

use std::collections::BTreeMap;
use std::fs;

use one_brackets::{Node, Tail, from_node, from_str, to_node, to_string};
use serde::{Deserialize, Serialize};

/// `root.txt`: `{2,<object GUID>,}`.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Root {
    version: u32,
    #[serde(with = "one_brackets::atom")]
    object: String,
    extra: Option<String>,
}

/// `version.txt`: `{{216,0,{80305,0}}}`.
#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct VersionFile {
    version: Version,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Version {
    major: u32,
    #[serde(default)]
    rest: Tail,
}

#[test]
fn maps_fixtures_by_position() {
    let root: Root =
        one_brackets::from_reader(fs::File::open("tests/fixtures/root.txt").unwrap()).unwrap();
    assert_eq!(
        root,
        Root {
            version: 2,
            object: "c2f34349-b1e0-4060-8abf-3c59f38ac69f".into(),
            extra: None,
        }
    );

    let text = fs::read_to_string("tests/fixtures/version.txt").unwrap();
    let file: VersionFile = from_str(&text).unwrap();
    assert_eq!(file.version.major, 216);
    insta::assert_snapshot!(Node::list(file.version.rest.iter().cloned()), @r"
    {0,
    {80305,0}
    }
    ");
}

#[test]
fn writes_fixtures_back_byte_for_byte() {
    for name in ["root", "version"] {
        let bytes = fs::read(format!("tests/fixtures/{name}.txt")).unwrap();
        let source = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap());
        let written = match name {
            "root" => to_string(&from_str::<Root>(&source).unwrap()),
            _ => to_string(&from_str::<VersionFile>(&source).unwrap()),
        };
        assert_eq!(written.unwrap(), source, "{name}");
    }
}

#[test]
fn tail_is_optional_and_keeps_order() {
    let version: Version = from_str("{216}").unwrap();
    assert!(version.rest.is_empty());

    let version: Version = from_str("{216,\"a\",{1},x}").unwrap();
    assert_eq!(to_string(&version).unwrap(), "{216,\"a\",\r\n{1},x}");
}

#[test]
fn extra_items_are_ignored_without_tail() {
    #[derive(Debug, Deserialize)]
    struct Pair(u8, u8);
    let Pair(a, b) = from_str("{1,2,3,{4}}").unwrap();
    assert_eq!((a, b), (1, 2));
}

#[test]
fn nested_collections() {
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    struct Item {
        id: u32,
        name: String,
        enabled: bool,
    }

    let text = "{{1,\"first\",1},{2,\"second\",0}}";
    let items: Vec<Item> = from_str(text).unwrap();
    assert_eq!(items[1].name, "second");
    assert!(!items[1].enabled);
    assert_eq!(
        to_string(&items).unwrap(),
        "{\r\n{1,\"first\",1},\r\n{2,\"second\",0}\r\n}"
    );

    let map: BTreeMap<u32, String> = from_str("{{1,\"a\"},{2,\"b\"}}").unwrap();
    assert_eq!(map[&2], "b");
    assert_eq!(
        to_string(&map).unwrap(),
        "{\r\n{1,\"a\"},\r\n{2,\"b\"}\r\n}"
    );
}

#[test]
fn enums_by_index_or_name() {
    #[derive(Debug, PartialEq, Deserialize, Serialize)]
    enum Kind {
        Plain,
        Sized(u32),
        Point { x: i32, y: i32 },
    }

    let kinds: Vec<Kind> = from_str("{0,Plain,{1,10},{2,-1,5}}").unwrap();
    assert_eq!(
        kinds,
        [
            Kind::Plain,
            Kind::Plain,
            Kind::Sized(10),
            Kind::Point { x: -1, y: 5 }
        ]
    );
    assert_eq!(
        to_string(&kinds).unwrap(),
        "{0,0,\r\n{1,10},\r\n{2,-1,5}\r\n}"
    );
}

#[test]
fn skipped_fields_keep_their_slot() {
    #[derive(Serialize)]
    struct Sparse {
        a: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        b: Option<u8>,
        c: u8,
    }
    let text = to_string(&Sparse {
        a: 1,
        b: None,
        c: 3,
    })
    .unwrap();
    assert_eq!(text, "{1,,3}");
}

#[test]
fn borrows_strings_from_node() {
    #[derive(Deserialize)]
    struct Borrowed<'a> {
        #[serde(borrow)]
        name: &'a str,
    }
    let node: Node = "{\"name\"}".parse().unwrap();
    let value: Borrowed = from_node(&node).unwrap();
    assert_eq!(value.name, "name");
}

#[test]
fn node_fields_keep_positions() {
    #[derive(Deserialize, Serialize)]
    struct Raw {
        version: u32,
        body: Node,
    }
    let node: Node = "{1,\r\n{x,\"y\"}}".parse().unwrap();
    let raw: Raw = from_node(&node).unwrap();
    insta::assert_snapshot!(raw.body[1].position().unwrap(), @"line 2, column 4");
    assert_eq!(to_node(&raw).unwrap(), node);
}

#[test]
fn whole_fixtures_round_trip_through_node() {
    insta::glob!("fixtures/*.txt", |path| {
        let node = Node::try_from(path).unwrap();
        let copy: Node = from_node(&node).unwrap();
        assert_eq!(copy, node);
        assert_eq!(copy.position(), node.position());
        assert_eq!(to_node(&copy).unwrap(), node);
    });
}

#[test]
fn other_formats_see_tagged_values() {
    let node: Node = "{1,\"s\",{}}".parse().unwrap();
    let json = serde_json::to_string(&node).unwrap();
    insta::assert_snapshot!(json, @r#"{"List":[{"Atom":"1"},{"Str":"s"},{"List":[]}]}"#);
    let back: Node = serde_json::from_str(&json).unwrap();
    assert_eq!(back, node);
    assert_eq!(back.position(), None);

    let tail = serde_json::to_string(&Tail(vec![Node::from(1)])).unwrap();
    assert_eq!(tail, r#"[{"Atom":"1"}]"#);
}

#[test]
fn errors_point_at_the_node() {
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Strict {
        version: u32,
        items: Vec<(u8, bool)>,
    }

    let cases = [
        "{x,{}}",
        "{1,\r\n{{1,1},\r\n{2,yes}}}",
        "{1,\r\n{{300,1}}}",
        "{1}",
        "{1,\"text\"}",
    ];
    let report = cases
        .iter()
        .map(|text| format!("{text:?}: {}", from_str::<Strict>(text).unwrap_err()))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(report, @r#"
    "{x,{}}": expected u32, found value `x` at line 1, column 2 (path [0]): invalid digit found in string
    "{1,\r\n{{1,1},\r\n{2,yes}}}": expected boolean (0 or 1), found value `yes` at line 3, column 4 (path [1, 1, 1])
    "{1,\r\n{{300,1}}}": expected u8, found value `300` at line 2, column 3 (path [1, 0, 0]): number too large to fit in target type
    "{1}": invalid length 1, expected struct Strict with 2 elements at line 1, column 1
    "{1,\"text\"}": expected list, found string "text" at line 1, column 4 (path [1])
    "#);

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct WithAtom {
        #[serde(with = "one_brackets::atom")]
        id: u32,
    }
    insta::assert_snapshot!(from_str::<WithAtom>("{\r\n abc}").unwrap_err(), @"expected u32, found value `abc` (invalid digit found in string) at line 2, column 2 (path [0])");

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    enum Kind {
        A,
        B(u8),
    }
    insta::assert_snapshot!(from_str::<Vec<Kind>>("{A,\r\nC}").unwrap_err(), @"unknown variant `C`, expected `A` or `B` at line 2, column 1 (path [1])");
    insta::assert_snapshot!(from_str::<Vec<Kind>>("{A,\r\n{1}}").unwrap_err(), @"expected a list {variant,value} for a variant with data at line 2, column 1 (path [1])");
}
