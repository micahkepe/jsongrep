/*!
Main binary for jsongrep.
*/

use anyhow::{Context as _, Result};
use clap::{ArgAction, CommandFactory as _, Parser, Subcommand};
use clap_complete::generate;
use colored::Colorize;
use memmap2::{Mmap, MmapOptions};
use serde_json_borrow::Value;
use std::{
    fs::OpenOptions,
    io::{
        self, BufWriter, ErrorKind, IsTerminal as _, Read as _, Write, stdout,
    },
    path::PathBuf,
    str::Utf8Error,
};

use jsongrep::{
    commands,
    query::{DFAQueryEngine, Query, QueryDFA},
    utils::{depth, write_colored_result},
};

/// Query an input JSON document against a jsongrep query.
#[derive(Parser)]
#[command(
    name = "jg",
    version,
    about,
    arg_required_else_help = true,
    long_about = None,
    disable_help_subcommand = true
)]
#[allow(clippy::struct_excessive_bools)]
struct Args {
    /// Optional subcommands
    #[command(subcommand)]
    command: Option<Commands>,
    /// Query string (e.g., "**.name")
    query: Option<String>,
    #[arg(value_name = "FILE")]
    /// Optional path to file. If omitted, reads from STDIN
    input: Option<PathBuf>,
    /// Case insensitive search
    #[arg(short, long, action = ArgAction::SetTrue)]
    ignore_case: bool,
    /// Do not pretty-print the JSON output
    #[arg(long, action = ArgAction::SetTrue)]
    compact: bool,
    /// Display count of number of matches
    #[arg(long, action = ArgAction::SetTrue)]
    count: bool,
    /// Display depth of the input document
    #[arg(long, action = ArgAction::SetTrue)]
    depth: bool,
    /// Do not display matched JSON values
    #[arg(short, long, action = ArgAction::SetTrue)]
    no_display: bool,
    /// Treat the query as a literal field name and search at any depth.
    ///
    /// Searches for the field at any depth, equivalent to `(* | [*])*."<query>"`.
    #[arg(short = 'F', long, action = ArgAction::SetTrue)]
    fixed_string: bool,
    /// Always print the path header, even when output is piped.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_path")]
    with_path: bool,
    /// Never print the path header, even in a terminal.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "with_path")]
    no_path: bool,
    /// Input format (auto-detects from file extension if omitted)
    #[arg(short = 'f', long, default_value = "auto")]
    format: Format,
}

/// Available subcommands for `jg`
#[derive(Subcommand)]
enum Commands {
    #[command(subcommand)]
    /// Generate additional documentation and/or completions
    Generate(GenerateCommand),
}

/// Generate shell completions and man page
#[derive(Subcommand)]
enum GenerateCommand {
    /// Generate shell completions for the given shell to stdout.
    Shell { shell: clap_complete::Shell },
    /// Generate a man page for jg to output directory if specified, else
    /// the current directory.
    Man {
        /// The output directory to write the man pages.
        #[clap(short, long)]
        output_dir: Option<PathBuf>,
    },
}

/// Possible input sources for jsongrep.
enum Input {
    /// Buffered standard input.
    Stdin(String),
    /// A memory-mapped file from the file system. Assumes an immutable handle.
    File(Mmap),
}

impl Input {
    fn to_str(&self) -> Result<&str, Utf8Error> {
        match self {
            Self::Stdin(buffer) => Ok(buffer.as_str()),
            Self::File(mmap) => str::from_utf8(mmap),
        }
    }

    fn to_bytes(&self) -> &[u8] {
        match self {
            Self::Stdin(buf) => buf.as_bytes(),
            Self::File(mmap) => mmap.as_ref(),
        }
    }

