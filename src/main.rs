/*!
# Main Query Binary

Query an input JSON document against a rq query.

To run:

```bash
cargo run --release --bin main
target/release/main
```

## Examples

From file input:

```bash
cargo run --release -- "**.[*]" ../path/to/file.json
```

Piping in JSON content:

```bash
echo '{"foo": "bar"}' | cargo run --release "foo"
```

View help:

```bash
cargo run --release -- --help
```
*/

use anyhow::{Context, Result};
use clap::CommandFactory;
use clap::{ArgAction, Parser};
use std::io;
use std::{
    fs,
    io::{IsTerminal, Read},
    path::PathBuf,
};

use rq::{query::*, schema::JSONValue};

/// Query an input JSON document against a rq query.
#[derive(Parser, Debug)]
#[command(version, about, arg_required_else_help = true, long_about = None)]
struct Args {
    /// Query string (e.g., "**.name")
    query: String,
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

/// Entry point for main binary.
fn main() -> Result<()> {
    let args = Args::parse();

    // Parse query
    let query: Query = args
        .query
        .parse()
        .with_context(|| "Failed to parse query")?;

    // Parse input content
    let input_content = if let Some(path) = args.input {
        fs::read_to_string(&path).with_context(|| format!("Failed to read file {:?}", path))?
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
    let json =
        JSONValue::try_from(input_content.as_str()).with_context(|| "Failed to parse JSON")?;

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
            let json_values: Vec<&JSONValue> = results.iter().map(|p| p.value).collect();
            println!("{}", serde_json::to_string_pretty(&json_values)?);
        } else {
            // Compact output
            let json_output: Vec<String> = results
                .iter()
                .map(|p| serde_json::to_string(p.value).unwrap_or_else(|_| "".to_string()))
                .collect();
            println!("{}", serde_json::to_string(&json_output)?);
        }
    }

    Ok(())
}
