use std::collections::HashMap;

use rand::Rng;

use serde::{Deserialize, Serialize};


/// Represents a state in an n-gram model.
///
/// A `State` corresponds to a fixed (n-1)-character prefix (`key`) and stores
/// all observed transitions from this prefix to the next character.
///
/// Conceptually, this is a node in a Markov chain where outgoing edges
/// are weighted by their number of observations.
///
/// ## Responsibilities:
/// - Accumulate transition occurrences during learning
/// - Predict the next character using weighted random sampling
/// - Merge with another state having the same key (ex. parallel learning support)
///
/// ## Invariants
/// - All transitions belong to the same `key`
/// - Each transition occurrence count is strictly positive
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
	/// Identifier of the state (n-1 character prefix).
	key: String,
	/// Outgoing transitions indexed by the next character.
	/// The value represents how many times this transition was observed.
	/// Example: { 'e' => 42, 'a' => 3 }
	transitions: HashMap<char, usize>
}

impl State {
	/// Creates a new empty state for the given prefix.
	pub fn new(key: &str) -> Self {
		Self {
			key: key.to_owned(),
			transitions: HashMap::new(),
		}
	}

	/// Records an occurrence of a transition toward `next_char`.
	///
	/// - If the transition already exists, its occurrence count is increased.
	/// - Otherwise, a new transition is created with an initial count of 1.
	pub fn add_transition(&mut self, next_char: char) {
		*self.transitions.entry(next_char).or_insert(0) += 1;
	}

	/// Predicts the next character using weighted random sampling.
	///
	/// The probability of selecting a character is proportional to its
	/// occurrence count.
	///
	/// This method performs:
	/// - an O(n) scan over the transitions
	/// - a cumulative subtraction to select a bucket
	///
	/// Returns `None` if the state has no transitions.
	pub fn predict(&self) -> Option<char> {
		if self.transitions.is_empty() {
			return None;
		}

		// Compute the total number of occurrences
		let total: usize = self.transitions.iter().map(|(_, occurrence)| occurrence).sum();
		if total == 0 {
			// Should not happen due to invariants, but kept for safety
			return None;
		}

		// Randomly select a character
		let mut r = rand::rng().random_range(0..total);

		let mut fallback: Option<char> = None;
		for (next_char, occurrence) in &self.transitions {
			if r < *occurrence {
				return Some(*next_char);
			}
			r -= occurrence;
			fallback = Some(*next_char);
		}

		// Fallback: should not happen, but kept for safety.
		fallback
	}

	/// Merges another state into this one.
	///
	/// Both states must represent the same prefix (`key`).
	/// Transition occurrence counts are summed.
	///
	/// This method is intended for parallel learning (or merging DB),
	/// where multiple partial models are combined into a single one.
	///
	/// # Errors
	/// Returns an error if the state keys do not match.
	pub fn merge(&mut self, other: &Self) -> Result<(), String> {
		if self.key != other.key {
			return Err("Key mismatch".to_owned());
		}

		for (next_char, occurrence) in &other.transitions {
			*self.transitions.entry(*next_char).or_insert(0) += *occurrence;
		}

		Ok(())
	}
}
