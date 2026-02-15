//! Integration test suite for `jsongrep` CLI
use assert_cmd::Command;

/// Helper function to run the `main` binary with the given arguments and return a
/// [`assert_cmd::assert::Assert`].
fn run_main(args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("jg").expect("Failed to find main binary");
    cmd.args(args);
    cmd.assert()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn nonexistent_field_simple_query() {
        let output = run_main(&["does.not.exist", "tests/data/simple.json"])
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
        let assert = run_main(&["unclosed\"", "tests/data/simple.json"]);
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
        let assert =
            run_main(&["age", "tests/data/simple.json", "--with-path"])
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
        let assert =
            run_main(&["age", "tests/data/simple.json", "--no-path"])
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
        let assert =
            run_main(&["age", "tests/data/simple.json", "--with-path"])
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
        run_main(&[
            "age",
            "tests/data/simple.json",
            "--with-path",
            "--no-path",
        ])
        .failure();
    }
}
