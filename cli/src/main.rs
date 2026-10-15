//! `one-brackets`: converts 1C:Enterprise bracket files to JSON, YAML and XML and back.

mod json;
mod tree;
mod xml;
mod yaml;

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use one_brackets::{Node, Values};

const BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Parser)]
#[command(
    version,
    about = "Converts 1C:Enterprise bracket files to JSON, YAML and XML and back"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a bracket file to JSON, YAML or XML
    Export(Export),
    /// Convert JSON, YAML or XML back to a bracket file
    Import(Import),
    /// Check that bracket files are well-formed
    Check {
        /// Bracket files (`-` for standard input)
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Rewrite a bracket file in the platform layout
    Fmt {
        /// Bracket file (`-` for standard input)
        input: PathBuf,
        /// Output file, may be the input file; standard output by default
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Print the node at a path of item indices
    Get(Get),
}

#[derive(Args)]
struct Export {
    /// Bracket file (`-` for standard input)
    input: PathBuf,
    /// Output file; standard output by default
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Output format; by default taken from the output file extension, JSON otherwise
    #[arg(long, value_enum)]
    to: Option<Format>,
    /// Write the value on one line
    #[arg(long)]
    compact: bool,
    /// JSON only: write atoms that are not numbers as strings; cannot be imported back
    #[arg(long)]
    plain: bool,
    /// Read comma-separated values one at a time, such as event log records (`*.lgp`);
    /// writes JSON Lines, YAML documents or XML `<values>`
    #[arg(long)]
    lines: bool,
    /// With `--lines`: skip that many header lines first (3 for `*.lgp`)
    #[arg(long, requires = "lines", default_value_t = 0)]
    skip_lines: usize,
}

#[derive(Args)]
struct Import {
    /// JSON, YAML or XML file (`-` for standard input)
    input: PathBuf,
    /// Output file; standard output by default
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Input format; by default taken from the input file extension
    #[arg(long, value_enum)]
    from: Option<Format>,
    /// Do not start the output with a UTF-8 BOM (files of the platform have one)
    #[arg(long)]
    no_bom: bool,
}

#[derive(Args)]
struct Get {
    /// Bracket file (`-` for standard input)
    input: PathBuf,
    /// Item indices separated by dots, such as `1.0.2`; empty for the whole file
    path: String,
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Brackets)]
    to: Format,
    /// Print a string or an atom as bare text
    #[arg(long, conflicts_with = "to")]
    raw: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Brackets,
    Json,
    Yaml,
    Xml,
}

