//! Demonstrates the full library flow: parse a query string, build a DFA, and
//! search a JSON document for matching paths.
//!
//! Run with:
//!
//! ```
//! cargo run --example parse_and_query
//! ```

use jsongrep::{
    Value,
    query::{Query, QueryDFA},
    utils::write_colored_result,
};
use std::io::{self, BufWriter, Write};

fn main() -> anyhow::Result<()> {
    let input = r#"{
        "users": [
            { "name": "Alice", "role": "admin" },
            { "name": "Bob",   "role": "viewer" }
        ]
    }"#;

    let json: Value = serde_json::from_str(input).expect("valid JSON");
    let mut writer = BufWriter::new(io::stdout().lock());

    // =========================================================================
    // Simple: one-liner with jsongrep::grep
    // =========================================================================

    writer.write_all(b"users[*].name results (via grep):\n")?;
    let results = jsongrep::grep(&json, "users[*].name").expect("valid query");

    for result in &results {
        write_colored_result(
            &mut writer,
            result.value,
            &result.path,
            true,
            true,
        )?;
    }
    writer.write_all(b"\n")?;

    // =========================================================================
    // Reusable: compile DFA once, run against multiple documents
    // =========================================================================

    let dfa = QueryDFA::from_query_str("name").expect("valid query syntax");

    let docs = [
        r#"{"name": "Alice", "age": 30}"#,
        r#"{"name": "Bob", "age": 25}"#,
        r#"{"name": "Charlie", "age": 35}"#,
    ];

    writer.write_all(b"Reusing compiled query across 3 documents:\n")?;
    for doc in &docs {
        let json: Value = serde_json::from_str(doc).expect("valid JSON");
        let results = dfa.find(&json);
        for result in &results {
            write_colored_result(
                &mut writer,
                result.value,
                &result.path,
                true,
                true,
            )?;
        }
    }
    writer.write_all(b"\n")?;

    // =========================================================================
    // Explicit: parse to AST first, then build DFA
    //
    // Useful when you need to inspect or transform the query before execution.
    // =========================================================================

    let query: Query = "users[*].*".parse().expect("valid query syntax");
    let dfa = QueryDFA::from_query(&query);
    let results = dfa.find(&json);

    writer.write_all(b"users[*].* results (via AST):\n")?;
    for result in &results {
        write_colored_result(
            &mut writer,
            result.value,
            &result.path,
            true,
            true,
        )?;
    }

    Ok(())
}
