# jsongrep (`jg`)

`jsongrep` is a JSONPath-inspired query language over JSON documents.

## Table of Contents

- [Installation](#installation)
- [Usage](#usage)
  - [Query Syntax](#query-syntax)
- [Examples](#examples)
- [Contributing](#contributing)
- [License](#license)

## Installation

`jsongrep` can be installed using `cargo`:

```bash
cargo install jsongrep
```

The `jg` binary will be installed to `~/.cargo/bin`.

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

### Query Syntax

The query engine allows you to query JSON data using a simple DSL. It supports
the following operators:

- Field accesses: `"foo"`
- Array accesses (0-indexed): `"[0]" | "[start: end]"`
- Field and array wild cards: `"foo.*", "foo[*]"`
- Optional chaining: `"foo?.bar"`
- Kleene star: `"foo*"`
- Disjunction: `"foo | bar"`
- Sequence: `"foo.bar.baz"`

The complete grammar for the query language can be found in the
[grammar](./src/query/grammar) directory.

---

<details>
<summary>CLI</summary>

**Example**: Pass input file by path

`simple.json`:

```json
{
  "name": {
    "first": "John",
    "last": "Doe"
  },
  "age": 32,
  "hobbies": ["fishing", "yoga"]
}
```

The following query will follow an arbitrary amount of filed accesses followed
by a wildcard array access:

```bash
jg "**.[*]" simple.json
```

Output:

```text
[
  "fishing",
  "yoga"
]
```

**Example**: Pipe input from STDIN

```bash
curl https://api.nobelprize.org/v1/prize.json | jg "prizes[4].laureates[1].motivation"
```

Output:

```text
[
  "\"for foundational discoveries and inventions that enable machine learning with artificial neural networks\""
]
```

**Example**: Check number of matches without displaying them

Again, using the `simple.json` file:

```bash
jg "**.[*]" simple.json --count --no-display
```

Output:

```text
Found matches: 2
```

</details>

---

<details>
<summary>QueryBuilder API</summary>

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

## Examples

Examples of using the `jsongrep` crate can be found in the
[examples](./examples) directory.

## Contributing

Contributions are welcome! Please see the [CONTRIBUTING.md](CONTRIBUTING.md)
file for more details.

## License

This project is licensed under the MIT License - see the
[LICENSE.md](LICENSE.md) file for details.
