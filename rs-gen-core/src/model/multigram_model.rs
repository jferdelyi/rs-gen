use std::cmp::{max, min};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use rand::Rng;

use serde::{Deserialize, Serialize};
use crate::io::{build_output_path, get_filename, read_file};
use super::ngram_model::NGramModel;

/// Enum representing the strategy for selecting the starting seed
/// when generating a new sequence from the n-gram model.
///
/// # Variants
/// - `Random(usize)`: pick a random starting prefix from the n-gram of size `usize`.
/// - `Custom(&str)`: use the provided string as the initial seed.
/// - `False`: do not use any seed, start from an empty.
#[derive(PartialEq)]
pub enum StartSeed<'a> {
	Random(usize),
	Custom(&'a str),
	False,
}

/// The top-level n-gram model containing multiple n-gram sizes and
/// metadata for generating sequences.
///
/// This struct manages:
/// - `ngrams`: a map from n-gram size to its corresponding `NGramModel`.
/// - `start_char` and `end_char`: special characters used to mark sentence boundaries.
/// - `sentences`: a set of all input sentences added to the model (used to avoid re-generating existing ones).
/// - `model_names`: name of models files loaded from disk.
#[derive(Serialize, Deserialize, Debug)]
pub struct MultiGramModel {
	start_char: char,
	end_char: char,
	ngrams: HashMap<usize, NGramModel>,
	sentences: HashSet<String>,
	model_names: Vec<String>
}

impl MultiGramModel {
	/// Returns a default, empty `GlobalNGramModel`.
	///
	/// Initializes:
	/// - `ngrams` as an empty HashMap
	/// - `start_char` as '<' and `end_char` as '>'
	/// - `sentences` as an empty HashSet
	///
	/// Useful for creating a blank model that can then be filled or merged.
	pub fn default() -> Self {
		Self { ngrams: HashMap::new(), start_char: '<', end_char: '>', sentences: HashSet::new(), model_names: Vec::new() }
	}

	/// Loads a `GlobalNGramModel` from a file if a binary exists,
	/// otherwise builds the model by reading the raw file and performing multithreaded merging.
	///
	/// - `filepath` is the input text file.
	/// - Checks if a binary file exists for fast loading.
	/// - Uses `postcard` for compact serialization/deserialization.
	/// - Calls `multi_merge` if binary does not exist.
	pub fn new<P: AsRef<Path>>(filepath: P) -> Result<Self, Box<dyn std::error::Error>> {
		let binary_data_path = build_output_path(&filepath, "bin")?;
		let mut model;
		if binary_data_path.exists() {
			let bytes = std::fs::read(binary_data_path)?;
			model = postcard::from_bytes(&bytes)?;
		} else {
			model = Self::read_database_file(&filepath, binary_data_path)?;
		}
		model.model_names.push(get_filename(&filepath)?);
		Ok(model)
	}

	/// Returns the list of loaded model names.
	///
	/// This function provides a **read-only reference** to the internal list
	/// of model names currently associated with this instance.
	///
	/// The returned vector contains model identifiers as strings, typically
	/// corresponding to model file names (without path).
	///
	/// # Returns
	///
	/// A reference to a vector of model names.
	///
	/// # Notes
	///
	/// - The returned reference is immutable; callers cannot modify the internal state.
	/// - The order of the model names is preserved as stored internally.
	pub fn get_model_names(&self) -> &Vec<String> {
		&self.model_names
	}

	/// Reads a file, splits its lines into chunks, creates partial models in parallel,
	/// merges all partial models into a final `GlobalNGramModel`, and serializes it.
	///
	/// # Parameters
	/// - `filename`: Input text file.
	/// - `binary_data_path`: Output path for the serialized binary model.
	///
	/// # Returns
	/// - `Ok(GlobalNGramModel)`: The merged and serialized model.
	/// - `Err(...)`: If file I/O or merging fails.
	///
	/// # Behavior
	/// - Splits input lines into chunks (based on CPU cores * factor).
	/// - Spawns threads to build partial models for each chunk.
	/// - Merges all partial models sequentially.
	/// - Serializes the final model to `binary_data_path` for future fast loading.
	///
	/// # Notes
	/// - Uses MPSC channels to collect models from threads.
	/// - Threads use `add_sentence` for each line.
	/// - `unwrap()` is safe because `NGramModel::new(n)` always succeeds for n >= 2.
	fn read_database_file<PF, PB>(filename: PF, binary_data_path: PB) -> Result<MultiGramModel, Box<dyn std::error::Error>>
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

