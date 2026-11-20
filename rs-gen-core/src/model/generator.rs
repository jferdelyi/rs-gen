use crate::io;
use crate::model::multigram_model::{MultiGramModel, START_CHAR};
use crate::model::prediction_input::{PredictionInput, StartSeed};
use rand::Rng;
use std::cmp::{max, min};
use std::collections::HashMap;
use std::path::Path;

/// High-level generator managing multiple n-gram models.
///
/// # Responsibilities
/// - Load and manage multiple `MultiGramModel`s
/// - Provide weighted model selection using `PredictionInput` probabilities
/// - Generate sequences with randomness, seed control, and duplicate avoidance
#[derive(Debug, Default)]
pub struct Generator {
	models: HashMap<String, MultiGramModel>
}

/// Draws the models one by one in a weighted random order, a model with a higher weight
/// coming first more often (weighted sampling without replacement).
///
/// Each draw costs one random number, so a character predicted by the first model drawn
/// does not pay for ordering every model.
struct ModelDraw<'m, 'a> {
	/// Models with a positive weight, and their weight.
	models: &'m [(&'a MultiGramModel, f64)],

	/// Models already drawn since the last restart.
	drawn: Vec<bool>,

	/// Sum of the weights of the models not drawn yet.
	remaining_weight: f64,

	/// Number of models not drawn yet.
	remaining: usize,
}

impl<'m, 'a> ModelDraw<'m, 'a> {
	/// Creates a draw over `models`, ready for its first model.
	fn new(models: &'m [(&'a MultiGramModel, f64)]) -> Self {
		let mut draw = Self {
			models,
			drawn: vec![false; models.len()],
			remaining_weight: 0.0,
			remaining: 0,
		};
		draw.reset();
		draw
	}

	/// Puts every model back.
	fn reset(&mut self) {
		self.drawn.fill(false);
		self.remaining_weight = self.models.iter().map(|(_, weight)| weight).sum();
		self.remaining = self.models.len();
	}

	/// Puts every model back and draws the first one (requires at least one model).
	fn restart(&mut self) -> &'a MultiGramModel {
		self.reset();
		self.next().expect("a draw restarts only over models")
	}

	/// Draws the next model, or `None` once every model was drawn.
	fn next(&mut self) -> Option<&'a MultiGramModel> {
		if self.remaining == 0 {
			return None;
		}
		let mut r = rand::rng().random::<f64>() * self.remaining_weight;
		let mut chosen = None;
		for (index, (_, weight)) in self.models.iter().enumerate() {
			if self.drawn[index] {
				continue;
			}
			// The last model not drawn is kept in case rounding leaves `r` above every weight
			chosen = Some(index);
			if r < *weight {
				break;
			}
			r -= weight;
		}
		let index = chosen?;
		self.drawn[index] = true;
		self.remaining_weight -= self.models[index].1;
		self.remaining -= 1;
		Some(self.models[index].0)
	}
}

impl Generator {
	/// Creates a generator by loading all `.dat` models from a directory.
	///
	/// # Parameters
	/// - `filepath`: Path to a directory containing model files.
	///   Both `"folder"` and `"folder/"` are accepted.
	///
	/// # Behavior
	/// - Lists all files with the `.dat` extension in the given directory.
	/// - Loads each model into the generator.
	/// - The model name is derived from the file name (without extension).
	///
	/// # Errors
	/// - Returns an error if the path does not exist or is not a directory.
	/// - Returns an error if a model fails to load.
	///
	/// # Notes
	/// - The directory path is normalized using `resolve_folder`.
	/// - Only files directly contained in the directory are loaded
	///   (subdirectories are ignored).
	pub fn new<P: AsRef<Path>>(filepath: P) -> Result<Self, Box<dyn std::error::Error>> {
		let mut generator = Self {
			models: HashMap::new()
		};

		let string_path = match filepath.as_ref().to_str() {
			Some(s) => s,
			None => return Err("Invalid filepath".into()),
		};
		// Normalize "folder" / "folder/"
		let folder = io::normalize_folder(string_path);

		if !folder.is_dir() {
			return Err(format!("Expected a directory, got: {}", folder.display()).into());
		}

		for file in io::list_files(&folder, "dat")? {
			let full_path = folder.join(&file);
			generator.load_model(&full_path)?;
		}

		Ok(generator)
	}

