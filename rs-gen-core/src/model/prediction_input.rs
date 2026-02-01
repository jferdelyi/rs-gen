use std::collections::HashMap;

/// Strategy used to select the starting seed when generating a sequence.
///
/// This enum controls how the initial prefix is chosen before generation begins.
///
/// # Variants
/// - `Random(usize)`: select a random starting prefix, using up to the given
///   n-gram size as a bound.
/// - `Custom(&str)`: use the provided string as the initial prefix.
/// - `False`: do not use any explicit seed; generation starts without
///   a predefined prefix.
#[derive(PartialEq)]
pub enum StartSeed {
	Random(usize),
	Custom(String),
	False,
}

/// Input parameters for generating predictions from multiple models.
///
/// `PredictionInput` contains both **configuration parameters** (like randomness,
/// number of tries, start seed) and **model selection weights** (intensity and
/// normalized probabilities).
///
/// # Responsibilities
/// - Track generation parameters (`max_n`, `nb_try`, `randomness`, `reduce_random`, `start_seed`)
/// - Track per-model intensity (user-adjustable) and compute normalized selection probabilities
/// - Ensure probabilities always sum to 1.0 (if there are any models)
///
/// # Invariants
/// - `models_intensity` keys are fixed and correspond to available models
/// - `models_probability` is always normalized or uniform if all intensities are zero
pub struct PredictionInput {
	/// Maximum n-gram size to consider in predictions.
	pub max_n: usize,

	/// Number of attempts to try when generating a prediction.
	pub nb_try: usize,

	/// Randomness factor for generation (0.0 = deterministic, 1.0 = fully random).
	randomness: f32,

	/// Whether to reduce n when falling back during a generation.
	pub reduce_random: bool,

	/// Optional starting seed for generation.
	pub start_seed: StartSeed,

	/// User-adjustable model intensities (not normalized).
	models_intensity: HashMap<String, f32>,

	/// Normalized probabilities derived from `models_intensity`.
	models_probability: HashMap<String, f32>,
}

impl<'a> PredictionInput {
	/// Creates a new `PredictionInput` from a set of model intensities.
	///
	/// Normalizes the probabilities internally.
	///
	/// # Visibility
	/// - `pub(crate)` to prevent construction outside the crate.
	pub(crate) fn new(models_intensity: HashMap<String, f32>) -> Self {
		let mut prediction_input = Self {
			max_n: 0,
			nb_try: 0,
			randomness: 0.0,
			reduce_random: false,
			start_seed: StartSeed::False,
			models_intensity,
			models_probability: HashMap::new(),
		};

		prediction_input.normalize();
		prediction_input
	}

	/// Normalizes `models_intensity` into `models_probability`.
	///
	/// - If total intensity > 0, probabilities are proportional to intensities.
	/// - If all intensities are 0, distributes uniform probabilities across models.
	fn normalize(&mut self) {
		let sum: f32 = self.models_intensity.values().sum();

		self.models_probability.clear();

		if sum > 0.0 {
			for (model, intensity) in &self.models_intensity {
				self.models_probability.insert(model.clone(), intensity / sum);
			}
		} else {
			let count = self.models_intensity.len();
			if count > 0 {
				let uniform = 1.0 / count as f32;
				for model in self.models_intensity.keys() {
					self.models_probability.insert(model.clone(), uniform);
				}
			}
		}
	}

	/// Returns the current randomness factor.
	pub fn randomness(&self) -> f32 {
		self.randomness
	}

	/// Returns an iterator over normalized model probabilities.
	///
	/// Each item is a tuple `(model_name, probability)`.
	pub fn models_probability(&self) -> impl Iterator<Item=(&str, f32)> {
		self.models_probability.iter().map(|(k, v)| (k.as_str(), *v))
	}

	/// Sets the randomness factor (0.0..1.0).
	///
	/// # Errors
	/// Returns an error if the value is outside the valid range.
	pub fn set_randomness(&mut self, randomness: f32) -> Result<(), String> {
		if !(0.0..=1.0).contains(&randomness) {
			return Err("Randomness must be between 0.0 and 1.0".to_owned());
		}
		self.randomness = randomness;
		Ok(())
	}

	/// Sets the intensity of a specific model.
	///
	/// Automatically normalizes probabilities after adjustment.
	///
	/// # Parameters
	/// - `model`: Model name to adjust.
	/// - `intensity`: New intensity value.
	///
	/// # Errors
	/// Returns an error if the model does not exist.
	pub fn set_model_intensity(&mut self, model: &str, intensity: f32) -> Result<(), String> {
		if !self.models_intensity.contains_key(model) {
			return Err(format!("Model {} not found", model));
		}
		if let Some(p) = self.models_intensity.get_mut(model) {
			*p = intensity;
		}
		self.normalize();
		Ok(())
	}
}