	/// Adds a sentence to the global n-gram model.
	///
	/// # Parameters
	/// - `sentence`: The input sentence to add.
	///
	/// # Behavior
	/// - Prepends `start_char` and appends `end_char` if missing.
	/// - Adds sentence to `self.sentences` (duplicates are ignored).
	/// - Updates all n-gram models, for n = 2 to len(sentence).
	///
	/// # Notes
	/// - UTF-8 safe: uses character counts.
	/// - Creates new `NGramModel`s if needed for a given n.
	/// - Uses `unwrap()` on `NGramModel::new`, safe because n >= 2.
	pub fn add_sentence(&mut self, sentence: &str) {
		let mut s = sentence.to_string();

		// Avoid duplicates
		if !self.sentences.insert(sentence.to_owned()) {
			return;
		}

		if !s.starts_with(self.start_char) {
			s.insert(0, self.start_char);
		}

		if !s.ends_with(self.end_char) {
			s.push(self.end_char);
		}

		// For each n-gram
		for n in 2..=s.chars().count() {
			// Impossible to panic, n-grams are initialized >= 2
			let model = self.ngrams.entry(n).or_insert_with(|| NGramModel::new(n).unwrap());
			model.add_sentence(&s);
		}
	}

	/// Returns the last `n` characters of a string.
	///
	/// # Arguments
	/// - `s`: The input string (UTF-8)
	/// - `n`: Number of characters to return
	///
	/// # Returns
	/// A new `String` containing the last `n` characters of `s`.
	/// If `n` is greater than the number of characters in `s`, the entire string is returned.
	///
	/// # Notes
	/// - Handles UTF-8 correctly (multibyte characters).
	fn last_n_chars(s: &str, n: usize) -> String {
		if n > s.chars().count() {
			return s.to_owned();
		}
		s.chars()
			.rev()
			.take(n)
			.collect::<Vec<_>>()
			.into_iter()
			.rev()
			.collect()
	}

	/// Generates a sequence (word or sentence) from the global n-gram model.
	///
	/// This is the internal function that actually performs the generation logic.
	/// It selects the starting prefix according to `start_seed` and iteratively
	/// chooses the next character based on n-gram probabilities, optionally
	/// applying randomness and reducing randomness at each step.
	///
	/// # Parameters
	/// - `max_n`: Maximum n-gram size to consider (2 or higher). If < 2, ignored.
	/// - `randomness`: Float [0.0, 1.0], probability to select a shorter n-gram randomly.
	/// - `reduce_random`: If true, recompute randomness during reduction too.
	/// - `start_seed`: Reference to `StartSeed` controlling the starting prefix.
	///
	/// # Returns
	/// - `Ok(String)`: The generated sequence (may include start/end characters internally).
	/// - `Err(String)`: If parameters are invalid or no n-grams are present.
	///
	/// # Notes
	/// - UTF-8 safe: iterates over characters, not bytes.
	/// - Handles `Random`, `Custom`, and `False` seeds.
	fn internal_predict(&mut self, max_n: usize, randomness: f32, reduce_random: bool, start_seed: &StartSeed) -> Result<String, String> {
		if randomness < 0.0 || randomness > 1.0 {
			return Err("Randomness must be between 0.0 and 1.0".to_owned());
		}
		if self.ngrams.keys().len() == 0 {
			return Err("No n-gram models available for prediction".to_owned());
		}

		let max_n: usize = if max_n < 2 { 0 } else { max_n };

		let mut word = String::new();
		let mut global_prefix = String::new();

		match start_seed {
			StartSeed::False => (),
			StartSeed::Random(random) => {
				let n: usize = if *random <= 0 {
					// <Salut> 7 elements
					// min "<" -> 'S' 		(2Gram)
					// max "<Salut" -> '>'	(7Gram)
					// If size = 1 then min (and max) NGram is 2
					// If size = 6 then min is 2 and max is 7
					// So 2..n+1
					rand::rng().random_range(2..=self.ngrams.keys().len() + 1)
				} else {
					max(2, min(*random, self.ngrams.keys().len() + 1))
				};

				// Should not panic
				word = self.ngrams.get(&n).unwrap().get_random_seed().unwrap_or_default();
				if word.starts_with(self.start_char) {
					word.remove(0);
				}
				global_prefix = word.clone();
			}
			StartSeed::Custom(s) => {
				word = (*s).to_owned();
				if word.starts_with(self.start_char) {
					word.remove(0);
				}
				global_prefix = word.clone()
			}
		}

		if !global_prefix.starts_with(self.start_char) {
			global_prefix.insert(0, self.start_char);
		}

		loop {

			let global_len = global_prefix.chars().count();

			// Select n
			// If the prefix is "<Salut>", then, size is 7
			// n is NGram model to select where n = prefix + next (ex. "<" -> 'S' is 2Gram)
			// So NGram model to select is n = prefix + 1
			let mut n = if max_n == 0 {
				global_len + 1
			} else {
				max_n.min(global_len + 1)
			};

			// If Randomness
			n = Self::compute_randomness(randomness, max_n, n)?;

			// Compute the next character
			let mut next_char = None;
			while n > 1 {
				if let Some(model) = self.ngrams.get(&n) {
					if reduce_random {
						// Next 'n' while be between 2 and n
						n = Self::compute_randomness(randomness, n - 1, n)?;
					}
					let prefix = Self::last_n_chars(&global_prefix, n - 1);
					if let Some(c) = model.predict(&prefix) {
						next_char = Some(c);
						break;
					}
				}
				n -= 1;
			}

			// Apply
			match next_char {
				Some(c) if c != self.end_char => {
					word.push(c);
					global_prefix.push(c);
				}
				_ => break,
			}
		}
		Ok(word)
	}

