/*!
# Value Predicates

A predicate is an optional trailing comparison in a query string that
filters *which matched values survive*, e.g.:

```text
users[*].age >= 30
**.name = "Alice"
config.*.enabled = true
```

Predicates are deliberately filter-only: they select paths by comparing the
matched leaf value against a scalar literal, and never construct or
transform values (that is `jq`'s job). They are evaluated as a
post-processing step over query results, so the NFA/DFA path machinery is
untouched.
*/
use std::fmt::Display;

use serde_json_borrow::Value;

/// Comparison operator in a value predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `=` or `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

impl Display for CmpOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        };
        write!(f, "{s}")
    }
}

/// A numeric predicate literal, preserving integer precision (comparing
/// through `f64` alone would conflate integers above 2^53).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumLiteral {
    /// A non-negative integer literal.
    UInt(u64),
    /// A negative integer literal.
    NegInt(i64),
    /// A floating-point literal (always finite).
    Float(f64),
}

impl NumLiteral {
    /// The literal as an `f64` (possibly lossy for large integers; only
    /// used when the compared JSON value is itself a float).
    #[must_use]
    pub const fn as_f64(self) -> f64 {
        match self {
            #[expect(
                clippy::cast_precision_loss,
                reason = "float comparison path is inherently f64-precision"
            )]
            Self::UInt(n) => n as f64,
            #[expect(
                clippy::cast_precision_loss,
                reason = "float comparison path is inherently f64-precision"
            )]
            Self::NegInt(n) => n as f64,
            Self::Float(n) => n,
        }
    }
}

impl Display for NumLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UInt(n) => write!(f, "{n}"),
            Self::NegInt(n) => write!(f, "{n}"),
            Self::Float(n) => write!(f, "{n}"),
        }
    }
}

/// Scalar literal on the right-hand side of a predicate.
#[derive(Debug, Clone, PartialEq)]
pub enum PredicateValue {
    /// A string literal, e.g. `"Alice"`.
    Str(String),
    /// A numeric literal, e.g. `30`, `-1.5`, `2e3`.
    Num(NumLiteral),
    /// A boolean literal.
    Bool(bool),
    /// The null literal.
    Null,
}

impl Display for PredicateValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Str(s) => {
                let quoted = serde_json::to_string(s)
                    .expect("string serialization cannot fail");
                write!(f, "{quoted}")
            }
            Self::Num(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Null => write!(f, "null"),
        }
    }
}

/// A leaf value predicate: `<op> <literal>`.
///
/// See [`Predicate::matches`] for the comparison semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct Predicate {
    /// The comparison operator.
    pub op: CmpOp,
    /// The literal to compare matched values against.
    pub value: PredicateValue,
}

impl Display for Predicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.op, self.value)
    }
}

impl Predicate {
    /// Whether a matched JSON value satisfies this predicate.
    ///
    /// Semantics (deliberately simple, jq-like):
    ///
    /// - `=`/`==` compares within the same scalar type: string to string,
    ///   number to number (numerically, so `30` matches `30.0`), boolean to
    ///   boolean, `null` to null. A type mismatch - including objects and
    ///   arrays, which are never equal to a scalar literal - is `false`.
    /// - `!=` is the exact negation of `=` (so an object matched against
    ///   `!= "x"` is `true`: it is indeed not equal).
    /// - `<`, `<=`, `>`, `>=` compare numbers numerically and strings
    ///   lexicographically (byte order, like Rust's `str` ordering). Any
    ///   other combination is `false`.
    #[must_use]
    pub fn matches(&self, value: &Value<'_>) -> bool {
        match self.op {
            CmpOp::Eq => Self::eq_value(&self.value, value),
            CmpOp::Ne => !Self::eq_value(&self.value, value),
            CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => {
                self.ord_value(value)
            }
        }
    }

