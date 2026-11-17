use super::state::State;
use crate::hash::FastHashMap;
use rand::prelude::IteratorRandom;
use serde::{Deserialize, Serialize};

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

	/// Mapping from a prefix (length n-1) to its corresponding state.
	/// `Box<str>` holds the same UTF-8 text as a `String` (accents included),
	/// without its spare capacity.
	states: FastHashMap<Box<str>, State>,
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
		Ok(Self { n, states: FastHashMap::default() })
	}

	/// Returns a random prefix (seed) from the current model.
	///
	/// Useful for starting a generation sequence.
	/// Returns `None` if the model has no states.
	pub fn get_random_seed(&self) -> Option<String> {
		self.states.keys().choose(&mut rand::rng()).map(|key| key.to_string())
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
			match self.states.get_mut(prefix.as_str()) {
				Some(state) => state.add_transition(next_char),
				None => {
					let mut state = State::new();
					state.add_transition(next_char);
					self.states.insert(prefix.into_boxed_str(), state);
				}
			}
		}
	}

	/// Predicts the next character given a prefix.
	///
	/// Returns `None` if the prefix is unknown or has no transitions.
	///
	/// # Notes
	/// - Converts the prefix to lowercase to match stored states.
	pub fn predict(&self, prefix: &str) -> Option<char> {
		// Generated prefixes are already lowercase: they are looked up without a copy
		let state = if prefix.chars().all(|c| c.to_lowercase().eq(std::iter::once(c))) {
			self.states.get(prefix)
		} else {
			let key: String = prefix
				.chars()
				.flat_map(|c| c.to_lowercase())
				.collect();
			self.states.get(key.as_str())
		};
		state?.predict()
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
				existing.merge(state);
			} else {
				self.states.insert(key.clone(), state.clone());
			}
		}

		Ok(())
	}

	/// Releases the unused capacity of the states once learning is over.
	pub(crate) fn shrink_to_fit(&mut self) {
		self.states.shrink_to_fit();
	}

	/// Returns the number of states and the UTF-8 bytes of their prefixes.
	pub(crate) fn states_and_key_bytes(&self) -> (usize, usize) {
		(self.states.len(), self.states.keys().map(|key| key.len()).sum())
	}

	/// Returns the number of transitions over every state.
	pub(crate) fn transition_count(&self) -> usize {
		self.states.values().map(State::transition_count).sum()
	}
}

#[cfg(test)]
impl NGramModel {
	/// Returns every state as (prefix, sorted transitions), sorted by prefix, to compare models in tests.
	pub(crate) fn dump(&self) -> Vec<(String, Vec<(char, usize)>)> {
		let mut states: Vec<(String, Vec<(char, usize)>)> = self.states
			.iter()
			.map(|(key, state)| (key.to_string(), state.sorted_transitions()))
			.collect();
		states.sort();
		states
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Builds a model of order `n` from the given sentences
	fn model_with(n: usize, sentences: &[&str]) -> NGramModel {
		let mut model = NGramModel::new(n).unwrap();
		for sentence in sentences {
			model.add_sentence(sentence);
		}
		model
	}

	#[test]
	fn new_rejects_order_below_two() {
		assert!(NGramModel::new(1).is_err());
		assert!(NGramModel::new(2).is_ok());
	}

	#[test]
	fn add_sentence_records_lowercase_transitions() {
		let model = model_with(3, &["<AbC>"]);
		assert_eq!(model.dump(), vec![
			("<a".to_owned(), vec![('b', 1)]),
			("ab".to_owned(), vec![('c', 1)]),
			("bc".to_owned(), vec![('>', 1)]),
		]);
	}

	#[test]
	fn add_sentence_ignores_sentences_shorter_than_order() {
		let model = model_with(4, &["<a>"]);
		assert!(model.dump().is_empty());
	}

	#[test]
	fn add_sentence_accumulates_repeated_ngrams() {
		let model = model_with(2, &["<aa>", "<ab>"]);
		assert_eq!(model.dump(), vec![
			("<".to_owned(), vec![('a', 2)]),
			("a".to_owned(), vec![('>', 1), ('a', 1), ('b', 1)]),
			("b".to_owned(), vec![('>', 1)]),
		]);
	}

	#[test]
	fn add_sentence_handles_accented_characters() {
		let model = model_with(3, &["<Été>"]);
		assert_eq!(model.dump(), vec![
			("<é".to_owned(), vec![('t', 1)]),
			("té".to_owned(), vec![('>', 1)]),
			("ét".to_owned(), vec![('é', 1)]),
		]);
		assert_eq!(model.predict("ÉT"), Some('é'));
	}

	#[test]
	fn predict_ignores_prefix_case() {
		let model = model_with(3, &["<abc>"]);
		assert_eq!(model.predict("AB"), Some('c'));
		assert_eq!(model.predict("zz"), None);
	}

	#[test]
	fn get_random_seed_returns_a_known_prefix() {
		assert_eq!(NGramModel::new(3).unwrap().get_random_seed(), None);
		let model = model_with(3, &["<abc>"]);
		let seed = model.get_random_seed().expect("a model with states must give a seed");
		assert!(["<a", "ab", "bc"].contains(&seed.as_str()), "unexpected seed {}", seed);
	}

	#[test]
	fn merge_sums_counts_and_keeps_new_states() {
		let mut model = model_with(2, &["<ab>"]);
		model.merge(&model_with(2, &["<ab>", "<c>"])).unwrap();
		assert_eq!(model.dump(), vec![
			("<".to_owned(), vec![('a', 2), ('c', 1)]),
			("a".to_owned(), vec![('b', 2)]),
			("b".to_owned(), vec![('>', 2)]),
			("c".to_owned(), vec![('>', 1)]),
		]);
	}

	#[test]
	fn merge_rejects_different_orders() {
		let mut model = NGramModel::new(2).unwrap();
		assert!(model.merge(&NGramModel::new(3).unwrap()).is_err());
	}

	#[test]
	fn state_entry_stays_small() {
		// Each entry is a prefix and its state, stored millions of times
		let model = model_with(2, &["<a>"]);
		let (key, state) = model.states.iter().next().unwrap();
		let size = std::mem::size_of_val(key) + std::mem::size_of_val(state);
		assert!(size <= 40, "a state entry takes {} bytes", size);
	}
}
