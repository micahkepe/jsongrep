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
        let assert =
            run_main(&["age", "tests/data/simple.json"]).success().code(0);
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
}
