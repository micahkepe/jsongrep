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

use crate::query::{
    QueryNFA, QueryParseError,
    ast::Query,
    common::{JSONPointer, PathType, TransitionLabel},
};

/// Error returned when DFA determinization exceeds a configured state
/// budget.
///
/// Subset construction is worst-case exponential in the query size (e.g.
/// `(a|b)*.a.(a|b).(a|b)...` doubles the state count with every trailing
/// step), so a short adversarial query string can otherwise consume
/// unbounded time and memory. Services compiling untrusted queries should
/// use [`QueryDFA::from_query_bounded`] and treat this error as "query too
/// complex".
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StateLimitExceeded {
    /// The state budget that would have been exceeded.
    pub limit: usize,
}

impl std::error::Error for StateLimitExceeded {}

impl Display for StateLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "query is too complex: DFA construction would exceed {} states; \
             simplify repetitions/alternations in the query",
            self.limit
        )
    }
}

/// Error returned by the bounded string-to-DFA constructors: either the
/// query string failed to parse, or determinization hit the state budget.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum QueryCompileError {
    /// The query string failed to parse.
    Parse(QueryParseError),
    /// Determinization would exceed the configured state budget.
    StateLimit(StateLimitExceeded),
}

impl std::error::Error for QueryCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            Self::StateLimit(e) => Some(e),
        }
    }
}

impl Display for QueryCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => e.fmt(f),
            Self::StateLimit(e) => e.fmt(f),
        }
    }
}

