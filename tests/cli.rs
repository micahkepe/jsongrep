//! Integration test suite for `jsongrep` CLI
use assert_cmd::Command;
use serde_json::Value;

/// Helper function to run the `main` binary with the given arguments and return a
/// [`assert_cmd::assert::Assert`].
fn run_main(args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("jg").expect("Failed to find main binary");
    cmd.args(args);
    cmd.assert()
}

#[test]
fn test_nonexistent_field_simple_query() {
    let output = run_main(&["does.not.exist", "tests/data/simple.json"])
        .success()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let output_str = String::from_utf8(output).expect("Invalid UTF-8 output");

    let expected = r#"[]"#;
    assert_eq!(output_str.trim(), expected);
}

#[test]
fn test_nonexistent_file() {
    let assert = run_main(&["", ""]);
    assert.failure();
}

#[test]
fn test_invalid_query() {
    let assert = run_main(&["unclosed\"", "tests/data/simple.json"]);
    assert.failure().code(1);
}

#[test]
fn test_simple_query() {
    // Test a simple query "age" on simple.json, expecting ["32"]
    let assert = run_main(&["age", "tests/data/simple.json"]).success().code(0); // Ensure the command exits successfully
    let output_str = String::from_utf8(assert.get_output().stdout.clone())
        .expect("Invalid UTF-8 output");

    // Parse the output JSON
    let output_json: Value =
        serde_json::from_str(&output_str).expect("Failed to parse output JSON");
    let expected_json: Value = serde_json::from_str(r#"["32"]"#)
        .expect("Failed to parse expected JSON");

    assert_eq!(output_json, expected_json);
}
