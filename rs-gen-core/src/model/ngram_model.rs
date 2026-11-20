use super::state::State;
use crate::hash::{FastHashMap, FastHasher};
use rand::Rng;
use serde::de::{MapAccess, Visitor};
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::hash_map::Entry;
use std::hash::{BuildHasher, BuildHasherDefault};

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
/// # Memory
/// A model learns in a hash table, then is frozen once learned or loaded: its prefixes
/// are put one after the other in a single buffer, its states in a table of 32-byte
/// entries, found through a small open addressing index. On the eleven models of the
/// server, a state then takes 58 bytes instead of 101, prefix included. A frozen model
/// that learns again goes back to the hash table first.
///
/// # Invariants
/// - `n` is always >= 2
/// - Each prefix of length n-1 has one state, either in the learning table or in the
///   frozen table, never in both
/// - All state transitions have occurrence counts >= 1
#[derive(Clone, Debug)]
pub struct NGramModel {
	/// The order of the model (number of characters in the n-gram)
	n: usize, // must be >= 2

	/// States while the model learns: prefix (length n-1) to state.
	/// `Box<str>` holds the same UTF-8 text as a `String` (accents included),
	/// without its spare capacity.
	learning: FastHashMap<Box<str>, State>,

	/// States once the model is learned or loaded.
	frozen: Frozen,
}

/// Slot of the index without entry.
const EMPTY: u32 = u32::MAX;

/// States of a learned model: their prefixes in one buffer, found through an index.
#[derive(Clone, Debug, Default)]
struct Frozen {
	/// Every prefix, one after the other.
	prefixes: String,

	/// One entry per state.
	entries: Vec<FrozenEntry>,

	/// Entry numbers by hash of their prefix, `EMPTY` for a free slot. Its length is a
	/// power of two, with at most 70 % of the slots used, so that a lookup stops soon.
	index: Vec<u32>,
}

/// A state and the place of its prefix in the buffer: 32 bytes.
#[derive(Clone, Debug)]
struct FrozenEntry {
	/// Start of the prefix in `Frozen::prefixes`, in bytes.
	start: u32,

	/// Length of the prefix, in bytes.
	len: u32,

	/// The state.
	state: State,
}

/// Hash of a prefix, for the index.
fn hash_of(prefix: &str) -> usize {
	BuildHasherDefault::<FastHasher>::default().hash_one(prefix) as usize
}

impl Frozen {
	/// Freezes the states of a learning table. Returns them back if they are too many for
	/// the 32-bit offsets (over 4 GB of prefixes): the model then stays a hash table.
	fn from_learning(learning: FastHashMap<Box<str>, State>) -> Result<Self, FastHashMap<Box<str>, State>> {
		let prefix_bytes: usize = learning.keys().map(|prefix| prefix.len()).sum();
		if prefix_bytes >= u32::MAX as usize || learning.len() >= EMPTY as usize {
			return Err(learning);
		}
		let mut frozen = Self { prefixes: String::with_capacity(prefix_bytes), entries: Vec::with_capacity(learning.len()), index: Vec::new() };
		for (prefix, state) in learning {
			frozen.push(&prefix, state);
		}
		frozen.build_index();
		Ok(frozen)
	}

	/// Appends a state, before `build_index`.
	fn push(&mut self, prefix: &str, state: State) {
		let start = self.prefixes.len() as u32;
		self.prefixes.push_str(prefix);
		self.entries.push(FrozenEntry { start, len: prefix.len() as u32, state });
	}

	/// Prefix of an entry.
	fn prefix(&self, entry: &FrozenEntry) -> &str {
		&self.prefixes[entry.start as usize..(entry.start + entry.len) as usize]
	}

	/// Builds the index of the entries, once they are all pushed.
	fn build_index(&mut self) {
		self.entries.shrink_to_fit();
		self.prefixes.shrink_to_fit();
		let slots = (self.entries.len() * 10 / 7 + 1).next_power_of_two();
		let mask = slots - 1;
		let mut index = vec![EMPTY; slots];
		for (number, entry) in self.entries.iter().enumerate() {
			let mut slot = hash_of(self.prefix(entry)) & mask;
			while index[slot] != EMPTY {
				slot = (slot + 1) & mask;
			}
			index[slot] = number as u32;
		}
		self.index = index;
	}

