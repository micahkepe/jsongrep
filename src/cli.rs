//! CLI utilities.

use anyhow::{Context as _, Result};
use colored::Colorize;
use memmap2::{Mmap, MmapOptions};
use serde_json_borrow::Value;
use std::fs::OpenOptions;
use std::io::{
    self, BufRead as _, BufReader, ErrorKind, IsTerminal as _, Read, Write,
};
use std::ops::ControlFlow;
use std::path::PathBuf;

use crate::query::{JSONPointer, PathType, QueryDFA};

/// Minimum file size for which memory-mapping is attempted.
///
/// For small files, it is likely that a single read call is at least as fast or
/// faster than mmap (mmap setup and page-fault overhead dominate for small
/// files) and avoids mmap's file-truncation hazards.
///
/// NOTE: with recursive directory walking now in place, consider skipping mmap
/// for multi-file walks (per-file syscall overhead adds up) and reserving it
/// for single large files.
///
/// See: <https://burntsushi.net/ripgrep/#mechanics>.
const MMAP_MIN_FILE_SIZE: u64 = 1 << 20; // 1 MiB

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
    fn to_str(&self) -> Result<&str, std::str::Utf8Error> {
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
            // YAML
            #[cfg(feature = "yaml")]
            Format::Yaml => {
                let text = self.to_str().map_err(|_| {
                    anyhow::anyhow!("YAML input is not valid UTF-8")
                })?;
                let value: serde_json::Value =
                    yaml_serde::from_str(text).context("parse YAML input")?;
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
            // (JSONL is parsed per line in `parse_jsonl`, borrowing from the
            // input buffer, so it never goes through this owned-string path.)
            Format::Auto | Format::Json | Format::Jsonl => {
                unreachable!(
                    "to_json_string called with Auto, Json, or Jsonl, not needed"
                )
            }
        }
    }
}

/// Whether any error in the chain is a broken pipe (the downstream consumer
/// of stdout has gone away), which is a signal to stop printing, not an
/// input-file failure.
#[must_use]
pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io_err| io_err.kind() == ErrorKind::BrokenPipe)
    })
}

/// Parse input content, from the input path buffer if provided, else try STDIN.
///
/// # Errors
///
/// Returns early with an error if the file cannot be opened or read. If the
/// input is not a file or piped input, prints the help message and exits with
/// an error.
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
            anyhow::bail!("No input provided and stdin is a terminal");
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
pub enum Format {
    /// Detects from filetype, else defaults to JSON.
    #[default]
    Auto,
    /// JSON format.
    Json,
    /// JSONL format.
    Jsonl,
    /// YAML format.
    Yaml,
    /// TOML format.
    Toml,
    /// CBOR format.
    Cbor,
    /// `MessagePack` format.
    Msgpack,
}

impl Format {
    /// Whether this format produces binary (non-text) output.
    #[must_use]
    pub const fn is_binary(self) -> bool {
        matches!(self, Self::Cbor | Self::Msgpack)
    }
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

/// Parse JSONL/NDJSON input line by line into a single top-level array,
/// borrowing each record directly from the input buffer.
///
/// Compared to concatenating all lines into a synthetic `[...]` JSON string
/// and re-parsing it, this avoids a second full-input-sized allocation and
/// reports parse errors with the actual line number of the offending record.
fn parse_jsonl(text: &str) -> Result<Value<'_>> {
    let mut records = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line).with_context(|| {
            format!("Failed to parse JSONL line {}", idx + 1)
        })?;
        records.push(record);
    }
    Ok(Value::Array(records))
}

