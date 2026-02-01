/*!
# Query Parser

Parser for converting string queries into [`Query`] objects.

## Examples

This module exposes the public API method [`parse_query`] that can be used to
convert raw JSON DSL query strings into [`Query`] objects.

For example, we can construct the associated [`Query`] for the query string
"foo | bar\[2\].baz?" as so:

```rust
use jsongrep::query::{Query, parser};
let query = "foo | bar[2].baz?";
let parsed: Query = parser::parse_query("foo | bar[2].baz?").expect("Invalid query string");
assert_eq!("foo | bar[2].baz?", parsed.to_string());
```

## Errors

If the input query string is invalid, [`parse_query`] returns a [`QueryParseError`]
describing how the parsing failed:

```rust
use jsongrep::query::parser::{self, QueryParseError};

let result = parser::parse_query("foo[notanindex]");
assert!(matches!(result, Err(QueryParseError::UnexpectedToken(_))));
```

```rust
use jsongrep::query::parser::{self, QueryParseError};

let result = parser::parse_query("?");
assert!(matches!(result, Err(QueryParseError::UnexpectedToken(_))));
```


## See Also

- [`Query`]: The enum representing the query AST.
- [`QueryParseError`]: The error type for failed query parses.

*/

use pest::Parser;
use pest_derive::Parser;
use std::error::Error;
use std::fmt;

use crate::query::Query;

/// Parser for turning raw query strings into [`Query`] objects.
#[derive(Parser)]
#[grammar = "query/grammar/query.pest"]
pub struct QueryDSLParser;

/// Represents errors that can occur while parsing a JSON query.
#[derive(Debug, Clone)]
pub enum QueryParseError {
    /// Unexpected token encountered during parsing.
    UnexpectedToken(String),
    /// The input ended unexpectedly, indicating an incomplete query.
    UnexpectedEndOfInput,
}

impl Error for QueryParseError {}

impl fmt::Display for QueryParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedToken(token) => {
                write!(f, "Unexpected token: {token}")
            }
            Self::UnexpectedEndOfInput => {
                write!(f, "Unexpected end of input")
            }
        }
    }
}

/// Parse an input query string into a [`Query`]
///
/// # Panics
///
/// Panics if the input query string is invalid.
///
/// # Errors
///
/// Returns a [`QueryParseError`] describing how the parsing failed.
pub fn parse_query(input: &str) -> Result<Query, QueryParseError> {
    let mut pairs = QueryDSLParser::parse(Rule::query, input)
        .map_err(|e| QueryParseError::UnexpectedToken(e.to_string()))?;

    // Get and unwrap the `query` rule
    let query = pairs.next().expect("Empty query string");

    // Query rule contains disjunction
    let mut inner = query.into_inner();

    let constructed_query: Query;

    // If disjunction is present, parse it; else return empty sequence
    match inner.next() {
        Some(disjunction_pair) => {
            if matches!(disjunction_pair.as_rule(), Rule::EOI) {
                constructed_query = Query::Sequence(vec![]);
            } else {
                constructed_query = parse_disjunction(disjunction_pair)?;
            }
        }
        None => return Err(QueryParseError::UnexpectedEndOfInput),
    }

    #[cfg(test)]
    println!("Constructed query AST:\n{constructed_query:?}");

    Ok(constructed_query)
}

/// Parse a disjunction rule into a Query.
fn parse_disjunction(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::disjunction {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected disjunction rule, got {:?}",
            pair.as_rule()
        )));
    }

    let sequences: Vec<Query> = pair
        .into_inner()
        .map(parse_sequence)
        .collect::<Result<Vec<Query>, _>>()?;

    if sequences.len() == 1 {
        // Single `Query::Sequence(_)`
        Ok(sequences[0].clone())
    } else {
        // Convert to disjunction if more than one sequence subquery
        Ok(Query::Disjunction(sequences))
    }
}

/// Parse a sequence rule into a `Query::Sequence(_)`.
fn parse_sequence(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::sequence {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected sequence rule, got {:?}",
            pair.as_rule()
        )));
    }

    let mut steps: Vec<Query> = vec![];

    for step_result in pair.into_inner().map(parse_step) {
        let step = step_result?;
        steps.push(step);
    }

    // Convert steps to Sequence
    Ok(Query::Sequence(steps))
}

