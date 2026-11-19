use super::ngram_model::NGramModel;
use crate::hash::{FastHashMap, FastHashSet};
use crate::io::{build_output_path, get_filename};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::Entry;
use std::fs::File;
use std::hash::{DefaultHasher, Hasher};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

pub(crate) const START_CHAR: char = '<';
pub(crate) const END_CHAR: char = '>';

/// Header written at the start of the binary cache.
/// Bump its version whenever the serialized layout changes: a cache with another
/// header is rebuilt from the text file instead of being misread.
const CACHE_HEADER: &[u8] = b"RSGEN-CACHE-4\n";

/// Buffer that holds one string at a time while the cache is decoded: far longer than
/// any prefix or sentence. A longer string makes the cache unreadable, hence rebuilt.
const CACHE_SCRATCH_BYTES: usize = 64 * 1024;

/// Lines learned between two updates of a `LearnProgress`: the shared counter is touched
/// once per batch, not once per line.
const PROGRESS_STEP: usize = 1024;

/// Progress of `MultiGramModel::learn_file`, readable from another thread while it runs.
#[derive(Debug, Default)]
pub struct LearnProgress {
	/// Lines learned so far.
	done: AtomicUsize,

	/// Lines of the text file, 0 until the file is read.
	total: AtomicUsize,
}

impl LearnProgress {
	/// Returns the lines learned so far and the lines of the text file (0 until it is read).
	pub fn lines(&self) -> (usize, usize) {
		(self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
	}
}

/// Size of a model, as built from its text file.
///
/// `MultiGramModel::stats` reads it from a model, and `MultiGramModel::count_text`
/// gives the same values from a text, without building the model.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelStats {
	/// Number of distinct sentences, compared in lowercase.
	pub sentences: usize,

	/// UTF-8 bytes of the distinct sentences, stored lowercase.
	pub sentence_bytes: usize,

	/// Highest n-gram order, 0 for an empty model.
	pub max_n: usize,

	/// Number of states (prefixes) over every order.
	pub states: usize,

	/// UTF-8 bytes of the state prefixes.
	pub key_bytes: usize,
}

impl ModelStats {
	/// Memory of a state of a frozen model besides the bytes of its prefix: its 32-byte
	/// entry, its slots in the index and its extra transitions, on average.
	const BYTES_PER_STATE: u64 = 46;

	/// Memory of a state of a model that learns, in its hash table, besides the bytes of its
	/// prefix: its slot in the table, the allocation of its prefix and its extra transitions.
	const BYTES_PER_LEARNING_STATE: u64 = 88;

	/// Memory of a sentence besides its bytes: its slot in the table and its allocation.
	const BYTES_PER_SENTENCE: u64 = 48;

	/// Estimated memory of the loaded model, frozen, in bytes.
	///
	/// Calibrated on a Linux server (musl allocator) holding 11 models: 5.9 million frozen
	/// states took 342 MB, about 44 bytes per state besides the prefixes and the sentences,
	/// and 45 for `french` alone. An estimate, not a measure: the system allocator of macOS
	/// takes about 20 % more. Right after a model is learned in the process, it takes about
	/// 1.4 times this memory until the process restarts: the allocator keeps the memory that
	/// the hash table of the learning freed.
	pub fn estimated_bytes(&self) -> u64 {
		self.bytes_with(Self::BYTES_PER_STATE)
	}

	/// Estimated memory of the model while it learns, in its hash table, in bytes.
	///
	/// Calibrated on the same server: 5.9 million states in hash tables took 581 MB,
	/// about 84 bytes per state besides the prefixes.
	fn learning_bytes(&self) -> u64 {
		self.bytes_with(Self::BYTES_PER_LEARNING_STATE)
	}

	/// Memory of the states at `bytes_per_state` each, with their prefixes and the sentences.
	fn bytes_with(&self, bytes_per_state: u64) -> u64 {
		self.states as u64 * bytes_per_state
			+ self.key_bytes as u64
			+ self.sentences as u64 * Self::BYTES_PER_SENTENCE
			+ self.sentence_bytes as u64
	}