	/// Generates a sequence from the n-gram model, avoiding duplicates if possible.
	///
	/// This function wraps `internal_predict` and ensures that the generated sequence
	/// does not match an existing dataset in `self.sentences` up to `nb_try` attempts.
	///
	/// # Parameters
	/// - `max_n`: Maximum n-gram size to consider for sequence generation.
	/// - `nb_try`: Number of attempts to avoid generating an existing sentence.
	/// - `randomness`: Float [0.0, 1.0], probability to pick a shorter n-gram randomly.
	/// - `reduce_random`: If true, recompute randomness during reduction too.
	/// - `start_seed`: Reference to a `StartSeed` controlling the starting prefix.
	///
	/// # Returns
	/// - `Ok(String)`: A newly generated sequence, or the best effort after `nb_try` attempts.
	/// - `Err(String)`: If parameters are invalid or the model has no n-grams.
	///
	/// # Behavior
	/// - Calls `internal_predict` to generate sequences.
	/// - Checks if the result already exists (case-insensitive) in `self.sentences`.
	/// - Retries up to `nb_try` times if a duplicate is found.
	/// - Resets `self.try_number` at the end to avoid side effects.
	///
	/// # Notes
	/// - Case-insensitive comparison uses `to_ascii_lowercase`.
	/// - If `nb_try` is 0, returns the first generated sequence.
	/// - Does not modify the underlying n-gram models.
	pub fn predict(&mut self, max_n: usize, mut nb_try: usize, randomness: f32, reduce_random: bool, start_seed: &StartSeed) -> Result<String, String> {
		let mut word = self.internal_predict(max_n, randomness, reduce_random, &start_seed)?;

		loop {
			let mut exists = false;
			if nb_try > 0 {
				exists = self.sentences.iter().find(|value| { *value.to_ascii_lowercase() == word.to_ascii_lowercase() }).is_some();
				if exists {
					word = self.internal_predict(max_n, randomness, reduce_random, &start_seed)?;
					nb_try -= 1;
				}
			}
			if !exists || nb_try <= 0 {
				break;
			}
		}
		Ok(word)
	}

	/// Computes a possibly randomized n-gram size based on `randomness`.
	///
	/// # Parameters
	/// - `randomness`: Float [0.0, 1.0], probability to pick a new n randomly.
	/// - `max_n`: Maximum length of the current prefix to consider (included).
	/// - `n`: Default n if no randomness is applied.
	///
	/// # Returns
	/// - `Ok(usize)`: Either the original `n` or a random value in [2, max_n].
	/// - `Err(String)`: If `randomness` is invalid.
	///
	/// # Notes
	/// - Randomness is applied with probability `randomness`.
	/// - Ensures returned n-gram size is at least 2.
	fn compute_randomness(randomness: f32, max_n: usize, n: usize) -> Result<usize, String> {
		if randomness < 0.0 || randomness > 1.0 {
			Err(format!("randomness must be between 0.0 and 1.0, got {}", randomness))?;
		}
		if randomness > 0.0 && rand::rng().random_range(0.0..=1.0) <= randomness {
			return Ok(rand::rng().random_range(2..=max_n.max(2)));
		}
		Ok(n)
	}

	/// Merges another `GlobalNGramModel` into this one.
	///
	/// # Parameters
	/// - `other`: Reference to another `GlobalNGramModel` to merge.
	///
	/// # Returns
	/// - `Ok(())`: Successfully merged.
	/// - `Err(String)`: If start or end characters of the two models do not match.
	///
	/// # Behavior
	/// - Merges each n-gram model: existing models are merged in place; missing ones are cloned.
	/// - Adds all sentences from `other` to `self.sentences`.
	///
	/// # Notes
	/// - Start and end characters are assumed immutable; mismatch causes an error.
	/// - Clones n-grams and sentences from `other`.
	pub fn merge(&mut self, other: &Self) -> Result<(), String> {
		if self.start_char != other.start_char || self.end_char != other.end_char {
			return Err(format!(
				"Start/end char mismatch: self=({}-{}), other=({}-{})",
				self.start_char, self.end_char, other.start_char, other.end_char
			));
		}

		for (k, t) in &other.ngrams {
			if let Some(existing) = self.ngrams.get_mut(k) {
				existing.merge(t)?;
			} else {
				self.ngrams.insert(k.clone(), t.clone());
			}
		}

		self.sentences.extend(other.sentences.clone());
		self.model_names.extend(other.model_names.clone());

		Ok(())
	}
}