	/// Returns the list of loaded model names.
	///
	/// Provides a read-only reference to internal names.
	pub fn get_model_names(&self) -> Vec<String> {
		self.models.keys().map(std::borrow::ToOwned::to_owned).collect::<Vec<_>>()
	}

	/// Loads a `MultiGramModel` from a file path.
	///
	/// # Errors
	/// Returns an error if the model is already loaded or if file I/O fails.
	fn load_model<P: AsRef<Path>>(&mut self, filepath: P) -> Result<(), Box<dyn std::error::Error>> {
		let path = filepath.as_ref();
		let key = io::get_filename(&filepath)?;
		if self.models.contains_key(&key) {
			return Err(Box::from("Model already loaded".to_owned()));
		}
		let model = MultiGramModel::new(path)?;
		self.models.insert(key, model);
		Ok(())
	}

	/// Adds an already loaded model, under its name.
	///
	/// Loading a model takes time: load it with `MultiGramModel::new` first, so that
	/// a generator shared between threads is locked only for the insertion.
	///
	/// # Errors
	/// Returns an error if a model with the same name is already loaded.
	pub fn insert_model(&mut self, model: MultiGramModel) -> Result<(), String> {
		let name = model.get_name().to_owned();
		if self.models.contains_key(&name) {
			return Err(format!("Model already loaded: {name}"));
		}
		self.models.insert(name, model);
		Ok(())
	}

	/// Removes a model and gives it back, or `None` if no model has this name.
	pub fn remove_model(&mut self, name: &str) -> Option<MultiGramModel> {
		self.models.remove(name)
	}

	/// Returns the model with this name, if loaded.
	pub fn get_model(&self, name: &str) -> Option<&MultiGramModel> {
		self.models.get(name)
	}

	/// Creates a new `PredictionInput` with default intensity for all models.
	///
	/// Probabilities are automatically normalized.
	pub fn make_prediction_input(&self) -> PredictionInput {
		let mut models_probability: HashMap<String, f32> = HashMap::new();
		for model in self.models.values() {
			models_probability.insert(model.get_name().to_owned(), 0.0);
		}
		PredictionInput::new(models_probability)
	}

	/// Returns a randomly ordered list of model names based on weighted probabilities.
	///
	/// Models with higher probability are more likely to appear earlier in the list.
	/// Only models present in `self.models` with a positive weight are included.
	pub fn get_random_models(
		&self,
		prediction_input: &PredictionInput,
	) -> Vec<String> {
		let models = self.weighted_models(prediction_input);
		let mut draw = ModelDraw::new(&models);
		std::iter::from_fn(|| draw.next()).map(|model| model.get_name().to_owned()).collect()
	}

	/// Returns the models with a positive weight, with their weight.
	///
	/// Computed once per generation, so that the loop over the characters
	/// never looks a model up by its name.
	fn weighted_models(&self, prediction_input: &PredictionInput) -> Vec<(&MultiGramModel, f64)> {
		prediction_input
			.models_probability()
			.filter(|(_, weight)| *weight > 0.0)
			.filter_map(|(name, weight)| self.models.get(name).map(|model| (model, f64::from(weight))))
			.collect()
	}

