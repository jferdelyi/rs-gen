use rand::Rng;
use serde::{Deserialize, Serialize};

/// Represents a state in an n-gram model.
///
/// A `State` corresponds to a fixed `(n-1)`-character prefix and
/// stores all observed transitions from this prefix to the next character.
/// The prefix itself is the key of the state in its `NGramModel`.
///
/// Conceptually, this is a node in a Markov chain, where outgoing edges
/// are weighted by their number of observations.
///
/// # Responsibilities
/// - Accumulate transition counts during learning
/// - Predict the next character using weighted random sampling
/// - Merge with another state of the same prefix (useful for parallel or incremental learning)
///
/// # Invariants
/// - Each next character appears at most once in `transitions`
/// - Each transition occurrence count is strictly positive
///
/// # Memory
/// A model holds millions of states and 81 % of them have a single transition:
/// the first transition is stored in the state itself, and only the states with more
/// than one transition allocate the others. A state stays 24 bytes.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
	/// First transition and the number of times it was observed.
	/// A count of 0 means that the state has no transition yet.
	first: (char, u32),

	/// Other transitions with their number of observations, in the order they were added.
	/// Example: `[('e', 42), ('a', 3)]`
	others: Option<Box<[(char, u32)]>>,
}

impl State {
	/// Creates a new empty state.
	pub fn new() -> Self {
		Self {
			first: ('\0', 0),
			others: None,
		}
	}

	/// Returns every transition with its number of observations, the first one first.
	fn transitions(&self) -> impl Iterator<Item = &(char, u32)> {
		let first = if self.first.1 > 0 { Some(&self.first) } else { None };
		first.into_iter().chain(self.others.iter().flat_map(|others| others.iter()))
	}

	/// Returns the number of distinct transitions.
	pub(crate) fn transition_count(&self) -> usize {
		self.transitions().count()
	}

	/// Records an occurrence of a transition toward `next_char`.
	///
	/// If the transition already exists, its occurrence count is increased.
	/// Otherwise, a new transition is created with an initial count of 1.
	pub fn add_transition(&mut self, next_char: char) {
		self.add_occurrences(next_char, 1);
	}

	/// Adds `occurrences` to the transition toward `next_char`, creating it if needed.
	fn add_occurrences(&mut self, next_char: char, occurrences: u32) {
		if self.first.1 == 0 {
			self.first = (next_char, occurrences);
			return;
		}
		if self.first.0 == next_char {
			self.first.1 = self.first.1.saturating_add(occurrences);
			return;
		}
		if let Some(others) = &mut self.others
			&& let Some((_, count)) = others.iter_mut().find(|(known_char, _)| *known_char == next_char)
		{
			*count = count.saturating_add(occurrences);
			return;
		}
		// A new transition: the others are reallocated at their exact size, since they stay few
		let mut others = self.others.take().map(<[(char, u32)]>::into_vec).unwrap_or_default();
		others.push((next_char, occurrences));
		self.others = Some(others.into_boxed_slice());
	}

	/// Predicts the next character using weighted random sampling.
	///
	/// The probability of selecting a character is proportional to its
	/// occurrence count.
	///
	/// # Algorithm
	/// - Linear scan over all transitions (O(n) in the number of transitions)
	/// - Subtracts randomly generated number from cumulative counts to select a bucket
	///
	/// Returns `None` if the state has no transitions.
	pub fn predict(&self) -> Option<char> {
		if self.first.1 == 0 {
			return None;
		}
		// Most states have a single transition: no random number needed
		if self.others.is_none() {
			return Some(self.first.0);
		}

		// Compute the total number of occurrences
		let total: u64 = self.transitions().map(|(_, occurrence)| u64::from(*occurrence)).sum();

		// Randomly select a character
		let mut r = rand::rng().random_range(0..total);
		let mut fallback: Option<char> = None;

		for (next_char, occurrence) in self.transitions() {
			let occurrence = u64::from(*occurrence);
			if r < occurrence {
				return Some(*next_char);
			}
			r -= occurrence;
			fallback = Some(*next_char);
		}

		// Fallback: safety measure; should not happen if invariants hold
		fallback
	}

	/// Merges another state into this one.
	///
	/// Both states must represent the same prefix: the caller guarantees it
	/// by merging states stored under the same key.
	/// Transition counts are summed.
	///
	/// Useful for parallel learning or combining multiple partial models.
	pub fn merge(&mut self, other: &Self) {
		for (next_char, occurrence) in other.transitions() {
			self.add_occurrences(*next_char, *occurrence);
		}
	}
}

#[cfg(test)]
impl State {
	/// Returns the transitions sorted by character, to compare states in tests.
	pub(crate) fn sorted_transitions(&self) -> Vec<(char, usize)> {
		let mut transitions: Vec<(char, usize)> = self
			.transitions()
			.map(|(next_char, occurrence)| (*next_char, *occurrence as usize))
			.collect();
		transitions.sort();
		transitions
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashMap;

	// Builds a state that observed each character of `observed` once
	fn state_with(observed: &[char]) -> State {
		let mut state = State::new();
		for next_char in observed {
			state.add_transition(*next_char);
		}
		state
	}

	#[test]
	fn add_transition_counts_each_occurrence() {
		let state = state_with(&['a', 'b', 'a', 'a']);
		assert_eq!(state.sorted_transitions(), vec![('a', 3), ('b', 1)]);
	}

	#[test]
	fn predict_returns_none_without_transition() {
		assert_eq!(state_with(&[]).predict(), None);
	}

	#[test]
	fn predict_returns_the_only_transition() {
		let state = state_with(&['x', 'x']);
		for _ in 0..50 {
			assert_eq!(state.predict(), Some('x'));
		}
	}

	#[test]
	fn predict_returns_only_observed_characters() {
		let state = state_with(&['a', 'b', 'c']);
		let mut seen = HashMap::new();
		for _ in 0..300 {
			let next_char = state.predict().expect("a state with transitions must predict");
			*seen.entry(next_char).or_insert(0) += 1;
		}
		let mut keys: Vec<char> = seen.keys().copied().collect();
		keys.sort();
		assert_eq!(keys, vec!['a', 'b', 'c']);
	}

	#[test]
	fn predict_follows_occurrence_weights() {
		let mut observed = vec!['a'; 99];
		observed.push('b');
		let state = state_with(&observed);
		let count_a = (0..1000).filter(|_| state.predict() == Some('a')).count();
		// Expected around 990; 900 leaves a very wide margin against bad luck
		assert!(count_a > 900, "'a' predicted {count_a} times out of 1000");
	}

	#[test]
	fn a_single_transition_allocates_nothing() {
		// 81 % of the states have a single transition: it must live in the state itself
		let state = state_with(&['a', 'a']);
		assert!(state.others.is_none());
		assert_eq!(state.sorted_transitions(), vec![('a', 2)]);
	}

	#[test]
	fn state_stays_small() {
		// Millions of states are loaded at once: any extra byte here is multiplied
		let size = std::mem::size_of::<State>();
		assert!(size <= 24, "a state takes {size} bytes");
	}
}
