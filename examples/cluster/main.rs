//! Reads `1cv8wsrv.lst`, the list of clusters registered on a 1C central server.
//!
//! On the bundled sample:
//!     cargo run --example cluster --features serde
//! On a real server:
//!     cargo run --example cluster --features serde -- "C:\Program Files\1cv8\srvinfo\1cv8wsrv.lst"

use std::path::Path;

use one_brackets::{Counted, Node, Rest};
use serde::{Deserialize, Serialize};

const SAMPLE: &str = include_str!("1cv8wsrv.lst");

/// Root of `1cv8wsrv.lst`.
#[derive(Debug, Deserialize, Serialize)]
struct Registry {
    clusters: Counted<Cluster>,
    admins: Counted<Node>,
    /// Items the example does not model, kept so the file can be written back.
    #[serde(default)]
    rest: Rest,
}

#[derive(Debug, Deserialize, Serialize)]
struct Cluster {
    #[serde(with = "one_brackets::atom")]
    id: String,
    name: String,
    port: u16,
    host: String,
    #[serde(default)]
    rest: Rest,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node: Node = match std::env::args_os().nth(1) {
        Some(path) => Node::try_from(Path::new(&path))?,
        None => SAMPLE.parse()?,
    };
    let registry: Registry = one_brackets::from_node(&node)?;

    for cluster in &registry.clusters {
        println!(
            "{:?} on {}:{} (id {}, {} more settings)",
            cluster.name,
            cluster.host,
            cluster.port,
            cluster.id,
            cluster.rest.len()
        );
    }
    println!("central server administrators: {}", registry.admins.len());

    // Unmodelled items were kept in `rest`, so writing reproduces the input.
    assert_eq!(one_brackets::to_node(&registry)?, node);
    println!("round trip: ok");
    Ok(())
}