/// Parse a step rule into a [`Query`].
fn parse_step(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::step {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected step rule, got {:?}",
            pair.as_rule()
        )));
    }

    let mut inner = pair.into_inner();
    let mut queries: Vec<Query> = vec![];

    // Process the first pair (field or atom)
    let first_pair =
        inner.next().ok_or(QueryParseError::UnexpectedEndOfInput)?;
    match first_pair.as_rule() {
        Rule::field => {
            let field = parse_field(&first_pair)?;
            queries.push(field);
        }
        Rule::index => {
            queries.push(parse_index(first_pair)?);
        }
        Rule::range => {
            queries.push(parse_range(first_pair)?);
        }
        Rule::array_wildcard => {
            queries.push(Query::ArrayWildcard);
        }
        Rule::field_wildcard => {
            queries.push(Query::FieldWildcard);
        }
        Rule::regex => {
            let regex = parse_regex(&first_pair)?;
            queries.push(regex);
        }
        Rule::group => {
            let group_query = parse_group(first_pair)?;
            queries.push(group_query);
        }
        _ => {
            return Err(QueryParseError::UnexpectedToken(format!(
                "Unexpected start of step: {:?}",
                first_pair.as_rule()
            )));
        }
    }

    // Process array accesses (index, range, array_wildcard), if they exist
    // NOTE: `peek` here to avoid unintentionally consuming the subsequent
    // optional modifier
    while let Some(pair) = inner.peek() {
        if matches!(
            pair.as_rule(),
            Rule::index | Rule::range | Rule::array_wildcard
        ) {
            let pair = inner.next().unwrap();
            match pair.as_rule() {
                Rule::index => {
                    queries.push(parse_index(pair)?);
                }
                Rule::range => {
                    queries.push(parse_range(pair)?);
                }
                Rule::array_wildcard => {
                    queries.push(Query::ArrayWildcard);
                }
                _ => unreachable!(),
            }
        } else {
            break;
        }
    }

    // Process postfix modifier if present
    if let Some(modifier_pair) = inner.next() {
        if modifier_pair.as_rule() == Rule::modifier {
            let last_query = queries.pop().ok_or_else(|| {
                QueryParseError::UnexpectedToken(
                    "No query to apply modifier to".to_string(),
                )
            })?;
            let modified_query = match modifier_pair.as_str() {
                "*" => Query::KleeneStar(Box::new(last_query)),
                "?" => Query::Optional(Box::new(last_query)),
                _ => {
                    return Err(QueryParseError::UnexpectedToken(format!(
                        "Unknown modifier: {}",
                        modifier_pair.as_str()
                    )));
                }
            };
            queries.push(modified_query);
        } else {
            return Err(QueryParseError::UnexpectedToken(format!(
                "Expected modifier, got {:?}",
                modifier_pair.as_rule()
            )));
        }
    }

    // Return a single Query if only one, otherwise wrap in Sequence
    Ok(if queries.len() == 1 {
        queries.into_iter().next().unwrap()
    } else {
        Query::Sequence(queries)
    })
}

/// Parse a field rule into a [`Query::Field`]. This handles both cases of quoted and unquoted
/// field accesses, e.g. `\"\"foo\"\"` and `\"foo\"`
fn parse_field(
    pair: &pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::field {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected field rule, got {:?}",
            pair.as_rule()
        )));
    }

    Ok(Query::Field(pair.as_str().to_string()))
}

/// Parse a group rule into a [`Query::Disjunction`]
fn parse_group(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::group {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected group rule, got {:?}",
            pair.as_rule()
        )));
    }

    let mut inner = pair.into_inner();
    let disjunction_pair =
        inner.next().ok_or(QueryParseError::UnexpectedEndOfInput)?;
    parse_disjunction(disjunction_pair)
}

/// Parse an index rule into a [`Query::Index`]
fn parse_index(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::index {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected index rule, got {:?}",
            pair.as_rule()
        )));
    }
    let number_pair = pair
        .into_inner()
        .next()
        .ok_or(QueryParseError::UnexpectedEndOfInput)?;
    let idx = number_pair.as_str().parse::<usize>().map_err(|_| {
        QueryParseError::UnexpectedToken(number_pair.as_str().to_string())
    })?;
    Ok(Query::Index(idx))
}

