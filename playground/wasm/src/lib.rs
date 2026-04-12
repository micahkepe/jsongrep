use crate::exports::jsongrep::jsongrep::jsongrep::{
    Guest, TimingResults, Timings,
};
use jsongrep::{
    query::{DFAQueryEngine, QueryDFA},
    Value,
};
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

fn guess_input<'a>(input: &'a str) -> Result<Value<'a>, String> {
    if let Ok(x) = serde_json::from_str(&input).map_err(|e| e.to_string()) {
        return Ok(x);
    };

    if let Ok(x) = serde_yaml::from_str(&input).map_err(|e| e.to_string()) {
        return Ok(x);
    };

    Err(String::from("unsupported input"))
}

impl Guest for JsonGrepper {
    fn query_first(input: String, query: String) -> Result<String, String> {
        let json = guess_input(&input)?;

        let dfa =
            QueryDFA::from_query_str(&query).map_err(|e| e.to_string())?;
        let results = DFAQueryEngine::find_with_dfa(&json, &dfa);

        let mut data = None;
        for result in &results {
            data = Some(result.value);
            break;
        }

        serde_json::to_string_pretty(&data).map_err(|e| e.to_string())
    }

    fn query(input: String, query: String) -> Result<Vec<String>, String> {
        let json = guess_input(&input)?;

        let dfa =
            QueryDFA::from_query_str(&query).map_err(|e| e.to_string())?;
        let results = DFAQueryEngine::find_with_dfa(&json, &dfa);

        let mut data = Vec::new();
        for result in &results {
            data.push(
                serde_json::to_string_pretty(result.value)
                    .map_err(|e| e.to_string())?,
            );
        }

        Ok(data)
    }

    fn query_with_path(
        input: String,
        query: String,
    ) -> Result<Vec<(String, String)>, String> {
        let result = Self::query_with_timings(input, query)?;
        Ok(result.results)
    }

    fn query_with_timings(
        input: String,
        query: String,
    ) -> Result<TimingResults, String> {
        let before_parsing_input = monotonic_clock::now();
        let json = guess_input(&input)?;
        let after_parsing_input = monotonic_clock::now();

        let before_compile_query = monotonic_clock::now();
        let dfa =
            QueryDFA::from_query_str(&query).map_err(|e| e.to_string())?;
        let after_compile_query = monotonic_clock::now();

        let before_run_query = monotonic_clock::now();
        let results = DFAQueryEngine::find_with_dfa(&json, &dfa);
        let after_run_query = monotonic_clock::now();

        let before_serializing = monotonic_clock::now();
        let mut data = Vec::new();
        for result in &results {
            let path_parts: Vec<_> =
                result.path.iter().map(|x| x.to_string()).collect();
            let string_path: String = path_parts.join(".");
            data.push((
                string_path,
                serde_json::to_string_pretty(result.value)
                    .map_err(|e| e.to_string())?,
            ));
        }
        let after_serializing = monotonic_clock::now();

        Ok(TimingResults {
            results: data,
            timings: Timings {
                compile_ns: after_compile_query - before_compile_query,
                query_ns: after_run_query - before_run_query,
                parsing_ns: after_parsing_input - before_parsing_input,
                stringify_ns: after_serializing - before_serializing,
            },
        })
    }
}

export!(JsonGrepper);