    /// Type-matched equality between the literal and a JSON value.
    fn eq_value(literal: &PredicateValue, value: &Value<'_>) -> bool {
        match (literal, value) {
            (PredicateValue::Str(a), Value::Str(b)) => a == b.as_ref(),
            (PredicateValue::Num(a), Value::Number(n)) => {
                cmp_number(*a, n) == Some(std::cmp::Ordering::Equal)
            }
            (PredicateValue::Bool(a), Value::Bool(b)) => a == b,
            (PredicateValue::Null, Value::Null) => true,
            _ => false,
        }
    }

    /// Ordered comparison: `matched_value <op> literal`.
    fn ord_value(&self, value: &Value<'_>) -> bool {
        let ordering = match (&self.value, value) {
            (PredicateValue::Num(a), Value::Number(n)) => cmp_number(*a, n),
            (PredicateValue::Str(a), Value::Str(b)) => {
                Some(b.as_ref().cmp(a.as_str()))
            }
            _ => None,
        };

        ordering.is_some_and(|ord| match self.op {
            CmpOp::Lt => ord.is_lt(),
            CmpOp::Le => ord.is_le(),
            CmpOp::Gt => ord.is_gt(),
            CmpOp::Ge => ord.is_ge(),
            CmpOp::Eq | CmpOp::Ne => unreachable!("handled in matches()"),
        })
    }
}

/// Compare a matched JSON number against a numeric literal, returning the
/// ordering of `value` relative to `literal`.
///
/// Integer-vs-integer comparisons are exact (no f64 rounding, so integers
/// above 2^53 compare correctly); comparisons where either side is a float
/// use `f64` semantics, which is the precision the JSON value itself has.
fn cmp_number(
    literal: NumLiteral,
    value: &serde_json_borrow::Number,
) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;

    match literal {
        NumLiteral::UInt(a) => {
            // Exact: non-negative integer value vs non-negative literal.
            if let Some(b) = value.as_u64() {
                return Some(b.cmp(&a));
            }
            // A negative integer value is always below a non-negative
            // literal.
            if !value.is_f64() {
                return Some(Ordering::Less);
            }
        }
        NumLiteral::NegInt(a) => {
            // A non-negative integer value is always above a negative
            // literal.
            if value.as_u64().is_some() {
                return Some(Ordering::Greater);
            }
            // Exact: negative integer value vs negative literal
            // (as_i64 is None for floats, which fall through below).
            if let Some(b) = value.as_i64() {
                return Some(b.cmp(&a));
            }
        }
        NumLiteral::Float(_) => {}
    }

    // At least one side is a float: compare with f64 semantics.
    value.as_f64()?.partial_cmp(&literal.as_f64())
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Unit testing.")]
mod tests {
    use super::*;

