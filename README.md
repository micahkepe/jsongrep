# jsongrep (jg)

`jsongrep` is a JSONPath-inspired query language over JSON documents.

## Installation

## Usage

```
Query an input JSON document against a jsongrep query

Usage: jg [OPTIONS] <QUERY> [FILE]

Arguments:
  <QUERY>  Query string (e.g., "**.name")
  [FILE]   Optional path to JSON file. If omitted, reads from STDIN

Options:
      --compact     Do not pretty-print the JSON output, instead use compact
      --count       Display count of number of matches
      --depth       Display depth of the input document
  -n, --no-display  Do not display matched JSON values
  -h, --help        Print help
  -V, --version     Print version
```

<details>
<summary>`QueryBuilder` API</summary>

The DSL engine allows you to query JSON data using a simple DSL. It supports
the following operators:

- Field accesses: `"foo"`
- Array accesses (0-indexed): `"[0]" | "[start: end]"`
- Field and array wild cards: `"foo.*", "foo[*]"`
- Regex matching: `"/regex/"`
- Optional chaining: `"foo?.bar"`
- Kleene star: `"foo*"`
- Disjunction: `"foo | bar"`
- Sequence: `"foo.bar.baz"`

The `jsongrep::query::ast` module defines the `QueryBuilder` fluent API for
building queries. It allows you to construct queries using a builder pattern.

**Example Usage**:

```rust
// Construct the query "foo[0].bar.*.baz"
use jsongrep::query::engine::QueryBuilder;
let query = QueryBuilder::new()
    .field("foo")
    .index(0)
    .field("bar")
    .field_wildcard()
    .field("baz")
    .build();
```

</details>

---

<details>
<summary>Query CLI</summary>

**Example usage**:

Pass input file by path:

```bash
jg "**.[*]" ../data/json/simple-nested-arrays.json
```

Pipe input from STDIN:

```bash
curl https://api.nobelprize.org/v1/prize.json | jg "prizes[2:4].laureates[2:25].motivation"
```

</details>

## License

This project is licensed under the MIT License - see the
[LICENSE.md](LICENSE.md) file for details.
