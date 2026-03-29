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
    query::{DFAQueryEngine, Query, QueryDFA},
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
    // Concise path: parse + build DFA in one step
    // =========================================================================

    writer.write_all(b"users[*].name results:\n")?;
    let dfa =
        QueryDFA::from_query_str("users[*].name").expect("valid query syntax");
    let results = DFAQueryEngine::find_with_dfa(&json, &dfa);

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
    // Explicit path: parse to AST first, then build DFA
    //
    // Useful when you need to inspect or transform the query before execution,
    // e.g., programmatic rewrites or logging the parsed structure.
    // =========================================================================

    let query: Query = "users[*].role".parse().expect("valid query syntax");
    let dfa = QueryDFA::from_query(&query);

    let results = DFAQueryEngine::find_with_dfa(&json, &dfa);

    writer.write_all(b"users[*].role results:\n")?;
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