	/// Internal sequence generation logic using a single model selection.
	///
	/// # Parameters
	/// - `models`: Models with a positive weight, from `weighted_models`.
	/// - `prediction_input`: Controls randomness, start seed, and max_n.
	///
	/// # Returns
	/// - `Ok(String)` with a generated sequence
	/// - `Err(String)` if no models or seeds are available
	///
	/// # Notes
	/// - Handles `StartSeed::None`, `StartSeed::Random`, `StartSeed::Custom`.
	/// - Iterates over characters safely (UTF-8 aware).
	/// - Loops until no next character is returned.
	fn internal_predict(&self, models: &[(&MultiGramModel, f64)], prediction_input: &PredictionInput) -> Result<String, String> {
		if models.is_empty() {
			return Err("No model available for prediction".to_owned());
		}

		let max_n: usize = if prediction_input.max_n < 2 { 0 } else { prediction_input.max_n };

		let mut word = String::new();
		let mut global_prefix = String::new();

		let mut draw = ModelDraw::new(models);
		let Some(mut model) = draw.next() else {
			return Err("No model available for prediction".to_owned());
		};

		match &prediction_input.start_seed {
			StartSeed::None => (),
			StartSeed::Random(random) => {
				let n = if *random == 0 {
					rand::rng().random_range(2..=model.size() + 1)
				} else {
					max(2, min(*random, model.size() + 1))
				};

				word = match model.get_random_seed(n) {
					Some(s) => s.to_owned(),
					None => return Err("No seed available for prediction".to_owned())
				};
				if word.starts_with(START_CHAR) { word.remove(0); }
				global_prefix = word.clone();
			}
			StartSeed::Custom(s) => {
				word = (*s).to_owned();
				if word.starts_with(START_CHAR) { word.remove(0); }
				global_prefix = word.clone();
			}
		}

		if !global_prefix.starts_with(START_CHAR) {
			global_prefix.insert(0, START_CHAR);
		}

		let mut n = Generator::compute_n(global_prefix.chars().count(), max_n, prediction_input.randomness())?;
		loop {
			match model.predict(&global_prefix, n)? {
				Some(c) => {
					global_prefix.push(c);
					if c != '>' {
						word.push(c);
					} else {
						break;
					}
				},
				None => {
					// Next model in the weighted random order
					if let Some(next) = draw.next() {
						model = next;
						continue;
					}
					// Every model failed: a smaller n is needed, and 2 is the smallest one
					if n <= 2 {
						return Err("No model can continue the prefix".to_owned());
					}
					n = Generator::reduce_n(n, prediction_input);
					model = draw.restart();
					continue;
				}
			};

			n = Generator::compute_n(global_prefix.chars().count(), max_n, prediction_input.randomness())?;
			model = draw.restart();
		}

		Ok(word)
	}

	/// Returns a smaller n after every model failed with `n` (requires `n > 2`).
	///
	/// - Without `reduce_random`, or with a zero randomness: `n - 1`.
	/// - With `reduce_random`: a random value in `[2, n - 1]`.
	///
	/// The result is always lower than `n`: keeping `n` would fail again the same way.
	fn reduce_n(n: usize, prediction_input: &PredictionInput) -> usize {
		if prediction_input.reduce_random && prediction_input.randomness() > 0.0 {
			rand::rng().random_range(2..=n - 1)
		} else {
			n - 1
		}
	}

	/// Compute next n
	fn compute_n(prefix_size: usize, max_n: usize, randomness: f32) -> Result<usize, String> {
		let key_len = prefix_size;
		let n = if max_n == 0 {
			key_len + 1
		} else {
			max_n.min(key_len + 1)
		};
		Generator::compute_randomness(randomness, max_n, n)
	}

	/// Computes a possibly randomized n-gram size based on a randomness factor.
	///
	/// This helper function is used to introduce controlled variability
	/// when selecting the n-gram size during prediction.
	///
	/// # Parameters
	/// - `randomness`: Probability in the range [0.0, 1.0] of selecting
	///   a random n-gram size.
	/// - `max`: Maximum n-gram size allowed (inclusive).
	/// - `default`: Default n-gram size used when randomness is not applied.
	///
	/// # Returns
	/// - `Ok(usize)`: Either `default` or a randomly selected value in the range `[2, max]`.
	/// - `Err(String)`: If `randomness` is outside the valid range.
	///
	/// # Behavior
	/// - With probability `randomness`, selects a random value in `[2, max]`.
	/// - Otherwise, returns `default`.
	/// - Ensures the returned n-gram size is always at least 2.
	fn compute_randomness(randomness: f32, max: usize, default: usize) -> Result<usize, String> {
		if !(0.0..=1.0).contains(&randomness) {
			Err(format!(
				"randomness must be between 0.0 and 1.0, got {randomness}"
			))?;
		}

		if randomness > 0.0 && rand::rng().random_range(0.0..=1.0) <= randomness {
			return Ok(rand::rng().random_range(2..=max.max(2)));
		}

		Ok(default)
	}

