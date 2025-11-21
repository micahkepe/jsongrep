/*!
This module provides the main query engine implementation, as well as the parser for the query
language and the intermediary AST representations of queries.
*/
pub mod ast;
pub(crate) mod common;
pub mod dfa;
pub(crate) mod nfa;
pub mod parser;

use serde_json::Value;

use common::JSONPointer;

/// Interface for query engine implementations.
pub trait QueryEngine {
    /// Finds all JSON pointers in the given JSON document that match the
    /// specified query.
    ///
    /// Follows the semantics of JSONPath, returning the matched values as an
    /// array of `JSONPointer` instances.
    fn find<'a>(
        &self,
        json: &'a Value,
        query: &'a Query,
    ) -> Vec<JSONPointer<'a>>;
}

// Re-exports
pub use ast::*;
pub use dfa::*;
pub use nfa::*;
pub use parser::*;
