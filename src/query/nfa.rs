/*!
# Query NFA

Constructs an epsilon-free non-deterministic finite automaton for JSON queries.
Serves as an intermediary implementation for converting a query into a DFA that
can be simulated. This NFA is not intended to be simulated, but instead
determinized via the subset construction to produce a DFA recognizing the same
language of the NFA.

The construction of the NFA closely follows the description of the Glushkov NFA
presented in the Wikipedia article on the algorithm.

For reference:

- [Wikipedia: Glushkov's construction algorithm](https://en.wikipedia.org/wiki/Glushkov's_construction_algorithm)
*/
use std::{fmt::Display, rc::Rc};

use crate::query::ast::Query;
use crate::query::common::TransitionLabel;

/// Represents a Non-Deterministic Finite Automaton (NFA) for JSON queries.
/// Importantly, the alphabet depends on the query.
pub struct QueryNFA {
    /// The number of states in the NFA.
    pub num_states: usize,

    /// Transition function for the NFA, which is an adjacency list of labeled
    /// graph, where:
    ///
    /// transitions\[state\] -> <Vec<(label_idx, destination next state>>
    pub transitions: Vec<Vec<(usize, usize)>>,

    /// Index in linearized query to atom/ predicate
    /// pos_to_label\[idx\] = TransitionLabel
    pub pos_to_label: Vec<TransitionLabel>,

    /// The starting state for the NFA; `0`
    pub start_state: usize,

    /// Bitmap of accepting/ final states
    pub is_accepting: Vec<bool>,

    /// Bitmap of alphabet symbols within the "first set" of the NFA; P(e')
    pub is_first: Vec<bool>,

    /// Bitmap of alphabet symbols within the "last set" of the NFA; D(e')
    pub is_ending: Vec<bool>,

    /// The set of letter pairs that can occur in words of L(e'), i.e., the set
    /// of factors of length 2 of the words of L(e')
    /// factors\[symbol\] = set of symbols that can follow `symbol` in a word
    pub factors: Vec<Vec<usize>>,

    /// Whether the empty word belongs to L(e')
    pub contains_empty_word: bool,
}

impl Display for QueryNFA {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "NFA States: {}", self.num_states)?;
        writeln!(f, "Start State: {}", self.start_state)?;
        writeln!(f, "Accepting States: {:?}", {
            self.is_accepting
                .iter()
                .enumerate()
                .filter_map(|(i, &b)| if b { Some(i) } else { None })
                .collect::<Vec<_>>()
        })?;
        writeln!(
            f,
            "First set: {:?}",
            self.is_first
                .iter()
                .enumerate()
                .filter_map(|(i, &b)| if b {
                    Some(format!("[{}] {}", i, &self.pos_to_label[i]))
                } else {
                    None
                })
                .collect::<Vec<_>>()
        )?;
        writeln!(
            f,
            "Last set: {:?}",
            self.is_ending
                .iter()
                .enumerate()
                .filter_map(|(i, &b)| if b {
                    Some(format!("[{}] {}", i, &self.pos_to_label[i]))
                } else {
                    None
                })
                .collect::<Vec<_>>()
        )?;
        writeln!(f, "Factors set:")?;
        for (i, followers) in self.factors.iter().enumerate() {
            if followers.is_empty() {
                writeln!(f, "\t[{}] {} cannot be followed", i, &self.pos_to_label[i])?;
                continue;
            }
            writeln!(f, "\t[{}] {} can be followed by:", i, &self.pos_to_label[i])?;
            for &j in followers {
                writeln!(f, "\t\t[{}] {}", j, &self.pos_to_label[j])?;
            }
        }
        writeln!(f, "Transitions:")?;
        for (st, row) in self.transitions.iter().enumerate() {
            writeln!(f, "\tstate {}:", st)?;
            for (label_idx, dest) in row {
                writeln!(
                    f,
                    "\t\ton [{}] {} -> [{}]",
                    *label_idx, self.pos_to_label[*label_idx], dest
                )?;
            }
        }
        Ok(())
    }
}