impl From<QueryParseError> for QueryCompileError {
    fn from(e: QueryParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<StateLimitExceeded> for QueryCompileError {
    fn from(e: StateLimitExceeded) -> Self {
        Self::StateLimit(e)
    }
}

/// Represents a Deterministic Finite Automaton (DFA) for JSON queries. An
/// important thing to note is that the alphabet depends on the query.
#[non_exhaustive]
#[derive(Debug)]
pub struct QueryDFA {
    /// The number of states in the DFA.
    pub num_states: usize,

    /// The starting state of the DFA.
    pub start_state: usize,

    /// Bitmap of accepting states.
    pub is_accepting: Vec<bool>,

    /// Transition table: transitions\[state\]\[`symbol_index`\] -> Option<`next_state`>.
    pub transitions: Vec<Vec<Option<usize>>>,

    /// The finite alphabet gathered from the input query.
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

    /// Whether fields are case-sensitive.
    pub case_insensitive: bool,
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
    /// Constructs a new [`QueryDFA`] from a constructed [`Query`].
    ///
    /// # Panics
    ///
    /// Panics if the query contains [`Query::Regex`], which the engine does
    /// not implement yet. Queries obtained from the string parser can never
    /// contain it (the parser rejects `/regex/` syntax with
    /// [`QueryParseError::UnsupportedFeature`]); only hand-constructed ASTs
    /// can reach this panic.
    ///
    /// Construction is unbounded: an adversarial query can require an
    /// exponential number of DFA states. Use
    /// [`QueryDFA::from_query_bounded`] when compiling untrusted input.
    #[must_use]
    pub fn from_query(query: &Query) -> Self {
        // A usize::MAX budget cannot be exceeded.
        Self::build_from_query(query, false, usize::MAX)
            .unwrap_or_else(|_| unreachable!("unbounded DFA build"))
    }

    /// Constructs a new case-insensitive [`QueryDFA`] from a constructed
    /// [`Query`]. Field names in the query and in JSON keys are compared
    /// after lowercasing.
    ///
    /// # Panics
    ///
    /// Panics if the query contains [`Query::Regex`]; see
    /// [`QueryDFA::from_query`].
    ///
    /// Construction is unbounded; see [`QueryDFA::from_query`] and
    /// [`QueryDFA::from_query_bounded_ignore_case`].
    #[must_use]
    pub fn from_query_ignore_case(query: &Query) -> Self {
        // A usize::MAX budget cannot be exceeded.
        Self::build_from_query(query, true, usize::MAX)
            .unwrap_or_else(|_| unreachable!("unbounded DFA build"))
    }

    /// Attempt to construct a new [`QueryDFA`] from the query string.
    ///
    /// # Errors
    ///
    /// Returns an error in the case of an invalid query string.
    pub fn from_query_str(query: &str) -> Result<Self, QueryParseError> {
        let query: Query = query.parse()?;
        Ok(Self::from_query(&query))
    }

    /// Attempt to construct a new case-insensitive [`QueryDFA`] from the
    /// query string.
    ///
    /// # Errors
    ///
    /// Returns an error in the case of an invalid query string.
    pub fn from_query_str_ignore_case(
        query: &str,
    ) -> Result<Self, QueryParseError> {
        let query: Query = query.parse()?;
        Ok(Self::from_query_ignore_case(&query))
    }

    /// Constructs a new [`QueryDFA`] from a [`Query`], failing if
    /// determinization would exceed `max_states` DFA states.
    ///
    /// This is the constructor to use when queries come from untrusted
    /// input (network services, playgrounds): subset construction is
    /// worst-case exponential, and the budget turns a potential
    /// memory/CPU blowup into a clean error.
    ///
    /// The budget bounds the number of DFA states; per-state cost (and the
    /// NFA construction that precedes determinization) remains polynomial
    /// in the query length, so callers accepting untrusted queries should
    /// bound the query string length as well.
    ///
    /// # Errors
    ///
    /// Returns [`StateLimitExceeded`] when the budget is exhausted (a
    /// budget of `n` permits at most `n` states; a budget of `0` always
    /// fails).
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::query::{Query, QueryDFA};
    ///
    /// let query: Query = "users[*].name".parse().unwrap();
    /// let dfa = QueryDFA::from_query_bounded(&query, 10_000).unwrap();
    /// assert!(dfa.num_states <= 10_000);
    /// ```
    pub fn from_query_bounded(
        query: &Query,
        max_states: usize,
    ) -> Result<Self, StateLimitExceeded> {
        Self::build_from_query(query, false, max_states)
    }

    /// Case-insensitive variant of [`QueryDFA::from_query_bounded`].
    ///
    /// # Errors
    ///
    /// Returns [`StateLimitExceeded`] when the budget is exhausted.
    pub fn from_query_bounded_ignore_case(
        query: &Query,
        max_states: usize,
    ) -> Result<Self, StateLimitExceeded> {
        Self::build_from_query(query, true, max_states)
    }

    /// Parse a query string and compile it with a state budget in one step:
    /// the safe entry point for untrusted query strings.
    ///
    /// # Errors
    ///
    /// Returns [`QueryCompileError::Parse`] for invalid query strings and
    /// [`QueryCompileError::StateLimit`] when determinization would exceed
    /// `max_states`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::query::QueryDFA;
    ///
    /// let dfa =
    ///     QueryDFA::from_query_str_bounded("users[*].name", 10_000).unwrap();
    /// assert!(dfa.num_states <= 10_000);
    /// ```
    pub fn from_query_str_bounded(
        query: &str,
        max_states: usize,
    ) -> Result<Self, QueryCompileError> {
        let query: Query = query.parse()?;
        Ok(Self::from_query_bounded(&query, max_states)?)
    }

    /// Case-insensitive variant of [`QueryDFA::from_query_str_bounded`].
    ///
    /// # Errors
    ///
    /// Returns [`QueryCompileError`]; see
    /// [`QueryDFA::from_query_str_bounded`].
    pub fn from_query_str_bounded_ignore_case(
        query: &str,
        max_states: usize,
    ) -> Result<Self, QueryCompileError> {
        let query: Query = query.parse()?;
        Ok(Self::from_query_bounded_ignore_case(&query, max_states)?)
    }

    /// Shared constructor that threads `case_insensitive` into the builder.
    fn build_from_query(
        query: &Query,
        case_insensitive: bool,
        max_states: usize,
    ) -> Result<Self, StateLimitExceeded> {
        let mut builder = DFABuilder::new();
        builder.case_insensitive = case_insensitive;
        builder.max_states = max_states;
        builder.build_dfa(query)
    }

    /// Execute this compiled query against a JSON document, returning all
    /// matches.
    ///
    /// This is the preferred way to run queries. If you need to apply the same
    /// query to multiple documents, construct the [`QueryDFA`] once and call
    /// this method repeatedly.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsongrep::{Value, query::QueryDFA};
    ///
    /// let json: Value = serde_json::from_str(r#"{"a": 1, "b": 2}"#).unwrap();
    /// let query = QueryDFA::from_query_str("a").unwrap();
    /// let results = query.find(&json);
    /// assert_eq!(results.len(), 1);
    /// ```
    #[must_use]
    pub fn find<'a>(&self, json: &'a Value<'a>) -> Vec<JSONPointer<'a>> {
        DFAQueryEngine::find_with_dfa(json, self)
    }

    /// Check if a given state is accepting/final.
    #[must_use]
    pub fn is_accepting_state(&self, state: usize) -> bool {
        state < self.num_states && self.is_accepting[state]
    }

    /// Get the symbol index for a field name. When the DFA was built with
    /// case-insensitive matching, the key is lowercased before lookup.
    #[must_use]
    pub fn get_field_symbol_id(&self, field: &str) -> usize {
        let normalized = if self.case_insensitive {
            field.to_lowercase()
        } else {
            field.to_owned()
        };
        let field_rc = Rc::new(normalized);
        self.key_to_key_id
            .get(&field_rc)
            .copied()
            .unwrap_or(TransitionLabel::other_idx())
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

    /// Get the next state given current state and symbol.
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

    /// Whether fields are case-sensitive.
    pub case_insensitive: bool,

    /// Maximum number of DFA states determinization may create before
    /// aborting with [`StateLimitExceeded`].
    max_states: usize,
}

impl DFABuilder {
    fn new() -> Self {
        Self {
            // start with only the "other" symbol
            alphabet: vec![TransitionLabel::Other],
            key_to_key_id: HashMap::new(),
            collected_ranges: Vec::new(),
            range_to_range_id: Vec::new(),
            case_insensitive: false,
            max_states: usize::MAX,
        }
    }

    /// Recursively extract all symbols from a query to build the alphabet.
    fn extract_symbols(&mut self, query: &Query) {
        match query {
            Query::Field(name) => {
                // When case-insensitive, store the lowercased form so that
                // JSON keys are matched after normalization.
                let normalized = if self.case_insensitive {
                    name.to_lowercase()
                } else {
                    name.clone()
                };
                let name_rc: Rc<String> = Rc::new(normalized);
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
                // [idx: idx + 1). Saturate on usize::MAX: the resulting
                // empty range matches nothing, which is correct because an
                // element at that index cannot exist (and it must not wrap
                // to an inverted range or panic in debug builds).
                self.collected_ranges.push((*idx, idx.saturating_add(1)));
            }
            Query::Range(s, e) => {
                self.collected_ranges.push((
                    (*s).unwrap_or(usize::MIN),
                    (*e).unwrap_or(usize::MAX),
                ));
            }
            Query::RangeFrom(s) => self.collected_ranges.push((*s, usize::MAX)),
            Query::ArrayWildcard => {
                // Treat array wildcard as unbounded range query, as they are
                // equivalent
                self.collected_ranges.push((usize::MIN, usize::MAX));
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
        self.range_to_range_id.sort_by_key(|a| a.0.start);
    }

    /// Use subset construction to convert the constructed epsilon-free NFA to a DFA,
    /// producing a `QueryDFA`. For each DFA state, we map it to a set of NFA
    /// states.
    #[expect(clippy::too_many_lines)]
    fn determinize_nfa(
        &mut self,
        nfa: &QueryNFA,
    ) -> Result<QueryDFA, StateLimitExceeded> {
        // Use a HashMap to map sets of currently reachable NFA states to DFA
        // state indices
        // `curr_nfa_states_to_dfa_state[NFA states bitmap]` -> DFA state index
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
                                // Field match: when case-insensitive, the DFA
                                // alphabet stores lowercased names (from
                                // extract_symbols), so we lowercase the NFA
                                // field before comparing.
                                (
                                    TransitionLabel::Field(nfa_field),
                                    TransitionLabel::Field(dfa_field),
                                ) if {
                                    if self.case_insensitive {
                                        nfa_field.to_lowercase() == **dfa_field
                                    } else {
                                        nfa_field == dfa_field
                                    }
                                } =>
                                {
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
                                    TransitionLabel::Range(
                                        usize::MIN,
                                        usize::MAX,
                                    ),
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
                        if new_dfa_state >= self.max_states {
                            return Err(StateLimitExceeded {
                                limit: self.max_states,
                            });
                        }
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

        Ok(QueryDFA {
            num_states: dfa_states.len(),
            start_state: 0,
            is_accepting,
            transitions,
            // use the existing constructed finite alphabet from the DFABuilder
            alphabet: std::mem::take(&mut self.alphabet),
            key_to_key_id: std::mem::take(&mut self.key_to_key_id),
            range_to_range_id: std::mem::take(&mut self.range_to_range_id),
            case_insensitive: self.case_insensitive,
        })
    }

    /// Builds a deterministic finite automaton from a query.
    ///
    /// First, all the symbols from the query are extracted to obtain a
    /// finite alphabet. Then, potentially overlapping symbols like ranges are
    /// made disjoint. After this, the DFA is constructed first by turning the
    /// query into an epsilon-free NFA via the Glushkov construction, and then
    /// determinized to obtain the final DFA.
    fn build_dfa(
        &mut self,
        query: &Query,
    ) -> Result<QueryDFA, StateLimitExceeded> {
        // Every DFA has at least the start state, so a zero budget can
        // never be honored (this also covers the empty-query fast path
        // below, which returns a one-state DFA).
        if self.max_states == 0 {
            return Err(StateLimitExceeded { limit: 0 });
        }

        // Handle empty query case: match root (identity)
        if let Query::Sequence(steps) = query
            && steps.is_empty()
        {
            return Ok(QueryDFA {
                num_states: 1,
                start_state: 0,
                is_accepting: vec![true],
                transitions: vec![],
                alphabet: vec![],
                key_to_key_id: HashMap::new(),
                range_to_range_id: vec![],
                case_insensitive: false,
            });
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
#[derive(Debug)]
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

impl DFAQueryEngine {
    /// Search a JSON document using a pre-compiled [`QueryDFA`].
    ///
    /// This is useful when the same query is applied to multiple documents,
    /// as it avoids re-compiling the DFA on each call.
    #[must_use]
    pub fn find_with_dfa<'a>(
        json: &'a Value<'a>,
        dfa: &QueryDFA,
    ) -> Vec<JSONPointer<'a>> {
        let mut results = Vec::new();
        let mut path = Vec::new();
        Self::traverse_json(
            dfa,
            dfa.start_state,
            &mut path,
            json,
            &mut results,
        );
        results
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Unit testing.")]
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

        // Should have 2 matches
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn simple_index_access() {
        // Query: baz[1]
        let query = QueryBuilder::new().field("baz").index(1).build();
        let json = create_simple_test_json();
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
        // Expect empty result set
        assert!(matches.is_empty());
    }

    #[test]
    fn simple_array_wildcard() {
        let json = create_simple_test_json();

        // Query: `baz[*]`
        let query = QueryBuilder::new().field("baz").array_wildcard().build();
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
        assert!(!matches.is_empty());
        assert_eq!(matches.len(), 4);
    }

    #[test]
    fn empty_query() {
        let json = create_simple_test_json();
        let query = QueryBuilder::new().build();
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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
        let result = QueryDFA::from_query(&query).find(&json);

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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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

        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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

        let query: Query =
            r#""/activities""#.parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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

        let query: Query =
            r#"paths."/activities""#.parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let query: Query =
            r#"**."/activities""#.parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let query: Query = r#""a.b""#.parse().expect("failed to parse query");
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Str(Cow::Borrowed("value")));
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
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

        assert_eq!(matches.len(), 2);
    }

    // ==============================================================================
    // Case-insensitive matching tests
    // ==============================================================================

    /// Helper: build a case-insensitive DFA and run it against JSON input.
    fn find_ignore_case<'a>(
        json: &'a Value<'a>,
        query: &Query,
    ) -> Vec<JSONPointer<'a>> {
        let dfa = QueryDFA::from_query_ignore_case(query);
        DFAQueryEngine::find_with_dfa(json, &dfa)
    }

    #[test]
    fn case_insensitive_basic_field() {
        let input = r#"{ "FOO": 1, "bar": 2 }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        // Lowercase query matches uppercase JSON key
        let query = QueryBuilder::new().field("foo").build();
        let matches = find_ignore_case(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Number(1u64.into()));
    }

    #[test]
    fn case_insensitive_sequence() {
        let input = r#"{ "Foo": { "BAR": "found" } }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        let query = QueryBuilder::new().field("foo").field("bar").build();
        let matches = find_ignore_case(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Str(Cow::Borrowed("found")));
    }

    #[test]
    fn case_insensitive_mixed_case_query() {
        let input = r#"{ "foo": { "bar": "found" } }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        // Query uses mixed case, JSON keys are lowercase
        let query = QueryBuilder::new().field("FoO").field("bAr").build();
        let matches = find_ignore_case(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Str(Cow::Borrowed("found")));
    }

    #[test]
    fn case_insensitive_recursive_wildcard() {
        let input = r#"
        {
          "a": {
            "FOO": "deep"
          },
          "FOO": "shallow"
        }
        "#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        // **.foo with case-insensitive should find both "FOO" keys
        let query = QueryBuilder::new()
            .field_wildcard()
            .kleene_star()
            .field("foo")
            .build();
        let matches = find_ignore_case(&json, &query);

        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn case_insensitive_disjunction_dedup() {
        let input = r#"{ "foo": 1, "bar": 2 }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        // "Foo | foo" should not produce duplicate matches — both
        // normalize to the same DFA symbol.
        let q1 = QueryBuilder::new().field("Foo").build();
        let q2 = QueryBuilder::new().field("foo").build();
        let query = QueryBuilder::new().disjunction(vec![q1, q2]).build();
        let matches = find_ignore_case(&json, &query);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Number(1u64.into()));
    }

    // ==============================================================================
    // Bounded determinization
    // ==============================================================================

    /// Build the classic exponential-blowup query `(a|b)*.a.(a|b)^n`,
    /// which determinizes to 2^(n+1) + 1 DFA states.
    fn pathological_query(n: usize) -> Query {
        let mut q = String::from("(a|b)*.a");
        for _ in 0..n {
            q.push_str(".(a|b)");
        }
        q.parse().expect("valid query")
    }

    #[test]
    fn bounded_build_rejects_exponential_query() {
        let query = pathological_query(8); // 513 states unbounded
        let result = QueryDFA::from_query_bounded(&query, 100);
        assert_eq!(result.unwrap_err(), StateLimitExceeded { limit: 100 });
    }

    #[test]
    fn bounded_build_exact_boundary() {
        // 513 states unbounded: a budget of exactly 513 must succeed, and
        // 512 must fail (budget n permits at most n states).
        let query = pathological_query(8);
        assert!(QueryDFA::from_query_bounded(&query, 513).is_ok());
        assert!(QueryDFA::from_query_bounded(&query, 512).is_err());
    }

    #[test]
    fn bounded_build_zero_budget_always_fails() {
        let simple: Query = "a".parse().expect("valid query");
        assert!(QueryDFA::from_query_bounded(&simple, 0).is_err());

        // The empty query's one-state fast path must honor the budget too.
        let empty: Query = "".parse().expect("valid query");
        assert!(QueryDFA::from_query_bounded(&empty, 0).is_err());
        assert!(QueryDFA::from_query_bounded(&empty, 1).is_ok());
    }

    #[test]
    fn bounded_str_constructor_reports_both_error_kinds() {
        assert!(matches!(
            QueryDFA::from_query_str_bounded("unclosed\"", 1000),
            Err(QueryCompileError::Parse(_))
        ));
        let mut q = String::from("(a|b)*.a");
        for _ in 0..8 {
            q.push_str(".(a|b)");
        }
        assert!(matches!(
            QueryDFA::from_query_str_bounded(&q, 100),
            Err(QueryCompileError::StateLimit(_))
        ));
        assert!(QueryDFA::from_query_str_bounded("users[*].name", 100).is_ok());
    }

    #[test]
    fn bounded_build_succeeds_within_budget() {
        let query = pathological_query(8);
        let dfa = QueryDFA::from_query_bounded(&query, 1000)
            .expect("513 states fit in a 1000-state budget");
        assert_eq!(dfa.num_states, 513);

        // The bounded DFA behaves identically to the unbounded one.
        let json: Value =
            serde_json::from_str(r#"{"a": {"a": {"a": 1}}}"#).expect("json");
        let unbounded = QueryDFA::from_query(&query);
        assert_eq!(dfa.find(&json).len(), unbounded.find(&json).len());
    }

    #[test]
    fn bounded_build_ignore_case_respects_budget() {
        let query = pathological_query(8);
        let result = QueryDFA::from_query_bounded_ignore_case(&query, 100);
        assert!(result.is_err());
        assert!(QueryDFA::from_query_bounded_ignore_case(&query, 1000).is_ok());
    }

    #[test]
    fn bounded_build_normal_query_unaffected() {
        let query: Query = "users.[*].name".parse().expect("valid query");
        let json: Value = serde_json::from_str(
            r#"{"users": [{"name": "a"}, {"name": "b"}]}"#,
        )
        .expect("json");
        let dfa = QueryDFA::from_query_bounded(&query, 1 << 20)
            .expect("simple query is far below any realistic budget");
        assert_eq!(dfa.find(&json).len(), 2);
    }

    #[test]
    fn usize_max_index_matches_nothing_without_panic() {
        // [usize::MAX] used to compute idx + 1, panicking in debug builds
        // and silently wrapping to an inverted range in release. An element
        // at that index cannot exist, so the query must simply match
        // nothing.
        let input = r#"{ "arr": [1, 2, 3] }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        let query = format!("arr.[{}]", usize::MAX);
        let dfa = QueryDFA::from_query_str(&query).expect("valid query");
        assert_eq!(dfa.find(&json).len(), 0);

        // Sanity: a normal index on the same document still matches.
        let dfa = QueryDFA::from_query_str("arr.[2]").expect("valid query");
        assert_eq!(dfa.find(&json).len(), 1);
    }

    #[test]
    fn usize_max_index_alongside_real_range() {
        // The empty (MAX, MAX) label must not inherit or overlap any real
        // range symbol: only the [0:2] branch of the disjunction matches.
        let input = r#"{ "arr": [10, 20, 30] }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        let query = format!("arr.([{}] | [0:2])", usize::MAX);
        let dfa = QueryDFA::from_query_str(&query).expect("valid query");
        let matches = dfa.find(&json);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].value, &Value::Number(10u64.into()));
        assert_eq!(matches[1].value, &Value::Number(20u64.into()));
    }

    #[test]
    fn case_insensitive_from_query_str() {
        let input = r#"{ "Foo": { "BAR": "found" } }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        let dfa = QueryDFA::from_query_str_ignore_case("foo.bar")
            .expect("valid query");
        let matches = DFAQueryEngine::find_with_dfa(&json, &dfa);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Str(Cow::Borrowed("found")));
    }

    #[test]
    fn case_sensitive_default_unchanged() {
        let input = r#"{ "FOO": 1, "foo": 2 }"#;
        let json: Value = serde_json::from_str(input).expect("hardcoded json");

        // Case-sensitive (default) should only match exact case
        let query = QueryBuilder::new().field("foo").build();
        let matches: Vec<JSONPointer> =
            QueryDFA::from_query(&query).find(&json);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].value, &Value::Number(2u64.into()));
    }
}
