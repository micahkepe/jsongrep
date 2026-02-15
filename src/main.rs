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
    query::{DFAQueryEngine, Query, QueryEngine as _},
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
    /// Treat the query as a literal field name and search at any depth.
    ///
    /// Searches for the field at any depth, equivalent to (* | [*])*."<query>".
    #[arg(short = 'F', long, action = ArgAction::SetTrue)]
    fixed_string: bool,
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
            let raw_query = args.query.ok_or_else(|| {
                anyhow::anyhow!("Query string required unless using subcommand")
            })?;

            let query: Query = if args.fixed_string {
                // -F/--fixed-string: treat the query as a literal field name
                // and search at any depth, equivalent to (* | [*])*."<literal>"
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

            let input_content = parse_input_content(args.input)?;
            let json: Value = serde_json::from_str(
                input_content
                    .to_str()
                    .context("File contents are not valid utf-8")?,
            )
            .with_context(|| "Failed to parse JSON")?;
            let results = DFAQueryEngine.find(&json, &query);

            // NOTE: use single, locked stdout handle to avoid interleaving
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
