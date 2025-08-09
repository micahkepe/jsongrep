/*!
This crate provides a query language for JSON data that can be used to search
for matching **regular** paths the JSON tree, using a derivation of [regular expressions].

[regular expressions]: https://en.wikipedia.org/wiki/Regular_expression
*/

pub mod query;
pub mod schema;
pub mod tokenizer;