    fn to_json_string(&self, format: Format) -> Result<String> {
        match format {
            Format::Jsonl => {
                let text = self.to_str().map_err(|_| {
                    anyhow::anyhow!("JSONL input is not valid UTF-8")
                })?;
                let mut buf = String::from("[");
                let mut first = true;
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if !first {
                        buf.push(',');
                    }
                    buf.push_str(line);
                    first = false;
                }
                buf.push(']');
                Ok(buf)
            }

            // YAML
            #[cfg(feature = "yaml")]
            Format::Yaml => {
                let text = self.to_str().map_err(|_| {
                    anyhow::anyhow!("YAML input is not valid UTF-8")
                })?;
                let value: serde_json::Value =
                    serde_yaml::from_str(text).context("parse YAML input")?;
                serde_json::to_string(&value).context("serialize YAML as JSON")
            }
            #[cfg(not(feature = "yaml"))]
            Format::Yaml => {
                anyhow::bail!(
                    "YAML support not enabled. Rebuild with --features yaml"
                )
            }

            // TOML
            #[cfg(feature = "toml")]
            Format::Toml => {
                let text = self.to_str().map_err(|_| {
                    anyhow::anyhow!("TOML input is not valid UTF-8")
                })?;
                let value: serde_json::Value =
                    toml::from_str(text).context("parse TOML input")?;
                serde_json::to_string(&value).context("serialize TOML as JSON")
            }
            #[cfg(not(feature = "toml"))]
            Format::Toml => {
                anyhow::bail!(
                    "TOML support not enabled. Rebuild with --features toml"
                )
            }

            // CBOR
            #[cfg(feature = "cbor")]
            Format::Cbor => {
                let value: serde_json::Value =
                    ciborium::from_reader(self.to_bytes())
                        .context("parse CBOR input")?;
                serde_json::to_string(&value).context("serialize CBOR as JSON")
            }
            #[cfg(not(feature = "cbor"))]
            Format::Cbor => {
                anyhow::bail!(
                    "CBOR support not enabled. Rebuild with --features cbor"
                )
            }

            // MESSAGEPACK
            #[cfg(feature = "msgpack")]
            Format::Msgpack => {
                let value: serde_json::Value =
                    rmp_serde::from_slice(self.to_bytes())
                        .context("parse MessagePack input")?;
                serde_json::to_string(&value)
                    .context("serialize MessagePack as JSON")
            }
            #[cfg(not(feature = "msgpack"))]
            Format::Msgpack => {
                anyhow::bail!(
                    "MessagePack support not enabled. Rebuild with --features msgpack"
                )
            }

            // Unreachable, someone made an oopsie
            Format::Auto | Format::Json => {
                unreachable!(
                    "to_json_string called with Auto or Json, not needed"
                )
            }
        }
    }
}

/// Parse input content
///
/// # Errors
///
/// Returns early with an error if the file cannot be opened or read. If the input is not a file or
/// piped input, prints the help message and exits with an error.
fn parse_input_content(input: Option<PathBuf>) -> Result<Input> {
    // Parse input content
    if let Some(path) = input {
        let fd =
            OpenOptions::new().read(true).open(&path).with_context(|| {
                format!("Failed to open file {}", path.display())
            })?;

        // SAFETY:
        // mmap is unsafe if the backing file is modified, either by ourselves or by
        // other processes.
        // We will never modify the file, and if other processes do,
        // there is not much we can do about it.
        let map = unsafe {
            MmapOptions::new().map(&fd).with_context(|| {
                format!("Failed to mmap file {}", path.display())
            })?
        };
        Ok(Input::File(map))
    } else {
        if io::stdin().is_terminal() {
            // No piped input and no file specified
            let mut cmd = Args::command();
            cmd.print_help()?;
            anyhow::bail!("No input specified");
        }
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(Input::Stdin(buffer))
    }
}

/// Supported input formats beyond JSON.
#[derive(Debug, Default, Clone, Copy, clap::ValueEnum)]
enum Format {
    #[default]
    Auto,
    Json,
    Jsonl,
    Yaml,
    Toml,
    Cbor,
    Msgpack,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto"),
            Self::Json => write!(f, "JSON"),
            Self::Jsonl => write!(f, "JSONL"),
            Self::Yaml => write!(f, "YAML"),
            Self::Toml => write!(f, "TOML"),
            Self::Cbor => write!(f, "CBOR"),
            Self::Msgpack => write!(f, "MessagePack"),
        }
    }
}

