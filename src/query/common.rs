/*!
# Shared Types

This module contains shared types used in the JSON query engine, including
the JSON pointer and path types. Additionally, this module defines the
predicate definitions for JSON automaton.
*/
use serde_json_borrow::Value;
use std::rc::Rc;

/// A JSON poenter that points to a value in a JSON document.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct JSONPointer<'a> {
    /// The path to the value in the JSON document, e.g., \["foo", "bar", "bass"\]
    pub path: Vec<PathType>,
    /// A reference to the value in the JSON document
    pub value: &'a Value<'a>,
}

impl std::fmt::Display for JSONPointer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "path: {:#?}", self.path)?;
        write!(f, "value: {:?}", self.value)
    }
}

/// Represents the type of path being explored in the query.
#[derive(Hash, PartialEq, Eq, Debug, Clone)]
pub enum PathType {
    /// Represents an index in an array, e.g., "foo\[3\]"
    Index(usize),
    /// Represents a field in an object, e.g., "foo.bar"
    Field(Rc<String>),
}

/// Represents the condition for a transition in an automaton from walking a
/// JSON document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionLabel {
    /// Matches a specific field name, e.g., "foo"
    /// Any field that doesn't match will be lumped together in the catch-all
    /// "other" symbol, which refers to key ID `0`.
    Field(Rc<String>),
    /// Matches any field name, e.g., "*"
    FieldWildcard,
    /// Matches a range of indices, e.g., "\[start:end\]"
    Range(usize, usize),
    /// Matches a range from a starting index, e.g., "\[start:\]"
    RangeFrom(usize),
    /// Special symbol for keys not in the query
    Other,
}

impl std::fmt::Display for TransitionLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Field(str) => write!(f, "Field({str})"),
            Self::FieldWildcard => write!(f, "FieldWildcard"),
            Self::Range(s, e) => write!(f, "Range({s}, {e})"),
            Self::RangeFrom(s) => write!(f, "RangeFrom({s})"),
            Self::Other => write!(f, "Other"),
        }
    }
}
