/*!
# Query AST and Builder

Defines the AST definition of JSON queries and exposes a fluent API for
constructing queries using a builder pattern.

# Examples

This module provides a fluent API for constructing queries using the
`QueryBuilder`.

For example, to construct a query that accesses a field named "foo", you can
use:
```
use jsongrep::query::{Query, QueryBuilder};
let query = QueryBuilder::new().field("foo").build();
assert_eq!(query, Query::Sequence(vec![Query::field("foo")]));
```

In addition, the query can be constructed from a raw string:

```
use jsongrep::query::Query;
let query : Query = "foo".parse().expect("Invalid query");
assert_eq!(query, Query::Sequence(vec![Query::field("foo")]));
```
*/
use std::{
    cmp::PartialEq,
    fmt::Display,
    ops::{Bound, RangeBounds},
    str::FromStr,
};

use super::{QueryParseError, parse_query};

/// The `Query` enum represents the different types of queries that can be
/// constructed
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Query {
    /// Field access by exact name, e.g., "foo"
    Field(String),
    /// Array index access (0-based), e.g, "\[3\]")
    Index(usize),
    /// Array range access with start and end: "\[3:5\]"
    ///
    /// NOTE: The end index is exclusive, so the range is `start..end`.
    Range(Option<usize>, Option<usize>),
    /// Array range access from a starting index, e.g., "foo\[3:\]"
    RangeFrom(usize),
    /// Wildcard field access, e.g., "foo.*". Represents a single-level field
    /// wildcard access and not a recursive descent.
    FieldWildcard,
    /// Wildcard array access, e.g., "foo\[*\]"
    ArrayWildcard,
    /// Regex access, e.g., "/regex/"
    Regex(String),
    /// Optional access, e.g., "?"
    /// This represents an optional query that may or may not match.
    Optional(Box<Self>),
    /// Kleene star, e.g., "foo*"
    KleeneStar(Box<Self>),
    /// Disjunction, e.g., "foo | bar"
    /// This represents a logical OR between an arbitrary number of queries.
    Disjunction(Vec<Self>),
    /// Sequence, e.g., "foo.bar"
    /// A wrapper for a sequence of queries that can be executed in order.
    Sequence(Vec<Self>),
}

impl Query {
    /// Calculate the depth of the query.
    #[must_use]
    pub fn depth(&self) -> usize {
        match self {
            Self::Disjunction(subqueries) => {
                1 + subqueries.iter().map(Self::depth).max().unwrap_or(0)
            }
            Self::Sequence(queries) => {
                queries.iter().map(Self::depth).sum::<usize>()
            }
            Self::Optional(inner) | Self::KleeneStar(inner) => {
                1 + inner.depth()
            }
            _ => 1,
        }
    }

    /// Helper for ergonomic construction of field queries
    pub fn field<T: Into<String>>(name: T) -> Self {
        Self::Field(name.into())
    }
}

