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
#[derive(Debug)]
pub struct Generator {
	models: HashMap<String, MultiGramModel>
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
		self.models.keys().map(|k| k.to_owned()).collect::<Vec<_>>()
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

	/// Creates a new `PredictionInput` with default intensity for all models.
	///
	/// Probabilities are automatically normalized.
	pub fn make_prediction_input<'a>(&self) -> PredictionInput {
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
		let mut rng = rand::rng();

		let mut scored: Vec<(String, f64)> = prediction_input
			.models_probability()
			.filter_map(|(name, weight): (&str, f32)| {
				if weight > 0.0 && self.models.contains_key(name) {
					let u: f64 = rng.random();
					let key = u.powf(1.0 / weight as f64);
					Some((name.to_string(), key))
				} else {
					None
				}
			})
			.collect();

		scored.sort_by(|a, b| b.1.total_cmp(&a.1));

		scored.into_iter().map(|(name, _)| name).collect()
	}

	/// Internal sequence generation logic using a single model selection.
	///
	/// # Parameters
	/// - `prediction_input`: Controls randomness, start seed, and max_n.
	///
	/// # Returns
	/// - `Ok(String)` with a generated sequence
	/// - `Err(String)` if no models or seeds are available
	///
	/// # Notes
	/// - Handles `StartSeed::False`, `StartSeed::Random`, `StartSeed::Custom`.
	/// - Iterates over characters safely (UTF-8 aware).
	/// - Loops until no next character is returned.
	fn internal_predict(&mut self, prediction_input: &PredictionInput) -> Result<String, String> {
		if self.models.is_empty() {
			return Err("No models available for prediction".to_owned());
		}

		let max_n: usize = if prediction_input.max_n < 2 { 0 } else { prediction_input.max_n };

		let mut word = String::new();
		let mut global_prefix = String::new();

		let mut models = self.get_random_models(prediction_input);
		let mut model = match self.models.get_mut(&models[0]) {
			Some(model) => model,
			None => return Err("No model available for prediction".to_owned())
		};

		match &prediction_input.start_seed {
			StartSeed::False => (),
			StartSeed::Random(random) => {
				let n = if *random <= 0 {
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

		let models_size = models.len();
		let mut model_index = 0;
		let mut n = Generator::compute_n(global_prefix.chars().count(), max_n, prediction_input.randomness())?;
		loop {
			match model.predict(&mut global_prefix, n)? {
				Some(c) => {
					global_prefix.push(c);
					if c != '>' {
						word.push(c);
					} else {
						break;
					}
				},
				None => {
					model_index = model_index + 1;
					if model_index >= models_size {
						model_index = 0;
						models = self.get_random_models(prediction_input);
						model = match self.models.get_mut(&models[model_index]) {
							Some(model) => model,
							None => return Err("No model available for prediction".to_owned())
						};
						if prediction_input.reduce_random {
							n = Generator::compute_randomness(prediction_input.randomness(), n - 1, n)?;
						} else {
							n = n - 1;
						}
						continue;
					} else {
						model = match self.models.get_mut(&models[model_index]) {
							Some(model) => model,
							None => return Err("No model available for prediction".to_owned())
						};
						continue;
					}
				}
			};

			n = Generator::compute_n(global_prefix.chars().count(), max_n, prediction_input.randomness())?;
			model_index = 0;
			models = self.get_random_models(prediction_input);
			model = match self.models.get_mut(&models[model_index]) {
				Some(model) => model,
				None => return Err("No model available for prediction".to_owned())
			};
		}

		Ok(word)
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
		if randomness < 0.0 || randomness > 1.0 {
			Err(format!(
				"randomness must be between 0.0 and 1.0, got {}",
				randomness
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
	pub fn predict(&mut self, prediction_input: &PredictionInput) -> Result<String, String> {
		let mut word = self.internal_predict(prediction_input)?;
		let mut nb_try = prediction_input.nb_try;

		loop {
			let mut exists = false;
			if nb_try > 0 {
				for model in self.models.values() {
					if model.check_if_exists(&word) {
						exists = true;
						break;
					}
				}
				if exists {
					word = self.internal_predict(prediction_input)?;
					nb_try -= 1;
				}
			}
			if !exists || nb_try <= 0 { break; }
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