	/// Returns the state of `prefix`, if any.
	fn get(&self, prefix: &str) -> Option<&State> {
		if self.entries.is_empty() {
			return None;
		}
		let mask = self.index.len() - 1;
		let mut slot = hash_of(prefix) & mask;
		// The index always has free slots: the lookup ends on one at worst
		loop {
			let number = self.index[slot];
			if number == EMPTY {
				return None;
			}
			let entry = &self.entries[number as usize];
			if self.prefix(entry) == prefix {
				return Some(&entry.state);
			}
			slot = (slot + 1) & mask;
		}
	}

	/// Every state with its prefix, in the order they were pushed.
	fn iter(&self) -> impl Iterator<Item = (&str, &State)> {
		self.entries.iter().map(|entry| (self.prefix(entry), &entry.state))
	}
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
		Ok(Self { n, learning: FastHashMap::default(), frozen: Frozen::default() })
	}

	/// Number of states.
	fn len(&self) -> usize {
		self.frozen.entries.len() + self.learning.len()
	}

	/// Every state with its prefix, frozen or learning.
	fn iter(&self) -> impl Iterator<Item = (&str, &State)> {
		self.frozen.iter().chain(self.learning.iter().map(|(prefix, state)| (prefix.as_ref(), state)))
	}

	/// Returns the state of `prefix`, if any.
	fn get(&self, prefix: &str) -> Option<&State> {
		self.frozen.get(prefix).or_else(|| self.learning.get(prefix))
	}

	/// Moves the frozen states back to the learning table, to learn again.
	fn thaw(&mut self) {
		if self.frozen.entries.is_empty() {
			return;
		}
		let frozen = std::mem::take(&mut self.frozen);
		self.learning.reserve(frozen.entries.len());
		for entry in &frozen.entries {
			self.learning.insert(frozen.prefix(entry).into(), entry.state.clone());
		}
	}

	/// Freezes the learned states: see the memory notes of `NGramModel`.
	fn freeze(&mut self) {
		self.thaw();
		match Frozen::from_learning(std::mem::take(&mut self.learning)) {
			Ok(frozen) => self.frozen = frozen,
			Err(learning) => self.learning = learning,
		}
	}

	/// Returns a random prefix (seed) from the current model.
	///
	/// Useful for starting a generation sequence.
	/// Returns `None` if the model has no states.
	pub fn get_random_seed(&self) -> Option<String> {
		let count = self.len();
		if count == 0 {
			return None;
		}
		let drawn = rand::rng().random_range(0..count);
		match self.frozen.entries.get(drawn) {
			Some(entry) => Some(self.frozen.prefix(entry).to_owned()),
			None => self.learning.keys().nth(drawn - self.frozen.entries.len()).map(std::string::ToString::to_string),
		}
	}

	/// Adds a sentence, word, or short phrase to the model.
	///
	/// Breaks the input into n-grams and updates states with observed transitions.
	///
	/// # Notes
	/// - Converts all characters to lowercase for consistency.
	/// - Ignores sentences shorter than `n`.
	/// - A frozen model goes back to its learning table first.
	pub fn add_sentence(&mut self, sentence: &str) {
		let chars: Vec<char> = sentence.chars().collect();
		if chars.len() < self.n {
			// Sentence too shorts, no n-grams to compute
			return;
		}
		self.thaw();

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
			match self.learning.get_mut(prefix.as_str()) {
				Some(state) => state.add_transition(next_char),
				None => {
					let mut state = State::new();
					state.add_transition(next_char);
					self.learning.insert(prefix.into_boxed_str(), state);
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
			self.get(prefix)
		} else {
			let key: String = prefix
				.chars()
				.flat_map(char::to_lowercase)
				.collect();
			self.get(key.as_str())
		};
		state?.predict()
	}

	/// Merges another n-gram model into this one.
	///
	/// # Notes
	/// - Both models must have the same order `n`.
	/// - Occurrence counts for matching states and transitions are summed.
	/// - The merged model is frozen.
	///
	/// # Errors
	/// Returns an error if the model orders do not match.
	pub fn merge(&mut self, other: &Self) -> Result<(), String> {
		if self.n != other.n {
			return Err("N mismatch".to_owned());
		}
		self.thaw();
		for (prefix, state) in other.iter() {
			if let Some(existing) = self.learning.get_mut(prefix) {
				existing.merge(state);
			} else {
				self.learning.insert(prefix.into(), state.clone());
			}
		}
		self.freeze();
		Ok(())
	}

	/// Merges another n-gram model of the same order into this one, moving its states
	/// instead of copying them.
	pub(crate) fn merge_from(&mut self, mut other: Self) {
		debug_assert_eq!(self.n, other.n);
		self.thaw();
		other.thaw();
		for (prefix, state) in other.learning {
			match self.learning.entry(prefix) {
				Entry::Occupied(mut entry) => entry.get_mut().merge(&state),
				Entry::Vacant(entry) => {
					entry.insert(state);
				}
			}
		}
	}

	/// Freezes the model once learning is over: see the memory notes of `NGramModel`.
	pub(crate) fn shrink_to_fit(&mut self) {
		self.freeze();
	}

	/// Returns the number of states and the UTF-8 bytes of their prefixes.
	pub(crate) fn states_and_key_bytes(&self) -> (usize, usize) {
		(self.len(), self.iter().map(|(prefix, _)| prefix.len()).sum())
	}

	/// Returns the number of transitions over every state.
	pub(crate) fn transition_count(&self) -> usize {
		self.iter().map(|(_, state)| state.transition_count()).sum()
	}
}

