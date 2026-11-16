use super::ngram_model::NGramModel;
use crate::io::{build_output_path, get_filename, read_file};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

pub(crate) const START_CHAR: char = '<';
pub(crate) const END_CHAR: char = '>';

/// Top-level n-gram model containing multiple n-gram sizes and metadata for sequence generation.
///
/// # Responsibilities
/// - Store `ngrams`: a map from n-gram size to its corresponding `NGramModel`.
/// - Track `sentences` already added to avoid duplicates.
/// - Maintain `name` of the model (from a file).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiGramModel {
	ngrams: HashMap<usize, NGramModel>,
	sentences: HashSet<String>,
	name: String
}

impl MultiGramModel {
	/// Creates a default, empty `MultiGramModel`.
	///
	/// # Returns
	/// - A `MultiGramModel` with:
	///   - `ngrams` initialized as an empty `HashMap`
	///   - `sentences` initialized as an empty `HashSet`
	///   - `name` set to an empty string
	///
	/// # Notes
	/// - Useful for creating a blank model that can later be filled or merged.
	/// - Does not load any data from disk; all fields are empty.
	/// - Not exposed
	pub(crate) fn default() -> Self {
		Self {
			ngrams: HashMap::new(),
			sentences: HashSet::new(),
			name: "".to_owned()
		}
	}

	/// Loads a `MultiGramModel` from a file if a binary exists,
	/// otherwise builds the model from the raw text file with multithreaded merging.
	///
	/// # Parameters
	/// - `filepath`: Input text file path.
	///
	/// # Returns
	/// - `Ok(MultiGramModel)` on success.
	/// - `Err` if I/O or deserialization fails.
	///
	/// # Notes
	/// - Uses a binary cache (`postcard`) for faster loading if available.
	pub fn new<P: AsRef<Path>>(filepath: P) -> Result<Self, Box<dyn std::error::Error>> {
		let binary_data_path = build_output_path(&filepath, "bin")?;
		let mut model;
		if binary_data_path.exists() {
			let bytes = std::fs::read(binary_data_path)?;
			model = postcard::from_bytes(&bytes)?;
		} else {
			model = Self::read_database_file(&filepath, binary_data_path)?;
		}
		model.name = get_filename(filepath)?;
		Ok(model)
	}

	/// Reads a raw text file, splits lines into chunks, builds partial models in parallel,
	/// merges them into a final `MultiGramModel`, and serializes it.
	fn read_database_file<PF, PB>(
		filename: PF,
		binary_data_path: PB,
	) -> Result<MultiGramModel, Box<dyn std::error::Error>>
	where
		PF: AsRef<Path>,
		PB: AsRef<Path>,
	{
		let lines = read_file(&filename)?;
		let cpus = num_cpus::get();
		let factor = 8;
		let chunks = cpus * factor;
		let chunk_size = (lines.len() + chunks - 1) / chunks;

		let (tx, rx) = mpsc::channel();
		for chunk in lines.chunks(chunk_size) {
			let tx = tx.clone();
			let chunk: Vec<String> = chunk.to_vec();

			thread::spawn(move || {
				let mut partial_model = MultiGramModel::default();
				for sentence in chunk {
					partial_model.add_sentence(&sentence);
				}
				tx.send(partial_model).expect("Failed to send from thread");
			});
		}
		drop(tx);

		let mut final_model = MultiGramModel::default();
		for partial_model in rx.iter() {
			final_model.merge(&partial_model)?;
		}

		let bytes = postcard::to_stdvec(&final_model)?;
		std::fs::write(binary_data_path, bytes)?;

		Ok(final_model)
	}

	/// Returns a random prefix (seed) from the n-gram model of order `n`.
	///
	/// # Returns
	/// - `Some(String)` if the model exists and has states.
	/// - `None` otherwise.
	pub fn get_random_seed(&self, n: usize) -> Option<String> {
		self.ngrams.get(&n)?.get_random_seed()
	}

	/// Returns the number of n-gram models stored.
	pub fn size(&self) -> usize {
		self.ngrams.len()
	}

	/// Returns the model's name.
	pub fn get_name(&self) -> &str {
		&self.name
	}



	/// Returns the last `n` characters of a string (UTF-8 safe).
	fn last_n_chars(s: &str, n: usize) -> String {
		if n > s.chars().count() {
			return s.to_owned();
		}
		s.chars().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect()
	}

	/// Adds a sentence to the model.
	///
	/// # Behavior
	/// - Prepends `START_CHAR` and appends `END_CHAR` if missing.
	/// - Adds sentence to `self.sentences`.
	/// - Updates all n-gram models, for n = 2 to len(sentence).
	/// - Creates new `NGramModel`s if needed.
	pub fn add_sentence(&mut self, sentence: &str) {
		let mut s = sentence.to_string();
		self.sentences.insert(sentence.to_owned());

		if !s.starts_with(START_CHAR) {
			s.insert(0, START_CHAR);
		}
		if !s.ends_with(END_CHAR) {
			s.push(END_CHAR);
		}

		for n in 2..=s.chars().count() {
			let model = self.ngrams.entry(n).or_insert_with(|| NGramModel::new(n).unwrap());
			model.add_sentence(&s);
		}
	}

