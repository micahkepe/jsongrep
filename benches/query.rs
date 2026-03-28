//! Criterion benchmark suite comparing jsongrep against popular JSON query tools.
//!
//! Benchmark groups:
//! - `document_parse` -- JSON parse time by format
//! - `query_compile` -- Query compilation time
//! - `query_search` -- Search time with pre-compiled queries + pre-parsed docs
//! - `end_to_end` -- Full pipeline (parse + compile + search)

use criterion::{
    BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main,
};
use std::hint::black_box;

use jsongrep::query::parser::parse_query;
use jsongrep::query::{DFAQueryEngine, QueryDFA};

// === Test data (embedded at compile time -- no disk I/O variance) ===

const SMALL_JSON: &str = include_str!("../tests/data/simple/simple.json");
const MEDIUM_JSON: &str = include_str!(
    "../tests/data/schemastore/src/schemas/json/kubernetes-definitions.json"
);
const LARGE_JSON: &str = include_str!(
    "../tests/data/schemastore/src/schemas/json/kestra-0.19.0.json"
);

const DATA_SETS: &[(&str, &str)] =
    &[("small", SMALL_JSON), ("medium", MEDIUM_JSON), ("large", LARGE_JSON)];

/// Try to load the xlarge `GeoJSON` file from disk (`just bench-download`).
/// Returns `None` if the file is absent — xlarge benchmarks are silently skipped.
fn load_xlarge() -> Option<String> {
    std::fs::read_to_string("benches/data/citylots.json").ok()
}

// === Query definitions per tool ===

/// jsongrep query equivalencies across tools (where applicable)
struct QueryVariants {
    name: &'static str,
    jsongrep: &'static str,
    jsonpath: Option<&'static str>,
    jmespath: Option<&'static str>,
    jaq: Option<&'static str>,
    jql: Option<&'static str>,
    /// Only run on medium/large (schema) documents
    schema_only: bool,
    /// Only run on the xlarge (`GeoJSON`) document
    geojson_only: bool,
}