/// Parses the input and invokes `f` with a borrowed [`Value`] to preserve
/// zero-copy path for JSON/Auto and JSONL `Format`s.
///
/// # Errors
///
/// Returns an error if the input cannot be read, is not valid for the
/// specified format, or if `f` itself returns an error.
pub fn with_json<F, T>(
    input: Option<PathBuf>,
    format: Format,
    f: F,
) -> Result<T>
where
    F: FnOnce(&Value) -> Result<T>,
{
    let input_content = parse_input_content(input)?;

    // For JSON/Auto we borrow directly from the mmap/stdin buffer,
    // preserving the zero-copy path that serde_json_borrow provides. JSONL
    // is parsed per line, likewise borrowing from the input buffer. For
    // other formats, we convert to an owned JSON string first and then
    // borrow from that.
    match format {
        Format::Json | Format::Auto => {
            let json_str =
                input_content.to_str().context("Input is not valid UTF-8")?;
            let json: Value = serde_json::from_str(json_str)
                .with_context(|| format!("Failed to parse as {format}"))?;
            f(&json)
        }
        Format::Jsonl => {
            let text = input_content.to_str().map_err(|_| {
                anyhow::anyhow!("JSONL input is not valid UTF-8")
            })?;
            let json = parse_jsonl(text)?;
            f(&json)
        }
        other => {
            let json_string_owned = input_content.to_json_string(other)?;
            let json: Value = serde_json::from_str(&json_string_owned)
                .with_context(|| format!("Failed to parse as {format}"))?;
            f(&json)
        }
    }
}

/// Searches JSONL input one line at a time, calling `visit` with `out` for
/// each match as soon as its line has been read.
///
/// Each line is parsed on its own, and matches get the same paths as with
/// [`with_json`], where record `i` is element `i` of the top-level array.
/// Blank lines are skipped. A query that matches that array itself (such as
/// `""` or `**`) needs the whole input, so its root match is not reported
/// here; use [`with_json`] for those.
///
/// `out` is flushed before every read that may block, that is whenever the
/// reader has no complete line buffered, so matches show up before a pipe
/// that is still open sends more lines.
/// Return [`ControlFlow::Break`] from `visit` to stop reading.
///
/// # Errors
///
/// Returns an error if the input cannot be read, if a line is not valid
/// UTF-8 or JSON (matches from earlier lines have already been passed to
/// `visit` by then), if flushing `out` fails, or if `visit` returns one.
pub fn for_each_jsonl_match<R, W, F>(
    reader: &mut BufReader<R>,
    dfa: &QueryDFA,
    out: &mut W,
    mut visit: F,
) -> Result<()>
where
    R: Read,
    W: Write,
    F: for<'a> FnMut(&mut W, JSONPointer<'a>) -> Result<ControlFlow<()>>,
{
    let mut line = Vec::new();
    let mut path = Vec::new();
    let mut line_number = 0usize;
    let mut index = 0usize;
    loop {
        // Reading the next line may block, so flush the matches printed
        // so far unless that line is already buffered in full.
        if !reader.buffer().contains(&b'\n') {
            out.flush()?;
        }
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .context("Failed to read JSONL input")?
            == 0
        {
            return Ok(());
        }
        line_number += 1;
        let text = str::from_utf8(&line).map_err(|_| {
            anyhow::anyhow!("JSONL line {line_number} is not valid UTF-8")
        })?;
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(text).with_context(|| {
            format!("Failed to parse JSONL line {line_number}")
        })?;
        let record_index = index;
        index += 1;
        let Some(state) = dfa
            .get_index_symbol_id(record_index)
            .and_then(|symbol| dfa.transition(dfa.start_state, symbol))
        else {
            continue;
        };
        path.clear();
        path.push(PathType::Index(record_index));
        let flow =
            dfa.for_each_match_from(
                state,
                &mut path,
                &record,
                |ptr| match visit(out, ptr) {
                    Ok(ControlFlow::Continue(())) => ControlFlow::Continue(()),
                    Ok(ControlFlow::Break(())) => ControlFlow::Break(Ok(())),
                    Err(err) => ControlFlow::Break(Err(err)),
                },
            );
        if let ControlFlow::Break(result) = flow {
            return result;
        }
    }
}