	/// Generates a sequence using multiple models while avoiding duplicates.
	///
	/// # Behavior
	/// - Calls `internal_predict`.
	/// - Checks for duplicates against all loaded models up to `nb_try`.
	/// - Returns the first non-duplicate or the last attempt if all fail.
	pub fn predict(&self, prediction_input: &PredictionInput) -> Result<String, String> {
		if self.models.is_empty() {
			return Err("No models available for prediction".to_owned());
		}
		let models = self.weighted_models(prediction_input);
		let mut word = self.internal_predict(&models, prediction_input)?;
		let mut nb_try = prediction_input.nb_try;

		loop {
			let mut exists = false;
			if nb_try > 0 {
				// Lowercased once for all the models
				let lowercase_word = word.to_lowercase();
				exists = self.models.values().any(|model| model.contains_lowercase(&lowercase_word));
				if exists {
					word = self.internal_predict(&models, prediction_input)?;
					nb_try -= 1;
				}
			}
			if !exists || nb_try == 0 { break; }
		}

		Ok(word)
	}

	/// Merges another `Generator` into this one.
	///
	/// # Notes
	/// - Existing models are merged; new ones are cloned.
	/// - Model names are appended.
	/// - Does not merge sentences internally (handled at model level).
	pub fn merge(&mut self, other: &Self) -> Result<(), String> {
		for (k, t) in &other.models {
			if let Some(existing) = self.models.get_mut(k) {
				existing.merge(t)?;
			} else {
				self.models.insert(k.clone(), t.clone());
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;
	use std::sync::mpsc;
	use std::time::Duration;

	// Writes one `.dat` file per model in a directory dedicated to one test, then loads them
	fn generator_with(test: &str, models: &[(&str, &str)]) -> (Generator, PathBuf) {
		let dir = std::env::temp_dir().join(format!("rs-gen-generator-{}-{}", std::process::id(), test));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		for (name, lines) in models {
			std::fs::write(dir.join(format!("{name}.dat")), lines).unwrap();
		}
		(Generator::new(&dir).unwrap(), dir)
	}

	// Runs `count` predictions in another thread, so that a generation that never ends fails the test
	fn predict_within_deadline(generator: Generator, input: PredictionInput, count: usize) -> Vec<Result<String, String>> {
		let (tx, rx) = mpsc::channel();
		std::thread::spawn(move || {
			let words: Vec<Result<String, String>> = (0..count).map(|_| generator.predict(&input)).collect();
			let _ = tx.send(words);
		});
		rx.recv_timeout(Duration::from_secs(10)).expect("the generation did not end within 10 seconds")
	}

	#[test]
	fn model_draw_gives_every_model_once() {
		let (a, b, c) = (MultiGramModel::default(), MultiGramModel::default(), MultiGramModel::default());
		let models = [(&a, 0.5), (&b, 0.3), (&c, 0.2)];
		let mut draw = ModelDraw::new(&models);
		for _ in 0..100 {
			let drawn: Vec<&MultiGramModel> = std::iter::from_fn(|| draw.next()).collect();
			assert_eq!(drawn.len(), 3);
			for model in [&a, &b, &c] {
				assert_eq!(drawn.iter().filter(|other| std::ptr::eq(**other, model)).count(), 1);
			}
			draw.reset();
		}
	}

	#[test]
	fn model_draw_follows_the_weights() {
		let (heavy, light) = (MultiGramModel::default(), MultiGramModel::default());
		let models = [(&heavy, 0.75), (&light, 0.25)];
		let mut draw = ModelDraw::new(&models);
		let heavy_first = (0..4000).filter(|_| std::ptr::eq(draw.restart(), &heavy)).count();
		// Expected 3000, with a standard deviation of about 27
		assert!((2700..=3300).contains(&heavy_first), "heavy drawn first {heavy_first} times out of 4000");
	}

	#[test]
	fn predict_uses_only_models_with_a_weight() {
		let (generator, dir) = generator_with("weights", &[("low", "abc\nbca\ncab\n"), ("up", "xyz\nyzx\nzxy\n")]);
		let mut input = generator.make_prediction_input();
		input.set_model_intensity("low", 1.0).unwrap();
		input.set_model_intensity("up", 0.0).unwrap();
		for word in predict_within_deadline(generator, input, 100) {
			let word = word.unwrap();
			assert!(word.chars().all(|c| "abc".contains(c)), "unexpected word {word}");
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn predict_keeps_a_custom_seed() {
		let (generator, dir) = generator_with("custom-seed", &[("low", "abc\nabd\n")]);
		let mut input = generator.make_prediction_input();
		input.start_seed = StartSeed::Custom("ab".to_owned());
		for word in predict_within_deadline(generator, input, 20) {
			let word = word.unwrap();
			assert!(word == "abc" || word == "abd", "unexpected word {word}");
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn predict_starts_from_a_random_seed_of_the_model() {
		let (generator, dir) = generator_with("random-seed", &[("low", "abc\n")]);
		let mut input = generator.make_prediction_input();
		input.start_seed = StartSeed::Random(3);
		for word in predict_within_deadline(generator, input, 20) {
			let word = word.unwrap();
			assert!(["abc", "bc", "c"].contains(&word.as_str()), "unexpected word {word}");
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn predict_stops_when_the_seed_has_an_unknown_character() {
		let (generator, dir) = generator_with("unknown-char", &[("low", "abc\n")]);
		let mut input = generator.make_prediction_input();
		input.start_seed = StartSeed::Custom("a€".to_owned());
		for word in predict_within_deadline(generator, input, 3) {
			assert!(word.is_err(), "no model knows '€', got {word:?}");
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn reduce_random_without_randomness_still_reduces() {
		// "<ba" exists in no model: the reduction must reach "a" even with a zero randomness
		let (generator, dir) = generator_with("reduce-random", &[("low", "abc\n")]);
		let mut input = generator.make_prediction_input();
		input.start_seed = StartSeed::Custom("ba".to_owned());
		input.reduce_random = true;
		input.set_randomness(0.0).unwrap();
		for word in predict_within_deadline(generator, input, 3) {
			assert_eq!(word, Ok("babc".to_owned()));
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	// Loads a model from a sub-directory, which `Generator::new` does not read
	fn model_outside(dir: &Path, name: &str, lines: &str) -> MultiGramModel {
		let other = dir.join("other");
		std::fs::create_dir_all(&other).unwrap();
		let path = other.join(format!("{name}.dat"));
		std::fs::write(&path, lines).unwrap();
		MultiGramModel::new(&path).unwrap()
	}

	#[test]
	fn insert_model_adds_a_model_under_its_name() {
		let (mut generator, dir) = generator_with("insert", &[("low", "abc\n")]);
		generator.insert_model(model_outside(&dir, "up", "xyz\nyzx\n")).unwrap();
		let mut names = generator.get_model_names();
		names.sort();
		assert_eq!(names, vec!["low", "up"]);
		assert_eq!(generator.get_model("up").map(MultiGramModel::get_name), Some("up"));

		let mut input = generator.make_prediction_input();
		input.set_model_intensity("up", 1.0).unwrap();
		for word in predict_within_deadline(generator, input, 20) {
			let word = word.unwrap();
			assert!(word.chars().all(|c| "xyz".contains(c)), "unexpected word {word}");
		}
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn insert_model_rejects_a_name_already_loaded() {
		let (mut generator, dir) = generator_with("insert-twice", &[("low", "abc\n")]);
		assert!(generator.insert_model(model_outside(&dir, "low", "xyz\n")).is_err());
		// The loaded model is kept
		assert!(generator.get_model("low").unwrap().check_if_exists("abc"));
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn remove_model_gives_the_model_back() {
		let (mut generator, dir) = generator_with("remove", &[("low", "abc\n")]);
		let model = generator.remove_model("low").unwrap();
		assert_eq!(model.get_name(), "low");
		assert!(generator.get_model_names().is_empty());
		assert!(generator.remove_model("low").is_none());
		let input = generator.make_prediction_input();
		assert!(generator.predict(&input).is_err());
		let _ = std::fs::remove_dir_all(&dir);
	}
}