fn all_queries() -> Vec<QueryVariants> {
    vec![
        // --- Generic queries (all documents) ---
        QueryVariants {
            name: "simple_field",
            jsongrep: "name",
            jsonpath: Some("$.name"),
            jmespath: Some("name"),
            jaq: Some(".name"),
            jql: Some(r#""name""#),
            schema_only: false,
            geojson_only: false,
        },
        QueryVariants {
            name: "nested_path",
            jsongrep: "name.first",
            jsonpath: Some("$.name.first"),
            jmespath: Some("name.first"),
            jaq: Some(".name.first"),
            jql: Some(r#""name""first""#),
            schema_only: false,
            geojson_only: false,
        },
        QueryVariants {
            name: "array_index",
            jsongrep: "hobbies[0]",
            jsonpath: Some("$.hobbies[0]"),
            jmespath: Some("hobbies[0]"),
            jaq: Some(".hobbies[0]"),
            jql: Some(r#""hobbies"[0]"#),
            schema_only: false,
            geojson_only: false,
        },
        QueryVariants {
            name: "wildcard_field",
            jsongrep: "*",
            jsonpath: Some("$.*"),
            jmespath: Some("*"),
            jaq: Some(".[]"),
            jql: None,
            schema_only: false,
            geojson_only: false,
        },
        QueryVariants {
            name: "array_wildcard",
            jsongrep: "hobbies[*]",
            jsonpath: Some("$.hobbies[*]"),
            jmespath: Some("hobbies[*]"),
            jaq: Some(".hobbies[]"),
            jql: None,
            schema_only: false,
            geojson_only: false,
        },
        // --- Schema-specific queries (medium + large only) ---
        QueryVariants {
            name: "recursive_field",
            jsongrep: "(* | [*])*.description",
            jsonpath: Some("$..description"),
            jmespath: None,
            jaq: Some(".. | .description? // empty"),
            jql: Some(".."),
            schema_only: true,
            geojson_only: false,
        },
        QueryVariants {
            name: "deep_nested",
            jsongrep: "definitions.*.properties.*.type",
            jsonpath: Some("$.definitions.*.properties.*.type"),
            jmespath: Some("definitions.*.properties.*.type"),
            jaq: Some(".definitions[].properties[].type"),
            jql: None,
            schema_only: true,
            geojson_only: false,
        },
        // --- GeoJSON-specific queries (xlarge only) ---
        QueryVariants {
            name: "geo_all_geometry_types",
            jsongrep: "features[*].geometry.type",
            jsonpath: Some("$.features[*].geometry.type"),
            jmespath: Some("features[*].geometry.type"),
            jaq: Some(".features[].geometry.type"),
            jql: None,
            schema_only: false,
            geojson_only: true,
        },
        QueryVariants {
            name: "geo_recursive_coords",
            jsongrep: "(* | [*])*.coordinates",
            jsonpath: Some("$..coordinates"),
            jmespath: None,
            jaq: Some(".. | .coordinates? // empty"),
            jql: Some(".."),
            schema_only: false,
            geojson_only: true,
        },
    ]
}

// === Helpers ===

/// Compile a jaq filter from source code.
fn compile_jaq_filter(
    code: &str,
) -> jaq_core::Filter<jaq_core::Native<jaq_json::Val>> {
    let arena = jaq_core::load::Arena::default();
    let loader =
        jaq_core::load::Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let program = jaq_core::load::File { code, path: () };
    let modules = loader.load(&arena, program).unwrap();
    jaq_core::Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .unwrap()
}

/// Run a pre-compiled jaq filter on a Val, collecting all results.
fn run_jaq_filter(
    filter: &jaq_core::Filter<jaq_core::Native<jaq_json::Val>>,
    val: jaq_json::Val,
) -> Vec<Result<jaq_json::Val, jaq_core::Error<jaq_json::Val>>> {
    let inputs = jaq_core::RcIter::new(core::iter::empty());
    filter.run((jaq_core::Ctx::new([], &inputs), val)).collect()
}

/// Choose a `BatchSize` appropriate for the data size (avoids OOM on large inputs).
const fn batch_size_for(json_str: &str) -> BatchSize {
    if json_str.len() > 1_000_000 {
        BatchSize::PerIteration
    } else if json_str.len() > 100_000 {
        BatchSize::LargeInput
    } else {
        BatchSize::SmallInput
    }
}

// ==========================================================================
// Group 1: document_parse -- JSON parse time by format
// ==========================================================================

fn bench_document_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("document_parse");

    let xlarge_json = load_xlarge();
    let mut sets: Vec<(&str, &str)> = DATA_SETS.to_vec();
    if let Some(ref xl) = xlarge_json {
        sets.push(("xlarge", xl));
    }

    for (data_name, json_str) in &sets {
        if *data_name == "large" || *data_name == "xlarge" {
            group.sample_size(10);
        }

        // serde_json_borrow::Value (zero-copy)
        group.bench_with_input(
            BenchmarkId::new("serde_json_borrow", data_name),
            &json_str,
            |b, &s| {
                b.iter(|| {
                    black_box(
                        serde_json::from_str::<serde_json_borrow::Value<'_>>(s)
                            .unwrap(),
                    )
                });
            },
        );

        // serde_json::Value (used by jsonpath-rust, jaq, jql)
        group.bench_with_input(
            BenchmarkId::new("serde_json", data_name),
            &json_str,
            |b, &s| {
                b.iter(|| {
                    black_box(
                        serde_json::from_str::<serde_json::Value>(s).unwrap(),
                    )
                });
            },
        );

        // jmespath::Variable
        group.bench_with_input(
            BenchmarkId::new("jmespath_variable", data_name),
            &json_str,
            |b, &s| {
                b.iter(|| black_box(jmespath::Variable::from_json(s).unwrap()));
            },
        );
    }

    group.finish();
}

// ==========================================================================
// Group 2: query_compile -- Query compilation time
// ==========================================================================

fn bench_query_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_compile");
    let queries = all_queries();

    for q in &queries {
        // jsongrep: parse query AST + build DFA
        group.bench_with_input(
            BenchmarkId::new("jsongrep", q.name),
            &q.jsongrep,
            |b, &query_str| {
                b.iter(|| {
                    black_box(QueryDFA::from_query_str(query_str).unwrap())
                });
            },
        );

        // jmespath
        if let Some(jmespath_query) = q.jmespath {
            group.bench_with_input(
                BenchmarkId::new("jmespath", q.name),
                &jmespath_query,
                |b, &query_str| {
                    b.iter(|| black_box(jmespath::compile(query_str).unwrap()));
                },
            );
        }

        // jaq (arena + loader + parse + compile)
        if let Some(jaq_query) = q.jaq {
            group.bench_with_input(
                BenchmarkId::new("jaq", q.name),
                &jaq_query,
                |b, &code| {
                    b.iter(|| black_box(compile_jaq_filter(code)));
                },
            );
        }

        // jql
        if let Some(jql_query) = q.jql {
            group.bench_with_input(
                BenchmarkId::new("jql", q.name),
                &jql_query,
                |b, &query_str| {
                    b.iter(|| {
                        black_box(jql_parser::parser::parse(query_str).unwrap())
                    });
                },
            );
        }

        // jsonpath-rust: no separate compile step -- cost appears in query_search
    }

    group.finish();
}

// ==========================================================================
// Group 3: query_search -- Search with pre-compiled queries + pre-parsed docs
// ==========================================================================

#[allow(clippy::too_many_lines)]
fn bench_query_search(c: &mut Criterion) {
    let queries = all_queries();

    let xlarge_json = load_xlarge();
    let mut sets: Vec<(&str, &str)> = DATA_SETS.to_vec();
    if let Some(ref xl) = xlarge_json {
        sets.push(("xlarge", xl));
    }

    for (data_name, json_str) in &sets {
        let mut group = c.benchmark_group(format!("query_search/{data_name}"));
        if *data_name == "large" || *data_name == "xlarge" {
            group.sample_size(10);
        }

        // Pre-parse documents into each tool's format
        let borrow_doc: serde_json_borrow::Value<'_> =
            serde_json::from_str(json_str).unwrap();
        let serde_doc: serde_json::Value =
            serde_json::from_str(json_str).unwrap();
        let jmespath_doc = jmespath::Variable::from_json(json_str).unwrap();
        let jaq_val = jaq_json::Val::from(serde_doc.clone());
        let bs = batch_size_for(json_str);

        for q in &queries {
            if q.schema_only
                && (*data_name == "small" || *data_name == "xlarge")
            {
                continue;
            }
            if q.geojson_only && *data_name != "xlarge" {
                continue;
            }

            // jsongrep (pre-compiled DFA, zero-copy doc)
            {
                let dfa = QueryDFA::from_query_str(q.jsongrep).unwrap();
                group.bench_with_input(
                    BenchmarkId::new("jsongrep", q.name),
                    &(),
                    |b, ()| {
                        b.iter(|| {
                            black_box(DFAQueryEngine::find_with_dfa(
                                &borrow_doc,
                                &dfa,
                            ))
                        });
                    },
                );
            }

            // jsonpath-rust (includes query parse -- no separate compile step)
            if let Some(path) = q.jsonpath {
                group.bench_with_input(
                    BenchmarkId::new("jsonpath_rust", q.name),
                    &(),
                    |b, ()| {
                        use jsonpath_rust::JsonPath;
                        b.iter(|| {
                            let results: Vec<&serde_json::Value> =
                                serde_doc.query(path).unwrap();
                            black_box(results)
                        });
                    },
                );
            }

            // jmespath (pre-compiled expression; clone Variable per iteration)
            if let Some(jmespath_query) = q.jmespath {
                let expr = jmespath::compile(jmespath_query).unwrap();
                group.bench_with_input(
                    BenchmarkId::new("jmespath", q.name),
                    &(),
                    |b, ()| {
                        b.iter_batched(
                            || jmespath_doc.clone(),
                            |doc| black_box(expr.search(doc).unwrap()),
                            bs,
                        );
                    },
                );
            }

            // jaq (pre-compiled filter; clone Val per iteration -- jaq takes ownership)
            if let Some(jaq_query) = q.jaq {
                let filter = compile_jaq_filter(jaq_query);
                group.bench_with_input(
                    BenchmarkId::new("jaq", q.name),
                    &(),
                    |b, ()| {
                        b.iter_batched(
                            || jaq_val.clone(),
                            |val| black_box(run_jaq_filter(&filter, val)),
                            bs,
                        );
                    },
                );
            }

            // jql (pre-parsed tokens, operates on &Value)
            if let Some(jql_query) = q.jql {
                let tokens = jql_parser::parser::parse(jql_query).unwrap();
                group.bench_with_input(
                    BenchmarkId::new("jql", q.name),
                    &(),
                    |b, ()| {
                        b.iter(|| {
                            // jql may return Err for queries that don't match -- still measures work
                            black_box(jql_runner::runner::token(
                                &tokens, &serde_doc,
                            ))
                        });
                    },
                );
            }
        }

        group.finish();
    }
}

// ==========================================================================
// Group 4: end_to_end -- Full pipeline (parse + compile + search)
// ==========================================================================

fn bench_end_to_end(c: &mut Criterion) {
    let queries = all_queries();

    let xlarge_json = load_xlarge();
    let mut sets: Vec<(&str, &str)> = DATA_SETS.to_vec();
    if let Some(ref xl) = xlarge_json {
        sets.push(("xlarge", xl));
    }

    for (data_name, json_str) in &sets {
        let mut group = c.benchmark_group(format!("end_to_end/{data_name}"));
        if *data_name == "large" || *data_name == "xlarge" {
            group.sample_size(10);
        }

        for q in &queries {
            if q.schema_only
                && (*data_name == "small" || *data_name == "xlarge")
            {
                continue;
            }
            if q.geojson_only && *data_name != "xlarge" {
                continue;
            }

            // jsongrep: parse JSON (zero-copy) + parse query + build DFA + search
            group.bench_with_input(
                BenchmarkId::new("jsongrep", q.name),
                &(),
                |b, ()| {
                    b.iter(|| {
                        let doc: serde_json_borrow::Value<'_> =
                            serde_json::from_str(json_str).unwrap();
                        let query = parse_query(q.jsongrep).unwrap();
                        let dfa = QueryDFA::from_query(&query);
                        let results = DFAQueryEngine::find_with_dfa(&doc, &dfa);
                        black_box(results.len())
                    });
                },
            );

            // jsonpath-rust: parse JSON + query (includes path parsing)
            if let Some(path) = q.jsonpath {
                group.bench_with_input(
                    BenchmarkId::new("jsonpath_rust", q.name),
                    &(),
                    |b, ()| {
                        use jsonpath_rust::JsonPath;
                        b.iter(|| {
                            let doc: serde_json::Value =
                                serde_json::from_str(json_str).unwrap();
                            let results: Vec<&serde_json::Value> =
                                doc.query(path).unwrap();
                            black_box(results.len())
                        });
                    },
                );
            }

            // jmespath: parse JSON to Variable + compile expression + search
            if let Some(jmespath_query) = q.jmespath {
                group.bench_with_input(
                    BenchmarkId::new("jmespath", q.name),
                    &(),
                    |b, ()| {
                        b.iter(|| {
                            let doc = jmespath::Variable::from_json(json_str)
                                .unwrap();
                            let expr =
                                jmespath::compile(jmespath_query).unwrap();
                            black_box(expr.search(doc).unwrap())
                        });
                    },
                );
            }

            // jaq: parse JSON + compile filter + run
            if let Some(jaq_query) = q.jaq {
                group.bench_with_input(
                    BenchmarkId::new("jaq", q.name),
                    &(),
                    |b, ()| {
                        b.iter(|| {
                            let doc: serde_json::Value =
                                serde_json::from_str(json_str).unwrap();
                            let val = jaq_json::Val::from(doc);
                            let filter = compile_jaq_filter(jaq_query);
                            black_box(run_jaq_filter(&filter, val))
                        });
                    },
                );
            }

            // jql: parse JSON + parse query + run
            if let Some(jql_query) = q.jql {
                group.bench_with_input(
                    BenchmarkId::new("jql", q.name),
                    &(),
                    |b, ()| {
                        b.iter(|| {
                            let doc: serde_json::Value =
                                serde_json::from_str(json_str).unwrap();
                            black_box(jql_runner::runner::raw(jql_query, &doc))
                        });
                    },
                );
            }
        }

        group.finish();
    }
}

criterion_group!(
    benches,
    bench_document_parse,
    bench_query_compile,
    bench_query_search,
    bench_end_to_end,
);
criterion_main!(benches);
