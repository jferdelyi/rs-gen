//! How a word was generated, letter by letter, from `Generator::predict_with_trace`.

use serde::Serialize;

/// How a word was generated: its seed, each letter generated after it, its end, and the
/// words generated before it but refused because a training file already held them.
///
/// The seed followed by the letters spells `word`.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct WordTrace {
	/// The word.
	pub word: String,

	/// The start of the word given to the generation, if any.
	pub seed: Option<SeedTrace>,

	/// Each letter generated after the seed.
	pub letters: Vec<LetterTrace>,

	/// How the word ended: the end is drawn as the letters are.
	pub end: Step,

	/// The words refused before this one (Retry Attempts), in order.
	pub rejected: Vec<String>,
}

/// The start of a word given to the generation.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SeedTrace {
	/// The letters of the seed.
	pub text: String,

	/// The model the seed was drawn from (`StartSeed::Random`); none for a seed given as it is.
	pub model: Option<String>,
}

/// A letter generated after the seed.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct LetterTrace {
	/// The letter.
	pub letter: char,

	/// How it was drawn.
	#[serde(flatten)]
	pub step: Step,
}

/// How one letter, or the end of the word, was drawn.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Step {
	/// The model that gave it.
	pub model: String,

	/// The models drawn before it, which could not continue the context.
	pub fallbacks: Vec<String>,

	/// The n-gram order used.
	pub n: usize,

	/// True when the order came from a random draw (randomness), not from the length of the word.
	pub random_n: bool,

	/// The orders given up before `n`, because no model knew the context.
	pub reductions: Vec<usize>,

	/// The letters looked at: the last `n - 1` of the word, without the start marker.
	pub context: String,

	/// True when the context starts at the start of the word.
	pub at_start: bool,

	/// How many times the training files held this letter after the context.
	pub count: u32,

	/// How many times they held the context followed by any letter: `count / total` is its odds.
	pub total: u64,

	/// How many different letters could follow the context: 1 means the letter was forced.
	pub choices: usize,
}