/// The states as the binary cache stores them: a map from prefix to state, as when the
/// model was a single hash table, so that the caches stay readable both ways.
struct StatesMap<'a>(&'a NGramModel);

impl Serialize for StatesMap<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut map = serializer.serialize_map(Some(self.0.len()))?;
		for (prefix, state) in self.0.iter() {
			map.serialize_entry(prefix, state)?;
		}
		map.end()
	}
}

impl Serialize for NGramModel {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut model = serializer.serialize_struct("NGramModel", 2)?;
		model.serialize_field("n", &self.n)?;
		model.serialize_field("states", &StatesMap(self))?;
		model.end()
	}
}

/// Reads the map of the cache straight into a frozen table, without a hash table between.
struct FrozenVisitor;

impl<'de> Visitor<'de> for FrozenVisitor {
	type Value = Frozen;

	fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
		formatter.write_str("a map from prefix to state")
	}

	fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Frozen, A::Error> {
		let mut frozen = Frozen::default();
		if let Some(count) = map.size_hint() {
			frozen.entries.reserve_exact(count);
		}
		while let Some((prefix, state)) = map.next_entry::<String, State>()? {
			if frozen.prefixes.len() + prefix.len() >= u32::MAX as usize || frozen.entries.len() + 1 >= EMPTY as usize {
				return Err(serde::de::Error::custom("too many prefixes for an n-gram model"));
			}
			frozen.push(&prefix, state);
		}
		frozen.build_index();
		Ok(frozen)
	}
}

/// Fields of an `NGramModel` in the binary cache.
#[derive(Deserialize)]
struct StoredModel {
	/// The order of the model.
	n: usize,

	/// Its states.
	#[serde(deserialize_with = "deserialize_frozen")]
	states: Frozen,
}

fn deserialize_frozen<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Frozen, D::Error> {
	deserializer.deserialize_map(FrozenVisitor)
}

impl<'de> Deserialize<'de> for NGramModel {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let stored = StoredModel::deserialize(deserializer)?;
		Ok(Self { n: stored.n, learning: FastHashMap::default(), frozen: stored.states })
	}
}

#[cfg(test)]
impl NGramModel {
	/// Returns every state as (prefix, sorted transitions), sorted by prefix, to compare models in tests.
	pub(crate) fn dump(&self) -> Vec<(String, Vec<(char, usize)>)> {
		let mut states: Vec<(String, Vec<(char, usize)>)> = self
			.iter()
			.map(|(prefix, state)| (prefix.to_string(), state.sorted_transitions()))
			.collect();
		states.sort();
		states
	}