	/// Estimated peak memory while `MultiGramModel::new` builds the model
	/// from a text file of `text_bytes` bytes, in bytes.
	///
	/// A model learns in a hash table and is frozen only at the end: the peak follows the
	/// hash table. Measured on real models: the peak reached 1.2 to 1.3 times the hash table
	/// on one processor (learning `french` on the server peaked at 26.4 MB for a table of
	/// 24.8 MB), and 2.25 to 2.4 times on ten, where the partial models built in parallel are
	/// merged into the final one.
	pub fn estimated_build_bytes(&self, text_bytes: u64) -> u64 {
		let model = if num_cpus::get() == 1 {
			self.learning_bytes() * 3 / 2
		} else {
			self.learning_bytes() * 5 / 2
		};
		// The text, and one slice per line pointing into it
		model + text_bytes * 4
	}
}

/// What a text file would give once learned, from `MultiGramModel::count_text`.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextCount {
	/// Number of lines.
	pub lines: usize,

	/// Number of empty lines (each one is learned as an empty word).
	pub empty_lines: usize,

	/// Length of the longest line, in characters.
	pub max_line_chars: usize,

	/// Size of the model that the text would build, partial when `complete` is false.
	pub stats: ModelStats,

	/// False when the count stopped at its state limit.
	pub complete: bool,
}