/// Parse a range rule into a range (`Query::Range`, `Query::RangeFrom`, or
/// `Query::ArrayWildcard`).
fn parse_range(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::range {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected range rule, got {:?}",
            pair.as_rule()
        )));
    }

    let mut inner = pair.into_inner();

    // Starting integer
    let start = inner
        .next()
        .map(|p| {
            p.as_str().parse::<usize>().map_err(|_| {
                QueryParseError::UnexpectedToken(p.as_str().to_string())
            })
        })
        .transpose()?;

    // Colon (skip)
    let _ = inner.next();

    // Ending integer
    let end = inner
        .next()
        .map(|p| {
            p.as_str().parse::<usize>().map_err(|_| {
                QueryParseError::UnexpectedToken(p.as_str().to_string())
            })
        })
        .transpose()?;

    match (start, end) {
        (None, None) => Ok(Query::ArrayWildcard),
        (None, Some(e)) => Ok(Query::Range(0, e)),
        (Some(s), None) => Ok(Query::RangeFrom(s)),
        (Some(s), Some(e)) => Ok(Query::Range(s, e)),
    }
}

/// Parse a regex rule into a `Query::Regex`.
fn parse_regex(
    pair: &pest::iterators::Pair<Rule>,
) -> Result<Query, QueryParseError> {
    if pair.as_rule() != Rule::regex {
        return Err(QueryParseError::UnexpectedToken(format!(
            "Expected regex rule, got {:?}",
            pair.as_rule()
        )));
    }

    let regex_str = pair.as_str();
    if regex_str.len() < 2
        || !regex_str.starts_with('/')
        || !regex_str.ends_with('/')
    {
        return Err(QueryParseError::UnexpectedToken(regex_str.to_string()));
    }

    let pattern = &regex_str[1..regex_str.len() - 1];
    let unescaped_pattern = pattern.replace("\\/", "/");
    Ok(Query::Regex(unescaped_pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_field() {
        let query = "foo";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_field_and_number() {
        let query = "foo123[42]";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_single_regex() {
        let query = "/foo.bar/";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_disjunction() {
        let query = "foo | bar";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_kleene_star() {
        let query = "a*";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_optional() {
        let query = "b?";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_complex_query() {
        let query = "foo.bar[0]?.baz*";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_multiple_optional() {
        let query = "c*.c?.c?";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_simple_disjunction_group() {
        let query = "(foo | bar).baz";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_any_path_group() {
        let query = "(* | [*])*";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_any_path_group_in_query() {
        let query = "a.(* | [*])*.b?";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_nested_groups_trivial() {
        let query = "((foo))";
        let result = parse_query(query).unwrap();
        // NOTE: drops context of the nested parentheses and simplifies
        assert_eq!("foo", result.to_string());
    }

    #[test]
    fn parse_nested_groups() {
        let query = "((foo.bar)* | bar)";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_group_sequence() {
        let query = "(foo.bar.baz)?";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_invalid_number() {
        let result = parse_query("foo[abc]");
        assert!(
            matches!(result, Err(QueryParseError::UnexpectedToken(_))),
            "Actual result: {result:?}"
        );
    }

    #[test]
    fn parse_invalid_regex() {
        let result = parse_query("/unclosed");
        assert!(matches!(result, Err(QueryParseError::UnexpectedToken(_))));
    }

    #[test]
    fn parse_empty() {
        let query = "";
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn reserved_chars_in_double_quotes() {
        let query = r#"".|*?[]()/""#;
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn group_any_reserved_chars_in_double_quotes() {
        let query = r#"("." | "|" | "*" | "?" | "[" | "]" | "(" | ")" | "/")*"#;
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_unclosed_double_quotes() {
        let query = r#"""#;
        let result = parse_query(query);
        assert!(matches!(result, Err(QueryParseError::UnexpectedToken(_))));
    }

    #[test]
    fn parse_valid_key_with_spaces() {
        let query = r#""key space".foo"#;
        let result = parse_query(query).unwrap();
        assert_eq!(query, result.to_string());
    }

    #[test]
    fn parse_invalid_key_with_spaces() {
        let query = r"spaces not allowed without double quotes";
        let result = parse_query(query);
        assert!(matches!(result, Err(QueryParseError::UnexpectedToken(_))));
    }

    #[test]
    fn parse_invalid_key_with_reserved_chars() {
        let result = parse_query(r"][");
        assert!(matches!(result, Err(QueryParseError::UnexpectedToken(_))));
    }
}
