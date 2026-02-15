/*!
# DFA-based Query Engine

This module implements an automaton-based query engine for JSON queries.

An input query is converted first to an epsilon-free NFA, which is then
determinized via the [subset construction](https://en.wikipedia.org/wiki/Powerset_construction)
to yield a deterministic finite automaton that can be simulated directly against
an input JSON document AST.

## Related Work

For an example of production-ready implementation, the
[regex_automata](https://docs.rs/regex-automata/latest/regex_automata/)
crate provides NFA and DFA implementations for regular expressions using
Thompson's construction and other techniques such as a Pike VM.
*/
use core::cmp::Ordering;
use serde_json_borrow::Value;
use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    rc::Rc,
};

use crate::query::ast::Query;
use crate::query::common::{JSONPointer, PathType, TransitionLabel};
use crate::query::{QueryEngine, QueryNFA};

/// Represents a Deterministic Finite Automaton (DFA) for JSON queries. An
/// important thing to note is that the alphabet depends on the query.
#[non_exhaustive]
pub struct QueryDFA {
    /// The number of states in the DFA
    pub num_states: usize,

    /// The starting state of the DFA
    pub start_state: usize,

    /// Bitmap of accepting states
    pub is_accepting: Vec<bool>,

    /// Transition table: transitions\[state\]\[`symbol_index`\] -> Option<`next_state`>
    pub transitions: Vec<Vec<Option<usize>>>,

    /// Alphabet symbols for this DFA. The alphabet is necessarily finite and
    /// disjoint. The alphabet is determined from the input query.
    pub alphabet: Vec<TransitionLabel>,

    /// Mapping of field names to symbol indices in alphabet. Uses a reference
    /// counter for the field name to avoid expensive clones. Any encountered
    /// key while traversing that was not found from symbol extraction phase
    /// of the query AST is lumped together with all "other" keys, which
    /// corresponds to key ID `0`.
    pub key_to_key_id: HashMap<Rc<String>, usize>,

    /// Maps non-overlapping ranges of array indices to their corresponding
    /// symbol IDs in the DFA's alphabet.
    ///
    /// Each tuple `(range, id)` represents a range `[range.start, range.end)`
    /// associated with a symbol ID, where the symbol is either a
    /// `TransitionLabel::Range` or `TransitionLabel::RangeFrom`. Ranges are
    /// constructed in `DFABuilder::finalize_ranges` to be disjoint and cover
    /// the domain `[0, usize::MAX]`.
    ///
    /// Individual index accesses (e.g., `Query::Index`) are represented as
    /// single-element ranges `[i, i+1)`. Used by `get_index_symbol_id` to
    /// resolve array indices to symbol IDs during DFA traversal.
    pub range_to_range_id: Vec<(std::ops::Range<usize>, usize)>,
}

impl Display for QueryDFA {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "DFA States: {}", self.num_states)?;
        writeln!(f, "Start State: {}", self.start_state)?;
        writeln!(f, "Accepting States: {:?}", {
            self.is_accepting
                .iter()
                .enumerate()
                .filter_map(|(i, &b)| if b { Some(i) } else { None })
                .collect::<Vec<_>>()
        })?;
        writeln!(f, "Alphabet ({} symbols):", self.alphabet.len())?;
        for (i, sym) in self.alphabet.iter().enumerate() {
            writeln!(f, "\t{i}: {sym:?}")?;
        }
        writeln!(f, "Transitions:")?;
        for (st, row) in self.transitions.iter().enumerate() {
            writeln!(f, "\tstate {st}:")?;
            for (col, entry) in row.iter().enumerate() {
                match entry {
                    Some(dest) => writeln!(
                        f,
                        "\t\ton [{:?}] -> {}",
                        self.alphabet[col], dest
                    )?,
                    // No transition
                    None => writeln!(
                        f,
                        "\t\ton [{:?}] -> (dead)",
                        self.alphabet[col]
                    )?,
                }
            }
        }
        Ok(())
    }
}

impl QueryDFA {
    /// Constructs a new `QueryDFA` from a query
    #[must_use]
    pub fn from_query(query: &Query) -> Self {
        let mut builder = DFABuilder::new();
        builder.build_dfa(query)
    }

    /// Check if a given state is accepting/final
    #[must_use]
    pub fn is_accepting_state(&self, state: usize) -> bool {
        state < self.num_states && self.is_accepting[state]
    }

    /// Get the key id for a key
    #[must_use]
    pub fn get_field_symbol_id(&self, field: &str) -> usize {
        let field_rc = Rc::new(field.to_string());
        self.key_to_key_id.get(&field_rc).copied().unwrap_or(0) // default to "other"
    }