	/// Tells whether the model is frozen.
	fn is_frozen(&self) -> bool {
		self.learning.is_empty() && !self.frozen.entries.is_empty()
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

	// The same model, frozen
	fn frozen_with(n: usize, sentences: &[&str]) -> NGramModel {
		let mut model = model_with(n, sentences);
		model.shrink_to_fit();
		assert!(model.is_frozen());
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
		assert!(["<a", "ab", "bc"].contains(&seed.as_str()), "unexpected seed {seed}");
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
		// Each entry is the place of a prefix and its state, stored millions of times
		assert!(size_of::<FrozenEntry>() <= 32, "a state entry takes {} bytes", size_of::<FrozenEntry>());
	}

	// Words sharing many prefixes, for the index
	fn many_words() -> Vec<String> {
		(0..3000).map(|i| format!("<w{}x{}é{}>", i % 37, i % 101, i % 7)).collect()
	}

	#[test]
	fn a_frozen_model_holds_and_finds_the_same_states() {
		let words = many_words();
		let words: Vec<&str> = words.iter().map(String::as_str).collect();
		let learning = model_with(4, &words);
		let frozen = frozen_with(4, &words);
		assert_eq!(frozen.dump(), learning.dump());
		assert_eq!(frozen.states_and_key_bytes(), learning.states_and_key_bytes());
		assert_eq!(frozen.transition_count(), learning.transition_count());
		// Every prefix is found, with the same state
		for (prefix, transitions) in learning.dump() {
			assert_eq!(frozen.get(&prefix).map(State::sorted_transitions), Some(transitions), "{prefix}");
		}
		// And the others are not
		for absent in ["zzz", "<w9x", "é0>", "", "<w1x1é1>"] {
			assert!(learning.get(absent).is_none(), "{absent} is a prefix of the model");
			assert!(frozen.get(absent).is_none(), "{absent} found in the frozen model");
		}
	}

	#[test]
	fn a_frozen_model_predicts_and_seeds_like_the_learning_one() {
		let frozen = frozen_with(3, &["<AbC>"]);
		assert_eq!(frozen.predict("AB"), Some('c'));
		assert_eq!(frozen.predict("<a"), Some('b'));
		assert_eq!(frozen.predict("zz"), None);
		let seed = frozen.get_random_seed().unwrap();
		assert!(["<a", "ab", "bc"].contains(&seed.as_str()), "unexpected seed {seed}");
		// An empty model has nothing to freeze nor to find
		let mut empty = NGramModel::new(3).unwrap();
		empty.shrink_to_fit();
		assert_eq!(empty.predict("ab"), None);
		assert_eq!(empty.get_random_seed(), None);
	}

	#[test]
	fn a_frozen_model_learns_again() {
		let mut model = frozen_with(3, &["<abc>"]);
		model.add_sentence("<abd>");
		assert!(!model.is_frozen());
		assert_eq!(model.dump(), model_with(3, &["<abc>", "<abd>"]).dump());
		model.shrink_to_fit();
		assert!(model.is_frozen());
		assert_eq!(model.dump(), model_with(3, &["<abc>", "<abd>"]).dump());
	}

	#[test]
	fn merge_works_on_frozen_models() {
		let mut model = frozen_with(2, &["<ab>"]);
		model.merge(&frozen_with(2, &["<ab>", "<c>"])).unwrap();
		assert!(model.is_frozen());
		assert_eq!(model.dump(), model_with(2, &["<ab>", "<ab>", "<c>"]).dump());
	}

	// The layout of the cache before the frozen table: a hash table of states
	#[derive(Serialize, Deserialize)]
	struct HashTableModel {
		n: usize,
		states: FastHashMap<Box<str>, State>,
	}

	#[test]
	fn the_cache_layout_is_the_one_of_the_hash_table() {
		let words = many_words();
		let words: Vec<&str> = words.iter().map(String::as_str).collect();
		let learning = model_with(4, &words);
		// A cache written by the hash table is read into a frozen model
		let old = postcard::to_stdvec(&HashTableModel { n: 4, states: learning.learning.clone() }).unwrap();
		let read: NGramModel = postcard::from_bytes(&old).unwrap();
		assert!(read.is_frozen());
		assert_eq!(read.dump(), learning.dump());
		// And a frozen model writes a cache the hash table reads
		let new = postcard::to_stdvec(&read).unwrap();
		let back: HashTableModel = postcard::from_bytes(&new).unwrap();
		assert_eq!(back.n, 4);
		assert_eq!(back.states.len(), learning.learning.len());
		for (prefix, state) in &back.states {
			assert_eq!(state.sorted_transitions(), learning.learning[prefix].sorted_transitions());
		}
	}
}
