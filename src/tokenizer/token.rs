//! # JSON Token
//!
//! Defines possible tokens from parsing a JSON document byte sequence.
use std::fmt::Display;

/// Represents a token value from a JSON document.
#[derive(Debug, PartialEq, Clone, Eq)]
pub enum JToken {
    /* Delimiters */
    /// Opening curly brace
    LCurly,

    /// Closing curly brace
    RCurly,

    /// Opening square bracket
    LSquare,

    /// Closing square bracket
    RSquare,

    /// Colon character
    Colon,

    /// Comma character
    Comma,

    /* Values */
    /// Nil value
    Null,

    /// Boolean value
    Bool(bool),

    /// String value
    JString(usize, usize),

    /// Numeric value
    // NOTE: (usize, usize) to mark the [start..=end] byte indices in the input
    // byte slice
    JNumber(usize, usize),

    /* Reserved */
    /// Invalid character
    Illegal,

    /// End of file
    Eof,
}

impl Display for JToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JToken::LCurly => write!(f, "{{"),
            JToken::RCurly => write!(f, "}}"),
            JToken::LSquare => write!(f, "["),
            JToken::RSquare => write!(f, "]"),
            JToken::Colon => write!(f, ";"),
            JToken::Comma => write!(f, ","),
            JToken::Null => write!(f, "Null"),
            JToken::Bool(val) => write!(f, "{}", val),
            JToken::JString(start, end) => write!(f, "[{}..{}]", start, end),
            JToken::JNumber(start, end) => write!(f, "[{}..{}]", start, end),
            JToken::Illegal => write!(f, ""),
            JToken::Eof => write!(f, ""),
        }
    }
}
