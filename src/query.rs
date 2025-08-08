//! # JSON Query DSL
//!
//! A JSONPath-inspired query language for JSON documents with support for:
//! - Field access, array indexing, ranges
//! - Wildcards, optional matching, Kleene star
//! - Disjunction and regular expressions
//! - Streaming-friendly design returning pointers to matched values

pub mod ast;
pub(crate) mod common;
pub mod dfa;
pub(crate) mod nfa;
pub mod parser;

use crate::schema::JSONValue;
use common::JSONPointer;

/// Interface for query engine implementations.
pub trait QueryEngine {
    /// Finds all JSON pointers in the given JSON document that match the
    /// specified query.
    ///
    /// Follows the semantics of JSONPath, returning the matched values as an
    /// array of `JSONPointer` instances.
    fn find<'a>(&self, json: &'a JSONValue, query: &'a Query) -> Vec<JSONPointer<'a>>;
}

// Re-exports
pub use ast::*;
pub use dfa::*;
pub use nfa::*;
pub use parser::*;
