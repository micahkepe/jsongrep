#![allow(rustdoc::private_intra_doc_links)]
/*!
A query language for JSON data that searches for matching **regular** paths in
the JSON tree, using a derivation of [regular expressions].

[regular expressions]: https://en.wikipedia.org/wiki/Regular_expression

# Quick Start

```
let input = r#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
let json: jsongrep::Value = serde_json::from_str(input).unwrap();

let results = jsongrep::grep(&json, "users[*].name").unwrap();

assert_eq!(results.len(), 2);
assert_eq!(results[0].value.to_string(), r#""Alice""#);
assert_eq!(results[1].value.to_string(), r#""Bob""#);
```

For more examples, see the [`examples`](https://github.com/micahkepe/jsongrep/tree/main/examples)
directory in the repository.

# Overview

The engine is implemented as a [deterministic finite automaton (DFA)]. The DFA
is constructed from a query AST, which is a tree-like structure that represents
the query. The DFA is then used to search for matches in the input JSON data.

[deterministic finite automaton (DFA)]: https://en.wikipedia.org/wiki/Deterministic_finite_automaton

A JSON data structure is represented as a tree, where each node is a JSON value
(string, number, boolean, null, or object/array) and each edge is either a field
name or an index. For example, let's consider the following JSON data:

```json
{
    "name": "John Doe",
    "age": 30,
    "foo": [1, 2, 3]
}
```

The corresponding tree structure would be the root node, with three edges:
`"name"`, `"age"`, and `"foo"`. The `"name"` edge would point to the string
`"John Doe"` and the `"age"` edge would point to the number `30`. The `"foo"`
edge would point to a node with three edges of the array access `[0]`, `[1]`,
and `[2]`, which point to the numbers `1`, `2`, and `3`, respectively.

To query the JSON document, the query and document are both parsed into intermediary
ASTs. The query AST is then used to construct first a non-deterministic finite
automaton (NFA) which is then determinized into a deterministic finite automaton
(DFA) that can be directly simulated against the input JSON document.

For more details on the automaton constructions, see the [`dfa`] and
[`nfa`] modules of the [`query`] module.

# Query Language

The query language relies on regular expression syntax, with some modifications
to support JSON.

| Operator     | Example              | Description                                                   |
| ------------ | -------------------- | ------------------------------------------------------------- |
| Sequence     | `foo.bar.baz`        | **Concatenation**: match path `foo` &rarr; `bar` &rarr; `baz` |
| Disjunction  | `foo \| bar`         | **Union**: match either `foo` or `bar`                        |
| Kleene star  | `**`                 | Match zero or more field accesses                             |
| Repetition   | `foo*`               | Repeat the preceding step zero or more times                  |
| Wildcards    | `*` or `[*]`         | Match any single field or array index                         |
| Optional     | `foo?.bar`           | Optional `foo` field access                                   |
| Field access | `foo` or `"foo bar"` | Match a specific field (quote if spaces)                      |
| Array index  | `[0]` or `[1:3]`     | Match specific index or slice (exclusive end)                 |

These queries can be arbitrarily nested with parentheses. For example,
`foo.(bar|baz).qux` matches `foo.bar.qux` or `foo.baz.qux`.

This also means that you can recursively descend **any** path with `(* | [*])*`,
e.g., `(* | [*])*.foo` to find all paths matching `foo` field at any
depth.
Here are some example queries and their meanings:

- `name`: Matches the `name` field in the root object (e.g., ```"John Doe"```).
- `address.street`: Matches the `street` field inside the `address` object.
- `address.*`: Matches any field in the `address` object (e.g., `street`, `city`, etc.).
- `address.[*]`: Matches all elements in an array if `address` were an array.
- `(name|age)`: Matches either the `name` or `age` field in the root object.
- `address.([*] | *)*`: Matches any value at any depth under `address`.

We can also use ranges to match specific indices in arrays:

- `foo.[2:4]`: Matches elements at indices 2 and 3 in the `foo` array.
- `foo.[2:]`: Matches all elements in the `foo` array from index 2 onward.

Finally, we can use wildcards to match any field or index:

- `*`: Matches any single field in the root object.
- `[*]`: Matches any single array index in the root array.
- `[*].*`: Matches any field inside each element of an array.
- `([*] | *)*`: Matches any field or index at any level of the JSON tree.

## Playground

You can try queries interactively in the [playground](https://micahkepe.com/jsongrep/playground/).

[`nfa`]: crate::query::nfa
[`dfa`]: crate::query::dfa
[`query`]: crate::query
*/
pub mod commands;
pub mod query;
pub mod utils;

/// Re-export [`serde_json_borrow::Value`] so downstream users don't need to
/// depend on `serde_json_borrow` directly.
pub use serde_json_borrow::Value;

/// Query a JSON document with a query string, returning all matches.
///
/// This is the simplest entry point for the library. For repeated queries
/// against different documents, prefer compiling the query once with
/// [`query::QueryDFA::from_query_str`] and calling [`query::QueryDFA::find`].
///
/// # Errors
///
/// Returns an error if the query string is invalid.
///
/// # Examples
///
/// ```
/// let json: jsongrep::Value = serde_json::from_str(r#"{"a": 1, "b": 2}"#).unwrap();
/// let results = jsongrep::grep(&json, "a").unwrap();
/// assert_eq!(results.len(), 1);
/// assert_eq!(results[0].value.to_string(), "1");
/// ```
pub fn grep<'a>(
    json: &'a Value<'a>,
    query: &str,
) -> Result<Vec<query::JSONPointer<'a>>, query::QueryParseError> {
    let dfa = query::QueryDFA::from_query_str(query)?;
    Ok(dfa.find(json))
}
