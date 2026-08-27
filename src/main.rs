/*!
Main binary for jsongrep.
*/

use anyhow::{Context as _, Result};
use clap::{ArgAction, CommandFactory as _, Parser, Subcommand};
use clap_complete::generate;
use colored::Colorize;
use globset::Glob;
use ignore::WalkBuilder;
use std::{
    collections::HashSet,
    io::{self, BufWriter, ErrorKind, IsTerminal as _, Write, stdout},
    path::PathBuf,
};

use jsongrep::{
    cli::{
        Format, WriteOptions, depth, detect_format, is_broken_pipe, with_json,
        write_colored_result,
    },
    commands,
    query::{Query, QueryDFA},
};

/// Ceiling on DFA states during query compilation. Subset construction is
/// worst-case exponential in the query length, so a short adversarial query
/// could otherwise consume unbounded time and memory; past this budget `jg`
/// reports "query is too complex" instead. 2^18 states keeps the worst-case
/// abort around a second while remaining orders of magnitude beyond any
/// realistic query (which needs tens of states).
const DEFAULT_MAX_DFA_STATES: usize = 1 << 18;

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
    /// Optional path(s) to file(s). If omitted, reads from STDIN. With
    /// multiple files, the query is compiled once and run against each
    /// file, with a file heading before each file's matches.
    inputs: Vec<PathBuf>,
    /// Include or exclude files and directories for searching that match the given glob.
    ///
    /// Does not filter explicitly named files, only directory entries.
    #[arg(short = 'g', long)]
    glob: Option<String>,
    /// Prints the files that will be searched.
    #[arg(long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["count", "depth", "no_display"]
    )]
    files: bool,
    /// Print only the names of files containing at least one match
    /// (like `grep -l`).
    #[arg(
        short = 'l',
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["count", "depth", "no_display"]
    )]
    files_with_matches: bool,
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
    /// Stop searching after NUM matches per input file.
    ///
    /// The limit applies to each input separately: with multiple files,
    /// every file yields up to NUM matches. Each document's traversal
    /// terminates as soon as its limit is reached, so `--max-count 1` on a
    /// large document only pays for the work up to the first match.
    #[arg(short = 'm', long, value_name = "NUM", conflicts_with = "depth")]
    max_count: Option<usize>,
    /// Always print the path header, even when output is piped.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_path")]
    with_path: bool,
    /// Never print the path header, even in a terminal.
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "with_path")]
    no_path: bool,
    /// Quiet: write nothing to stdout; communicate via the exit status
    /// only (errors still print to stderr).
    #[arg(
        short = 'q',
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["count", "depth"]
    )]
    quiet: bool,
    /// Input format (auto-detects from file extension if omitted).
    #[arg(short = 'f', long, default_value = "auto")]
    format: Format,
    /// Output format (auto-detects from file extension if omitted).
    #[arg(short = 'o', long, default_value = "auto")]
    output: Format,
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

/// Entry point for main binary.
///
/// Exit codes follow grep/ripgrep conventions:
/// * 0 = at least one match
/// * 1 = no match
/// * 2 = error.
fn main() -> std::process::ExitCode {
    let args = Args::parse();

    match run(args) {
        Ok(matched) => {
            if matched {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::from(1)
            }
        }
        Err(err) => {
            eprintln!("Error: {err:?}");
            std::process::ExitCode::from(2)
        }
    }
}

