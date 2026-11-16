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
/// A model holds millions of states, most of them with a single transition:
/// a small vector costs far less than a hash map per state.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
	/// Outgoing transitions with the number of times each one was observed.
	/// Example: `[('e', 42), ('a', 3)]`
	transitions: Vec<(char, u32)>,
}

impl State {
	/// Creates a new empty state.
	pub fn new() -> Self {
		Self {
			transitions: Vec::new(),
		}
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
		match self.transitions.iter_mut().find(|(known_char, _)| *known_char == next_char) {
			Some((_, count)) => *count = count.saturating_add(occurrences),
			None => self.transitions.push((next_char, occurrences)),
		}
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
		if self.transitions.is_empty() {
			return None;
		}

		// Compute the total number of occurrences
		let total: u64 = self.transitions.iter().map(|(_, occurrence)| u64::from(*occurrence)).sum();
		if total == 0 {
			// Should not happen due to invariants, kept for safety
			return None;
		}

		// Randomly select a character
		let mut r = rand::rng().random_range(0..total);
		let mut fallback: Option<char> = None;

		for (next_char, occurrence) in &self.transitions {
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
		for (next_char, occurrence) in &other.transitions {
			self.add_occurrences(*next_char, *occurrence);
		}
	}

	/// Releases the unused capacity of the transitions once learning is over.
	pub fn shrink_to_fit(&mut self) {
		self.transitions.shrink_to_fit();
	}
}

#[cfg(test)]
impl State {
	/// Returns the transitions sorted by character, to compare states in tests.
	pub(crate) fn sorted_transitions(&self) -> Vec<(char, usize)> {
		let mut transitions: Vec<(char, usize)> = self.transitions
			.iter()
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
		assert!(count_a > 900, "'a' predicted {} times out of 1000", count_a);
	}

	#[test]
	fn state_stays_small() {
		// Millions of states are loaded at once: any extra byte here is multiplied
		let size = std::mem::size_of::<State>();
		assert!(size <= 24, "a state takes {} bytes", size);
	}
}
