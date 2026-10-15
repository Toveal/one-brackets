# one-brackets-cli

**English** | [Русский](README.ru.md)

Command-line tool for 1C:Enterprise bracket files (`{1,"text",{nested,list}}`), built on the
[`one-brackets`](https://crates.io/crates/one-brackets) library. It converts them to JSON, YAML and XML
and back without losses: export followed by import gives the original file byte for byte.

```sh
cargo install one-brackets-cli
```

## Commands

```sh
one-brackets export Form.txt -o Form.yaml        # format from the extension
one-brackets export Form.txt --to json --compact # to standard output, on one line
one-brackets import Form.yaml -o Form.txt        # writes a BOM like the platform; --no-bom to skip it
one-brackets check *.txt                         # exit code 1 if any file is broken
one-brackets fmt Form.txt -o Form.txt            # rewrite in the platform layout
one-brackets get Form.txt 1.0.2                  # node by item indices; --raw, --to json
```

`-` reads standard input or writes standard output. Errors carry the line and column:

```text
error: failed to read `Form.json`: atom " a" at line 12, column 7 starts with whitespace: use a quoted string instead
```

Event logs (`*.lgp`) are read one record at a time and written as JSON Lines, YAML documents
or XML `<values>`:

```sh
one-brackets export 20260908000000.lgp --lines --skip-lines 3 > records.jsonl
```

## Mapping

The bracket format has three kinds of values: lists, quoted strings and unquoted atoms
(numbers, GUIDs, literals). Each format keeps them apart.

| Brackets | JSON | YAML | XML |
|---|---|---|---|
| `{1,2}` | `[1, 2]` | sequence | `<list>…</list>` |
| `"text"` | `"text"` | `"text"` (always quoted) | `<str>text</str>` |
| `42`, `-0.5` | `42`, `-0.5` (the text is kept) | `42` | `<atom>42</atom>` |
| `N`, GUID | `{"atom": "N"}` | `N` (plain) | `<atom>N</atom>` |
| empty atom | `null` | `~` | `<atom/>` |

- **JSON.** An atom is written as a number only when it is a JSON number; `007` and `1.` are
  `{"atom": …}`. `{"str": …}` is read as a string. `--plain` writes other atoms as strings:
  easier to read, but the result cannot be imported back.
- **YAML.** Plain scalars are atoms, quoted and block scalars are strings, whatever they look like.
  Atoms that cannot be written plain (base64 with line breaks, `-`, `a:`, `[x]` and so on) are
  tagged: `!atom "…"`. `!str` and `!!str` mark strings. Mappings, aliases and other tags are errors.
- **XML.** `\r` and control characters are written as character references; whitespace between
  elements is ignored, text inside `<str>` and `<atom>` is kept as is.

Lists of scalars are written on one line, other lists one item per line; `--compact` writes
everything on one line.

On import, an atom that the bracket parser would read differently (with `,` or `}`, or starting
with `{`, a quote or whitespace) is an error. Unpaired surrogates, which the platform allows in
strings, are kept in JSON as `\uXXXX` escapes; YAML and XML cannot hold them, and export to them
fails.

## Depth

Conversion is not recursive, so deeply nested files do not overflow the stack. One exception:
the YAML parser reads at most 255 levels of flow style (`[[…]]`). Block style, which `export`
writes by default, has no such limit; `--compact` YAML deeper than that cannot be imported back.

## License

Licensed under either of [Apache License, Version 2.0](https://github.com/Toveal/one-brackets/blob/main/LICENSE-APACHE) or
[MIT license](https://github.com/Toveal/one-brackets/blob/main/LICENSE-MIT) at your option.