impl Format {
    fn from_extension(path: &Path) -> Option<Format> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "json" | "jsonl" => Some(Format::Json),
            "yaml" | "yml" => Some(Format::Yaml),
            "xml" => Some(Format::Xml),
            _ => None,
        }
    }

    fn read(self, text: &str) -> Result<Node> {
        match self {
            Format::Brackets => Ok(Node::try_from(text)?),
            Format::Json => json::read(text),
            Format::Yaml => yaml::read(text),
            Format::Xml => xml::read(text),
        }
    }

    /// Writes `node`; all formats but brackets end with a line break.
    fn write(self, node: &Node, out: &mut dyn Write, compact: bool, plain: bool) -> io::Result<()> {
        match self {
            Format::Brackets => node.write_to(out),
            Format::Json => json::write(node, out, compact, plain),
            Format::Yaml => yaml::write(node, out, compact),
            Format::Xml => xml::write(node, out, compact),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Export(args) => export(args).map(|()| true),
        Command::Import(args) => import(args).map(|()| true),
        Command::Check { inputs } => Ok(check(&inputs)),
        Command::Fmt { input, output } => fmt(&input, output.as_deref()).map(|()| true),
        Command::Get(args) => get(args).map(|()| true),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        // The reader of the output has gone, e.g. `| head`.
        Err(e) if is_broken_pipe(&e) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn export(args: Export) -> Result<()> {
    let format = args
        .to
        .or_else(|| args.output.as_deref().and_then(Format::from_extension))
        .unwrap_or(Format::Json);
    if args.plain && format != Format::Json {
        bail!("--plain applies to JSON only");
    }
    if args.lines {
        let mut out = create(args.output.as_deref())?;
        export_values(&args, format, &mut out)?;
        return finish(out, args.output.as_deref());
    }
    let (node, _) = read_brackets(&args.input)?;
    let mut out = create(args.output.as_deref())?;
    format.write(&node, &mut out, args.compact, args.plain)?;
    finish(out, args.output.as_deref())
}

/// Writes values as they are read: an event log may not fit in memory.
fn export_values(args: &Export, format: Format, out: &mut dyn Write) -> Result<()> {
    let input = &args.input;
    let read_error = || format!("failed to read `{}`", input.display());
    let mut values = Values::new(open(input)?);
    for _ in 0..args.skip_lines {
        values.read_line().with_context(read_error)?;
    }
    match format {
        Format::Json => {
            for value in values {
                json::write(&value.with_context(read_error)?, out, true, args.plain)?;
            }
        }
        Format::Yaml => {
            for value in values {
                let value = value.with_context(read_error)?;
                out.write_all(b"---\n")?;
                yaml::write(&value, out, args.compact)?;
            }
        }
        Format::Xml => {
            let mut writer = xml::ValuesWriter::start(out, args.compact)?;
            for value in values {
                writer.value(&value.with_context(read_error)?)?;
            }
            writer.finish()?;
        }
        Format::Brackets => bail!("--lines writes JSON, YAML or XML"),
    }
    Ok(())
}

fn import(args: Import) -> Result<()> {
    let format = match args.from {
        Some(format) => format,
        None => Format::from_extension(&args.input)
            .context("cannot tell the input format from the file extension: pass --from")?,
    };
    let text = read_text(&args.input)?;
    let node = format
        .read(&text)
        .with_context(|| format!("failed to read `{}`", args.input.display()))?;
    write_brackets(&node, !args.no_bom, args.output.as_deref())
}

/// Prints the result for each file; `false` if any is broken.
fn check(inputs: &[PathBuf]) -> bool {
    let mut valid = true;
    for input in inputs {
        match read_brackets(input) {
            Ok(_) => println!("{}: ok", input.display()),
            Err(e) => {
                eprintln!("{}: {:#}", input.display(), e.root_cause());
                valid = false;
            }
        }
    }
    valid
}

fn fmt(input: &Path, output: Option<&Path>) -> Result<()> {
    let (node, bom) = read_brackets(input)?;
    write_brackets(&node, bom, output)
}

fn get(args: Get) -> Result<()> {
    let path = if args.path.is_empty() {
        Vec::new()
    } else {
        args.path
            .split('.')
            .map(str::parse)
            .collect::<Result<Vec<usize>, _>>()
            .with_context(|| {
                format!(
                    "invalid path `{}`: expected indices separated by dots",
                    args.path
                )
            })?
    };
    let (node, _) = read_brackets(&args.input)?;
    let found = node.try_get_path(&path)?;
    let mut out = create(None)?;
    if args.raw {
        let Some(text) = found.as_str() else {
            bail!(
                "the node at `{}` is a list: --raw prints strings and atoms",
                args.path
            );
        };
        writeln!(out, "{text}")?;
    } else {
        args.to.write(found, &mut out, false, false)?;
        if args.to == Format::Brackets {
            writeln!(out)?;
        }
    }
    finish(out, None)
}

fn open(path: &Path) -> Result<Box<dyn BufRead>> {
    if path == Path::new("-") {
        return Ok(Box::new(io::stdin().lock()));
    }
    let file = File::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
    Ok(Box::new(BufReader::new(file)))
}

/// Reads a bracket file; also tells whether it starts with a BOM.
fn read_brackets(path: &Path) -> Result<(Node, bool)> {
    let mut input = open(path)?;
    let read_error = || format!("failed to read `{}`", path.display());
    let bom = input.fill_buf().with_context(read_error)?.starts_with(BOM);
    let node = Node::from_reader(input).with_context(read_error)?;
    Ok((node, bom))
}

/// Reads a whole text file without its BOM.
fn read_text(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    open(path)?
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    if bytes.starts_with(BOM) {
        bytes.drain(..BOM.len());
    }
    String::from_utf8(bytes).with_context(|| format!("`{}` is not valid UTF-8", path.display()))
}

fn write_brackets(node: &Node, bom: bool, output: Option<&Path>) -> Result<()> {
    let mut out = create(output)?;
    if bom {
        out.write_all(BOM)?;
    }
    node.write_to(&mut out)?;
    finish(out, output)
}

/// The file is created only after the input is read, so it may be the input file.
fn create(path: Option<&Path>) -> Result<Box<dyn Write>> {
    match path {
        None => Ok(Box::new(BufWriter::new(io::stdout().lock()))),
        Some(path) if path == Path::new("-") => Ok(Box::new(BufWriter::new(io::stdout().lock()))),
        Some(path) => {
            let file = File::create(path)
                .with_context(|| format!("failed to create `{}`", path.display()))?;
            Ok(Box::new(BufWriter::new(file)))
        }
    }
}

fn finish(mut out: Box<dyn Write>, path: Option<&Path>) -> Result<()> {
    out.flush().with_context(|| match path {
        Some(path) => format!("failed to write `{}`", path.display()),
        None => "failed to write to standard output".to_owned(),
    })
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    })
}