    fn val(json: &str) -> Value<'_> {
        serde_json::from_str(json).unwrap()
    }

    fn pred(op: CmpOp, value: PredicateValue) -> Predicate {
        Predicate { op, value }
    }

    #[test]
    fn eq_string() {
        let p = pred(CmpOp::Eq, PredicateValue::Str("Alice".into()));
        assert!(p.matches(&val(r#""Alice""#)));
        assert!(!p.matches(&val(r#""Bob""#)));
        assert!(!p.matches(&val("42")));
        assert!(!p.matches(&val(r#"{"name": "Alice"}"#)));
    }

    #[test]
    fn eq_number_int_and_float_forms() {
        let p = pred(CmpOp::Eq, PredicateValue::Num(NumLiteral::UInt(30)));
        assert!(p.matches(&val("30")));
        assert!(p.matches(&val("30.0")));
        assert!(!p.matches(&val("31")));
        assert!(!p.matches(&val(r#""30""#)), "no cross-type coercion");
    }

    #[test]
    fn eq_bool_and_null() {
        assert!(
            pred(CmpOp::Eq, PredicateValue::Bool(true)).matches(&val("true"))
        );
        assert!(
            !pred(CmpOp::Eq, PredicateValue::Bool(true)).matches(&val("false"))
        );
        assert!(pred(CmpOp::Eq, PredicateValue::Null).matches(&val("null")));
        assert!(!pred(CmpOp::Eq, PredicateValue::Null).matches(&val("0")));
    }

    #[test]
    fn ne_is_negation_including_containers() {
        let p = pred(CmpOp::Ne, PredicateValue::Str("x".into()));
        assert!(p.matches(&val(r#""y""#)));
        assert!(!p.matches(&val(r#""x""#)));
        // An object is not equal to a scalar literal, so != holds.
        assert!(p.matches(&val(r#"{"a": 1}"#)));
        assert!(p.matches(&val("[1, 2]")));
    }

    #[test]
    fn ordered_numbers() {
        let gt30 = pred(CmpOp::Gt, PredicateValue::Num(NumLiteral::UInt(30)));
        assert!(gt30.matches(&val("31")));
        assert!(!gt30.matches(&val("30")));
        assert!(
            pred(CmpOp::Ge, PredicateValue::Num(NumLiteral::UInt(30)))
                .matches(&val("30"))
        );
        assert!(
            pred(CmpOp::Lt, PredicateValue::Num(NumLiteral::UInt(0)))
                .matches(&val("-1.5"))
        );
        assert!(
            pred(CmpOp::Le, PredicateValue::Num(NumLiteral::Float(2000.0)))
                .matches(&val("2e3"))
        );
    }

    #[test]
    fn large_integer_equality_is_exact() {
        // 2^53 + 1 and 2^53 collapse to the same f64; integer comparison
        // must stay exact.
        let p = pred(
            CmpOp::Eq,
            PredicateValue::Num(NumLiteral::UInt(9_007_199_254_740_993)),
        );
        assert!(p.matches(&val("9007199254740993")));
        assert!(!p.matches(&val("9007199254740992")));

        let gt = pred(
            CmpOp::Gt,
            PredicateValue::Num(NumLiteral::UInt(9_007_199_254_740_992)),
        );
        assert!(gt.matches(&val("9007199254740993")));
        assert!(!gt.matches(&val("9007199254740992")));
    }

    #[test]
    fn mixed_sign_integer_comparisons() {
        let ge_neg =
            pred(CmpOp::Ge, PredicateValue::Num(NumLiteral::NegInt(-5)));
        assert!(ge_neg.matches(&val("0")));
        assert!(ge_neg.matches(&val("-5")));
        assert!(!ge_neg.matches(&val("-6")));

        let lt_pos = pred(CmpOp::Lt, PredicateValue::Num(NumLiteral::UInt(3)));
        assert!(lt_pos.matches(&val("-100")));
        assert!(!lt_pos.matches(&val("3")));
    }

    #[test]
    fn int_literal_matches_float_value() {
        let p = pred(CmpOp::Eq, PredicateValue::Num(NumLiteral::UInt(30)));
        assert!(p.matches(&val("30.0")));
        assert!(!p.matches(&val("30.5")));
    }

    #[test]
    fn ordered_strings_lexicographic() {
        let p = pred(CmpOp::Lt, PredicateValue::Str("m".into()));
        assert!(p.matches(&val(r#""apple""#)));
        assert!(!p.matches(&val(r#""zebra""#)));
    }

    #[test]
    fn ordered_type_mismatch_is_false() {
        let p = pred(CmpOp::Gt, PredicateValue::Num(NumLiteral::UInt(0)));
        assert!(!p.matches(&val(r#""1""#)));
        assert!(!p.matches(&val("true")));
        assert!(!p.matches(&val("null")));
        assert!(!p.matches(&val("[1]")));
    }

    #[test]
    fn display_round_trip_forms() {
        assert_eq!(
            pred(CmpOp::Ge, PredicateValue::Num(NumLiteral::UInt(30)))
                .to_string(),
            ">= 30"
        );
        assert_eq!(
            pred(CmpOp::Eq, PredicateValue::Str("a \"b\"".into())).to_string(),
            r#"= "a \"b\"""#
        );
        assert_eq!(
            pred(CmpOp::Ne, PredicateValue::Null).to_string(),
            "!= null"
        );
    }
}