fn detect_format(path: Option<&PathBuf>, explicit: Format) -> Format {
    // Use explicit if user overrode the default.
    if !matches!(explicit, Format::Auto) {
        return explicit;
    }
    let Some(path) = path else {
        // NOTE: we don't support streaming type inference, maybe someday
        return Format::Json;
    };

    match path.extension().and_then(|e| e.to_str()) {
        Some("ndjson" | "jsonl") => Format::Jsonl,
        Some("yaml" | "yml") => Format::Yaml,
        Some("msgpack" | "mp") => Format::Msgpack,
        Some("toml") => Format::Toml,
        Some("cbor") => Format::Cbor,
        _ => Format::Json,
    }
}

/// Entry point for main binary.
///
/// This parses the command line arguments and executes the query. If the input
/// is piped in, it reads from STDIN. The output is printed to STDOUT, with
/// formatting determined by the command line arguments.
fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Some(Commands::Generate(cmd)) => match cmd {
            GenerateCommand::Shell { shell } => {
                let mut cmd = Args::command();
                generate(shell, &mut cmd, "jg", &mut stdout().lock());
            }
            GenerateCommand::Man { output_dir } => {
                commands::generate::generate_man_pages(
                    &Args::command(),
                    output_dir,
                )?;
            }
        },
        None => {
            let raw_query = args.query.ok_or_else(|| {
                anyhow::anyhow!("Query string required unless using subcommand")
            })?;

            let query: Query = if args.fixed_string {
                // -F/--fixed-string: treat the query as a literal field name
                // and search at any depth, equivalent to `(* | [*])*."<literal>"`
                Query::Sequence(vec![
                    Query::KleeneStar(Box::new(Query::Disjunction(vec![
                        Query::FieldWildcard,
                        Query::ArrayWildcard,
                    ]))),
                    Query::Field(raw_query),
                ])
            } else {
                raw_query.parse().with_context(|| "Failed to parse query")?
            };

            let format = detect_format(args.input.as_ref(), args.format);
            let input_content = parse_input_content(args.input)?;

            // For JSON/Auto we borrow directly from the mmap/stdin buffer,
            // preserving the zero-copy path that serde_json_borrow provides.
            // For other formats, we convert to an owned JSON string first
            // and then borrow from that.
            let json_string_owned = match format {
                Format::Json | Format::Auto => None,
                other => Some(input_content.to_json_string(other)?),
            };
            let json_str: &str = match &json_string_owned {
                Some(s) => s.as_str(),
                None => input_content
                    .to_str()
                    .context("File contents are not valid UTF-8")?,
            };

            let json: Value = serde_json::from_str(json_str)
                .with_context(|| format!("Failed to parse as {format}"))?;
            let dfa = if args.ignore_case {
                QueryDFA::from_query_ignore_case(&query)
            } else {
                QueryDFA::from_query(&query)
            };
            let results = DFAQueryEngine::find_with_dfa(&json, &dfa);

            // NOTE: use single, locked stdout handle to avoid interleaving
            let stdout = stdout().lock();

            // Path headers follow ripgrep conventions: shown in terminals,
            // hidden when piped, with explicit overrides.
            let show_path = if args.with_path {
                true
            } else if args.no_path {
                false
            } else {
                stdout.is_terminal()
            };

            let mut writer = BufWriter::new(stdout);

            if args.count {
                writeln!(
                    writer,
                    "{} {}",
                    "Found matches:".bold().blue(),
                    results.len()
                )
                .with_context(|| "Failed to write to stdout")?;
            }

            if args.depth {
                writeln!(
                    writer,
                    "{} {}",
                    "Depth:".bold().blue(),
                    depth(&json)
                )?;
            }

            if !args.no_display {
                let pretty = !args.compact;
                for result in &results {
                    write_colored_result(
                        &mut writer,
                        result.value,
                        &result.path,
                        pretty,
                        show_path,
                    )?;
                }
            }

            match writer.flush() {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::BrokenPipe => {}
                Err(err) => return Err(err.into()),
            }
        }
    }

    Ok(())
}
