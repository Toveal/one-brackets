# one-brackets

**English** | [Русский](README.ru.md)

Reader and writer for the 1C:Enterprise bracket format: `{1,"text",{nested,list}}`.

The format is used across the 1C platform: configuration files, forms, cluster
settings (`1cv8wsrv.lst`), event logs (`*.lgp`) and `ValueToStringInternal()` output.

- **Lossless.** Quoted strings and unquoted values are kept apart, so writing reproduces
  the platform's text byte for byte. Checked against files written by the platform itself.
- **Any source.** Reads from any `std::io::Read`, streaming; deep nesting needs no recursion.
- **Clear errors.** Parse and access errors carry the line, column and path to the node.
- **Editable.** Change nodes in place and write the file back.
- **serde.** Map lists to your types by field order (`serde` feature).

## Usage

```toml
[dependencies]
one-brackets = "0.1"
```

Parse, read and edit:

```rust
use one_brackets::Node;

fn main() -> one_brackets::Result<()> {
    let mut node: Node = "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,\"Demo\"}".parse()?;

    assert_eq!(node.try_get(0)?.try_parse::<u32>()?, 2);
    assert!(node[1].as_guid().is_some());
    assert_eq!(node[2].as_quoted(), Some("Demo"));

    node[2] = Node::string("Renamed");
    assert_eq!(node.to_string(), "{2,c2f34349-b1e0-4060-8abf-3c59f38ac69f,\"Renamed\"}");

    let err = Node::try_from("{1,\n{2").unwrap_err();
    assert_eq!(
        err.to_string(),
        "unexpected end of input at line 2, column 3: list opened at line 2, column 1 is not closed",
    );
    Ok(())
}
```

Files are read with `Node::try_from(Path::new("Form.txt"))` or `Node::from_reader(reader)`
and written with `node.write_to(writer)` or `node.to_string()`.

### Mapping to your types

With `features = ["serde"]` list items map to fields in declaration order.
`Rest` keeps the items after the last field, so unknown data survives a round trip:

```rust
use one_brackets::Rest;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct Cluster {
    #[serde(with = "one_brackets::atom")]
    id: String,
    name: String,
    port: u16,
    #[serde(default)]
    rest: Rest,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = "{e75cc2cd-9060-434e-972f-e42cc4cba705,\"Local cluster\",1541,\"SRV1C\",0}";
    let cluster: Cluster = one_brackets::from_str(text)?;
    assert_eq!(cluster.port, 1541);
    assert_eq!(cluster.rest.len(), 2);
    assert_eq!(one_brackets::to_string(&cluster)?, text);
    Ok(())
}
```

`Counted<T>` reads the common `{count,item1,item2,...}` lists. Errors point at the node:

```text
expected u8, found value `300` at line 2, column 3 (path [1, 0, 0]): number too large to fit in target type
```

### Event logs and other value sequences

`Values` streams comma-separated top-level values, such as the records of an event log,
keeping only the current one in memory:

```rust
use one_brackets::Values;

fn main() -> Result<(), one_brackets::ParseError> {
    let log = "1CV8LOG(ver 2.0)\r\n794d72bc-c31d-4245-b5a3-c765d53177ea\r\n\r\n{20260908233443,N},\r\n{20260908233445,U}";
    let mut values = Values::new(log.as_bytes());
    for _ in 0..3 {
        values.read_line()?;
    }
    for record in values {
        let record = record?;
        println!("{} at {}", record[0], record.position().unwrap());
    }
    Ok(())
}
```

## Examples

- [`cluster`](examples/cluster/main.rs): the cluster registry `1cv8wsrv.lst` with serde.
- [`v8log`](examples/v8log/main.rs): event log records, names from the `1Cv8.lgf` dictionary.

```sh
cargo run --example v8log --features serde -- path/to/20260908000000.lgp
```

## Performance

Parsing allocates a `String` per value and a `Vec` per list, so large inputs are bound
by the memory allocator. The library does not pick one; the application can switch to
[mimalloc](https://crates.io/crates/mimalloc), which works on Windows, Linux and macOS:

```toml
[dependencies]
mimalloc = "0.1"
```

```rust,ignore
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

Best of 7 runs, 3 MB of forms and 4 MB of event log records:

| OS | Allocator | Forms | Event log |
|---|---|---|---|
| Windows | system | 85 ms | 67 ms |
| Windows | mimalloc | 32 ms (2.7×) | 44 ms (1.5×) |
| Linux (glibc 2.39) | system | 48 ms | 49 ms |
| Linux (glibc 2.39) | mimalloc | 32 ms (1.5×) | 40 ms (1.2×) |
| Linux (glibc 2.39) | jemalloc | 36 ms (1.3×) | 39 ms (1.25×) |

The gain is largest on Windows, whose system allocator is the slowest of these;
on Linux jemalloc is on par with mimalloc but does not build for Windows MSVC.
For event logs prefer `Values` over parsing one huge list, and process files
in parallel: logs are split into files by period. `cargo bench` measures reading and writing.

## Minimum Rust version

1.85 (edition 2024).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.