impl QueryNFA {
    /// Construct an NFA recognizing the language defined by a query.
    pub fn from_query(query: &Query) -> QueryNFA {
        let mut temp_nfa = QueryNFA {
            num_states: 1, // start state
            transitions: Vec::new(),
            pos_to_label: Vec::new(),
            start_state: 0,
            is_accepting: vec![false; 1], // initially just start state
            is_first: Vec::new(),
            is_ending: Vec::new(),
            factors: Vec::new(),
            contains_empty_word: false,
        };

        // Linearize query
        temp_nfa.linearize_query(query);

        let alphabet_size = temp_nfa.pos_to_label.len();

        // Handle empty query case
        if alphabet_size == 0 {
            let empty_nfa = QueryNFA {
                num_states: 1, // start state
                transitions: Vec::new(),
                pos_to_label: Vec::new(),
                start_state: 0,
                is_accepting: vec![true; 1],
                is_first: Vec::new(),
                is_ending: Vec::new(),
                factors: Vec::new(),
                contains_empty_word: true,
            };

            #[cfg(test)]
            println!("Constructed NFA for `{}`:\n{}", query, empty_nfa);

            return empty_nfa;
        }

        // Update number of states: 0 (start) + one per linearized symbol
        temp_nfa.num_states += alphabet_size;

        // Compute sets
        temp_nfa.contains_empty_word = contains_empty_word(query);

        // The current position in the linearized alphabet
        let mut position: usize;

        position = 0;
        temp_nfa.is_first = vec![false; alphabet_size];
        compute_first_set(&mut temp_nfa.is_first, query, &mut position);

        // Start at right-most position
        position = 0;
        temp_nfa.is_ending = vec![false; alphabet_size];
        compute_last_set(&mut temp_nfa.is_ending, query, &mut position);

        // Compute follows set last so that first and last sets are available
        // for boundary pairings
        position = 0;
        temp_nfa.factors = vec![Vec::new(); alphabet_size];
        compute_follows_set(&mut temp_nfa.factors, query, &mut position);

        // Construct automaton
        // NOTE: + 1 for transitions and final states to include state 0 (start state)
        temp_nfa.transitions = vec![Vec::new(); 1 + alphabet_size];
        temp_nfa.is_accepting = vec![false; 1 + alphabet_size];
        let nfa = temp_nfa.construct_nfa();

        #[cfg(test)]
        println!("Constructed NFA for `{}`:\n{}", query, nfa);

        nfa
    }

    /// Recursively extract all symbols from a query to build the linearized
    /// alphabet.
    fn linearize_query(&mut self, query: &Query) {
        match query {
            Query::Field(name) => {
                // create a new key state if it does not exist
                let name_rc: Rc<String> = Rc::new(name.clone());
                self.pos_to_label
                    .push(TransitionLabel::Field(name_rc.clone()));
            }
            Query::FieldWildcard => {
                let field_wildcard = TransitionLabel::FieldWildcard;
                self.pos_to_label.push(field_wildcard);
            }
            Query::Index(idx) => {
                // Represent individual index as a single-element range
                // [idx: idx + 1)
                let range = TransitionLabel::Range(*idx, *idx + 1);
                self.pos_to_label.push(range);
            }
            Query::Range(s, e) => {
                let range = TransitionLabel::Range(*s, *e);
                self.pos_to_label.push(range);
            }
            Query::RangeFrom(s) => self.pos_to_label.push(TransitionLabel::RangeFrom(*s)),
            Query::ArrayWildcard => {
                // Treat array wildcard as unbounded range query, as they are
                // equivalent
                let range = TransitionLabel::Range(0, usize::MAX);
                self.pos_to_label.push(range);
            }
            Query::Disjunction(queries) | Query::Sequence(queries) => {
                for q in queries {
                    self.linearize_query(q);
                }
            }
            Query::KleeneStar(q) | Query::Optional(q) => {
                self.linearize_query(q);
            }
            _ => unimplemented!(),
        }
    }

