//! Reads a 1C event log file (`*.lgp`, format "1CV8LOG(ver 2.0)").
//!
//! Format description: <https://infostart.ru/1c/articles/182061/> (with corrections
//! for 8.3 from its comments).
//!
//! The file is a three-line header followed by comma-separated records, read one at a time
//! with `Values`. Users, computers, events and the like are stored as indices into
//! the dictionary `1Cv8.lgf`; when it lies next to the `.lgp` file, names are shown.
//!
//! On the bundled sample:
//!     cargo run --example v8log --features serde
//! On a real log:
//!     cargo run --example v8log --features serde -- "C:\...\1Cv8Log\20260908000000.lgp"

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use one_brackets::{Node, Rest, Values};
use serde::Deserialize;

// Logs are large and parsing makes many small allocations: a fast allocator
// saves 20-35% of the time, more on Windows. See "Performance" in the crate docs.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const SAMPLE: &[u8] = include_bytes!("20260908000000.lgp");
const HEADER: &str = "1CV8LOG(ver 2.0)";

/// One event log record. All fields are listed to document the layout,
/// the example prints only some of them.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Record {
    /// `YYYYMMDDhhmmss`, local time of the server.
    date: u64,
    transaction_status: TransactionStatus,
    transaction: Transaction,
    user: usize,
    computer: usize,
    application: usize,
    connection: u64,
    event: usize,
    severity: Severity,
    comment: String,
    metadata: usize,
    data: Data,
    data_presentation: String,
    server: usize,
    main_port: usize,
    additional_port: usize,
    session: u64,
    /// Followed in `rest` by that many metadata indices and a final item, usually `{0}`.
    additional_metadata_count: usize,
    #[serde(default)]
    rest: Rest,
}

impl Record {
    /// Indices of additional metadata, listed after their count.
    fn additional_metadata(&self) -> impl Iterator<Item = usize> + '_ {
        self.rest
            .iter()
            .take(self.additional_metadata_count)
            .filter_map(Node::as_parsed)
    }
}

/// Letters as in 8.3; in 8.2 `U` meant committed, `R` unfinished and `C` rolled back.
/// Names follow the platform's XML export of the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum TransactionStatus {
    #[serde(rename = "N")]
    NotApplicable,
    #[serde(rename = "C")]
    Committed,
    #[serde(rename = "R")]
    RolledBack,
    #[serde(rename = "U")]
    InProgress,
}

/// `{start,offset}`, both hexadecimal; `{0,0}` outside a transaction.
#[derive(Debug, Deserialize)]
struct Transaction {
    /// Ten-thousandths of a second since 0001-01-01.
    start: String,
    /// Byte offset of the transaction's first record in this file, BOM included.
    /// It identifies the transaction: all its records carry the same value.
    offset: String,
}

impl Transaction {
    /// Start time as `YYYY-MM-DD hh:mm:ss`, if there is a transaction.
    fn start(&self) -> Option<String> {
        let start = u64::from_str_radix(&self.start, 16)
            .ok()
            .filter(|&t| t != 0)?;
        Some(format_seconds(start / 10_000))
    }

    fn offset(&self) -> Option<u64> {
        u64::from_str_radix(&self.offset, 16).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
enum Severity {
    #[serde(rename = "E")]
    Error,
    #[serde(rename = "W")]
    Warning,
    #[serde(rename = "I")]
    Information,
    #[serde(rename = "N")]
    Note,
}

/// The data field: a type tag followed by the value, e.g. `{"S","text"}` or `{"U"}`.
#[derive(Debug, Deserialize)]
enum Data {
    #[serde(rename = "U")]
    Undefined,
    #[serde(rename = "S")]
    String(String),
    #[serde(rename = "N")]
    Number(String),
    #[serde(rename = "B")]
    Boolean(bool),
    #[serde(rename = "D")]
    Date(u64),
    /// A reference `{"R",class:id}`.
    #[serde(rename = "R")]
    Reference(String),
    /// A structure or an array of values.
    #[serde(rename = "P")]
    Composite(Node),
    /// Value tables (`"Z"`) and other kinds the example does not decode.
    #[serde(other)]
    Other,
}

impl fmt::Display for Data {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Data::Undefined => Ok(()),
            Data::String(s) | Data::Number(s) | Data::Reference(s) => f.write_str(s),
            Data::Boolean(b) => write!(f, "{b}"),
            Data::Date(d) => f.write_str(&format_date(*d)),
            Data::Composite(node) => {
                // Show the plain values inside: `{"S","x"}` pairs at any depth.
                let mut values = Vec::new();
                node.walk(|_, item| {
                    if let Some([tag, value]) = item.as_list()
                        && matches!(tag.as_quoted(), Some("S" | "N" | "B" | "D" | "R"))
                        && let Some(text) = value.as_str()
                        && !text.is_empty()
                    {
                        values.push(text.to_owned());
                    }
                });
                f.write_str(&values.join(", "))
            }
            Data::Other => f.write_str("(not decoded)"),
        }
    }
}

