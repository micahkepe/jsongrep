//! Property-based tests for query parsing, display, and DFA construction.
//!
//! Unlike the example-based suites, these properties guard the whole input
//! space: proptest generates hundreds of random inputs per run and shrinks
//! any failure to a minimal counterexample. The display round-trip property
//! is exactly the invariant behind the `foo*.[0]` and `(foo?)*` display
//! fixes; these tests keep that whole class of bug from reappearing.
//!
//! Known limitation: the convergence property checks syntactic stability,
//! not meaning preservation - a formatter that stably printed the *wrong*
//! query would pass it. Semantic equivalence needs a DFA-differential
//! oracle (future work).

use jsongrep::query::{Query, QueryDFA, parser::parse_query};
use proptest::prelude::*;

/// Maximum recursive nesting depth [`arb_query`] generates.
const MAX_GEN_DEPTH: u32 = 4;

/// Rounds allowed for display/reparse to reach a fixpoint. Each round
/// strips at most one layer of redundant structure (e.g. "(a.([0]))*"
/// becomes "(a.[0])*" becomes "(a[0])*"), so a bound comfortably above
/// [`MAX_GEN_DEPTH`] cannot mask a genuine failure to converge.
const MAX_CONVERGENCE_ROUNDS: usize = 2 * MAX_GEN_DEPTH as usize + 2;

/// Characters the query DSL actually uses, plus a few plain field
/// characters. Biasing random strings toward this alphabet reaches far
/// deeper into the parser than uniformly random Unicode would.
fn dsl_string() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::sample::select(vec![
            'a',
            'b',
            'c',
            '0',
            '1',
            '9',
            '.',
            '|',
            '*',
            '?',
            '[',
            ']',
            '(',
            ')',
            ':',
            '"',
            '/',
            ' ',
            '_',
            char::from(92), // backslash
        ]),
        0..30,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

/// `true` if the query displays as the empty string (mirrors the private
/// `displays_as_empty` in `src/query/ast.rs`). Used to keep generated
/// disjunction members non-empty: the DSL has no syntax for an empty
/// disjunct (`"a | "` does not parse), and unlike sequence concatenation
/// the empty query is not the identity of union, so display cannot
/// soundly elide it either.
fn displays_empty(query: &Query) -> bool {
    match query {
        Query::Sequence(queries) | Query::Disjunction(queries) => {
            queries.iter().all(displays_empty)
        }
        Query::Optional(inner) | Query::KleeneStar(inner) => {
            displays_empty(inner)
        }
        _ => false,
    }
}

/// Field names mixing plain identifiers, fully arbitrary strings, and
/// targeted cases that exercise quoting and escaping in display (dots,
/// spaces, quotes, backslashes, reserved characters, empty, non-ASCII).
fn arb_field_name() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[a-z]{1,6}",
        2 => any::<String>(),
        1 => Just(String::new()),
        1 => Just("a.b".to_string()),
        1 => Just("a b".to_string()),
        1 => Just("*".to_string()),
        1 => Just(['a', '"', 'b'].iter().collect()),
        1 => Just(['a', char::from(92), 'b'].iter().collect()),
        1 => Just("😀".to_string()),
    ]
}

/// Arbitrary query ASTs, excluding only:
///
/// - `Regex`: display does not re-escape `/` in the pattern, and the DFA
///   cannot execute them (see [`contains_regex`]).
/// - empty-displaying `Disjunction` members (see [`displays_empty`]).
///
/// Everything else is fair game, including shapes the parser itself never
/// produces (nested and empty sequences, singleton disjunctions,
/// `Range(None, _)`): display must still render them as parseable,
/// meaning-preserving syntax.
fn arb_query() -> impl Strategy<Value = Query> {
    let leaf = prop_oneof![
        arb_field_name().prop_map(Query::Field),
        (0usize..100).prop_map(Query::Index),
        (proptest::option::of(0usize..50), proptest::option::of(0usize..50))
            .prop_map(|(start, end)| Query::Range(start, end)),
        (0usize..50).prop_map(Query::RangeFrom),
        Just(Query::FieldWildcard),
        Just(Query::ArrayWildcard),
    ];
    leaf.prop_recursive(MAX_GEN_DEPTH, 24, 3, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..=3)
                .prop_map(Query::Sequence),
            proptest::collection::vec(
                inner.clone().prop_filter(
                    "disjunction members must not display empty",
                    |q| !displays_empty(q),
                ),
                1..=3,
            )
            .prop_map(Query::Disjunction),
            inner.clone().prop_map(|q| Query::Optional(Box::new(q))),
            inner.prop_map(|q| Query::KleeneStar(Box::new(q))),
        ]
    })
}

/// Query strings for the DFA-build property: displayed [`arb_query`] ASTs
/// (guaranteed parseable, so the build path is exercised on every such
/// case and the property cannot silently become vacuous) mixed with raw
/// [`dsl_string`]s (mostly unparseable, but the ones that parse are
/// shapes the AST generator would never produce).
fn dfa_query_string() -> impl Strategy<Value = String> {
    prop_oneof![arb_query().prop_map(|q| q.to_string()), dsl_string()]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Parsing must never panic, whatever the input.
    #[test]
    fn parse_never_panics_on_arbitrary_strings(s in any::<String>()) {
        let _ = parse_query(&s);
    }

    /// Parsing must never panic on strings drawn from the DSL's own
    /// alphabet, which reach much deeper into the grammar.
    #[test]
    fn parse_never_panics_on_dsl_strings(s in dsl_string()) {
        let _ = parse_query(&s);
    }

    /// Every query that parses must compile to a DFA without panicking.
    /// The two former exclusions here (regex queries, indices at exactly
    /// `usize::MAX`) are gone: the parser now rejects regex syntax with
    /// `UnsupportedFeature`, and `usize::MAX` indices are well-defined, so
    /// whatever `parse_query` accepts must build.
    #[test]
    fn dfa_build_never_panics_on_parsed_queries(s in dfa_query_string()) {
        if let Ok(query) = parse_query(&s) {
            let _ = QueryDFA::from_query(&query);
        }
    }

    /// Displaying any AST yields parseable syntax at every round, and
    /// repeated display/reparse reaches a fixpoint. Strict display
    /// idempotence does not hold: the parser normalizes one layer of
    /// redundant structure per round trip, so deeply nested hand-built
    /// ASTs legitimately take several rounds - but every intermediate
    /// form must parse, and the process must stabilize rather than
    /// oscillate.
    #[test]
    fn display_reparse_converges(q in arb_query()) {
        let mut current = q.to_string();
        let mut converged = false;
        for _ in 0..MAX_CONVERGENCE_ROUNDS {
            let parsed = parse_query(&current)
                .expect("every displayed form must be parseable");
            let next = parsed.to_string();
            if next == current {
                converged = true;
                break;
            }
            current = next;
        }
        prop_assert!(
            converged,
            "display did not stabilize within {MAX_CONVERGENCE_ROUNDS} \
             reparses; last form: {current}"
        );
    }
}
