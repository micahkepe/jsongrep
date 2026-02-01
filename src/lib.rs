/*!
This crate provides a query language for JSON data that can be used to search
for matching **regular** paths the JSON tree, using a derivation of [regular
expressions].

[regular expressions]: https://en.wikipedia.org/wiki/Regular_expression

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

More more details on the automaton constructions can be found in the `dfa` and
`nfa` modules of the `query` module.

# Query Language

The query language relies on regular expression syntax, with some modifications
to support JSON.

## Grammar

The grammar for the query language is defined in the `query.pest` file in the
`grammar` directory.

# Examples

Here are some example queries and their meanings:

- `name`: Matches the `name` field in the root object (e.g., ```"John Doe"```).
- `address.street`: Matches the `street` field inside the `address` object.
- `address.*`: Matches any field in the `address` object (e.g., `street`, `city`, etc.).
- `address.[*]`: Matches all elements in an array if `address` were an array.
- `(name|age)`: Matches either the `name` or `age` field in the root object.
- `address.*.*`: Matches any field in any object nested under `address`.

We can also use ranges to match specific indices in arrays:

- `address.[2:4]`: Matches the `street` and `city` fields in the `address` object.
- `address.[2:]`: Matches all elements in the `address` array after index 2.

Finally, we can use wildcards to match any field or index:

- `*`: Matches any field or index in the root object.
- `[*]`: Matches any field or index in any object nested under the root object.
- `[*].*`: Matches any field or index in any object nested under any object
  nested under the root object.
- `([*] | *)*`: Matches any filed or index at any level of the JSON tree.
*/

use serde_json::Value;

pub mod commands;
pub mod query;

/// Returns the depth of the JSON value.
pub fn depth(json: &Value) -> usize {
    match json {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
        Value::Array(arr) => {
            let inner_depth = arr.iter().map(depth).max().unwrap_or(0);
            1 + inner_depth
        }
        Value::Object(map) => {
            let inner_depth = map.values().map(depth).max().unwrap_or(0);
            1 + inner_depth
        }
    }
}