impl Display for Query {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Field(name) => {
                if needs_quoting(name) {
                    write!(f, "\"{}\"", escape_for_quoted_field(name))
                } else {
                    write!(f, "{name}")
                }
            }
            Self::Index(idx) => write!(f, "[{idx}]"),
            Self::Range(start, end) => {
                write!(f, "[")?;
                if let Some(s) = start {
                    write!(f, "{s}")?;
                }
                write!(f, ":")?;
                if let Some(e) = end {
                    write!(f, "{e}")?;
                }
                write!(f, "]")
            }
            Self::RangeFrom(start) => write!(f, "[{start}:]"),
            Self::FieldWildcard => write!(f, "*"),
            Self::ArrayWildcard => write!(f, "[*]"),
            Self::Regex(re) => write!(f, "/{re}/"),
            Self::Optional(q) => match &**q {
                Self::Disjunction(queries) | Self::Sequence(queries) => {
                    if queries.len() > 1 {
                        write!(f, "({q})?")
                    } else {
                        write!(f, "{q}?")
                    }
                }
                _ => write!(f, "{q}?"),
            },
            Self::KleeneStar(q) => match &**q {
                Self::Disjunction(queries) | Self::Sequence(queries) => {
                    if queries.len() > 1 {
                        write!(f, "({q})*")
                    } else {
                        write!(f, "{q}*")
                    }
                }
                _ => write!(f, "{q}*"),
            },
            Self::Disjunction(queries) => {
                let joined = queries
                    .iter()
                    .map(|q| format!("{q}"))
                    .collect::<Vec<_>>()
                    .join(" | ");
                write!(f, "{joined}")
            }
            Self::Sequence(queries) => {
                /*
                 * For fields we don't want `.` delimiters between the optional
                 * range accesses and/or postfix unary operators, e.g, the query
                 * "foo.bar[0]?.baz*" should be formatted as such, and NOT as
                 * "foo.bar.[0]?.baz"
                 */
                for (i, query) in queries.iter().enumerate() {
                    if i > 0
                        && let Some(prev_query) = queries.get(i - 1)
                    {
                        /* Handle optional modifiers -> extract inner queries */
                        let inner_query = match query {
                            Self::Optional(inner) | Self::KleeneStar(inner) => {
                                inner
                            }
                            _ => query,
                        };
                        let prev_inner = match prev_query {
                            Self::Optional(inner) | Self::KleeneStar(inner) => {
                                inner
                            }
                            _ => prev_query,
                        };
                        /* Handle field accessed followed by a ranged accessed. */
                        match (prev_inner, inner_query) {
                            (
                                Self::Field(_),
                                Self::Index(_)
                                | Self::Range(_, _)
                                | Self::RangeFrom(_)
                                | Self::FieldWildcard
                                | Self::ArrayWildcard,
                            ) => {
                                // continue; no '.' separator
                            }
                            _ => write!(f, ".")?,
                        }
                    }

                    // Wrap disjunctions in a sequence with parentheses
                    match query {
                        Self::Disjunction(_) => write!(f, "({query})")?,
                        _ => write!(f, "{query}")?,
                    }
                }
                Ok(())
            }
        }
    }
}

/// Returns `true` if a field name contains characters that require quoting
/// in the query DSL. This mirrors the pest grammar's `unquoted_field` rule,
/// which forbids reserved characters, whitespace, and double quotes.
fn needs_quoting(name: &str) -> bool {
    // An empty field name cannot be represented unquoted
    name.is_empty()
        || name.contains(|c: char| {
            matches!(c, '.' | '|' | '*' | '?' | '[' | ']' | '(' | ')' | '/')
                || c.is_whitespace()
                || c == '"'
                || c == '\\'
        })
}

/// Escape characters inside a quoted field name for display. This is the
/// inverse of `unescape_json_string` in the parser: `"` -> `\"` and
/// `\` -> `\\`.
fn escape_for_quoted_field(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            _ => result.push(c),
        }
    }
    result
}

impl FromStr for Query {
    type Err = QueryParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_query(s)
    }
}

/// Constructs a query that matches a specific field name.
#[macro_export]
macro_rules! field {
    ($name: expr) => {
        Query::Field($name.to_owned())
    };
}

/// Builder for constructing queries
pub struct QueryBuilder {
    /// The underlying query being built
    query: Query,
}

