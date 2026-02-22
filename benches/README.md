# `jsongrep` Benchmarking

Criterion benchmark suite comparing `jsongrep` against popular JSON query
tools. Inspired by the
[ripgrep benchmarking methodology](https://burntsushi.net/ripgrep/): equivalent
work across tools, realistic queries, transparency about what's measured, and
statistical rigor (via Criterion).

## Compared Tools

| Tool              | Crate(s)                            | JSON Type                  | Compile Step                               | Query Syntax                    |
| ----------------- | ----------------------------------- | -------------------------- | ------------------------------------------ | ------------------------------- |
| **jsongrep**      | this crate                          | `serde_json_borrow::Value` | `Query::from_str` + `QueryDFA::from_query` | `foo.bar`, `**`, `[*]`, `[1:3]` |
| **jsonpath-rust** | `jsonpath-rust`                     | `serde_json::Value`        | None (inline)                              | `$.foo.bar`, `$..x`, `$[*]`     |
| **jmespath**      | `jmespath`                          | `jmespath::Variable`       | `jmespath::compile()`                      | `foo.bar`, `[*]`, `[0:3]`       |
| **jaq**           | `jaq-core` + `jaq-std` + `jaq-json` | `jaq_json::Val`            | Arena + Loader + Compiler                  | `.foo.bar`, `..`, `.[]`         |
| **jql**           | `jql-parser` + `jql-runner`         | `serde_json::Value`        | `jql_parser::parser::parse()`              | `"foo"."bar"`, `..`, `[0]`      |

## Running

```sh
cargo bench --bench query
```

Or via justfile:

```sh
just bench
```

HTML reports are generated at `target/criterion/report/index.html`.

## Benchmark Groups

### 1. `document_parse` — JSON parse time by format

Isolates the cost of parsing raw JSON into each tool's in-memory
representation. jsongrep's `serde_json_borrow::Value` is zero-copy while
others allocate.

Formats: `serde_json_borrow::Value`, `serde_json::Value`, `jmespath::Variable`

### 2. `query_compile` — Query compilation time

Measures DFA construction (jsongrep), expression compilation (jmespath, jaq,
jql). jsonpath-rust has no separate compile step — its cost appears entirely
in `query_search`.

### 3. `query_search` — Search with pre-compiled queries + pre-parsed docs

The core benchmark. Pre-compiles queries and pre-parses documents, measuring
only the traversal/execution.

### 4. `end_to_end` — Full pipeline (parse + compile + search)

Closest to real CLI usage. Nothing pre-cached.

## Test Data

| Tier   | File                                                     | Size    |
| ------ | -------------------------------------------------------- | ------- |
| Small  | `tests/data/simple.json`                                 | 106 B   |
| Medium | `tests/data/schemastore/.../kubernetes-definitions.json` | ~992 KB |
| Large  | `tests/data/schemastore/.../kestra-0.19.0.json`          | ~7.6 MB |

All loaded via `include_str!` for reproducibility (no disk I/O variance).

## Query Equivalence

**Generic queries (all documents):**

| Name             | jsongrep     | JSONPath       | JMESPath     | jq (jaq)      | jql             |
| ---------------- | ------------ | -------------- | ------------ | ------------- | --------------- |
| `simple_field`   | `name`       | `$.name`       | `name`       | `.name`       | `"name"`        |
| `nested_path`    | `name.first` | `$.name.first` | `name.first` | `.name.first` | `"name""first"` |
| `array_index`    | `hobbies[0]` | `$.hobbies[0]` | `hobbies[0]` | `.hobbies[0]` | `"hobbies"[0]`  |
| `wildcard_field` | `*`          | `$.*`          | `*`          | `.[]`         | N/A             |
| `array_wildcard` | `hobbies[*]` | `$.hobbies[*]` | `hobbies[*]` | `.hobbies[]`  | N/A             |

**Schema-specific queries (medium/large documents):**

| Name              | jsongrep                          | JSONPath                            | JMESPath                          | jq (jaq)                           | jql  |
| ----------------- | --------------------------------- | ----------------------------------- | --------------------------------- | ---------------------------------- | ---- |
| `recursive_field` | `(* \| [*])*.description`         | `$..description`                    | N/A                               | `.. \| .description? // empty`     | `..` |
| `deep_nested`     | `definitions.*.properties.*.type` | `$.definitions.*.properties.*.type` | `definitions.*.properties.*.type` | `.definitions[].properties[].type` | N/A  |

## Fairness Notes

1. **Parse format difference:** jsongrep's zero-copy parsing is a genuine
   advantage, isolated in the `document_parse` group.
2. **Query compilation cost:** jsongrep's DFA construction is heavier upfront
   but enables faster traversal — shown in separate groups.
3. **Result consumption:** All results are fully collected/consumed via
   `black_box` to ensure equivalent work.
4. **Missing capabilities:** When a tool lacks a feature (e.g., JMESPath has no
   recursive descent), the benchmark is skipped — not faked.
5. **jsonpath-rust has no separate compile step** — its cost appears entirely
   in `query_search`, making those numbers not directly comparable to
   pre-compiled tools.
6. **Ownership semantics:** jaq and jmespath require ownership of the input
   value. In `query_search`, `iter_batched` separates the clone cost from the
   measured search time.

## Machine Info

I ran the benchmarks on my 2021 MacBook Pro (M1, 16 GB RAM, 1 TB SSD).

## References

- [ripgrep is faster than {grep, ag, git grep, ucg, pt, sift}](https://burntsushi.net/ripgrep/)
