use super::state::State;
use rand::prelude::IteratorRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an n-gram model for sequences of characters.
///
/// The `NGramModel` stores states for prefixes of length `n-1`
/// and allows probabilistic prediction of the next character
/// based on learned sequences.
///
/// # Responsibilities
/// - Build the n-gram model from sentences, words, or phrases
/// - Accumulate transition counts for each state
/// - Predict the next character given a prefix
/// - Merge with another n-gram model of the same order `n`
///
/// # Invariants
/// - `n` is always >= 2
/// - Each state in `states` corresponds to a unique prefix of length `n-1`
/// - All state transitions have occurrence counts >= 1
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NGramModel {
	/// The order of the model (number of characters in the n-gram)
	n: usize, // must be >= 2

	/// Mapping from a prefix (length n-1) to its corresponding state
	states: HashMap<String, State>,
}

impl NGramModel {
	/// Creates a new n-gram model of order `n`.
	///
	/// # Errors
	/// Returns an error if `n < 2`.
	pub fn new(n: usize) -> Result<Self, String> {
		if n < 2 {
			return Err("n must be >= 2".to_owned());
		}
		Ok(Self { n, states: HashMap::new() })
	}

	/// Returns a random prefix (seed) from the current model.
	///
	/// Useful for starting a generation sequence.
	/// Returns `None` if the model has no states.
	pub fn get_random_seed(&self) -> Option<String> {
		self.states.keys().choose(&mut rand::rng()).cloned()
	}

	/// Adds a sentence, word, or short phrase to the model.
	///
	/// Breaks the input into n-grams and updates states with observed transitions.
	///
	/// # Notes
	/// - Converts all characters to lowercase for consistency.
	/// - Ignores sentences shorter than `n`.
	pub fn add_sentence(&mut self, sentence: &str) {
		let chars: Vec<char> = sentence.chars().collect();
		if chars.len() < self.n {
			// Sentence too shorts, no n-grams to compute
			return;
		}

		// For each n-gram in the sentence
		for i in 0..=chars.len() - self.n {
			// Get the prefix and the next character
			let prefix: String = chars[i..i + self.n - 1]
				.iter()
				.flat_map(|c| c.to_lowercase())
				.collect();
			// Should not panic
			let next_char = chars[i + self.n - 1].to_lowercase().next().unwrap();

			// Get or create the state for this prefix
			let state = self.states.entry(prefix.clone()).or_insert_with(|| State::new(&prefix));
			state.add_transition(next_char);
		}
	}

	/// Predicts the next character given a prefix.
	///
	/// Returns `None` if the prefix is unknown or has no transitions.
	///
	/// # Notes
	/// - Converts the prefix to lowercase to match stored states.
	pub fn predict(&self, prefix: &str) -> Option<char> {
		let key: String = prefix
			.chars()
			.flat_map(|c| c.to_lowercase())
			.collect();
		self.states.get(&key)?.predict()
	}

	/// Merges another n-gram model into this one.
	///
	/// # Notes
	/// - Both models must have the same order `n`.
	/// - Occurrence counts for matching states and transitions are summed.
	///
	/// # Errors
	/// Returns an error if the model orders do not match.
	pub fn merge(&mut self, other: &Self) -> Result<(), String> {
		if self.n != other.n {
			return Err("N mismatch".to_owned());
		}

		for (key, state) in &other.states {
			if let Some(existing) = self.states.get_mut(key) {
				existing.merge(state)?;
			} else {
				self.states.insert(key.clone(), state.clone());
			}
		}

		Ok(())
	}
}
