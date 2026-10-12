//! Error messages are pinned with snapshots.

use std::path::Path;

use one_brackets::Node;

fn error(input: &[u8]) -> String {
    Node::try_from(input).unwrap_err().to_string()
}

#[test]
fn parse_errors() {
    let cases: &[(&str, &[u8])] = &[
        ("empty", b""),
        ("only_bom_and_spaces", b"\xEF\xBB\xBF \r\n "),
        ("unclosed_list", b"{1,\r\n{2,3}"),
        ("unclosed_string", "{1,\r\n\"строка}".as_bytes()),
        ("unmatched_close", b"{1}}"),
        ("trailing_data", b"{1} {2}"),
        ("missing_comma", b"{1 2}"),
        ("unexpected_unicode", "{\"a\"ж}".as_bytes()),
        ("list_after_value", b"{1{2}}"),
        ("invalid_utf8_string", b"{\"\xFF\"}"),
        ("invalid_utf8_value", b"{1,\xC3}"),
    ];
    let report = cases
        .iter()
        .map(|(name, input)| format!("{name}: {}", error(input)))
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(report);
}

#[test]
fn open_error_names_the_file() {
    let err = Node::try_from(Path::new("tests/fixtures/missing.txt")).unwrap_err();
    let text = err.to_string();
    assert!(
        text.starts_with("failed to open file `tests/fixtures/missing.txt`: "),
        "{text}"
    );
}

#[test]
fn access_errors() {
    let node: Node = "{1,\"text\",{}}".parse().unwrap();
    let report = [
        node.try_get(3).unwrap_err(),
        node[0].try_get(0).unwrap_err(),
        node[1].try_i64().unwrap_err(),
        node[2].try_str().unwrap_err(),
        node[0].try_guid().unwrap_err(),
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    insta::assert_snapshot!(report);
}
