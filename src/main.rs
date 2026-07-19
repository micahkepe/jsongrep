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
    query::{Query, QueryDFA},
    utils::{WriteOptions, depth, write_colored_result},
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
#[expect(clippy::struct_excessive_bools, reason = "CLI flags.")]
struct Args {
    /// Optional subcommands.
    #[command(subcommand)]
    command: Option<Commands>,
    /// Query string (e.g., "**.name").
    query: Option<String>,
    #[arg(value_name = "FILE")]
    /// Optional path to file. If omitted, reads from STDIN.
    input: Option<PathBuf>,
    /// Case insensitive search.
    #[arg(short, long, action = ArgAction::SetTrue)]
    ignore_case: bool,
    /// Do not pretty-print the JSON output.
    #[arg(long, action = ArgAction::SetTrue)]
    compact: bool,
    /// Print matched strings without JSON quotes or escaping (like `jq -r`).
    ///
    /// Useful in shell pipelines: `TOKEN=$(... | jg -r token)`. Non-string
    /// values print as JSON, unchanged.
    #[arg(short = 'r', long, action = ArgAction::SetTrue)]
    raw_output: bool,
    /// Display count of number of matches.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "depth")]
    count: bool,
    /// Display depth of the input document.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "count")]
    depth: bool,
    /// Machine-readable output: strip labels and colors, print one JSON
    /// value per line (implies --compact).
    #[arg(long, action = ArgAction::SetTrue)]
    porcelain: bool,
    /// Do not display matched JSON values.
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
    /// Input format (auto-detects from file extension if omitted).
    #[arg(short = 'f', long, default_value = "auto")]
    format: Format,
}

/// Available subcommands for `jg`.
#[derive(Subcommand)]
enum Commands {
    #[command(subcommand)]
    /// Generate additional documentation and/or completions.
    Generate(GenerateCommand),
}

/// Generate shell completions and man page.
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

/// Minimum file size for which memory-mapping is attempted.
///
/// For small files, it is likely that a single read call is at least as fast or faster than mmap
/// (mmap setup and page-fault overhead dominate for small files) and avoids mmap's file-truncation
/// hazards.
///
/// NOTE: in the future when globbing (<https://github.com/micahkepe/jsongrep/issues/33>) and
/// recursive searching are enabled, can look into other heuristics for performance.
///
/// See: <https://burntsushi.net/ripgrep/#mechanics>.
const MMAP_MIN_FILE_SIZE: u64 = 1 << 20; // 1 MiB

/// Ceiling on DFA states during query compilation. Subset construction is
/// worst-case exponential in the query length, so a short adversarial query
/// could otherwise consume unbounded time and memory; past this budget `jg`
/// reports "query is too complex" instead. 2^18 states keeps the worst-case
/// abort around a second while remaining orders of magnitude beyond any
/// realistic query (which needs tens of states).
const DEFAULT_MAX_DFA_STATES: usize = 1 << 18;

/// Possible input sources for jsongrep.
///
/// Input is kept as raw bytes so that binary formats (CBOR, `MessagePack`)
/// work from any source; UTF-8 is validated only when a text format needs
/// it.
enum Input {
    /// Fully buffered input: stdin, small files, non-regular files (FIFOs,
    /// process substitution), and the fallback when mmap fails.
    Buffer(Vec<u8>),
    /// A memory-mapped file from the file system. Assumes an immutable handle.
    File(Mmap),
}

impl Input {
    fn to_str(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(self.to_bytes())
    }