    /// Construct the automaton recognizing the language defined by the query.
    pub fn construct_nfa(&mut self) -> QueryNFA {
        // Mark start state as accepting if empty word is in language
        if self.contains_empty_word {
            self.is_accepting[0] = true;
        }

        // Add transitions from start state to states in first set
        for (pos, &is_first) in self.is_first.iter().enumerate() {
            if is_first {
                self.transitions[0].push((pos, pos + 1));
                // state 0 -> state (pos + 1)
            }
        }

        // Mark final states (positions in last set)
        for (pos, &is_ending) in self.is_ending.iter().enumerate() {
            if is_ending {
                self.is_accepting[pos + 1] = true; // state is accepting
            }
        }

        // Add transitions from follows set
        for (from_state, followers) in self.factors.iter().enumerate() {
            for &follower in followers {
                self.transitions[from_state + 1]
                    // + 1 to follower to account for start state
                    .push((follower, follower + 1));
            }
        }

        QueryNFA {
            num_states: self.num_states,
            transitions: std::mem::take(&mut self.transitions),
            pos_to_label: std::mem::take(&mut self.pos_to_label),
            start_state: self.start_state,
            is_accepting: std::mem::take(&mut self.is_accepting),
            is_first: std::mem::take(&mut self.is_first),
            is_ending: std::mem::take(&mut self.is_ending),
            factors: std::mem::take(&mut self.factors),
            contains_empty_word: self.contains_empty_word,
        }
    }
}

/// Recursively determines whether the empty word is a member of L(e').
pub fn contains_empty_word(query: &Query) -> bool {
    match query {
        Query::Field(_)
        | Query::Index(_)
        | Query::Range(_, _)
        | Query::RangeFrom(_)
        | Query::ArrayWildcard
        | Query::FieldWildcard => false,
        Query::Sequence(queries) => queries.iter().all(contains_empty_word),
        Query::Disjunction(queries) => queries.iter().any(contains_empty_word),
        Query::Optional(_) => true,
        Query::KleeneStar(_) => true,
        _ => unimplemented!(),
    }
}

/// Recursively computes the set of letters which occur as the first letter
/// of a word in L(e').
pub fn compute_first_set(first_set: &mut [bool], query: &Query, position: &mut usize) {
    match query {
        Query::Field(_)
        | Query::Index(_)
        | Query::Range(_, _)
        | Query::RangeFrom(_)
        | Query::ArrayWildcard
        | Query::FieldWildcard
        | Query::Regex(_) => {
            if *position < first_set.len() {
                first_set[*position] = true;
                *position += 1;
            }
        }
        Query::Disjunction(queries) => {
            for q in queries {
                let start_pos = *position;
                let branch_len = count_subquery_positions(q);
                compute_first_set(first_set, q, position);
                *position = start_pos + branch_len;
            }
        }
        Query::Sequence(queries) => {
            for q in queries {
                compute_first_set(first_set, q, position);
                if !contains_empty_word(q) {
                    break;
                }
            }
        }
        Query::KleeneStar(q) => {
            compute_first_set(first_set, q, position);
        }
        Query::Optional(q) => {
            compute_first_set(first_set, q, position);
        }
    }
}

/// Recursively computes the set of letters which occur as the last letter
/// of a word in L(e').
pub fn compute_last_set(last_set: &mut [bool], query: &Query, position: &mut usize) {
    match query {
        Query::Field(_)
        | Query::Index(_)
        | Query::Range(_, _)
        | Query::RangeFrom(_)
        | Query::ArrayWildcard
        | Query::FieldWildcard
        | Query::Regex(_) => {
            if *position < last_set.len() {
                last_set[*position] = true;
                *position += 1;
            }
        }
        Query::Disjunction(queries) => {
            for q in queries {
                let start_pos = *position;
                let branch_len = count_subquery_positions(q);
                compute_last_set(last_set, q, position);
                *position = start_pos + branch_len;
            }
        }
        Query::Sequence(queries) => {
            // Compute how many positions each subquery consumes
            let subquery_lengths: Vec<usize> =
                queries.iter().map(count_subquery_positions).collect();

            // Store starting position of the sequence
            let seq_start_pos = *position;

            // Process the queries in reverse order
            for (i, q) in queries.iter().enumerate().rev() {
                // Sum the length of the subqueries before the current one to
                // determine the current's starting position
                let mut subquery_start =
                    seq_start_pos + subquery_lengths[..i].iter().sum::<usize>();
                compute_last_set(last_set, q, &mut subquery_start);
                if !contains_empty_word(q) {
                    break;
                }
            }
            // Advance past the sequence
            *position = seq_start_pos + subquery_lengths.iter().sum::<usize>();
        }
        Query::KleeneStar(q) | Query::Optional(q) => {
            compute_last_set(last_set, q, position);
        }
    }
}

