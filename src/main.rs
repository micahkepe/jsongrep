/*!
Main binary for jsongrep.
*/

use anyhow::{Context as _, Result};
use clap::{ArgAction, CommandFactory as _, Parser, Subcommand};
use clap_complete::generate;
use colored::Colorize;
use memmap2::{Mmap, MmapOptions};
use serde::Serialize;
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
    query::{DFAQueryEngine, PathType, Query, QueryEngine as _},
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
    /// Optional path to JSON file. If omitted, reads from STDIN
    input: Option<PathBuf>,
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
            // Parse query
            let query: Query = args
                .query
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Query string required unless using subcommand"
                    )
                })?
                .parse()
                .with_context(|| "Failed to parse query")?;

            let input_content = parse_input_content(args.input)?;
            let json: Value = serde_json::from_str(
                input_content
                    .to_str()
                    .context("File contents are not valid utf-8")?,
            )
            .with_context(|| "Failed to parse JSON")?;
            let results = DFAQueryEngine.find(&json, &query);

            let stdout = stdout().lock();
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

            // Display depth
            if args.depth {
                writeln!(
                    writer,
                    "{} {}",
                    "Depth:".bold().blue(),
                    jsongrep::depth(&json)
                )?;
            }

            if !args.no_display {
                if args.compact {
                    // Compact output
                    results
                        .iter()
                        .map(|p| {
                            let val = serde_json::to_string(p.value)
                                .unwrap_or_else(|_| String::new());
                            write_colored_result_to_stdout(
                                &mut writer,
                                &val,
                                &p.path,
                                true,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                } else {
                    // Pretty-printed output
                    results
                        .iter()
                        .map(|p| {
                            write_colored_result_to_stdout(
                                &mut writer,
                                p.value,
                                &p.path,
                                true,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                }
            }

            writer.flush()?;
        }
    }

    Ok(())
}

/// Write found result to stdout with color.
fn write_colored_result_to_stdout<T: Serialize, W: Write>(
    writer: &mut W,
    value: &T,
    path: &[PathType],
    pretty: bool,
) -> Result<()> {
    // Print path
    let path = path
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(".");

    writeln!(writer, "{}:", path.bold().magenta())?;

    let result = if pretty {
        serde_json::to_writer_pretty(&mut *writer, value)
    } else {
        serde_json::to_writer(&mut *writer, value)
    };

    match result {
        Err(err) if err.io_error_kind() == Some(ErrorKind::BrokenPipe) => {
            Ok(())
        }
        Err(err) => Err(err.into()),
        Ok(()) => {
            writeln!(writer)?;
            Ok(())
        }
    }
}