/// Available options for printing matches.
#[derive(Debug, Default)]
pub struct WriteOptions {
    /// Whether to pretty print the output.
    pub pretty: bool,
    /// Whether to include the file paths for matches.
    pub show_path: bool,
    /// Whether to print the raw output or auto-escape characters.
    pub raw: bool,
    /// Output format for results.
    pub output_format: Format,
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
    let binary = options.output_format.is_binary();
    let result = (|| -> io::Result<()> {
        if !binary && options.show_path && !path.is_empty() {
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
            write!(writer, "{s}")?;
        } else {
            write_value(writer, value, options)?;
        }
        if !binary {
            writeln!(writer)?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(false),
        Err(err) => Err(err).context("write colorized JSON to stdout"),
    }
}

/// Writes a single value with the given [`WriteOptions`].
fn write_value<W: Write>(
    writer: &mut W,
    value: &Value,
    options: &WriteOptions,
) -> io::Result<()> {
    match options.output_format {
        Format::Auto | Format::Json => {
            write_colored_json(writer, value, 0, options.pretty)
        }
        Format::Jsonl => {
            let s = serde_json::to_string(value)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            write!(writer, "{s}")
        }
        #[cfg(feature = "yaml")]
        Format::Yaml => {
            let s = yaml_serde::to_string(value)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            write!(writer, "{}", s.trim_end())
        }
        #[cfg(not(feature = "yaml"))]
        Format::Yaml => Err(io::Error::new(
            ErrorKind::Unsupported,
            "YAML support not enabled. Rebuild with --features yaml",
        )),
        #[cfg(feature = "toml")]
        Format::Toml => {
            let owned: serde_json::Value = serde_json::to_value(value)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            if !owned.is_object() {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "TOML output requires a table (object) value; \
                     this match is a non-table type",
                ));
            }
            let s = toml::to_string(&owned)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            write!(writer, "{}", s.trim_end())
        }
        #[cfg(not(feature = "toml"))]
        Format::Toml => Err(io::Error::new(
            ErrorKind::Unsupported,
            "TOML support not enabled. Rebuild with --features toml",
        )),
        #[cfg(feature = "cbor")]
        Format::Cbor => ciborium::into_writer(value, writer)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e)),
        #[cfg(not(feature = "cbor"))]
        Format::Cbor => Err(io::Error::new(
            ErrorKind::Unsupported,
            "CBOR support not enabled. Rebuild with --features cbor",
        )),
        #[cfg(feature = "msgpack")]
        Format::Msgpack => {
            let bytes = rmp_serde::to_vec(value)
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            writer.write_all(&bytes)
        }
        #[cfg(not(feature = "msgpack"))]
        Format::Msgpack => Err(io::Error::new(
            ErrorKind::Unsupported,
            "MessagePack support not enabled. Rebuild with --features msgpack",
        )),
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
        assert_ne!(out.len(), 0);
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

    const JSONL: &str = concat!(
        "{\"id\": 1, \"tags\": [\"a\", \"b\"], \"user\": {\"id\": 10}}\n",
        "\n",
        "  {\"id\": 2, \"tags\": []}\r\n",
        "[{\"id\": 3}, {\"id\": 4}]\n",
        "\"plain\"\n",
        "{\"id\": 5, \"user\": {\"id\": 50, \"tags\": [\"c\"]}}",
    );

    /// Runs [`for_each_jsonl_match`] over `input` and returns each match as
    /// its path and compact JSON value.
    fn jsonl_matches(input: &str, dfa: &QueryDFA) -> Vec<(String, String)> {
        let mut found = Vec::new();
        let mut reader = BufReader::new(input.as_bytes());
        for_each_jsonl_match(&mut reader, dfa, &mut io::sink(), |_, ptr| {
            found.push((
                format!("{:?}", ptr.path),
                serde_json::to_string(ptr.value).unwrap(),
            ));
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
        found
    }

    #[test]
    fn jsonl_matches_agree_with_whole_input_search() {
        let whole = parse_jsonl(JSONL).unwrap();
        for query in [
            "[*].id",
            "[1]",
            "[2].*",
            "[1:4].id",
            "[3:].user.id",
            "**.id",
            "[*].tags[*]",
            "[*]",
            "[*].(id | user)",
            "[*].**.tags[0]",
            "[*].**",
            "missing",
        ] {
            let dfa = QueryDFA::from_query_str(query).unwrap();
            let expected: Vec<_> = dfa
                .find(&whole)
                .iter()
                .map(|ptr| {
                    (
                        format!("{:?}", ptr.path),
                        serde_json::to_string(ptr.value).unwrap(),
                    )
                })
                .collect();
            assert_eq!(jsonl_matches(JSONL, &dfa), expected, "query {query}");
        }
    }

    #[test]
    fn jsonl_matches_stop_on_break() {
        let dfa = QueryDFA::from_query_str("[*].id").unwrap();
        let mut seen = 0;
        // Line 3 is not JSON: breaking on line 1 must not read that far.
        let mut reader = BufReader::with_capacity(
            1,
            b"{\"id\": 1}\n{\"id\": 2}\nnot json\n".as_slice(),
        );
        for_each_jsonl_match(&mut reader, &dfa, &mut io::sink(), |_, _| {
            seen += 1;
            Ok(ControlFlow::Break(()))
        })
        .unwrap();
        assert_eq!(seen, 1);
    }

    #[test]
    fn jsonl_matches_report_bad_line_after_earlier_matches() {
        let dfa = QueryDFA::from_query_str("[*].id").unwrap();
        let mut seen = Vec::new();
        let mut reader =
            BufReader::new(b"{\"id\": 1}\n\n{\"id\": oops}\n".as_slice());
        let err = for_each_jsonl_match(
            &mut reader,
            &dfa,
            &mut io::sink(),
            |_, ptr| {
                seen.push(serde_json::to_string(ptr.value).unwrap());
                Ok(ControlFlow::Continue(()))
            },
        )
        .unwrap_err();
        assert_eq!(seen, ["1"]);
        assert!(
            format!("{err:#}").contains("Failed to parse JSONL line 3"),
            "{err:#}"
        );
    }

    #[test]
    fn jsonl_matches_flush_output_when_input_runs_dry() {
        /// Counts how often it is flushed.
        #[derive(Default)]
        struct CountFlushes(usize);

        impl Write for CountFlushes {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.0 += 1;
                Ok(())
            }
        }

        let dfa = QueryDFA::from_query_str("[*].id").unwrap();
        let mut out = CountFlushes::default();
        let mut flushes_at_match = Vec::new();
        // A one-byte buffer runs dry before every read, like a pipe whose
        // writer sends a line at a time.
        let mut reader = BufReader::with_capacity(
            1,
            b"{\"id\": 1}\n{\"id\": 2}\n".as_slice(),
        );
        for_each_jsonl_match(&mut reader, &dfa, &mut out, |out, _| {
            flushes_at_match.push(out.0);
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
        // The match from line 1 is flushed before line 2 is read.
        assert!(
            flushes_at_match[1] > flushes_at_match[0],
            "{flushes_at_match:?}"
        );
    }

    #[test]
    fn jsonl_matches_flush_output_before_waiting_on_partial_line() {
        use std::cell::RefCell;
        use std::rc::Rc;

        /// Logs what happens to the input and the output, in order.
        type Log = Rc<RefCell<Vec<&'static str>>>;

        /// Hands out one chunk of input per read, logging each read.
        struct Chunks(Vec<&'static [u8]>, Log);

        impl Read for Chunks {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                let chunk = self.0.remove(0);
                buf[..chunk.len()].copy_from_slice(chunk);
                self.1.borrow_mut().push("read");
                Ok(chunk.len())
            }
        }

        /// Logs writes and flushes.
        struct Output(Log);

        impl Write for Output {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.borrow_mut().push("write");
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.0.borrow_mut().push("flush");
                Ok(())
            }
        }

        let log = Log::default();
        let dfa = QueryDFA::from_query_str("[*].id").unwrap();
        // The first read brings a whole matching line and the start of the
        // next one, so reading the rest of that line would wait on a pipe.
        let mut reader = BufReader::new(Chunks(
            vec![b"{\"id\": 1}\n{\"id\"", b": 2}\n"],
            Rc::clone(&log),
        ));
        let mut out = Output(Rc::clone(&log));
        for_each_jsonl_match(&mut reader, &dfa, &mut out, |out, _| {
            out.write_all(b"match")?;
            Ok(ControlFlow::Continue(()))
        })
        .unwrap();
        let log = log.borrow();
        let first_write = log.iter().position(|&e| e == "write").unwrap();
        let second_read = log
            .iter()
            .enumerate()
            .filter(|&(_, &e)| e == "read")
            .nth(1)
            .unwrap()
            .0;
        assert!(
            log[first_write..second_read].contains(&"flush"),
            "the first match must be flushed before the second read: {log:?}"
        );
    }
}
