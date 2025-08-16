/*!
Main binary for jsongrep.
*/

use anyhow::{Context, Result};
use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::generate;
use std::io::stdout;
use std::io::{self};
use std::path::Path;
use std::{
    fs::{self, OpenOptions},
    io::{IsTerminal, Read},
    path::PathBuf,
};

use jsongrep::{query::*, schema::JSONValue};

/// Query an input JSON document against a jsongrep query.
#[derive(Parser)]
#[command(name = "jg", version, about, arg_required_else_help = true, long_about = None, disable_help_subcommand = true)]
struct Args {
    /// Optional subcommands
    #[command(subcommand)]
    command: Option<Commands>,
    /// Query string (e.g., "**.name")
    query: Option<String>,
    #[arg(value_name = "FILE")]
    /// Optional path to JSON file. If omitted, reads from STDIN
    input: Option<PathBuf>,
    /// Do not pretty-print the JSON output, instead use compact
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

/// Utility function to generate all Man pages for the main [`Args`] structure and all dependent
/// recursive subcommand pages to the output directory if specified, else the current directory.
fn generate_man_pages(output_dir: Option<PathBuf>) -> Result<()> {
    let output_dir: PathBuf = output_dir.unwrap_or(
        std::env::current_dir().context("Opening current directory")?,
    );

    std::fs::create_dir_all(&output_dir)
        .context("create output Man directories")?;

    let cmd = Args::command();
    let main_man = clap_mangen::Man::new(cmd.clone());
    let main_man_path = output_dir.join(format!("{}.1", cmd.get_name()));
    let mut man_man_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&main_man_path)
        .with_context(|| {
            format!("failed to create {}", main_man_path.display())
        })?;
    main_man.render(&mut man_man_file)?;
    println!("Generated: {}", main_man_path.display());

    // Recurse over subcommands
    generate_subcommand_man_pages(&cmd, &output_dir, cmd.get_name())?;

    Ok(())
}

/// Generate subcommand Man pages recursively.
fn generate_subcommand_man_pages(
    cmd: &clap::Command,
    output_dir: &Path,
    prefix: &str,
) -> Result<()> {
    for subcmd in cmd.get_subcommands() {
        let subcmd_man = clap_mangen::Man::new(subcmd.clone());
        let file_name = format!("{}-{}", prefix, subcmd_man.get_filename());
        let man_path = output_dir.join(&file_name);
        let mut subcmd_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&man_path)
            .with_context(|| {
                format!("failed to create {}", man_path.display())
            })?;

        subcmd_man.render(&mut subcmd_file)?;
        println!("Generated: {}", man_path.display());
        if subcmd.has_subcommands() {
            generate_subcommand_man_pages(
                subcmd,
                output_dir,
                subcmd.get_name(),
            )?
        }
    }

    Ok(())
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
                generate(shell, &mut cmd, "jg", &mut stdout().lock())
            }
            GenerateCommand::Man { output_dir } => {
                generate_man_pages(output_dir)?
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

            // Parse input content
            let input_content = if let Some(path) = args.input {
                fs::read_to_string(&path).with_context(|| {
                    format!("Failed to read file {:?}", path)
                })?
            } else {
                if io::stdin().is_terminal() {
                    // No piped input and no file specified
                    let mut cmd = Args::command();
                    return Ok(cmd.print_help()?);
                }
                let mut buffer = String::new();
                io::stdin().read_to_string(&mut buffer)?;
                buffer
            };
            let json = JSONValue::try_from(input_content.as_str())
                .with_context(|| "Failed to parse JSON")?;

            // Execute query
            let results = DFAQueryEngine.find(&json, &query);

            // Display output
            if args.count {
                println!("Found matches: {}", results.len());
            }

            // Display depth
            if args.depth {
                println!("Document depth: {}", json.depth())
            }

            if !args.no_display {
                if !args.compact {
                    // Pretty-printed output
                    let json_values: Vec<&JSONValue> =
                        results.iter().map(|p| p.value).collect();
                    println!("{}", serde_json::to_string_pretty(&json_values)?);
                } else {
                    // Compact output
                    let json_output: Vec<String> = results
                        .iter()
                        .map(|p| {
                            serde_json::to_string(p.value)
                                .unwrap_or_else(|_| "".to_string())
                        })
                        .collect();
                    println!("{}", serde_json::to_string(&json_output)?);
                }
            }
        }
    }

    Ok(())
}