/// Names from `1Cv8.lgf`: records `{kind,...,name,index}`.
#[derive(Default)]
struct Dictionary(HashMap<(u8, usize), String>);

impl Dictionary {
    const USER: u8 = 1;
    const COMPUTER: u8 = 2;
    const APPLICATION: u8 = 3;
    const EVENT: u8 = 4;

    fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut values = Values::new(File::open(path)?);
        check_header(&mut values)?;
        let mut names = HashMap::new();
        for node in values {
            let node = node?;
            let items = node.try_list()?;
            // Kinds above 8 hold other data (data separation and the like).
            let kind: u8 = node.try_get(0)?.try_parse()?;
            if let (1..=8, [.., name, index]) = (kind, items) {
                names.insert((kind, index.try_parse()?), name.try_str()?.to_owned());
            }
        }
        Ok(Dictionary(names))
    }

    fn name(&self, kind: u8, index: usize) -> String {
        match self.0.get(&(kind, index)) {
            Some(name) => name.clone(),
            None if index == 0 => "-".to_owned(),
            None => format!("#{index}"),
        }
    }
}

fn check_header<R: Read>(values: &mut Values<R>) -> Result<(), Box<dyn std::error::Error>> {
    let format = values.read_line()?.unwrap_or_default();
    if format != HEADER {
        return Err(format!("expected header {HEADER:?}, found {format:?}").into());
    }
    let _log_id = values.read_line()?;
    let _empty = values.read_line()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os().nth(1);
    let (mut values, dictionary): (Values<Box<dyn Read>>, _) = match &path {
        Some(path) => {
            let path = Path::new(path);
            let lgf = path.with_file_name("1Cv8.lgf");
            let dictionary = if lgf.exists() {
                Dictionary::load(&lgf)?
            } else {
                Dictionary::default()
            };
            (Values::new(Box::new(File::open(path)?)), dictionary)
        }
        None => (Values::new(Box::new(SAMPLE)), Dictionary::default()),
    };
    check_header(&mut values)?;

    let mut by_severity = BTreeMap::new();
    for node in values {
        let node = node?;
        // A record the model does not fit is reported with its position, the rest are read on.
        let record: Record = match one_brackets::from_node(&node) {
            Ok(record) => record,
            Err(e) => {
                eprintln!("skipped record: {e}");
                continue;
            }
        };
        *by_severity.entry(record.severity).or_insert(0) += 1;

        let mut line = format!(
            "{} {:?} event {} user {} computer {} app {} session {}",
            format_date(record.date),
            record.severity,
            dictionary.name(Dictionary::EVENT, record.event),
            dictionary.name(Dictionary::USER, record.user),
            dictionary.name(Dictionary::COMPUTER, record.computer),
            dictionary.name(Dictionary::APPLICATION, record.application),
            record.session,
        );
        if record.transaction_status != TransactionStatus::NotApplicable
            && let (Some(start), Some(offset)) =
                (record.transaction.start(), record.transaction.offset())
        {
            // Node positions count bytes from the start of the file, like the log does.
            let first = node.position().map(|p| p.offset) == Some(offset);
            line += &format!(
                " transaction {start} @{offset}{} ({:?})",
                if first { " begins" } else { "" },
                record.transaction_status
            );
        }
        let metadata: Vec<usize> = record.additional_metadata().collect();
        if !metadata.is_empty() {
            line += &format!(" additional metadata {metadata:?}");
        }
        let data = record.data.to_string();
        if !data.is_empty() {
            line += &format!(" data [{data}]");
        }
        if let Some(comment) = record.comment.lines().find(|l| !l.trim().is_empty()) {
            line += &format!(" comment {:?}", comment.trim());
        }
        println!("{line}");
    }
    println!("{by_severity:?}");
    Ok(())
}

/// `20260908233443` → `2026-09-08 23:34:43`.
fn format_date(date: u64) -> String {
    let s = format!("{date:014}");
    format!(
        "{}-{}-{} {}:{}:{}",
        &s[0..4],
        &s[4..6],
        &s[6..8],
        &s[8..10],
        &s[10..12],
        &s[12..14]
    )
}

/// Seconds since 0001-01-01 → `YYYY-MM-DD hh:mm:ss`.
fn format_seconds(seconds: u64) -> String {
    const DAYS_TO_UNIX_EPOCH: i64 = 719_162;
    let days = (seconds / 86_400) as i64 - DAYS_TO_UNIX_EPOCH;
    let (year, month, day) = civil_from_days(days);
    let time = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

/// Days since 1970-01-01 → (year, month, day); Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