	/// Predicts the next character for a given prefix.
	///
	/// # Parameters
	/// - `key`: the prefix string
	/// - `max_n`: maximum n-gram order to consider
	/// - `randomness`: randomness factor (0.0..1.0)
	/// - `reduce_random`: whether to reduce n on fallback
	///
	/// # Returns
	/// - `Ok(Some(char))` if a prediction is made
	/// - `Ok(None)` if no prediction is possible
	/// - `Err(String)` on invalid randomness calculation
	pub fn predict(
		&self,
		key: &str,
		n: usize
	) -> Result<Option<char>, String> {
		let key_len = key.chars().count();
		if key_len < n - 1 {
			return Ok(None);
		}
		let mut next_char = None;
		match self.ngrams.get(&n) {
			Some(model) => {
				let prefix = Self::last_n_chars(&key, n - 1);
				if let Some(c) = model.predict(&prefix) {
					next_char = Some(c);
				}
			},
			None => {}
		}

		Ok(next_char)
	}

	/// Checks if a word already exists in the model.
	pub fn check_if_exists(&self, word: &str) -> bool {
		self.sentences.iter().any(|value| value.to_lowercase() == word.to_lowercase())
	}

	/// Merges another `MultiGramModel` into this one.
	///
	/// # Behavior
	/// - Merges each n-gram model: existing models are merged in place; missing ones are cloned.
	/// - Adds all sentences from `other` to `self.sentences`.
	///
	/// # Returns
	/// - `Ok(())` on success
	/// - `Err(String)` if merging fails (should not occur under normal conditions)
	pub fn merge(&mut self, other: &Self) -> Result<(), String> {
		for (k, t) in &other.ngrams {
			if let Some(existing) = self.ngrams.get_mut(k) {
				existing.merge(t)?;
			} else {
				self.ngrams.insert(*k, t.clone());
			}
		}
		self.sentences.extend(other.sentences.clone());
		Ok(())
	}
}

#[cfg(test)]
impl MultiGramModel {
	/// Returns every state as (n, prefix, sorted transitions), sorted, to compare models in tests.
	pub(crate) fn dump(&self) -> Vec<(usize, String, Vec<(char, usize)>)> {
		let mut states: Vec<(usize, String, Vec<(char, usize)>)> = Vec::new();
		for (n, model) in &self.ngrams {
			for (prefix, transitions) in model.dump() {
				states.push((*n, prefix, transitions));
			}
		}
		states.sort();
		states
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	// Expected states of the word "ab", once wrapped as "<ab>"
	fn states_of_ab() -> Vec<(usize, String, Vec<(char, usize)>)> {
		vec![
			(2, "<".to_owned(), vec![('a', 1)]),
			(2, "a".to_owned(), vec![('b', 1)]),
			(2, "b".to_owned(), vec![('>', 1)]),
			(3, "<a".to_owned(), vec![('b', 1)]),
			(3, "ab".to_owned(), vec![('>', 1)]),
			(4, "<ab".to_owned(), vec![('>', 1)]),
		]
	}

	// Creates an empty directory dedicated to one test
	fn test_dir(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("rs-gen-core-{}-{}", std::process::id(), name));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		dir
	}

	#[test]
	fn add_sentence_builds_every_order() {
		let mut model = MultiGramModel::default();
		model.add_sentence("ab");
		assert_eq!(model.size(), 3);
		assert_eq!(model.dump(), states_of_ab());
	}

	#[test]
	fn add_sentence_keeps_existing_markers() {
		let mut model = MultiGramModel::default();
		model.add_sentence("<ab>");
		assert_eq!(model.dump(), states_of_ab());
	}

	#[test]
	fn check_if_exists_ignores_case() {
		let mut model = MultiGramModel::default();
		model.add_sentence("Paris");
		assert!(model.check_if_exists("paris"));
		assert!(model.check_if_exists("PARIS"));
		assert!(!model.check_if_exists("lyon"));
	}

	#[test]
	fn predict_uses_the_requested_order() {
		let mut model = MultiGramModel::default();
		model.add_sentence("abc");
		assert_eq!(model.predict("<ab", 3), Ok(Some('c')));
		assert_eq!(model.predict("<ab", 4), Ok(Some('c')));
		assert_eq!(model.predict("b", 3), Ok(None));
	}

	#[test]
	fn merge_combines_ngrams_and_sentences() {
		let mut model = MultiGramModel::default();
		model.add_sentence("ab");
		let mut other = MultiGramModel::default();
		other.add_sentence("ab");
		other.add_sentence("cd");
		model.merge(&other).unwrap();
		assert!(model.check_if_exists("cd"));
		let start: Vec<(usize, String, Vec<(char, usize)>)> = model.dump()
			.into_iter()
			.filter(|(n, prefix, _)| *n == 2 && prefix == "<")
			.collect();
		assert_eq!(start, vec![(2, "<".to_owned(), vec![('a', 2), ('c', 1)])]);
	}

	#[test]
	fn binary_cache_gives_back_the_same_model() {
		let dir = test_dir("cache-round-trip");
		let dat = dir.join("words.dat");
		std::fs::write(&dat, "abc\nabd\nAbe\n").unwrap();

		let built = MultiGramModel::new(&dat).unwrap();
		assert!(dir.join("words.bin").exists());
		let loaded = MultiGramModel::new(&dat).unwrap();

		assert_eq!(loaded.get_name(), "words");
		assert_eq!(loaded.dump(), built.dump());
		assert!(loaded.check_if_exists("abe"));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn accented_words_survive_the_binary_cache() {
		let dir = test_dir("accents");
		let dat = dir.join("villes.dat");
		std::fs::write(&dat, "Saint-Étienne\nÈze\nÉpinal\n").unwrap();

		let built = MultiGramModel::new(&dat).unwrap();
		let loaded = MultiGramModel::new(&dat).unwrap();

		assert_eq!(loaded.dump(), built.dump());
		assert!(loaded.check_if_exists("saint-étienne"));
		assert_eq!(loaded.predict("<èz", 4), Ok(Some('e')));
		let _ = std::fs::remove_dir_all(&dir);
	}
}
