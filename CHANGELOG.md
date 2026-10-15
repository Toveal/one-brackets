# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Reading follows the reader of the platform (8.3.27) to the byte.

### Changed

- **Breaking:** `Value::Str` and `Value::Atom` hold `Text` instead of `String`. `Text` keeps
  unpaired surrogates, which the platform allows in strings; `as_str()` gives `None` for such
  text and `to_string_lossy()` replaces them with U+FFFD. `Node::atom` and `Node::string`
  take `impl Into<Text>`, so `&str` and `String` work as before.
- **Breaking:** an atom runs up to the next `,` or `}` (or the end of input): whitespace,
  line breaks, `{` and `"` inside it are part of its text, trailing whitespace included.
  Before, an atom ended at whitespace or `{`, so `{1 2}` was an error and is now the atom `1 2`.
  `as_atom()` gives the text as written; the typed accessors (`as_i64`, `try_parse`,
  `as_bool`, `as_guid`, serde) read it trimmed.
- **Breaking:** characters outside the Basic Multilingual Plane in quoted strings are written
  as escapes of their UTF-16 code units, as the platform writes them: `"a😀"` is written
  `"a"\d83d"\de00"`.
- Whitespace between items is any whitespace of ICU `u_isspace`, as in the platform:
  U+00A0, U+2000..U+200A, U+3000 and the like, not only ASCII whitespace.
- `Node::into_string` replaces unpaired surrogates with U+FFFD; `into_text` keeps them.

### Added

- `Text`, `Node::as_text`, `Node::try_text`, `Node::into_text`, `Node::to_string_lossy`,
  `Node::as_atom_trimmed`.
- Escapes `"\xxxx` in quoted strings: a UTF-16 code unit in hex, which the platform writes
  for each surrogate. `ParseError::InvalidEscape` reports a broken one.
- CLI: unpaired surrogates are kept in JSON as `\uXXXX` escapes; YAML and XML report them
  as an error. Atoms read from JSON, YAML and XML may hold whitespace and `{`.

## [0.2.0] - 2026-10-15

### Added

- Command-line tool `one-brackets` (package `one-brackets-cli`): lossless export of bracket
  files to JSON, YAML and XML and import back, streaming export of event log records,
  `check`, `fmt` and `get` commands.

### Fixed

- `Clone`, `PartialEq`, `Hash`, `Debug` and dropping of `Node` are no longer recursive:
  a deeply nested tree (e.g. 100 000 levels) no longer overflows the stack. Parsing and
  writing were already non-recursive. The `Debug` output is unchanged.

### Removed

- `TryFrom<String>`, `TryFrom<Vec<u8>>` and `TryFrom<File>` for `Node`: use `&str`, `&[u8]`
  and `Node::from_reader` instead.
- `From<Node>` for `String` and `Vec<u8>`: use `to_string()` or `From<&Node>`.

### Changed

- Parsing is about 10% faster: list items are collected in one reused buffer and moved
  into a `Vec` of the exact size, and single bytes and separators skip the generic scanning.
  Parsed trees are also more compact, which makes writing about 15% faster.
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

[Unreleased]: https://github.com/Toveal/one-brackets/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Toveal/one-brackets/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Toveal/one-brackets/releases/tag/v0.1.0
