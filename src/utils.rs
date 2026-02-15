//! Miscellaneous utility functions.

use anyhow::Context as _;
use colored::Colorize;
use serde_json_borrow::Value;
use std::io::Write;
use std::io::{self, ErrorKind};

use crate::query::PathType;

/// Returns the depth of the JSON value.
pub fn depth(json: &Value) -> usize {
    match json {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Str(_) => 1,
        Value::Array(arr) => {
            let inner_depth = arr.iter().map(depth).max().unwrap_or(0);
            1 + inner_depth
        }
        Value::Object(map) => {
            let inner_depth = map.values().map(depth).max().unwrap_or(0);
            1 + inner_depth
        }
    }
}

// ==============================================================================
// Colorized JSON Output
// ==============================================================================

/// Write a single query result (path header + colorized JSON value) to `writer`.
/// Silently returns `Ok(())` on broken pipe so that piping to tools like
/// `less` or `head` exits cleanly.
///
/// # Errors
///
/// Returns an error if writing to `writer` fails.
pub fn write_colored_result<W: Write>(
    writer: &mut W,
    value: &Value,
    path: &[PathType],
    pretty: bool,
    show_path: bool,
) -> anyhow::Result<()> {
    let path = path
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(".");

    let result = (|| -> io::Result<()> {
        if show_path && !path.is_empty() {
            writeln!(writer, "{}:", path.bold().magenta())?;
        }
        write_colored_json(writer, value, 0, pretty)?;
        writeln!(writer)?;
        Ok(())
    })();

    match result {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(err).context("write colorized JSON to stdout"),
    }
}

/// Recursively write a JSON value with syntax highlighting.
fn write_colored_json<W: Write>(
    writer: &mut W,
    value: &Value,
    indent: usize,
    pretty: bool,
) -> io::Result<()> {
    let next_indent = indent + 2;

    match value {
        Value::Null => write!(writer, "{}", "null".red().dimmed()),
        Value::Bool(b) => {
            write!(writer, "{}", b.to_string().yellow().bold())
        }
        Value::Number(n) => write!(writer, "{}", n.to_string().yellow()),
        Value::Str(s) => {
            // NOTE: Re-serialize to get proper JSON escaping and quoting.
            let quoted = serde_json::to_string(s.as_ref())
                .expect("string serialization cannot fail");
            write!(writer, "{}", quoted.green())
        }
        Value::Array(arr) => {
            write!(writer, "[")?;
            for (i, item) in arr.iter().enumerate() {
                if pretty {
                    writeln!(writer)?;
                    write!(writer, "{:width$}", "", width = next_indent)?;
                }
                write_colored_json(writer, item, next_indent, pretty)?;
                if i < arr.len() - 1 {
                    write!(writer, ",")?;
                }
            }
            if pretty && !arr.is_empty() {
                writeln!(writer)?;
                write!(writer, "{:width$}", "", width = indent)?;
            }
            write!(writer, "]")
        }
        Value::Object(obj) => {
            write!(writer, "{{")?;
            let entries: Vec<_> = obj.iter().collect();
            for (i, (key, val)) in entries.iter().enumerate() {
                if pretty {
                    writeln!(writer)?;
                    write!(writer, "{:width$}", "", width = next_indent)?;
                }
                // Key with quotes -> colored cyan.
                let quoted_key = serde_json::to_string(key)
                    .expect("key serialization cannot fail");
                write!(writer, "{}", quoted_key.cyan())?;
                if pretty {
                    write!(writer, ": ")?;
                } else {
                    write!(writer, ":")?;
                }
                write_colored_json(writer, val, next_indent, pretty)?;
                if i < entries.len() - 1 {
                    write!(writer, ",")?;
                }
            }
            if pretty && !entries.is_empty() {
                writeln!(writer)?;
                write!(writer, "{:width$}", "", width = indent)?;
            }
            write!(writer, "}}")
        }
    }
}