    /// Get the symbol index for an array index by performing a binary search
    /// over the sorted vector of all range entries.
    #[must_use]
    pub fn get_index_symbol_id(&self, index: usize) -> Option<usize> {
        // Perform a binary search to find the range that contains the index,
        // if any. If the index is not found, return the "other" symbol.
        self.range_to_range_id
            .binary_search_by(|(range, _)| {
                if index < range.start {
                    Ordering::Greater
                } else if index >= range.end {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .map_or(None, |i| Some(self.range_to_range_id[i].1))
    }

    /// Get the next state given current state and symbol
    #[must_use]
    pub fn transition(&self, state: usize, symbol_id: usize) -> Option<usize> {
        if state < self.num_states && symbol_id < self.alphabet.len() {
            self.transitions[state][symbol_id]
        } else {
            None
        }
    }

    /// Check whether a given index satisfies a range bounds.
    #[must_use]
    pub const fn index_in_range(
        &self,
        index: usize,
        start: usize,
        end: usize,
    ) -> bool {
        start <= index && index < end
    }
}

/// Builder for constructing a DFA from a given `Query` instance.
struct DFABuilder {
    /// The constructed finite alphabet of extracted DFA symbols from the query.
    alphabet: Vec<TransitionLabel>,

    /// Mapping of keys/fields to their index in the alphabet.
    key_to_key_id: HashMap<Rc<String>, usize>,

    /// Store the original ranges from the raw queries so that they can be
    /// deduplicated and made disjoint for deterministic transition edges in
    /// the constructed DFA. This includes direct indexing and range queries.
    collected_ranges: Vec<(usize, usize)>,

    /// Sorted array of tuples containing the disjoint ranges by start index and
    /// their respective index in the alphabet.
    range_to_range_id: Vec<(std::ops::Range<usize>, usize)>,
}

impl DFABuilder {
    fn new() -> Self {
        Self {
            // start with only the "other" symbol
            alphabet: vec![TransitionLabel::Other],
            key_to_key_id: HashMap::new(),
            collected_ranges: Vec::new(),
            range_to_range_id: Vec::new(),
        }
    }

    /// Recursively extract all symbols from a query to build the alphabet.
    fn extract_symbols(&mut self, query: &Query) {
        match query {
            Query::Field(name) => {
                // create a new key state if it does not exist
                let name_rc: Rc<String> = Rc::new(name.clone());
                self.key_to_key_id.entry(name_rc.clone()).or_insert_with(
                    || {
                        // NOTE: `or_insert_with` defers execution until it is
                        // verified that the default function returns empty,
                        // unlike `or_insert`, which would push a duplicate symbol
                        // onto the alphabet regardless of whether the key was
                        // already in the map
                        let symbol_id = self.alphabet.len();
                        self.alphabet
                            .push(TransitionLabel::Field(name_rc.clone()));
                        symbol_id
                    },
                );
            }
            Query::FieldWildcard => {
                // NOTE: Continue; don't record a symbol as a field wildcard
                // can match on either our collected named fields or the "Other"
                // symbol; only use `TransitionLabel::FieldWildcard` in the NFA
                // representation
            }
            Query::Index(idx) => {
                // Represent individual index as a single-element range
                // [idx: idx + 1)
                self.collected_ranges.push((*idx, *idx + 1));
            }
            Query::Range(s, e) => {
                self.collected_ranges
                    .push(((*s).unwrap_or(0), (*e).unwrap_or(usize::MAX)));
            }
            Query::RangeFrom(s) => self.collected_ranges.push((*s, usize::MAX)),
            Query::ArrayWildcard => {
                // Treat array wildcard as unbounded range query, as they are
                // equivalent
                self.collected_ranges.push((0, usize::MAX));
            }
            Query::Disjunction(queries) | Query::Sequence(queries) => {
                for q in queries {
                    self.extract_symbols(q);
                }
            }
            Query::KleeneStar(q) | Query::Optional(q) => {
                self.extract_symbols(q);
            }
            // Any unsupported operators
            Query::Regex(_) => unimplemented!(),
        }
    }

    /// Sorts and builds disjoint ranges from the collected ranges, updating the
    /// `alphabet` and `range_to_range_id` with the finalized ranges.
    fn finalize_ranges(&mut self) {
        // Collect all unique endpoints
        let mut points: Vec<usize> = Vec::new();
        for &(start, end) in &self.collected_ranges {
            if start < end {
                // Only consider valid ranges
                points.push(start);
                points.push(end);
            }
        }

        // Sort and de-duplicate endpoints
        points.sort_unstable();
        points.dedup();

        // Create disjoint ranges from consecutive endpoints
        let mut disjoint_ranges = Vec::new();

        // NOTE: use `saturating_sub` here to handle edge cases of empty or
        // single-value `points` array (only want to create ranges from each
        // pairwise consecutive elements)
        //
        // Here, if subtracting 1 produces a negative value, the value goes
        // to 0 (lower numeric bound) instead of overflowing.
        for i in 0..points.len().saturating_sub(1) {
            let start = points[i];
            let end = points[i + 1];
            // skip invalid ranges (end < start or empty case start == end)
            if start < end {
                disjoint_ranges.push(start..end);
            }
        }

        // Assign symbol IDs to the disjoint ranges
        for range in disjoint_ranges {
            let symbol_id = self.alphabet.len();
            self.alphabet.push(TransitionLabel::Range(range.start, range.end));
            self.range_to_range_id.push((range, symbol_id));
        }

        // Ensure that `range_to_range_id` is sorted for binary search on each
        // range's start value
        self.range_to_range_id.sort_by(|a, b| a.0.start.cmp(&b.0.start));
    }

    /// Use subset construction to convert the constructed epsilon-free NFA to a DFA,
    /// producing a `QueryDFA`. For each DFA state, we map it to a set of NFA
    /// states.
    #[allow(clippy::too_many_lines)]
    fn determinize_nfa(&mut self, nfa: &QueryNFA) -> QueryDFA {
        // Use a HashMap to map sets of currently reachable NFA states to DFA
        // state indices
        // curr_nfa_states_to_dfa_state[NFA states bitmap] -> DFA state index
        let mut nfa_states_to_dfa_state: HashMap<Vec<bool>, usize> =
            HashMap::new();

        // Queue to store DFA states to process (each is a set of NFA states as
        // a bitmap)
        let mut work_queue: VecDeque<Vec<bool>> = VecDeque::new();

        // List of DFA states, each represented as a set of NFA states
        // dfa_states[DFA state] -> set of NFA states
        let mut dfa_states: Vec<Vec<bool>> = Vec::new();

        // Transition table for the DFA
        let mut transitions: Vec<Vec<Option<usize>>> = Vec::new();

        // Accepting states bitmap for the DFA
        let mut is_accepting: Vec<bool> = Vec::new();

        // Initialize with the start state (NFA start state)
        let mut start_set = vec![false; nfa.num_states];
        start_set[nfa.start_state] = true; // start set is just `0`
        nfa_states_to_dfa_state.insert(start_set.clone(), 0);
        dfa_states.push(start_set.clone());
        work_queue.push_back(start_set);
        transitions.push(vec![None; self.alphabet.len()]);
        is_accepting.push(nfa.is_accepting[nfa.start_state]);

        // Process each DFA state
        while let Some(current_set) = work_queue.pop_front() {
            let current_dfa_state =
                *nfa_states_to_dfa_state.get(&current_set).unwrap();

            // For each symbol in the DFA alphabet
            for (symbol_id, dfa_symbol) in self.alphabet.iter().enumerate() {
                // Collect all NFA states reachable from the current set via this symbol
                let mut next_nfa_states = vec![false; nfa.num_states];

                // Check each NFA state in the current DFA state
                (0..nfa.num_states).for_each(|nfa_state| {
                    if current_set[nfa_state] {
                        // Check transitions from this NFA state
                        for &(label_idx, dest_state) in
                            &nfa.transitions[nfa_state]
                        {
                            let nfa_label = &nfa.pos_to_label[label_idx];

                            // Check if the NFA transition label matches or overlaps with the DFA symbol
                            match (nfa_label, dfa_symbol) {
                                // Field match
                                (
                                    TransitionLabel::Field(nfa_field),
                                    TransitionLabel::Field(dfa_field),
                                ) if nfa_field == dfa_field => {
                                    next_nfa_states[dest_state] = true;
                                }

                                // FieldWildcard match: can match on "Other" (keys
                                // not in query), or a seen Field
                                (
                                    TransitionLabel::FieldWildcard
                                    | TransitionLabel::Other,
                                    TransitionLabel::Other,
                                )
                                | (
                                    TransitionLabel::FieldWildcard,
                                    TransitionLabel::Field(_),
                                )
                                | (
                                    TransitionLabel::Range(0, usize::MAX),
                                    TransitionLabel::Range(_, _),
                                ) => {
                                    next_nfa_states[dest_state] = true;
                                }
                                // Range match: NFA range includes DFA range
                                (
                                    TransitionLabel::Range(nfa_start, nfa_end),
                                    TransitionLabel::Range(dfa_start, dfa_end),
                                ) if *nfa_start <= *dfa_start
                                    && *dfa_end <= *nfa_end =>
                                {
                                    next_nfa_states[dest_state] = true;
                                }

                                // RangeFrom match: NFA range starts before or at DFA range start
                                (
                                    TransitionLabel::RangeFrom(nfa_start),
                                    TransitionLabel::Range(dfa_start, _),
                                ) if *nfa_start <= *dfa_start => {
                                    next_nfa_states[dest_state] = true;
                                }

                                // ArrayWildcard match: matches any range
                                // Other symbol match
                                _ => {}
                            }
                        }
                    }
                });

                // If there are reachable states, create or find the
                // corresponding DFA state
                if next_nfa_states.iter().any(|&b| b) {
                    let next_dfa_state = if let Some(&dfa_state) =
                        nfa_states_to_dfa_state.get(&next_nfa_states)
                    {
                        dfa_state
                    } else {
                        // New DFA state
                        let new_dfa_state = dfa_states.len();
                        nfa_states_to_dfa_state
                            .insert(next_nfa_states.clone(), new_dfa_state);
                        dfa_states.push(next_nfa_states.clone());
                        work_queue.push_back(next_nfa_states.clone());
                        transitions.push(vec![None; self.alphabet.len()]);

                        // Accepting if any NFA state in the set is accepting
                        is_accepting.push(
                            next_nfa_states
                                .iter()
                                .enumerate()
                                .any(|(i, &b)| b && nfa.is_accepting[i]),
                        );
                        new_dfa_state
                    };

                    // Add transition
                    transitions[current_dfa_state][symbol_id] =
                        Some(next_dfa_state);
                }
            }
        }

        QueryDFA {
            num_states: dfa_states.len(),
            start_state: 0,
            is_accepting,
            transitions,
            // use the existing constructed finite alphabet from the DFABuilder
            alphabet: std::mem::take(&mut self.alphabet),
            key_to_key_id: std::mem::take(&mut self.key_to_key_id),
            range_to_range_id: std::mem::take(&mut self.range_to_range_id),
        }
    }

    /// Builds a deterministic finite automaton from a query.
    ///
    /// First, all the symbols from the query are extracted to obtain a
    /// finite alphabet. Then, potentially overlapping symbols like ranges are
    /// made disjoint. After this, the DFA is constructed first by turning the
    /// query into an epsilon-free NFA via the Glushkov construction, and then
    /// determinized to obtain the final DFA.
    fn build_dfa(&mut self, query: &Query) -> QueryDFA {
        // Handle empty query case: match root (identity)
        if let Query::Sequence(steps) = query
            && steps.is_empty()
        {
            return QueryDFA {
                num_states: 1,
                start_state: 0,
                is_accepting: vec![true],
                transitions: vec![],
                alphabet: vec![],
                key_to_key_id: HashMap::new(),
                range_to_range_id: vec![],
            };
        }

        // Extract symbols to obtain finite alphabet
        self.extract_symbols(query);

        // Make overlapping ranges disjoint
        self.finalize_ranges();

        // Create epsilon-free NFA via Glushkov construction
        let nfa = QueryNFA::from_query(query);

        // Determinize the NFA to achieve the DFA
        self.determinize_nfa(&nfa)
    }
}

/// A query engine that uses a DFA to find matches in a JSON document based on
/// the provided query.
pub struct DFAQueryEngine;

impl DFAQueryEngine {
    /// Performs a depth-first search over the JSON document AST, accumulating
    /// results as it traverses and finds final states.
    fn traverse_json<'a>(
        dfa: &QueryDFA,
        current_state: usize,
        path: &mut Vec<PathType>,
        value: &'a Value<'a>,
        results: &mut Vec<JSONPointer<'a>>,
    ) {
        // Check if current state is accepting
        if dfa.is_accepting_state(current_state) {
            results.push(JSONPointer {
                path: path.clone(), // clone path only for result
                value,
            });
        }

        match value {
            Value::Object(map) => {
                for (key, val) in map.as_vec() {
                    // Get symbol ID for this field
                    let symbol_id = dfa.get_field_symbol_id(key);

                    // Try to transition on this symbol
                    if let Some(next_state) =
                        dfa.transition(current_state, symbol_id)
                    {
                        // extend the current path using reference counter smart pointer
                        let key_rc: Rc<String> = Rc::new(key.to_string());
                        path.push(PathType::Field(key_rc));

                        // Recurse on the extended path
                        Self::traverse_json(
                            dfa, next_state, path, val, results,
                        );

                        // Backtrack by removing what we just added
                        path.pop();
                    }
                }
            }
            Value::Array(vals) => {
                for (idx, val) in vals.iter().enumerate() {
                    // Get symbol ID for this index
                    if let Some(symbol_id) = dfa.get_index_symbol_id(idx) {
                        // Try to transition on this symbol
                        if let Some(next_state) =
                            dfa.transition(current_state, symbol_id)
                        {
                            // Extend the current path
                            path.push(PathType::Index(idx));

                            // Recurse on the extended path
                            Self::traverse_json(
                                dfa, next_state, path, val, results,
                            );

                            // Backtrack
                            path.pop();
                        }
                    }
                    // If get_index_symbol_id returns None, skip this index (no valid transition)
                }
            }
            // Leaf JSON nodes - no further traversal needed
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Str(_) => {
            }
        }
    }
}

impl QueryEngine for DFAQueryEngine {
    fn find<'haystack>(
        &self,
        json: &'haystack Value,
        query: &'haystack Query,
    ) -> Vec<JSONPointer<'haystack>> {
        // Compile the query into a DFA
        let dfa = QueryDFA::from_query(query);

        #[allow(clippy::print_stdout)]
        #[cfg(test)]
        {
            println!("Constructed DFA for query: `{query}`\n{dfa}\n");
        };

        // Traverse the JSON document tree via depth-first search
        let mut results: Vec<JSONPointer> = Vec::new();
        let mut path = Vec::new();

        // Collect matches based on the DFA transitions and acceptance states
        Self::traverse_json(
            &dfa,
            dfa.start_state,
            &mut path,
            json,
            &mut results,
        );

        #[cfg(test)]
        println!("Found matches:\n{results:?}");

        results
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use anyhow::Context;
    use std::borrow::Cow;

    use super::*;
    use crate::query::QueryBuilder;
    use crate::query::common::JSONPointer;

    /// Creates the following simple JSON object for testing:
    /// ````
    /// {
    ///   "foo": {
    ///     "bar": "val"
    ///   },
    ///   "baz": [1, 2, 3, 4, 5],
    ///   "other": 42
    /// }
    /// ```
    fn create_simple_test_json() -> Value<'static> {
        static TEST_JSON: &str = r#"
            {
              "foo": {
                "bar": "val"
              },
              "baz": [1, 2, 3, 4, 5],
              "other": 42
            }
        "#;

        serde_json::from_str::<Value<'static>>(TEST_JSON)
            .expect("hardcoded test json")
    }

