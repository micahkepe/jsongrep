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

/// Available options for printing matches.
#[derive(Debug, Default)]
pub struct WriteOptions {
    /// Whether to pretty print the output.
    pub pretty: bool,
    /// Whether to include the file paths for matches.
    pub show_path: bool,
    /// Whether to print the raw output or auto-escape characters.
    pub raw: bool,
}

/// Write a single query result (path header + colorized JSON value) to `writer`.
///
/// Returns `Ok(true)` when the result was written and the caller may keep
/// writing further results, and `Ok(false)` on a broken pipe (the downstream
/// consumer, e.g. `head` or `less`, has gone away), so the caller can stop
/// formatting the remaining results instead of writing into a dead pipe.
///
/// When `raw` is set, a matched value that is a string is written without
/// JSON quotes or escaping (like `jq -r`), so shell pipelines get the bare
/// text: `TOKEN=$(... | jg -r token)`. Non-string values are unaffected
/// (their JSON form is already their raw form).
///
/// # Errors
///
/// Returns an error if writing to `writer` fails for any reason other than a
/// broken pipe.
pub fn write_colored_result<W: Write>(
    writer: &mut W,
    value: &Value,
    path: &[PathType],
    options: &WriteOptions,
) -> anyhow::Result<bool> {
    let result = (|| -> io::Result<()> {
        if options.show_path && !path.is_empty() {
            // Only pay for building the joined path string when it is
            // actually shown.
            let mut header = String::new();
            for (i, part) in path.iter().enumerate() {
                if i > 0 {
                    header.push('.');
                }
                header.push_str(&part.to_string());
            }
            writeln!(writer, "{}:", header.bold().magenta())?;
        }
        if options.raw
            && let Value::Str(s) = value
        {
            // Raw output: the string's contents, not its JSON encoding.
            // Escape sequences in the source (e.g. \n) have already been
            // decoded by the JSON parser, so this writes real newlines etc.
            write!(writer, "{s}")?;
        } else {
            write_colored_json(writer, value, 0, options.pretty)?;
        }
        writeln!(writer)?;
        Ok(())
    })();

    match result {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(false),
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
            let len = obj.len();
            for (i, (key, val)) in obj.iter().enumerate() {
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
                if i < len - 1 {
                    write!(writer, ",")?;
                }
            }
            if pretty && len > 0 {
                writeln!(writer)?;
                write!(writer, "{:width$}", "", width = indent)?;
            }
            write!(writer, "}}")
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Unit testing.")]
mod tests {
    use super::*;

    /// A writer that always fails with the given [`ErrorKind`].
    struct FailingWriter(ErrorKind);

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(self.0))
        }
    }

    #[test]
    fn write_colored_result_returns_true_on_success() {
        let value: Value = serde_json::from_str("{\"a\": 1}").unwrap();
        let mut out = Vec::new();
        let keep_going = write_colored_result(
            &mut out,
            &value,
            &[],
            &WriteOptions { pretty: true, ..Default::default() },
        )
        .unwrap();
        assert!(keep_going);
        assert!(!out.is_empty());
    }

    #[test]
    fn write_colored_result_signals_stop_on_broken_pipe() {
        let value: Value = serde_json::from_str("{\"a\": 1}").unwrap();
        let mut broken = FailingWriter(ErrorKind::BrokenPipe);
        let keep_going = write_colored_result(
            &mut broken,
            &value,
            &[],
            &WriteOptions { pretty: true, ..Default::default() },
        )
        .unwrap();
        assert!(!keep_going, "broken pipe should signal the caller to stop");
    }

    #[test]
    fn write_colored_result_propagates_other_errors() {
        let value: Value = serde_json::from_str("{\"a\": 1}").unwrap();
        let mut failing = FailingWriter(ErrorKind::PermissionDenied);
        let result = write_colored_result(
            &mut failing,
            &value,
            &[],
            &WriteOptions { pretty: true, ..Default::default() },
        );
        assert!(result.is_err(), "non-pipe IO errors should propagate");
    }
}