/// Calculate the number of positions a given subquery consumes of a linearized
/// alphabet.
fn count_subquery_positions(query: &Query) -> usize {
    match query {
        Query::Field(_)
        | Query::Index(_)
        | Query::Range(_, _)
        | Query::RangeFrom(_)
        | Query::ArrayWildcard
        | Query::FieldWildcard => 1,
        Query::Sequence(queries) | Query::Disjunction(queries) => {
            queries.iter().map(count_subquery_positions).sum()
        }
        Query::Optional(q) | Query::KleeneStar(q) => count_subquery_positions(q),
        _ => unimplemented!(),
    }
}

/// Recursively computes the factors set of letter bigrams that can occur in a
/// word in L(e').
pub fn compute_follows_set(factors: &mut [Vec<usize>], query: &Query, position: &mut usize) {
    match query {
        Query::Field(_)
        | Query::Index(_)
        | Query::Range(_, _)
        | Query::RangeFrom(_)
        | Query::ArrayWildcard
        | Query::FieldWildcard
        | Query::Regex(_) => {
            // Base case: no internal factors
            *position += 1;
        }
        // F(e+f) = F(e) U F(f)
        Query::Disjunction(queries) => {
            for q in queries {
                compute_follows_set(factors, q, position);
            }
        }

        // F(ef) = F(e) U F(f) U D(e)P(f)
        Query::Sequence(queries) => {
            // Compute subquery position ranges
            let mut subquery_ranges = Vec::new();

            // Compute each subquery's factors
            for q in queries {
                let sub_start = *position;
                let sub_len = count_subquery_positions(q);
                compute_follows_set(factors, q, position);
                subquery_ranges.push((sub_start, sub_start + sub_len));
            }

            // Compute boundary pairings; D(e)P(f)
            // Consider ALL possible transitions, not just adjacent ones, meaning that
            // is Λ(f) = { ε } (f is nullable/ contain empty word), continue with next
            // possible follow query
            for i in 0..queries.len() {
                let left_query = &queries[i];
                let (left_start, left_end) = subquery_ranges[i];

                // Compute D(left_query)
                let mut left_last = vec![false; factors.len()];
                let mut left_pos = left_start;
                compute_last_set(&mut left_last, left_query, &mut left_pos);

                // For each subsequent query j where all queries between i and j
                // can accept empty word, add D(queries[i])P(queries[j])
                for j in (i + 1)..queries.len() {
                    // Check if all queries between i and j (exclusive) can accept empty word
                    let can_skip_middle = (i + 1..j).all(|k| contains_empty_word(&queries[k]));

                    if can_skip_middle {
                        let right_query = &queries[j];
                        let (right_start, right_end) = subquery_ranges[j];

                        // Compute P(right_query)
                        let mut right_first = vec![false; factors.len()];
                        let mut right_pos = right_start;
                        compute_first_set(&mut right_first, right_query, &mut right_pos);

                        // Add boundary pairs: for each element in D(left), add all
                        // elements in P(right)
                        for left_idx in left_start..left_end {
                            if left_last[left_idx] {
                                (right_start..right_end).for_each(|right_idx| {
                                    if right_first[right_idx] {
                                        factors[left_idx].push(right_idx);
                                    }
                                });
                            }
                        }
                    }

                    // If the current right query doesn't accept empty word,
                    // we can't skip past it, so break
                    if !contains_empty_word(&queries[j]) {
                        break;
                    }
                }
            }
        }

        // F(e*) = F(e) U D(e)P(e)
        Query::KleeneStar(q) => {
            let start_pos = *position;
            let q_len = count_subquery_positions(q);

            // Compute F(e)
            compute_follows_set(factors, q, position);

            // Add boundary pairs: D(e)P(e)
            let mut last_set = vec![false; factors.len()];
            let mut last_pos = start_pos;
            compute_last_set(&mut last_set, q, &mut last_pos);

            let mut first_set = vec![false; factors.len()];
            let mut first_pos = start_pos;
            compute_first_set(&mut first_set, q, &mut first_pos);

            for i in start_pos..start_pos + q_len {
                if last_set[i] {
                    (start_pos..start_pos + q_len).for_each(|j| {
                        if first_set[j] {
                            factors[i].push(j);
                        }
                    });
                }
            }
        }

        // F(e?) = F(e)
        Query::Optional(q) => {
            compute_follows_set(factors, q, position);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryBuilder;

    /// Compute number of members in a bit set.
    fn number_of_members(bitset: &[bool]) -> usize {
        bitset
            .iter()
            .fold(0, |acc, b| if *b { acc + 1 } else { acc })
    }

    #[test]
    fn test_empty_query_nfa() {
        let query = QueryBuilder::new().build();
        let nfa = QueryNFA::from_query(&query);
        assert_eq!(nfa.num_states, 1);
        assert!(nfa.contains_empty_word);
    }

    #[test]
    fn test_simple_field_nfa() {
        let query = QueryBuilder::new().field("foo").build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(nfa.num_states, 2); // start + field symbol
        assert_eq!(nfa.pos_to_label.len(), 1);
        assert!(!nfa.contains_empty_word);

        assert!(nfa.is_first[0]);
        assert_eq!(number_of_members(&nfa.is_first), 1);
        assert!(nfa.is_ending[0]);
        assert_eq!(number_of_members(&nfa.is_ending), 1);
    }

    #[test]
    fn test_simple_optional_field_nfa() {
        let query = QueryBuilder::new().field("foo").optional().build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(nfa.num_states, 2); // start + field symbol
        assert_eq!(nfa.pos_to_label.len(), 1);
        assert!(nfa.contains_empty_word);

        assert_eq!(number_of_members(&nfa.is_accepting), 2);
        assert_eq!(number_of_members(&nfa.is_first), 1);
        assert_eq!(number_of_members(&nfa.is_ending), 1);

        assert!(nfa.is_first[0]);
        assert!(nfa.is_ending[0]);
    }

    #[test]
    fn test_simple_seq_nfa() {
        let query = QueryBuilder::new()
            .field("foo")
            .field("bar")
            .field("baz")
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 1);
        assert_eq!(number_of_members(&nfa.is_first), 1);
        assert!(nfa.is_first[0]);

        assert_eq!(number_of_members(&nfa.is_ending), 1);
        assert!(nfa.is_ending[2]);
    }

    #[test]
    fn test_simple_seq_dis_nfa() {
        let query1 = QueryBuilder::new()
            .field("foo")
            .field("bar")
            .field("baz")
            .build();
        let query2 = QueryBuilder::new().field("x").field("y").field("z").build();
        let query = QueryBuilder::new()
            .disjunction(vec![query1, query2])
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 2);

        assert_eq!(number_of_members(&nfa.is_first), 2);
        assert!(nfa.is_first[0]);
        assert!(nfa.is_first[3]);

        assert_eq!(number_of_members(&nfa.is_ending), 2);
        assert!(nfa.is_ending[2]);
        assert!(nfa.is_ending[5]);
    }

    #[test]
    fn test_simple_disjunction_nfa() {
        let query1 = QueryBuilder::new().field("foo").build();
        let query2 = QueryBuilder::new().field("bar").build();
        let query = QueryBuilder::new()
            .disjunction(vec![query1, query2])
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 2);

        assert_eq!(number_of_members(&nfa.is_first), 2);
        assert!(&nfa.is_first[0]); // `foo`
        assert!(&nfa.is_first[1]); // `bar`

        assert_eq!(number_of_members(&nfa.is_ending), 2);
        assert!(nfa.is_ending[0]);
        assert!(nfa.is_ending[1]);
    }

    #[test]
    fn test_field_branch_disjunction_nfa() {
        let query1 = QueryBuilder::new().field("foo").field("a").build();
        let query2 = QueryBuilder::new().field("foo").field("b").build();
        let query = QueryBuilder::new()
            .disjunction(vec![query1, query2])
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 2);

        assert_eq!(number_of_members(&nfa.is_first), 2); // the two different "foo"s
        assert!(&nfa.is_first[0]); // first `foo`
        assert!(&nfa.is_first[2]); // second `foo`

        assert_eq!(number_of_members(&nfa.is_ending), 2);
        assert!(&nfa.is_ending[1]);
        assert!(&nfa.is_ending[3]);
    }

    #[test]
    fn test_foobar_nfa() {
        let query1 = QueryBuilder::new().field("foo").field("a").build();
        let query2 = QueryBuilder::new().field("bar").field("b").build();
        let query = QueryBuilder::new()
            .disjunction(vec![query1, query2])
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 2);
        assert_eq!(number_of_members(&nfa.is_first), 2);
        assert!(&nfa.is_first[0]);
        assert!(&nfa.is_first[2]);

        assert_eq!(number_of_members(&nfa.is_ending), 2);
        assert!(&nfa.is_ending[1]);
        assert!(&nfa.is_ending[3]);
    }

    #[test]
    fn test_complex_disjunction_nfa() {
        let query1 = QueryBuilder::new().field("foo").field("bar").build();
        let query2 = QueryBuilder::new().field("bar").optional().build();
        let query3 = QueryBuilder::new().field("baz").kleene_star().build();
        let query = QueryBuilder::new()
            .disjunction(vec![query1, query2, query3])
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 4);
        assert_eq!(number_of_members(&nfa.is_first), 3);
        assert!(&nfa.is_first[0]); // `foo`
        assert!(&nfa.is_first[2]); // second `bar`
        assert!(&nfa.is_first[3]); // `baz`

        assert_eq!(number_of_members(&nfa.is_ending), 3);
        assert!(&nfa.is_ending[1]); // first `bar`
        assert!(&nfa.is_ending[2]); // second `bar`
        assert!(&nfa.is_ending[3]); // `baz`
    }

    #[test]
    fn test_range_overlap_nfa() {
        let query1 = QueryBuilder::new().field("foo").index(1).build();
        let query2 = QueryBuilder::new().field("foo").array_wildcard().build();
        let query = QueryBuilder::new()
            .disjunction(vec![query1, query2])
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 2);
        assert_eq!(number_of_members(&nfa.is_first), 2);
        assert!(&nfa.is_first[0]); // first `foo`
        assert!(&nfa.is_first[2]); // second `foo`

        assert_eq!(number_of_members(&nfa.is_ending), 2);
        assert!(&nfa.is_ending[1]); // [1] index
        assert!(&nfa.is_ending[3]); // array wildcard
    }

    #[test]
    fn test_kleene_nfa() {
        let query = QueryBuilder::new()
            .field("a")
            .kleene_star()
            .field("b")
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 1);
        assert_eq!(number_of_members(&nfa.is_first), 2); // `a` or `b`
        assert!(&nfa.is_first[0]); // `a`
        assert!(&nfa.is_first[1]); // `b`

        assert_eq!(number_of_members(&nfa.is_ending), 1); // must end with `b`
        assert!(&nfa.is_ending[1]); // `b`
    }

    #[test]
    fn test_multiple_optional_nfa() {
        // Query: `a*.b?.c?`
        let query = QueryBuilder::new()
            .field("a")
            .kleene_star()
            .field("b")
            .optional()
            .field("c")
            .optional()
            .build();
        let nfa = QueryNFA::from_query(&query);

        assert_eq!(number_of_members(&nfa.is_accepting), 4);

        assert_eq!(number_of_members(&nfa.is_first), 3); // `a` or `b` or `c`
        assert!(&nfa.is_first[0]); // `a`
        assert!(&nfa.is_first[1]); // `b`
        assert!(&nfa.is_first[2]); // `b`

        assert_eq!(number_of_members(&nfa.is_ending), 3); // must end with `b`
        assert!(&nfa.is_ending[0]); // `a`
        assert!(&nfa.is_ending[1]); // `b`
        assert!(&nfa.is_ending[2]); // `b`
    }

    #[test]
    fn test_kleene_sequence_nfa() {
        let query = QueryBuilder::new()
            .field_wildcard()
            .kleene_star()
            .array_wildcard()
            .kleene_star()
            .array_wildcard()
            .build();
        let nfa = QueryNFA::from_query(&query);
        assert!(
            nfa.factors[0].contains(&2),
            "FieldWildcard should be followed by second ArrayWildcard"
        );
    }
}
