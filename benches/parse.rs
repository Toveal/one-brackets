//! Throughput of reading and writing: `cargo bench`.
//!
//! Inputs are built from the bundled samples: an event log repeated to a few megabytes
//! and a managed form repeated as items of one list.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use one_brackets::{Node, Values};

const LOG: &[u8] = include_bytes!("../examples/v8log/20260908000000.lgp");
const FORM: &str = include_str!("../tests/fixtures/form.txt");

/// About 4 MB of log records after the original header.
fn event_log() -> Vec<u8> {
    let body_start = LOG.windows(3).position(|w| w == b"\r\n{").unwrap() + 2;
    let (header, body) = LOG.split_at(body_start);
    let mut log = header.to_vec();
    log.extend_from_slice(body);
    for _ in 0..500 {
        log.extend_from_slice(b",\r\n");
        log.extend_from_slice(body);
    }
    log
}

/// About 3 MB: the form 200 times as items of one list.
fn forms() -> String {
    let form = FORM.trim_start_matches('\u{feff}');
    format!("{{{}}}", vec![form; 200].join(",\r\n"))
}

fn bench(c: &mut Criterion) {
    let log = event_log();
    let forms = forms();
    let forms_node = Node::try_from(forms.as_str()).unwrap();

    let mut group = c.benchmark_group("read");
    group.throughput(Throughput::Bytes(log.len() as u64));
    group.bench_function("event log records", |b| {
        b.iter(|| {
            let mut values = Values::new(black_box(log.as_slice()));
            for _ in 0..3 {
                values.read_line().unwrap();
            }
            values.map(Result::unwrap).count()
        })
    });
    group.throughput(Throughput::Bytes(forms.len() as u64));
    group.bench_function("forms", |b| {
        b.iter(|| Node::try_from(black_box(forms.as_str())).unwrap())
    });
    group.finish();

    let mut group = c.benchmark_group("write");
    group.throughput(Throughput::Bytes(forms.len() as u64));
    group.bench_function("forms", |b| b.iter(|| black_box(&forms_node).to_string()));
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