/// Parse the command line arguments and execute the query. If the input
/// is piped in, it reads from STDIN. The output is printed to STDOUT, with
/// formatting determined by the command line arguments.
///
/// Returns whether the run "matched": `true` for at least one query match
/// and for non-query commands (`generate`, `--depth`), `false` for a query
/// that found nothing.
#[expect(clippy::too_many_lines, reason = "Argument parsing combinations")]
fn run(mut args: Args) -> Result<bool> {
    let mut matched = false;

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

            // `--depth` takes only files, no query string. Clap parses the
            // first positional into `query`; move it into `inputs`.
            //
            // `--files` takes optional filepath(s).
            if (args.depth || args.files)
                && let Some(query) = args.query.take()
            {
                args.inputs.insert(0, PathBuf::from(query));
            }

            // Update inputs with glob filtering and/or directory walking if
            // needed.
            if args.inputs.is_empty()
                && (args.files
                    || args.glob.is_some()
                    || io::stdin().is_terminal())
            {
                args.inputs.push(PathBuf::from("."));
            }
            let mut explicit_files = Vec::new();
            let mut dirs = Vec::new();
            let mut other = Vec::new();
            for p in std::mem::take(&mut args.inputs) {
                if p.is_file() {
                    explicit_files.push(p);
                } else if p.is_dir() {
                    dirs.push(p);
                } else {
                    other.push(p);
                }
            }
            let explicit_set: HashSet<_> =
                explicit_files.iter().cloned().collect();

            args.inputs = explicit_files
                .into_iter()
                .chain(other)
                .chain(dirs.iter().flat_map::<Vec<PathBuf>, _>(|f| {
                    WalkBuilder::new(f)
                        .build()
                        .filter_map(std::result::Result::ok)
                        .map(ignore::DirEntry::into_path)
                        .filter(|p| p.is_file())
                        .map(|p| {
                            p.strip_prefix("./").map(PathBuf::from).unwrap_or(p)
                        })
                        .collect()
                }))
                .collect();

            if let Some(pat) = &args.glob {
                let globber = Glob::new(pat)?.compile_matcher();
                args.inputs.retain(|p| {
                    explicit_set.contains(p) || globber.is_match(p)
                });
            }

            // --files to just list to-be-searched files and exit.
            if args.files {
                for input in &args.inputs {
                    if args.porcelain {
                        writeln!(writer, "{}", input.display())?;
                    } else {
                        writeln!(
                            writer,
                            "{}",
                            format!("{}", input.display()).magenta()
                        )?;
                    }
                }
                return Ok(true);
            }

            // short circuit to only perform the depth computation
            if args.depth && !args.inputs.is_empty() {
                let multi = args.inputs.len() > 1;
                let mut failed_inputs = 0usize;
                for path in args.inputs {
                    let format = detect_format(Some(&path), args.format);
                    let name = path.display().to_string();
                    let file_result = with_json(Some(path), format, |json| {
                        if multi {
                            // Attribute per file, grep -c style.
                            let styled_name = if args.porcelain {
                                name.normal()
                            } else {
                                name.bold().magenta()
                            };
                            writeln!(
                                writer,
                                "{}:{}",
                                styled_name,
                                depth(json)
                            )?;
                        } else if args.porcelain {
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
                    });
                    if let Err(err) = file_result {
                        if multi && !is_broken_pipe(&err) {
                            writer.flush().ok();
                            eprintln!("jg: {name}: {err:#}");
                            failed_inputs += 1;
                        } else if multi {
                            break;
                        } else {
                            return Err(err);
                        }
                    }
                }

                if failed_inputs > 0 {
                    match writer.flush() {
                        Ok(()) => {}
                        Err(err) if err.kind() == ErrorKind::BrokenPipe => {}
                        Err(err) => return Err(err.into()),
                    }
                    anyhow::bail!(
                        "{failed_inputs} input file(s) could not be processed"
                    );
                }
                return Ok(true);
            }

            let raw_query = args.query.ok_or_else(|| {
                anyhow::anyhow!("Query string required unless using subcommand")
            })?;

            let query: Query = if args.fixed_string {
                // `-F`/`--fixed-string:` treat the query as a literal field name
                // and search at any depth, equivalent to `(* | [*])*."<literal>"`
                Query::recursive_depth_fixed_string(raw_query)
            } else {
                raw_query.parse().with_context(|| "Failed to parse query")?
            };

            // Compile the DFA once; run it against every input.
            let dfa = if args.ignore_case {
                QueryDFA::from_query_bounded_ignore_case(
                    &query,
                    DEFAULT_MAX_DFA_STATES,
                )
            } else {
                QueryDFA::from_query_bounded(&query, DEFAULT_MAX_DFA_STATES)
            }?;

            if args.count || args.depth {
                args.no_display = true;
            }

            let multi = args.inputs.len() > 1;
            let inputs: Vec<Option<PathBuf>> = if args.inputs.is_empty() {
                vec![None]
            } else {
                args.inputs.into_iter().map(Some).collect()
            };

            // Errors in one file must not prevent searching the rest
            // (grep semantics); remember and report at the end.
            let mut failed_inputs = 0usize;
            let mut printed_block = false;

            for input in inputs {
                let format = detect_format(input.as_ref(), args.format);
                let name = input.as_ref().map_or_else(
                    || "(standard input)".to_string(),
                    |p| p.display().to_string(),
                );

                let file_result = with_json(input, format, |json| {
                    // --max-count applies per input file, so the limit
                    // resets for each document.
                    let results = args.max_count.map_or_else(
                        || dfa.find(json),
                        |limit| dfa.find_limited(json, limit),
                    );
                    if !results.is_empty() {
                        matched = true;
                    }

                    if args.quiet {
                        return Ok(());
                    }

                    if args.files_with_matches {
                        if !results.is_empty() {
                            writeln!(writer, "{name}")?;
                        }
                        return Ok(());
                    }

                    if args.count {
                        if multi {
                            // grep -c style per-file attribution.
                            let styled_name = if args.porcelain {
                                name.normal()
                            } else {
                                name.bold().magenta()
                            };
                            writeln!(
                                writer,
                                "{}:{}",
                                styled_name,
                                results.len()
                            )?;
                        } else if args.porcelain {
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

                    if !args.no_display && !results.is_empty() {
                        // ripgrep-style headings: with several inputs, name
                        // the file once before its matches, with a blank
                        // line between file blocks.
                        if multi {
                            if printed_block {
                                writeln!(writer)?;
                            }
                            let styled_name = if args.porcelain {
                                name.normal()
                            } else {
                                name.bold().green()
                            };
                            writeln!(writer, "{styled_name}")?;
                        }
                        printed_block = true;

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
                                    output_format: args.output,
                                },
                            )?;
                        }
                    }

                    Ok(())
                });

                if let Err(err) = file_result {
                    if multi && !is_broken_pipe(&err) {
                        // Keep going, grep-style; attribute the failure.
                        // Flush pending matches first so stdout/stderr
                        // interleave in file order.
                        writer.flush().ok();
                        eprintln!("jg: {name}: {err:#}");
                        failed_inputs += 1;
                    } else if multi {
                        // The output pipe is gone: nothing more can be
                        // printed, so stop quietly (same as single-input
                        // broken-pipe handling).
                        break;
                    } else {
                        return Err(err);
                    }
                }
            }

            if failed_inputs > 0 {
                // Flush what we printed before reporting the failure.
                match writer.flush() {
                    Ok(()) => {}
                    Err(err) if err.kind() == ErrorKind::BrokenPipe => {}
                    Err(err) => return Err(err.into()),
                }
                anyhow::bail!(
                    "{failed_inputs} input file(s) could not be processed"
                );
            }

            match writer.flush() {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::BrokenPipe => {}
                Err(err) => return Err(err.into()),
            }
        }
    }

    Ok(matched)
}