impl QueryBuilder {
    /// Creates a new `QueryBuilder` instance with an empty query.
    ///
    /// # Examples
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let builder = QueryBuilder::new();
    /// assert!(matches!(builder.build(), Query::Sequence(_)));
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self { query: Query::Sequence(vec![]) }
    }

    /// Adds a field access to the query.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new().field("foo").build();
    /// assert_eq!(query, Query::Sequence(vec![Query::field("foo")]));
    /// ```
    #[must_use]
    pub fn field(mut self, name: &str) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) => {
                // append to current sequence
                seq.push(Query::Field(name.to_string()));
                Query::Sequence(seq)
            }
            q => Query::Sequence(vec![q, Query::Field(name.to_string())]),
        };
        self
    }

    /// Adds an index access to the query.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new().index(3).build();
    /// assert_eq!(query, Query::Sequence(vec![Query::Index(3)]));
    /// ```
    #[must_use]
    pub fn index(mut self, idx: usize) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) => {
                seq.push(Query::Index(idx));
                Query::Sequence(seq)
            }
            q => Query::Sequence(vec![q, Query::Index(idx)]),
        };
        self
    }

    /// Wrap the last atom in an optional query. If the last atom is a sequence,
    /// it wraps the last element in an optional. If the query is empty or has
    /// no elements, it creates a new sequence with the optional as the only
    /// element.
    ///
    /// # Examples
    ///
    /// This example shows how to use the `optional` method to wrap a field query
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new().field("foo").optional().build();
    ///
    /// assert!(
    ///     matches!(query, Query::Sequence(ref seq) if matches!(seq[0], Query::Optional(_)))
    /// );
    /// ```
    ///
    /// If the query is empty, it creates a new sequence with the optional as
    /// the only element:
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new().optional().build();
    ///
    /// assert!(
    ///     matches!(query, Query::Sequence(seq) if seq.len() == 1 &&
    ///     matches!(seq[0], Query::Optional(_)))
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the query is empty.
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) if !seq.is_empty() => {
                let last_atom = seq.pop().unwrap();
                seq.push(Query::Optional(Box::new(last_atom)));
                Query::Sequence(seq)
            }
            // Nothing yet, wrap empty sequence in an optional
            q => Query::Sequence(vec![Query::Optional(Box::new(q))]),
        };
        self
    }

    /// Add a Kleene star to the last atom in the query. If the last atom is a
    /// sequence, it wraps the last element in a Kleene star. Else, if the query
    /// is empty or has no elements, it creates a new sequence with the Kleene
    /// star as the only element.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new().field("foo").kleene_star().build();
    /// assert!(matches!(query, Query::Sequence(ref seq) if matches!(seq[0], Query::KleeneStar(_))));
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the query is empty.
    #[must_use]
    pub fn kleene_star(mut self) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) if !seq.is_empty() => {
                let last_atom = seq.pop().unwrap();
                seq.push(Query::KleeneStar(Box::new(last_atom)));
                Query::Sequence(seq)
            }
            q => Query::Sequence(vec![Query::KleeneStar(Box::new(q))]),
        };
        self
    }

    /// Adds a range query to the last atom in the query.
    ///
    /// # Examples
    ///
    /// Apply a range to the last atom in the query:
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    ///
    /// // Query: "foo[3:5]"
    /// let query = QueryBuilder::new().field("foo").range(3..5).build();
    /// assert!(
    ///     matches!(query, Query::Sequence(ref seq) if matches!(seq[0], Query::Field(_)) &&
    ///     matches!(seq[1], Query::Range(Some(3), Some(5))))
    /// );
    /// ```
    #[must_use]
    pub fn range(mut self, range: impl RangeBounds<usize>) -> Self {
        let start = match range.start_bound() {
            Bound::Included(&s) => Some(s),
            Bound::Excluded(&s) => Some(s + 1),
            Bound::Unbounded => None,
        };
        let end = match range.end_bound() {
            Bound::Included(&e) => Some(e + 1),
            Bound::Excluded(&e) => Some(e),
            Bound::Unbounded => None,
        };

        let q = Query::Range(start, end);

        self.query = match self.query {
            Query::Sequence(mut seq) => {
                seq.push(q);
                Query::Sequence(seq)
            }
            q0 => Query::Sequence(vec![q0, q]),
        };
        self
    }

    /// Adds a field access wildcard query to the last atom in the query.
    /// Represents a single-level wildcard field access and not a recursive
    /// descent match.
    ///
    /// # Examples
    ///
    /// Apply a field wildcard to the last atom in the query:
    /// ```
    /// use jsongrep::query::{QueryBuilder, Query};
    /// // Query: "foo.*"
    /// let query = QueryBuilder::new().field("foo").field_wildcard().build();
    ///
    /// assert!(
    ///     matches!(query, Query::Sequence(ref seq) if matches!(seq[0], Query::Field(_)) &&
    ///     matches!(seq[1], Query::FieldWildcard))
    /// );
    /// ```
    #[must_use]
    pub fn field_wildcard(mut self) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) => {
                seq.push(Query::FieldWildcard);
                Query::Sequence(seq)
            }
            q => Query::Sequence(vec![q, Query::FieldWildcard]),
        };
        self
    }

    /// Adds an array access wildcard query to the last atom in the query.
    ///
    /// # Examples
    ///
    /// Apply an array wildcard to the last atom in the query:
    /// ```
    /// use jsongrep::query::{QueryBuilder, Query};
    /// // Query: "foo[*]"
    /// let query = QueryBuilder::new().field("foo").array_wildcard().build();
    ///
    /// assert!(
    ///     matches!(query, Query::Sequence(ref seq) if matches!(seq[0], Query::Field(_)) &&
    ///     matches!(seq[1], Query::ArrayWildcard))
    /// );
    /// ```
    #[must_use]
    pub fn array_wildcard(mut self) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) => {
                seq.push(Query::ArrayWildcard);
                Query::Sequence(seq)
            }
            q => Query::Sequence(vec![q, Query::ArrayWildcard]),
        };
        self
    }

    /// Adds a regex query to the query builder.
    ///
    /// # Examples
    ///
    /// Apply a regex to the last atom in the query:
    /// ```
    /// use jsongrep::query::{QueryBuilder, Query};
    /// use regex::Regex;
    /// // Create a regex to match any string starting with "foo"
    /// let re = r"^foo";
    /// // Query: "foo.*"
    /// let query = QueryBuilder::new().field("foo").regex(re).build();
    ///
    /// assert!(
    ///     matches!(query,
    ///         Query::Sequence(ref seq) if matches!(seq[0], Query::Field(_)) &&
    ///         matches!(seq[1], Query::Regex(_)))
    /// );
    /// ```
    #[must_use]
    pub fn regex(mut self, re: &str) -> Self {
        self.query = match self.query {
            Query::Sequence(mut seq) => {
                seq.push(Query::Regex(re.to_string()));
                Query::Sequence(seq)
            }
            q => Query::Sequence(vec![q, Query::Regex(re.to_string())]),
        };
        self
    }

    /// Adds a disjunction (logical OR) of multiple queries to the current
    /// query.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new()
    ///    .disjunction(vec![
    ///    Query::field("foo"),
    ///    Query::field("bar"),
    ///    ]);
    /// ```
    #[must_use]
    pub fn disjunction(mut self, queries: Vec<Query>) -> Self {
        self.query = Query::Disjunction(queries);
        self
    }

    /// Adds a sequence of queries to the current query.
    ///
    /// # Examples
    ///
    /// Sequential field accesses:
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// // Create a sequence of queries: "foo.bar.baz"
    /// let query = QueryBuilder::new().sequence(vec![
    ///   Query::field("foo"),
    ///   Query::field("bar"),
    ///   Query::field("baz"),
    ///   ]).build();
    ///
    /// assert!(
    ///     matches!(query, Query::Sequence(ref seq) if seq.len() == 3 &&
    ///     matches!(seq[0], Query::Field(_)) &&
    ///     matches!(seq[1], Query::Field(_)) &&
    ///     matches!(seq[2], Query::Field(_)))
    ///     );
    /// ```
    ///
    #[must_use]
    pub fn sequence(mut self, queries: Vec<Query>) -> Self {
        self.query = Query::Sequence(queries);
        self
    }

    /// Return the built query as `Query`.
    ///
    /// # Examples
    ///
    /// Simple field access query: `foo`
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new().field("foo").build();
    ///
    /// assert!(
    ///     matches!(query, Query::Sequence(ref seq) if matches!(seq[..], [Query::Field(_)]))
    /// );
    /// ```
    ///
    /// Query containing mixed atoms and modifiers: "foo.bar\[3\]?.baz*"
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryBuilder};
    /// let query = QueryBuilder::new()
    ///                         .field("foo")
    ///                         .field("bar")
    ///                         .index(3)
    ///                         .optional()
    ///                         .field("baz")
    ///                         .kleene_star()
    ///                         .build();
    ///
    /// let expected = Query::Sequence(vec![
    ///    Query::field("foo"),
    ///    Query::field("bar"),
    ///    Query::Optional(Box::new(Query::Index(3))),
    ///    Query::KleeneStar(Box::new(Query::field("baz"))),
    ///    ]);
    ///
    /// assert_eq!(query, expected, "Got: {:?}, Expected: {:?}", query, expected);
    /// ```
    #[must_use]
    pub fn build(self) -> Query {
        self.query
    }
}

impl Default for QueryBuilder {
    fn default() -> Self {
        Self::new()
    }
}