    /// Creates a nested test JSON object for unit tests.
    /// This JSON object contains:
    /// ```json
    /// {
    ///   "nested": {
    ///     "a": {
    ///       "b": {
    ///         "c": "target"
    ///       }
    ///     }
    ///   }
    /// }
    /// ```
    fn create_nested_test_json() -> Value<'static> {
        static TEST_JSON: &str = r#"
            {
              "nested": {
                "a": {
                  "b": {
                    "c": "target"
                  }
                }
              }
            }
        "#;
        serde_json::from_str::<Value<'static>>(TEST_JSON)
            .expect("hardcoded test json")
    }

    /// Creates a nested test JSON object with duplicate keys for unit tests.
    // ```json
    // {
    //   "c": {
    //     "c": {
    //        "c": "target"
    //     }
    //   }
    // }
    fn create_duplicate_key_nested_test_json() -> Value<'static> {
        static TEST_JSON: &str = r#"
            {
              "c": {
                "c": {
                   "c": "target"
                }
              }
            }
        "#;
        serde_json::from_str::<Value<'static>>(TEST_JSON)
            .expect("hardcoded test json")
    }

    /// Checks that a constructed `QueryDFA` does not contain any overlapping
    /// range transition symbols.
    fn check_no_range_overlaps(dfa: &QueryDFA) {
        let mut prev_end = 0;
        for (range, _) in &dfa.range_to_range_id {
            assert!(range.start >= prev_end, "Encounter overlapping range");
            prev_end = range.end;
        }
    }

    #[test]
    fn simple_field_sequence() {
        // Query: foo.bar
        let query = QueryBuilder::new().field("foo").field("bar").build();
        let json = create_simple_test_json();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        // Expect exactly one match at path ["foo","bar"], value = "val"
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].path,
            vec![
                PathType::Field(Rc::new("foo".to_string())),
                PathType::Field(Rc::new("bar".to_string())),
            ]
        );
        assert_eq!(matches[0].value, &Value::Str(Cow::Borrowed("val")));
    }

    #[test]
    fn dfa_construction() {
        let query = QueryBuilder::new().field("foo").field("bar").build();
        let dfa = QueryDFA::from_query(&query);

        #[cfg(test)]
        println!("Constructed DFA for `{query}`:\n{dfa}");

        // Should have 3 states: start, after "foo", after "bar" (accepting)
        assert_eq!(dfa.num_states, 3);
        assert_eq!(dfa.start_state, 0);
        assert!(dfa.is_accepting_state(2));
        assert!(!dfa.is_accepting_state(0));
        assert!(!dfa.is_accepting_state(1));

        // Should have "foo" and "bar" in the alphabet
        assert!(dfa.key_to_key_id.contains_key(&Rc::new("foo".to_string())));
        assert!(dfa.key_to_key_id.contains_key(&Rc::new("bar".to_string())));
    }

    #[test]
    fn simple_field_disjunction() {
        // Query: foo | baz
        let query_1 = QueryBuilder::new().field("foo").build();
        let query_2 = QueryBuilder::new().field("baz").build();
        let query =
            QueryBuilder::new().disjunction(vec![query_1, query_2]).build();
        let json = create_simple_test_json();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        // Should have 2 matches
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn simple_index_access() {
        // Query: baz[1]
        let query = QueryBuilder::new().field("baz").index(1).build();
        let json = create_simple_test_json();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Should have 1 match
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Number(2u64.into()));
    }

    #[test]
    fn nested_field_disjunction() {
        let mut json = create_nested_test_json();

        // add another field in "nested"
        if let Value::Object(ref mut root) = json
            && let Some(Value::Object(nested)) = root.get_mut("nested")
        {
            nested.insert("d", Value::Null);
        }

        // Query: nested.a.b.c | nested.d
        let query1 = QueryBuilder::new()
            .field("nested")
            .field("a")
            .field("b")
            .field("c")
            .build();
        let query2 = QueryBuilder::new().field("nested").field("d").build();
        let query =
            QueryBuilder::new().disjunction(vec![query1, query2]).build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        assert_eq!(matches.len(), 2);
        let values: Vec<&Value> = matches.iter().map(|m| m.value).collect();
        assert!(values.contains(&&Value::Null));
        assert!(values.contains(&&Value::Str(Cow::Borrowed("target"))));
    }

    #[test]
    fn simple_bounded_range() {
        let json = create_simple_test_json();
        // Query: `baz[1:4]`
        let query: Query = QueryBuilder::new().field("baz").range(1..4).build();

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Expect [2, 3, 4]
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].value, &Value::Number(2u64.into()));
        assert_eq!(matches[1].value, &Value::Number(3u64.into()));
        assert_eq!(matches[2].value, &Value::Number(4u64.into()));
    }

    #[test]
    fn simple_unbounded_range() {
        let json = create_simple_test_json();
        // Query: `baz[:]` => equivalent to `baz[*]`
        let query: Query = QueryBuilder::new().field("baz").range(..).build();

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Expect [1, 2, 3, 4, 5]
        assert_eq!(matches.len(), 5);
        assert_eq!(matches[0].value, &Value::Number(1u64.into()));
        assert_eq!(matches[1].value, &Value::Number(2u64.into()));
        assert_eq!(matches[2].value, &Value::Number(3u64.into()));
        assert_eq!(matches[3].value, &Value::Number(4u64.into()));
        assert_eq!(matches[4].value, &Value::Number(5u64.into()));
    }

    #[test]
    fn simple_unbounded_start() {
        let json = create_simple_test_json();
        // Query: `baz[:2]`
        let query: Query = QueryBuilder::new().field("baz").range(..2).build();

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Expect [0, 1]
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].value, &Value::Number(1u64.into()));
        assert_eq!(matches[1].value, &Value::Number(2u64.into()));
    }

    #[test]
    fn simple_unbounded_end() {
        let json = create_simple_test_json();
        // Query: `baz[2:]`
        let query: Query = QueryBuilder::new().field("baz").range(2..).build();

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Expect [3, 4, 5]
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].value, &Value::Number(3u64.into()));
        assert_eq!(matches[1].value, &Value::Number(4u64.into()));
        assert_eq!(matches[2].value, &Value::Number(5u64.into()));
    }

    #[test]
    fn simple_range_bounds_eq() {
        let json = create_simple_test_json();
        // Query: `baz[1:1]`
        let query: Query = QueryBuilder::new().field("baz").range(1..1).build();

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Expect empty result set
        assert!(matches.is_empty());
    }

    #[test]
    fn simple_array_wildcard() {
        let json = create_simple_test_json();

        // Query: `baz[*]`
        let query = QueryBuilder::new().field("baz").array_wildcard().build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        // Expected [1, 2, 3, 4, 5]
        assert_eq!(matches.len(), 5);
        assert_eq!(matches[0].value, &Value::Number(1u64.into()));
        assert_eq!(matches[1].value, &Value::Number(2u64.into()));
        assert_eq!(matches[2].value, &Value::Number(3u64.into()));
        assert_eq!(matches[3].value, &Value::Number(4u64.into()));
        assert_eq!(matches[4].value, &Value::Number(5u64.into()));
    }

    #[test]
    fn simple_optional_query() {
        let json = create_simple_test_json();
        // Query: `other?`
        let query = QueryBuilder::new().field("other").optional().build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        // Expected [(root object), 42]
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].value, &json); // the root object
        assert_eq!(matches[1].value, &Value::Number(42u64.into()));
    }

    #[test]
    fn overlapping_ranges() {
        let json = create_simple_test_json();
        // Query: `baz[0:3] | baz[1:]` = `baz[0:]`
        let q1 = QueryBuilder::new().field("baz").range(..3).build();
        let q2 = QueryBuilder::new().field("baz").range(1..).build();
        let query = QueryBuilder::new().disjunction(vec![q1, q2]).build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        // Only expected matches [1, 2, 3, 4, 5]
        assert_eq!(
            5,
            matches.len(),
            "Expected: 5 matches, Actual: {} matches [{:#?}]",
            matches.len(),
            matches
        );
    }

    #[test]
    fn single_query_overlap() {
        // Query: `foo[1:5].bar[2]`
        let query = QueryBuilder::new()
            .field("foo")
            .range(1..5)
            .field("baz")
            .index(2)
            .build();

        // Build DFA and inspect constructed ranges
        let dfa = QueryDFA::from_query(&query);
        println!("Constructed DFA: {dfa}");
        check_no_range_overlaps(&dfa);
    }

    #[test]
    fn single_arraywildcard_overlap() {
        // Query: `foo[*].bar[2]`
        let query = QueryBuilder::new()
            .field("foo")
            .array_wildcard()
            .field("baz")
            .index(2)
            .build();

        // Build DFA and inspect constructed ranges
        let dfa = QueryDFA::from_query(&query);
        println!("Constructed DFA: {dfa}");
        check_no_range_overlaps(&dfa);
    }

    #[test]
    fn single_startfrom_overlap() {
        // Query: `foo[1:].bar[2]`
        let query = QueryBuilder::new()
            .field("foo")
            .range(1..)
            .field("baz")
            .index(2)
            .build();

        // Build DFA and inspect constructed ranges
        let dfa = QueryDFA::from_query(&query);
        println!("Constructed DFA: {dfa}");
        check_no_range_overlaps(&dfa);
    }

    #[test]
    fn fieldwildcard_not_recursive() {
        let json = create_nested_test_json();
        // Query: `*.c`
        let query = QueryBuilder::new().field_wildcard().field("c").build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        assert!(matches.is_empty());
    }

    #[test]
    fn single_nested_fieldwildcard_access_query() {
        let json = create_nested_test_json();
        // Query: `nested.*.*.c`
        let query = QueryBuilder::new()
            .field("nested")
            .field_wildcard()
            .field_wildcard()
            .field("c")
            .build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn fieldwildcard_access_query() {
        let json = create_nested_test_json();
        // Query: `*.*.*.c`
        let query = QueryBuilder::new()
            .field_wildcard()
            .field_wildcard()
            .field_wildcard()
            .field("c")
            .build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn kleene_same_key() {
        static KLEENE_JSON: &str = r#"
            {
              "c": {
                "c": {
                   "c": "target"
                }
              }
            }
        "#;
        let json = serde_json::from_str::<Value<'_>>(KLEENE_JSON)
            .expect("hardcoded json");

        // Query: `c*`
        let query = QueryBuilder::new().field("c").kleene_star().build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());

        // Expected [(root object), top level c object, c1, c2]
        assert_eq!(matches.len(), 4);
        assert_eq!(matches[0].value, &json); // the root object
        assert_eq!(
            matches[1].path,
            vec![PathType::Field(Rc::from("c".to_string()))]
        );
        assert_eq!(
            matches[2].path,
            vec![
                PathType::Field(Rc::from("c".to_string())),
                PathType::Field(Rc::from("c".to_string()))
            ]
        );
        assert_eq!(
            matches[3].path,
            vec![
                PathType::Field(Rc::from("c".to_string())),
                PathType::Field(Rc::from("c".to_string())),
                PathType::Field(Rc::from("c".to_string()))
            ]
        );
    }

    #[test]
    fn fieldwildcard_nonunique_keys() {
        let json = create_duplicate_key_nested_test_json();
        // Query: `c.*.c`
        let query =
            QueryBuilder::new().field_wildcard().field("c").field("c").build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn multiple_optional_dfa() {
        let json = create_duplicate_key_nested_test_json();
        // Query: `c*.c?.c?`
        let query = QueryBuilder::new()
            .field("c")
            .kleene_star()
            .field("c")
            .optional()
            .field("c")
            .optional()
            .build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 4);
    }

    #[test]
    fn empty_query() {
        let json = create_simple_test_json();
        let query = QueryBuilder::new().build();
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 1); // identity
    }

    #[test]
    fn kleene_star_recursive_type() {
        let input = r#"
            {
              "type": {
                "type": "value1",
                "b": {
                  "type": "value2"
                }
              }
            }
            "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        // Query: `**.type`
        let query = QueryBuilder::new()
            .field_wildcard()
            .kleene_star()
            .field("type")
            .build();
        let result = DFAQueryEngine.find(&json, &query);

        assert_eq!(result.len(), 3);
    }

    #[test]
    fn get_all_array_elements_after_root_or_after_field() {
        let input = r#"
        {
          "root": [["1", "2"], ["3"]]
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();
        let query: Query = "**.[*]".parse().expect("failed to parse query");

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn two_field_wildcards() {
        let input = r#"
        {
          "root": {
              "foo": "bar"
          }
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();
        let query: Query = "*.*".parse().expect("failed to parse query");

        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn dfa_array_obj_no_fields() {
        let input = r#"
        [{
          "root": {
              "foo": "bar"
          }
        }]
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        #[cfg(test)]
        println!("Input Value:\n\t{json:?}\n");

        let query: Query = "*.*".parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(matches.is_empty());
    }

    #[test]
    fn dfa_recursive_array_indexing() {
        let input = r"[[1], [2, 3]]";
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        #[cfg(test)]
        println!("Input Value:\n\t{json:?}\n");

        let query: Query = "[*]*".parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());

        // expect 6 total: root obj, 2 top-level array elements, 3 inner-most
        //   array elements
        assert_eq!(
            matches.len(),
            6,
            "found {} matches:\n\t{:?}",
            matches.len(),
            matches
        );
    }

    #[test]
    fn dfa_recursive_array_indexing_any_level() {
        let input = r"[[1], [2, 3]]";
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        #[cfg(test)]
        println!("Input Value:\n\t{json:?}\n");

        let query: Query =
            "**.[*]*.[*]".parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());

        // expect 5 total: 2 top-level array elements, 3 inner-most array elements
        assert_eq!(matches.len(), 5);
    }

    #[test]
    fn dfa_simple_disjunction_group_query() {
        let input = r#"{"x": {"y": 5, "z": { "t": 2}}}"#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        #[cfg(test)]
        println!("Input Value:\n\t{json:?}\n");

        let query: Query =
            "x.(y | z.t)".parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);
        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn dfa_recursive_geojson_fmt_any_fields_then_arrays() {
        let input = r#"
        {
           "type":"FeatureCollection",
           "features":[
              {
                 "geometry":{
                    "coordinates":[
                       [
                          [
                             1,
                             2
                          ]
                       ]
                    ]
                 }
              }
           ]
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        #[cfg(test)]
        println!("Input Value:\n\t{json:?}\n");

        let query: Query =
            "**.[*]*.[*]".parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn dfa_recursive_geojson_fmt_any_level_group() {
        let input = r#"
        {
           "type":"FeatureCollection",
           "features":[
              {
                 "geometry":{
                    "coordinates":[
                       [
                          [
                             1,
                             2
                          ]
                       ]
                    ]
                 }
              }
           ]
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        #[cfg(test)]
        println!("Input Value:\n\t{json:?}\n");

        let query: Query =
            "(* | [*])*.[*]".parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 5);
    }

    // ==============================================================================
    // Quoted field matching tests — verify that quoted fields with special
    // characters actually match the corresponding JSON keys
    // ==============================================================================

    #[test]
    fn quoted_field_with_slash_matches_json_key() {
        let input = r#"{ "/activities": { "get": "list" } }"#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        let query: Query = r#""/activities""#
            .parse()
            .expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].path,
            vec![PathType::Field(Rc::new("/activities".to_string()))]
        );
    }

    #[test]
    fn quoted_field_sequence_openapi_style() {
        let input = r#"
        {
          "paths": {
            "/activities": { "get": "list" },
            "/users": { "get": "list_users" }
          }
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        let query: Query = r#"paths."/activities""#
            .parse()
            .expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].path,
            vec![
                PathType::Field(Rc::new("paths".to_string())),
                PathType::Field(Rc::new("/activities".to_string())),
            ]
        );
    }

    #[test]
    fn quoted_field_recursive_descent() {
        let input = r#"
        {
          "paths": {
            "/activities": { "get": "list" },
            "/activities/statistics": { "get": "stats" }
          }
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        // Use ** to recursively find the key
        let query: Query = r#"**."/activities""#
            .parse()
            .expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].path,
            vec![
                PathType::Field(Rc::new("paths".to_string())),
                PathType::Field(Rc::new("/activities".to_string())),
            ]
        );
    }

    #[test]
    fn quoted_field_with_dot_matches_json_key() {
        let input = r#"{ "a.b": 42, "a": { "b": 99 } }"#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        // Quoted "a.b" should match the literal key "a.b", not the path a → b
        let query: Query =
            r#""a.b""#.parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Number(42u64.into()));
    }

    #[test]
    fn quoted_field_with_spaces_matches_json_key() {
        let input = r#"{ "my key": "value" }"#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        let query: Query =
            r#""my key""#.parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].value,
            &Value::Str(Cow::Borrowed("value"))
        );
    }

    #[test]
    fn quoted_field_disjunction() {
        let input = r#"
        {
          "paths": {
            "/activities": { "get": "list" },
            "/users": { "get": "list_users" }
          }
        }
        "#;
        let json = serde_json::from_str(input)
            .with_context(|| "Failed to parse JSON")
            .unwrap();

        let query: Query = r#"paths.("/activities" | "/users")"#
            .parse()
            .expect("failed to parse query");
        let matches: Vec<JSONPointer> = DFAQueryEngine.find(&json, &query);

        assert_eq!(matches.len(), 2);
    }
}
