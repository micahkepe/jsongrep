//! Integration test suite for `jsongrep` CLI
use assert_cmd::Command;
use std::io::Write as _;

/// Path to the `simple.json` file.
const SIMPLE_JSON_FILEPATH: &str = "tests/data/simple/simple.json";

/// Path to the `simple.jsonl` file.
const SIMPLE_JSONL_FILEPATH: &str = "tests/data/simple/simple.jsonl";

/// Path to the `simple.yaml` file.
const SIMPLE_YAML_FILEPATH: &str = "tests/data/simple/simple.yaml";

/// Path to the `simple.toml` file.
const SIMPLE_TOML_FILEPATH: &str = "tests/data/simple/simple.toml";

/// The canonical simple.json content, embedded at compile time for generating
/// binary test fixtures (`CBOR`, `MessagePack`) without checking in opaque blobs.
const SIMPLE_JSON_STR: &str = include_str!("data/simple/simple.json");

/// Helper function to run the `main` binary with the given arguments and return a
/// [`assert_cmd::assert::Assert`].
fn run_main(args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("jg").expect("Failed to find main binary");
    cmd.args(args);
    cmd.assert()
}

/// Write `data` into a temporary file with the given `suffix` (e.g. ".cbor") and return the
/// [`tempfile::NamedTempFile`] handle. The file stays alive as long as the handle is held.
fn temp_file_with(suffix: &str, data: &[u8]) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(suffix)
        .tempfile()
        .expect("create temp file");
    f.write_all(data).expect("write temp file");
    f.flush().expect("flush temp file");
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn nonexistent_field_simple_query() {
        let output = run_main(&["does.not.exist", SIMPLE_JSON_FILEPATH])
            .success()
            .code(0)
            .get_output()
            .stdout
            .clone();
        let output_str =
            String::from_utf8(output).expect("Invalid UTF-8 output");

        assert!(
            output_str.trim().is_empty(),
            "Expected no output for nonexistent field, got: {output_str:?}"
        );
    }

    #[test]
    fn nonexistent_file() {
        let assert = run_main(&["", ""]);
        assert.failure();
    }

    #[test]
    fn invalid_query() {
        let assert = run_main(&["unclosed\"", SIMPLE_JSON_FILEPATH]);
        assert.failure().code(1);
    }

    #[test]
    fn simple_query() {
        // Test a simple query "age" on simple.json. The output format is a
        // path header line followed by the JSON value, e.g.:
        //   "age":
        //   32
        // NOTE: --with-path is needed because assert_cmd captures stdout
        // via pipe, so path headers are auto-hidden without it.
        let assert = run_main(&["age", SIMPLE_JSON_FILEPATH, "--with-path"])
            .success()
            .code(0);
        let output_str = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");

        let mut lines = output_str.lines();
        let path_line = lines.next().expect("expected path header line");
        assert_eq!(path_line, "age:");

        let value_str: String = lines.collect::<Vec<_>>().join("\n");
        let output_json: Value = serde_json::from_str(value_str.trim())
            .expect("Failed to parse output JSON");
        let expected_json: Value =
            serde_json::from_str("32").expect("Failed to parse expected JSON");

        assert_eq!(output_json, expected_json);
    }

    // ==============================================================================
    // Quoted field and --fixed-string tests
    // ==============================================================================

    #[test]
    fn quoted_field_query_matches() {
        let assert = run_main(&[
            r#"paths."/activities""#,
            "tests/data/openapi_paths.json",
            "--count",
            "--no-display",
        ])
        .success()
        .code(0);
        let output_str = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");

        assert!(
            output_str.contains('1'),
            "Expected 1 match for quoted field query, got: {output_str:?}"
        );
    }

    #[test]
    fn fixed_string_finds_key_at_any_depth() {
        let assert =
            run_main(&["-F", "/activities", "tests/data/openapi_paths.json"])
                .success()
                .code(0);
        let output_str = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");

        // Should find the "/activities" key and output its value
        assert!(
            !output_str.trim().is_empty(),
            "Expected output for -F '/activities', got empty"
        );
    }

    #[test]
    fn fixed_string_count() {
        let assert = run_main(&[
            "-F",
            "/activities",
            "tests/data/openapi_paths.json",
            "--count",
            "--no-display",
        ])
        .success()
        .code(0);
        let output_str = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");

        // Only "/activities" should match, not "/activities/{id}"
        assert!(
            output_str.contains('1'),
            "Expected exactly 1 match for -F '/activities', got: {output_str:?}"
        );
    }

    #[test]
    fn fixed_string_no_match() {
        let output =
            run_main(&["-F", "/nonexistent", "tests/data/openapi_paths.json"])
                .success()
                .code(0)
                .get_output()
                .stdout
                .clone();
        let output_str =
            String::from_utf8(output).expect("Invalid UTF-8 output");

        assert!(
            output_str.trim().is_empty(),
            "Expected no output for nonexistent fixed string, got: {output_str:?}"
        );
    }

    // ==============================================================================
    // Path header display tests
    // ==============================================================================

    #[test]
    fn no_path_flag_suppresses_headers() {
        let assert = run_main(&["age", SIMPLE_JSON_FILEPATH, "--no-path"])
            .success()
            .code(0);
        let output_str = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");

        assert!(
            !output_str.contains("age:"),
            "Expected no path header with --no-path, got: {output_str:?}"
        );
        assert!(
            output_str.contains("32"),
            "Expected value to be present, got: {output_str:?}"
        );
    }

    #[test]
    fn with_path_flag_shows_headers() {
        let assert = run_main(&["age", SIMPLE_JSON_FILEPATH, "--with-path"])
            .success()
            .code(0);
        let output_str = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");

        assert!(
            output_str.contains("age:"),
            "Expected path header with --with-path, got: {output_str:?}"
        );
    }

    #[test]
    fn path_flags_are_mutually_exclusive() {
        run_main(&["age", SIMPLE_JSON_FILEPATH, "--with-path", "--no-path"])
            .failure();
    }

    // ==============================================================================
    // Multi-format input tests
    // ==============================================================================

    /// Helper: get the compact, no-path output of a query so we can compare
    /// across formats without worrying about whitespace differences.
    fn query_output(args: &[&str]) -> String {
        let assert = run_main(args).success().code(0);
        String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output")
    }

    /// Reference outputs from the canonical JSON file, used to verify that
    /// each format produces identical results after conversion.
    fn json_reference(query: &str) -> String {
        query_output(&[query, SIMPLE_JSON_FILEPATH, "--no-path", "--compact"])
    }

    // ---------- JSONL ----------

    #[test]
    fn jsonl_auto_detect_from_extension() {
        let output = query_output(&[
            "[0].email",
            SIMPLE_JSONL_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            r#""bguise0@indiegogo.com""#,
            "JSONL auto-detect should return exact first email"
        );
    }

    #[test]
    fn jsonl_explicit_format_flag() {
        let output = query_output(&[
            "-f",
            "jsonl",
            "[2].email",
            SIMPLE_JSONL_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            r#""brosling2@forbes.com""#,
            "JSONL explicit flag should index correctly (line 3 = [2])"
        );
    }

    #[test]
    fn jsonl_wildcard_returns_all_lines() {
        let output = query_output(&[
            "[*].id",
            SIMPLE_JSONL_FILEPATH,
            "--no-path",
            "--compact",
            "--count",
            "--no-display",
        ]);
        assert!(
            output.contains('3'),
            "JSONL [*].id should match all 3 lines, got: {output:?}"
        );
    }

    #[test]
    fn jsonl_stdin_with_format_flag() {
        let jsonl_content =
            std::fs::read_to_string(SIMPLE_JSONL_FILEPATH).expect("read jsonl");
        let mut cmd =
            Command::cargo_bin("jg").expect("Failed to find main binary");
        let assert = cmd
            .args(["-f", "jsonl", "[0].email", "--no-path", "--compact"])
            .write_stdin(jsonl_content)
            .assert()
            .success()
            .code(0);
        let output = String::from_utf8(assert.get_output().stdout.clone())
            .expect("Invalid UTF-8 output");
        assert_eq!(
            output.trim(),
            r#""bguise0@indiegogo.com""#,
            "JSONL via stdin should produce exact output"
        );
    }

    // ---------- YAML ----------

    #[test]
    fn yaml_scalar_field() {
        let output = query_output(&[
            "age",
            SIMPLE_YAML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("age").trim());
    }

    #[test]
    fn yaml_nested_field() {
        let output = query_output(&[
            "name.first",
            SIMPLE_YAML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            json_reference("name.first").trim(),
            "YAML nested field should match JSON equivalent"
        );
    }

    #[test]
    fn yaml_array_field() {
        let output = query_output(&[
            "hobbies",
            SIMPLE_YAML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            json_reference("hobbies").trim(),
            "YAML array field should match JSON equivalent"
        );
    }

    #[test]
    fn yaml_explicit_format_flag() {
        let output = query_output(&[
            "-f",
            "yaml",
            "name.last",
            SIMPLE_YAML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("name.last").trim(),);
    }

    // ---------- TOML ----------

    #[test]
    fn toml_scalar_field() {
        let output = query_output(&[
            "age",
            SIMPLE_TOML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("age").trim());
    }

    #[test]
    fn toml_nested_field() {
        let output = query_output(&[
            "name.first",
            SIMPLE_TOML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            json_reference("name.first").trim(),
            "TOML nested field should match JSON equivalent"
        );
    }

    #[test]
    fn toml_array_field() {
        let output = query_output(&[
            "hobbies",
            SIMPLE_TOML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            json_reference("hobbies").trim(),
            "TOML array field should match JSON equivalent"
        );
    }

    #[test]
    fn toml_explicit_format_flag() {
        let output = query_output(&[
            "-f",
            "toml",
            "name.last",
            SIMPLE_TOML_FILEPATH,
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("name.last").trim(),);
    }

    // ---------- CBOR ----------

    /// Generate a CBOR temp file from the canonical simple.json.
    fn cbor_temp(suffix: &str) -> tempfile::NamedTempFile {
        let value: serde_json::Value =
            serde_json::from_str(SIMPLE_JSON_STR).expect("parse simple.json");
        let mut cbor_buf = Vec::new();
        ciborium::into_writer(&value, &mut cbor_buf)
            .expect("CBOR serialization");
        temp_file_with(suffix, &cbor_buf)
    }

    #[test]
    fn cbor_scalar_field() {
        let tmp = cbor_temp(".cbor");
        let output = query_output(&[
            "age",
            tmp.path().to_str().expect("temp path"),
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("age").trim());
    }

    #[test]
    fn cbor_nested_field() {
        let tmp = cbor_temp(".cbor");
        let output = query_output(&[
            "name.first",
            tmp.path().to_str().expect("temp path"),
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            json_reference("name.first").trim(),
            "CBOR nested field should match JSON equivalent"
        );
    }

    #[test]
    fn cbor_explicit_format_flag() {
        let tmp = cbor_temp(".bin");
        let output = query_output(&[
            "-f",
            "cbor",
            "name.last",
            tmp.path().to_str().expect("temp path"),
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("name.last").trim(),);
    }

    // ---------- MessagePack ----------

    /// Generate a `MessagePack` temp file from the canonical simple.json.
    fn msgpack_temp(suffix: &str) -> tempfile::NamedTempFile {
        let value: serde_json::Value =
            serde_json::from_str(SIMPLE_JSON_STR).expect("parse simple.json");
        let msgpack_buf =
            rmp_serde::to_vec(&value).expect("MessagePack serialization");
        temp_file_with(suffix, &msgpack_buf)
    }

    #[test]
    fn msgpack_scalar_field() {
        let tmp = msgpack_temp(".msgpack");
        let output = query_output(&[
            "age",
            tmp.path().to_str().expect("temp path"),
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("age").trim());
    }

    #[test]
    fn msgpack_nested_field() {
        let tmp = msgpack_temp(".msgpack");
        let output = query_output(&[
            "name.first",
            tmp.path().to_str().expect("temp path"),
            "--no-path",
            "--compact",
        ]);
        assert_eq!(
            output.trim(),
            json_reference("name.first").trim(),
            "MessagePack nested field should match JSON equivalent"
        );
    }

    #[test]
    fn msgpack_explicit_format_flag() {
        let tmp = msgpack_temp(".bin");
        let output = query_output(&[
            "-f",
            "msgpack",
            "name.last",
            tmp.path().to_str().expect("temp path"),
            "--no-path",
            "--compact",
        ]);
        assert_eq!(output.trim(), json_reference("name.last").trim(),);
    }

    // ---------- Negative / error cases ----------

    #[test]
    fn yaml_content_with_json_format_fails() {
        run_main(&["-f", "json", "age", SIMPLE_YAML_FILEPATH]).failure();
    }

    #[test]
    fn malformed_yaml_gives_error() {
        let tmp = temp_file_with(".yaml", b"{{invalid yaml");
        run_main(&["age", tmp.path().to_str().expect("temp path")]).failure();
    }

    #[test]
    fn malformed_toml_gives_error() {
        let tmp = temp_file_with(".toml", b"[invalid\ntoml");
        run_main(&["age", tmp.path().to_str().expect("temp path")]).failure();
    }

    #[test]
    fn invalid_cbor_gives_error() {
        let tmp = temp_file_with(".cbor", b"\xff\xfe\xfd");
        run_main(&["age", tmp.path().to_str().expect("temp path")]).failure();
    }

    #[test]
    fn invalid_msgpack_gives_error() {
        // 0x85 declares a fixmap with 5 entries, but provide no key/value
        // pairs -> rmp_serde will fail reading the truncated map.
        let tmp = temp_file_with(".msgpack", b"\x85");
        run_main(&["age", tmp.path().to_str().expect("temp path")]).failure();
    }
}