/// Top-level n-gram model containing multiple n-gram sizes and metadata for sequence generation.
///
/// # Responsibilities
/// - Store `ngrams`: a map from n-gram size to its corresponding `NGramModel`.
/// - Track `sentences` already added to avoid duplicates.
/// - Maintain `name` of the model (from a file).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MultiGramModel {
	ngrams: FastHashMap<usize, NGramModel>,
	sentences: FastHashSet<Box<str>>,
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
			ngrams: FastHashMap::default(),
			sentences: FastHashSet::default(),
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
	/// - A cache without the current `CACHE_HEADER`, or a damaged one, is rebuilt from the text file.
	/// - Runs `from_cache`, or else `learn_file` then `save_cache`: these steps can also be
	///   called one by one, to tell which one is running.
	pub fn new<P: AsRef<Path>>(filepath: P) -> Result<Self, Box<dyn std::error::Error>> {
		if let Some(model) = Self::from_cache(&filepath)? {
			return Ok(model);
		}
		// No cache, a cache written with another layout, or a damaged one
		let model = Self::learn_file(&filepath, &LearnProgress::default())?;
		model.save_cache(&filepath)?;
		Ok(model)
	}

	/// Reads the model of the text file `filepath` from its binary cache, the `.bin` file next to it.
	///
	/// # Returns
	/// - `Ok(None)` if there is no cache, or a cache written with another layout, or a damaged one.
	/// - `Err` if the path has no file name.
	pub fn from_cache<P: AsRef<Path>>(filepath: P) -> Result<Option<Self>, Box<dyn std::error::Error>> {
		let Some(mut model) = Self::read_cache(&build_output_path(&filepath, "bin")?) else {
			return Ok(None);
		};
		model.name = get_filename(filepath)?;
		Ok(Some(model))
	}

	/// Builds the model of the text file `filepath` from its lines, without reading or writing the cache.
	///
	/// `progress` counts the lines learned, to be read from another thread while the model is built.
	///
	/// # Errors
	/// Returns an error if the file cannot be read.
	pub fn learn_file<P: AsRef<Path>>(filepath: P, progress: &LearnProgress) -> Result<Self, Box<dyn std::error::Error>> {
		let mut model = Self::read_database_file(&filepath, progress)?;
		model.name = get_filename(filepath)?;
		Ok(model)
	}

	/// Writes the binary cache of the model next to its text file `filepath`, as the `.bin` file
	/// that `from_cache` reads.
	///
	/// # Errors
	/// Returns an error if the cache cannot be written.
	pub fn save_cache<P: AsRef<Path>>(&self, filepath: P) -> Result<(), Box<dyn std::error::Error>> {
		self.write_cache(&build_output_path(&filepath, "bin")?)
	}

	/// Reads the binary cache, streamed from the file: the cache is never held whole in memory.
	///
	/// Returns `None` if there is no cache, or if it was written with another layout or is damaged.
	fn read_cache(path: &Path) -> Option<Self> {
		let mut reader = BufReader::new(File::open(path).ok()?);
		let mut header = [0u8; CACHE_HEADER.len()];
		reader.read_exact(&mut header).ok()?;
		if header != CACHE_HEADER {
			return None;
		}
		// Holds one string at a time while it is decoded: a prefix, a sentence or the name
		let mut scratch = vec![0u8; CACHE_SCRATCH_BYTES];
		postcard::from_io((reader, scratch.as_mut_slice())).ok().map(|(model, _)| model)
	}

	/// Writes the binary cache, streamed to a temporary file renamed once complete:
	/// an interrupted write never leaves a damaged cache behind.
	fn write_cache(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
		let temporary = path.with_extension("bin.tmp");
		let mut writer = BufWriter::new(File::create(&temporary)?);
		writer.write_all(CACHE_HEADER)?;
		let mut writer = postcard::to_io(self, writer)?;
		writer.flush()?;
		drop(writer);
		std::fs::rename(&temporary, path)?;
		Ok(())
	}

	/// Reads a raw text file and learns its lines, in parallel when the machine has
	/// several processors: one partial model per processor, merged into the final model.
	fn read_database_file<P: AsRef<Path>>(filename: P, progress: &LearnProgress) -> Result<MultiGramModel, Box<dyn std::error::Error>> {
		let text = std::fs::read_to_string(filename)?;
		let lines: Vec<&str> = text.lines().collect();
		progress.total.store(lines.len(), Ordering::Relaxed);
		// More partial models than processors would only repeat the prefixes they share
		let threads = num_cpus::get().clamp(1, lines.len().max(1));

		let mut final_model = if threads == 1 {
			Self::learn(&lines, progress)
		} else {
			let chunk_size = lines.len().div_ceil(threads);
			thread::scope(|scope| {
				let learners: Vec<_> = lines.chunks(chunk_size).map(|chunk| scope.spawn(move || Self::learn(chunk, progress))).collect();
				let mut partial_models = learners.into_iter().map(|learner| learner.join().expect("a learning thread panicked"));
				let mut final_model = partial_models.next().unwrap_or_else(MultiGramModel::default);
				for partial_model in partial_models {
					final_model.merge_from(partial_model);
				}
				final_model
			})
		};
		final_model.shrink_to_fit();
		Ok(final_model)
	}

	/// Learns every line into a new model, adding the lines learned to `progress` batch by batch.
	fn learn(lines: &[&str], progress: &LearnProgress) -> Self {
		let mut model = Self::default();
		for batch in lines.chunks(PROGRESS_STEP) {
			for line in batch {
				model.add_sentence(line);
			}
			progress.done.fetch_add(batch.len(), Ordering::Relaxed);
		}
		model
	}

	/// Merges another model into this one, moving its states instead of copying them.
	fn merge_from(&mut self, other: Self) {
		for (n, model) in other.ngrams {
			match self.ngrams.entry(n) {
				Entry::Occupied(mut entry) => entry.get_mut().merge_from(model),
				Entry::Vacant(entry) => {
					entry.insert(model);
				}
			}
		}
		self.sentences.extend(other.sentences);
	}

	/// Releases the unused capacity of every n-gram model once learning is over.
	fn shrink_to_fit(&mut self) {
		self.ngrams.shrink_to_fit();
		for model in self.ngrams.values_mut() {
			model.shrink_to_fit();
		}
		self.sentences.shrink_to_fit();
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

	/// Returns the size of the model: sentences, states and their bytes.
	pub fn stats(&self) -> ModelStats {
		let mut stats = ModelStats {
			sentences: self.sentences.len(),
			sentence_bytes: self.sentences.iter().map(|sentence| sentence.len()).sum(),
			max_n: self.ngrams.keys().copied().max().unwrap_or(0),
			..ModelStats::default()
		};
		for model in self.ngrams.values() {
			let (states, key_bytes) = model.states_and_key_bytes();
			stats.states += states;
			stats.key_bytes += key_bytes;
		}
		stats
	}

	/// Returns the number of transitions over every state of every order.
	pub fn transition_count(&self) -> usize {
		self.ngrams.values().map(NGramModel::transition_count).sum()
	}

	/// Counts what `add_sentence` would build from each line of `text`, without building it.
	///
	/// # Parameters
	/// - `text`: the content of a text file, one sentence per line.
	/// - `max_states`: the count stops once the states exceed it.
	///
	/// # Notes
	/// - Each state is recorded as a 64-bit fingerprint, several times smaller than the state:
	///   a text too big to be learned is detected without running out of memory.
	/// - The fingerprints come from SipHash: the fast hasher of the tables pads the last bytes
	///   with zeros, so it gives the same 64 bits to words that differ only by trailing NUL
	///   bytes ("abc", "abc\0"). A table compares the whole keys and only loses a little speed;
	///   a count of fingerprints would count them once.
	/// - The statistics are the ones `stats` gives once the text is learned, as long as
	///   no two fingerprints collide (less than 1 chance in 3 000 below 100 million states).
	pub fn count_text(text: &str, max_states: usize) -> TextCount {
		let mut count = TextCount { complete: true, ..TextCount::default() };
		let mut states: FastHashSet<u64> = FastHashSet::default();
		let mut sentences: FastHashSet<u64> = FastHashSet::default();
		let mut chars: Vec<char> = Vec::new();

		for line in text.lines() {
			count.lines += 1;
			if line.is_empty() {
				count.empty_lines += 1;
			}
			count.max_line_chars = count.max_line_chars.max(line.chars().count());

			let lowercase = line.to_lowercase();
			let mut hasher = DefaultHasher::new();
			hasher.write(lowercase.as_bytes());
			if sentences.insert(hasher.finish()) {
				count.stats.sentences += 1;
				count.stats.sentence_bytes += lowercase.len();
			}

			// Same markers as `add_sentence`
			chars.clear();
			if !line.starts_with(START_CHAR) {
				chars.push(START_CHAR);
			}
			chars.extend(line.chars());
			if !line.ends_with(END_CHAR) {
				chars.push(END_CHAR);
			}
			count.stats.max_n = count.stats.max_n.max(chars.len());

			// A state of order n is a lowercase prefix of n - 1 characters followed by one
			// more character: every substring that does not end on the last character
			for start in 0..chars.len() - 1 {
				let mut hasher = DefaultHasher::new();
				let mut key_bytes = 0;
				for end in start + 1..chars.len() {
					for c in chars[end - 1].to_lowercase() {
						hasher.write_u32(u32::from(c));
						key_bytes += c.len_utf8();
					}
					// The order tells apart equal prefixes of different models
					let mut state = hasher.clone();
					state.write_usize(end - start);
					if states.insert(state.finish()) {
						count.stats.states += 1;
						count.stats.key_bytes += key_bytes;
						if count.stats.states > max_states {
							count.complete = false;
							return count;
						}
					}
				}
			}
		}
		count
	}

	/// Returns the last `n` characters of a string (UTF-8 safe), without copying them.
	fn last_n_chars(s: &str, n: usize) -> &str {
		if n == 0 {
			return "";
		}
		match s.char_indices().rev().nth(n - 1) {
			Some((index, _)) => &s[index..],
			// Fewer than `n` characters: the whole string
			None => s,
		}
	}

	/// Adds a sentence to the model.
	///
	/// # Behavior
	/// - Prepends `START_CHAR` and appends `END_CHAR` if missing.
	/// - Adds the lowercase sentence to `self.sentences`.
	/// - Updates all n-gram models, for n = 2 to len(sentence).
	/// - Creates new `NGramModel`s if needed.
	pub fn add_sentence(&mut self, sentence: &str) {
		let mut s = sentence.to_string();
		// Stored lowercase so that `check_if_exists` is a single lookup
		self.sentences.insert(sentence.to_lowercase().into_boxed_str());

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
	/// - `key`: the prefix string; its last `n - 1` characters are looked up
	/// - `n`: order of the n-gram model to use
	///
	/// # Returns
	/// - `Ok(Some(char))` if a prediction is made
	/// - `Ok(None)` if no prediction is possible (no model of order `n`, key too short, unknown key)
	pub fn predict(
		&self,
		key: &str,
		n: usize
	) -> Result<Option<char>, String> {
		// No model below 2, and the key must hold at least n - 1 characters
		if n < 2 || key.chars().count() < n - 1 {
			return Ok(None);
		}
		let mut next_char = None;
		if let Some(model) = self.ngrams.get(&n) {
			let prefix = Self::last_n_chars(key, n - 1);
			if let Some(c) = model.predict(prefix) {
				next_char = Some(c);
			}
		}

		Ok(next_char)
	}

	/// Checks if a word already exists in the model, ignoring case.
	pub fn check_if_exists(&self, word: &str) -> bool {
		self.contains_lowercase(&word.to_lowercase())
	}

	/// Checks if an already lowercase word exists in the model.
	pub(crate) fn contains_lowercase(&self, lowercase_word: &str) -> bool {
		self.sentences.contains(lowercase_word)
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

/// One state as (n, prefix, sorted transitions), to compare models in tests.
#[cfg(test)]
pub(crate) type StateDump = (usize, String, Vec<(char, usize)>);

#[cfg(test)]
impl MultiGramModel {
	/// Returns every state, sorted, to compare models in tests.
	pub(crate) fn dump(&self) -> Vec<StateDump> {
		let mut states: Vec<StateDump> = Vec::new();
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
	fn states_of_ab() -> Vec<StateDump> {
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
	fn sentences_are_stored_lowercase() {
		// check_if_exists runs for every model at each try: it must stay a single lookup
		let mut model = MultiGramModel::default();
		model.add_sentence("Saint-Étienne");
		assert!(model.sentences.contains("saint-étienne"));
		assert!(!model.sentences.contains("Saint-Étienne"));
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
	fn predict_without_a_model_of_that_order_returns_none() {
		let mut model = MultiGramModel::default();
		model.add_sentence("abc");
		assert_eq!(model.predict("<ab", 0), Ok(None));
		assert_eq!(model.predict("<ab", 1), Ok(None));
		assert_eq!(model.predict("<ab", 9), Ok(None));
	}

	#[test]
	fn last_n_chars_counts_characters_not_bytes() {
		assert_eq!(MultiGramModel::last_n_chars("<été", 2), "té");
		assert_eq!(MultiGramModel::last_n_chars("<été", 4), "<été");
		assert_eq!(MultiGramModel::last_n_chars("<été", 9), "<été");
		assert_eq!(MultiGramModel::last_n_chars("<été", 0), "");
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
		let start: Vec<StateDump> = model.dump()
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

	#[test]
	fn empty_text_file_gives_an_empty_model() {
		let dir = test_dir("empty");
		let dat = dir.join("empty.dat");
		std::fs::write(&dat, "").unwrap();
		let model = MultiGramModel::new(&dat).unwrap();
		assert_eq!(model.size(), 0);
		assert_eq!(model.predict("<a", 2), Ok(None));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn cache_written_with_another_layout_is_rebuilt() {
		let dir = test_dir("stale-cache");
		let dat = dir.join("words.dat");
		std::fs::write(&dat, "ab\n").unwrap();
		// A cache without the current header, describing another word
		let mut stale = MultiGramModel::default();
		stale.add_sentence("zz");
		let stale_bytes = postcard::to_stdvec(&stale).unwrap();
		std::fs::write(dir.join("words.bin"), &stale_bytes).unwrap();

		let model = MultiGramModel::new(&dat).unwrap();

		assert_eq!(model.dump(), states_of_ab());
		assert_ne!(std::fs::read(dir.join("words.bin")).unwrap(), stale_bytes);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_damaged_cache_is_rebuilt() {
		// A write cut short leaves the right header and half a model: the server must still start
		let dir = test_dir("damaged-cache");
		let dat = dir.join("words.dat");
		std::fs::write(&dat, "ab\n").unwrap();
		MultiGramModel::new(&dat).unwrap();
		let bin = dir.join("words.bin");
		let bytes = std::fs::read(&bin).unwrap();
		std::fs::write(&bin, &bytes[..CACHE_HEADER.len() + 3]).unwrap();

		let model = MultiGramModel::new(&dat).unwrap();

		assert_eq!(model.dump(), states_of_ab());
		assert_eq!(std::fs::read(&bin).unwrap(), bytes);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn the_cache_is_written_without_leaving_a_temporary_file() {
		let dir = test_dir("cache-temporary");
		std::fs::write(dir.join("words.dat"), "ab\n").unwrap();
		MultiGramModel::new(dir.join("words.dat")).unwrap();
		let mut files: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
		files.sort();
		assert_eq!(files, vec!["words.bin", "words.dat"]);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_parallel_build_gives_the_model_learned_in_order() {
		// Enough lines for one chunk per processor, sharing prefixes across the chunks
		let text: String = (0..2000).map(|i| format!("w{}x{}\n", i % 37, i % 101)).collect();
		let dir = test_dir("parallel-build");
		std::fs::write(dir.join("words.dat"), &text).unwrap();
		let built = MultiGramModel::read_database_file(dir.join("words.dat"), &LearnProgress::default()).unwrap();
		let learned = model_of(&text);
		assert_eq!(built.dump(), learned.dump());
		assert_eq!(built.stats(), learned.stats());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn learn_file_counts_every_line() {
		// More lines than one progress batch, over several processors
		let text: String = (0..PROGRESS_STEP * 3 + 7).map(|i| format!("w{}\n", i)).collect();
		let dir = test_dir("learn-progress");
		let dat = dir.join("words.dat");
		std::fs::write(&dat, &text).unwrap();
		let progress = LearnProgress::default();
		assert_eq!(progress.lines(), (0, 0));
		let model = MultiGramModel::learn_file(&dat, &progress).unwrap();
		assert_eq!(progress.lines(), (PROGRESS_STEP * 3 + 7, PROGRESS_STEP * 3 + 7));
		assert_eq!(model.get_name(), "words");
		assert_eq!(model.dump(), model_of(&text).dump());
		// Learning does not write the cache
		assert!(!dir.join("words.bin").exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn from_cache_reads_what_save_cache_wrote() {
		let dir = test_dir("cache-steps");
		let dat = dir.join("words.dat");
		std::fs::write(&dat, "ab\nPomme\n").unwrap();
		assert!(MultiGramModel::from_cache(&dat).unwrap().is_none());
		let learned = MultiGramModel::learn_file(&dat, &LearnProgress::default()).unwrap();
		learned.save_cache(&dat).unwrap();
		let loaded = MultiGramModel::from_cache(&dat).unwrap().expect("the cache was just written");
		assert_eq!(loaded.get_name(), "words");
		assert_eq!(loaded.dump(), learned.dump());
		assert_eq!(loaded.stats(), learned.stats());
		// `new` reads the same cache
		assert_eq!(MultiGramModel::new(&dat).unwrap().dump(), learned.dump());
		let _ = std::fs::remove_dir_all(&dir);
	}

	// Learns every line of a text, as `read_database_file` does
	fn model_of(text: &str) -> MultiGramModel {
		let mut model = MultiGramModel::default();
		for line in text.lines() {
			model.add_sentence(line);
		}
		model
	}

	#[test]
	fn stats_counts_the_sentences_and_the_states() {
		let model = model_of("ab\n");
		// States "<", "a", "b", "<a", "ab", "<ab": 10 bytes
		assert_eq!(model.stats(), ModelStats { sentences: 1, sentence_bytes: 2, max_n: 4, states: 6, key_bytes: 10 });
		assert_eq!(model.transition_count(), 6);
		assert_eq!(MultiGramModel::default().stats(), ModelStats::default());
	}

	#[test]
	fn the_estimates_follow_the_frozen_and_the_learning_layouts() {
		let stats = ModelStats { sentences: 0, sentence_bytes: 0, max_n: 3, states: 1000, key_bytes: 0 };
		// A frozen state is a 32-byte entry and 4-byte index slots, at most 70 % used: about
		// 38 bytes at least; measured 44 to 45 on the server
		let per_state = stats.estimated_bytes() / 1000;
		assert!((38..=50).contains(&per_state), "{} bytes per frozen state", per_state);
		// Learning goes through the hash table, at 88 bytes per state, 1.5 times at least
		assert!(stats.estimated_build_bytes(0) >= 1000 * 88 * 3 / 2);
	}

	#[test]
	fn count_text_gives_the_stats_of_the_learned_text() {
		let texts = [
			"ab\nab\n",
			"Paris\nparis\nPARIS\n",
			"Saint-Étienne\nÉté\nété\n",
			"<ab>\n<x\ny>\n<\n>\n",
			"\n\nab\n",
			"İstanbul\nistanbul\n",
			"aaaa\naaab\nbaaa\n",
			"a\r\nb\r\n",
			"",
			// Pairs that differ only by their last byte
			"Susanna\nSusannah\nacademics\nacademise\nberakot\nberakoth\n",
			// "İ" lowercases to two characters: "<İ" and "<i\u{307}" give the same prefix
			// for two different orders, hence two states
			"İ\ni\u{307}\n",
		];
		for text in texts {
			let count = MultiGramModel::count_text(text, usize::MAX);
			assert!(count.complete, "{:?}", text);
			assert_eq!(count.stats, model_of(text).stats(), "{:?}", text);
		}
	}

	#[test]
	fn count_text_stops_at_the_state_limit() {
		// "<abcdef>" has 28 states
		assert_eq!(MultiGramModel::count_text("abcdef", usize::MAX).stats.states, 28);
		let count = MultiGramModel::count_text("abcdef", 10);
		assert!(!count.complete);
		assert_eq!(count.stats.states, 11);
	}

	#[test]
	fn count_text_describes_the_lines() {
		let count = MultiGramModel::count_text("ab\n\nabcdé\n", usize::MAX);
		assert_eq!((count.lines, count.empty_lines, count.max_line_chars), (3, 1, 5));
	}
}
