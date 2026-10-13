# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `Clone`, `PartialEq`, `Hash`, `Debug` and dropping of `Node` are no longer recursive:
  a deeply nested tree (e.g. 100 000 levels) no longer overflows the stack. Parsing and
  writing were already non-recursive. The `Debug` output is unchanged.

### Changed

- Documentation states that whitespace between items is not kept on writing and that
  serde mapping is recursive.

## [0.1.0] - 2026-10-13

First release.

### Added

- Streaming parser of the bracket format from any `std::io::Read`: `Node::from_reader`,
  `TryFrom` for `&str`, `String`, `&[u8]`, `Vec<u8>`, `File`, `&Path`, and `FromStr`.
  Deep nesting is parsed without recursion; a leading BOM is skipped.
- Lossless tree: `Node` holds a `Value` (`List`, `Str`, `Atom`) and the position it was
  parsed from. Quoted strings and atoms stay distinct, `#base64:` atoms keep their line breaks.
- Writer in the platform layout: `Display`, `Node::write_to`, `From<&Node>` for `String`
  and `Vec<u8>`. Platform files and `ValueToStringInternal()` output are written back
  byte for byte.
- Accessors: `as_*` returning `Option`, `try_*` returning `AccessError`, generic
  `try_parse::<T>()`, path access `get_path` / `try_get_path`, counted lists
  `try_counted` / `Node::counted`, pre-order `walk`.
- In-place editing: `value_mut`, `as_list_mut`, `get_mut`, `get_path_mut`, `iter_mut`,
  `try_*_mut`, `IndexMut`.
- Errors with positions: `ParseError` for invalid input, `AccessError` with the node position
  and path, and `Error` wrapping both.
- `Values`: streaming reader of comma-separated top-level values with header lines,
  e.g. event log (`*.lgp`) records.
- `serde` feature: positional mapping of lists to user types (`from_node`, `from_str`,
  `from_reader`, `to_node`, `to_string`), `Rest` for trailing items, `Counted<T>` for
  `{count,items...}` lists, the `atom` field helper; `Node` and `Value` implement
  `Serialize` and `Deserialize`, and parsed nodes keep their positions.
- Examples: cluster registry `1cv8wsrv.lst` and event log reader with the `1Cv8.lgf`
  dictionary.
- Benchmarks and performance notes on allocators for Windows and Linux.

[Unreleased]: https://github.com/Toveal/one-brackets/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Toveal/one-brackets/releases/tag/v0.1.0