    fn to_bytes(&self) -> &[u8] {
        match self {
            Self::Buffer(buf) => buf.as_slice(),
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

/// Parse input content, from the input path buffer if provided, else try STDIN.
///
/// # Errors
///
/// Returns early with an error if the file cannot be opened or read. If the input is not a file or
/// piped input, prints the help message and exits with an error.
fn parse_input_content(input: Option<PathBuf>) -> Result<Input> {
    if let Some(path) = input {
        let mut fd =
            OpenOptions::new().read(true).open(&path).with_context(|| {
                format!("Failed to open file {}", path.display())
            })?;

        // Only mmap large regular files. Non-regular files (FIFOs, process
        // substitution like `jg q <(curl ...)`, character devices) cannot be
        // mapped, and small files gain nothing from mapping. If mapping
        // fails anyway, fall back to a plain read instead of erroring.
        let metadata = fd.metadata().ok();
        let is_large_regular_file = metadata
            .as_ref()
            .is_some_and(|m| m.is_file() && m.len() >= MMAP_MIN_FILE_SIZE);

        if is_large_regular_file {
            // SAFETY:
            // mmap is unsafe if the backing file is modified, either by
            // ourselves or by other processes. We will never modify the
            // file, and if other processes do, there is not much we can do
            // about it.
            if let Ok(map) = unsafe { MmapOptions::new().map(&fd) } {
                return Ok(Input::File(map));
            }
        }

        // Capacity hint capped at the mmap threshold: only files below it
        // (or rare mmap fallbacks) reach this path, and a stale/huge stat
        // length must not trigger a giant allocation.
        let capacity_hint = metadata
            .map_or(0, |m| m.len().min(MMAP_MIN_FILE_SIZE))
            .try_into()
            .unwrap_or(0);
        let mut buffer = Vec::with_capacity(capacity_hint);
        fd.read_to_end(&mut buffer).with_context(|| {
            format!("Failed to read file {}", path.display())
        })?;
        Ok(Input::Buffer(buffer))
    } else {
        if io::stdin().is_terminal() {
            // No piped input and no file specified
            let mut cmd = Args::command();
            cmd.print_help()?;
            anyhow::bail!("No input specified");
        }
        // Read raw bytes: binary formats (CBOR, MessagePack) are valid
        // stdin inputs; UTF-8 is only required (and validated) for text
        // formats.
        let mut buffer = Vec::new();
        io::stdin().read_to_end(&mut buffer)?;
        Ok(Input::Buffer(buffer))
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

/// Parses the input and invokes `f` with a borrowed [`Value`] to preserve zero-copy path for
/// JSON/Auto `Format`s.
fn with_json<F, T>(input: Option<PathBuf>, format: Format, f: F) -> Result<T>
where
    F: FnOnce(&Value) -> Result<T>,
{
    let input_content = parse_input_content(input)?;

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
        None => input_content.to_str().context("Input is not valid UTF-8")?,
    };
    let json: Value = serde_json::from_str(json_str)
        .with_context(|| format!("Failed to parse as {format}"))?;
    f(&json)
}

/// Entry point for main binary.
///
/// This parses the command line arguments and executes the query. If the input
/// is piped in, it reads from STDIN. The output is printed to STDOUT, with
/// formatting determined by the command line arguments.
#[expect(clippy::too_many_lines, reason = "Argument parsing combinations")]
fn main() -> Result<()> {
    let mut args = Args::parse();

    // Porcelain means machine-parseable: force colors off (regardless of
    // TTY detection) and one JSON value per line, so consumers can rely on
    // the output shape. Previously only the --count/--depth labels were
    // affected, while match output stayed colored (on a TTY) and
    // multi-line pretty-printed.
    if args.porcelain {
        colored::control::set_override(false);
        args.compact = true;
    }

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

            // `--depth` without a query: sole positional argument is the file
            if args.depth && args.query.is_some() && args.input.is_none() {
                args.input = args.query.take().map(PathBuf::from);
            }
            // short circuit to only perform the depth computation
            if args.depth && args.input.is_some() {
                let format = detect_format(args.input.as_ref(), args.format);
                with_json(args.input, format, |json| {
                    if args.porcelain {
                        writeln!(writer, "{}", depth(json))?;
                    } else {
                        writeln!(
                            writer,
                            "{} {}",
                            "Depth:".bold().blue(),
                            depth(json)
                        )?;
                    }
                    Ok(())
                })?;

                return Ok(());
            }

            let raw_query = args.query.ok_or_else(|| {
                anyhow::anyhow!("Query string required unless using subcommand")
            })?;

            let (query, predicate) = if args.fixed_string {
                // `-F`/`--fixed-string:` treat the query as a literal field name
                // and search at any depth, equivalent to `(* | [*])*."<literal>"`
                (Query::recursive_depth_fixed_string(raw_query), None)
            } else {
                jsongrep::query::parse_query_with_predicate(&raw_query)
                    .with_context(|| "Failed to parse query")?
            };

            let format = detect_format(args.input.as_ref(), args.format);
            with_json(args.input, format, |json| {
                let dfa = if args.ignore_case {
                    QueryDFA::from_query_bounded_ignore_case(
                        &query,
                        DEFAULT_MAX_DFA_STATES,
                    )
                } else {
                    QueryDFA::from_query_bounded(&query, DEFAULT_MAX_DFA_STATES)
                }?;
                let mut results = dfa.find(json);
                // Trailing value predicate (e.g. `age >= 30`): filter which
                // matched values survive.
                if let Some(predicate) = &predicate {
                    results.retain(|result| predicate.matches(result.value));
                }

                if args.count || args.depth {
                    args.no_display = true;
                }

                if args.count {
                    if args.porcelain {
                        writeln!(writer, "{}", results.len())?;
                    } else {
                        writeln!(
                            writer,
                            "{} {}",
                            "Found matches:".bold().blue(),
                            results.len()
                        )
                        .with_context(|| "Failed to write to stdout")?;
                    }
                }

                if args.depth {
                    if args.porcelain {
                        writeln!(writer, "{}", depth(json))?;
                    } else {
                        writeln!(
                            writer,
                            "{} {}",
                            "Depth:".bold().blue(),
                            depth(json)
                        )?;
                    }
                }

                if !args.no_display {
                    let pretty = !args.compact;
                    for result in &results {
                        write_colored_result(
                            &mut writer,
                            result.value,
                            &result.path,
                            &WriteOptions {
                                pretty,
                                show_path,
                                raw: args.raw_output,
                            },
                        )?;
                    }
                }

                Ok(())
            })?;

            match writer.flush() {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::BrokenPipe => {}
                Err(err) => return Err(err.into()),
            }
        }
    }

    Ok(())
}
