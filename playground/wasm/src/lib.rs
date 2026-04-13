use crate::exports::jsongrep::jsongrep::jsongrep::{
    Guest, TimingResults, Timings,
};
use jsongrep::query::{DFAQueryEngine, QueryDFA};
use wasip2::clocks::monotonic_clock;

wit_bindgen::generate!(
    {
        with: {
            "wasi:io/poll@0.2.6": wasip2::io::poll,
            "wasi:clocks/monotonic-clock@0.2.6": wasip2::clocks::monotonic_clock,
        }
    }
);

struct JsonGrepper;

/// Normalize input to a JSON string. YAML and other text formats are converted through
/// `serde_json::Value` so that serde_json_borrow::Value can safely borrow from the resulting JSON
/// string.
fn normalize_to_json(input: &str) -> Result<String, String> {
    // Try JSON first -> if valid, return as-is
    if serde_json::from_str::<serde_json::Value>(input).is_ok() {
        return Ok(input.to_string());
    }

    // Try YAML -> must round-trip through serde_json::Value
    match serde_yaml::from_str::<serde_json::Value>(input) {
        Ok(value) => serde_json::to_string(&value).map_err(|e| e.to_string()),
        Err(yaml_err) => {
            Err(format!("Failed to parse as JSON or YAML: {yaml_err}"))
        }
    }
}

/// Shared query logic: parse JSON string, compile DFA, run query, return
/// results with timings.
fn run_query(input: &str, query: &str) -> Result<TimingResults, String> {
    let before_parsing = monotonic_clock::now();
    let json_str = normalize_to_json(input)?;
    let json: jsongrep::Value =
        serde_json::from_str(&json_str).map_err(|e| e.to_string())?;
    let after_parsing = monotonic_clock::now();

    let before_compile = monotonic_clock::now();
    let dfa = QueryDFA::from_query_str(query).map_err(|e| e.to_string())?;
    let after_compile = monotonic_clock::now();

    let before_query = monotonic_clock::now();
    let results = DFAQueryEngine::find_with_dfa(&json, &dfa);
    let after_query = monotonic_clock::now();

    let before_serialize = monotonic_clock::now();
    let mut data = Vec::new();
    for result in &results {
        let path_parts: Vec<_> =
            result.path.iter().map(|x| x.to_string()).collect();
        let string_path = path_parts.join(".");
        data.push((
            string_path,
            serde_json::to_string_pretty(result.value)
                .map_err(|e| e.to_string())?,
        ));
    }
    let after_serialize = monotonic_clock::now();

    Ok(TimingResults {
        results: data,
        timings: Timings {
            compile_ns: after_compile - before_compile,
            query_ns: after_query - before_query,
            parsing_ns: after_parsing - before_parsing,
            stringify_ns: after_serialize - before_serialize,
        },
    })
}

impl Guest for JsonGrepper {
    fn query_first(input: String, query: String) -> Result<String, String> {
        let result = run_query(&input, &query)?;
        let (_, value) =
            result.results.into_iter().next().ok_or("no matches found")?;
        Ok(value)
    }

    fn query(input: String, query: String) -> Result<Vec<String>, String> {
        let result = run_query(&input, &query)?;
        Ok(result.results.into_iter().map(|(_, value)| value).collect())
    }

    fn query_with_path(
        input: String,
        query: String,
    ) -> Result<Vec<(String, String)>, String> {
        let result = run_query(&input, &query)?;
        Ok(result.results)
    }

    fn query_with_timings(
        input: String,
        query: String,
    ) -> Result<TimingResults, String> {
        run_query(&input, &query)
    }
}

export!(JsonGrepper);
